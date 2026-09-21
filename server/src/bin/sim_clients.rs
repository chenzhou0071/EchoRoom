//! 模拟客户端：注册/登录 + 定时公屏 + UDP 语音模拟 + 收包统计。
//! 用法：sim_clients [addr] [n] [seconds] [invite]
//! 带 invite：先尝试注册（同名已存在则自动回退登录）；不带：直接登录
//! （要求账号已注册过，否则认证被拒后退出）。
use echoroom_protocol::messages::{TcpMessage, UdpPacket};
use echoroom_protocol::{tcp, udp, HEARTBEAT_INTERVAL_MS};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let invite = args.get(4).cloned();

    let mut handles = Vec::new();
    for i in 0..n {
        let addr = addr.clone();
        let invite = invite.clone();
        handles.push(std::thread::spawn(move || sim_one(&addr, i, seconds, invite)));
    }
    for h in handles {
        let _ = h.join();
    }
}

fn sim_one(addr: &str, index: usize, seconds: u64, invite: Option<String>) {
    let name = format!("sim{index}");
    let account = name.clone(); // 账号需符合 [A-Za-z0-9][A-Za-z0-9_]{2,19}，sim0/sim1… 合法
    let password = format!("pw-{index}-123456");
    let mut stream = TcpStream::connect(addr).expect("连接失败");
    let first = match &invite {
        Some(code) => TcpMessage::Register {
            account: account.clone(),
            password: password.clone(),
            invite: code.clone(),
        },
        None => TcpMessage::Login { account: account.clone(), password: password.clone() },
    };
    stream.write_all(&tcp::encode(&first)).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap(); // 循环节拍：20ms

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut fallback_done = false; // 注册撞名后是否已回退登录（防死循环）

    // UDP 状态（LoginOk 后建立）
    let mut udp_sock: Option<UdpSocket> = None;
    let mut my_uid: u16 = 0;
    let mut seq: u32 = 0;
    let mut sent_voice: u64 = 0;
    let mut recv_stats: HashMap<u16, u64> = HashMap::new();
    let mut ubuf = [0u8; 2048];

    let start = Instant::now();
    let mut last_chat = Instant::now();
    let mut last_voice = Instant::now();
    let mut last_heartbeat = Instant::now();

    'main: loop {
        if start.elapsed().as_secs() >= seconds {
            break;
        }
        // ---- 定时公屏（每秒）----
        if last_chat.elapsed().as_millis() >= 1000 {
            let text = format!("{name} 报到 {}", start.elapsed().as_secs());
            stream.write_all(&tcp::encode(&TcpMessage::Chat { uid: 0, text })).unwrap();
            last_chat = Instant::now();
        }
        // ---- UDP：语音 20ms 一包 + 心跳 ----
        if let Some(sock) = &udp_sock {
            if last_voice.elapsed().as_millis() >= 20 {
                let payload: Vec<u8> = (0..60).map(|i| (i as u32 + seq) as u8).collect();
                sock.send(&udp::encode(my_uid, seq, &UdpPacket::Voice { opus: payload })).ok();
                seq = seq.wrapping_add(1);
                sent_voice += 1;
                last_voice = Instant::now();
            }
            if last_heartbeat.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS as u128 {
                sock.send(&udp::encode(my_uid, 0, &UdpPacket::Heartbeat)).ok();
                last_heartbeat = Instant::now();
            }
            // 非阻塞收包统计
            loop {
                match sock.recv(&mut ubuf) {
                    Ok(n) => match udp::decode(&ubuf[..n]) {
                        Ok((src_uid, _, UdpPacket::Voice { .. })) => {
                            *recv_stats.entry(src_uid).or_insert(0) += 1;
                        }
                        Ok((_, _, UdpPacket::RegisterAck)) => println!("[{name}] UDP 注册确认 (RegisterAck)"),
                        _ => {}
                    },
                    Err(_) => break, // 无更多包（WouldBlock）
                }
            }
        }
        // ---- TCP 读（20ms 超时）----
        if let Ok(n) = stream.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            while let Ok(Some((msg, used))) = tcp::try_decode(&buf) {
                buf.drain(..used);
                match msg {
                    TcpMessage::LoginOk { uid, udp_token, .. } => {
                        my_uid = uid;
                        println!("[{name}] LoginOk uid={uid}");
                        let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
                        sock.connect(addr).unwrap();
                        sock.set_nonblocking(true).unwrap();
                        sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token: udp_token })).unwrap();
                        udp_sock = Some(sock);
                    }
                    TcpMessage::AuthReject { reason } => {
                        if invite.is_some() && reason == "账号已存在" && !fallback_done {
                            fallback_done = true;
                            println!("[{name}] 账号已存在，回退登录");
                            let login = TcpMessage::Login { account: account.clone(), password: password.clone() };
                            stream.write_all(&tcp::encode(&login)).unwrap();
                        } else {
                            eprintln!("[{name}] 认证失败：{reason}");
                            break 'main;
                        }
                    }
                    TcpMessage::Chat { uid, text } => println!("[{name}] 收到 uid={uid}: {text}"),
                    TcpMessage::MemberJoin { uid, nickname, .. } => println!("[{name}] +{nickname}(uid={uid})"),
                    TcpMessage::MemberLeave { uid } => println!("[{name}] -uid={uid}"),
                    other => println!("[{name}] {other:?}"),
                }
            }
        }
    }
    // ---- 退出统计 ----
    let mut stats: Vec<(u16, u64)> = recv_stats.into_iter().collect();
    stats.sort_unstable();
    for (src, count) in &stats {
        println!("[{name}] 来自 uid={src} 的语音 {count} 包");
    }
    println!("[{name}] 共发出语音 {sent_voice} 包，退出");
}
