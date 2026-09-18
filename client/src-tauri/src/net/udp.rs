//! UDP 语音通道：发送线程（PCM→Opus→UDP）与接收线程（注册 + 心跳 + 收流转发）。
//! 服务器按来源地址反查 uid 转发（server/src/udp.rs），本模块只负责收发与注册维持。
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use echoroom_protocol::messages::UdpPacket;
use echoroom_protocol::udp;
use echoroom_protocol::{FRAME_SAMPLES, HEARTBEAT_INTERVAL_MS};

/// 启动 UDP 语音收发线程。
///
/// - `stop`：与音频管线共享的停止旗标（置位后两个线程都退出）
/// - 返回 `(PCM 送入口, 语音事件出口)`
///   - PCM 送入口：采集侧发送已降噪的 960 样本块（队列满则丢弃，接收端按缺帧 PLC）
///   - 语音事件出口：播放侧读取 `(from_uid, seq, opus)`
pub fn spawn_udp_voice(
    server_addr: String,
    uid: u16,
    token: u32,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<(SyncSender<Vec<i16>>, Receiver<(u16, u32, Vec<u8>)>)> {
    let (tx_pcm, rx_pcm) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
    let (tx_voice, rx_voice) = std::sync::mpsc::sync_channel::<(u16, u32, Vec<u8>)>(256);

    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(&server_addr)?; // connect 后 send/recv 只面向服务器
    let sock_send = sock.try_clone()?;

    // 发送线程：PCM → Opus → UDP
    {
        let stop = stop.clone();
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
                        let _ = sock_send.send(&pkt);
                        seq = seq.wrapping_add(1);
                    }
                    Err(e) => eprintln!("[udp] 编码失败: {e}"),
                }
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

    Ok((tx_pcm, rx_voice))
}
