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
                            room.broadcast(Some(ok.uid), &TcpMessage::Speaking { uid: ok.uid, on });
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
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break 'read;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    // ---- 离开 ----
    {
        let mut room = room.lock().unwrap();
        room.leave(ok.uid);
        room.broadcast(None, &TcpMessage::MemberLeave { uid: ok.uid });
    }
    println!("[tcp] {nickname}(uid={}) 离开", ok.uid);
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
