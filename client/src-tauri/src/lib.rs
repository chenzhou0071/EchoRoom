pub mod audio;
pub mod bridge;
pub mod config;
pub mod net;

use bridge::AppState;

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
            bridge::set_peer_gain
        ])
        // 连接时机交给 UI：前端注册好事件监听后再 invoke("connect")，
        // 避免"连接过快、事件先于监听器到达"导致首屏丢事件。
        .run(tauri::generate_context!())
        .expect("error while running Echo");
}
