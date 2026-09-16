//! EchoRoom 服务端：TCP 控制 + UDP 语音转发（单房间）。
fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(echoroom_protocol::DEFAULT_PORT);
    println!("echoroom-server listening on port {port} (tcp+udp)");
    // T4 起填充实际逻辑
}
