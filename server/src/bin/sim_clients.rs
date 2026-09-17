//! 模拟客户端：登录 + 定时公屏 + 打印收到的一切（T6 扩展 UDP）。
use echoroom_protocol::messages::TcpMessage;
use echoroom_protocol::tcp;
use std::io::{Read, Write};
use std::net::TcpStream;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    let mut handles = Vec::new();
    for i in 0..n {
        let addr = addr.clone();
        handles.push(std::thread::spawn(move || sim_one(&addr, i, seconds)));
    }
    for h in handles {
        let _ = h.join();
    }
}

fn sim_one(addr: &str, index: usize, seconds: u64) {
    let name = format!("sim{index}");
    let mut stream = TcpStream::connect(addr).expect("连接失败");
    stream.write_all(&tcp::encode(&TcpMessage::Login { nickname: name.clone() })).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let start = std::time::Instant::now();
    let mut last_chat = std::time::Instant::now();
    loop {
        if start.elapsed().as_secs() >= seconds {
            break;
        }
        // 定时发公屏
        if last_chat.elapsed().as_millis() >= 1000 {
            let text = format!("{name} 报到 {}", start.elapsed().as_secs());
            stream.write_all(&tcp::encode(&TcpMessage::Chat { uid: 0, text })).unwrap();
            last_chat = std::time::Instant::now();
        }
        // 读（非阻塞式：设短超时）
        stream.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
        if let Ok(n) = stream.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Ok(Some((msg, used))) = tcp::try_decode(&buf) {
                buf.drain(..used);
                match msg {
                    TcpMessage::Chat { uid, text } => println!("[{name}] 收到 uid={uid}: {text}"),
                    TcpMessage::MemberJoin { uid, nickname } => println!("[{name}] +{nickname}(uid={uid})"),
                    TcpMessage::MemberLeave { uid } => println!("[{name}] -uid={uid}"),
                    other => println!("[{name}] {other:?}"),
                }
            }
        }
    }
    println!("[{name}] 退出");
}
