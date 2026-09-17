//! UDP 语音转发：Register 校验 / Voice 原样转发 / Heartbeat 刷新 / 超时清理。
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use echoroom_protocol::messages::UdpPacket;
use echoroom_protocol::udp;
use echoroom_protocol::UDP_TIMEOUT_MS;

use crate::room::Room;

pub fn spawn_udp_loop(port: u16, room: Arc<Mutex<Room>>) {
    std::thread::spawn(move || {
        let socket = UdpSocket::bind(("0.0.0.0", port)).expect("UDP bind 失败");
        println!("[udp] listening on 0.0.0.0:{port}");
        let mut buf = [0u8; 2048];
        loop {
            let (n, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    // Windows 特有：向已关闭的 UDP 端口发包会收到 ICMP 回报，recv 报 ConnectionReset。
                    // 属正常噪音（对端刚退出），静默继续。
                    continue;
                }
                Err(e) => {
                    eprintln!("[udp] recv 错误: {e}");
                    continue;
                }
            };
            let (uid, seq, pkt) = match udp::decode(&buf[..n]) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[udp] 丢弃异常包({from}): {e:?}");
                    continue;
                }
            };
            match pkt {
                UdpPacket::Register { token } => {
                    let mut room = room.lock().unwrap();
                    if room.validate_token(uid, token) {
                        room.set_udp(uid, from);
                        socket.send_to(&udp::encode(uid, 0, &UdpPacket::RegisterAck), from).ok();
                        println!("[udp] uid={uid} 注册 {from}");
                    } else {
                        socket.send_to(&udp::encode(uid, 0, &UdpPacket::RegisterReject), from).ok();
                        eprintln!("[udp] uid={uid} token 校验失败（{from}）");
                    }
                }
                UdpPacket::Voice { opus } => {
                    let (real_uid, targets) = {
                        let mut room = room.lock().unwrap();
                        // 必须已注册地址：按来源地址反查 uid（防伪造）
                        let real_uid = match room.member_by_addr(from) {
                            Some(u) => u,
                            None => continue,
                        };
                        room.touch(real_uid);
                        (real_uid, room.others_with_udp(real_uid))
                    };
                    // 转发：uid 用反查出的真实来源（不信任包头），保留发送者 seq
                    let out = udp::encode(real_uid, seq, &UdpPacket::Voice { opus });
                    for t in targets {
                        let _ = socket.send_to(&out, t);
                    }
                }
                UdpPacket::Heartbeat => {
                    let mut room = room.lock().unwrap();
                    if let Some(u) = room.member_by_addr(from) {
                        room.touch(u);
                    }
                }
                UdpPacket::RegisterAck | UdpPacket::RegisterReject => {
                    // 服务器不应收到这两个方向；忽略
                }
            }
        }
    });
}

pub fn spawn_cleanup_loop(room: Arc<Mutex<Room>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let expired = {
            let mut room = room.lock().unwrap();
            room.clear_udp_expired(Duration::from_millis(UDP_TIMEOUT_MS))
        };
        for uid in expired {
            println!("[udp] uid={uid} 地址映射超时清理");
        }
    });
}
