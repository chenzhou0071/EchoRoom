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
    pub fn emit_member_list(&self, members: Vec<(u16, String)>) {
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

/// 应用状态：配置 + 当前网络会话
pub struct AppState {
    pub config: Mutex<Config>,
    pub net: Mutex<Option<NetHandle>>,
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
    let handle = tcp::spawn(addr, nickname, bridge);
    let state = app.state::<AppState>();
    let mut slot = state.net.lock().unwrap();
    if let Some(old) = slot.take() {
        let _ = old.tx.send(NetCmd::Shutdown);
    }
    *slot = Some(handle);
}
