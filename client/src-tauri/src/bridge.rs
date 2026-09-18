//! tauri 桥接：UI 命令（invoke）与事件出口（emit）。
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::{default_config_path, Config};
use crate::net::tcp::{self, NetCmd, NetHandle};

/// 连接状态（emit 到 UI 的 "conn" 事件）
#[derive(Clone, PartialEq)]
pub enum ConnState {
    Connecting,
    Connected,
    Reconnecting,
    Rejected(String),
}

/// 事件出口：网络线程 → UI
#[derive(Clone)]
pub struct Bridge {
    pub app: AppHandle,
}

impl Bridge {
    pub fn emit_member_list(&self, members: Vec<(u16, String, bool)>) {
        let _ = self.app.emit("members", members);
    }
    pub fn emit_member_join(&self, uid: u16, nickname: String) {
        let _ = self.app.emit("member_join", (uid, nickname));
    }
    pub fn emit_member_leave(&self, uid: u16) {
        let _ = self.app.emit("member_leave", uid);
    }
    pub fn emit_chat(&self, uid: u16, text: String) {
        let _ = self.app.emit("chat", serde_json::json!({ "uid": uid, "text": text }));
    }
    pub fn emit_speaking(&self, uid: u16, on: bool) {
        let _ = self.app.emit("speaking", serde_json::json!({ "uid": uid, "on": on }));
    }
    pub fn emit_muted(&self, uid: u16, on: bool) {
        let _ = self.app.emit("muted", serde_json::json!({ "uid": uid, "on": on }));
    }
    pub fn emit_self_uid(&self, uid: u16) {
        let _ = self.app.emit("self_uid", uid);
    }
    pub fn emit_conn(&self, state: ConnState) {
        let s = match state {
            ConnState::Connecting => "connecting".to_string(),
            ConnState::Connected => "connected".to_string(),
            ConnState::Reconnecting => "reconnecting".to_string(),
            ConnState::Rejected(reason) => format!("rejected:{reason}"),
        };
        let _ = self.app.emit("conn", s);
    }
}

/// 应用状态：配置 + 当前网络会话 + 音频管线
pub struct AppState {
    pub config: Mutex<Config>,
    pub net: Mutex<Option<NetHandle>>,
    pub audio: Mutex<Option<crate::audio::session::AudioHandle>>,
    /// 音量/静音共享态（音频线程、网络线程共享读写）
    pub shared: crate::audio::session::SharedAudio,
}

// ---- UI 命令 ----

#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_config(
    app: AppHandle,
    state: State<AppState>,
    nickname: String,
    server_addr: String,
) -> Result<(), String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.nickname = nickname.clone();
        cfg.server_addr = server_addr.clone();
        cfg.save(&default_config_path()).map_err(|e| e.to_string())?;
    }
    // 保存后立即用新配置重连
    connect_with_app(&app, nickname, server_addr);
    Ok(())
}

#[tauri::command]
pub fn connect(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config.lock().unwrap().clone();
    if cfg.nickname.is_empty() {
        return Err("请先设置昵称".into());
    }
    connect_with_app(&app, cfg.nickname, cfg.server_addr);
    Ok(())
}

#[tauri::command]
pub fn send_chat(state: State<AppState>, text: String) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::SendChat(text)).map_err(|e| e.to_string())
}

/// 用（新的）配置发起连接：先起新会话，再停掉旧会话（UI 事件无感切换）。
pub fn connect_with_app(app: &AppHandle, nickname: String, addr: String) {
    let bridge = Bridge { app: app.clone() };
    let state = app.state::<AppState>();
    let handle = tcp::spawn(addr, nickname, bridge, state.shared.clone());
    let mut slot = state.net.lock().unwrap();
    if let Some(old) = slot.take() {
        let _ = old.tx.send(NetCmd::Shutdown);
    }
    *slot = Some(handle);
}

/// 音量状态快照（emit "volume" 事件用）
fn volume_json(state: &State<AppState>) -> serde_json::Value {
    use std::sync::atomic::Ordering;
    let self_gain = f32::from_bits(state.shared.self_gain.load(Ordering::Relaxed));
    let muted = state.shared.self_muted.load(Ordering::Relaxed);
    let peer_gains = state.shared.peer_gains.lock().unwrap().clone();
    serde_json::json!({ "self_gain": self_gain, "muted": muted, "peer_gains": peer_gains })
}

/// 配置写盘走独立线程：滑块拖动会高频调用 set_*，避免写文件阻塞 IPC 主线程
fn persist(state: &AppState) {
    let cfg = state.config.lock().unwrap().clone();
    std::thread::spawn(move || {
        let _ = cfg.save(&default_config_path());
    });
}

#[tauri::command]
pub fn set_self_gain(app: AppHandle, state: State<AppState>, gain: f32) {
    let g = gain.clamp(0.0, 2.0);
    state
        .shared
        .self_gain
        .store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
    state.config.lock().unwrap().self_gain = g;
    persist(&state);
    let _ = app.emit("volume", volume_json(&state));
}

#[tauri::command]
pub fn set_muted(app: AppHandle, state: State<AppState>, on: bool) {
    state
        .shared
        .self_muted
        .store(on, std::sync::atomic::Ordering::Relaxed);
    state.config.lock().unwrap().muted = on;
    persist(&state);
    if let Some(h) = state.net.lock().unwrap().as_ref() {
        let _ = h.tx.send(NetCmd::SetMuted(on));
    }
    let _ = app.emit("volume", volume_json(&state));
}

#[tauri::command]
pub fn set_peer_gain(app: AppHandle, state: State<AppState>, nickname: String, gain: f32) {
    let g = gain.clamp(0.0, 2.0);
    let snapshot = {
        let mut map = state.shared.peer_gains.lock().unwrap();
        map.insert(nickname, g);
        map.clone()
    };
    state.config.lock().unwrap().peer_gains = snapshot;
    persist(&state);
    let _ = app.emit("volume", volume_json(&state));
}

/// 登录成功后：停旧音频管线，按新 uid/token/服务器地址启动（失败不影响文字聊天）。
/// `tcp_tx` 供采集线程上报说话状态（VAD）。
pub fn start_audio(
    app: &AppHandle,
    uid: u16,
    token: u32,
    server_addr: String,
    tcp_tx: std::sync::mpsc::Sender<NetCmd>,
) {
    let state = app.state::<AppState>();
    let mut slot = state.audio.lock().unwrap();
    if let Some(old) = slot.take() {
        old.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    match crate::audio::session::spawn_audio_pipeline(
        server_addr,
        uid,
        token,
        tcp_tx,
        state.shared.clone(),
    ) {
        Ok(h) => *slot = Some(h),
        Err(e) => eprintln!("[audio] 管线启动失败: {e:#}"),
    }
}
