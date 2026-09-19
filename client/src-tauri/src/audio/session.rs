//! 音频管线编排：采集（含 RNNoise 降噪）→ Opus → UDP 发送；
//! UDP 接收 → 抖动缓冲 → Opus 解码（缺帧 PLC）→ 混音软限幅 → WASAPI 播放。
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use echoroom_protocol::FRAME_SAMPLES;

use crate::audio::capture::MicCapture;
use crate::audio::denoise::Denoiser;
use crate::audio::jitter::{JitterBuffer, PopResult};
use crate::audio::mixer::MixAccumulator;
use crate::audio::opus::OpusDec;
use crate::audio::vad::SpeakingDetector;
use crate::net::tcp::NetCmd;

/// 音量/静音共享态：bridge（写）+ 网络线程（写 uid_names）+ 音频线程（读）三方共享。
#[derive(Clone)]
pub struct SharedAudio {
    /// 自己的采集增益（f32 bits 存于 AtomicU32）
    pub self_gain: Arc<AtomicU32>,
    /// 自己的静音状态
    pub self_muted: Arc<AtomicBool>,
    /// 对他人的播放增益（按昵称）
    pub peer_gains: Arc<std::sync::Mutex<HashMap<String, f32>>>,
    /// uid → 昵称（网络线程维护；播放端按 uid 查增益）
    pub uid_names: Arc<std::sync::Mutex<HashMap<u16, String>>>,
    /// 本端流位图镜像（bit0 = 投屏、bit1 = 摄像头；重连补报用）
    pub my_streams: Arc<AtomicU8>,
    /// 当前订阅人数（屏幕音频推流门控；LoginOk 归零）
    pub viewer_count: Arc<AtomicU16>,
}

impl SharedAudio {
    pub fn new(self_gain: f32, muted: bool, peer_gains: HashMap<String, f32>) -> Self {
        SharedAudio {
            self_gain: Arc::new(AtomicU32::new(self_gain.to_bits())),
            self_muted: Arc::new(AtomicBool::new(muted)),
            peer_gains: Arc::new(std::sync::Mutex::new(peer_gains)),
            uid_names: Arc::new(std::sync::Mutex::new(HashMap::new())),
            my_streams: Arc::new(AtomicU8::new(0)),
            viewer_count: Arc::new(AtomicU16::new(0)),
        }
    }
}

/// 本块要对外发送的 speaking 上报决策（None = 不发送）。
/// 语义：进入静音瞬间强制上报“停止说话”；静音期间抑制“开始说话”；
/// 解除静音瞬间补报 VAD 当前真实状态（静音期间 VAD 照跑，状态可能已翻转）。
fn speaking_report(
    flip: Option<bool>,
    muted_now: bool,
    was_muted: bool,
    speaking_now: bool,
) -> Option<bool> {
    if !was_muted && muted_now {
        return Some(false);
    }
    if was_muted && !muted_now {
        return Some(speaking_now);
    }
    match flip {
        Some(on) if !muted_now || !on => Some(on),
        _ => None,
    }
}

/// 音频管线句柄：`stop` 置位后所有线程退出（Drop 不自动停止，由上层显式管理）。
pub struct AudioHandle {
    pub stop: Arc<AtomicBool>,
    /// 视频帧送入口（前端 send_video_frame 命令写入；UDP 线程消费）
    pub video_tx: std::sync::mpsc::SyncSender<crate::net::udp::VideoOut>,
    /// 视频帧接收端（观看时由 bridge 的 watch 线程消费）
    pub video_rx: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<crate::net::udp::VideoIn>>>,
    /// 屏幕 PCM 送入口（屏幕捕获线程写入；编码线程消费）
    pub screen_pcm_tx: std::sync::mpsc::SyncSender<Vec<i16>>,
}

/// 断流判定：连续 PLC 上限（5 × 40ms 等待 ≈ 200ms 无语音数据即静音，
/// 否则对方退出/断流后播放端会无限 PLC 外推形成持续噪声）。新帧到达自动恢复。
const MAX_PLC_STREAK: u32 = 5;

/// 单个发送者的播放状态。
/// 解码器必须每路独立：Opus 解码器有内部状态（PLC 外推），多流共用一个会互相污染。
struct Sender {
    jb: JitterBuffer,
    dec: Option<OpusDec>,
    /// 当前解码帧的余量：跨声卡周期消费（输出驱动拉取，消费速率 = 输出速率）
    pcm: Vec<i16>,
    pos: usize,
    plc_streak: u32,
}

impl Sender {
    fn new() -> Sender {
        Sender {
            jb: JitterBuffer::new(3, 8),
            dec: OpusDec::new().ok(),
            pcm: Vec::new(),
            pos: 0,
            plc_streak: 0,
        }
    }
}

/// 启动完整音频管线（登录成功后调用；失败不影响文字聊天）。
///
/// 线程：
/// 1. 采集线程：MicCapture（960 块）→ Denoiser 降噪 → VAD 说话状态上报 → tx_pcm
/// 2. UDP 发送/接收线程（net::udp）
/// 3. 播放线程：drain 语音 → 每发送者抖动缓冲 → 解码/PLC → 混音 → 声卡回调
/// 4. 监视线程：stop 置位后关闭播放器
pub fn spawn_audio_pipeline(
    server_addr: String,
    uid: u16,
    token: u32,
    tcp_tx: std::sync::mpsc::Sender<NetCmd>,
    shared: SharedAudio,
) -> anyhow::Result<AudioHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let (udp_tx, udp_rx) = crate::net::udp::spawn_udp(server_addr, uid, token, stop.clone())?;
    let tx_pcm = udp_tx.tx_pcm.clone();
    let rx_voice = udp_rx.rx_voice;

    // 采集线程：MicCapture → 累积 960 → 降噪 →（增益）→ VAD → tx_pcm
    {
        let stop = stop.clone();
        let self_gain = shared.self_gain.clone();
        let self_muted = shared.self_muted.clone();
        std::thread::spawn(move || {
            let mic = match MicCapture::open() {
                Ok(m) => m,
                Err(e) => {
                    // 不发 conn 事件：TCP 连接实际正常，此处仅音频不可用
                    eprintln!("[audio] 麦克风不可用: {e:#}");
                    return;
                }
            };
            let mut denoiser = Denoiser::new();
            // 阈值实测自降噪后信号（底噪残留 ≈ 17、语音 ≥ 200）：进入 50 / 退出 25
            let mut detector = SpeakingDetector::new(50.0, 25.0, Duration::from_millis(400));
            let mut pending: Vec<i16> = Vec::with_capacity(1920);
            let mut was_muted = false;
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = mic.pump(&mut pending) {
                    eprintln!("[audio] 采集错误: {e:#}");
                    break;
                }
                while pending.len() >= FRAME_SAMPLES {
                    let mut block: Vec<i16> = pending.drain(..FRAME_SAMPLES).collect();
                    denoiser.process(&mut block); // 960 = 480×2 帧，整倍数合法
                    // 采集增益：denoise 后、VAD 前（增益调大 → VAD 更灵敏，符合直觉）
                    let g = f32::from_bits(self_gain.load(Ordering::Relaxed));
                    if (g - 1.0).abs() > 1e-6 {
                        for s in block.iter_mut() {
                            *s = ((*s as f32) * g).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                        }
                    }
                    let rms = (block.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                        / block.len() as f64)
                        .sqrt();
                    // VAD 静音期间照跑：状态机连续，解除静音后状态立即正确
                    let flip = detector.update(rms, std::time::Instant::now());
                    let now_muted = self_muted.load(Ordering::Relaxed);
                    if let Some(on) = speaking_report(flip, now_muted, was_muted, detector.speaking()) {
                        let _ = tcp_tx.send(NetCmd::SetSpeaking(on)); // 无界队列：不阻塞
                    }
                    was_muted = now_muted;
                    // 静音不发包：仅说话状态（含 400ms 保持）且未静音期间发送；接收端 PLC 超时静默兜底
                    if !now_muted && detector.speaking() {
                        let _ = tx_pcm.try_send(block); // 队列满：丢块（接收端按缺帧 PLC）
                    }
                }
            }
        });
    }

    // 屏幕音频编码线程：屏幕 PCM（1920 交错样本）→ Opus → UDP（无人观看时丢弃不编码）
    let (screen_pcm_tx, screen_pcm_rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
    {
        let stop = stop.clone();
        let viewer_count = shared.viewer_count.clone();
        let tx_screen_audio = udp_tx.tx_screen_audio.clone();
        std::thread::spawn(move || {
            let mut enc = match crate::audio::opus::OpusEncStereo::new() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[audio] 屏幕音频编码器创建失败: {e}");
                    return;
                }
            };
            while !stop.load(Ordering::Relaxed) {
                let pcm = match screen_pcm_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if pcm.len() != FRAME_SAMPLES * 2 {
                    continue;
                }
                if viewer_count.load(Ordering::Relaxed) == 0 {
                    continue; // 0 观众：不推流
                }
                match enc.encode(&pcm) {
                    Ok(opus) => {
                        let _ = tx_screen_audio.try_send(opus);
                    }
                    Err(e) => eprintln!("[audio] 屏幕音频编码失败: {e}"),
                }
            }
        });
    }

    // 播放线程：drain 语音 → jitter → decode → mix → 声卡
    let mut jbs: HashMap<u16, Sender> = HashMap::new();
    let mut seen: HashSet<u16> = HashSet::new();
    let mut acc = MixAccumulator::new(FRAME_SAMPLES);
    let mut scratch = vec![0i16; FRAME_SAMPLES];
    // 屏幕音频：解码器 + 单声道降混队列（每播放周期消费）
    let mut scr_dec: Option<crate::audio::opus::OpusDecStereo> = None;
    let mut scr_buf: std::collections::VecDeque<i16> = std::collections::VecDeque::new();
    let peer_gains = shared.peer_gains.clone();
    let uid_names = shared.uid_names.clone();
    let player = crate::audio::playback::spawn_player(move |out| {
        // 收流：每发送者独立抖动缓冲
        while let Ok((uid, seq, opus)) = rx_voice.try_recv() {
            if seen.insert(uid) {
                println!("[audio] 首次收到 uid={uid} 的语音");
            }
            jbs.entry(uid).or_insert_with(Sender::new).jb.insert(seq, opus);
        }
        // 收屏幕音频：解码 → 降混单声道 → 入队（上限 2s 防积压）
        while let Ok((_uid, opus)) = udp_rx.rx_screen_audio.try_recv() {
            if scr_dec.is_none() {
                scr_dec = crate::audio::opus::OpusDecStereo::new().ok();
            }
            if let Some(dec) = scr_dec.as_mut() {
                let mut st = vec![0i16; FRAME_SAMPLES * 2];
                if dec.decode(Some(&opus), &mut st).is_ok() {
                    for i in 0..FRAME_SAMPLES {
                        scr_buf.push_back(((st[2 * i] as i32 + st[2 * i + 1] as i32) / 2) as i16);
                    }
                    while scr_buf.len() > FRAME_SAMPLES * 100 {
                        scr_buf.pop_front();
                    }
                }
            }
        }
        // 出流：输出驱动拉取——每路按需取样本直到填满声卡周期。
        // 帧余量跨周期保留，消费速率严格等于输出速率（避免每段各取一帧导致帧超量消耗）。
        let mut filled = 0usize;
        while filled < out.len() {
            let want = (out.len() - filled).min(FRAME_SAMPLES);
            // 每段快照一次 uid → gain（避免逐路重复查昵称）；无自定义增益时为空表走默认 1.0
            let gain_snapshot: HashMap<u16, f32> = {
                let names = uid_names.lock().unwrap();
                let gains = peer_gains.lock().unwrap();
                if gains.is_empty() {
                    HashMap::new()
                } else {
                    names
                        .iter()
                        .map(|(u, n)| (*u, gains.get(n).copied().unwrap_or(1.0)))
                        .collect()
                }
            };
            acc.clear();
            for (uid, s) in jbs.iter_mut() {
                let Some(dec) = s.dec.as_mut() else { continue };
                let mut got = 0usize;
                while got < want {
                    if s.pos >= s.pcm.len() {
                        // 余量耗尽：从抖动缓冲取下一帧
                        match s.jb.pop() {
                            PopResult::Ready(Some(f)) => {
                                s.pcm.resize(FRAME_SAMPLES, 0);
                                if dec.decode(Some(&f), &mut s.pcm).is_ok() {
                                    s.pos = 0;
                                    s.plc_streak = 0;
                                } else {
                                    // 解码失败：本帧按静音（继续取后续帧）
                                    s.pcm.clear();
                                    s.pos = 0;
                                    break;
                                }
                            }
                            PopResult::Ready(None) => {
                                if s.plc_streak >= MAX_PLC_STREAK {
                                    break; // 连续缺帧达上限：对方断流，静音等待新帧
                                }
                                s.pcm.resize(FRAME_SAMPLES, 0);
                                if dec.decode(None, &mut s.pcm).is_ok() {
                                    s.pos = 0;
                                    s.plc_streak += 1;
                                } else {
                                    s.pcm.clear();
                                    s.pos = 0;
                                    break;
                                }
                            }
                            PopResult::NotYet => break,
                        }
                    }
                    let n = (want - got).min(s.pcm.len() - s.pos);
                    let g = gain_snapshot.get(uid).copied().unwrap_or(1.0);
                    acc.add_scaled(&s.pcm[s.pos..s.pos + n], g);
                    s.pos += n;
                    got += n;
                }
            }
            // 屏幕声音叠加（固定 1.0 增益；所有输出段共用同一队列）
            let avail = scr_buf.len().min(want);
            if avail > 0 {
                let seg: Vec<i16> = scr_buf.drain(..avail).collect();
                acc.add_scaled(&seg, 1.0);
            }
            acc.finalize(&mut scratch[..want]);
            out[filled..filled + want].copy_from_slice(&scratch[..want]);
            filled += want;
        }
    })?;

    // 监视线程：总 stop 置位后关闭播放器（PlayerHandle 由本线程持有保活）
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
            }
            player.stop.store(true, Ordering::Relaxed);
        });
    }

    Ok(AudioHandle {
        stop,
        video_tx: udp_tx.tx_video,
        video_rx: Arc::new(std::sync::Mutex::new(udp_rx.rx_video)),
        screen_pcm_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaking_report_mutes_and_realigns() {
        // 正常翻转透传
        assert_eq!(speaking_report(Some(true), false, false, true), Some(true));
        assert_eq!(speaking_report(Some(false), false, false, false), Some(false));
        // 进入静音瞬间：强制上报停止说话（清别人卡片蓝框）
        assert_eq!(speaking_report(None, true, false, true), Some(false));
        // 静音期间：翻入说话被抑制；翻出静音可上报（无害）
        assert_eq!(speaking_report(Some(true), true, true, true), None);
        assert_eq!(speaking_report(Some(false), true, true, false), Some(false));
        // 解除静音：补报 VAD 当前状态（可能在静音期间已翻转为说话）
        assert_eq!(speaking_report(None, false, true, true), Some(true));
        assert_eq!(speaking_report(None, false, true, false), Some(false));
    }
}
