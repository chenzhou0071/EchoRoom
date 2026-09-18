//! 音频管线编排：采集（含 RNNoise 降噪）→ Opus → UDP 发送；
//! UDP 接收 → 抖动缓冲 → Opus 解码（缺帧 PLC）→ 混音软限幅 → WASAPI 播放。
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use echoroom_protocol::FRAME_SAMPLES;

use crate::audio::capture::MicCapture;
use crate::audio::denoise::Denoiser;
use crate::audio::jitter::{JitterBuffer, PopResult};
use crate::audio::mixer::MixAccumulator;
use crate::audio::opus::OpusDec;

/// 音频管线句柄：`stop` 置位后所有线程退出（Drop 不自动停止，由上层显式管理）。
pub struct AudioHandle {
    pub stop: Arc<AtomicBool>,
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
/// 1. 采集线程：MicCapture（960 块）→ Denoiser 降噪 → tx_pcm
/// 2. UDP 发送/接收线程（net::udp）
/// 3. 播放线程：drain 语音 → 每发送者抖动缓冲 → 解码/PLC → 混音 → 声卡回调
/// 4. 监视线程：stop 置位后关闭播放器
pub fn spawn_audio_pipeline(
    server_addr: String,
    uid: u16,
    token: u32,
) -> anyhow::Result<AudioHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx_pcm, rx_voice) = crate::net::udp::spawn_udp_voice(server_addr, uid, token, stop.clone())?;

    // 采集线程：MicCapture → 累积 960 → 降噪 → tx_pcm
    {
        let stop = stop.clone();
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
            let mut pending: Vec<i16> = Vec::with_capacity(1920);
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = mic.pump(&mut pending) {
                    eprintln!("[audio] 采集错误: {e:#}");
                    break;
                }
                while pending.len() >= FRAME_SAMPLES {
                    let mut block: Vec<i16> = pending.drain(..FRAME_SAMPLES).collect();
                    denoiser.process(&mut block); // 960 = 480×2 帧，整倍数合法
                    let _ = tx_pcm.try_send(block); // 队列满：丢块（接收端按缺帧 PLC）
                }
            }
        });
    }

    // 播放线程：drain 语音 → jitter → decode → mix → 声卡
    let mut jbs: HashMap<u16, Sender> = HashMap::new();
    let mut seen: HashSet<u16> = HashSet::new();
    let mut acc = MixAccumulator::new(FRAME_SAMPLES);
    let mut scratch = vec![0i16; FRAME_SAMPLES];
    let player = crate::audio::playback::spawn_player(move |out| {
        // 收流：每发送者独立抖动缓冲
        while let Ok((uid, seq, opus)) = rx_voice.try_recv() {
            if seen.insert(uid) {
                println!("[audio] 首次收到 uid={uid} 的语音");
            }
            jbs.entry(uid).or_insert_with(Sender::new).jb.insert(seq, opus);
        }
        // 出流：输出驱动拉取——每路按需取样本直到填满声卡周期。
        // 帧余量跨周期保留，消费速率严格等于输出速率（避免每段各取一帧导致帧超量消耗）。
        let mut filled = 0usize;
        while filled < out.len() {
            let want = (out.len() - filled).min(FRAME_SAMPLES);
            acc.clear();
            for s in jbs.values_mut() {
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
                    acc.add(&s.pcm[s.pos..s.pos + n]);
                    s.pos += n;
                    got += n;
                }
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

    Ok(AudioHandle { stop })
}
