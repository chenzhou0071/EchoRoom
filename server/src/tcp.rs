//! 服务端 TCP：accept 循环 + 每连接读/写线程。
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use echoroom_protocol::messages::TcpMessage;
use echoroom_protocol::tcp;

use crate::room::Room;

pub fn serve(listener: TcpListener, room: Arc<Mutex<Room>>) -> std::io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let room = room.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s, room) {
                        eprintln!("[tcp] 连接结束: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[tcp] accept 错误: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(mut stream: TcpStream, room: Arc<Mutex<Room>>) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    // ---- 登录（第一条消息必须是 Login）----
    let (nickname, buf_rest) = read_first_login(&mut stream)?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let join = room.lock().unwrap().join(nickname.clone(), tx);
    let ok = match join {
        Ok(ok) => ok,
        Err(_) => {
            let reject = tcp::encode(&TcpMessage::LoginReject { reason: "房间已满（6人）".into() });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    println!("[tcp] {nickname}(uid={}) 加入 {peer}", ok.uid);
    // 离开守卫：此后无论何种路径结束（正常断开 / RST / 提前 return / panic），
    // 都保证移除成员并广播 MemberLeave，不产生"僵尸连接"。
    let guard = LeaveGuard { room: room.clone(), uid: ok.uid, nickname: nickname.clone() };
    // LoginOk（定向）+ MemberJoin（广播给其他人）
    {
        let login_ok = TcpMessage::LoginOk { uid: ok.uid, token: ok.token, members: ok.members.clone() };
        stream.write_all(&tcp::encode(&login_ok))?;
        let room = room.lock().unwrap();
        room.broadcast(Some(ok.uid), &TcpMessage::MemberJoin { uid: ok.uid, nickname: nickname.clone() });
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

/// 读缓冲直到解出第一条 Login 消息；返回 (昵称, 剩余未消费字节)
fn read_first_login(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match tcp::try_decode(&buf) {
            Ok(Some((TcpMessage::Login { nickname }, n))) => {
                buf.drain(..n);
                return Ok((nickname, buf));
            }
            Ok(Some(_)) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "首条消息必须是 Login"));
            }
            Ok(None) => {
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "连接在登录前关闭"));
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
