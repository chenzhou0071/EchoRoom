//! 测试工具：把一段 WAV 录音用真实 Opus 链路按 20ms/帧实时发送到服务器，
//! 模拟"一个正在说话的真实客户端"（供真人收听验证播放链路质量）。
//! 用法：cargo run --bin send_wav -- <服务器addr> <wav路径> [循环遍数，默认 1]
//! WAV 要求 48kHz 单声道 16bit PCM（可直接用 mic_record 录制）。
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use echoroom_client_lib::audio::opus::OpusEnc;
use echoroom_protocol::messages::{TcpMessage, UdpPacket};
use echoroom_protocol::{tcp, udp, FRAME_SAMPLES, HEARTBEAT_INTERVAL_MS};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let wav_path = match args.get(2) {
        Some(p) => p.clone(),
        None => {
            println!("用法: cargo run --bin send_wav -- <服务器addr> <wav路径> [循环遍数，默认 1]");
            return Ok(());
        }
    };
    let loops: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let pcm = load_wav_48k_mono(&wav_path)?;
    let secs = pcm.len() as f64 / 48000.0;
    println!("已读取 {wav_path}（{secs:.1}s）");

    // ---- TCP 登录拿 uid/token ----
    let mut stream = TcpStream::connect(&addr)?;
    stream.write_all(&tcp::encode(&TcpMessage::Login { nickname: "wavbot".into() }))?;
    stream.set_read_timeout(Some(Duration::from_millis(20)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let (mut uid, mut token) = (0u16, 0u32);
    let deadline = Instant::now() + Duration::from_secs(5);
    while uid == 0 {
        if Instant::now() > deadline {
            anyhow::bail!("登录超时（服务器无响应）");
        }
        match stream.read(&mut chunk) {
            Ok(0) => anyhow::bail!("服务器关闭了连接"),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                while let Ok(Some((msg, used))) = tcp::try_decode(&buf) {
                    buf.drain(..used);
                    if let TcpMessage::LoginOk { uid: u, token: t, .. } = msg {
                        uid = u;
                        token = t;
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
    sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token }))?;

    // ---- 预编码全部帧 ----
    let mut enc = OpusEnc::new().map_err(|e| anyhow::anyhow!("编码器创建失败: {e}"))?;
    let mut frames: Vec<Vec<u8>> = Vec::new();
    for chunk in pcm.chunks_exact(FRAME_SAMPLES) {
        match enc.encode(chunk) {
            Ok(o) => frames.push(o),
            Err(e) => eprintln!("编码失败（跳过）: {e}"),
        }
    }
    if frames.is_empty() {
        anyhow::bail!("没有可发送的完整帧");
    }

    // ---- 实时发送（绝对时间锚定 20ms/帧，防时钟漂移）----
    println!(
        "开始发送：{} 帧 × {loops} 遍（约 {:.1}s）",
        frames.len(),
        frames.len() as f64 * 0.02 * loops as f64
    );
    let t0 = Instant::now();
    let mut sent: u64 = 0;
    let mut seq: u32 = 0;
    let mut last_hb = Instant::now();
    for _ in 0..loops {
        for f in &frames {
            let target = t0 + Duration::from_millis(sent * 20);
            let now = Instant::now();
            if target > now {
                std::thread::sleep(target - now);
            }
            let pkt = udp::encode(uid, seq, &UdpPacket::Voice { opus: f.clone() });
            sock.send(&pkt)?;
            seq = seq.wrapping_add(1);
            sent += 1;
            if last_hb.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS as u128 {
                let _ = sock.send(&udp::encode(uid, 0, &UdpPacket::Heartbeat));
                last_hb = Instant::now();
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
