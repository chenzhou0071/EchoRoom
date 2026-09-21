pub mod audio;
pub mod bridge;
pub mod config;
pub mod indicator;
pub mod net;

use bridge::AppState;

pub fn run() {
    let cfg = config::Config::load(&config::default_config_path());
    let shared = audio::session::SharedAudio::new(
        cfg.self_gain,
        cfg.muted,
        cfg.peer_gains.clone(),
        cfg.screen_gain,
    );
    tauri::Builder::default()
        .manage(AppState {
            config: std::sync::Mutex::new(cfg),
            net: std::sync::Mutex::new(None),
            audio: std::sync::Mutex::new(None),
            shared,
            sharing: std::sync::atomic::AtomicBool::new(false),
            screen_cap: std::sync::Mutex::new(None),
            watching: std::sync::Mutex::new(None),
            indicator: std::sync::Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            bridge::get_config,
            bridge::auth_login,
            bridge::auth_register,
            bridge::auto_connect,
            bridge::send_chat,
            bridge::set_profile,
            bridge::avatar_request,
            bridge::set_self_gain,
            bridge::set_muted,
            bridge::set_peer_gain,
            bridge::set_screen_gain,
            bridge::subscribe,
            bridge::report_stream,
            bridge::request_keyframe,
            bridge::set_share_active,
            bridge::set_share_audio,
            bridge::set_share_quality,
            bridge::screen_audio_supported,
            bridge::send_video_frame,
            bridge::watch_start,
            bridge::watch_stop,
            bridge::open_url
        ])
        // 连接时机交给 UI：前端注册好事件监听后再 invoke("connect")，
        // 避免"连接过快、事件先于监听器到达"导致首屏丢事件。
        .run(tauri::generate_context!())
        .expect("error while running Echo");
}

