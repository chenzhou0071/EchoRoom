//! EchoRoom 服务端：TCP 控制 + UDP 语音转发（单房间）。
mod room;
mod tcp;
mod udp;

use std::sync::{Arc, Mutex};

fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(echoroom_protocol::DEFAULT_PORT);
    let room = Arc::new(Mutex::new(room::Room::new()));

    // UDP 转发 + 清理线程（T6 实现；先占位）
    udp::spawn_udp_loop(port, room.clone());
    udp::spawn_cleanup_loop(room.clone());

    let listener = std::net::TcpListener::bind(("0.0.0.0", port)).expect("TCP bind 失败");
    println!("Echo server listening on 0.0.0.0:{port} (tcp+udp)");
    tcp::serve(listener, room).expect("TCP 服务异常退出");
}
