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
    pub fn emit_member_list(&self, members: Vec<echoroom_protocol::messages::MemberInfo>) {
        let _ = self.app.emit("members", members);
    }
    pub fn emit_member_join(&self, uid: u16, nickname: String, has_avatar: bool) {
        let _ = self.app.emit("member_join", (uid, nickname, has_avatar));
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
    /// 认证成功（注册/登录/自动登录）
    pub fn emit_auth_ok(&self) {
        let _ = self.app.emit("auth_ok", ());
    }
    /// 认证失败（登录/注册/自动登录被拒）：客户端已停止重连，交还 UI
    pub fn emit_auth_fail(&self, reason: String) {
        let _ = self.app.emit("auth_fail", reason);
    }
    /// 资料更新失败提示（如昵称不合法；连接保持）
    pub fn emit_profile_error(&self, reason: String) {
        let _ = self.app.emit("profile_error", reason);
    }
    /// 昵称变更广播（头像变化由前端重拉 AvatarData 感知）
    pub fn emit_profile_changed(&self, uid: u16, nickname: String) {
        let _ = self.app.emit("profile_changed", serde_json::json!({ "uid": uid, "nickname": nickname }));
    }
    /// 头像数据（空数组 = 无头像）；前端 Blob 缓存渲染
    pub fn emit_avatar_data(&self, uid: u16, data: Vec<u8>) {
        let _ = self.app.emit("avatar_data", serde_json::json!({ "uid": uid, "data": data }));
    }
    /// 持久化 auth_token（LoginOk 后调用；独立线程写盘防阻塞网络线程）
    pub fn save_auth_token(&self, token: String) {
        let state = self.app.state::<AppState>();
        let cfg = {
            let mut cfg = state.config.lock().unwrap();
            cfg.auth_token = token;
            cfg.clone()
        };
        std::thread::spawn(move || {
            let _ = cfg.save(&default_config_path());
        });
    }
    /// 清除 auth_token（Resume 失效：凭证过期）
    pub fn clear_auth_token(&self) {
        let state = self.app.state::<AppState>();
        let cfg = {
            let mut cfg = state.config.lock().unwrap();
            cfg.auth_token.clear();
            cfg.clone()
        };
        std::thread::spawn(move || {
            let _ = cfg.save(&default_config_path());
        });
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
pub fn auth_login(
    app: AppHandle,
    state: State<AppState>,
    server_addr: String,
    account: String,
    password: String,
) -> Result<(), String> {
    auth_common(
        &app,
        &state,
        &server_addr,
        &account,
        tcp::AuthMode::Login { account: account.clone(), password },
    )
}

#[tauri::command]
pub fn auth_register(
    app: AppHandle,
    state: State<AppState>,
    server_addr: String,
    account: String,
    password: String,
    invite: String,
) -> Result<(), String> {
    auth_common(
        &app,
        &state,
        &server_addr,
        &account,
        tcp::AuthMode::Register { account: account.clone(), password, invite },
    )
}

/// 自动登录：用已保存的 auth_token 走 Resume
#[tauri::command]
pub fn auto_connect(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let token = state.config.lock().unwrap().auth_token.clone();
    if token.is_empty() {
        return Err("没有可用的登录凭证".into());
    }
    connect_with_app(&app, tcp::AuthMode::Resume { auth_token: token });
    Ok(())
}

/// 认证命令公共部分：持久化服务器地址/账号后发起连接
fn auth_common(
    app: &AppHandle,
    state: &State<AppState>,
    server_addr: &str,
    account: &str,
    mode: tcp::AuthMode,
) -> Result<(), String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.server_addr = server_addr.to_string();
        cfg.account = account.to_string();
        cfg.save(&default_config_path()).map_err(|e| e.to_string())?;
    }
    connect_with_app(app, mode);
    Ok(())
}

#[tauri::command]
pub fn send_chat(state: State<AppState>, text: String) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::SendChat(text)).map_err(|e| e.to_string())
}

/// 更新资料（昵称必填；avatar = None 不改头像）
#[tauri::command]
pub fn set_profile(state: State<AppState>, nickname: String, avatar: Option<Vec<u8>>) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::SetProfile { nickname, avatar }).map_err(|e| e.to_string())
}

/// 请求某人的头像（懒加载；服务器回 AvatarData 事件）
#[tauri::command]
pub fn avatar_request(state: State<AppState>, uid: u16) -> Result<(), String> {
    let slot = state.net.lock().unwrap();
    let h = slot.as_ref().ok_or("未连接")?;
    h.tx.send(NetCmd::AvatarRequest(uid)).map_err(|e| e.to_string())
}

/// 退出登录：清凭证 → 停音频管线 → 停视频会话 → 断 TCP（服务器按正常断开广播 leave）。
/// 前端在命令返回后清理本地 UI 状态并回登录页；`account` 保留在 config 供预填。
#[tauri::command]
pub fn logout(state: State<AppState>) {
    use std::sync::atomic::Ordering;
    // 1) 清凭证：下次启动不再自动登录
    state.config.lock().unwrap().auth_token.clear();
    persist(&state);
    // 2) 停音频管线（Drop 关闭采集/播放/编解码线程）
    if let Some(h) = state.audio.lock().unwrap().take() {
        h.stop.store(true, Ordering::Relaxed);
    }
    // 3) 视频会话清零：投屏采集 / 观看线程 / 提示条隐藏器（Drop 即停止）
    state.sharing.store(false, Ordering::Relaxed);
    *state.screen_cap.lock().unwrap() = None;
    if let Some(stop) = state.watching.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
    *state.indicator.lock().unwrap() = None;
    state.shared.my_streams.store(0, Ordering::Relaxed);
    // 4) 断开连接：网络线程正常收尾（Shutdown → 不重连、不发 conn 事件）
    if let Some(h) = state.net.lock().unwrap().take() {
        let _ = h.tx.send(NetCmd::Shutdown);
    }
    println!("[auth] 已退出登录");
}

// ---- D：背景图（文件驱动；单文件无扩展名，MIME 由文件头嗅探） ----

/// 背景图上限 10MB（前端已校验；此处兜底）
const BACKGROUND_MAX: usize = 10 * 1024 * 1024;

/// 背景图文件路径（本地数据目录，与 config.json 同目录）
fn background_path() -> std::path::PathBuf {
    default_config_path()
        .parent()
        .map(|p| p.join("background.img"))
        .unwrap_or_else(|| std::path::PathBuf::from("background.img"))
}

/// 由文件头嗅探图片 MIME：仅 PNG/JPEG/WebP；未知返回 None
fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// 设置背景图：前端读文件后的原始字节（Tauri 原始请求体）；校验通过才写盘，直接覆盖旧图
#[tauri::command]
pub fn set_background(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("set_background 需要二进制参数".into());
    };
    if bytes.is_empty() {
        return Err("图片内容为空".into());
    }
    if bytes.len() > BACKGROUND_MAX {
        return Err("图片过大（上限 10MB）".into());
    }
    if sniff_mime(bytes).is_none() {
        return Err("仅支持 PNG / JPEG / WebP 图片".into());
    }
    let path = background_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("写入背景图失败：{e}"))?;
    println!("[bg] 背景图已更新: {} 字节", bytes.len());
    Ok(())
}

/// 清除背景图（无图时静默成功）
#[tauri::command]
pub fn clear_background() -> Result<(), String> {
    match std::fs::remove_file(background_path()) {
        Ok(()) => {
            println!("[bg] 背景图已清除");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除背景图失败：{e}")),
    }
}

/// 读取背景图：原始响应体（前端收到 ArrayBuffer）；空数组 = 无背景（不存在/为空/超限异常残留）
#[tauri::command]
pub fn get_background() -> tauri::ipc::Response {
    let bytes = std::fs::read(background_path())
        .ok()
        .filter(|b| !b.is_empty() && b.len() <= BACKGROUND_MAX)
        .unwrap_or_default();
    tauri::ipc::Response::new(bytes)
}

/// 用（新的）配置发起连接：先起新会话，再停掉旧会话（UI 事件无感切换）。
pub fn connect_with_app(app: &AppHandle, mode: tcp::AuthMode) {
    let bridge = Bridge { app: app.clone() };
    let state = app.state::<AppState>();
    let addr = state.config.lock().unwrap().server_addr.clone();
    let handle = tcp::spawn(addr, mode, bridge, state.shared.clone());
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
    let g = gain.clamp(0.0, 4.0);
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
    let g = gain.clamp(0.0, 4.0);
    let snapshot = {
        let mut map = state.shared.peer_gains.lock().unwrap();
        map.insert(nickname, g);
        map.clone()
    };
    state.config.lock().unwrap().peer_gains = snapshot;
    persist(&state);
    let _ = app.emit("volume", volume_json(&state));
}

/// 观看端：投屏（屏幕）声音的播放增益（0.0–4.0）；无需事件回传，前端持有滑块真值
#[tauri::command]
pub fn set_screen_gain(state: State<AppState>, gain: f32) {
    let g = gain.clamp(0.0, 4.0);
    state
        .shared
        .screen_gain
        .store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
    state.config.lock().unwrap().screen_gain = g;
    persist(&state);
}

/// 枚举音频输入/输出设备（wasapi；含系统默认标记）。
/// async：COM 枚举跑在线程池而非主线程——同步命令在主线程执行，投屏中逐帧上行任务排队时会拖延
/// 枚举完成，冻结 WebView2 宿主 UI，导致设备下拉"按下瞬间收起"。
#[tauri::command]
pub async fn list_audio_devices() -> Result<serde_json::Value, String> {
    let inputs = crate::audio::device::list(&wasapi::Direction::Capture).map_err(|e| e.to_string())?;
    let outputs = crate::audio::device::list(&wasapi::Direction::Render).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "inputs": inputs, "outputs": outputs }))
}

/// 选择麦克风（空 = 系统默认）：写运行时偏好（采集线程下一轮热切换）+ 持久化
#[tauri::command]
pub fn set_input_device(state: State<AppState>, id: String) {
    use crate::audio::session::device_pref;
    *state.shared.input_device.lock().unwrap() = device_pref(&id);
    state.config.lock().unwrap().input_device = id.trim().to_string();
    persist(&state);
}

/// 选择扬声器（空 = 系统默认）：写运行时偏好（播放线程下一轮热切换）+ 持久化
#[tauri::command]
pub fn set_output_device(state: State<AppState>, id: String) {
    use crate::audio::session::device_pref;
    *state.shared.output_device.lock().unwrap() = device_pref(&id);
    state.config.lock().unwrap().output_device = id.trim().to_string();
    persist(&state);
}

/// 选择摄像头（空 = 系统默认）：仅持久化（前端读取后应用于 getUserMedia）
#[tauri::command]
pub fn set_camera_device(state: State<AppState>, id: String) {
    state.config.lock().unwrap().camera_device = id.trim().to_string();
    persist(&state);
}

/// 选择主题（id 由前端校验回退；仅持久化——前端读 config 应用）
#[tauri::command]
pub fn set_theme(state: State<AppState>, theme: String) {
    state.config.lock().unwrap().theme = theme;
    persist(&state);
}

/// 选择音效方案（"default" / "none" / 未来 id；仅持久化——播放端读共享状态）
#[tauri::command]
pub fn set_sound_pack(state: State<AppState>, pack: String) {
    state.config.lock().unwrap().sound_pack = pack;
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
        Bridge { app: app.clone() },
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

/// R11：聊天正文里的链接 → 系统默认浏览器打开。前端已做识别，这里是第二道闸。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if !is_web_url(url) {
        return Err("仅支持 http/https 链接".into());
    }
    open_in_browser(url).map_err(|e| format!("打开浏览器失败：{e}"))
}

/// 仅接受 http:// 或 https:// 开头的地址（大小写不敏感）
fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// ShellExecuteW 以 "open" 动词交给系统默认浏览器
fn open_in_browser(url: &str) -> anyhow::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let h: HINSTANCE = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Win32 约定：返回值 <= 32 表示失败
    if h.0 as isize <= 32 {
        anyhow::bail!("ShellExecuteW 返回 {}", h.0 as isize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_web_url, sniff_mime};

    #[test]
    fn is_web_url_accepts_http_https_case_insensitive() {
        assert!(is_web_url("http://b23.tv/abc"));
        assert!(is_web_url("https://www.bilibili.com/video/BV1xx?p=2&t=3"));
        assert!(is_web_url("HTTPS://Example.com"));
        assert!(is_web_url("  https://example.com  "));
    }

    #[test]
    fn is_web_url_rejects_other_schemes_and_plain_text() {
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("file:///C:/Windows/System32"));
        assert!(!is_web_url("ftp://example.com"));
        assert!(!is_web_url("www.bilibili.com")); // 前端补全 https:// 后才交给后端
        assert!(!is_web_url(""));
    }

    #[test]
    fn sniff_mime_recognizes_png_jpeg_webp() {
        assert_eq!(
            sniff_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00]),
            Some("image/png")
        );
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        webp.extend_from_slice(b"WEBP");
        assert_eq!(sniff_mime(&webp), Some("image/webp"));
    }

    #[test]
    fn sniff_mime_rejects_unknown() {
        assert_eq!(sniff_mime(b"GIF89a"), None);
        assert_eq!(sniff_mime(&[]), None);
        assert_eq!(sniff_mime(b"RIFF"), None); // 长度不足 12：不 panic
    }
}
