//! TCP 控制客户端：登录、公屏、成员与说话状态事件；断线自动重连（退避）。
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use echoroom_protocol::messages::TcpMessage;
use echoroom_protocol::tcp;

use crate::bridge::{Bridge, ConnState};

/// UI → 网络线程的命令
pub enum NetCmd {
    SendChat(String),
    SetSpeaking(bool),
    Shutdown,
}

/// 网络线程句柄：命令通道 + 登录身份（uid/token，语音通道后续使用）
pub struct NetHandle {
    pub tx: Sender<NetCmd>,
    pub my_uid: Arc<AtomicU16>,
    pub my_token: Arc<AtomicU32>,
}

/// 重连退避（秒）：1 → 2 → 5 → 10（封顶）
const BACKOFF_SECS: [u64; 4] = [1, 2, 5, 10];

pub fn spawn(addr: String, nickname: String, bridge: Bridge) -> NetHandle {
    let (tx, rx) = std::sync::mpsc::channel::<NetCmd>();
    let my_uid = Arc::new(AtomicU16::new(0));
    let my_token = Arc::new(AtomicU32::new(0));
    let (uid_c, tok_c) = (my_uid.clone(), my_token.clone());
    let tx_for_loop = tx.clone(); // 采集线程 VAD 的 SetSpeaking 命令经会话线程写 TCP
    std::thread::spawn(move || run_loop(addr, nickname, bridge, rx, uid_c, tok_c, tx_for_loop));
    NetHandle { tx, my_uid, my_token }
}

/// 会话结束原因
enum SessionEnd {
    /// 连接断开（服务端关闭或读错误）：从最短退避重新开始
    Disconnected,
    /// 被服务器拒绝（如房间满）：退避递增，避免高频重试
    Rejected,
    /// 收到 Shutdown 命令：线程退出
    Shutdown,
}

fn run_loop(
    addr: String,
    nickname: String,
    bridge: Bridge,
    rx: Receiver<NetCmd>,
    my_uid: Arc<AtomicU16>,
    my_token: Arc<AtomicU32>,
    tx: Sender<NetCmd>,
) {
    let mut attempt = 0usize;
    loop {
        bridge.emit_conn(if attempt == 0 { ConnState::Connecting } else { ConnState::Reconnecting });
        let result = match TcpStream::connect(&addr) {
            Ok(mut stream) => run_session(&mut stream, &addr, &nickname, &bridge, &rx, &my_uid, &my_token, &tx),
            Err(e) => {
                eprintln!("[net] 连接失败: {e}");
                Err(e)
            }
        };
        match result {
            Ok(SessionEnd::Shutdown) => return,
            Ok(SessionEnd::Disconnected) => {
                attempt = 0;
                bridge.emit_conn(ConnState::Reconnecting);
            }
            Ok(SessionEnd::Rejected) => bridge.emit_conn(ConnState::Reconnecting),
            Err(e) => {
                eprintln!("[net] 会话结束: {e}");
                bridge.emit_conn(ConnState::Reconnecting);
            }
        }
        let wait = BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)];
        attempt += 1;
        if wait_or_shutdown(&rx, Duration::from_secs(wait)) {
            return;
        }
    }
}

/// 等待 `total` 时长，期间以 100ms 粒度轮询 Shutdown。返回 true = 收到 Shutdown。
fn wait_or_shutdown(rx: &Receiver<NetCmd>, total: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < total {
        match rx.try_recv() {
            Ok(NetCmd::Shutdown) => return true,
            _ => {} // 断线期间的发送/说话命令直接丢弃
        }
        std::thread::sleep(step);
        waited += step;
    }
    false
}

fn run_session(
    stream: &mut TcpStream,
    addr: &str,
    nickname: &str,
    bridge: &Bridge,
    rx: &Receiver<NetCmd>,
    my_uid: &AtomicU16,
    my_token: &AtomicU32,
    tx: &Sender<NetCmd>,
) -> std::io::Result<SessionEnd> {
    stream.write_all(&tcp::encode(&TcpMessage::Login { nickname: nickname.to_string() }))?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        // 发：轮询 UI 命令写出
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                NetCmd::SendChat(text) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Chat { uid: 0, text }))?;
                }
                NetCmd::SetSpeaking(on) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Speaking { uid: 0, on }))?;
                }
                NetCmd::Shutdown => return Ok(SessionEnd::Shutdown),
            }
        }
        // 收：50ms 超时轮询（超时 = 回去处理命令）
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(SessionEnd::Disconnected), // 服务端关闭
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
        // 解析（一次可能出多条，或只攒到半条）
        loop {
            match tcp::try_decode(&buf) {
                Ok(Some((msg, used))) => {
                    buf.drain(..used);
                    match msg {
                        TcpMessage::LoginOk { uid, token, members } => {
                            my_uid.store(uid, Ordering::Relaxed);
                            my_token.store(token, Ordering::Relaxed);
                            bridge.emit_conn(ConnState::Connected);
                            // 服务器返回的列表不含自己：补上后整表发给 UI（UI 端无需特判自己）
                            let mut all = members;
                            all.push((uid, nickname.to_string()));
                            bridge.emit_member_list(all);
                            // 启动音频链路（麦克风/编码/播放/VAD 上报；失败不影响文字聊天）
                            crate::bridge::start_audio(&bridge.app, uid, token, addr.to_string(), tx.clone());
                        }
                        TcpMessage::LoginReject { reason } => {
                            bridge.emit_conn(ConnState::Rejected(reason));
                            return Ok(SessionEnd::Rejected);
                        }
                        TcpMessage::MemberJoin { uid, nickname } => {
                            bridge.emit_member_join(uid, nickname)
                        }
                        TcpMessage::MemberLeave { uid } => bridge.emit_member_leave(uid),
                        TcpMessage::Chat { uid, text } => bridge.emit_chat(uid, text),
                        TcpMessage::Speaking { uid, on } => bridge.emit_speaking(uid, on),
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    eprintln!("[net] 协议解码错误: {e:?}");
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "协议解码错误"));
                }
            }
        }
    }
}
