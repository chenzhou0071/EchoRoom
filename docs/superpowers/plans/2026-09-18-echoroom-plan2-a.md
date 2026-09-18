# EchoRoom 计划2 · A 子项目（UI 卡片改版 + 个体音量 + 静音广播 + 进出音效）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 EchoRoom 成员区从"纯文字方块"升级为 200×230 头像卡片（按钮排 / 个体音量 / 手动静音广播 / 进出音效）。

**Architecture:** 协议层新增 `Mute`/`Muted` 消息、`LoginOk.members` 携带 muted；服务器 room 存 muted 并广播；客户端 Rust 侧为音量/静音唯一真值（`SharedAudio`：原子增益 + 按昵称 gain 表 + uid→昵称映射），音频链三处落点（采集增益、静音门控、混音增益）；前端纯渲染 + invoke。

**Tech Stack:** Rust（workspace：echoroom-protocol / echoroom-server / echoroom-client）+ Tauri v2 + 原生 HTML/CSS/JS（无构建工具）。

## Global Constraints

- 卡片尺寸 200×230 = 头像区 200×200 + 名字条 30px；成员区高度 250px（12 + 230 + 8）
- 增益范围一律 0.0–2.0（clamp），默认 1.0；滑块显示为百分比 0–200%
- 静音 = 采集继续（VAD 照跑）、不发包、不上报 speaking；解除后补报
- peer_gains 按**昵称**存（uid 每次连接都变）
- 消息 type：`Mute`=8（C→S，uid 填 0）、`Muted`=9（S→C 广播，含发送者本人）
- 激活态色：静音 `rgba(248,113,113,.15)` 底 + `#f87171` 图标；投屏/摄像头 `rgba(59,130,246,.15)` 底 + `#3b82f6` 图标
- 音效播放音量统一 0.5；自己退出不播；重连不重播
- 最小窗口 768×560 不变；列数 auto-fill 200px 不变
- 环境：Windows PowerShell（用 `;` 分隔命令）；测试命令为 `cargo test -p <包名>`
- 每个任务结束时全 workspace 必须编译通过、已有测试全绿

## File Structure（全景）

**protocol**（协议）
- `protocol/src/messages.rs`：`TcpMessage` 加 `Mute`/`Muted`；`LoginOk.members` 变 `Vec<(u16, String, bool)>`
- `protocol/src/tcp.rs`：编解码 + 往返测试

**server**（服务器）
- `server/src/room.rs`：`Member.muted` 字段、`set_muted()`、join 返回带 muted
- `server/src/tcp.rs`：`Mute` 转发处理（改状态 + 广播 `Muted`）

**client / src-tauri**（客户端 Rust）
- `src/config.rs`：`self_gain` / `muted` / `peer_gains` 三字段（serde default 兼容旧文件）
- `src/audio/session.rs`：`SharedAudio` 定义；采集线程接 self_gain/muted；播放线程按 uid→昵称查 gain；`speaking_report()` 纯函数
- `src/audio/mixer.rs`：`MixAccumulator::add_scaled()`
- `src/net/tcp.rs`：`NetCmd::SetMuted`、`Muted` 解析→事件、uid_names 维护、重连补报静音
- `src/bridge.rs`：`AppState.shared`、`emit_muted`/`emit_self_uid`、3 个 invoke 命令
- `src/lib.rs`：AppState 初始化 + invoke 注册
- `tauri.conf.json`：`additionalBrowserArgs` 放开 WebView2 自动播放（进出音效）

**client / ui**（前端）
- `ui/style.css`：卡片新结构 + 按钮三态 + popover + 静音图标
- `ui/app.js`：卡片渲染、popover、invoke/listen 接线、音效
- `ui/in.mp3`、`ui/out.mp3`：从仓库根目录 `in.mp3`/`out.mp3` 复制

---

### Task 1: 协议扩展——Mute/Muted 消息 + Members 携带 muted

**Files:**
- Modify: `protocol/src/messages.rs`
- Modify: `protocol/src/tcp.rs`
- Modify: `server/src/room.rs`（仅类型适配：JoinOk.members 三元组）

**Interfaces:**
- Consumes: 现有 `TcpMessage`、`tcp::encode/try_decode`（`[len u32][type u8][payload]`，u16/u32 大端、String=[len u16][utf8]、bool=u8）
- Produces（后续任务依赖）：
  - `TcpMessage::Mute { uid: u16, on: bool }`（type 8）
  - `TcpMessage::Muted { uid: u16, on: bool }`（type 9）
  - `TcpMessage::LoginOk { uid, token, members: Vec<(u16, String, bool)> }`（第三位 = muted）
  - `server::room::JoinOk.members: Vec<(u16, String, bool)>`

- [ ] **Step 1: 写失败测试（protocol roundtrip 扩展）**

修改 `protocol/src/tcp.rs` 的 `roundtrip_all_variants` 测试，把 `LoginOk` 样本改为三元组并加入两条新消息：

```rust
    #[test]
    fn roundtrip_all_variants() {
        let samples = vec![
            TcpMessage::Login { nickname: "阿信".into() },
            TcpMessage::LoginOk { uid: 3, token: 0xDEAD_BEEF, members: vec![(1, "小K".into(), false), (2, "你".into(), true)] },
            TcpMessage::MemberJoin { uid: 5, nickname: "新来的".into() },
            TcpMessage::MemberLeave { uid: 2 },
            TcpMessage::Chat { uid: 1, text: "晚上开黑吗".into() },
            TcpMessage::Speaking { uid: 4, on: true },
            TcpMessage::Mute { uid: 0, on: true },
            TcpMessage::Muted { uid: 4, on: true },
            TcpMessage::LoginReject { reason: "房间已满（6人）".into() },
        ];
        for msg in samples {
            let bytes = encode(&msg);
            let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
            assert_eq!(decoded, msg);
            assert_eq!(n, bytes.len());
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-protocol`
Expected: 编译失败（`LoginOk` 字段类型不匹配、`Mute`/`Muted` 变体不存在）

- [ ] **Step 3: 实现消息定义**

`protocol/src/messages.rs`——`TcpMessage` 中 `Speaking` 之后加两个变体、`LoginOk` 改类型、`type_id` 加两条：

```rust
pub enum TcpMessage {
    /// C→S：进入房间
    Login { nickname: String },
    /// S→C：登录成功（uid、token、已有成员；成员 = (uid, 昵称, 是否静音)）
    LoginOk { uid: u16, token: u32, members: Vec<(u16, String, bool)> },
    /// S→C：有成员加入
    MemberJoin { uid: u16, nickname: String },
    /// S→C：有成员离开
    MemberLeave { uid: u16 },
    /// C→S（uid 填 0）公屏消息；S→C（uid 为发送者）广播
    Chat { uid: u16, text: String },
    /// C→S（uid 填 0）说话状态；S→C 广播
    Speaking { uid: u16, on: bool },
    /// C→S（uid 填 0）静音状态；S→C 广播（Muted）
    Mute { uid: u16, on: bool },
    /// S→C 广播静音状态（含发送者本人）
    Muted { uid: u16, on: bool },
    /// S→C：房间满等拒绝原因
    LoginReject { reason: String },
}

impl TcpMessage {
    pub fn type_id(&self) -> u8 {
        match self {
            TcpMessage::Login { .. } => 1,
            TcpMessage::LoginOk { .. } => 2,
            TcpMessage::MemberJoin { .. } => 3,
            TcpMessage::MemberLeave { .. } => 4,
            TcpMessage::Chat { .. } => 5,
            TcpMessage::Speaking { .. } => 6,
            TcpMessage::LoginReject { .. } => 7,
            TcpMessage::Mute { .. } => 8,
            TcpMessage::Muted { .. } => 9,
        }
    }
}
```

- [ ] **Step 4: 实现编解码**

`protocol/src/tcp.rs`——`encode` 中 `LoginOk` 成员序列化加 muted、新增两个分支：

```rust
        TcpMessage::LoginOk { uid, token, members } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&token.to_be_bytes());
            payload.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for (uid, name, muted) in members {
                payload.extend_from_slice(&uid.to_be_bytes());
                put_str(&mut payload, name);
                payload.push(if *muted { 1 } else { 0 });
            }
        }
```

```rust
        TcpMessage::Mute { uid, on } | TcpMessage::Muted { uid, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(if *on { 1 } else { 0 });
        }
```

`try_decode` 中 `LoginOk` 解码改三元组、新增 8/9 分支：

```rust
        2 => {
            let uid = field!(r.u16());
            let token = field!(r.u32());
            let count = field!(r.u16()) as usize;
            let mut members = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let m_uid = field!(r.u16());
                let name = field!(r.string());
                let muted = field!(r.u8()) != 0;
                members.push((m_uid, name, muted));
            }
            TcpMessage::LoginOk { uid, token, members }
        }
```

```rust
        8 => {
            let uid = field!(r.u16());
            TcpMessage::Mute { uid, on: field!(r.u8()) != 0 }
        }
        9 => {
            let uid = field!(r.u16());
            TcpMessage::Muted { uid, on: field!(r.u8()) != 0 }
        }
```

- [ ] **Step 5: 服务器 room.rs 类型适配（编译修复）**

`server/src/room.rs`：`JoinOk.members` 改类型；`join()` 中收集处补第三位 `false`（Task 2 会换成真实 muted）：

```rust
pub struct JoinOk {
    pub uid: u16,
    pub token: u32,
    /// 加入前已在房间的成员：(uid, 昵称, 是否静音)
    pub members: Vec<(u16, String, bool)>,
}
```

```rust
        let members: Vec<(u16, String, bool)> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), false))
            .collect();
```

同时更新 room.rs 测试 `join_returns_existing_members_and_leaves_removes` 的断言：

```rust
        assert_eq!(ok2.members, vec![(ok1.uid, "A".to_string(), false)]);
```

- [ ] **Step 6: 全 workspace 编译 + 测试全绿**

Run: `cargo test`
Expected: 全部 `test result: ok`（protocol roundtrip 通过、room 测试通过；如 `server/src/tcp.rs` 有类型错误则为遗漏——它用 `ok.members.clone()` 自动适配，不应报错）

- [ ] **Step 7: Commit**

```bash
git add protocol/src/messages.rs protocol/src/tcp.rs server/src/room.rs
git commit -m "feat: protocol mute messages and muted flag in member list"
```

---

### Task 2: 服务器——静音状态存储与广播

**Files:**
- Modify: `server/src/room.rs`
- Modify: `server/src/tcp.rs`

**Interfaces:**
- Consumes: Task 1 的 `TcpMessage::Mute/Muted`、`JoinOk.members` 三元组
- Produces:
  - `Room::set_muted(&mut self, uid: u16, on: bool) -> bool`（成员存在返回 true）
  - `Member.muted: bool`（join 默认 false）
  - 服务器收到 `Mute{on}` 后广播 `Muted{uid, on}` 给全房间（含发送者）

- [ ] **Step 1: 写失败测试（room）**

在 `server/src/room.rs` 测试模块追加：

```rust
    #[test]
    fn set_muted_updates_member_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), tx1).unwrap();
        assert!(room.set_muted(a.uid, true));
        assert!(!room.set_muted(999, true)); // 不存在的成员
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), tx2).unwrap();
        // 后加入者应看到 A 处于静音
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), true)]);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-server`
Expected: 编译失败（`set_muted` 不存在）

- [ ] **Step 3: 实现 room 改动**

`server/src/room.rs`：`Member` 加字段、`join` 初始化、join 收集真实 muted、新增 `set_muted`：

```rust
pub struct Member {
    pub uid: u16,
    pub nickname: String,
    pub token: u32,
    pub muted: bool,
    pub tx: Sender<Vec<u8>>, // TCP 发送队列（预编码字节）
    pub udp_addr: Option<SocketAddr>,
    pub last_seen: Instant,
}
```

```rust
            Member {
                uid,
                nickname,
                token,
                muted: false,
                tx,
                udp_addr: None,
                last_seen: Instant::now(),
            },
```

```rust
        let members: Vec<(u16, String, bool)> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), m.muted))
            .collect();
```

```rust
    /// 更新成员静音状态；成员不存在返回 false
    pub fn set_muted(&mut self, uid: u16, on: bool) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.muted = on;
            true
        } else {
            false
        }
    }
```

- [ ] **Step 4: 运行 room 测试**

Run: `cargo test -p echoroom-server`
Expected: 全部通过（含新测试）

- [ ] **Step 5: 服务器转发处理**

`server/src/tcp.rs` 读循环 `match msg` 中，`Speaking` 分支后追加：

```rust
                        TcpMessage::Mute { on, .. } => {
                            let room = room.lock().unwrap();
                            room.set_muted(ok.uid, on);
                            // 广播回所有人（含自己）：与 Chat/Speaking 同模式，卡片图标统一由广播驱动
                            room.broadcast(None, &TcpMessage::Muted { uid: ok.uid, on });
                        }
```

- [ ] **Step 6: 编译 + 全量测试**

Run: `cargo test`
Expected: 全部 `test result: ok`

- [ ] **Step 7: Commit**

```bash
git add server/src/room.rs server/src/tcp.rs
git commit -m "feat: server stores and broadcasts member mute state"
```

---

### Task 3: 客户端配置——音量三字段持久化

**Files:**
- Modify: `client/src-tauri/src/config.rs`

**Interfaces:**
- Consumes: 现有 `Config::load/save`（serde_json，`%APPDATA%\com.echoroom.dev\config.json`）
- Produces:
  - `Config.self_gain: f32`（默认 1.0）、`Config.muted: bool`（默认 false）、`Config.peer_gains: std::collections::HashMap<String, f32>`（默认空）
  - 旧配置文件（无新字段）加载时自动补默认值（serde `default`）

- [ ] **Step 1: 写失败测试**

`client/src-tauri/src/config.rs` 文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_volume_fields() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_roundtrip.json");
        let mut cfg = Config::default();
        cfg.self_gain = 1.5;
        cfg.muted = true;
        cfg.peer_gains.insert("小林".into(), 0.5);
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_config_without_volume_fields_loads_defaults() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_old.json");
        std::fs::write(&path, r#"{"nickname":"老用户","server_addr":"127.0.0.1:9000"}"#).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.nickname, "老用户");
        assert_eq!(loaded.self_gain, 1.0);
        assert!(!loaded.muted);
        assert!(loaded.peer_gains.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-client`
Expected: 编译失败（`self_gain` 等字段不存在）

- [ ] **Step 3: 实现字段**

`client/src-tauri/src/config.rs`：

```rust
fn default_gain() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub nickname: String,
    pub server_addr: String,
    /// 自己的采集增益（0.0–2.0）
    #[serde(default = "default_gain")]
    pub self_gain: f32,
    /// 自己的静音状态
    #[serde(default)]
    pub muted: bool,
    /// 对他人的播放增益（按昵称）
    #[serde(default)]
    pub peer_gains: std::collections::HashMap<String, f32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            nickname: String::new(),
            server_addr: "127.0.0.1:9000".into(),
            self_gain: 1.0,
            muted: false,
            peer_gains: std::collections::HashMap::new(),
        }
    }
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p echoroom-client`
Expected: 新测试通过（其余测试不受影响）

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/config.rs
git commit -m "feat: persist self gain, mute state and per-nickname gains in config"
```

---
### Task 4: 音频侧——SharedAudio + 混音增益 + 采集/播放接线

**Files:**
- Modify: `client/src-tauri/src/audio/mixer.rs`
- Modify: `client/src-tauri/src/audio/session.rs`
- Modify: `client/src-tauri/src/bridge.rs`（仅 `AppState.shared` 字段 + `start_audio` 传参，编译闭环所需）
- Modify: `client/src-tauri/src/lib.rs`（仅初始化 `SharedAudio` 并放入 AppState）

**Interfaces:**
- Consumes: Task 3 的 `Config` 字段（构造 SharedAudio 用）；现有 `MixAccumulator`、`spawn_audio_pipeline`、`SpeakingDetector`
- Produces（Task 5 依赖）：
  - `audio::session::SharedAudio { self_gain: Arc<AtomicU32>, self_muted: Arc<AtomicBool>, peer_gains: Arc<Mutex<HashMap<String, f32>>>, uid_names: Arc<Mutex<HashMap<u16, String>>> }` + `SharedAudio::new(self_gain: f32, muted: bool, peer_gains: HashMap<String, f32>)`
  - `MixAccumulator::add_scaled(&mut self, pcm: &[i16], gain: f32)`
  - `spawn_audio_pipeline(server_addr: String, uid: u16, token: u32, tcp_tx: Sender<NetCmd>, shared: SharedAudio)`（**新增第 5 参数**）
  - `bridge::AppState.shared: SharedAudio`（Task 5 的 invoke 命令依赖）
  - 内部纯函数 `speaking_report(flip: Option<bool>, muted_now: bool, was_muted: bool, speaking_now: bool) -> Option<bool>`

- [ ] **Step 1: 写混音增益失败测试**

`client/src-tauri/src/audio/mixer.rs` 测试模块追加：

```rust
    #[test]
    fn add_scaled_applies_gain() {
        let sig = ramp(10000);
        let mut acc = MixAccumulator::new(N);
        acc.add_scaled(&sig, 0.5);
        let mut out = vec![0i16; N];
        acc.finalize(&mut out);
        // 半幅信号在近似线性区：约为输入一半（小信号误差 < 100）
        for (o, s) in out.iter().zip(sig.iter()) {
            let expect = *s as f32 * 0.5;
            assert!((*o as f32 - expect).abs() < 100.0, "o={o} s={s}");
        }
    }

    #[test]
    fn add_scaled_zero_is_silence() {
        let sig = ramp(20000);
        let mut acc = MixAccumulator::new(N);
        acc.add_scaled(&sig, 0.0);
        let mut out = vec![1i16; N];
        acc.finalize(&mut out);
        assert!(out.iter().all(|&v| v == 0));
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p echoroom-client`
Expected: 编译失败（`add_scaled` 不存在）

- [ ] **Step 3: 实现 add_scaled**

`client/src-tauri/src/audio/mixer.rs`——`MixAccumulator` impl 中 `add` 之后插入：

```rust
    /// 增益后累加（gain 通常 0.0–2.0；越界样本 clamp 到 i16 域后再进 i32 累加）。
    /// gain ≈ 1.0 走原路径，避免无意义的浮点乘。
    pub fn add_scaled(&mut self, pcm: &[i16], gain: f32) {
        if (gain - 1.0).abs() < 1e-6 {
            self.add(pcm);
            return;
        }
        for (a, &s) in self.acc.iter_mut().zip(pcm.iter()) {
            let v = (s as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i32;
            *a += v;
        }
    }
```

- [ ] **Step 4: 运行 mixer 测试**

Run: `cargo test -p echoroom-client mixer`
Expected: PASS（含新测试）

- [ ] **Step 5: session.rs 加 SharedAudio 与 speaking_report（含测试）**

`client/src-tauri/src/audio/session.rs`：
1. imports 行 `use std::sync::atomic::{AtomicBool, Ordering};` 改为 `use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};`
2. 文件顶部 `AudioHandle` 定义之前插入：

```rust
/// 音量/静音共享态：bridge（写）+ 网络线程（写 uid_names）+ 音频线程（读）三方共享。
#[derive(Clone)]
pub struct SharedAudio {
    /// 自己的采集增益（f32 bits 存于 AtomicU32）
    pub self_gain: Arc<AtomicU32>,
    /// 自己的静音状态
    pub self_muted: Arc<AtomicBool>,
    /// 对他人的播放增益（按昵称）
    pub peer_gains: Arc<std::sync::Mutex<HashMap<String, f32>>>,
    /// uid → 昵称（网络线程维护；播放端按 uid 查增益）
    pub uid_names: Arc<std::sync::Mutex<HashMap<u16, String>>>,
}

impl SharedAudio {
    pub fn new(self_gain: f32, muted: bool, peer_gains: HashMap<String, f32>) -> Self {
        SharedAudio {
            self_gain: Arc::new(AtomicU32::new(self_gain.to_bits())),
            self_muted: Arc::new(AtomicBool::new(muted)),
            peer_gains: Arc::new(std::sync::Mutex::new(peer_gains)),
            uid_names: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

/// 本块要对外发送的 speaking 上报决策（None = 不发送）。
/// 语义：进入静音瞬间强制上报"停止说话"；静音期间抑制"开始说话"；
/// 解除静音瞬间补报 VAD 当前真实状态（静音期间 VAD 照跑，状态可能已翻转）。
fn speaking_report(
    flip: Option<bool>,
    muted_now: bool,
    was_muted: bool,
    speaking_now: bool,
) -> Option<bool> {
    if !was_muted && muted_now {
        return Some(false);
    }
    if was_muted && !muted_now {
        return Some(speaking_now);
    }
    match flip {
        Some(on) if !muted_now || !on => Some(on),
        _ => None,
    }
}
```

3. 文件末尾追加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaking_report_mutes_and_realigns() {
        // 正常翻转透传
        assert_eq!(speaking_report(Some(true), false, false, true), Some(true));
        assert_eq!(speaking_report(Some(false), false, false, false), Some(false));
        // 进入静音瞬间：强制上报停止说话（清别人卡片蓝框）
        assert_eq!(speaking_report(None, true, false, true), Some(false));
        // 静音期间：翻入说话被抑制；翻出静音可上报（无害）
        assert_eq!(speaking_report(Some(true), true, true, true), None);
        assert_eq!(speaking_report(Some(false), true, true, false), Some(false));
        // 解除静音：补报 VAD 当前状态（可能在静音期间已翻转为说话）
        assert_eq!(speaking_report(None, false, true, true), Some(true));
        assert_eq!(speaking_report(None, false, true, false), Some(false));
    }
}
```

- [ ] **Step 6: 运行确认 speaking_report 测试绿**

Run: `cargo test -p echoroom-client speaking_report`
Expected: PASS

- [ ] **Step 7: 改造 spawn_audio_pipeline 签名与采集线程**

`spawn_audio_pipeline` 签名加第 5 参数：

```rust
pub fn spawn_audio_pipeline(
    server_addr: String,
    uid: u16,
    token: u32,
    tcp_tx: std::sync::mpsc::Sender<NetCmd>,
    shared: SharedAudio,
) -> anyhow::Result<AudioHandle> {
```

采集线程块改为（克隆 Arc 进线程；增益在 denoise 后、VAD 前；静音门控用 speaking_report）：

```rust
    // 采集线程：MicCapture → 累积 960 → 降噪 →（增益）→ VAD → tx_pcm
    {
        let stop = stop.clone();
        let self_gain = shared.self_gain.clone();
        let self_muted = shared.self_muted.clone();
        std::thread::spawn(move || {
            let mic = match MicCapture::open() {
                Ok(m) => m,
                Err(e) => {
                    // 不发 conn 事件：TCP 连接实际正常，此处仅音频不可用
                    eprintln!("[audio] 麦克风不可用: {e:#}");
                    return;
                }
            };
            let mut denoiser = Denoiser::new();
            // 阈值实测自降噪后信号（底噪残留 ≈ 17、语音 ≥ 200）：进入 50 / 退出 25
            let mut detector = SpeakingDetector::new(50.0, 25.0, Duration::from_millis(400));
            let mut pending: Vec<i16> = Vec::with_capacity(1920);
            let mut was_muted = false;
            while !stop.load(Ordering::Relaxed) {
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

- [ ] **Step 8: 改造播放线程（peer gain 查表）**

`spawn_audio_pipeline` 中播放线程部分——闭包前克隆 Arc、循环改 `iter_mut()` + `add_scaled`：

```rust
    // 播放线程：drain 语音 → jitter → decode → mix → 声卡
    let mut jbs: HashMap<u16, Sender> = HashMap::new();
    let mut seen: HashSet<u16> = HashSet::new();
    let mut acc = MixAccumulator::new(FRAME_SAMPLES);
    let mut scratch = vec![0i16; FRAME_SAMPLES];
    let peer_gains = shared.peer_gains.clone();
    let uid_names = shared.uid_names.clone();
    let player = crate::audio::playback::spawn_player(move |out| {
        // 收流：每发送者独立抖动缓冲
        while let Ok((uid, seq, opus)) = rx_voice.try_recv() {
            if seen.insert(uid) {
                println!("[audio] 首次收到 uid={uid} 的语音");
            }
            jbs.entry(uid).or_insert_with(Sender::new).jb.insert(seq, opus);
        }
        // 出流：输出驱动拉取——每路按需取样本直到填满声卡周期。
        // 帧余量跨周期保留，消费速率严格等于输出速率（避免每段各取一帧导致帧超量消耗）。
        let mut filled = 0usize;
        while filled < out.len() {
            let want = (out.len() - filled).min(FRAME_SAMPLES);
            // 每段快照一次 uid → gain（避免逐路重复查昵称）；无自定义增益时为空表走默认 1.0
            let gain_snapshot: HashMap<u16, f32> = {
                let names = uid_names.lock().unwrap();
                let gains = peer_gains.lock().unwrap();
                if gains.is_empty() {
                    HashMap::new()
                } else {
                    names
                        .iter()
                        .map(|(u, n)| (*u, gains.get(n).copied().unwrap_or(1.0)))
                        .collect()
                }
            };
            acc.clear();
            for (uid, s) in jbs.iter_mut() {
                let Some(dec) = s.dec.as_mut() else { continue };
                let mut got = 0usize;
                while got < want {
                    if s.pos >= s.pcm.len() {
                        // 余量耗尽：从抖动缓冲取下一帧
                        match s.jb.pop() {
                            PopResult::Ready(Some(f)) => {
                                s.pcm.resize(FRAME_SAMPLES, 0);
                                if dec.decode(Some(&f), &mut s.pcm).is_ok() {
                                    s.pos = 0;
                                    s.plc_streak = 0;
                                } else {
                                    // 解码失败：本帧按静音（继续取后续帧）
                                    s.pcm.clear();
                                    s.pos = 0;
                                    break;
                                }
                            }
                            PopResult::Ready(None) => {
                                if s.plc_streak >= MAX_PLC_STREAK {
                                    break; // 连续缺帧达上限：对方断流，静音等待新帧
                                }
                                s.pcm.resize(FRAME_SAMPLES, 0);
                                if dec.decode(None, &mut s.pcm).is_ok() {
                                    s.pos = 0;
                                    s.plc_streak += 1;
                                } else {
                                    s.pcm.clear();
                                    s.pos = 0;
                                    break;
                                }
                            }
                            PopResult::NotYet => break,
                        }
                    }
                    let n = (want - got).min(s.pcm.len() - s.pos);
                    let g = gain_snapshot.get(uid).copied().unwrap_or(1.0);
                    acc.add_scaled(&s.pcm[s.pos..s.pos + n], g);
                    s.pos += n;
                    got += n;
                }
            }
            acc.finalize(&mut scratch[..want]);
            out[filled..filled + want].copy_from_slice(&scratch[..want]);
            filled += want;
        }
    })?;
```

- [ ] **Step 9: bridge.rs / lib.rs 最小接线（编译闭环）**

`spawn_audio_pipeline` 新增第 5 参后，client crate 需同步两处（仅此两处，其余桥接改动在 Task 5）：

`client/src-tauri/src/bridge.rs`——`AppState` 加字段（置于 `audio` 字段之后）：

```rust
pub struct AppState {
    pub config: Mutex<Config>,
    pub net: Mutex<Option<NetHandle>>,
    pub audio: Mutex<Option<crate::audio::session::AudioHandle>>,
    /// 音量/静音共享态（音频线程、网络线程共享读写）
    pub shared: crate::audio::session::SharedAudio,
}
```

`bridge.rs` 的 `start_audio` 内 spawn 调用补第 5 参：

```rust
    match crate::audio::session::spawn_audio_pipeline(
        server_addr,
        uid,
        token,
        tcp_tx,
        state.shared.clone(),
    ) {
```

`client/src-tauri/src/lib.rs`——`run()` 初始化共享态（invoke 注册不变，Task 5 追加三个新命令）：

```rust
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
            bridge::send_chat
        ])
        .run(tauri::generate_context!())
        .expect("error while running Echo");
}
```

- [ ] **Step 10: 全量测试 + 编译**

Run: `cargo test`
Expected: 全部 `test result: ok`（protocol / server / client 全绿）

- [ ] **Step 11: Commit**

```bash
git add client/src-tauri/src/audio/mixer.rs client/src-tauri/src/audio/session.rs client/src-tauri/src/bridge.rs client/src-tauri/src/lib.rs
git commit -m "feat: audio pipeline honors self gain, mute gating and per-peer gains"
```

---

### Task 5: 客户端网络与桥接——Muted 事件、SetMuted 命令、uid 映射、音量 invoke

**Files:**
- Modify: `client/src-tauri/src/net/tcp.rs`
- Modify: `client/src-tauri/src/bridge.rs`
- Modify: `client/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的协议消息、Task 3 的 Config 字段、Task 4 的 `SharedAudio`
- Produces（前端依赖）：
  - invoke 命令：`set_self_gain(gain: f32)` / `set_muted(on: bool)` / `set_peer_gain(nickname: String, gain: f32)`（均 clamp 到 0–2 并保存）
  - 事件 `volume`：`{ self_gain: f32, muted: bool, peer_gains: {昵称: f32} }`
  - 事件 `muted`：`{ uid: u16, on: bool }`（服务器广播转发）
  - 事件 `self_uid`：`u16`（自己在房间中的 uid，登录后先于 `members` 发出）
  - `members` 事件 payload 改为 `[[uid, 昵称, muted], ...]`

- [ ] **Step 1: net/tcp.rs——命令与事件**

1. `NetCmd` 枚举加一个变体：

```rust
pub enum NetCmd {
    SendChat(String),
    SetSpeaking(bool),
    SetMuted(bool),
    Shutdown,
}
```

2. `spawn` 签名加共享参数（供 uid_names 维护与重连补报静音）：

```rust
pub fn spawn(
    addr: String,
    nickname: String,
    bridge: Bridge,
    shared: crate::audio::session::SharedAudio,
) -> NetHandle {
    let (tx, rx) = std::sync::mpsc::channel::<NetCmd>();
    let my_uid = Arc::new(AtomicU16::new(0));
    let my_token = Arc::new(AtomicU32::new(0));
    let (uid_c, tok_c) = (my_uid.clone(), my_token.clone());
    let tx_for_loop = tx.clone(); // 采集线程 VAD 的 SetSpeaking 命令经会话线程写 TCP
    std::thread::spawn(move || {
        run_loop(addr, nickname, bridge, rx, uid_c, tok_c, tx_for_loop, shared)
    });
    NetHandle { tx, my_uid, my_token }
}
```

3. `run_loop` 与 `run_session` 签名末尾加 `shared: crate::audio::session::SharedAudio` 参数并透传。

4. `run_session` 内，发送命令循环加分支：

```rust
                NetCmd::SetMuted(on) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on }))?;
                }
```

5. `run_session` 的 `LoginOk` 分支改为：

```rust
                        TcpMessage::LoginOk { uid, token, members } => {
                            my_uid.store(uid, Ordering::Relaxed);
                            my_token.store(token, Ordering::Relaxed);
                            bridge.emit_conn(ConnState::Connected);
                            // 重建 uid → 昵称映射（含自己；重连场景先清空）
                            {
                                let mut names = shared.uid_names.lock().unwrap();
                                names.clear();
                                for (u, n, _) in &members {
                                    names.insert(*u, n.clone());
                                }
                                names.insert(uid, nickname.to_string());
                            }
                            bridge.emit_self_uid(uid);
                            // 服务器返回的列表不含自己：补上后整表发给 UI；
                            // 自己的 muted 取本地当前值（重连后保持界面与实际一致）
                            let my_muted = shared.self_muted.load(Ordering::Relaxed);
                            let mut all = members;
                            all.push((uid, nickname.to_string(), my_muted));
                            bridge.emit_member_list(all);
                            // 重连后若本地处于静音，向新会话重新声明（否则服务器端 muted=false，别人看不到）
                            if my_muted {
                                stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on: true }))?;
                            }
                            // 启动音频链路（麦克风/编码/播放/VAD 上报；失败不影响文字聊天）
                            crate::bridge::start_audio(&bridge.app, uid, token, addr.to_string(), tx.clone());
                        }
```

6. 成员与静音消息分支改为：

```rust
                        TcpMessage::MemberJoin { uid, nickname } => {
                            shared.uid_names.lock().unwrap().insert(uid, nickname.clone());
                            bridge.emit_member_join(uid, nickname)
                        }
                        TcpMessage::MemberLeave { uid } => {
                            shared.uid_names.lock().unwrap().remove(&uid);
                            bridge.emit_member_leave(uid)
                        }
                        TcpMessage::Chat { uid, text } => bridge.emit_chat(uid, text),
                        TcpMessage::Speaking { uid, on } => bridge.emit_speaking(uid, on),
                        TcpMessage::Muted { uid, on } => bridge.emit_muted(uid, on),
```

- [ ] **Step 2: bridge.rs——AppState/事件/命令**

1. `Bridge` impl 加两个 emit：

```rust
    pub fn emit_muted(&self, uid: u16, on: bool) {
        let _ = self.app.emit("muted", serde_json::json!({ "uid": uid, "on": on }));
    }
    pub fn emit_self_uid(&self, uid: u16) {
        let _ = self.app.emit("self_uid", uid);
    }
```

2. `emit_member_list` 签名改为三元组：

```rust
    pub fn emit_member_list(&self, members: Vec<(u16, String, bool)>) {
        let _ = self.app.emit("members", members);
    }
```

3. `AppState.shared` 字段（Task 4 Step 9 已加，核对一致）：

```rust
pub struct AppState {
    pub config: Mutex<Config>,
    pub net: Mutex<Option<NetHandle>>,
    pub audio: Mutex<Option<crate::audio::session::AudioHandle>>,
    /// 音量/静音共享态（音频线程、网络线程共享读写）
    pub shared: crate::audio::session::SharedAudio,
}
```

4. 文件末尾（`start_audio` 之前）加辅助与命令：

```rust
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
```

5. `connect_with_app` 改为（先取 state，再 spawn，传共享态）：

```rust
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
```

6. 核对 `start_audio` 的 spawn 调用已传 `state.shared.clone()`（Task 4 Step 9 已改）。

- [ ] **Step 3: lib.rs——注册三个新命令（在 Task 4 已初始化的 `run()` 基础上，最终形态）**

```rust
pub fn run() {
    let cfg = config::Config::load(&config::default_config_path());
    let shared = audio::session::SharedAudio::new(cfg.self_gain, cfg.muted, cfg.peer_gains.clone());
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
```

- [ ] **Step 4: 编译 + 全量测试 + 冒烟**

Run: `cargo test; cargo build -p echoroom-client`
Expected: 测试全绿、构建通过。冒烟（可选）：终端跑 server + `cargo tauri dev`，确认能连接、能发言（老功能不回归）。

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/net/tcp.rs client/src-tauri/src/bridge.rs client/src-tauri/src/lib.rs
git commit -m "feat: client mute/muted plumbing and volume commands with shared state"
```

---
### Task 6: 前端卡片重构——200×230 结构、按钮排、音量面板骨架

**Files:**
- Modify: `client/ui/style.css`
- Modify: `client/ui/app.js`

**Interfaces:**
- Consumes: Task 5 的事件契约（`self_uid` / `members` 三元组 / `muted`）；现有 `renderOnline`、聊天、设置面板逻辑
- Produces（Task 7/8 依赖）：
  - DOM 结构：`.member > .member-avatar + .member-bar(> .member-mute-ico + .member-name + .member-btns > .mbtn*)`
  - `buildCard(uid, m)` / `renderMembers()` / `openVolPop(anchor, uid, nickname)` / `closeVolPop()`
  - 全局 `volState`（音量真值本地副本）与 `volTarget`（面板当前目标）
  - 音量滑块输入事件中的 `// T7` 标记处为 invoke 接线点

- [ ] **Step 1: 替换 style.css 成员区样式段**

将 `client/ui/style.css` 中 `/* ---- 成员网格：固定尺寸正方形卡片，一行 3~6 个随窗口宽度，超出滚动 ---- */` 起、到 `.member.speaking { border-color: var(--accent); }` 止的整段（约 L99–L127），替换为（宽沿用既有变量 `--tile`）；另外把 `:root` 中 `--tile` 行注释同步改为 `/* 成员卡片宽度（定死）：最小窗口 768 每行 3 个，拉宽后 4/5/6 个 */`：

```css
/* ---- 成员网格：200×230 头像卡片，一行 3~6 个随窗口宽度，超出滚动 ---- */
.members {
  display: grid;
  grid-template-columns: repeat(auto-fill, var(--tile));
  justify-content: center;
  gap: 8px;
  grid-auto-rows: 230px; /* 显式行高：保证第二行位置稳定 */
  padding: 12px 14px 8px; /* 底部 padding = 行 gap：第二行恰好被截在可见边界外 */
  overflow-y: auto;
  height: 250px; /* 恰一行：12 + 卡片 230 + 8 */
  flex-shrink: 0; /* 成员行固定高度，不被压缩 */
}
.member {
  width: var(--tile);
  height: 230px;
  display: flex;
  flex-direction: column;
  border-radius: 10px;
  background: var(--panel);
  border: 2px solid transparent; /* 预留蓝框位，避免 speaking 切换时布局跳动 */
  overflow: hidden;
  transition: border-color 120ms ease;
}
.member.speaking { border-color: var(--accent); }

/* 头像区 200×200（A 阶段占位剪影；C 阶段接真头像） */
.member-avatar {
  height: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #15181f;
  color: #333a49;
  flex-shrink: 0;
}
.member-avatar svg { width: 96px; height: 96px; }

/* 名字条 30px：左名（他人静音时前置小图标）+ 右按钮排 */
.member-bar {
  height: 30px;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 0 5px;
  flex-shrink: 0;
}
.member-mute-ico {
  display: none;
  width: 14px;
  height: 14px;
  color: var(--err);
  flex-shrink: 0;
}
.member-mute-ico svg { width: 14px; height: 14px; }
.member.muted .member-mute-ico { display: flex; }
.member-name {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  color: #cbd0dc;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.member-btns {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.mbtn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
}
.mbtn svg { width: 15px; height: 15px; }
.mbtn:hover:not(:disabled) { background: #262b36; color: #cbd0dc; }
.mbtn:disabled { opacity: .35; cursor: not-allowed; }
.mbtn.active.err { background: rgba(248, 113, 113, .15); color: var(--err); }
.mbtn.active.acc { background: rgba(59, 130, 246, .15); color: var(--accent); }

/* ---- 音量弹出面板（单例，定位到按钮旁） ---- */
.vol-pop {
  position: fixed;
  z-index: 30;
  width: 148px;
  padding: 10px 12px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, .45);
  display: none;
  flex-direction: column;
  gap: 6px;
}
.vol-pop.show { display: flex; }
.vol-pop .val { font-size: 12px; color: var(--text-dim); text-align: center; }
.vol-pop input[type="range"] { width: 100%; }
```

- [ ] **Step 2: 重写 app.js（完整文件）**

整体替换 `client/ui/app.js` 为以下内容（保留原有聊天/状态/设置面板逻辑；新增卡片渲染与音量面板骨架）：

```js
// EchoRoom 前端：tauri 事件 → DOM 渲染；操作经 invoke 发给 Rust。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---- 状态 ----
const members = new Map(); // uid → { nickname, speaking, muted }
let myUid = null; // 自己的 uid（self_uid 事件，先于 members 到达）
let volState = { self_gain: 1.0, muted: false, peer_gains: {} }; // Rust 侧音量真值的本地副本

// ---- 图标（feather 风格内联 SVG，currentColor 随按钮状态变色）----
const ICONS = {
  mic: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  micOff: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="1" y1="1" x2="23" y2="23"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6"/><path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  volume: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>',
  screen: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
  cam: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="23 7 16 12 23 17 23 7"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/></svg>',
  avatar: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-5.33 0-9 2.67-9 6v2h18v-2c0-3.33-3.67-6-9-6z"/></svg>',
};

// ---- 小工具 ----
const el = (id) => document.getElementById(id);

function nowHm() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function nicknameOf(uid) {
  const m = members.get(uid);
  return m ? m.nickname : `uid=${uid}`;
}

// ---- 渲染：成员（左栏在线名单共用同一数据） ----
function sortedMembers() {
  return [...members.entries()].sort((a, b) => a[0] - b[0]); // 按 uid 升序 = 进入顺序
}

function buildCard(uid, m) {
  const isSelf = uid === myUid;
  const div = document.createElement("div");
  div.className = "member" + (m.speaking ? " speaking" : "") + (!isSelf && m.muted ? " muted" : "");
  div.dataset.uid = uid;

  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  avatar.innerHTML = ICONS.avatar;
  div.appendChild(avatar);

  const bar = document.createElement("div");
  bar.className = "member-bar";
  const muteIco = document.createElement("span");
  muteIco.className = "member-mute-ico";
  muteIco.innerHTML = ICONS.micOff;
  bar.appendChild(muteIco);
  const name = document.createElement("span");
  name.className = "member-name";
  name.textContent = m.nickname;
  bar.appendChild(name);

  const btns = document.createElement("div");
  btns.className = "member-btns";
  if (isSelf) {
    const muteBtn = document.createElement("button");
    muteBtn.className = "mbtn mbtn-mute" + (volState.muted ? " active err" : "");
    muteBtn.title = volState.muted ? "解除静音" : "静音";
    muteBtn.innerHTML = volState.muted ? ICONS.micOff : ICONS.mic;
    // T7 在此挂 click：invoke set_muted
    btns.appendChild(muteBtn);
  }
  const volBtn = document.createElement("button");
  volBtn.className = "mbtn mbtn-vol";
  volBtn.title = "音量";
  volBtn.innerHTML = ICONS.volume;
  volBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (volTarget && volTarget.uid === uid) closeVolPop();
    else openVolPop(volBtn, uid, m.nickname);
  });
  btns.appendChild(volBtn);
  if (isSelf) {
    const screenBtn = document.createElement("button");
    screenBtn.className = "mbtn";
    screenBtn.disabled = true;
    screenBtn.title = "即将推出";
    screenBtn.innerHTML = ICONS.screen;
    btns.appendChild(screenBtn);
    const camBtn = document.createElement("button");
    camBtn.className = "mbtn";
    camBtn.disabled = true;
    camBtn.title = "即将推出";
    camBtn.innerHTML = ICONS.cam;
    btns.appendChild(camBtn);
  }
  bar.appendChild(btns);
  div.appendChild(bar);
  return div;
}

function renderMembers() {
  const box = el("members");
  box.innerHTML = "";
  for (const [uid, m] of sortedMembers()) box.appendChild(buildCard(uid, m));
}

function renderOnline() {
  const list = el("online-list");
  list.innerHTML = "";
  for (const [, m] of sortedMembers()) {
    const li = document.createElement("li");
    li.className = "online-item";
    li.textContent = m.nickname;
    list.appendChild(li);
  }
}

// ---- 音量弹出面板（单例） ----
const volPop = document.createElement("div");
volPop.className = "vol-pop";
volPop.innerHTML = '<div class="val"></div><input type="range" min="0" max="200" step="1" />';
document.body.appendChild(volPop);
const volVal = volPop.querySelector(".val");
const volSlider = volPop.querySelector('input[type="range"]');
let volTarget = null; // { uid, nickname } 当前面板目标

function openVolPop(anchor, uid, nickname) {
  volTarget = { uid, nickname };
  const cur = uid === myUid ? volState.self_gain : (volState.peer_gains[nickname] ?? 1.0);
  volSlider.value = String(Math.round(cur * 100));
  volVal.textContent = volSlider.value + "%";
  const r = anchor.getBoundingClientRect();
  volPop.classList.add("show");
  const w = volPop.offsetWidth;
  const h = volPop.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - w - 8);
  let top = r.bottom + 6;
  if (top + h > window.innerHeight - 8) top = r.top - h - 6;
  volPop.style.left = left + "px";
  volPop.style.top = top + "px";
}

function closeVolPop() {
  volPop.classList.remove("show");
  volTarget = null;
}

volSlider.addEventListener("input", () => {
  volVal.textContent = volSlider.value + "%";
  // T7 在此 invoke 实时生效（set_self_gain / set_peer_gain）
});

document.addEventListener("click", (e) => {
  if (!e.target.closest(".vol-pop") && !e.target.closest(".mbtn-vol")) closeVolPop();
});

// ---- 渲染：公屏（两行：昵称：HH:MM / 正文；时间取到达时刻的本地时间） ----
function appendMessage(uid, text) {
  const box = el("chat");
  const nearBottom = box.scrollTop + box.clientHeight >= box.scrollHeight - 30;
  const div = document.createElement("div");
  div.className = "msg";
  const meta = document.createElement("div");
  meta.className = "msg-meta";
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = nicknameOf(uid) + "：";
  meta.appendChild(name);
  const time = document.createElement("span");
  time.className = "time";
  time.textContent = nowHm();
  meta.appendChild(time);
  div.appendChild(meta);
  const body = document.createElement("div");
  body.className = "msg-text";
  body.textContent = text;
  div.appendChild(body);
  box.appendChild(div);
  if (nearBottom) box.scrollTop = box.scrollHeight; // 仅当贴底时自动跟随，不打断回看
}

// ---- 渲染：连接状态（顶栏 + 输入可用性） ----
const CONN_TEXT = { connecting: "连接中…", connected: "已连接", reconnecting: "重连中…" };

function setConn(status) {
  const node = el("conn-status");
  if (status.startsWith("rejected:")) {
    node.textContent = "被拒绝：" + status.slice("rejected:".length);
  } else {
    node.textContent = CONN_TEXT[status] || status;
  }
  node.classList.toggle("online", status === "connected");
  node.classList.toggle("offline", status.startsWith("rejected:"));
  const usable = status === "connected";
  el("chat-input").disabled = !usable;
  el("chat-send").disabled = !usable;
  if (usable) el("chat-input").focus();
}

// ---- 发送 ----
function sendCurrent() {
  const input = el("chat-input");
  const text = input.value.trim();
  if (!text || input.disabled) return;
  // 不做本地回显：服务器会把消息广播回给所有人（包括自己）
  invoke("send_chat", { text }).catch((e) => console.error("发送失败:", e));
  input.value = "";
}

el("chat-send").addEventListener("click", sendCurrent);
el("chat-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") sendCurrent();
});

// ---- 首次启动：昵称 / 服务器设置面板 ----
async function submitSetup() {
  const nickname = el("setup-nickname").value.trim();
  const serverAddr = el("setup-server").value.trim();
  if (!nickname || !serverAddr) return;
  try {
    await invoke("set_config", { nickname, serverAddr }); // 保存后 Rust 侧自动发起连接
    el("setup-mask").classList.add("hidden");
  } catch (e) {
    console.error("保存配置失败:", e);
  }
}

el("setup-connect").addEventListener("click", submitSetup);
for (const id of ["setup-nickname", "setup-server"]) {
  el(id).addEventListener("keydown", (e) => {
    if (e.key === "Enter") submitSetup();
  });
}

// ---- 事件接线与启动 ----
async function init() {
  // 先注册监听器，再发起连接：保证事件不因时序竞态丢失
  await listen("self_uid", (e) => {
    myUid = e.payload;
    renderMembers(); // 幂等：myUid 变化后重渲染（区分自己/他人卡片）
  });
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname, muted] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted });
    }
    renderMembers();
    renderOnline();
  });
  await listen("member_join", (e) => {
    const [uid, nickname] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false });
    renderMembers();
    renderOnline();
  });
  await listen("member_leave", (e) => {
    members.delete(e.payload);
    renderMembers();
    renderOnline();
  });
  await listen("chat", (e) => appendMessage(e.payload.uid, e.payload.text));
  await listen("speaking", (e) => {
    const { uid, on } = e.payload;
    const m = members.get(uid);
    if (m) m.speaking = on;
    const card = document.querySelector(`.member[data-uid="${uid}"]`);
    if (card) card.classList.toggle("speaking", on);
  });
  await listen("muted", (e) => {
    const { uid, on } = e.payload;
    const m = members.get(uid);
    if (m) {
      m.muted = on;
      renderMembers();
    }
  });
  await listen("volume", (e) => {
    volState = e.payload;
    renderMembers(); // 自己卡静音按钮态随 muted 更新
    // 面板开着：同步滑块显示（仅值不同才写，避免与拖动打架）
    if (volTarget) {
      const cur =
        volTarget.uid === myUid ? volState.self_gain : (volState.peer_gains[volTarget.nickname] ?? 1.0);
      const pct = String(Math.round(cur * 100));
      if (volSlider.value !== pct) {
        volSlider.value = pct;
        volVal.textContent = pct + "%";
      }
    }
  });
  await listen("conn", (e) => setConn(e.payload));

  const cfg = await invoke("get_config");
  // 音量真值来自 Rust（config 持久化）：初始化本地副本
  volState = { self_gain: cfg.self_gain, muted: cfg.muted, peer_gains: cfg.peer_gains };
  if (!cfg.nickname) {
    el("setup-mask").classList.remove("hidden");
    el("setup-nickname").focus();
  } else {
    await invoke("connect"); // 打开即自动连接
  }
}

init().catch((e) => console.error("初始化失败:", e));
```

- [ ] **Step 3: 启动验证（视觉）**

Run（两个终端）:
1. `cargo run -p echoroom-server --bin echoroom-server`
2. `cargo tauri dev`

Expected: 卡片显示 200×230（上大片头像剪影、下名字条带按钮）；自己卡 4 个按钮、他人卡 1 个；点音量按钮弹出面板（拖动滑块数字变化，但**尚无实际效果**——T7 接线）；投屏/摄像头按钮灰置且有"即将推出"提示；说话蓝框正常。

- [ ] **Step 4: Commit**

```bash
git add client/ui/style.css client/ui/app.js
git commit -m "feat: member cards revamp with avatar, button rows and volume panel shell"
```

---

### Task 7: 前端交互接线——静音按钮与音量滑块 invoke

**Files:**
- Modify: `client/ui/app.js`

**Interfaces:**
- Consumes: Task 6 的 `volState` / `volTarget` / `openVolPop` / `closeVolPop` 与两处 `// T7` 标记；Task 5 的命令契约 `set_self_gain(gain: f32)` / `set_muted(on: bool)` / `set_peer_gain(nickname: String, gain: f32)`，事件 `volume` / `muted`
- Produces: 可用的完整音量/静音交互（全部状态回流走 Rust → `volume`/`muted` 事件 → 重渲染，无本地乐观更新）

说明：Task 6 已完成事件监听闭环（`volume` 事件同步 `volState` 与滑块回显、`muted` 事件触发重渲染、`init()` 从 `get_config` 初始化 `volState`）；本任务只补两处 invoke 挂点（T6 代码中已用 `// T7` 注释标记）。

- [ ] **Step 1: 挂静音按钮 click**

`client/ui/app.js` 定位（`buildCard` 内，自己卡片静音按钮，Task 6 写入的标记行）：

```js
    muteBtn.innerHTML = volState.muted ? ICONS.micOff : ICONS.mic;
    // T7 在此挂 click：invoke set_muted
    btns.appendChild(muteBtn);
```

替换为：

```js
    muteBtn.innerHTML = volState.muted ? ICONS.micOff : ICONS.mic;
    muteBtn.addEventListener("click", () => {
      // 不做本地乐观更新：Rust 侧会 emit "volume" 事件回来驱动重渲染
      // 不变量：静音 ⟺ self_gain == 0。进入静音写 0；解除静音回默认 100%（先写增益再切静音）
      if (volState.muted) {
        invoke("set_self_gain", { gain: 1.0 }).catch((e) => console.error("恢复音量失败:", e));
        invoke("set_muted", { on: false }).catch((e) => console.error("解除静音失败:", e));
      } else {
        invoke("set_self_gain", { gain: 0 }).catch((e) => console.error("音量归零失败:", e));
        invoke("set_muted", { on: true }).catch((e) => console.error("静音失败:", e));
      }
    });
    btns.appendChild(muteBtn);
```

- [ ] **Step 2: 挂音量滑块 input**

同文件定位（音量面板滑块监听，Task 6 写入的标记行）：

```js
volSlider.addEventListener("input", () => {
  volVal.textContent = volSlider.value + "%";
  // T7 在此 invoke 实时生效（set_self_gain / set_peer_gain）
});
```

替换为：

```js
volSlider.addEventListener("input", () => {
  volVal.textContent = volSlider.value + "%";
  if (!volTarget) return;
  const gain = Number(volSlider.value) / 100;
  if (volTarget.uid === myUid) {
    if (gain === 0) {
      // 拖到 0% = 进入静音（与静音键同效：不发包 + 广播）；已静音则不重复发
      if (!volState.muted) {
        invoke("set_self_gain", { gain: 0 }).catch((e) => console.error("音量归零失败:", e));
        invoke("set_muted", { on: true }).catch((e) => console.error("静音失败:", e));
      }
    } else {
      // 实时调增益；若正在静音中（滑块从 0 拖出）则同时解除静音
      invoke("set_self_gain", { gain }).catch((e) => console.error("设置自己音量失败:", e));
      if (volState.muted) {
        invoke("set_muted", { on: false }).catch((e) => console.error("解除静音失败:", e));
      }
    }
  } else {
    invoke("set_peer_gain", { nickname: volTarget.nickname, gain }).catch((e) =>
      console.error("设置他人音量失败:", e)
    );
  }
});
```

回显闭环说明：命令落地后 Rust emit `volume` → T6 的监听仅在“值不同”时写滑块，拖动过程中不会被回写打架；静音联动始终维持 `muted ⟺ self_gain == 0` 不变量（先写增益后切静音），因此滑块/面板显示无需任何静音特判（静音中打开面板自然显示 0%）。

- [ ] **Step 3: 双客户端手动验证**

前置（三个终端）：
1. 终端 1：`cargo run -p echoroom-server --bin echoroom-server`
2. 终端 2：`cargo tauri dev`（实例 A；构建产物在 `E:\pro\EchoRoom\target\debug\echoroom-client.exe`）
3. 终端 3：第二实例（独立 APPDATA，避免两实例共享同一 `config.json`）：

```powershell
$env:APPDATA = "E:\pro\EchoRoom\target\test-appdata"; Start-Process E:\pro\EchoRoom\target\debug\echoroom-client.exe
```

（如 exe 不存在先 `cargo build -p echoroom-client`。首次运行弹设置面板，填不同昵称，如 A=小K、B=阿信。）

Expected:
- A 点自己卡片静音按钮 → A 按钮变红激活；**B 侧 A 卡片名字条前出现红色静音图标**；A 对着麦说话，B 侧 A 的蓝框不亮且听不到声音；A 再点解除 → **音量回 100%（面板滑块在 100%）**、图标消失、蓝框随说话恢复点亮
- B 拖 **A（他人卡片）** 的音量滑块到 0% → B 听不到 A；拉到 200% → A 的声音明显更大；拖动过程中实时生效
- A 拖 **自己卡片** 的音量到 0% → **等价于静音**：A 按钮变红、B 侧出现静音图标、B 听不到 A；把滑块从 0 拖回 50% → 解除静音（B 图标消失），B 听到的音量约为原来的 50%
- 关闭音量面板再打开 → 滑块停在已设值（`volState` 回显）；静音中打开 → 显示 0%；点空白处或再点音量按钮 → 面板关闭

- [ ] **Step 4: Commit**

```bash
git add client/ui/app.js
git commit -m "feat: wire mute button and volume slider to tauri commands"
```

---

### Task 8: 进出音效——资源复制 + 播放逻辑 + 自动播放策略

**Files:**
- Create: `client/ui/in.mp3`、`client/ui/out.mp3`（从仓库根目录复制）
- Modify: `client/ui/app.js`
- Modify: `client/src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: T6 的 `members` / `member_join` / `member_leave` 监听；`E:\pro\EchoRoom\in.mp3`、`E:\pro\EchoRoom\out.mp3`（已存在，各约 62KB，内容不同）
- Produces: 进出音效（自己首次进房 + 他人进出；重连不重播；自己退出不播；固定音量 0.5）

- [ ] **Step 1: 复制资源到前端目录**

```powershell
Copy-Item E:\pro\EchoRoom\in.mp3 E:\pro\EchoRoom\client\ui\in.mp3
Copy-Item E:\pro\EchoRoom\out.mp3 E:\pro\EchoRoom\client\ui\out.mp3
```

（源文件放在仓库根目录（非 `target/`——构建目录可能被清理）；`ui/` 副本随 `frontendDist` 打包进应用。）

- [ ] **Step 2: app.js 播放逻辑（三处）**

(a) 定位 `// ---- 音量弹出面板（单例） ----`，在其上方插入：

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

(b) `members` 监听（首次同步 = 自己进房成功），末尾补音效：

```js
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname, muted] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted });
    }
    renderMembers();
    renderOnline();
    if (!everEntered) {
      everEntered = true;
      playSnd(sndIn); // 自己首次进入；之后的重连同步不播
    }
  });
```

(c) `member_join` / `member_leave` 各补一行：

```js
  await listen("member_join", (e) => {
    const [uid, nickname] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false });
    renderMembers();
    renderOnline();
    playSnd(sndIn); // 别人进入
  });
  await listen("member_leave", (e) => {
    members.delete(e.payload);
    renderMembers();
    renderOnline();
    playSnd(sndOut); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）
  });
```

- [ ] **Step 3: tauri.conf.json——放开 WebView2 自动播放策略**

音效由远端事件触发（自己没有用户手势），WebView2 默认按 Chromium 自动播放策略拦截"无手势播放有声媒体"。`client/src-tauri/tauri.conf.json` 的 main 窗口中加 `additionalBrowserArgs`（显式保留 Tauri 默认的 `--disable-features` 参数集，不改动既有行为）：

```json
      {
        "label": "main",
        "title": "Echo",
        "width": 768,
        "height": 640,
        "minWidth": 768,
        "minHeight": 560,
        "resizable": true,
        "additionalBrowserArgs": "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required"
      }
```

- [ ] **Step 4: 验证（含自动播放策略）**

沿用 T7 步骤 3 的三终端环境（A=`cargo tauri dev` 已配置昵称；B=独立 APPDATA 实例）：

1. 启动第三个实例（再开一个带独立/沿用 test-appdata 的 exe）→ **三边都听到 in.mp3**；关闭该实例 → 剩余方听到 out.mp3
2. **无手势场景**（关键）：完全退出 A → 重新 `cargo tauri dev` → 自动连接成功瞬间应听到 in.mp3（验证 `additionalBrowserArgs` 已生效；若无声：确认 `cargo tauri dev` 已重启、配置正确落盘）
3. **重连不重播**：保持 A 打开，终端 1 `Ctrl+C` 杀 server → 重启 server → A 自动重连成功 → **不**应播 in.mp3
4. 音量固定 0.5：音效响度明显低于系统全量（主观通过即可）

- [ ] **Step 5: Commit**

```bash
git add client/ui/in.mp3 client/ui/out.mp3 client/ui/app.js client/src-tauri/tauri.conf.json
git commit -m "feat: entry/exit sounds with webview autoplay policy"
```

---

### Task 9: 端到端验收（手动清单）

**Files:** 无代码改动（纯验收）

**Interfaces:**
- Consumes: Task 1–8 全部产物
- Produces: 验收记录（异常项回写任务清单）

- [ ] **Step 1: 全量测试与构建**

Run: `cargo test; cargo build -p echoroom-client`
Expected: 全部 `test result: ok`；构建成功无错误

- [ ] **Step 2: 冷启动验收（首次使用路径）**

```powershell
Rename-Item "$env:APPDATA\com.echoroom.dev\config.json" "config.json.bak" -ErrorAction SilentlyContinue
```

`cargo tauri dev` → 弹设置面板 → 填昵称/服务器 → 点连接 → **in.mp3 响**（有用户手势场景）。验收后还原旧配置：

```powershell
Move-Item -Force "$env:APPDATA\com.echoroom.dev\config.json.bak" "$env:APPDATA\com.echoroom.dev\config.json"
```

- [ ] **Step 3: 双客户端验收清单（耳朵验收由用户执行）**

环境同 T7 步骤 3（server + 实例 A + 实例 B）。逐项通过后勾选：

- [ ] 1. B 拖低/拉高 A 的音量（0% / 200%）→ 对 A 的听感随之变小/变大，拖动实时生效
- [ ] 2. A 点静音（或拖自己滑块到 0%）→ B 侧 A 卡片出现红色静音图标、A 自己蓝框不亮、B 听不到 A；点静音键解除 → 音量回 100%（滑块跳 100）、蓝框恢复、声音回来；从静音把滑块拖出（如 50%）→ 解除且音量为 50%；此时 C 新进 → C 的 LoginOk 记录同步即带 A 的静音图标
- [ ] 3. A 拖自己音量（50% / 200%）→ B 对 A 的听感随比例改变（拖到 0% 的静音场景见第 2 条）
- [ ] 4. 双方分别重启客户端：音量（自己+他人按昵称）与静音设置保持；A 静音态重启重连后 B 侧仍显示静音图标（重连补发 Mute 生效）
- [ ] 5. hover 投屏/摄像头按钮 → 显示"即将推出"，点击无效果、控制台无报错
- [ ] 6. 音效：新成员加入 → 所有人（含新人自己）听到 in.mp3；成员退出 → 剩余方听到 out.mp3；server 重启触发重连 → 不播
- [ ] 7. 回归：公屏聊天、说话蓝框、窗口缩放（3~6 列卡片、第二行滚动）与改造前行为一致

- [ ] Step 4: 收尾

验收通过后将结果（含异常项与处理）记录在任务清单；代码提交按用户指示执行。之后按 executing-plans 流程进入 finishing-a-development-branch。

---

## 附：任务依赖关系

```
T1(协议) → T2(服务器)          （T2 依赖 T1 的消息定义与三元组类型）
T3(config) → T4(音频) → T5(网络+桥接)   （T5 依赖 T1 与 T4；T1/T2 线可与 T3/T4 线并行推进）
T5 → T6(前端卡片) → T7(接线) → T8(音效) → T9(验收)
```

T9 为纯验收任务，依赖全部前序任务。
