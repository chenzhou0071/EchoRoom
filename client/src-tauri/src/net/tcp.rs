//! TCP 控制客户端：注册/登录/自动登录、公屏、成员与说话状态事件；断线自动重连（退避）。
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use echoroom_protocol::messages::{MemberInfo, TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
use echoroom_protocol::tcp;

use crate::bridge::{Bridge, ConnState};

/// 认证方式（连接首消息三选一；登录成功后的断线重连自动走 Resume）
#[derive(Clone, Debug)]
pub enum AuthMode {
    Login { account: String, password: String },
    Register { account: String, password: String, invite: String },
    /// 自动登录（有 auth_token 时的默认方式）
    Resume { auth_token: String },
}

/// UI → 网络线程的命令
pub enum NetCmd {
    SendChat(String),
    SetSpeaking(bool),
    SetMuted(bool),
    /// 订阅某人（None = 取消订阅）
    Subscribe(Option<u16>),
    /// 请求目标发关键帧
    RequestKeyframe(u16),
    /// 上报本端某路流开/停（kind = STREAM_*）
    SetStream { kind: u8, on: bool },
    /// 更新资料（昵称必填；avatar = None 表示不改头像）
    SetProfile { nickname: String, avatar: Option<Vec<u8>> },
    /// 请求某人的头像（懒加载）
    AvatarRequest(u16),
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

pub fn spawn(
    addr: String,
    mode: AuthMode,
    bridge: Bridge,
    shared: crate::audio::session::SharedAudio,
) -> NetHandle {
    let (tx, rx) = std::sync::mpsc::channel::<NetCmd>();
    let my_uid = Arc::new(AtomicU16::new(0));
    let my_token = Arc::new(AtomicU32::new(0));
    let (uid_c, tok_c) = (my_uid.clone(), my_token.clone());
    let tx_for_loop = tx.clone(); // 采集线程 VAD 的 SetSpeaking 命令经会话线程写 TCP
    std::thread::spawn(move || {
        run_loop(addr, mode, bridge, rx, uid_c, tok_c, tx_for_loop, shared)
    });
    NetHandle { tx, my_uid, my_token }
}

/// 会话结束原因
enum SessionEnd {
    /// 连接断开（服务端关闭或读错误）：从最短退避重新开始
    Disconnected,
    /// 认证失败（账号密码错误 / Resume 过期 / 房间满）：停止重连，交还 UI
    AuthFailed(String),
    /// 收到 Shutdown 命令：线程退出
    Shutdown,
}

/// 本轮连接的首消息：有 auth_token 一律走 Resume（登录成功后的重连同路径）
fn first_message(mode: &AuthMode, token: Option<&str>) -> TcpMessage {
    match token {
        Some(t) => TcpMessage::Resume { auth_token: t.to_string() },
        None => match mode {
            AuthMode::Login { account, password } => {
                TcpMessage::Login { account: account.clone(), password: password.clone() }
            }
            AuthMode::Register { account, password, invite } => TcpMessage::Register {
                account: account.clone(),
                password: password.clone(),
                invite: invite.clone(),
            },
            AuthMode::Resume { auth_token } => TcpMessage::Resume { auth_token: auth_token.clone() },
        },
    }
}

fn run_loop(
    addr: String,
    mode: AuthMode,
    bridge: Bridge,
    rx: Receiver<NetCmd>,
    my_uid: Arc<AtomicU16>,
    my_token: Arc<AtomicU32>,
    tx: Sender<NetCmd>,
    shared: crate::audio::session::SharedAudio,
) {
    let mut attempt = 0usize;
    // 自动登录凭证：初始来自 Resume 模式；Login/Register 成功后由 LoginOk 滚动更新
    let mut auth_token: Option<String> = match &mode {
        AuthMode::Resume { auth_token } => Some(auth_token.clone()),
        _ => None,
    };
    loop {
        bridge.emit_conn(if attempt == 0 { ConnState::Connecting } else { ConnState::Reconnecting });
        let used_resume = auth_token.is_some();
        let first = first_message(&mode, auth_token.as_deref());
        let result = match TcpStream::connect(&addr) {
            Ok(mut stream) => run_session(
                &mut stream, &addr, &first, &mut auth_token, &bridge, &rx, &my_uid, &my_token, &tx,
                &shared,
            ),
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
            Ok(SessionEnd::AuthFailed(reason)) => {
                eprintln!("[net] 认证失败: {reason}");
                if used_resume {
                    // Resume 失效（凭证被清 / 服务器换库）：清 token 回登录页
                    bridge.clear_auth_token();
                }
                bridge.emit_conn(ConnState::Rejected(reason.clone()));
                bridge.emit_auth_fail(reason);
                return; // 不自动重试，交还 UI 决定下一步
            }
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
    first: &TcpMessage,
    auth_token: &mut Option<String>,
    bridge: &Bridge,
    rx: &Receiver<NetCmd>,
    my_uid: &AtomicU16,
    my_token: &AtomicU32,
    tx: &Sender<NetCmd>,
    shared: &crate::audio::session::SharedAudio,
) -> std::io::Result<SessionEnd> {
    stream.write_all(&tcp::encode(first))?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut authenticated = false; // AuthReject 双语义：未认证 = 断开；已认证 = 资料错误提示
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
                NetCmd::SetMuted(on) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on }))?;
                }
                NetCmd::Subscribe(Some(target)) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Subscribe { uid: 0, target }))?;
                }
                NetCmd::Subscribe(None) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Unsubscribe { uid: 0 }))?;
                }
                NetCmd::RequestKeyframe(target) => {
                    stream.write_all(&tcp::encode(&TcpMessage::RequestKeyframe { uid: 0, target }))?;
                }
                NetCmd::SetStream { kind, on } => {
                    stream.write_all(&tcp::encode(&TcpMessage::StreamState { uid: 0, kind, on }))?;
                }
                NetCmd::SetProfile { nickname, avatar } => {
                    stream.write_all(&tcp::encode(&TcpMessage::SetProfile { nickname, avatar }))?;
                }
                NetCmd::AvatarRequest(uid) => {
                    stream.write_all(&tcp::encode(&TcpMessage::AvatarRequest { uid }))?;
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
                        TcpMessage::LoginOk { uid, udp_token, auth_token: fresh_token, members } => {
                            authenticated = true;
                            my_uid.store(uid, Ordering::Relaxed);
                            my_token.store(udp_token, Ordering::Relaxed);
                            // 凭证滚动：Login/Register 签发新 token，Resume 换取新 token
                            *auth_token = Some(fresh_token.clone());
                            bridge.save_auth_token(fresh_token);
                            bridge.emit_conn(ConnState::Connected);
                            // 重建 uid → 昵称映射（members 已含自己；重连场景先清空）
                            {
                                let mut names = shared.uid_names.lock().unwrap();
                                names.clear();
                                for (u, n, _, _, _) in &members {
                                    names.insert(*u, n.clone());
                                }
                            }
                            bridge.emit_self_uid(uid);
                            // 自己的 muted 取本地当前值（重连后保持界面与实际一致）
                            let my_muted = shared.self_muted.load(Ordering::Relaxed);
                            let all: Vec<MemberInfo> = members
                                .into_iter()
                                .map(|(u, n, m, s, h)| (u, n, if u == uid { my_muted } else { m }, s, h))
                                .collect();
                            bridge.emit_member_list(all);
                            bridge.emit_auth_ok();
                            // 重连后若本地处于静音，向新会话重新声明（否则服务器端 muted=false，别人看不到）
                            if my_muted {
                                stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on: true }))?;
                            }
                            // 重连：把本地仍在推的流重新声明给新会话（服务器端状态随旧连接清零）
                            {
                                let bm = shared.my_streams.load(Ordering::Relaxed);
                                for kind in [STREAM_SCREEN, STREAM_CAMERA] {
                                    if bm & (1 << kind) != 0 {
                                        stream.write_all(&tcp::encode(&TcpMessage::StreamState { uid: 0, kind, on: true }))?;
                                    }
                                }
                            }
                            // 观众数随新会话归零（服务器接线后会推回真实值）
                            shared.viewer_count.store(0, Ordering::Relaxed);
                            // 启动音频链路（麦克风/编码/播放/VAD 上报；失败不影响文字聊天）
                            crate::bridge::start_audio(&bridge.app, uid, udp_token, addr.to_string(), tx.clone());
                        }
                        TcpMessage::AuthReject { reason } => {
                            if authenticated {
                                // 已进房：资料更新失败等（不断开）
                                bridge.emit_profile_error(reason);
                            } else {
                                // 未进房：认证失败（含房间满）→ 停止重连
                                return Ok(SessionEnd::AuthFailed(reason));
                            }
                        }
                        TcpMessage::MemberJoin { uid, nickname, has_avatar } => {
                            shared.uid_names.lock().unwrap().insert(uid, nickname.clone());
                            bridge.emit_member_join(uid, nickname, has_avatar)
                        }
                        TcpMessage::MemberLeave { uid } => {
                            shared.uid_names.lock().unwrap().remove(&uid);
                            bridge.emit_member_leave(uid)
                        }
                        TcpMessage::Chat { uid, text } => bridge.emit_chat(uid, text),
                        TcpMessage::Speaking { uid, on } => bridge.emit_speaking(uid, on),
                        TcpMessage::Muted { uid, on } => bridge.emit_muted(uid, on),
                        TcpMessage::StreamState { uid, kind, on } => bridge.emit_stream_state(uid, kind, on),
                        TcpMessage::Viewers { uids } => {
                            shared.viewer_count.store(uids.len() as u16, Ordering::Relaxed);
                            bridge.emit_viewer_count(&uids);
                        }
                        TcpMessage::RequestKeyframe { .. } => bridge.emit_request_keyframe(),
                        TcpMessage::ProfileChanged { uid, nickname } => {
                            shared.uid_names.lock().unwrap().insert(uid, nickname.clone());
                            bridge.emit_profile_changed(uid, nickname)
                        }
                        TcpMessage::AvatarData { uid, data } => bridge.emit_avatar_data(uid, data),
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
