# EchoRoom 计划2 · D 子项目（设置系统）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增全窗口设置页（账号 / 设备 / 音效 / 外观四分类）：改资料与退出登录入口、音频输入/输出设备枚举与热切换、摄像头设备选择、进出音效方案（Default / 关闭 / 可扩展）、6 套主题与自定义背景图——客户端 0.3.0。

**Architecture:** 纯客户端改动（protocol/server 零改动）。Rust 侧：wasapi 设备枚举 + 音频管线线程内软切换（麦克风换句柄 / 扬声器播放线程外层重初始化循环，抖动缓冲与解码器状态保留）；bridge 新增 9 个命令与 1 个回退事件。前端：顶栏以下的覆盖式设置页（4 pane），主题走 CSS 变量组（`:root[data-theme]`），背景图存本地数据目录单文件（`background.img`）经 bytes→Blob 铺层，音效按目录约定 + 注册表扩展。

**Tech Stack:** Rust（wasapi 0.15 设备枚举/热切换；tauri v2 命令与事件）+ 原生 HTML/CSS/JS（无构建工具）。

## Global Constraints

- **零协议/服务器改动**：D 全部落在客户端；服务器不需重部署
- **旧配置兼容**：config 新字段一律 `#[serde(default)]`；升级首启不丢旧设置
- **前端无构建工具**：每次改 JS 后 `node --check client/ui/app.js`（含 `video_capture.js`）必须无输出
- **主题变量纪律**：改样式一律走 CSS 变量；本次把既有硬编码派生色（`rgba(59,130,246,.15)`、`rgba(248,113,113,.15)`、`#262b36`、`#15181f`）收编为变量 `--accent-soft` / `--err-soft` / `--panel-hover` / `--panel-deep`；此后加主题只加一组变量
- **设备回退纪律**：任何音频设备打开失败 → 回退系统默认 + emit `audio_device_fallback`（方向 + 原因）；运行时偏好清空（`None`），由前端收到事件后调 `set_*_device("")` 持久化并刷新下拉
- **音效扩展约定**：新音效 = 放 `client/ui/sounds/<id>/in.mp3` + `out.mp3`，并在 `SOUND_PACKS` 注册一行；`"none"` 为关闭特例
- **功能开关纪律**：`client/ui/features.js` 为唯一开关源（`sound` / `theme` / `background`，缺省全开）；关闭的功能 = 界面不渲染 + 行为不接线（音效播放短路 / 主题启动不套用 / 背景图不读不铺）；新增可关功能照此模式加一行
- **背景图**：单文件 `%APPDATA%\com.echoroom.dev\background.img`（无扩展名，MIME 由文件头嗅探）；上限 10MB；仅 PNG/JPEG/WebP；换图直接覆盖、清除即删文件（无 config 字段——文件存在即生效）
- **层级纪律**：设置页 `z-index: 22`（盖住观看视图 20/25，低于弹层 30）；资料弹窗 > 设置页 > 主界面
- **每任务收尾**：`cargo test -p echoroom-client` 全绿 + 相关 `node --check`；末尾按步骤提交（仓库在 `dev` 分支）
- **版本**：仅 `client/src-tauri/tauri.conf.json` 升 0.3.0（workspace `Cargo.toml` 的 0.1.0 不动，与 C 做法一致）
- 环境：Windows PowerShell（`;` 分隔命令）；cargo 使用 workspace 共享 target（产物在仓库根 `target/`）

## File Structure（全景）

**client / src-tauri（客户端 Rust）**
- `src/config.rs`：+5 字段（input_device / output_device / camera_device / theme / sound_pack）+ 测试
- `src/audio/device.rs`（新）：wasapi 设备枚举（DeviceInfo）与按 id 查找
- `src/audio/mod.rs`：+`pub mod device`
- `src/audio/capture.rs`：`MicCapture::open(device_id: Option<&str>)`
- `src/audio/playback.rs`：`init_render(device_id)` + `run_player` 外层重初始化循环（`Control::Restart`）+ `spawn_player` 增参
- `src/audio/session.rs`：`SharedAudio` +2 偏好字段、`device_pref()`、采集线程热切换、`spawn_audio_pipeline` 增 `Bridge` 参数
- `src/bridge.rs`：新命令 `list_audio_devices` / `set_input_device` / `set_output_device` / `set_camera_device` / `set_theme` / `set_sound_pack` / `set_background` / `clear_background` / `get_background` / `logout`；`audio_device_fallback` 事件
- `src/lib.rs`：SharedAudio 构造与命令注册更新

**client / ui（前端）**
- `features.js`（新）：功能开关（构建前配置；sound / theme / background）
- `index.html`：顶栏改造（Echo 放大 + ⚙）；设置页骨架（4 pane 内容逐步补齐）；资料弹窗标题「个人资料」；引入 features.js
- `style.css`：`--topbar-h` 与顶栏、设置页布局、6 组主题变量、4 处硬编码色变量化、音效块样式、背景图铺层
- `app.js`：设置页逻辑（开关/分类/账号/设备/音效/主题/背景/登出）、功能开关接线（FEATURES）、音效注册表与播放改造
- `video_capture.js`：摄像头设备（deviceId 应用 + 运行中重启 + 回退）
- `sounds/default/in.mp3`、`out.mp3`（从 ui 根移入）
- `tauri.conf.json`：版本 0.3.0

---

### Task 1: config 新增设置字段（设备 / 主题 / 音效）

**Files:**
- Modify: `client/src-tauri/src/config.rs`（字段 + Default + 测试）

**Interfaces:**
- Consumes: 现有 `Config`（serde + 手写 Default + 测试模块）
- Produces（后续任务依赖）：
  - `Config.input_device: String` / `Config.output_device: String` / `Config.camera_device: String`（空 = 系统默认）
  - `Config.theme: String`（默认 `"dianlan"`）
  - `Config.sound_pack: String`（默认 `"default"`）

- [ ] **Step 1: 写失败测试——新字段 round-trip 与旧配置兼容**

`client/src-tauri/src/config.rs` 的测试模块（`old_config_without_volume_fields_loads_defaults` 之后）追加：

```rust
    #[test]
    fn roundtrip_with_settings_fields() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_settings.json");
        let mut cfg = Config::default();
        cfg.input_device = "dev-in-1".into();
        cfg.output_device = "dev-out-2".into();
        cfg.camera_device = "cam-3".into();
        cfg.theme = "anzi".into();
        cfg.sound_pack = "none".into();
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_config_without_settings_fields_loads_defaults() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_old2.json");
        std::fs::write(&path, r#"{"server_addr":"127.0.0.1:9000","account":"alice"}"#).unwrap();
        let loaded = Config::load(&path);
        assert!(loaded.input_device.is_empty());
        assert!(loaded.output_device.is_empty());
        assert!(loaded.camera_device.is_empty());
        assert_eq!(loaded.theme, "dianlan");
        assert_eq!(loaded.sound_pack, "default");
        let _ = std::fs::remove_file(&path);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-client config::`
Expected: 编译错误（`no field input_device`）——新增字段前预期失败。

- [ ] **Step 3: 实现字段 + 默认值**

`config.rs` 顶部默认函数区（`default_share_quality` 之后）加：

```rust
fn default_theme() -> String {
    "dianlan".into()
}

fn default_sound_pack() -> String {
    "default".into()
}
```

结构体 `share_audio` 字段之后加：

```rust
    /// 音频输入设备 id（空 = 系统默认）
    #[serde(default)]
    pub input_device: String,
    /// 音频输出设备 id（空 = 系统默认）
    #[serde(default)]
    pub output_device: String,
    /// 摄像头设备 id（空 = 系统默认）
    #[serde(default)]
    pub camera_device: String,
    /// 界面主题 id（"dianlan" / "anzi" / "molv" / "yingfen" / "hupo" / "qinglan"）
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 进出音效方案（"default" / "none" / 未来 id）
    #[serde(default = "default_sound_pack")]
    pub sound_pack: String,
```

`Default` impl 补齐（`share_audio: true` 之后）：

```rust
            input_device: String::new(),
            output_device: String::new(),
            camera_device: String::new(),
            theme: "dianlan".into(),
            sound_pack: "default".into(),
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p echoroom-client config::`
Expected: `test result: ok`（4 个 config 测试全绿）。

- [ ] **Step 5: 提交**

```powershell
git add client/src-tauri/src/config.rs; git commit -m "feat(client): add settings config fields (devices/theme/sound)"
```

---

### Task 2: 设备枚举模块 + 设备偏好共享 + 6 个 bridge 命令

**Files:**
- Create: `client/src-tauri/src/audio/device.rs`
- Modify: `client/src-tauri/src/audio/mod.rs`、`src/audio/session.rs`、`src/bridge.rs`、`src/lib.rs`

**Interfaces:**
- Consumes: `wasapi::DeviceCollection`（0.15：`new(&Direction)` / `get_nbr_devices()` / `get_device_at_index(u32)`）；`Device::get_id()` / `get_friendlyname()`；现有 `persist()`（bridge）
- Produces（后续任务依赖）：
  - `audio::device::DeviceInfo { id: String, name: String, is_default: bool }`（`serde::Serialize`）
  - `audio::device::list(&Direction) -> anyhow::Result<Vec<DeviceInfo>>`
  - `audio::device::find(&Direction, id: &str) -> anyhow::Result<Option<wasapi::Device>>`
  - `audio::session::device_pref(s: &str) -> Option<String>`（空/空白 = None）
  - `SharedAudio.input_device` / `SharedAudio.output_device`：`Arc<Mutex<Option<String>>>`（None = 系统默认）
  - `SharedAudio::new(self_gain, muted, peer_gains, screen_gain, input_device, output_device)`
  - 命令：`list_audio_devices` / `set_input_device` / `set_output_device` / `set_camera_device` / `set_theme` / `set_sound_pack`

- [ ] **Step 1: 新建 device.rs（枚举与查找）**

`client/src-tauri/src/audio/device.rs`：

```rust
//! 音频设备枚举与查找：wasapi 设备集合（输入/输出）→ 设备信息；按 id 查找指定设备。
//! COM 必须在同一线程初始化与使用：本模块函数在音频线程或命令线程内调用。
use anyhow::{Context, Result};
use wasapi::{Device, DeviceCollection, Direction};

/// wasapi 0.15 的错误类型为 `Box<dyn Error>`（非 Send+Sync，无法直接进 anyhow），统一转字符串。
fn err2any(e: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// 设备信息（bridge → UI 序列化用；纯数据，可跨线程）
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DeviceInfo {
    /// 端点 id（持久化到 config；wasapi Device::get_id）
    pub id: String,
    /// 友好名（UI 下拉显示）
    pub name: String,
    /// 是否为当前系统默认设备
    pub is_default: bool,
}

/// 枚举某方向的活跃设备（DEVICE_STATE_ACTIVE；含 is_default 标记）
pub fn list(direction: &Direction) -> Result<Vec<DeviceInfo>> {
    wasapi::initialize_mta().ok().context("initialize COM (MTA)")?;
    let default_id = wasapi::get_default_device(direction)
        .ok()
        .and_then(|d| d.get_id().ok());
    let col = DeviceCollection::new(direction).map_err(err2any).context("设备集合")?;
    let n = col.get_nbr_devices().map_err(err2any).context("设备数量")?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let Ok(dev) = col.get_device_at_index(i) else { continue };
        let id = dev.get_id().map_err(err2any)?;
        let name = dev.get_friendlyname().map_err(err2any)?;
        out.push(DeviceInfo { is_default: default_id.as_deref() == Some(id.as_str()), id, name });
    }
    Ok(out)
}

/// 按 id 查找设备；找不到返回 None（由调用方决定回退系统默认）
pub fn find(direction: &Direction, id: &str) -> Result<Option<Device>> {
    wasapi::initialize_mta().ok().context("initialize COM (MTA)")?;
    let col = DeviceCollection::new(direction).map_err(err2any).context("设备集合")?;
    let n = col.get_nbr_devices().map_err(err2any).context("设备数量")?;
    for i in 0..n {
        let Ok(dev) = col.get_device_at_index(i) else { continue };
        if dev.get_id().map_err(err2any)? == id {
            return Ok(Some(dev));
        }
    }
    Ok(None)
}
```

- [ ] **Step 2: mod.rs 注册模块**

`client/src-tauri/src/audio/mod.rs`，`pub mod denoise;` 之后加：

```rust
pub mod device;
```

- [ ] **Step 3: session.rs——device_pref 测试先行**

`client/src-tauri/src/audio/session.rs` 测试模块（`speaking_report_mutes_and_realigns` 之后）加：

```rust
    #[test]
    fn device_pref_maps_empty_to_none() {
        assert_eq!(device_pref(""), None);
        assert_eq!(device_pref("   "), None);
        assert_eq!(device_pref("id-1"), Some("id-1".to_string()));
    }
```

Run: `cargo test -p echoroom-client device_pref`
Expected: 编译错误（函数不存在）——预期失败。

- [ ] **Step 4: session.rs——SharedAudio 加偏好字段 + device_pref 实现**

`session.rs` 的 `SharedAudio` 结构（`viewer_count` 之后）加：

```rust
    /// 输入设备偏好（None = 系统默认；bridge 写、采集线程读）
    pub input_device: Arc<std::sync::Mutex<Option<String>>>,
    /// 输出设备偏好（None = 系统默认；bridge 写、播放线程读）
    pub output_device: Arc<std::sync::Mutex<Option<String>>>,
```

`impl SharedAudio` 的 `new` 签名与 body 更新（在 `viewer_count: Arc::new(AtomicU16::new(0)),` 之后加两行）：

```rust
    pub fn new(
        self_gain: f32,
        muted: bool,
        peer_gains: HashMap<String, f32>,
        screen_gain: f32,
        input_device: Option<String>,
        output_device: Option<String>,
    ) -> Self {
        SharedAudio {
            self_gain: Arc::new(AtomicU32::new(self_gain.to_bits())),
            self_muted: Arc::new(AtomicBool::new(muted)),
            peer_gains: Arc::new(std::sync::Mutex::new(peer_gains)),
            screen_gain: Arc::new(AtomicU32::new(screen_gain.to_bits())),
            uid_names: Arc::new(std::sync::Mutex::new(HashMap::new())),
            my_streams: Arc::new(AtomicU8::new(0)),
            viewer_count: Arc::new(AtomicU16::new(0)),
            input_device: Arc::new(std::sync::Mutex::new(input_device)),
            output_device: Arc::new(std::sync::Mutex::new(output_device)),
        }
    }
```

文件顶部（`impl SharedAudio` 之前）加：

```rust
/// config 的设备字符串 → 运行时偏好（空/空白 = 系统默认 = None）
pub fn device_pref(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
```

- [ ] **Step 5: lib.rs——SharedAudio 构造更新**

`client/src-tauri/src/lib.rs` 的 `SharedAudio::new(...)` 调用改为：

```rust
    let shared = audio::session::SharedAudio::new(
        cfg.self_gain,
        cfg.muted,
        cfg.peer_gains.clone(),
        cfg.screen_gain,
        audio::session::device_pref(&cfg.input_device),
        audio::session::device_pref(&cfg.output_device),
    );
```

- [ ] **Step 6: bridge.rs——6 个设置命令（设备 / 主题 / 音效）**

`client/src-tauri/src/bridge.rs`，`set_screen_gain` 命令之后加：

```rust
/// 枚举音频输入/输出设备（wasapi；含系统默认标记）
#[tauri::command]
pub fn list_audio_devices() -> Result<serde_json::Value, String> {
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
```

- [ ] **Step 7: lib.rs——命令注册**

`invoke_handler` 列表中 `bridge::set_screen_gain,` 之后加：

```rust
            bridge::list_audio_devices,
            bridge::set_input_device,
            bridge::set_output_device,
            bridge::set_camera_device,
            bridge::set_theme,
            bridge::set_sound_pack,
```

- [ ] **Step 8: 编译 + 测试**

Run: `cargo test -p echoroom-client`
Expected: 全绿（含 `device_pref_maps_empty_to_none`）。

Run: `cargo build`
Expected: workspace 编译通过。

- [ ] **Step 9: 提交**

```powershell
git add client/src-tauri/src/audio/device.rs client/src-tauri/src/audio/mod.rs client/src-tauri/src/audio/session.rs client/src-tauri/src/bridge.rs client/src-tauri/src/lib.rs; git commit -m "feat(client): audio device enumeration, prefs and bridge commands"
```

---

### Task 3: 麦克风热切换（采集线程内换设备）

**Files:**
- Modify: `client/src-tauri/src/audio/capture.rs`（open 增参）
- Modify: `client/src-tauri/src/audio/session.rs`（采集线程热切换 + `spawn_audio_pipeline` 增 `Bridge` 参数）
- Modify: `client/src-tauri/src/bridge.rs`（`start_audio` 调用处传 Bridge）

**Interfaces:**
- Consumes: Task 2 的 `device::find`、`SharedAudio.input_device`
- Produces（后续任务依赖）：
  - `MicCapture::open(device_id: Option<&str>) -> Result<MicCapture>`（None = 系统默认；Some(id) 不存在 → Err）
  - `spawn_audio_pipeline(server_addr, uid, token, tcp_tx, shared, bridge: Bridge)`（+1 参数）
  - 事件 `audio_device_fallback`：`{ direction: "input", reason: String }`

- [ ] **Step 1: capture.rs——open 支持指定设备**

`client/src-tauri/src/audio/capture.rs` 的 `pub fn open()` 签名与设备获取段替换为（其余方法不动）：

```rust
    pub fn open(device_id: Option<&str>) -> Result<MicCapture> {
        wasapi::initialize_mta()
            .ok()
            .context("initialize COM (MTA)")?;
        let device = match device_id {
            Some(id) => crate::audio::device::find(&Direction::Capture, id)
                .context("查找所选麦克风")?
                .ok_or_else(|| anyhow::anyhow!("所选麦克风不存在"))?,
            None => wasapi::get_default_device(&Direction::Capture)
                .map_err(err2any)
                .context("default capture device")?,
        };
        let mut client = device
            .get_iaudioclient()
            .map_err(err2any)
            .context("IAudioClient")?;
```

（原 `let device = wasapi::get_default_device(...)...;` 两段被上面的 match 替换；后续 `if let Ok(mix) = client.get_mixformat()` 起全部保持原样。）

- [ ] **Step 2: session.rs——spawn_audio_pipeline 增 Bridge 参数**

`session.rs` 顶部 import 区加：

```rust
use crate::bridge::Bridge;
```

`pub fn spawn_audio_pipeline(` 签名末尾（`shared: SharedAudio,` 之后）加参数，函数体开头无论どこ需保证 `bridge` 可用：

```rust
pub fn spawn_audio_pipeline(
    server_addr: String,
    uid: u16,
    token: u32,
    tcp_tx: std::sync::mpsc::Sender<NetCmd>,
    shared: SharedAudio,
    bridge: Bridge,
) -> anyhow::Result<AudioHandle> {
```

- [ ] **Step 3: session.rs——采集线程热切换**

将现有"采集线程"整块（`// 采集线程：MicCapture → 累积 960 → ...` 的 `{ ... }` 块，含 `std::thread::spawn`）替换为：

```rust
/// 按偏好打开麦克风；所选设备不可用 → 清偏好 + 上报 `audio_device_fallback` + 回退系统默认。
/// None = 连系统默认都打不开（调用方决定中断/放弃）。启动与切换两条路径共用。
fn open_mic(input_device: &std::sync::Mutex<Option<String>>, bridge: &Bridge) -> Option<MicCapture> {
    let pref = input_device.lock().unwrap().clone();
    match MicCapture::open(pref.as_deref()) {
        Ok(m) => Some(m),
        Err(e) => {
            if let Some(bad) = pref {
                eprintln!("[audio] 麦克风 `{bad}` 不可用: {e:#}，回退系统默认");
                input_device.lock().unwrap().take(); // 运行时清空；前端据事件持久化
                let _ = bridge.app.emit(
                    "audio_device_fallback",
                    serde_json::json!({ "direction": "input", "reason": e.to_string() }),
                );
                match MicCapture::open(None) {
                    Ok(m) => Some(m),
                    Err(e2) => {
                        eprintln!("[audio] 默认麦克风不可用: {e2:#}");
                        None
                    }
                }
            } else {
                eprintln!("[audio] 麦克风不可用: {e:#}");
                None
            }
        }
    }
}

    // 采集线程：MicCapture → 累积 960 → 降噪 →（增益）→ VAD → tx_pcm；
    // 每轮检查设备偏好变化 → 线程内热切换（保降噪/VAD 状态，仅换句柄）
    {
        let stop = stop.clone();
        let self_gain = shared.self_gain.clone();
        let self_muted = shared.self_muted.clone();
        let input_device = shared.input_device.clone();
        let bridge = bridge.clone();
        std::thread::spawn(move || {
            // 按偏好打开（所选设备不可用 → 自动回退系统默认 + 上报，与切换路径同一逻辑）
            let mut mic = match open_mic(&input_device, &bridge) {
                Some(m) => m,
                None => return,
            };
            let mut pref = input_device.lock().unwrap().clone(); // 实际生效偏好（可能已回退为 None）
            let mut denoiser = Denoiser::new();
            // 阈值实测自降噪后信号（底噪残留 ≈ 17、语音 ≥ 200）：进入 50 / 退出 25
            let mut detector = SpeakingDetector::new(50.0, 25.0, Duration::from_millis(400));
            let mut pending: Vec<i16> = Vec::with_capacity(1920);
            let mut was_muted = false;
            while !stop.load(Ordering::Relaxed) {
                // 设备偏好变化 → 热切换（失效自动回退 + 上报，逻辑见 open_mic）
                let want = input_device.lock().unwrap().clone();
                if want != pref {
                    match open_mic(&input_device, &bridge) {
                        Some(m) => {
                            mic = m;
                            pref = input_device.lock().unwrap().clone(); // 归一化后的实际偏好
                            println!("[audio] 麦克风已切换");
                        }
                        None => return, // 默认设备也不可用：放弃采集（恢复由下次启动/重连触发）
                    }
                }
                if let Err(e) = mic.pump(&mut pending) {
                    eprintln!("[audio] 采集错误: {e:#}");
                    break;
                }
                while pending.len() >= FRAME_SAMPLES {
                    let mut block: Vec<i16> = pending.drain(..FRAME_SAMPLES).collect();
                    denoiser.process(&mut block); // 960 = 480×2 帧，整倍数合法
                    // 采集增益：denoise 后、VAD 前（增益调大 → VAD 更灵敏，符合直觉）
                    let g = f32::from_bits(self_gain.load(Ordering::Relaxed));
                    if (g - 1.0).abs() > 1e-6 {
                        for s in block.iter_mut() {
                            *s = ((*s as f32) * g).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                        }
                    }
                    let rms = (block.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                        / block.len() as f64)
                        .sqrt();
                    // VAD 静音期间照跑：状态机连续，解除静音后状态立即正确
                    let flip = detector.update(rms, std::time::Instant::now());
                    let now_muted = self_muted.load(Ordering::Relaxed);
                    if let Some(on) = speaking_report(flip, now_muted, was_muted, detector.speaking()) {
                        let _ = tcp_tx.send(NetCmd::SetSpeaking(on)); // 无界队列：不阻塞
                    }
                    was_muted = now_muted;
                    // 静音不发包：仅说话状态（含 400ms 保持）且未静音期间发送；接收端 PLC 超时静默兜底
                    if !now_muted && detector.speaking() {
                        let _ = tx_pcm.try_send(block); // 队列满：丢块（接收端按缺帧 PLC）
                    }
                }
            }
        });
    }
```

（`tx_pcm` 在闭包外已 clone 好——现有代码 `let tx_pcm = udp_tx.tx_pcm.clone();` 在 spawn 前执行 ✓ 保持不动。`open_mic` 为模块级函数：定义在 `spawn_audio_pipeline` 之后；`Bridge` 已在 Step 2 导入。）

- [ ] **Step 4: bridge.rs——start_audio 传 Bridge**

`bridge.rs` 的 `start_audio` 内 `spawn_audio_pipeline(...)` 调用尾部补参数：

```rust
    match crate::audio::session::spawn_audio_pipeline(
        server_addr,
        uid,
        token,
        tcp_tx,
        state.shared.clone(),
        Bridge { app: app.clone() },
    ) {
```

（`start_audio(app: &AppHandle, ...)` 已有 `app` 引用；`Bridge` 已在 bridge.rs 内定义。）

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p echoroom-client`
Expected: 全绿。

Run: `cargo build`
Expected: workspace 编译通过。

- [ ] **Step 6: 提交**

```powershell
git add client/src-tauri/src/audio/capture.rs client/src-tauri/src/audio/session.rs client/src-tauri/src/bridge.rs; git commit -m "feat(client): hot-swap microphone inside capture thread"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：双客户端语音中切换麦克风 → 对方实际听到声音来源变化、语音不中断；拔出所选麦克风后再说话 → 回退系统默认并收到 audio_device_fallback 事件（前端提示在 Task 7 接入）；启动时配置的麦克风已失效 → 同样自动回退默认 + 事件上报。
```

---

### Task 4: 扬声器热切换（播放线程外层重初始化循环）

**Files:**
- Modify: `client/src-tauri/src/audio/playback.rs`（全量替换）
- Modify: `client/src-tauri/src/audio/session.rs`（`spawn_player` 调用处）

**Interfaces:**
- Consumes: Task 2 的 `device::find`、`SharedAudio.output_device`、`Bridge`
- Produces（后续任务依赖）：
  - `spawn_player(fill, output_device: Arc<Mutex<Option<String>>>, bridge: Bridge) -> Result<PlayerHandle>`（+2 参数）
  - 内部 `enum Control { Stop, Restart }`；`init_render(device_id: Option<&str>)`
  - 事件 `audio_device_fallback`：`{ direction: "output", reason: String }`

- [ ] **Step 1: playback.rs 全量替换**

`client/src-tauri/src/audio/playback.rs` 全文替换为：

```rust
//! WASAPI 播放：选定输出设备（默认或指定）、48kHz i16 输出（信号单声道，双声道左右同源）、事件驱动 + 填充回调。
//! 设备热切换：run_player 外层重初始化循环——render_loop 检测偏好变化后主动退出，同一 fill 闭包跨设备保留全部状态。
//! COM 必须在同一线程初始化与使用：设备对象全部在播放线程内创建并持有。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use wasapi::{AudioRenderClient, BufferFlags, Direction, SampleType, ShareMode, WaveFormat};

use echoroom_protocol::SAMPLE_RATE;

use crate::bridge::Bridge;

/// 输出按双声道 i16 请求：每帧 4 字节（信号仍单声道，左右同源复制）
const OUT_BYTES_PER_FRAME: usize = 4;

/// wasapi 0.15 的错误类型为 `Box<dyn Error>`（非 Send+Sync，无法直接进 anyhow），
/// 统一转成字符串错误。
fn err2any(e: Box<dyn std::error::Error>) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// 播放器句柄：`stop` 置位后播放线程退出；Drop 时自动置位。
pub struct PlayerHandle {
    pub stop: Arc<AtomicBool>,
}

impl Drop for PlayerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 启动播放线程：`fill` 回调负责填充每个播放周期的单声道样本（长度 = 本次可写帧数）。
/// `output_device` 为输出设备偏好（None = 系统默认；变化即热切换）。
/// 返回前会等待设备初始化完成；首轮初始化失败直接报错。
pub fn spawn_player<F>(
    fill: F,
    output_device: Arc<Mutex<Option<String>>>,
    bridge: Bridge,
) -> Result<PlayerHandle>
where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (ready_tx, ready_rx) = channel::<Result<()>>();

    std::thread::spawn(move || run_player(fill, &stop_thread, &ready_tx, &output_device, &bridge));

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(PlayerHandle { stop }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("player thread exited before init")),
    }
}

/// 渲染循环退出控制：正常停止 / 设备偏好变化需重初始化
enum Control {
    Stop,
    Restart,
}

fn run_player<F>(
    mut fill: F,
    stop: &AtomicBool,
    ready: &Sender<Result<()>>,
    output_device: &Mutex<Option<String>>,
    bridge: &Bridge,
) where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    let mut first = true;
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let mut pref = output_device.lock().unwrap().clone();
        // 打开目标设备；所选设备打不开（含启动时已失效）→ 清偏好 + 上报事件 + 回退系统默认
        let mut init = init_render(pref.as_deref());
        if init.is_err() && pref.is_some() {
            let e = init.unwrap_err();
            eprintln!("[audio] 所选输出设备不可用: {e:#}，回退系统默认");
            output_device.lock().unwrap().take(); // 运行时清空；前端据事件持久化
            let _ = bridge.app.emit(
                "audio_device_fallback",
                serde_json::json!({ "direction": "output", "reason": e.to_string() }),
            );
            pref = None;
            init = init_render(None);
        }
        let (client, render, event) = match init {
            Ok(triple) => triple,
            Err(e) => {
                if first {
                    // 系统默认也失败：保持 spawn_player 现有语义（直接报错，线程退出）
                    let _ = ready.send(Err(e));
                    return;
                }
                // 运行期默认设备被拔（等）：等待重试（stop 在循环头检查）
                eprintln!("[audio] 默认输出设备不可用: {e:#}，稍后重试");
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
        };
        if first {
            let _ = ready.send(Ok(()));
            first = false;
        }
        match render_loop(&mut fill, &client, &render, &event, stop, output_device, pref) {
            Ok(Control::Stop) => return,
            Ok(Control::Restart) => continue,
            Err(e) => {
                eprintln!("[audio] 播放线程异常退出: {e:#}");
                return;
            }
        }
    }
}

/// 设备初始化：按偏好打开（None = 系统默认），返回 (AudioClient, AudioRenderClient, 事件句柄)。
fn init_render(device_id: Option<&str>) -> Result<(wasapi::AudioClient, AudioRenderClient, wasapi::Handle)> {
    wasapi::initialize_mta()
        .ok()
        .context("initialize COM (MTA)")?;
    let device = match device_id {
        Some(id) => crate::audio::device::find(&Direction::Render, id)
            .context("查找所选扬声器")?
            .ok_or_else(|| anyhow::anyhow!("所选扬声器不存在"))?,
        None => wasapi::get_default_device(&Direction::Render)
            .map_err(err2any)
            .context("default render device")?,
    };
    let mut client = device
        .get_iaudioclient()
        .map_err(err2any)
        .context("IAudioClient")?;
    // 双声道请求：信号仍为单声道，写出前由本模块复制左右声道（听感居中、双耳平衡）；
    // 共享模式 + convert=true：输出侧同样走 AUTOCONVERTPCM，统一 48k/i16。
    let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE as usize, 2, None);
    client
        .initialize_client(
            &format,
            200_000, // 20ms 缓冲（低延迟；系统会向上对齐到设备周期）
            &Direction::Render,
            &ShareMode::Shared,
            true,
        )
        .map_err(err2any)
        .context("initialize render client")?;
    let render = client
        .get_audiorenderclient()
        .map_err(err2any)
        .context("render client")?;
    let buf_frames = client.get_bufferframecount().map_err(err2any)? as usize;
    println!(
        "播放缓冲: {} 帧 (~{:.0}ms)",
        buf_frames,
        buf_frames as f64 * 1000.0 / SAMPLE_RATE as f64
    );
    // 启动前把当前可用空间填满静音，避免上电瞬间欠载产生爆音
    let prefill = client.get_available_space_in_frames().map_err(err2any)? as usize;
    if prefill > 0 {
        let silence = vec![0u8; prefill * OUT_BYTES_PER_FRAME];
        render
            .write_to_device(prefill, &silence, Some(BufferFlags::none()))
            .map_err(err2any)
            .context("prefill silence")?;
    }
    let event = client
        .set_get_eventhandle()
        .map_err(err2any)
        .context("set event handle")?;
    client
        .start_stream()
        .map_err(err2any)
        .context("start stream")?;
    Ok((client, render, event))
}

fn render_loop<F>(
    fill: &mut F,
    client: &wasapi::AudioClient,
    render: &AudioRenderClient,
    event: &wasapi::Handle,
    stop: &AtomicBool,
    output_device: &Mutex<Option<String>>,
    active_pref: Option<String>,
) -> Result<Control>
where
    F: FnMut(&mut [i16]) + Send + 'static,
{
    while !stop.load(Ordering::Relaxed) {
        // 设备偏好变化 → 停流并交给外层用新设备重初始化（fill 状态全保留）
        if output_device.lock().unwrap().clone() != active_pref {
            client.stop_stream().map_err(err2any)?;
            return Ok(Control::Restart);
        }
        // 事件驱动；10ms 超时兜底轮询（事件可能不触发；也保证 stop 快速响应）
        let _ = event.wait_for_event(10);
        let avail = client.get_available_space_in_frames().map_err(err2any)? as usize;
        if avail == 0 {
            continue;
        }
        let mut buf = vec![0i16; avail];
        fill(&mut buf);
        // 单声道样本 → 双声道交错（左右同源；Windows 小端，与采集侧 from_le_bytes 对称）
        let mut bytes = Vec::with_capacity(buf.len() * OUT_BYTES_PER_FRAME);
        for s in &buf {
            let b = s.to_le_bytes();
            bytes.extend_from_slice(&b);
            bytes.extend_from_slice(&b);
        }
        render
            .write_to_device(avail, &bytes, Some(BufferFlags::none()))
            .map_err(err2any)?;
    }
    client.stop_stream().map_err(err2any)?;
    Ok(Control::Stop)
}
```

- [ ] **Step 2: session.rs——spawn_player 调用更新**

`session.rs` 播放线程块尾部（`})?;` 处）的调用改为：

```rust
    let player = crate::audio::playback::spawn_player(
        move |out| {
            // ...（闭包体完全保持原样：收流/收屏幕音频/出流混音不动）...
        },
        shared.output_device.clone(),
        bridge.clone(),
    )?;
```

（只加最后两个实参；闭包体一行不改。）

- [ ] **Step 3: 编译 + 测试**

Run: `cargo test -p echoroom-client`
Expected: 全绿。

Run: `cargo build`
Expected: workspace 编译通过。

- [ ] **Step 4: 提交**

```powershell
git add client/src-tauri/src/audio/playback.rs client/src-tauri/src/audio/session.rs; git commit -m "feat(client): hot-swap output device via player reinit loop"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：双客户端语音中切换扬声器 → 声音从新设备输出、仅一瞬静默（抖动缓冲/解码器状态保留，无重连无爆音）；打开不存在的设备（拔掉后选择，含启动时配置已失效）→ 回退系统默认 + 事件上报；播放线程在默认设备也失败期间持续重试（插回即恢复）。
```

---

### Task 5: 前端设置页骨架（顶栏改造 + 覆盖页 + 分类切换 + 版本号）

**Files:**
- Create: `client/ui/features.js`（功能开关：sound / theme / background）
- Modify: `client/ui/index.html`（顶栏 + 设置页结构 + features.js 引入）
- Modify: `client/ui/style.css`（`--topbar-h`、顶栏与设置页样式、基础变量补充）
- Modify: `client/ui/app.js`（开关 / 分类切换 / 齿轮门控 / 版本号 / FEATURES 读取与分类隐藏）

**Interfaces:**
- Consumes: 无（纯骨架；各 pane 内容由 Task 6–9 补齐）
- Produces（后续任务依赖）：
  - `const FEATURES`（全局功能开关；各任务在渲染/接线处判断）
  - `openSettings()` / `closeSettings()` / `switchCat(cat)`
  - `const paneRefreshers = {}`：各页面任务注册刷新钩子（`paneRefreshers.device = refreshDevices` 等）
  - CSS 变量：`--topbar-h`、`--accent-soft`、`--err-soft`、`--panel-hover`、`--panel-deep`（默认值 = 现状色；Task 9 为其余主题重定义）
  - DOM 契约：`#settings-page` / `.settings-cat[data-cat]` / `.settings-pane[data-pane]` / `#settings-version` / `#open-settings`

- [ ] **Step 1: index.html——顶栏改造 + 设置页骨架**

`client/ui/index.html` 的 `<header class="topbar">...</header>` 替换为：

```html
  <header class="topbar">
    <span class="title">Echo</span>
    <div class="topbar-right">
      <span class="status" id="conn-status">未连接</span>
      <button class="topbar-gear" id="open-settings" title="设置" hidden></button>
    </div>
  </header>
```

`</div>`（`.app-body` 的结束标签，即 `</footer>` 后的那个）之后、`<div class="setup-mask hidden" id="setup-mask">` 之前插入：

```html
  <div class="settings-page" id="settings-page" hidden>
    <aside class="settings-nav">
      <button class="settings-back" id="settings-back">‹ 返回</button>
      <nav class="settings-cats">
        <button class="settings-cat active" data-cat="account">账号</button>
        <button class="settings-cat" data-cat="device">设备</button>
        <button class="settings-cat" data-cat="sound">音效</button>
        <button class="settings-cat" data-cat="appearance">外观</button>
      </nav>
      <div class="settings-version" id="settings-version">v0.3.0</div>
    </aside>
    <div class="settings-body">
      <section class="settings-pane" data-pane="account"></section>
      <section class="settings-pane" data-pane="device" hidden></section>
      <section class="settings-pane" data-pane="sound" hidden></section>
      <section class="settings-pane" data-pane="appearance" hidden></section>
    </div>
  </div>
```

文件底部脚本区，在 `<script src="video_capture.js"></script>` 之前（即脚本区第一行）插入：

```html
  <script src="features.js"></script>
```

- [ ] **Step 2: style.css——变量补充 + 顶栏替换 + 设置页样式**

`:root` 块（`--sidebar-w` 之前）加：

```css
  --topbar-h: 48px; /* 顶栏高度（设置页从其下方起覆盖） */
  /* 派生色（Task 9 各主题重定义；此处为默认主题值） */
  --accent-soft: rgba(59, 130, 246, .15);
  --err-soft: rgba(248, 113, 113, .15);
  --panel-hover: #262b36;
  --panel-deep: #15181f;
```

顶栏段（`/* ---- 顶栏：标题 + 连接状态 ---- */` 起至 `.topbar .status.offline` 行）整体替换为：

```css
/* ---- 顶栏：标题 + 连接状态 + 设置入口 ---- */
.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: var(--topbar-h);
  padding: 0 14px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.topbar .title { font-weight: 600; font-size: 20px; letter-spacing: .5px; }
.topbar-right { display: flex; align-items: center; gap: 10px; }
.topbar .status { font-size: 12px; color: var(--text-dim); }
.topbar .status.online { color: var(--ok); }
.topbar .status.offline { color: var(--err); }
.topbar-gear {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: 7px;
  background: none;
  color: var(--text-dim);
  cursor: pointer;
}
.topbar-gear:hover { background: var(--panel-hover); color: var(--text); }
.topbar-gear svg { width: 17px; height: 17px; }
.topbar-gear[hidden] { display: none; }
```

文件末尾追加：

```css
/* ---- D：设置页（覆盖顶栏以下整个窗口；主界面不销毁） ---- */
.settings-page {
  position: fixed;
  top: var(--topbar-h);
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 22; /* 盖住观看视图（20/25）；低于弹层（30） */
  display: flex;
  background: var(--bg);
}
.settings-page[hidden] { display: none; }
.settings-nav {
  width: 150px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 8px;
  border-right: 1px solid var(--border);
}
.settings-back {
  text-align: left;
  padding: 7px 10px;
  border: none;
  border-radius: 7px;
  background: none;
  color: var(--text-dim);
  font-size: 13px;
  cursor: pointer;
}
.settings-back:hover { background: var(--panel); color: var(--text); }
.settings-cats { display: flex; flex-direction: column; gap: 2px; margin-top: 8px; }
.settings-cat {
  text-align: left;
  padding: 7px 10px;
  border: none;
  border-radius: 7px;
  background: none;
  color: var(--text-dim);
  font-size: 13px;
  cursor: pointer;
}
.settings-cat:hover { background: var(--panel); color: var(--text); }
.settings-cat.active { background: var(--accent-soft); color: var(--accent); }
.settings-cat[hidden] { display: none; } /* 功能开关关闭的分类 */
.settings-version {
  margin-top: auto;
  padding: 0 10px 6px;
  font-size: 11px;
  color: var(--text-dim);
}
.settings-body {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px 20px;
}
.settings-pane[hidden] { display: none; }
.settings-title { font-size: 15px; font-weight: 600; margin: 0 0 14px; }
```

- [ ] **Step 3: features.js 创建 + app.js 齿轮图标与设置页逻辑**

新建 `client/ui/features.js`：

```js
// EchoRoom 功能开关（构建前配置）
// 修改后需重新打包生效：cargo tauri build（dev 模式刷新即生效）
// 关闭的功能：界面不显示、行为不生效
window.FEATURES = {
  sound: true,      // 进出音效（含设置页"音效"分类）
  theme: true,      // 6 套主题（含外观页主题区）
  background: true, // 自定义背景图（含外观页背景图区）
};
```

`app.js` 顶部 `// ---- 小工具 ----` 段（`const el = (id) => document.getElementById(id);` 行之后）加：

```js
// D：功能开关（features.js 提供；缺文件/缺项按全开）
const FEATURES = Object.assign({ sound: true, theme: true, background: true }, window.FEATURES || {});
```

`ICONS` 对象（`avatar` 行之后）加：

```js
  gear: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
```

`// ---- 事件接线与启动 ----` 段之前插入新段：

```js
// ---- D：设置页（顶栏 ⚙ 覆盖式；分类切换 + 版本号） ----
const settingsPage = el("settings-page");
const paneRefreshers = {}; // cat → 刷新钩子（各页面任务注册：paneRefreshers.device = refreshDevices 等）

function switchCat(cat) {
  for (const btn of settingsPage.querySelectorAll(".settings-cat")) {
    btn.classList.toggle("active", btn.dataset.cat === cat);
  }
  for (const pane of settingsPage.querySelectorAll(".settings-pane")) {
    pane.hidden = pane.dataset.pane !== cat;
  }
  const fn = paneRefreshers[cat];
  if (fn) fn();
}

function openSettings() {
  settingsPage.hidden = false;
  const active = settingsPage.querySelector(".settings-cat.active");
  switchCat(active ? active.dataset.cat : "account");
}

function closeSettings() {
  settingsPage.hidden = true;
}

el("open-settings").innerHTML = ICONS.gear;
el("open-settings").addEventListener("click", openSettings);
el("settings-back").addEventListener("click", closeSettings);
for (const btn of settingsPage.querySelectorAll(".settings-cat")) {
  btn.addEventListener("click", () => switchCat(btn.dataset.cat));
}

// D：功能开关——关闭的分类整个隐藏（features.js；默认全开时无副作用）
if (!FEATURES.sound) settingsPage.querySelector('.settings-cat[data-cat="sound"]').hidden = true;
if (!FEATURES.theme && !FEATURES.background) settingsPage.querySelector('.settings-cat[data-cat="appearance"]').hidden = true;
```

- [ ] **Step 4: app.js——齿轮门控 + 版本号**

`setConn` 函数（`el("chat-send").disabled = !usable;` 之后）加：

```js
  el("open-settings").hidden = !usable;
  if (!usable) closeSettings(); // 断线/被拒：设置页一并收起，交还登录/重连流程
```

`init()` 内 `const cfg = await invoke("get_config");` 块内（`el("setup-server").value = ...` 之前）加：

```js
  // D：设置页版本号（取打包版本；失败保留静态占位）
  window.__TAURI__.app
    .getVersion()
    .then((v) => {
      el("settings-version").textContent = "v" + v;
    })
    .catch(() => {});
```

- [ ] **Step 5: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 6: 提交**

```powershell
git add client/ui/features.js client/ui/index.html client/ui/style.css client/ui/app.js; git commit -m "feat(client): settings page skeleton (topbar, overlay, categories)"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：顶栏加高、Echo 放大、⚙ 出现在"连接中/已连接"右侧；未登录/断线时 ⚙ 隐藏且设置页自动收起；点 ⚙ 覆盖顶栏以下整个窗口，四个分类可切换、`‹ 返回` 关闭；开合期间聊天与卡片状态无损；左列底部显示版本号（本任务暂为 v0.3.0 静态值，构建后为真实版本）；features.js 创建并接入（全开状态下与无开关行为一致；关闭矩阵验证在 Task 11）。
```
### Task 6: 账号页与退出登录

**Files:**
- Modify: `client/src-tauri/src/bridge.rs`（`logout` 命令）
- Modify: `client/src-tauri/src/lib.rs`（注册）
- Modify: `client/ui/index.html`（账号 pane + 资料弹窗标题）
- Modify: `client/ui/style.css`（账号页样式）
- Modify: `client/ui/app.js`（账号页渲染 + 资料弹窗参数化 + 两段式登出）

**Interfaces:**
- Consumes: Task 5 的 `paneRefreshers` / `closeSettings()` / `settingsPage` / `#settings-page` 门控；现有 `el()` / `invoke` / `members` / `myUid` / `avatarCache` / `ICONS` / `setConn` / `showAuthPanel` / `renderMembers` / `renderOnline`；`window.videoCapture.stop(kind)` / `window.videoView.end()`；Rust 侧 `persist()`、`AppState`、`NetCmd::Shutdown`
- Produces（后续任务依赖）：
  - 命令 `logout`
  - `openProfilePop(defaultName: string, fromSettings = false)`（新增第 2 参数；注册首弹仍单参调用）
  - `renderAccountPane()`（`paneRefreshers.account` 注册）、`renderAccountIfActive()`

- [ ] **Step 1: bridge.rs——logout 命令**

`client/src-tauri/src/bridge.rs` 的 `avatar_request` 命令之后加：

```rust
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
```

- [ ] **Step 2: lib.rs——注册命令**

`invoke_handler` 列表中 Task 2 加的最后一行 `bridge::set_sound_pack,` 之后加：

```rust
            bridge::logout,
```

- [ ] **Step 3: 编译验证**

Run: `cargo build`
Expected: workspace 编译通过（`logout` 命令纯后台逻辑，无单测；行为经 Task 10 手工验收）。

- [ ] **Step 4: index.html——账号 pane + 资料弹窗标题**

账号 pane（Task 5 的 `<section class="settings-pane" data-pane="account"></section>`）替换为：

```html
      <section class="settings-pane" data-pane="account">
        <h2 class="settings-title">账号</h2>
        <div class="account-row">
          <div class="account-avatar" id="account-avatar"></div>
          <div class="account-info">
            <div class="account-nick" id="account-nick">—</div>
            <div class="account-name" id="account-name">—</div>
          </div>
        </div>
        <button class="settings-btn" id="account-profile">修改资料</button>
        <div class="settings-divider"></div>
        <button class="settings-btn danger" id="account-logout">退出登录</button>
      </section>
```

资料弹窗标题统一（spec 2.1）：`<h2>完善资料</h2>` 替换为：

```html
      <h2>个人资料</h2>
```

- [ ] **Step 5: style.css——账号页样式**

文件末尾（Task 5 设置页样式之后）追加：

```css
/* ---- D：账号页 ---- */
.account-row { display: flex; align-items: center; gap: 12px; margin-bottom: 14px; }
.account-avatar {
  width: 56px;
  height: 56px;
  border-radius: 50%;
  overflow: hidden;
  background: var(--panel-deep);
  color: #333a49;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.account-avatar svg { width: 30px; height: 30px; }
.account-nick { font-size: 14px; color: var(--text); }
.account-name { font-size: 12px; color: var(--text-dim); margin-top: 2px; }
.settings-btn {
  display: block;
  min-width: 120px;
  padding: 8px 14px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.settings-btn:hover { background: var(--panel-hover); }
.settings-btn.danger { color: var(--err); border-color: var(--err-soft); }
.settings-btn:disabled { opacity: .5; cursor: default; }
.settings-divider { height: 1px; background: var(--border); margin: 14px 0; }
```

- [ ] **Step 6: app.js——openProfilePop 参数化**

`openProfilePop` 函数头三行替换为：

```js
function openProfilePop(defaultName, fromSettings = false) {
  showProfileError("");
  pendingAvatar = null;
  el("profile-nickname").value = defaultName || "";
  el("profile-skip").textContent = fromSettings ? "取消" : "以后再说";
```

（函数其余部分——头像预览/显示 mask/聚焦——保持原样；注册成功的既有单参调用 `openProfilePop(el("setup-account").value.trim())` 不必改，`fromSettings` 默认 false。）

- [ ] **Step 7: app.js——账号页段（渲染 + 接线 + 两段式登出）**

在 Task 5 的设置页段之后、`// ---- 事件接线与启动 ----` 之前追加：

```js
// ---- D：账号页（头像/昵称/账号只读 + 修改资料 + 退出登录） ----
async function renderAccountPane() {
  const cfg = await invoke("get_config").catch(() => null);
  el("account-name").textContent = "账号：" + (cfg && cfg.account ? cfg.account : "—");
  const me = myUid != null ? members.get(myUid) : null;
  el("account-nick").textContent = me ? me.nickname : "—";
  const preview = el("account-avatar");
  const cached = myUid != null ? avatarCache.get(myUid) : null;
  if (cached) {
    const img = document.createElement("img");
    img.className = "member-avatar-img";
    img.src = cached;
    img.draggable = false;
    preview.replaceChildren(img);
  } else {
    preview.innerHTML = ICONS.avatar;
  }
}
paneRefreshers.account = renderAccountPane;

/// 账号 pane 正显示时刷新（profile_changed / avatar_data 事件里调用）
function renderAccountIfActive() {
  const active = settingsPage.querySelector(".settings-cat.active");
  if (active && active.dataset.cat === "account") renderAccountPane();
}

el("account-profile").addEventListener("click", () => {
  const me = myUid != null ? members.get(myUid) : null;
  openProfilePop(me ? me.nickname : "", true); // 设置页入口：skip 显示「取消」
});

// 两段式登出：点一次变「确认退出？」，再点执行；3 秒未动恢复
let logoutArmed = false;
let logoutTimer = 0;

el("account-logout").addEventListener("click", () => {
  const btn = el("account-logout");
  if (!logoutArmed) {
    logoutArmed = true;
    btn.textContent = "确认退出？";
    clearTimeout(logoutTimer);
    logoutTimer = setTimeout(() => {
      logoutArmed = false;
      btn.textContent = "退出登录";
    }, 3000);
    return;
  }
  clearTimeout(logoutTimer);
  doLogout();
});

async function doLogout() {
  const btn = el("account-logout");
  if (btn.disabled) return;
  btn.disabled = true;
  try {
    // 先停两路采集（仍在线：report_stream off / set_share_active false 能送达服务器）
    await Promise.all([
      videoCapture.stop(videoCapture.STREAM_SCREEN),
      videoCapture.stop(videoCapture.STREAM_CAMERA),
    ]);
    window.videoView?.end?.();
    await invoke("logout");
  } catch (e) {
    console.error("退出登录失败:", e);
  }
  // 本地状态清理并回登录页（服务器侧已随断线广播自己离开）
  for (const url of avatarCache.values()) if (url) URL.revokeObjectURL(url);
  avatarCache.clear();
  avatarPending.clear();
  members.clear();
  myUid = null;
  everEntered = false; // 下次登录重新播放入场音效
  renderMembers();
  renderOnline();
  el("chat").replaceChildren(); // 清空公屏
  setConn("未连接"); // 输入禁用 + ⚙ 隐藏（同时收起设置页）
  showAuthPanel("login");
  btn.disabled = false;
  btn.textContent = "退出登录";
  logoutArmed = false;
}
```

- [ ] **Step 8: app.js——事件联动（资料变更时刷新账号 pane）**

`init()` 中 `profile_changed` 监听器末尾（`if (profileSubmitting && ...) closeProfilePop();` 行之后）加：

```js
    // D：设置页账号 pane 正打开 → 同步刷新昵称（头像随后由 avatar_data 刷新）
    if (uid === myUid && !settingsPage.hidden) renderAccountIfActive();
```

`avatar_data` 监听器末尾（`renderMembers();` 行之后）加：

```js
    // D：设置页账号 pane 正打开 → 同步刷新头像（自己换头像后立即可见）
    if (uid === myUid && !settingsPage.hidden) renderAccountIfActive();
```

- [ ] **Step 9: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 10: 提交**

```powershell
git add client/src-tauri/src/bridge.rs client/src-tauri/src/lib.rs client/ui/index.html client/ui/style.css client/ui/app.js; git commit -m "feat(client): account pane and logout flow"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：⚙ → 账号：头像/昵称/账号名（只读）正确显示；[修改资料] 打开弹窗标题「个人资料」、skip 为「取消」，改昵称/头像后账号 pane 立即刷新、双客户端成员卡同步；[退出登录] 一次变「确认退出？」、3 秒不动恢复，二次点击后回登录页（账号预填保留、不自动登录），房间内他人看到自己离开；重新登录一切正常、入场音效重播。
```

---

### Task 7: 设备页（麦克风 / 扬声器 / 摄像头）

**Files:**
- Modify: `client/ui/index.html`（设备 pane）
- Modify: `client/ui/style.css`（设备行样式）
- Modify: `client/ui/app.js`（枚举/下拉/接线/回退事件）
- Modify: `client/ui/video_capture.js`（摄像头设备应用 + 运行中重启 + 失效回退）

**Interfaces:**
- Consumes: Task 2 命令 `list_audio_devices` / `set_input_device` / `set_output_device` / `set_camera_device`；Task 3/4 事件 `audio_device_fallback`（payload `{ direction: "input"|"output", reason }`）；Task 5 的 `paneRefreshers`；现有 `invoke` / `el` / `init()` 的 cfg 块
- Produces（后续任务依赖）：
  - `refreshDevices()`（`paneRefreshers.device` 注册）
  - `showDeviceHint(msg)`（内联提示；6 秒自动隐藏）
  - `window.videoCapture.setCameraDevice(id)` / `restartCamera() -> Promise<boolean>`；`startCamera()` 改为返回 `Promise<boolean>` 且内部含「指定设备打不开 → 清偏好 + 回退默认 + `camera-device-fallback` 事件」

- [ ] **Step 1: index.html——设备 pane**

Task 5 的 `<section class="settings-pane" data-pane="device" hidden></section>` 替换为：

```html
      <section class="settings-pane" data-pane="device" hidden>
        <h2 class="settings-title">设备</h2>
        <div class="settings-row">
          <label class="settings-label" for="dev-input">麦克风</label>
          <select class="settings-select" id="dev-input"></select>
        </div>
        <div class="settings-row">
          <label class="settings-label" for="dev-output">扬声器</label>
          <select class="settings-select" id="dev-output"></select>
        </div>
        <div class="settings-row">
          <label class="settings-label" for="dev-camera">摄像头</label>
          <select class="settings-select" id="dev-camera"></select>
        </div>
        <div class="settings-hint" id="device-hint" hidden></div>
      </section>
```

- [ ] **Step 2: style.css——设备页样式**

文件末尾追加：

```css
/* ---- D：设备页 ---- */
.settings-row { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; max-width: 460px; }
.settings-label { width: 60px; flex-shrink: 0; font-size: 13px; color: var(--text-dim); }
.settings-select {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
}
.settings-select:focus { outline: none; border-color: var(--accent); }
.settings-hint { margin-top: 10px; font-size: 12px; color: var(--err); }
```

- [ ] **Step 3: video_capture.js——摄像头设备应用**

`startCamera` 整函数替换为（新增设备约束 + 失效回退 + 布尔返回）：

```js
  // D：摄像头设备（空 = 系统默认）。deviceId 由设置页写入；不匹配（设备拔出）→ 清偏好回退默认。
  let cameraDeviceId = "";

  function setCameraDevice(id) {
    cameraDeviceId = id || "";
  }

  /// 按 deviceId 打开摄像头流；失败返回 null（不抛）。
  /// exact 约束：选中设备必须命中——命中不了说明已失效，交由 startCamera 回退。
  async function tryGetCamera(deviceId) {
    try {
      return await navigator.mediaDevices.getUserMedia({
        video: {
          width: { ideal: 1280 },
          height: { ideal: 720 },
          frameRate: { ideal: 30 },
          ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
        },
      });
    } catch (e) {
      console.warn("[video] 摄像头开启失败:", e.message);
      return null;
    }
  }

  /// 返回 true = 摄像头流已就绪
  async function startCamera() {
    if (sessions.has(STREAM_CAMERA)) return true;
    let stream = await tryGetCamera(cameraDeviceId);
    if (!stream && cameraDeviceId) {
      // 所选设备打不开（拔出/被占用）：清偏好（持久化）→ 回退系统默认 → 通知 UI
      console.warn("[video] 所选摄像头不可用，回退系统默认");
      cameraDeviceId = "";
      await invoke("set_camera_device", { id: "" }).catch(() => {});
      window.dispatchEvent(new CustomEvent("camera-device-fallback"));
      stream = await tryGetCamera("");
    }
    if (!stream) return false;
    await startPipeline(STREAM_CAMERA, stream);
    await invoke("report_stream", { kind: STREAM_CAMERA, on: true }).catch(() => {});
    return true;
  }

  /// D：运行中切换摄像头——停→开（观众端短暂中断后由新流 IDR 恢复）；返回是否恢复成功
  async function restartCamera() {
    await stop(STREAM_CAMERA);
    return startCamera();
  }
```

模块返回对象（`return { ... }`）加两行：

```js
    setCameraDevice,
    restartCamera,
```

（`startScreen`/`switchScreen`/`stop` 等其余函数不动；卡片的摄像头开关调用点无需改动——`isActive` 判断在新逻辑下依旧成立。）

- [ ] **Step 4: app.js——设备页段（枚举/下拉/接线）**

在 Task 6 的账号页段之后、`// ---- 事件接线与启动 ----` 之前追加：

```js
// ---- D：设备页（麦克风/扬声器/摄像头；选择立即生效 + 失效回退提示） ----
let devicesFreshAt = 0; // 最近一次枚举时间（pointerdown 防抖：3 秒内不重复枚举）
let deviceHintTimer = 0;

function showDeviceHint(msg) {
  const node = el("device-hint");
  node.textContent = msg;
  node.hidden = !msg;
  clearTimeout(deviceHintTimer);
  if (msg) {
    deviceHintTimer = setTimeout(() => {
      node.hidden = true;
    }, 6000);
  }
}

/// 用设备列表重建下拉；current 不在列表中 → 落到「系统默认」显示（回退由事件路径持久化）
function fillDeviceSelect(sel, devices, current, defaultLabel) {
  sel.replaceChildren();
  const def = document.createElement("option");
  def.value = "";
  def.textContent = defaultLabel;
  sel.appendChild(def);
  for (const d of devices) {
    const o = document.createElement("option");
    o.value = d.id;
    o.textContent = d.is_default ? d.name + "（系统默认）" : d.name;
    sel.appendChild(o);
  }
  sel.value = current;
  if (sel.value !== current) sel.value = ""; // 设定值不在列表中：落到系统默认
}

async function refreshDevices() {
  devicesFreshAt = Date.now();
  // 1) 音频设备（wasapi 枚举，Rust 侧；含 is_default 标记）
  let audio = { inputs: [], outputs: [] };
  try {
    audio = await invoke("list_audio_devices");
  } catch (e) {
    console.warn("音频设备枚举失败:", e);
  }
  const cfg = await invoke("get_config").catch(() => null);
  fillDeviceSelect(el("dev-input"), audio.inputs, cfg ? cfg.input_device : "", "系统默认");
  fillDeviceSelect(el("dev-output"), audio.outputs, cfg ? cfg.output_device : "", "系统默认");
  // 2) 摄像头（浏览器 enumerateDevices；未授权过 label 为空 → 显示序号）
  let cams = [];
  try {
    const all = await navigator.mediaDevices.enumerateDevices();
    cams = all
      .filter((d) => d.kind === "videoinput")
      .map((d, i) => ({ id: d.deviceId, name: d.label || "摄像头 " + (i + 1) }));
  } catch (e) {
    console.warn("摄像头枚举失败:", e);
  }
  let camPref = cfg ? cfg.camera_device : "";
  if (camPref && !cams.some((c) => c.id === camPref)) camPref = ""; // 无法匹配：显示系统默认
  fillDeviceSelect(el("dev-camera"), cams, camPref, "系统默认");
}
paneRefreshers.device = refreshDevices;

el("dev-input").addEventListener("change", () => {
  invoke("set_input_device", { id: el("dev-input").value }).catch((e) => console.warn(e));
});
el("dev-output").addEventListener("change", () => {
  invoke("set_output_device", { id: el("dev-output").value }).catch((e) => console.warn(e));
});
el("dev-camera").addEventListener("change", async () => {
  const id = el("dev-camera").value;
  await invoke("set_camera_device", { id }).catch((e) => console.warn(e));
  videoCapture.setCameraDevice(id);
  if (videoCapture.isActive(videoCapture.STREAM_CAMERA)) {
    await videoCapture.restartCamera(); // 运行中：自动重启（失效回退在 startCamera 内部）
    renderMembers(); // 卡片摄像头按钮态跟随
  }
  refreshDevices(); // 回退/切换后同步下拉显示
});

// 展开下拉前刷新（pointerdown 先于原生下拉展开；3 秒防抖避免高频枚举）
for (const id of ["dev-input", "dev-output", "dev-camera"]) {
  el(id).addEventListener("pointerdown", () => {
    if (Date.now() - devicesFreshAt > 3000) refreshDevices();
  });
}

// 摄像头失效回退（video_capture.js 派发）：提示 + 刷新下拉（偏好已由该模块清空持久化）
window.addEventListener("camera-device-fallback", () => {
  showDeviceHint("所选摄像头不可用，已回退系统默认");
  refreshDevices();
});
```

- [ ] **Step 5: app.js——音频回退事件监听 + 启动时应用摄像头偏好**

`init()` 中 `avatar_data` 监听器之后加：

```js
  // D：音频设备回退（采集/播放线程）→ 清持久化偏好 + 刷新下拉 + 内联提示
  await listen("audio_device_fallback", (e) => {
    const { direction, reason } = e.payload;
    invoke(direction === "input" ? "set_input_device" : "set_output_device", { id: "" }).catch(() => {});
    showDeviceHint((direction === "input" ? "麦克风" : "扬声器") + "不可用，已回退系统默认（" + reason + "）");
    refreshDevices();
  });
```

`init()` 的 cfg 块中 `videoCapture.setQuality(shareQuality);` 行之后加：

```js
  videoCapture.setCameraDevice(cfg.camera_device || ""); // D：摄像头设备（空 = 系统默认）
```

- [ ] **Step 6: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

Run: `node --check client/ui/video_capture.js`
Expected: 无输出（语法通过）。

- [ ] **Step 7: 提交**

```powershell
git add client/ui/index.html client/ui/style.css client/ui/app.js client/ui/video_capture.js; git commit -m "feat(client): device settings pane (mic/speaker/camera)"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：设备页三组下拉（首项「系统默认」+ 各设备友好名）；切麦克风对方听声源变化、切扬声器声音换设备输出（均不断线）；拔出所选设备 → 下拉回落「系统默认」+ 内联提示；运行中切摄像头 → 观众端短暂黑屏后恢复；重启客户端后摄像头/音频设备选择保持。
```

---

### Task 8: 音效页（块状方案 + 试听 + 播放改造）

**Files:**
- Move: `client/ui/in.mp3` → `client/ui/sounds/default/in.mp3`；`client/ui/out.mp3` → `client/ui/sounds/default/out.mp3`
- Modify: `client/ui/index.html`（音效 pane 容器）
- Modify: `client/ui/style.css`（音效块样式）
- Modify: `client/ui/app.js`（注册表 + 播放函数改造 + 调用点替换 + 音效页渲染 + init 读方案）

**Interfaces:**
- Consumes: Task 2 命令 `set_sound_pack`；Task 1 的 `Config.sound_pack`；Task 5 的 `paneRefreshers` / `FEATURES`
- Produces（后续任务依赖）：
  - `SOUND_PACKS = [{ id, name }]`（注册表）、全局 `soundPack`（当前方案，`"none"` = 关闭）
  - `playPackSnd(kind: "in" | "out")`（实播；替代原 `playSnd`）
  - `previewSound(id, kind)`（试听，不影响选中）
  - `buildSoundPane()`（`paneRefreshers.sound` 注册）

- [ ] **Step 1: 音效目录迁移（现有文件挪入 sounds/default/）**

```powershell
New-Item -ItemType Directory -Force client\ui\sounds\default; git mv client/ui/in.mp3 client/ui/sounds/default/in.mp3; git mv client/ui/out.mp3 client/ui/sounds/default/out.mp3
```

Expected: `client/ui/sounds/default/` 下出现 in.mp3 / out.mp3；git status 显示 renamed。

- [ ] **Step 2: app.js——音效注册表与播放函数**

`client/ui/app.js` 的整段旧音效代码：

```js
// ---- 音效：进入 / 退出（webview 原生 Audio；重连不重播；自己退出不播） ----
const sndIn = new Audio("in.mp3");
const sndOut = new Audio("out.mp3");
sndIn.volume = 0.5;
sndOut.volume = 0.5;
let everEntered = false; // 本进程内是否已首次进入过房间（重连不重播）

function playSnd(audio) {
  audio.currentTime = 0; // 连点重入时从头播，不叠音
  audio.play().catch(() => {}); // 自动播放被策略阻止时静默忽略
}
```

替换为：

```js
// ---- 音效：方案注册表（进入 / 退出；重连不重播；自己退出不播） ----
// 新增音效 = 放 client/ui/sounds/<id>/in.mp3 + out.mp3，并在 SOUND_PACKS 加一行
const SOUND_PACKS = [{ id: "default", name: "Default" }];
let soundPack = "default"; // 当前方案（init 从 config 读入；"none" = 关闭）
let everEntered = false; // 本进程内是否已首次进入过房间（重连不重播）
const packAudios = new Map(); // `${id}:${kind}` → Audio（懒建；试听与实播共用）

function packAudio(id, kind) {
  const key = id + ":" + kind;
  let a = packAudios.get(key);
  if (!a) {
    a = new Audio("sounds/" + id + "/" + kind + ".mp3");
    a.volume = 0.5;
    packAudios.set(key, a);
  }
  return a;
}

// 实播：member_join / member_leave / 首次进入（"none" 不播；功能开关关闭时不播）
function playPackSnd(kind) {
  if (!FEATURES.sound || soundPack === "none") return;
  const a = packAudio(soundPack, kind);
  a.currentTime = 0; // 连点重入时从头播，不叠音
  a.play().catch(() => {}); // 文件缺失/自动播放被阻止：静默忽略
}

// 试听（不影响选中）
function previewSound(id, kind) {
  const a = packAudio(id, kind);
  a.currentTime = 0;
  a.play().catch(() => {});
}
```

- [ ] **Step 3: app.js——3 处调用点替换**

`members` 监听器内：

```js
    if (!everEntered) {
      everEntered = true;
      playSnd(sndIn); // 自己首次进入；之后的重连同步不播
    }
```

改为：

```js
    if (!everEntered) {
      everEntered = true;
      playPackSnd("in"); // 自己首次进入；之后的重连同步不播
    }
```

`member_join` 监听器内 `playSnd(sndIn); // 别人进入` 改为 `playPackSnd("in"); // 别人进入`。

`member_leave` 监听器内 `playSnd(sndOut); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）` 改为 `playPackSnd("out"); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）`。

- [ ] **Step 4: index.html——音效 pane**

Task 5 的 `<section class="settings-pane" data-pane="sound" hidden></section>` 替换为：

```html
      <section class="settings-pane" data-pane="sound" hidden>
        <h2 class="settings-title">音效</h2>
        <div class="sound-list" id="sound-list"></div>
      </section>
```

- [ ] **Step 5: style.css——音效块样式**

文件末尾追加：

```css
/* ---- D：音效页 ---- */
.sound-list { display: flex; flex-direction: column; gap: 10px; max-width: 420px; }
.sound-block {
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--panel);
  cursor: pointer;
}
.sound-block:hover { background: var(--panel-hover); }
.sound-block.active { border-color: var(--accent); }
.sound-head { display: flex; align-items: center; justify-content: space-between; }
.sound-name { font-size: 13px; color: var(--text); }
.sound-mark { font-size: 12px; color: var(--accent); }
.sound-actions { display: flex; gap: 8px; margin-top: 10px; }
.sound-preview {
  padding: 5px 12px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: none;
  color: var(--text-dim);
  font-size: 12px;
  cursor: pointer;
}
.sound-preview:hover { border-color: var(--accent); color: var(--text); }
```

- [ ] **Step 6: app.js——音效页渲染段 + init 读取方案**

在 Task 7 的设备页段之后、`// ---- 事件接线与启动 ----` 之前追加：

```js
// ---- D：音效页（块状方案列表：点块=使用；两个试听按钮） ----
function soundBlock(id, name, withButtons) {
  const block = document.createElement("div");
  block.className = "sound-block" + (soundPack === id ? " active" : "");
  const head = document.createElement("div");
  head.className = "sound-head";
  const nm = document.createElement("span");
  nm.className = "sound-name";
  nm.textContent = name;
  head.appendChild(nm);
  if (soundPack === id) {
    const mark = document.createElement("span");
    mark.className = "sound-mark";
    mark.textContent = "● 使用中";
    head.appendChild(mark);
  }
  block.appendChild(head);
  if (withButtons) {
    const row = document.createElement("div");
    row.className = "sound-actions";
    for (const [kind, label] of [["in", "▶ 入场"], ["out", "▶ 离场"]]) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "sound-preview";
      b.textContent = label;
      b.addEventListener("click", (e) => {
        e.stopPropagation(); // 试听不影响选中
        previewSound(id, kind);
      });
      row.appendChild(b);
    }
    block.appendChild(row);
  }
  block.addEventListener("click", () => selectSoundPack(id));
  return block;
}

async function selectSoundPack(id) {
  if (soundPack === id) return;
  soundPack = id;
  buildSoundPane(); // 立即反映选中态（持久化失败也保持本次选择）
  await invoke("set_sound_pack", { pack: id }).catch((e) => console.warn(e));
}

function buildSoundPane() {
  const list = el("sound-list");
  list.replaceChildren();
  for (const pack of SOUND_PACKS) list.appendChild(soundBlock(pack.id, pack.name, true));
  list.appendChild(soundBlock("none", "关闭（无提示音）", false)); // 固定末尾：关闭块无试听按钮
}
paneRefreshers.sound = buildSoundPane;
```

`init()` 的 cfg 块中 `videoCapture.setCameraDevice(cfg.camera_device || "");` 行之后加：

```js
  soundPack = cfg.sound_pack || "default"; // D：音效方案（"none" = 关闭）
```

- [ ] **Step 7: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 8: 提交**

```powershell
git add client/ui/sounds client/ui/index.html client/ui/style.css client/ui/app.js; git commit -m "feat(client): sound pack registry, preview pane and playback"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：音效页 Default 块 ▶入场/▶离场可试听（不影响选中）；选中后他人进出/自己首进按方案播放、重连不重播；选「关闭（无提示音）」后无任何提示音；试听按钮不触发选中；重启客户端后方案保持（config.sound_pack）；features.js 关闭 sound 时不播任何音效（含默认方案；Task 11 矩阵）。
```

---

### Task 9: 主题系统（6 套预设 + 硬编码派生色变量化）

**Files:**
- Modify: `client/ui/style.css`（6 组主题变量 + 7 处硬编码替换 + 主题 chip 样式）
- Modify: `client/ui/index.html`（外观 pane·主题部分）
- Modify: `client/ui/app.js`（THEMES / applyTheme / 主题页渲染 / init 尽早应用）

**Interfaces:**
- Consumes: Task 2 命令 `set_theme`；Task 1 的 `Config.theme`；Task 5 已加的 4 个派生变量（`--accent-soft` / `--err-soft` / `--panel-hover` / `--panel-deep`）、`paneRefreshers` 与 `FEATURES`
- Produces（后续任务依赖）：
  - `THEMES = [{ id, name, accent }]`、`applyTheme(id)`、全局 `currentTheme`
  - `buildThemePane()`、`buildAppearancePane()`（`paneRefreshers.appearance` 注册；Task 10 会向其中追加背景图刷新）
  - CSS：`:root[data-theme="<id>"]` 6 组变量块；`document.documentElement.dataset.theme` 为应用入口

- [ ] **Step 1: style.css——6 组主题变量块**

`:root { ... }` 块（`--tile` 行后的 `}`）之后插入：

```css
/* ---- D：主题（底色系 + 强调色成套；--text/--text-dim/--ok/--err 全局不变） ---- */
:root[data-theme="dianlan"] {
  --bg: #12141a;
  --panel: #1d2027;
  --border: #2a2f3a;
  --accent: #3b82f6;
  --accent-soft: rgba(59, 130, 246, .15);
  --panel-hover: #262b36;
  --panel-deep: #15181f;
}
:root[data-theme="anzi"] {
  --bg: #141020;
  --panel: #1f1a2e;
  --border: #2e2740;
  --accent: #a78bfa;
  --accent-soft: rgba(167, 139, 250, .15);
  --panel-hover: #282238;
  --panel-deep: #171226;
}
:root[data-theme="molv"] {
  --bg: #0f1a14;
  --panel: #1a2620;
  --border: #263830;
  --accent: #34d399;
  --accent-soft: rgba(52, 211, 153, .15);
  --panel-hover: #22332a;
  --panel-deep: #121c15;
}
:root[data-theme="yingfen"] {
  --bg: #1a1216;
  --panel: #241a20;
  --border: #362a32;
  --accent: #f472b6;
  --accent-soft: rgba(244, 114, 182, .15);
  --panel-hover: #2f222a;
  --panel-deep: #1c1318;
}
:root[data-theme="hupo"] {
  --bg: #1a1610;
  --panel: #241f16;
  --border: #383020;
  --accent: #fbbf24;
  --accent-soft: rgba(251, 191, 36, .15);
  --panel-hover: #2f281c;
  --panel-deep: #1c1710;
}
:root[data-theme="qinglan"] {
  --bg: #0f171c;
  --panel: #192128;
  --border: #263640;
  --accent: #22d3ee;
  --accent-soft: rgba(34, 211, 238, .15);
  --panel-hover: #202b34;
  --panel-deep: #121a1f;
}
```

（`:root` 基础块本身保留现状值：作为 `data-theme` 未设置时的兜底 = 黛蓝。`--err-soft` 不随主题变，留在基础块。）

- [ ] **Step 2: style.css——硬编码派生色变量化（7 处）**

按原文精确替换：

1. `.member-avatar`（卡片头像区底色）：

```css
  justify-content: center;
  background: #15181f;
  color: #333a49;
  flex-shrink: 0;
}
.member-avatar svg { width: 96px; height: 96px; }
```

改为：

```css
  justify-content: center;
  background: var(--panel-deep);
  color: #333a49;
  flex-shrink: 0;
}
.member-avatar svg { width: 96px; height: 96px; }
```

2. `.setup-panel input`：

```css
  border: 1px solid var(--border);
  background: #15181f;
  color: var(--text);
```

改为：

```css
  border: 1px solid var(--border);
  background: var(--panel-deep);
  color: var(--text);
```

3. `.profile-avatar-preview`：

```css
  border-radius: 50%;
  overflow: hidden;
  background: #15181f;
  color: #333a49;
```

改为：

```css
  border-radius: 50%;
  overflow: hidden;
  background: var(--panel-deep);
  color: #333a49;
```

4. `.mbtn:hover:not(:disabled) { background: #262b36; color: #cbd0dc; }` 改为 `.mbtn:hover:not(:disabled) { background: var(--panel-hover); color: #cbd0dc; }`

5. `.mbtn.active.err { background: rgba(248, 113, 113, .15); color: var(--err); }` 改为 `.mbtn.active.err { background: var(--err-soft); color: var(--err); }`

6. `.mbtn.active.acc { background: rgba(59, 130, 246, .15); color: var(--accent); }` 改为 `.mbtn.active.acc { background: var(--accent-soft); color: var(--accent); }`

7. `.share-pop .sp-stop` 的 `background: rgba(248, 113, 113, .15);` 改为 `background: var(--err-soft);`

（保留不动：`#cbd0dc` 文字色、`#333a49` 剪影色、`.member-badge`/`.sp-stop.start`/`.sp-stop:hover` 的视频遮罩类固定色——不在本次主题范围。）

- [ ] **Step 3: index.html——外观 pane（主题部分）**

Task 5 的 `<section class="settings-pane" data-pane="appearance" hidden></section>` 替换为（主题区包在 `theme-section` 中，供功能开关隐藏）：

```html
      <section class="settings-pane" data-pane="appearance" hidden>
        <h2 class="settings-title">外观</h2>
        <div id="theme-section">
          <div class="appearance-label">主题</div>
          <div class="theme-grid" id="theme-grid"></div>
        </div>
      </section>
```

- [ ] **Step 4: style.css——主题 chip 样式**

文件末尾追加：

```css
/* ---- D：外观页·主题 ---- */
.appearance-label { font-size: 13px; color: var(--text-dim); margin: 4px 0 10px; }
.theme-grid { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 18px; max-width: 460px; }
.theme-chip {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 7px 12px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--panel);
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
}
.theme-chip:hover { background: var(--panel-hover); }
.theme-chip.active { border-color: var(--accent); color: var(--accent); }
.theme-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
```

- [ ] **Step 5: app.js——主题逻辑段 + init 尽早应用**

在 Task 8 的音效页段之后、`// ---- 事件接线与启动 ----` 之前追加：

```js
// ---- D：外观页·主题（6 套预设：底色系 + 强调色成套切换；文字/语义色不变） ----
const THEMES = [
  { id: "dianlan", name: "黛蓝", accent: "#3b82f6" },
  { id: "anzi", name: "暗紫", accent: "#a78bfa" },
  { id: "molv", name: "墨绿", accent: "#34d399" },
  { id: "yingfen", name: "樱粉", accent: "#f472b6" },
  { id: "hupo", name: "琥珀", accent: "#fbbf24" },
  { id: "qinglan", name: "青岚", accent: "#22d3ee" },
];
let currentTheme = "dianlan";

function applyTheme(id) {
  if (!THEMES.some((t) => t.id === id)) id = "dianlan"; // 未知 id（手改配置）：回默认
  currentTheme = id;
  document.documentElement.dataset.theme = id;
}

function buildThemePane() {
  const grid = el("theme-grid");
  grid.replaceChildren();
  for (const t of THEMES) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "theme-chip" + (currentTheme === t.id ? " active" : "");
    b.dataset.themeId = t.id;
    const dot = document.createElement("span");
    dot.className = "theme-dot";
    dot.style.background = t.accent;
    b.appendChild(dot);
    b.appendChild(document.createTextNode(t.name));
    b.addEventListener("click", () => {
      applyTheme(t.id);
      invoke("set_theme", { theme: t.id }).catch((e) => console.warn(e));
      buildThemePane(); // 更新选中态
    });
    grid.appendChild(b);
  }
}

function buildAppearancePane() {
  if (FEATURES.theme) buildThemePane(); // 功能开关：主题关闭则不构建
}
paneRefreshers.appearance = buildAppearancePane;

// D：功能开关——主题关闭则整区隐藏（features.js）
if (!FEATURES.theme) el("theme-section").hidden = true;
```

`init()` 函数头替换（尽早防闪烁）：

```js
async function init() {
  // D：尽早取配置应用主题（防启动闪烁；后续 cfg 块读取其余设置；功能开关关闭时不套用 = 固定黛蓝）
  const bootCfg = await invoke("get_config").catch(() => null);
  if (bootCfg && FEATURES.theme) applyTheme(bootCfg.theme || "dianlan");
  // 先注册监听器，再发起连接：保证事件不因时序竞态丢失
```

（原 `async function init() {` 之后紧跟的 `// 先注册监听器…` 注释行保留。）

- [ ] **Step 6: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 7: 提交**

```powershell
git add client/ui/index.html client/ui/style.css client/ui/app.js; git commit -m "feat(client): six preset themes with CSS variable groups"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：6 套主题逐套点选——窗口底色/面板/边框/强调色（发送按钮、说话框 border、聚焦边、链接、激活态）成套变化，文字与红绿语义色不变；切换立即生效、开合设置页保持；重启客户端后主题保持；手改 config 为非法 id → 回默认黛蓝不崩；features.js 关闭 theme 时外观页无主题区、启动固定黛蓝（不读 config.theme；Task 11 矩阵）。
```

---

### Task 10: 背景图（文件驱动：选择 / 清除 / 铺层）

**Files:**
- Modify: `client/src-tauri/src/bridge.rs`（`sniff_mime` + `describe` 三个命令 + 测试）
- Modify: `client/src-tauri/src/lib.rs`（注册 3 命令）
- Modify: `client/ui/index.html`（外观 pane·背景图区）
- Modify: `client/ui/style.css`（背景行/缩略图 + `.app-body` 双伪元素铺层）
- Modify: `client/ui/app.js`（背景逻辑段 + 外观刷新钩子 + init 启动加载）

**Interfaces:**
- Consumes: 既有 `default_config_path()`（`%APPDATA%\com.echoroom.dev\`）；Task 5 的 `paneRefreshers` / `FEATURES`；Task 9 的 `buildAppearancePane`；`send_video_frame` 已验证的原始字节通道（JS `u8.buffer` → Rust `InvokeBody::Raw`）
- Produces（后续任务依赖）：
  - 命令 `set_background(request)`（raw body ≤10MB + 文件头白名单）、`clear_background()`、`get_background() -> tauri::ipc::Response`（前端收到 **ArrayBuffer**；空 = 无图。tauri 2.11.5 源码已核实：`InvokeResponseBody::Raw` → 小载荷走 `new Uint8Array([...]).buffer`、大载荷走 fetch `arrayBuffer()`）
  - `background_path()`：单文件 `background.img`（无扩展名，MIME 由文件头嗅探）；**无 config 字段**（文件存在即生效）
  - JS：`refreshBackground()` / `applyBackgroundBytes(u8, mime)` / `clearBackgroundUi()`；CSS 变量 `--bg-image`

- [ ] **Step 1: bridge.rs——嗅探测试先行**

`bridge.rs` 的 `#[cfg(test)] mod tests` 中：`use super::is_web_url;` 改为 `use super::{is_web_url, sniff_mime};`，并在 `is_web_url_rejects_other_schemes_and_plain_text` 之后追加：

```rust
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
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-client sniff_mime`
Expected: 编译错误（`sniff_mime` 未定义）——预期失败。

- [ ] **Step 3: bridge.rs——背景图命令实现**

`bridge.rs` 的 `logout` 命令之后新增段：

```rust
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
```

- [ ] **Step 4: lib.rs——注册命令**

`invoke_handler` 列表中 `bridge::logout,` 之后加：

```rust
            bridge::set_background,
            bridge::clear_background,
            bridge::get_background,
```

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p echoroom-client`
Expected: 全绿（含两个 `sniff_mime_*` 测试）。

Run: `cargo build`
Expected: workspace 编译通过。

- [ ] **Step 6: index.html——外观 pane·背景图区**

Task 9 的外观 pane 中 `theme-section` 的 `</div>` 之后、`</section>` 之前插入（整段包在 `bg-section` 中，供功能开关隐藏）：

```html
        <div id="bg-section">
          <div class="appearance-label">背景图</div>
          <div class="bg-row">
            <button class="settings-btn" id="bg-pick">选择图片</button>
            <button class="settings-btn" id="bg-clear">清除</button>
            <input id="bg-file" type="file" accept="image/*" hidden />
          </div>
          <div class="bg-preview" id="bg-preview" hidden></div>
          <div class="settings-hint" id="bg-hint" hidden></div>
        </div>
```

- [ ] **Step 7: style.css——背景样式与铺层**

文件末尾追加：

```css
/* ---- D：外观页·背景图 ---- */
.bg-row { display: flex; gap: 8px; margin-bottom: 10px; }
.bg-preview {
  width: 260px;
  height: 146px;
  border: 1px solid var(--border);
  border-radius: 10px;
  overflow: hidden;
}
.bg-preview img { width: 100%; height: 100%; object-fit: cover; display: block; }

/* 铺层：.app-body 双伪元素（图片 + 主题底色 72% 罩），子项抬升到罩上方；
   无图时罩色 = 背景色，视觉零差异；设置页/弹层为 fixed 高层，天然不铺图 */
.app-body { position: relative; }
.app-body::before,
.app-body::after {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
}
.app-body::before {
  background-image: var(--bg-image, none);
  background-size: cover;
  background-position: center;
}
.app-body::after {
  background: var(--bg);
  opacity: .72;
}
.app-body > * { position: relative; z-index: 1; }
```

- [ ] **Step 8: app.js——背景逻辑段 + 外观刷新 + 启动加载**

在 Task 9 的主题段之后、`// ---- 事件接线与启动 ----` 之前追加：

```js
// ---- D：外观页·背景图（Rust 单文件 background.img；文件存在即生效） ----
let bgUrl = null; // 当前背景图 Blob URL（换图/清除时 revoke）

function sniffImageMime(u8) {
  if (u8.length >= 8 && u8[0] === 0x89 && u8[1] === 0x50 && u8[2] === 0x4e && u8[3] === 0x47) {
    return "image/png";
  }
  if (u8.length >= 3 && u8[0] === 0xff && u8[1] === 0xd8 && u8[2] === 0xff) {
    return "image/jpeg";
  }
  if (u8.length >= 12 && u8[8] === 0x57 && u8[9] === 0x45 && u8[10] === 0x42 && u8[11] === 0x50) {
    return "image/webp";
  }
  return "";
}

function applyBackgroundBytes(u8, mime) {
  if (bgUrl) URL.revokeObjectURL(bgUrl);
  bgUrl = URL.createObjectURL(new Blob([u8], { type: mime }));
  document.body.style.setProperty("--bg-image", 'url("' + bgUrl + '")');
  const preview = el("bg-preview");
  const img = document.createElement("img");
  img.src = bgUrl;
  img.draggable = false;
  preview.replaceChildren(img);
  preview.hidden = false;
}

function clearBackgroundUi() {
  if (bgUrl) URL.revokeObjectURL(bgUrl);
  bgUrl = null;
  document.body.style.removeProperty("--bg-image");
  el("bg-preview").replaceChildren();
  el("bg-preview").hidden = true;
}

function showBgHint(msg) {
  const node = el("bg-hint");
  node.textContent = msg;
  node.hidden = !msg;
}

async function refreshBackground() {
  const buf = await invoke("get_background").catch(() => null);
  const u8 = buf && buf.byteLength ? new Uint8Array(buf) : new Uint8Array(0);
  const mime = u8.length ? sniffImageMime(u8) : "";
  if (!mime) {
    clearBackgroundUi(); // 无图/非法残留：纯色兜底
    return;
  }
  applyBackgroundBytes(u8, mime);
}

el("bg-pick").addEventListener("click", () => el("bg-file").click());
el("bg-file").addEventListener("change", async (e) => {
  const file = e.target.files[0];
  e.target.value = ""; // 允许重复选择同一文件
  if (!file) return;
  if (file.size > 10 * 1024 * 1024) {
    showBgHint("图片过大（上限 10MB）");
    return;
  }
  try {
    const u8 = new Uint8Array(await file.arrayBuffer());
    await invoke("set_background", u8.buffer); // 原始字节体（与 send_video_frame 同通道）
    showBgHint("");
  } catch (err) {
    showBgHint(String(err && err.message ? err.message : err)); // 非白名单格式等
    return;
  }
  refreshBackground(); // 读回校验后的真实存储 → 铺层 + 缩略图（覆盖=旧图已删）
});

el("bg-clear").addEventListener("click", async () => {
  await invoke("clear_background").catch(() => {});
  clearBackgroundUi();
  showBgHint("");
});

// D：功能开关——背景图关闭则整区隐藏（features.js）
if (!FEATURES.background) el("bg-section").hidden = true;
```

Task 9 的 `buildAppearancePane` 改为：

```js
function buildAppearancePane() {
  if (FEATURES.theme) buildThemePane();
  if (FEATURES.background) refreshBackground();
}
```

`init()` 的 cfg 块中 `soundPack = cfg.sound_pack || "default";` 行之后加：

```js
  if (FEATURES.background) refreshBackground(); // D：背景图（文件存在即铺层；无图静默；功能开关关闭时不读不铺）
```

- [ ] **Step 9: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 10: 提交**

```powershell
git add client/src-tauri/src/bridge.rs client/src-tauri/src/lib.rs client/ui/index.html client/ui/style.css client/ui/app.js; git commit -m "feat(client): custom background image (file-driven, mime sniffing)"
```

```markdown
验收要点（手工，统一并入 Task 11 验收）：选择图片 → 立即铺满侧栏+卡片区+聊天+输入栏（卡片/文字仍可读）；缩略图显示；重启客户端保留；换图后旧文件被覆盖（`%APPDATA%\com.echoroom.dev\background.img` 只有一个）；清除 → 恢复纯色 + 文件消失；>10MB 被拒（提示）；伪造扩展名的非图片文件被拒（白名单嗅探）；设置页/登录页不铺背景；features.js 关闭 background 时外观页无背景图区且不铺层（Task 11 矩阵）。
```

---

### Task 11: 全量验收与 0.3.0 构建交付

**Files:**
- Modify: `client/src-tauri/tauri.conf.json`（版本号）
- 产物：`Echo.exe` + `Echo_0.3.0_x64-setup.exe`

**Interfaces:**
- Consumes: Task 1–10 的全部功能；验收依赖服务器 + 两台客户端（或一台客户端 + sim_clients）
- Produces: 0.3.0 安装包与免安装 exe（交付物）

- [ ] **Step 1: 版本号升级**

`client/src-tauri/tauri.conf.json`：

```json
// 原文
  "version": "0.2.0",
// 替换为
  "version": "0.3.0",
```

- [ ] **Step 2: 全量测试与构建**

Run: `cargo test`
Expected: 全绿（protocol + server + client 全部测例；server/protocol 零改动）。

Run: `cargo build`
Expected: workspace 编译通过。

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

Run: `node --check client/ui/features.js`
Expected: 无输出（语法通过）。

Run: `node --check client/ui/video_capture.js`
Expected: 无输出（语法通过）。

- [ ] **Step 3: 服务器 + 双客户端手工验收（11 条）**

先启动服务器（终端 A；零协议改动，部署无需更新）：

```powershell
cd E:\pro\EchoRoom; cargo run -p echoroom-server -- 9000 --invite echo-2026
```

Expected: `Echo server listening on 0.0.0.0:9000 (tcp+udp)`。

| # | 场景 | 操作 | 期望 |
|---|------|------|------|
| 1 | 设置页开合 | ⚙ → 四个分类切换 → `‹ 返回` | 覆盖顶栏以下全窗口；开合后聊天/卡片/语音状态无损；未登录/断线时 ⚙ 隐藏 |
| 2 | 主题 | 逐套点选 6 套主题 | 底色系+强调色成套变化（发送按钮/说话框/聚焦边/链接/激活态）；重启客户端保持 |
| 3 | 背景图 | 选择（含 >10MB 拒绝）→ 换图 → 清除 | 铺满主工作区可读；重启保留；换图旧文件被删（数据目录仅一个 background.img）；清除恢复纯色+文件被删 |
| 4 | 麦克风热切换 | 双端语音中切麦克风；再拔出所选设备说话 | 对方听到声源变化、语音不断；拔出后回退系统默认+设置页内联提示+下拉回落 |
| 5 | 扬声器热切换 | 语音中切扬声器 | 声音从新设备输出、仅一瞬静默、无爆音；拔掉所选设备 → 回退+提示 |
| 6 | 摄像头 | 切摄像头后开启；运行中再切 | 新流用新设备；运行中切换对方短暂黑屏后恢复；重启后选择保持（deviceId 不稳定时回落系统默认属预期） |
| 7 | 音效 | 试听两按钮；选中 Default；选「关闭」；进出房 | 试听不影响选中；选中后进出有音效、自己首进有音效、重连不重播；关闭后无音效 |
| 8 | 退出登录 | 两段式确认退出 → 重启客户端 → 重新登录 | 回登录页（账号预填、不自动登录）；房间内他人看到离开；重登一切正常 |
| 9 | 修改资料 | 设置页 [修改资料] → 改昵称/头像 | 弹窗标题「个人资料」、skip 为「取消」；保存后自己卡片与对方界面同步刷新 |
| 10 | 版本号 | 设置页左列底部 | 显示 `v0.3.0`；顶栏 Echo 放大、⚙ 在连接状态右侧 |
| 11 | 功能开关 | 改 `client/ui/features.js` 逐项 `false` → `cargo tauri dev` 重启（或直接重打包） | 音效关：设置页无「音效」分类 + 进出房无声（默认音效也不播）；主题关：外观页无主题区 + 皮肤固定黛蓝（不读 config.theme）；背景图关：外观页无背景图区 + 不铺层；主题+背景图同关：无「外观」分类；全关：只剩账号/设备；三项恢复 `true` 后一切如常 |

- [ ] **Step 4: 桌面构建与产物拷贝**

```powershell
cd E:\pro\EchoRoom\client\src-tauri; cargo tauri build
```

Expected: 构建成功，产物（cargo 使用 workspace 共享 target 目录，在仓库根）：

- `E:\pro\EchoRoom\target\release\Echo.exe`
- `E:\pro\EchoRoom\target\release\bundle\nsis\Echo_0.3.0_x64-setup.exe`

```powershell
Copy-Item 'E:\pro\EchoRoom\target\release\Echo.exe' "$env:USERPROFILE\Desktop" -Force; Copy-Item 'E:\pro\EchoRoom\target\release\bundle\nsis\Echo_0.3.0_x64-setup.exe' "$env:USERPROFILE\Desktop" -Force
```

Expected: 桌面出现 `Echo.exe` 与 `Echo_0.3.0_x64-setup.exe`。

```markdown
交付说明：0.3.0 为纯客户端功能更新（协议零改动）——服务器无需重部署，旧版客户端可继续使用；所有设置（主题/设备/音效/背景图）存于客户端本地（`%APPDATA%\com.echoroom.dev\`），不随账号同步；功能开关在 `client/ui/features.js`（构建前配置，改后重新打包生效）。
```

---

## 计划完成后的执行提示

- 执行顺序严格按 Task 1 → 11；每个任务结束运行其验证命令（`cargo test -p echoroom-client` + 相关 `node --check`），全绿后才进入下一任务
- Task 3/4 涉及音频线程真实设备操作：真机验证（耳机+扬声器、双麦克风）放在 Task 11 手工验收第 4/5 条统一做；Rust 侧仅保证编译与纯函数单测
- 手工验收（Task 11 Step 3）需要服务器 + 两台客户端（可用 `sim_clients` 替代其一）；背景图/主题/音效为本地设置，重启验证用同一客户端即可
- 功能开关（第 11 条）在 `client/ui/features.js`：验证矩阵用 `cargo tauri dev` 改动重启即可（快）；**最终交付前将三个开关恢复为 `true` 再打包**（除非用户明确要求关闭某项）
- 全程零协议/服务器改动：`protocol` 与 `server` 不应出现任何 diff；若出现，说明任务越界，需回退
- 仓库在 `dev` 分支；提交信息按各任务 Step 末尾给出；推送按用户确认后执行（与既往流程一致）
