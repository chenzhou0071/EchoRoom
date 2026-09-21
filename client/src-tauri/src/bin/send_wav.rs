//! 测试工具：把一段 WAV 录音用真实 Opus 链路按 20ms/帧实时发送到服务器，
//! 模拟"一个正在说话的真实客户端"（供真人收听/看蓝框验证链路）。
//! 说话状态（VAD）与真人客户端一致：能量双阈值检测，翻转时经 TCP 上报。
//! 用法：cargo run --bin send_wav -- <服务器addr> <wav路径> [循环遍数，默认 1] [遍间停顿秒数，默认 0] [邀请码]
//! 账号固定 wavbot：提供邀请码时先注册（已存在自动回退登录），否则直接登录（需已注册过）。
//! WAV 要求 48kHz 单声道 16bit PCM（可直接用 mic_record 录制）。
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use echoroom_client_lib::audio::opus::OpusEnc;
use echoroom_client_lib::audio::vad::SpeakingDetector;
use echoroom_protocol::messages::{TcpMessage, UdpPacket};
use echoroom_protocol::{tcp, udp, FRAME_SAMPLES, HEARTBEAT_INTERVAL_MS};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let wav_path = match args.get(2) {
        Some(p) => p.clone(),
        None => {
            println!("用法: cargo run --bin send_wav -- <服务器addr> <wav路径> [循环遍数，默认 1] [遍间停顿秒数，默认 0] [邀请码]");
            return Ok(());
        }
    };
    let loops: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let gap_secs: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let invite = args.get(5).cloned();

    let pcm = load_wav_48k_mono(&wav_path)?;
    let secs = pcm.len() as f64 / 48000.0;
    println!("已读取 {wav_path}（{secs:.1}s）");

    // ---- TCP 登录拿 uid/token ----
    let account = "wavbot".to_string();
    let password = "pw-wavbot-123456".to_string();
    let mut stream = TcpStream::connect(&addr)?;
    // 提供邀请码：先注册（撞名自动回退登录）；未提供：要求 wavbot 已注册，直接登录
    let first = match &invite {
        Some(code) => TcpMessage::Register {
            account: account.clone(),
            password: password.clone(),
            invite: code.clone(),
        },
        None => TcpMessage::Login { account: account.clone(), password: password.clone() },
    };
    stream.write_all(&tcp::encode(&first))?;
    stream.set_read_timeout(Some(Duration::from_millis(20)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let (mut uid, mut udp_token) = (0u16, 0u32);
    let mut fallback_done = false; // 注册撞名后是否已回退登录（防死循环）
    let mut rejected: Option<String> = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while uid == 0 {
        if let Some(reason) = &rejected {
            anyhow::bail!("认证失败：{reason}");
        }
        if Instant::now() > deadline {
            anyhow::bail!("登录超时（服务器无响应）");
        }
        match stream.read(&mut chunk) {
            Ok(0) => anyhow::bail!("服务器关闭了连接"),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                while let Ok(Some((msg, used))) = tcp::try_decode(&buf) {
                    buf.drain(..used);
                    match msg {
                        TcpMessage::LoginOk { uid: u, udp_token: t, .. } => {
                            uid = u;
                            udp_token = t;
                        }
                        TcpMessage::AuthReject { reason } => {
                            if invite.is_some() && reason == "账号已存在" && !fallback_done {
                                fallback_done = true;
                                println!("wavbot 已注册，回退登录");
                                let login =
                                    TcpMessage::Login { account: account.clone(), password: password.clone() };
                                stream.write_all(&tcp::encode(&login))?;
                            } else {
                                rejected = Some(reason);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {} // 读超时：继续等
        }
    }
    println!("登录成功 uid={uid}");

    // ---- UDP 注册 ----
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(&addr)?;
    sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token: udp_token }))?;

    // ---- 预编码全部帧（顺便算每帧 RMS 供 VAD 使用）----
    let mut enc = OpusEnc::new().map_err(|e| anyhow::anyhow!("编码器创建失败: {e}"))?;
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut rms_list: Vec<f64> = Vec::new();
    for chunk in pcm.chunks_exact(FRAME_SAMPLES) {
        match enc.encode(chunk) {
            Ok(o) => {
                let rms = (chunk.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                    / chunk.len() as f64)
                    .sqrt();
                frames.push(o);
                rms_list.push(rms);
            }
            Err(e) => eprintln!("编码失败（跳过）: {e}"),
        }
    }
    if frames.is_empty() {
        anyhow::bail!("没有可发送的完整帧");
    }

    // ---- 实时发送（每遍独立绝对时间锚定 20ms/帧，防时钟漂移）----
    let per_loop_secs = frames.len() as f64 * 0.02;
    println!(
        "开始发送：{} 帧 × {loops} 遍（每遍约 {per_loop_secs:.1}s，遍间停顿 {gap_secs}s）",
        frames.len(),
    );
    let mut sent: u64 = 0;
    let mut seq: u32 = 0;
    let mut last_hb = Instant::now();
    // VAD：与真人客户端同参数（进入 50 / 退出 25 / 保持 400ms），翻转时上报
    let mut detector = SpeakingDetector::new(50.0, 25.0, Duration::from_millis(400));
    for _ in 0..loops {
        let t0 = Instant::now();
        for (i, f) in frames.iter().enumerate() {
            let target = t0 + Duration::from_millis(i as u64 * 20);
            let now = Instant::now();
            if target > now {
                std::thread::sleep(target - now);
            }
            // 说话状态：帧能量 → 状态翻转即上报（uid=0 由服务器填真实 uid 后广播）
            if let Some(on) = detector.update(rms_list[i], Instant::now()) {
                stream.write_all(&tcp::encode(&TcpMessage::Speaking { uid: 0, on }))?;
            }
            // 静音不发包：与真实客户端一致，说话状态（含 400ms 保持）期间才发送
            if detector.speaking() {
                let pkt = udp::encode(uid, seq, &UdpPacket::Voice { opus: f.clone() });
                sock.send(&pkt)?;
                seq = seq.wrapping_add(1);
                sent += 1;
            }
            if last_hb.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS as u128 {
                let _ = sock.send(&udp::encode(uid, 0, &UdpPacket::Heartbeat));
                last_hb = Instant::now();
            }
        }
        // 遍间停顿：模拟静音输入（喂 0 能量让 VAD 自然翻 off），期间维持心跳
        if gap_secs > 0 {
            let t_end = Instant::now() + Duration::from_secs(gap_secs);
            while Instant::now() < t_end {
                std::thread::sleep(Duration::from_millis(20));
                if let Some(on) = detector.update(0.0, Instant::now()) {
                    stream.write_all(&tcp::encode(&TcpMessage::Speaking { uid: 0, on }))?;
                }
                if last_hb.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS as u128 {
                    let _ = sock.send(&udp::encode(uid, 0, &UdpPacket::Heartbeat));
                    last_hb = Instant::now();
                }
            }
        }
    }
    // 退出前保持连接片刻，让尾部帧完成转发
    std::thread::sleep(Duration::from_millis(200));
    println!("发送完成：{sent} 帧");
    Ok(())
}

/// 读取 48kHz 单声道 16bit PCM WAV（不可用格式给出明确提示）。
fn load_wav_48k_mono(path: &str) -> anyhow::Result<Vec<i16>> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| anyhow::anyhow!("打开 WAV 失败: {e}"))?;
    let spec = reader.spec();
    if spec.sample_rate != 48000
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        anyhow::bail!(
            "需要 48kHz/单声道/16bit PCM WAV（当前: {}Hz/{}ch/{}bit）\
             ——可用 mic_record 工具录制：cargo run --bin mic_record -- 30 record.wav",
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample
        );
    }
    let pcm: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
    Ok(pcm)
}
