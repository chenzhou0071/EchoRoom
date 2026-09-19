pub mod audio;
pub mod bridge;
pub mod config;
pub mod net;

use bridge::AppState;
use tauri::Manager;

pub fn run() {
    let cfg = config::Config::load(&config::default_config_path());
    let shared =
        audio::session::SharedAudio::new(cfg.self_gain, cfg.muted, cfg.peer_gains.clone());
    tauri::Builder::default()
        .manage(AppState {
            config: std::sync::Mutex::new(cfg),
            net: std::sync::Mutex::new(None),
            audio: std::sync::Mutex::new(None),
            shared,
        })
        .invoke_handler(tauri::generate_handler![
            bridge::get_config,
            bridge::set_config,
            bridge::connect,
            bridge::send_chat,
            bridge::set_self_gain,
            bridge::set_muted,
            bridge::set_peer_gain,
            bridge::subscribe,
            bridge::report_stream,
            bridge::request_keyframe,
            spike_echo,
            spike_feed,
            spike_minimize,
            spike_min_check
        ])
        // 连接时机交给 UI：前端注册好事件监听后再 invoke("connect")，
        // 避免"连接过快、事件先于监听器到达"导致首屏丢事件。
        .run(tauri::generate_context!())
        .expect("error while running Echo");
}

// --- 临时 spike 命令（Task 11 删除）---
#[tauri::command]
fn spike_echo(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(b) if b.len() == 16 * 1024 => Ok(()),
        tauri::ipc::InvokeBody::Raw(_) => Err("长度不符".into()),
        _ => Err("非 raw body（invoke 二进制参数未生效）".into()),
    }
}

#[tauri::command]
fn spike_feed(ch: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<(), String> {
    // 注意：Channel<Vec<u8>> 会把字节序列化成 JSON 数字数组（JS 侧收到 Array，
    // 实测 byteLength=undefined）。必须用 InvokeResponseBody::Raw 才走二进制路径，
    // JS 侧 invoke 以 octet-stream 读成真正的 ArrayBuffer。
    let payload: Vec<u8> = (0..16 * 1024).map(|i| (i % 251) as u8).collect();
    for _ in 0..100 {
        ch.send(tauri::ipc::InvokeResponseBody::Raw(payload.clone()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn spike_minimize(window: tauri::Window, minimize: bool) -> Result<(), String> {
    if minimize {
        window.minimize().map_err(|e| e.to_string())
    } else {
        window
            .unminimize()
            .and_then(|_| window.set_focus())
            .map_err(|e| e.to_string())
    }
}

/// 延迟复查窗口最小化状态，直接打印到 dev 控制台（后台线程，不依赖 JS 存活）
#[tauri::command]
fn spike_min_check(app: tauri::AppHandle, after_ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(after_ms));
        match app.get_webview_window("main") {
            Some(w) => match w.is_minimized() {
                Ok(m) => println!("[spike] {after_ms}ms 后 is_minimized = {m}"),
                Err(e) => println!("[spike] is_minimized 查询失败: {e}"),
            },
            None => println!("[spike] 找不到 main 窗口"),
        }
    });
}
