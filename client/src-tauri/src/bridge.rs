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
    pub fn emit_member_list(&self, members: Vec<(u16, String, bool, u8)>) {
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
    pub fn emit_stream_state(&self, uid: u16, kind: u8, on: bool) {
        let _ = self.app.emit("stream_state", serde_json::json!({ "uid": uid, "kind": kind, "on": on }));
    }
    /// R1：观看名单（uid 数组；人数 = len）
    pub fn emit_viewer_count(&self, uids: &[u16]) {
        let _ = self.app.emit("viewer_count", uids);
    }
    pub fn emit_request_keyframe(&self) {
        let _ = self.app.emit("request_keyframe", ());
    }
}

/// 应用状态：配置 + 当前网络会话 + 音频管线
pub struct AppState {
    pub config: Mutex<Config>,
    pub net: Mutex<Option<NetHandle>>,
    pub audio: Mutex<Option<crate::audio::session::AudioHandle>>,
    /// 音量/静音共享态（音频线程、网络线程共享读写）
    pub shared: crate::audio::session::SharedAudio,
    /// 本端是否正在投屏（屏幕流激活；驱动屏幕捕获生命周期）
    pub sharing: std::sync::atomic::AtomicBool,
    /// 屏幕捕获会话（Drop 即停止）
    pub screen_cap: Mutex<Option<crate::audio::screen_capture::ScreenCaptureHandle>>,
    /// 观看线程停止旗标（None = 未在观看）
    pub watching: Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    /// 屏幕共享提示条隐藏器（投屏期间运行；见 indicator.rs）
    pub indicator: Mutex<Option<crate::indicator::Hider>>,
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

/// 观看端：投屏（屏幕）声音的播放增益（0.0–2.0）；无需事件回传，前端持有滑块真值
#[tauri::command]
pub fn set_screen_gain(state: State<AppState>, gain: f32) {
    let g = gain.clamp(0.0, 2.0);
    state
        .shared
        .screen_gain
        .store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
    state.config.lock().unwrap().screen_gain = g;
    persist(&state);
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

// ---- 投屏/观看命令（计划2 B）----

/// 订阅（None = 取消）：服务器开始/停止把目标的视频转发给本端
#[tauri::command]
pub fn subscribe(state: State<AppState>, target: Option<u16>) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::Subscribe(target)).map_err(|e| e.to_string())
}

/// 上报本端某路流开/停：更新共享位图（重连补报）并向服务器广播
#[tauri::command]
pub fn report_stream(state: State<AppState>, kind: u8, on: bool) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let prev = state.shared.my_streams.load(Ordering::Relaxed);
    let bit = 1u8 << kind;
    let next = if on { prev | bit } else { prev & !bit };
    state.shared.my_streams.store(next, Ordering::Relaxed);
    let slot = state.net.lock().unwrap();
    if let Some(h) = slot.as_ref() {
        let _ = h.tx.send(NetCmd::SetStream { kind, on });
    }
    Ok(())
}

/// 请求目标发关键帧（进入观看 / 解码失败时用）
#[tauri::command]
pub fn request_keyframe(state: State<AppState>, target: u16) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::RequestKeyframe(target)).map_err(|e| e.to_string())
}

/// 统一切换屏幕捕获期望状态：投屏中 && 共享声音开 && 平台支持。
/// 已启动但条件不再满足 → Drop 句柄停止；条件满足但未启动 → 尝试启动。
fn sync_screen_capture(state: &AppState, active: bool) {
    let want =
        active && state.config.lock().unwrap().share_audio && crate::audio::screen_capture::is_supported();
    let mut slot = state.screen_cap.lock().unwrap();
    if want && slot.is_none() {
        let tx = {
            let audio = state.audio.lock().unwrap();
            audio.as_ref().map(|h| h.screen_pcm_tx.clone())
        };
        let Some(tx) = tx else {
            return; // 音频管线未启动（重连窗口期）：下次调用会再试
        };
        match crate::audio::screen_capture::spawn_screen_capture(tx) {
            Ok(h) => {
                println!("[screen] 屏幕声音采集启动");
                *slot = Some(h);
            }
            Err(e) => eprintln!("[screen] 采集启动失败: {e:#}"),
        }
    } else if !want && slot.is_some() {
        *slot = None; // Drop → 停止采集
        println!("[screen] 屏幕声音采集停止");
    }
}

/// 投屏开/停（前端在投屏开始/结束时调用；联动屏幕声音采集与提示条隐藏）
#[tauri::command]
pub fn set_share_active(state: State<AppState>, active: bool) {
    state.sharing.store(active, std::sync::atomic::Ordering::Relaxed);
    sync_screen_capture(&state, active);
    // 隐藏 WebView2 自带的"正在共享你的屏幕"提示条（仅投屏期间轮询）
    let mut slot = state.indicator.lock().unwrap();
    if active {
        if slot.is_none() {
            *slot = Some(crate::indicator::Hider::spawn());
        }
    } else {
        *slot = None; // Drop → 停轮询线程
    }
}

/// 共享系统声音开关（仅 Win11 支持）
#[tauri::command]
pub fn set_share_audio(app: AppHandle, state: State<AppState>, on: bool) {
    state.config.lock().unwrap().share_audio = on;
    persist(&state);
    sync_screen_capture(&state, state.sharing.load(std::sync::atomic::Ordering::Relaxed));
    let _ = app.emit("share_audio", on);
}

/// 投屏画质档位（仅持久化；编码参数由前端 setQuality 应用）
#[tauri::command]
pub fn set_share_quality(state: State<AppState>, quality: String) {
    state.config.lock().unwrap().share_quality = quality;
    persist(&state);
}

/// 系统是否支持共享屏幕声音（Windows 11 build 22000+）
#[tauri::command]
pub fn screen_audio_supported() -> bool {
    crate::audio::screen_capture::is_supported()
}

/// 前端编码帧上行（Tauri 原始请求体：[kind u8][keyframe u8][annexb data...]）
#[tauri::command]
pub fn send_video_frame(state: State<AppState>, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static VIDEO_FRAMES: AtomicU64 = AtomicU64::new(0);
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("send_video_frame 需要二进制参数".into());
    };
    if bytes.len() < 3 {
        return Err("帧数据过短".into());
    }
    let kind = bytes[0];
    let keyframe = bytes[1] != 0;
    let data = bytes[2..].to_vec();
    let tx = {
        let audio = state.audio.lock().unwrap();
        audio.as_ref().map(|h| h.video_tx.clone())
    };
    let Some(tx) = tx else {
        return Err("音频管线未启动".into());
    };
    tx.try_send(crate::net::udp::VideoOut { kind, keyframe, data })
        .map_err(|_| "视频队列满".to_string())?;
    let n = VIDEO_FRAMES.fetch_add(1, Ordering::Relaxed);
    if n % 150 == 0 {
        println!("[video] 上行帧 #{n}");
    }
    Ok(())
}

// ---- 观看命令（计划2 B / Task 9）----

/// 开始观看某人：订阅 + 建视频下行线程（新调用会替换旧观看线程）。
/// 下行帧格式：[uid u16 BE][kind u8][keyframe u8][annexb data]
/// 二进制通道走 InvokeResponseBody::Raw（spike A4：Channel<Vec<u8>> 会退化成 JSON 数组）。
#[tauri::command]
pub fn watch_start(
    state: State<AppState>,
    ch: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
    target: u16,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    // 先取视频接收端（音频管线启动后才有）——先于订阅校验，避免失败时留下悬挂订阅
    let rx = {
        let audio = state.audio.lock().unwrap();
        let Some(ah) = audio.as_ref() else {
            return Err("音频管线未启动".into());
        };
        ah.video_rx.clone()
    };
    // 停旧观看线程
    if let Some(old) = state.watching.lock().unwrap().take() {
        old.store(true, Ordering::Relaxed);
    }
    // 订阅（服务器开始把 target 的流转给本端）
    {
        let slot = state.net.lock().unwrap();
        let h = slot.as_ref().ok_or("未连接")?;
        h.tx.send(NetCmd::Subscribe(Some(target))).map_err(|e| e.to_string())?;
    }
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    *state.watching.lock().unwrap() = Some(stop.clone());
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let item = {
                let guard = rx.lock().unwrap();
                guard.recv_timeout(std::time::Duration::from_millis(100))
            };
            match item {
                Ok(frame) => {
                    let mut payload = Vec::with_capacity(4 + frame.data.len());
                    payload.extend_from_slice(&frame.uid.to_be_bytes());
                    payload.push(frame.kind);
                    payload.push(if frame.keyframe { 1 } else { 0 });
                    payload.extend_from_slice(&frame.data);
                    if ch.send(tauri::ipc::InvokeResponseBody::Raw(payload)).is_err() {
                        break; // 前端已销毁
                    }
                }
                Err(_) => continue, // 超时：循环检查 stop
            }
        }
    });
    Ok(())
}

/// 停止观看：退订 + 停线程
#[tauri::command]
pub fn watch_stop(state: State<AppState>) -> Result<(), String> {
    if let Some(old) = state.watching.lock().unwrap().take() {
        old.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let slot = state.net.lock().unwrap();
    if let Some(h) = slot.as_ref() {
        let _ = h.tx.send(NetCmd::Subscribe(None));
    }
    Ok(())
}
