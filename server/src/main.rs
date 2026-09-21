//! EchoRoom 服务端：TCP 控制 + UDP 语音转发（单房间）。
//! 参数：echoroom-server [port] [--data <目录>] [--invite <码>]
mod auth;
mod db;
mod room;
mod tcp;
mod udp;

use std::sync::{Arc, Mutex};

fn main() {
    // ---- 参数解析：位置参数 port（默认 9000）+ --data/--invite ----
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port: Option<u16> = None;
    let mut data_dir = "data".to_string();
    let mut invite: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data" => {
                i += 1;
                data_dir = args.get(i).expect("--data 需要一个目录参数").clone();
            }
            "--invite" => {
                i += 1;
                invite = Some(args.get(i).expect("--invite 需要一个邀请码").clone());
            }
            other => port = Some(other.parse::<u16>().expect("端口必须是数字")),
        }
        i += 1;
    }
    let port = port.unwrap_or(echoroom_protocol::DEFAULT_PORT);

    let room = Arc::new(Mutex::new(room::Room::new()));
    let db_path = std::path::Path::new(&data_dir).join("echoroom.db");
    let db = Arc::new(db::Db::open(&db_path).expect("打开数据库失败"));
    println!("[auth] 数据库: {}", db_path.display());
    match &invite {
        Some(_) => println!("[auth] 邀请码已配置，注册开放"),
        None => println!("[auth] 未配置 --invite：注册已关闭"),
    }

    // UDP 语音/投屏转发 + 地址映射超时清理线程（已实现，见 udp.rs）
    udp::spawn_udp_loop(port, room.clone());
    udp::spawn_cleanup_loop(room.clone());

    let listener = std::net::TcpListener::bind(("0.0.0.0", port)).expect("TCP bind 失败");
    println!("Echo server listening on 0.0.0.0:{port} (tcp+udp)");
    tcp::serve(listener, room, db, invite).expect("TCP 服务异常退出");
}
