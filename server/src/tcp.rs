//! 服务端 TCP：accept 循环 + 每连接读/写线程。
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use echoroom_protocol::messages::TcpMessage;
use echoroom_protocol::tcp;

use crate::auth;
use crate::db::Db;
use crate::room::Room;

pub fn serve(
    listener: TcpListener,
    room: Arc<Mutex<Room>>,
    db: Arc<Db>,
    invite: Option<String>,
) -> std::io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let room = room.clone();
                let db = db.clone();
                let invite = invite.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s, room, db, invite) {
                        eprintln!("[tcp] 连接结束: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[tcp] accept 错误: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(
    mut stream: TcpStream,
    room: Arc<Mutex<Room>>,
    db: Arc<Db>,
    invite: Option<String>,
) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    // ---- 认证（第一条消息必须是 Register / Login / Resume 之一）----
    let (first, buf_rest) = read_first_auth(&mut stream, &db, invite.as_deref())?;
    let auth_result = match first {
        FirstAuth::Ok(r) => r,
        FirstAuth::Reject(reason) => {
            let reject = tcp::encode(&TcpMessage::AuthReject { reason });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let join = room.lock().unwrap().join(
        auth_result.nickname.clone(),
        auth_result.account_id,
        auth_result.has_avatar,
        tx,
    );
    let ok = match join {
        Ok(ok) => ok,
        Err(_) => {
            let reject = tcp::encode(&TcpMessage::AuthReject { reason: "房间已满（6人）".into() });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    println!("[tcp] {}(uid={}) 加入 {peer}", auth_result.nickname, ok.uid);
    // 离开守卫：此后无论何种路径结束（正常断开 / RST / 提前 return / panic），
    // 都保证移除成员并广播 MemberLeave，不产生"僵尸连接"。
    let guard = LeaveGuard { room: room.clone(), uid: ok.uid, nickname: auth_result.nickname.clone() };
    // LoginOk（定向）+ MemberJoin（广播给其他人）
    {
        // members 含自己：客户端需要自己初始的静音/流位图/头像标记
        let mut members = ok.members.clone();
        members.push((ok.uid, auth_result.nickname.clone(), false, 0, auth_result.has_avatar));
        let login_ok = TcpMessage::LoginOk {
            uid: ok.uid,
            udp_token: ok.token,
            auth_token: auth_result.auth_token.clone(),
            members,
        };
        stream.write_all(&tcp::encode(&login_ok))?;
        let room = room.lock().unwrap();
        room.broadcast(
            Some(ok.uid),
            &TcpMessage::MemberJoin {
                uid: ok.uid,
                nickname: auth_result.nickname.clone(),
                has_avatar: auth_result.has_avatar,
            },
        );
    }
    // ---- 写线程：从 rx 取预编码字节写出 ----
    let mut write_stream = stream.try_clone()?;
    let writer = std::thread::spawn(move || {
        for bytes in rx {
            if write_stream.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    // ---- 读循环 ----
    let mut buf: Vec<u8> = buf_rest;
    let mut chunk = [0u8; 4096];
    'read: loop {
        // 先尝试消费已缓冲数据
        loop {
            match tcp::try_decode(&buf) {
                Ok(Some((msg, n))) => {
                    buf.drain(..n);
                    match msg {
                        TcpMessage::Chat { text, .. } => {
                            let room = room.lock().unwrap();
                            room.broadcast(None, &TcpMessage::Chat { uid: ok.uid, text });
                        }
                        TcpMessage::Speaking { on, .. } => {
                            let room = room.lock().unwrap();
                            // 广播回所有人（含自己）：与 Chat 同模式，说话者自己的卡片也要亮
                            room.broadcast(None, &TcpMessage::Speaking { uid: ok.uid, on });
                        }
                        TcpMessage::Mute { on, .. } => {
                            let mut room = room.lock().unwrap();
                            room.set_muted(ok.uid, on);
                            // 广播回所有人（含自己）：与 Chat/Speaking 同模式，卡片图标统一由广播驱动
                            room.broadcast(None, &TcpMessage::Muted { uid: ok.uid, on });
                        }
                        TcpMessage::StreamState { kind, on, .. } => {
                            let mut room = room.lock().unwrap();
                            if room.set_stream(ok.uid, kind, on).is_some() {
                                room.broadcast(None, &TcpMessage::StreamState { uid: ok.uid, kind, on });
                                // 流开/停时同步最新观看名单（R1；0 人在看 → 空列表）
                                notify_viewers(&mut room, ok.uid);
                            }
                        }
                        TcpMessage::Subscribe { target, .. } => {
                            let mut room = room.lock().unwrap();
                            // 覆盖式：切换订阅前先刷新旧目标的名单
                            if let Some(old) = room.subscription_of(ok.uid) {
                                room.unsubscribe(ok.uid);
                                if old != target {
                                    notify_viewers(&mut room, old);
                                }
                            }
                            if room.subscribe(ok.uid, target) {
                                notify_viewers(&mut room, target);
                            }
                        }
                        TcpMessage::Unsubscribe { .. } => {
                            let mut room = room.lock().unwrap();
                            if let Some(old) = room.subscription_of(ok.uid) {
                                room.unsubscribe(ok.uid);
                                notify_viewers(&mut room, old);
                            }
                        }
                        TcpMessage::RequestKeyframe { target, .. } => {
                            let room = room.lock().unwrap();
                            // 转发给目标（uid 替换为请求者，供流主识别）
                            room.send_to(target, &TcpMessage::RequestKeyframe { uid: ok.uid, target });
                        }
                        TcpMessage::SetProfile { nickname, avatar } => {
                            match auth::apply_profile(&db, auth_result.account_id, &nickname, avatar.as_deref()) {
                                Ok(()) => {
                                    let mut room = room.lock().unwrap();
                                    room.update_nickname(ok.uid, &nickname);
                                    if avatar.is_some() {
                                        room.set_has_avatar(ok.uid, true);
                                    }
                                    room.broadcast(None, &TcpMessage::ProfileChanged { uid: ok.uid, nickname });
                                }
                                Err(reason) => {
                                    // 已进房：AuthReject 语义为"资料更新失败"，仅定向提示不断开
                                    let room = room.lock().unwrap();
                                    room.send_to(ok.uid, &TcpMessage::AuthReject { reason });
                                }
                            }
                        }
                        TcpMessage::AvatarRequest { uid: target } => {
                            // 两段取锁：先短锁房间拿 account_id，再查库（避免跨锁嵌套）
                            let account_id = room.lock().unwrap().account_id_of(target);
                            if let Some(account_id) = account_id {
                                let data = db.get_avatar(account_id).unwrap_or_default();
                                let room = room.lock().unwrap();
                                room.send_to(ok.uid, &TcpMessage::AvatarData { uid: target, data });
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    eprintln!("[tcp] uid={} 协议错误: {e:?}", ok.uid);
                    break 'read;
                }
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => break 'read,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break 'read, // 连接错误（如客户端 RST）：同样按断开处理
        }
    }
    // ---- 离开：显式触发守卫（先于 writer.join，保证发送端通道关闭使写线程可结束）----
    drop(guard);
    drop(stream);
    let _ = writer.join();
    Ok(())
}

/// 首条消息认证结果
enum FirstAuth {
    Ok(auth::AuthResult),
    Reject(String),
}

/// 读缓冲直到解出第一条 Register/Login/Resume 并完成认证校验；
/// 返回 (认证结果, 剩余未消费字节)
fn read_first_auth(
    stream: &mut TcpStream,
    db: &Db,
    invite: Option<&str>,
) -> std::io::Result<(FirstAuth, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match tcp::try_decode(&buf) {
            Ok(Some((msg, n))) => {
                buf.drain(..n);
                let result = match msg {
                    TcpMessage::Register { account, password, invite: user_invite } => {
                        auth::register(db, invite, &account, &password, &user_invite)
                    }
                    TcpMessage::Login { account, password } => auth::login(db, &account, &password),
                    TcpMessage::Resume { auth_token } => auth::resume(db, &auth_token),
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "首条消息必须是 Register/Login/Resume",
                        ));
                    }
                };
                let first = match result {
                    Ok(r) => FirstAuth::Ok(r),
                    Err(reason) => FirstAuth::Reject(reason),
                };
                return Ok((first, buf));
            }
            Ok(None) => {
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "连接在认证前关闭"));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(e) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")));
            }
        }
    }
}

/// 离开守卫：连接线程无论以何种方式结束（正常断开 / RST / 提前返回 / panic），
/// 都保证从房间移除成员并广播 MemberLeave。
struct LeaveGuard {
    room: Arc<Mutex<Room>>,
    uid: u16,
    nickname: String,
}

impl Drop for LeaveGuard {
    fn drop(&mut self) {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner()); // 锁 poisoned 时也尽力清理
        // 离开者若正在观看别人：先取出目标，离开后刷新该目标名单（R1）
        let watching = room.subscription_of(self.uid);
        if room.leave(self.uid) {
            room.broadcast(None, &TcpMessage::MemberLeave { uid: self.uid });
            println!("[tcp] {}(uid={}) 离开", self.nickname, self.uid);
        }
        if let Some(target) = watching {
            notify_viewers(&mut room, target);
        }
    }
}

/// R1：向流主与该流全部订阅者发送最新观看名单（同一份内容；无人看时流主收到空列表）
fn notify_viewers(room: &mut Room, target: u16) {
    let uids = room.viewers_of(target);
    let msg = TcpMessage::Viewers { uids: uids.clone() };
    room.send_to(target, &msg);
    for uid in uids {
        room.send_to(uid, &msg);
    }
}
