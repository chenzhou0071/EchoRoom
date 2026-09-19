//! UDP 通道：语音 / 视频分片 / 屏幕音频的收发线程与视频帧重组。
//! 服务器按来源地址反查 uid，且只转发给订阅者（server/src/udp.rs），本模块只负责收发与注册维持。
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use echoroom_protocol::messages::UdpPacket;
use echoroom_protocol::udp::{self, VF_KEYFRAME, VF_LAST};
use echoroom_protocol::{FRAME_SAMPLES, HEARTBEAT_INTERVAL_MS, VIDEO_CHUNK_DATA};

/// 视频送出一帧（前端编码产物 → UDP 分片）
pub struct VideoOut {
    pub kind: u8,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// 重组完成的一帧（UDP → 前端解码）
pub struct VideoIn {
    pub uid: u16,
    pub kind: u8,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// UDP 送入口集合
pub struct UdpTx {
    /// 语音 PCM（960 样本块）
    pub tx_pcm: SyncSender<Vec<i16>>,
    /// 视频帧
    pub tx_video: SyncSender<VideoOut>,
    /// 屏幕音频 Opus 包
    pub tx_screen_audio: SyncSender<Vec<u8>>,
}

/// UDP 出口集合
pub struct UdpRx {
    /// 语音：(from_uid, seq, opus)
    pub rx_voice: Receiver<(u16, u32, Vec<u8>)>,
    /// 视频：已重组完整帧
    pub rx_video: Receiver<VideoIn>,
    /// 屏幕音频：(from_uid, opus)
    pub rx_screen_audio: Receiver<(u16, Vec<u8>)>,
}

/// 视频帧重组器：按 frame_seq 收集分片，凑齐即产出完整帧。
/// 策略：新 frame_seq 到达即弃掉未完成的旧帧（丢片丢整帧，不重传）。
pub struct FrameAssembler {
    cur: Option<Partial>,
}

struct Partial {
    seq: u16,
    kind: u8,
    keyframe: bool,
    chunks: Vec<Option<Vec<u8>>>,
    have: usize,
}

impl FrameAssembler {
    pub fn new() -> Self {
        FrameAssembler { cur: None }
    }

    /// 送入一个分片；若凑齐当前帧则返回 `(kind, keyframe, 完整数据)`
    pub fn push(
        &mut self,
        kind: u8,
        flags: u8,
        seq: u16,
        idx: u8,
        count: u8,
        data: &[u8],
    ) -> Option<(u8, bool, Vec<u8>)> {
        if count == 0 || idx >= count {
            return None; // 非法分片
        }
        if self.cur.as_ref().map(|p| p.seq != seq).unwrap_or(true) {
            // 新帧：丢弃未完成的旧帧
            self.cur = Some(Partial {
                seq,
                kind,
                keyframe: flags & VF_KEYFRAME != 0,
                chunks: vec![None; count as usize],
                have: 0,
            });
        }
        let p = self.cur.as_mut().unwrap();
        if p.chunks.len() != count as usize {
            self.cur = None; // 同 seq 片数矛盾：整帧放弃
            return None;
        }
        let slot = &mut p.chunks[idx as usize];
        if slot.is_none() {
            *slot = Some(data.to_vec());
            p.have += 1;
        }
        if flags & VF_KEYFRAME != 0 {
            p.keyframe = true; // 任一片携带关键帧标志即生效（发送端每片都带）
        }
        if p.have == p.chunks.len() {
            let p = self.cur.take().unwrap();
            let mut out = Vec::new();
            for c in p.chunks {
                out.extend_from_slice(&c.unwrap());
            }
            return Some((p.kind, p.keyframe, out));
        }
        None
    }
}

/// 启动 UDP 收发线程组（语音 + 视频分片 + 屏幕音频）。
///
/// - `stop`：与音频管线共享的停止旗标
/// - 返回 `(送入口集合, 出口集合)`
pub fn spawn_udp(
    server_addr: String,
    uid: u16,
    token: u32,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<(UdpTx, UdpRx)> {
    let (tx_pcm, rx_pcm) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
    let (tx_voice, rx_voice) = std::sync::mpsc::sync_channel::<(u16, u32, Vec<u8>)>(256);
    let (tx_video, rx_video) = std::sync::mpsc::sync_channel::<VideoOut>(8);
    let (vod_tx, vod_rx) = std::sync::mpsc::sync_channel::<VideoIn>(8);
    let (tx_screen_audio, rx_screen_audio) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
    let (sa_tx, sa_rx) = std::sync::mpsc::sync_channel::<(u16, Vec<u8>)>(64);

    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(&server_addr)?; // connect 后 send/recv 只面向服务器
    let sock_send = sock.try_clone()?;

    // 发送线程：PCM → Opus → UDP
    {
        let stop = stop.clone();
        let sock = sock_send.try_clone()?;
        std::thread::spawn(move || {
            let mut enc = match crate::audio::opus::OpusEnc::new() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[udp] 编码器创建失败: {e}");
                    return;
                }
            };
            let mut seq: u32 = 0;
            while !stop.load(Ordering::Relaxed) {
                let pcm = match rx_pcm.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if pcm.len() != FRAME_SAMPLES {
                    continue;
                }
                match enc.encode(&pcm) {
                    Ok(opus) => {
                        let pkt = udp::encode(uid, seq, &UdpPacket::Voice { opus });
                        let _ = sock.send(&pkt);
                        seq = seq.wrapping_add(1);
                    }
                    Err(e) => eprintln!("[udp] 编码失败: {e}"),
                }
            }
        });
    }

    // 视频发送线程：帧 → 分片 → UDP（每片带 kind/flags/frame_seq/chunk_idx/count）
    {
        let stop = stop.clone();
        let sock = sock_send.try_clone()?;
        std::thread::spawn(move || {
            let mut frame_seq: u16 = 0;
            while !stop.load(Ordering::Relaxed) {
                let f = match rx_video.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if f.data.is_empty() {
                    continue;
                }
                let count = f.data.len().div_ceil(VIDEO_CHUNK_DATA);
                if count > 255 {
                    eprintln!("[udp] 帧过大（{}B，{} 片）丢弃", f.data.len(), count);
                    continue;
                }
                for (i, chunk) in f.data.chunks(VIDEO_CHUNK_DATA).enumerate() {
                    let mut flags = 0u8;
                    if f.keyframe {
                        flags |= VF_KEYFRAME;
                    }
                    if i + 1 == count {
                        flags |= VF_LAST;
                    }
                    let pkt = udp::encode(
                        uid,
                        frame_seq as u32,
                        &UdpPacket::VideoChunk {
                            kind: f.kind,
                            flags,
                            frame_seq,
                            chunk_idx: i as u8,
                            chunk_count: count as u8,
                            data: chunk.to_vec(),
                        },
                    );
                    let _ = sock.send(&pkt);
                }
                frame_seq = frame_seq.wrapping_add(1);
            }
        });
    }

    // 屏幕音频发送线程：Opus 包 → UDP
    {
        let stop = stop.clone();
        let sock = sock_send.try_clone()?;
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let opus = match rx_screen_audio.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let pkt = udp::encode(uid, 0, &UdpPacket::ScreenAudio { opus });
                let _ = sock.send(&pkt);
            }
        });
    }

    // 接收线程：注册（未确认前按心跳节奏重试）+ 心跳 + 收流
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut registered = false;
            let _ = sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token }));
            sock.set_read_timeout(Some(Duration::from_millis(100))).ok();
            let mut buf = [0u8; 2048];
            let mut last_hb = Instant::now();
            let mut va = FrameAssembler::new();
            while !stop.load(Ordering::Relaxed) {
                if last_hb.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS as u128 {
                    let pkt = if registered {
                        UdpPacket::Heartbeat
                    } else {
                        // Register 丢包兜底：注册未确认前持续重发（服务器端校验幂等）
                        UdpPacket::Register { token }
                    };
                    let _ = sock.send(&udp::encode(uid, 0, &pkt));
                    last_hb = Instant::now();
                }
                match sock.recv(&mut buf) {
                    Ok(n) => match udp::decode(&buf[..n]) {
                        Ok((from_uid, seq, UdpPacket::Voice { opus })) => {
                            let _ = tx_voice.try_send((from_uid, seq, opus)); // 满则丢（抗积压）
                        }
                        Ok((from_uid, _, UdpPacket::VideoChunk { kind, flags, frame_seq, chunk_idx, chunk_count, data })) => {
                            if let Some((k, key, full)) =
                                va.push(kind, flags, frame_seq, chunk_idx, chunk_count, &data)
                            {
                                let _ = vod_tx.try_send(VideoIn { uid: from_uid, kind: k, keyframe: key, data: full });
                            }
                        }
                        Ok((from_uid, _, UdpPacket::ScreenAudio { opus })) => {
                            let _ = sa_tx.try_send((from_uid, opus));
                        }
                        Ok((_, _, UdpPacket::RegisterAck)) => {
                            registered = true;
                            println!("[udp] 注册成功");
                        }
                        Ok((_, _, UdpPacket::RegisterReject)) => {
                            eprintln!("[udp] 注册被拒（token 校验失败）")
                        }
                        Ok(_) => {}
                        Err(_) => {} // 异常包静默丢弃
                    },
                    Err(_) => {} // 超时：继续循环（心跳节拍）
                }
            }
        });
    }

    Ok((
        UdpTx { tx_pcm, tx_video, tx_screen_audio },
        UdpRx { rx_voice, rx_video: vod_rx, rx_screen_audio: sa_rx },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_in_order() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 7, 0, 3, b"aa").is_none());
        assert!(a.push(0, 0, 7, 1, 3, b"bb").is_none());
        let (kind, key, data) = a.push(0, VF_KEYFRAME | VF_LAST, 7, 2, 3, b"cc").unwrap();
        assert_eq!((kind, key), (0, true));
        assert_eq!(data, b"aabbcc".to_vec());
    }

    #[test]
    fn assembles_out_of_order() {
        let mut a = FrameAssembler::new();
        assert!(a.push(1, VF_KEYFRAME, 9, 2, 3, b"cc").is_none());
        assert!(a.push(1, VF_KEYFRAME, 9, 0, 3, b"aa").is_none());
        let (kind, key, data) = a.push(1, VF_KEYFRAME, 9, 1, 3, b"bb").unwrap();
        assert_eq!((kind, key), (1, true));
        assert_eq!(data, b"aabbcc".to_vec());
    }

    #[test]
    fn drops_incomplete_frame_on_new_seq() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 1, 0, 3, b"x").is_none()); // seq=1 缺片
        assert!(a.push(0, 0, 2, 0, 2, b"y").is_none()); // seq=2 开始 → seq=1 被丢弃
        let (_, key, data) = a.push(0, 0, 2, 1, 2, b"z").unwrap();
        assert!(!key);
        assert_eq!(data, b"yz".to_vec());
    }

    #[test]
    fn ignores_invalid_chunk() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 1, 0, 0, b"x").is_none(), "count=0");
        assert!(a.push(0, 0, 1, 3, 3, b"x").is_none(), "idx>=count");
    }
}
