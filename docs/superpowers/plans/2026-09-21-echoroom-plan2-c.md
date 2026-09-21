# EchoRoom 计划2 · C 子项目（账号系统）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 EchoRoom 引入账号体系：账号+密码（argon2id）注册/登录、邀请码准入、SQLite 持久化（昵称/头像存服务器）、auth_token 自动登录、头像上传与懒加载渲染——身份从一次性昵称升级为持久账号，认证成功即进房。

**Architecture:** 账号持久化进 SQLite（服务器 `data/echoroom.db`），在线成员仍是内存 `Room`。TCP 连接首消息由"直接发昵称"改为 **Register / Login / Resume 三选一**，服务器查库校验通过后直接 join 房间（登录与进房合一）。登录/注册成功下发 `auth_token`（自动登录）与会话专用 `udp_token` 两个独立凭证。头像走懒加载：成员五元组带 `has_avatar` 标记，前端按需 `AvatarRequest` 拉取 `AvatarData`（≤64KB；服务器把图片存文件系统 `data/avatars/{id}.jpg`，DB 内只留标记）。

**Tech Stack:** Rust（protocol / server / client 三包；server 新增 `rusqlite`(bundled) / `argon2` / `rand@0.8` / `anyhow`）+ Tauri v2（`withGlobalTauri`）+ 原生 HTML/CSS/JS（无构建工具）。

## Global Constraints

- 协议改造：TCP type 1 `Login{account,password}`（替换 `{nickname}`）、2 `LoginOk{uid,udp_token,auth_token,members}`（字段重命名，members 含自己）、7 `LoginReject`→`AuthReject{reason}`（改名复用）；新增 15 `Register{account,password,invite}`、16 `Resume{auth_token}`、17 `SetProfile{nickname,avatar:Option<Vec<u8>>}`、18 `ProfileChanged{uid,nickname}`、19 `AvatarRequest{uid}`、20 `AvatarData{uid,data}`
- 成员元组全局五元组 `(uid, nickname, muted, streams, has_avatar)`；`MemberJoin` 同步加 `has_avatar: bool`
- bytes 字段编码 `[len u32][原始字节]`；`try_decode` 长度上限由 `64*1024` 提高到 `256*1024`（容纳头像单帧）
- 账号 `[A-Za-z0-9][A-Za-z0-9_]{2,19}`（3-20 位、不能以 `_` 开头）原样存储、大小写敏感（`Alice` 与 `alice` 是两个独立账号）；密码 6–64 字符（大小写敏感）；昵称 1–24 字符（允许重复，默认=账号名）；头像 ≤64KB（客户端缩 256×256 居中裁剪 JPEG q85）
- 每账号最多 8 条 auth_token（超出删最旧）；auth_token = 16 字节真随机 → 32 位 hex；UDP token 同步换真随机（`rand::rngs::OsRng`）
- 错误文案（服务器下发，客户端原样展示）：`未开放注册` / `邀请码错误` / `账号已存在` / `账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）` / `密码至少 6 位` / `密码过长（上限 64 字符）` / `昵称不合法（1-24 字符）` / `头像过大（上限 64KB）`；登录统一 `账号或密码错误`（不泄露账号是否存在）；Resume 失效 `登录已过期`；房间满 `房间已满（6人）`
- 服务器参数：位置参数 port（默认 9000）、`--data <目录>`（默认 `./data`，DB = `<data>/echoroom.db`，启动自动建目录/建表）、`--invite <码>`（未配置 = 拒绝一切注册）
- ⚠ C 为协议**破坏性升级**（复用 type 1/2/7 改语义），无法像 B 那样逐任务保持编译全绿：**Task 1 结束时仅 `echoroom-protocol` 可测（server/client 编译断裂属预期状态）**；Task 2 结束 server 恢复可测；Task 3 结束 workspace 恢复全绿。Task 1–2 期间不要试图在包之间来回打补丁，按任务顺序推进即可
- Task 3 起每个任务结束时全 workspace 编译通过、已有测试全绿
- 环境：Windows PowerShell（命令用 `;` 分隔；`cd` 用绝对路径）；测试命令 `cargo test -p echoroom-protocol` / `-p echoroom-server` / `-p echoroom-client`；前端无构建工具（原生 JS 直接放 `client/ui/`），改动后用 `node --check` 校验语法
- 客户端版本 **0.2.0**（`client/src-tauri/tauri.conf.json`；协议破坏性变更，旧客户端连接将被断开）
- 已知取舍（记录不处理）：他人音量 `peer_gains` 按昵称存储，对方改昵称后其音量设置回 100%（C 不做迁移）

## File Structure（全景）

**protocol**（协议）
- `protocol/src/messages.rs`：`Login`/`LoginOk` 改造、`LoginReject`→`AuthReject`、新增 `Register`/`Resume`/`SetProfile`/`ProfileChanged`/`AvatarRequest`/`AvatarData`、`MemberInfo` 五元组别名
- `protocol/src/tcp.rs`：bytes 字段编解码、全部新消息序列化、长度上限 256KB、往返测试

**server**（服务器）
- `server/src/db.rs`（新）：SQLite 持久层（建表 / 账号 CRUD / token 修剪 / 头像文件读写）
- `server/src/auth.rs`（新）：格式校验、argon2id 哈希与验证、token 生成、注册/登录/Resume/资料更新业务
- `server/src/room.rs`：`Member.account_id`/`has_avatar`、`join` 五元组、`update_nickname`/`set_has_avatar`/`account_id_of`、UDP token 换真随机
- `server/src/tcp.rs`：`read_first_auth` 三路认证、`SetProfile`/`AvatarRequest` 处理、`LoginOk` 组装含自己
- `server/src/main.rs`：`--data`/`--invite` 参数解析与 DB 初始化
- `server/src/bin/sim_clients.rs`：认证适配（带邀请码注册，已存在回退登录）
- `server/Cargo.toml`：`rusqlite`(bundled) / `argon2` / `rand@0.8` / `anyhow`

**client / src-tauri**（客户端 Rust）
- `src/config.rs`：加 `account`/`auth_token`，删 `nickname`
- `src/net/tcp.rs`：`AuthMode` 三态、首消息流动（有 token 优先 Resume）、五元组、`AuthReject` 双语义（未进房=认证失败 / 已进房=资料错误）、新事件出口、`AuthFailed` 停止重连循环
- `src/bridge.rs`：`auth_login`/`auth_register`/`auto_connect`/`set_profile`/`avatar_request` 命令；`auth_ok`/`auth_fail`/`profile_changed`/`profile_error`/`avatar_data` 事件；token 持久化/清除
- `src/lib.rs`：命令注册更新

**client / ui**（前端）
- `ui/index.html`：登录/注册面板（改造 setup-mask）+ 完善资料弹窗（新）
- `ui/app.js`：认证流程接线、头像缓存/懒加载/渲染、资料弹窗逻辑
- `ui/style.css`：错误行/链接按钮/头像图/资料弹窗样式
- `tauri.conf.json`：版本 0.2.0

---

### Task 1: 协议扩展——认证消息（type 1/2/7 改造 + 15–20 新增 + 成员五元组）

**Files:**
- Modify: `protocol/src/messages.rs`（全文件替换）
- Modify: `protocol/src/tcp.rs`（encode/decode/Reader/测试）
- 注：本任务结束时 `server` 与 `client` **编译断裂属预期**（引用了旧 `Login{nickname}`/`LoginReject`/四元组），由 Task 2/3 修复

**Interfaces:**
- Consumes: 现有手写序列化约定（u16/u32 大端、String = `[len u16][utf8]`、bool = u8）
- Produces（后续任务依赖）：
  - `MemberInfo = (u16, String, bool, u8, bool)`（uid, 昵称, 静音, 流位图, 有头像）
  - `TcpMessage::Login { account: String, password: String }`（type 1，C→S）
  - `TcpMessage::LoginOk { uid: u16, udp_token: u32, auth_token: String, members: Vec<MemberInfo> }`（type 2，S→C，members 含自己）
  - `TcpMessage::AuthReject { reason: String }`（type 7，S→C）
  - `TcpMessage::Register { account, password, invite }`（type 15，C→S）
  - `TcpMessage::Resume { auth_token: String }`（type 16，C→S）
  - `TcpMessage::SetProfile { nickname: String, avatar: Option<Vec<u8>> }`（type 17，C→S）
  - `TcpMessage::ProfileChanged { uid: u16, nickname: String }`（type 18，S→C 广播）
  - `TcpMessage::AvatarRequest { uid: u16 }`（type 19，C→S）
  - `TcpMessage::AvatarData { uid: u16, data: Vec<u8> }`（type 20，S→C）
  - `TcpMessage::MemberJoin { uid, nickname, has_avatar }`（加字段）
  - `tcp::encode/ try_decode`：bytes 字段 `[len u32][原始字节]`；解码长度上限 256KB

- [ ] **Step 1: 替换 messages.rs 为认证版消息定义**

`protocol/src/messages.rs` 全文件替换为（UDP 部分保持原样）：

```rust
//! TCP 控制消息定义。类型号固定，客户端与服务端共用。

/// 流类型常量（u8 位图 bit0 = 屏幕 / bit1 = 摄像头）
pub const STREAM_SCREEN: u8 = 0;
pub const STREAM_CAMERA: u8 = 1;

/// 在线成员元组：(uid, 昵称, 是否静音, 流位图, 是否有头像)
pub type MemberInfo = (u16, String, bool, u8, bool);

#[derive(Debug, Clone, PartialEq)]
pub enum TcpMessage {
    /// C→S：登录（认证成功即进房）
    Login { account: String, password: String },
    /// S→C：认证成功（`auth_token` 供下次自动登录；`udp_token` 绑定 UDP 地址；members 含自己）
    LoginOk { uid: u16, udp_token: u32, auth_token: String, members: Vec<MemberInfo> },
    /// S→C：有成员加入
    MemberJoin { uid: u16, nickname: String, has_avatar: bool },
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
    /// 双向：C→S（uid 填 0）上报本端开/停某路流；S→C 广播（uid 为流主）
    StreamState { uid: u16, kind: u8, on: bool },
    /// C→S（uid 填 0）：订阅某人的视频流（覆盖式）
    Subscribe { uid: u16, target: u16 },
    /// C→S（uid 填 0）：取消订阅
    Unsubscribe { uid: u16 },
    /// S→C 定向（发给流主与该流全部订阅者）：当前观看者 uid 名单（人数 = len）
    Viewers { uids: Vec<u16> },
    /// 双向：C→S（uid 填 0）请求目标发关键帧；S→C 转发（uid 为请求者）
    RequestKeyframe { uid: u16, target: u16 },
    /// S→C：认证被拒（未进房 = 断开；已进房 = 资料更新失败提示）；也用于房间满
    AuthReject { reason: String },
    /// C→S：注册（成功即进房，昵称 = 账号名）
    Register { account: String, password: String, invite: String },
    /// C→S：自动登录
    Resume { auth_token: String },
    /// C→S：更新资料（`avatar` = None 表示不改头像）
    SetProfile { nickname: String, avatar: Option<Vec<u8>> },
    /// S→C 广播：昵称变更（头像变化由前端重拉 AvatarData 感知）
    ProfileChanged { uid: u16, nickname: String },
    /// C→S：请求某人的头像（懒加载）
    AvatarRequest { uid: u16 },
    /// S→C：头像数据（data 为空 = 无头像）
    AvatarData { uid: u16, data: Vec<u8> },
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
            TcpMessage::AuthReject { .. } => 7,
            TcpMessage::Mute { .. } => 8,
            TcpMessage::Muted { .. } => 9,
            TcpMessage::StreamState { .. } => 10,
            TcpMessage::Subscribe { .. } => 11,
            TcpMessage::Unsubscribe { .. } => 12,
            TcpMessage::Viewers { .. } => 13,
            TcpMessage::RequestKeyframe { .. } => 14,
            TcpMessage::Register { .. } => 15,
            TcpMessage::Resume { .. } => 16,
            TcpMessage::SetProfile { .. } => 17,
            TcpMessage::ProfileChanged { .. } => 18,
            TcpMessage::AvatarRequest { .. } => 19,
            TcpMessage::AvatarData { .. } => 20,
        }
    }
}
```

（`UdpPacket` 及其 `type_id` 保持文件原样不动。）

- [ ] **Step 2: tcp.rs——加 put_bytes 与 Reader.bytes、提高长度上限**

`protocol/src/tcp.rs`：在 `put_str` 之后加：

```rust
/// bytes 字段：[len u32][原始字节]（头像等大块二进制；u16 前缀不够 64KB 上限）
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}
```

`Reader` 的 `string` 方法之后加：

```rust
    fn bytes(&mut self) -> Result<Vec<u8>, FieldErr> {
        let len = self.u32()? as usize;
        self.need(len)?;
        let b = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(b)
    }
```

`try_decode` 中长度检查一行替换（原 `len > 64 * 1024`）：

```rust
    // 上限 256KB：容纳头像单帧（≤64KB BLOB + 头部）与成员列表余量
    if len < 1 || len > 256 * 1024 {
        return Err(DecodeError::UnknownType(0)); // 长度异常：视为协议错误
    }
```

- [ ] **Step 3: tcp.rs——encode 全量替换**

`protocol/src/tcp.rs` 的 `encode` 函数整体替换为：

```rust
pub fn encode(msg: &TcpMessage) -> Vec<u8> {
    let mut payload = Vec::new();
    match msg {
        TcpMessage::Login { account, password } => {
            put_str(&mut payload, account);
            put_str(&mut payload, password);
        }
        TcpMessage::LoginOk { uid, udp_token, auth_token, members } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&udp_token.to_be_bytes());
            put_str(&mut payload, auth_token);
            payload.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for (uid, name, muted, streams, has_avatar) in members {
                payload.extend_from_slice(&uid.to_be_bytes());
                put_str(&mut payload, name);
                payload.push(if *muted { 1 } else { 0 });
                payload.push(*streams);
                payload.push(if *has_avatar { 1 } else { 0 });
            }
        }
        TcpMessage::MemberJoin { uid, nickname, has_avatar } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, nickname);
            payload.push(if *has_avatar { 1 } else { 0 });
        }
        TcpMessage::MemberLeave { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::Chat { uid, text } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, text);
        }
        TcpMessage::Speaking { uid, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::Mute { uid, on } | TcpMessage::Muted { uid, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::StreamState { uid, kind, on } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.push(*kind);
            payload.push(if *on { 1 } else { 0 });
        }
        TcpMessage::Subscribe { uid, target } | TcpMessage::RequestKeyframe { uid, target } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&target.to_be_bytes());
        }
        TcpMessage::Unsubscribe { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::Viewers { uids } => {
            payload.extend_from_slice(&(uids.len() as u16).to_be_bytes());
            for uid in uids {
                payload.extend_from_slice(&uid.to_be_bytes());
            }
        }
        TcpMessage::AuthReject { reason } => put_str(&mut payload, reason),
        TcpMessage::Register { account, password, invite } => {
            put_str(&mut payload, account);
            put_str(&mut payload, password);
            put_str(&mut payload, invite);
        }
        TcpMessage::Resume { auth_token } => put_str(&mut payload, auth_token),
        TcpMessage::SetProfile { nickname, avatar } => {
            put_str(&mut payload, nickname);
            match avatar {
                Some(b) => {
                    payload.push(1);
                    put_bytes(&mut payload, b);
                }
                None => payload.push(0),
            }
        }
        TcpMessage::ProfileChanged { uid, nickname } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_str(&mut payload, nickname);
        }
        TcpMessage::AvatarRequest { uid } => payload.extend_from_slice(&uid.to_be_bytes()),
        TcpMessage::AvatarData { uid, data } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            put_bytes(&mut payload, data);
        }
    }
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    out.push(msg.type_id());
    out.extend_from_slice(&payload);
    out
}
```

- [ ] **Step 4: tcp.rs——decode 的 type 1/2/3/7 替换 + 15–20 追加**

`try_decode` 中 `let msg = match type_id {` 的各分支修改为（1/2/3 替换、7 替换、15–20 在 `14 =>` 分支后追加；4–6、8–14 保持原样）：

```rust
        1 => {
            let account = field!(r.string());
            TcpMessage::Login { account, password: field!(r.string()) }
        }
        2 => {
            let uid = field!(r.u16());
            let udp_token = field!(r.u32());
            let auth_token = field!(r.string());
            let count = field!(r.u16()) as usize;
            let mut members = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let m_uid = field!(r.u16());
                let name = field!(r.string());
                let muted = field!(r.u8()) != 0;
                let streams = field!(r.u8());
                let has_avatar = field!(r.u8()) != 0;
                members.push((m_uid, name, muted, streams, has_avatar));
            }
            TcpMessage::LoginOk { uid, udp_token, auth_token, members }
        }
        3 => {
            let uid = field!(r.u16());
            let nickname = field!(r.string());
            TcpMessage::MemberJoin { uid, nickname, has_avatar: field!(r.u8()) != 0 }
        }
```

```rust
        7 => TcpMessage::AuthReject { reason: field!(r.string()) },
```

```rust
        15 => {
            let account = field!(r.string());
            let password = field!(r.string());
            TcpMessage::Register { account, password, invite: field!(r.string()) }
        }
        16 => TcpMessage::Resume { auth_token: field!(r.string()) },
        17 => {
            let nickname = field!(r.string());
            let has_avatar = field!(r.u8()) != 0;
            let avatar = if has_avatar { Some(field!(r.bytes())) } else { None };
            TcpMessage::SetProfile { nickname, avatar }
        }
        18 => {
            let uid = field!(r.u16());
            TcpMessage::ProfileChanged { uid, nickname: field!(r.string()) }
        }
        19 => TcpMessage::AvatarRequest { uid: field!(r.u16()) },
        20 => {
            let uid = field!(r.u16());
            TcpMessage::AvatarData { uid, data: field!(r.bytes()) }
        }
```

- [ ] **Step 5: 替换 tcp.rs 测试模块**

`#[cfg(test)] mod tests` 的 `roundtrip_all_variants` 与 `partial_data_returns_none` 替换、新增头像大帧测试（`two_messages_in_one_buffer`、`unknown_type_is_error`、`bad_utf8_is_error` 保持原样）：

```rust
    #[test]
    fn roundtrip_all_variants() {
        let samples = vec![
            TcpMessage::Login { account: "alice".into(), password: "secret123".into() },
            TcpMessage::LoginOk {
                uid: 3,
                udp_token: 0xDEAD_BEEF,
                auth_token: "0123456789abcdef0123456789abcdef".into(),
                members: vec![(1, "小K".into(), false, 0, true), (2, "你".into(), true, 0b11, false)],
            },
            TcpMessage::MemberJoin { uid: 5, nickname: "新来的".into(), has_avatar: true },
            TcpMessage::MemberLeave { uid: 2 },
            TcpMessage::Chat { uid: 1, text: "晚上开黑吗".into() },
            TcpMessage::Speaking { uid: 4, on: true },
            TcpMessage::Mute { uid: 0, on: true },
            TcpMessage::Muted { uid: 4, on: true },
            TcpMessage::StreamState { uid: 0, kind: 1, on: true },
            TcpMessage::StreamState { uid: 4, kind: 0, on: false },
            TcpMessage::Subscribe { uid: 0, target: 4 },
            TcpMessage::Unsubscribe { uid: 0 },
            TcpMessage::Viewers { uids: vec![1, 2, 3] },
            TcpMessage::Viewers { uids: vec![] },
            TcpMessage::RequestKeyframe { uid: 0, target: 4 },
            TcpMessage::AuthReject { reason: "账号或密码错误".into() },
            TcpMessage::Register { account: "bob".into(), password: "pw123456".into(), invite: "echo-2026".into() },
            TcpMessage::Resume { auth_token: "deadbeef".into() },
            TcpMessage::SetProfile { nickname: "阿信".into(), avatar: None },
            TcpMessage::SetProfile { nickname: "阿信".into(), avatar: Some(vec![0xFF, 0xD8, 0xFF, 0xE0]) },
            TcpMessage::ProfileChanged { uid: 3, nickname: "阿信".into() },
            TcpMessage::AvatarRequest { uid: 3 },
            TcpMessage::AvatarData { uid: 3, data: vec![] },
            TcpMessage::AvatarData { uid: 3, data: vec![1, 2, 3, 4] },
        ];
        for msg in samples {
            let bytes = encode(&msg);
            let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
            assert_eq!(decoded, msg);
            assert_eq!(n, bytes.len());
        }
    }

    #[test]
    fn avatar_data_roundtrip_64kb() {
        // 头像上限 64KB：单帧必须可编码可解码（长度上限 256KB 之内）
        let msg = TcpMessage::AvatarData { uid: 7, data: vec![0xAB; 64 * 1024] };
        let bytes = encode(&msg);
        let (decoded, n) = try_decode(&bytes).unwrap().unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(n, bytes.len());
    }

    #[test]
    fn partial_data_returns_none() {
        let bytes = encode(&TcpMessage::Login { account: "abc".into(), password: "x".into() });
        for cut in 0..bytes.len() {
            assert!(try_decode(&bytes[..cut]).unwrap().is_none(), "cut={cut} 应等待更多数据");
        }
    }
```

- [ ] **Step 6: 运行协议包测试**

Run: `cargo test -p echoroom-protocol`
Expected: 全绿（TCP 往返含 6 条新消息与五元组、64KB 头像单帧、截断等待、粘包、未知类型、坏 UTF-8）。
```
（server/client 当前编译不过属预期，见 Global Constraints。）
```

- [ ] **Step 7: 记录断裂范围（不修复）**

确认断裂仅来自引用点，供 Task 2/3 校验修复完整：
- server：`room.rs`（`join(nickname)`/四元组/`MemberJoin`）、`tcp.rs`（`read_first_login`/`LoginReject`/四元组）、`bin/sim_clients.rs`（`Login{nickname}`/`LoginOk{token}`）
- client：`net/tcp.rs`（`Login{nickname}`/`LoginOk{token}`/`LoginReject`/`MemberJoin` 二元组）、`bridge.rs`（`emit_member_list` 四元组）、`config.rs`（`nickname` 字段）

Run: `cargo check -p echoroom-server`（预期报错：`Login`/`LoginOk` 等类型不匹配）
Expected: 报错清单与上述一致（多出的报错逐条核对后留给 Task 2）

---

### Task 2: 服务器——SQLite 持久层与认证业务（db.rs + auth.rs）

**Files:**
- Modify: `server/Cargo.toml`（`cargo add`）
- Create: `server/src/db.rs`
- Create: `server/src/auth.rs`

**Interfaces:**
- Consumes: Task 1 的协议类型（集成步骤 6–9 使用）
- 任务结构：持久层/业务两个新模块（Step 1–5）+ 服务器整体切换到认证协议（Step 6–9）。**本任务结束时 `cargo test -p echoroom-server` 全绿**；client 仍断裂（Task 3 修复）
- Produces（Task 3 依赖）：
  - `db::Db`：`open(path) -> anyhow::Result<Db>`；`create_account(account, password_hash, nickname) -> Result<i64, DbError>`；`find_account(account) -> Option<Account>`；`insert_token(account_id, token) -> anyhow::Result<()>`（自动修剪到 8 条 + 清理过期）；`find_account_by_token(token) -> Option<Account>`（仅命中 24h 内使用过的，见 TOKEN_TTL_SECS）；`touch_token(token)`（滑动续期，Resume 成功时回写 last_used_at）；`update_nickname(account_id, nickname)`；`update_avatar(account_id, avatar)`；`get_avatar(account_id) -> Option<Vec<u8>>`；`#[cfg(test)] open_in_memory()`
  - `db::Account { id: i64, account: String, password_hash: String, nickname: String, has_avatar: bool }`；`db::DbError::{AccountExists, Other}`
  - `auth::AuthResult { account_id: i64, nickname: String, has_avatar: bool, auth_token: String }`
  - `auth::register(db, invite_cfg: Option<&str>, account, password, invite) -> Result<AuthResult, String>`
  - `auth::login(db, account, password) -> Result<AuthResult, String>`
  - `auth::resume(db, auth_token) -> Result<AuthResult, String>`
  - `auth::apply_profile(db, account_id, nickname, avatar: Option<&[u8]>) -> Result<(), String>`
  - `auth::validate_nickname/validate_account/validate_password`；`auth::AVATAR_MAX: usize = 64*1024`
  - `room::Room::join(nickname, account_id: i64, has_avatar: bool, tx)`（`JoinOk.members: Vec<MemberInfo>` 五元组）；`update_nickname(uid, &str) -> bool`；`set_has_avatar(uid, bool) -> bool`；`account_id_of(uid) -> Option<i64>`
  - `tcp::serve(listener, room: Arc<Mutex<Room>>, db: Arc<Db>, invite: Option<String>)`；`main.rs` 参数：`echoroom-server [port] [--data <目录>] [--invite <码>]`

- [ ] **Step 1: 添加服务器依赖**

`rand` 必须锁 `0.8`：其 `rand_core 0.6` 与 argon2 的 `password-hash` 依赖同一版本，`OsRng` 可直接传给 `SaltString::generate`；新版 rand（0.9+）会引入 rand_core 0.9 造成类型不兼容。

Run:
```powershell
cd E:\pro\EchoRoom\server; cargo add rusqlite --features bundled; cargo add argon2; cargo add rand@0.8; cargo add anyhow
```
Expected: 四个依赖加入 `server/Cargo.toml` 的 `[dependencies]`（版本号以 cargo 解析结果为准；`rusqlite` 带 `bundled` feature）。

- [ ] **Step 2: 新建 db.rs（持久层实现）**

`server/src/db.rs`（新建）：

```rust
//! SQLite 持久层：账号、密码哈希、token（含 8 条修剪与 24h 滑动过期）；头像图片存文件系统
//! （<数据目录>/avatars/<account_id>.jpg），库内只留 has_avatar 标记。
//! 单连接 + Mutex 串行化（6 人规模足够）；rusqlite bundled 把 SQLite 编进二进制。
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

/// 每账号最多保留的 auth_token 条数（超出删最旧，支持换设备）
pub const TOKEN_LIMIT: usize = 8;

/// auth_token 有效期（秒）：超过此时长未使用即失效；每次使用滑动续期
pub const TOKEN_TTL_SECS: i64 = 24 * 3600;

/// 账号记录（has_avatar 为库内标记列：0 无头像 / 1 有头像）
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: i64,
    pub account: String,
    pub password_hash: String,
    pub nickname: String,
    pub has_avatar: bool,
}

#[derive(Debug)]
pub enum DbError {
    /// 账号 UNIQUE 约束冲突（并发注册兜底）
    AccountExists,
    Other(String),
}

pub struct Db {
    conn: Mutex<Connection>,
    /// 头像文件目录（`<db 文件所在目录>/avatars/<account_id>.jpg`）
    avatar_dir: PathBuf,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        account: row.get(1)?,
        password_hash: row.get(2)?,
        nickname: row.get(3)?,
        has_avatar: row.get(4)?,
    })
}

impl Db {
    /// 打开（或创建）数据库文件并确保表结构；父目录与头像目录自动创建
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        Db::with_conn(Connection::open(path)?, dir.join("avatars"))
    }

    /// 组装 Db：建头像目录 + 初始化表结构
    fn with_conn(conn: Connection, avatar_dir: PathBuf) -> anyhow::Result<Db> {
        std::fs::create_dir_all(&avatar_dir)?;
        let db = Db { conn: Mutex::new(conn), avatar_dir };
        db.init_schema()?;
        Ok(db)
    }

    /// 内存库（测试用；auth.rs 的测试也复用）；头像落盘临时目录（pid + 纳秒命名，互不干扰）
    #[cfg(test)]
    pub fn open_in_memory() -> Db {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("echoroom-test-avatars-{}-{nanos}", std::process::id());
        Db::with_conn(Connection::open_in_memory().unwrap(), std::env::temp_dir().join(name))
            .unwrap()
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
               id            INTEGER PRIMARY KEY AUTOINCREMENT,
               account       TEXT NOT NULL UNIQUE,
               password_hash TEXT NOT NULL,
               nickname      TEXT NOT NULL,
               has_avatar    INTEGER NOT NULL DEFAULT 0,
               created_at    INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tokens (
               token        TEXT PRIMARY KEY,
               account_id   INTEGER NOT NULL,
               created_at   INTEGER NOT NULL,
               last_used_at INTEGER NOT NULL
             );",
        )?;
        // 旧库迁移：补 last_used_at 列（列已存在时 duplicate column 报错，忽略）
        let _ = conn.execute("ALTER TABLE tokens ADD COLUMN last_used_at INTEGER NOT NULL DEFAULT 0", []);
        // 旧行回填：以创建时间作最后使用时间（新库无 0 值行，无影响）
        conn.execute("UPDATE tokens SET last_used_at = created_at WHERE last_used_at = 0", [])?;
        Ok(())
    }

    /// 建账号；UNIQUE 冲突 → `AccountExists`
    pub fn create_account(
        &self,
        account: &str,
        password_hash: &str,
        nickname: &str,
    ) -> Result<i64, DbError> {
        let conn = self.conn.lock().unwrap();
        let r = conn.execute(
            "INSERT INTO accounts (account, password_hash, nickname, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![account, password_hash, nickname, now_secs()],
        );
        match r {
            Ok(_) => Ok(conn.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(DbError::AccountExists)
            }
            Err(e) => Err(DbError::Other(e.to_string())),
        }
    }

    pub fn find_account(&self, account: &str) -> Option<Account> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, account, password_hash, nickname, has_avatar
             FROM accounts WHERE account = ?1",
            params![account],
            row_to_account,
        )
        .ok()
    }

    /// 写入 auth_token：插入即开始滑动计时；顺带清理该账号过期 token 并修剪到 TOKEN_LIMIT 条
    pub fn insert_token(&self, account_id: i64, token: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = now_secs();
        conn.execute(
            "INSERT INTO tokens (token, account_id, created_at, last_used_at) VALUES (?1, ?2, ?3, ?3)",
            params![token, account_id, now],
        )?;
        // 过期清理：失效 token 不再占 8 条额度
        conn.execute(
            "DELETE FROM tokens WHERE account_id = ?1 AND last_used_at <= ?2",
            params![account_id, now - TOKEN_TTL_SECS],
        )?;
        conn.execute(
            "DELETE FROM tokens WHERE account_id = ?1 AND token NOT IN (
               SELECT token FROM tokens WHERE account_id = ?1
               ORDER BY last_used_at DESC, rowid DESC LIMIT ?2
             )",
            params![account_id, TOKEN_LIMIT as i64],
        )?;
        Ok(())
    }

    /// 按 token 查账号：仅命中 24h 内使用过的（使用即续期由 touch_token 完成）
    pub fn find_account_by_token(&self, token: &str) -> Option<Account> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT a.id, a.account, a.password_hash, a.nickname, a.has_avatar
             FROM tokens t JOIN accounts a ON a.id = t.account_id
             WHERE t.token = ?1 AND t.last_used_at > ?2",
            params![token, now_secs() - TOKEN_TTL_SECS],
            row_to_account,
        )
        .ok()
    }

    /// 滑动续期：使用 token 成功后回写最后使用时间（下一个 24h）
    pub fn touch_token(&self, token: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tokens SET last_used_at = ?1 WHERE token = ?2",
            params![now_secs(), token],
        )?;
        Ok(())
    }

    pub fn update_nickname(&self, account_id: i64, nickname: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET nickname = ?1 WHERE id = ?2",
            params![nickname, account_id],
        )?;
        Ok(())
    }

    /// 写头像文件后置库内标记；顺序=先文件后库（最坏留孤儿文件，无害；下次上传覆盖）
    pub fn update_avatar(&self, account_id: i64, avatar: &[u8]) -> anyhow::Result<()> {
        std::fs::write(self.avatar_dir.join(format!("{account_id}.jpg")), avatar)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET has_avatar = 1 WHERE id = ?1",
            params![account_id],
        )?;
        Ok(())
    }

    /// 读头像文件（不存在 → None）
    pub fn get_avatar(&self, account_id: i64) -> Option<Vec<u8>> {
        std::fs::read(self.avatar_dir.join(format!("{account_id}.jpg"))).ok()
    }
}
```

- [ ] **Step 3: db.rs 测试模块**

`server/src/db.rs` 文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_find_account() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "hash1", "alice").unwrap();
        let acc = db.find_account("alice").unwrap();
        assert_eq!(acc.id, id);
        assert_eq!(acc.nickname, "alice");
        assert_eq!(acc.password_hash, "hash1");
        assert!(!acc.has_avatar);
        assert!(db.find_account("bob").is_none());
    }

    #[test]
    fn duplicate_account_is_constraint_error() {
        let db = Db::open_in_memory();
        db.create_account("alice", "h1", "alice").unwrap();
        assert!(matches!(db.create_account("alice", "h2", "阿信"), Err(DbError::AccountExists)));
    }

    #[test]
    fn token_insert_find_and_trim() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        db.insert_token(id, "tok-0").unwrap();
        for i in 1..10 {
            db.insert_token(id, &format!("tok-{i}")).unwrap();
        }
        // 上限 8 条：最旧的两条被修剪，最新的仍在
        assert!(db.find_account_by_token("tok-0").is_none(), "最旧 token 应被修剪");
        assert!(db.find_account_by_token("tok-1").is_none());
        assert_eq!(db.find_account_by_token("tok-2").unwrap().id, id);
        assert_eq!(db.find_account_by_token("tok-9").unwrap().account, "alice");
        assert!(db.find_account_by_token("不存在").is_none());
    }

    #[test]
    fn update_nickname_and_avatar() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        assert!(!db.find_account("alice").unwrap().has_avatar);
        db.update_nickname(id, "阿信").unwrap();
        assert_eq!(db.find_account("alice").unwrap().nickname, "阿信");
        let img = vec![1u8, 2, 3, 4, 5];
        db.update_avatar(id, &img).unwrap();
        assert!(db.avatar_dir.join(format!("{id}.jpg")).exists(), "头像应落盘为文件");
        assert_eq!(db.get_avatar(id).unwrap(), img);
        assert!(db.find_account("alice").unwrap().has_avatar);
    }

    #[test]
    fn get_avatar_none_when_missing() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        assert_eq!(db.get_avatar(id), None);
        assert_eq!(db.get_avatar(999), None);
    }

    #[test]
    fn token_sliding_expiry() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        db.insert_token(id, "tok").unwrap();
        assert!(db.find_account_by_token("tok").is_some());
        // 模拟超过 24h 未使用 → 过期
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE tokens SET last_used_at = ?1 WHERE token = 'tok'",
                params![now_secs() - TOKEN_TTL_SECS - 60],
            )
            .unwrap();
        }
        assert!(db.find_account_by_token("tok").is_none(), "超 24h 未使用应失效");
        // 使用即续期：touch 后重新有效
        db.touch_token("tok").unwrap();
        assert!(db.find_account_by_token("tok").is_some(), "touch 后应恢复有效");
    }

    #[test]
    fn insert_token_cleans_expired() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        db.insert_token(id, "old-a").unwrap();
        db.insert_token(id, "old-b").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("UPDATE tokens SET last_used_at = ?1", params![now_secs() - TOKEN_TTL_SECS - 60])
                .unwrap();
        }
        db.insert_token(id, "new").unwrap();
        let cnt: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM tokens WHERE account_id = ?1", params![id], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(cnt, 1, "过期 token 应在插入时被清理，只留新的");
        assert!(db.find_account_by_token("new").is_some());
    }

    #[test]
    fn legacy_tokens_table_migrates() {
        // 模拟旧库：tokens 表无 last_used_at 列
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tokens (token TEXT PRIMARY KEY, account_id INTEGER NOT NULL, created_at INTEGER NOT NULL);
             INSERT INTO tokens VALUES ('legacy', 1, 12345);",
        )
        .unwrap();
        let db = Db::with_conn(conn, std::env::temp_dir().join("echoroom-test-migrate-avatars")).unwrap();
        let last: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("SELECT last_used_at FROM tokens WHERE token = 'legacy'", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(last, 12345, "旧行应回填 last_used_at = created_at");
    }
}
```

- [ ] **Step 4: 新建 auth.rs（认证业务实现）**

`server/src/auth.rs`（新建）：

```rust
//! 账号业务逻辑：格式校验、argon2id 哈希、token 生成、注册/登录/Resume/资料更新。
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;

use crate::db::{Db, DbError};

/// 头像上限（字节）：客户端缩 256×256 JPEG q85 一般 10–30KB
pub const AVATAR_MAX: usize = 64 * 1024;

/// 认证成功后的身份凭证束
pub struct AuthResult {
    pub account_id: i64,
    pub nickname: String,
    pub has_avatar: bool,
    /// 供下次自动登录的会话 token（32 位 hex）
    pub auth_token: String,
}

pub fn validate_account(account: &str) -> Result<(), String> {
    let ok = (3..=20).contains(&account.len())
        && !account.starts_with('_')
        && account.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err("账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）".into())
    }
}

pub fn validate_password(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < 6 {
        return Err("密码至少 6 位".into());
    }
    if n > 64 {
        return Err("密码过长（上限 64 字符）".into());
    }
    Ok(())
}

pub fn validate_nickname(nickname: &str) -> Result<(), String> {
    let n = nickname.chars().count();
    if (1..=24).contains(&n) {
        Ok(())
    } else {
        Err("昵称不合法（1-24 字符）".into())
    }
}

/// argon2id（PHC 字符串自带盐与参数，直接入库）
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("哈希失败: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

/// 16 字节真随机 → 32 位 hex
pub fn new_auth_token() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 注册：邀请码校验 → 格式校验 → 建号（昵称 = 账号名）→ 发 token
pub fn register(
    db: &Db,
    invite_cfg: Option<&str>,
    account: &str,
    password: &str,
    invite: &str,
) -> Result<AuthResult, String> {
    let Some(expected) = invite_cfg else {
        return Err("未开放注册".into());
    };
    if invite != expected {
        return Err("邀请码错误".into());
    }
    let account = account.trim().to_string();
    validate_account(&account)?;
    validate_password(password)?;
    let hash = hash_password(password).map_err(|e| format!("服务器错误：{e}"))?;
    let id = match db.create_account(&account, &hash, &account) {
        Ok(id) => id,
        Err(DbError::AccountExists) => return Err("账号已存在".into()),
        Err(DbError::Other(e)) => return Err(format!("服务器错误：{e}")),
    };
    let token = new_auth_token();
    db.insert_token(id, &token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult { account_id: id, nickname: account, has_avatar: false, auth_token: token })
}

/// 登录：查账号 → argon2 校验 → 发新 token（失败统一文案，不泄露账号是否存在）
pub fn login(db: &Db, account: &str, password: &str) -> Result<AuthResult, String> {
    let account = account.trim();
    let Some(acc) = db.find_account(&account) else {
        return Err("账号或密码错误".into());
    };
    if !verify_password(password, &acc.password_hash) {
        return Err("账号或密码错误".into());
    }
    let token = new_auth_token();
    db.insert_token(acc.id, &token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult {
        account_id: acc.id,
        nickname: acc.nickname,
        has_avatar: acc.has_avatar,
        auth_token: token,
    })
}

/// 自动登录：token 换身份（失效/超 24h 未用 → 客户端回登录页）
pub fn resume(db: &Db, auth_token: &str) -> Result<AuthResult, String> {
    let Some(acc) = db.find_account_by_token(auth_token) else {
        return Err("登录已过期".into());
    };
    // 滑动续期：本次使用成功即刷新 24h 有效期（活跃则可持续免登）
    db.touch_token(auth_token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult {
        account_id: acc.id,
        nickname: acc.nickname,
        has_avatar: acc.has_avatar,
        auth_token: auth_token.to_string(),
    })
}

/// 资料更新：昵称必填；头像 Some 才更新（None = 保持原样）
pub fn apply_profile(
    db: &Db,
    account_id: i64,
    nickname: &str,
    avatar: Option<&[u8]>,
) -> Result<(), String> {
    validate_nickname(nickname)?;
    if let Some(data) = avatar {
        if data.is_empty() {
            return Err("头像数据为空".into());
        }
        if data.len() > AVATAR_MAX {
            return Err("头像过大（上限 64KB）".into());
        }
    }
    db.update_nickname(account_id, nickname).map_err(|e| format!("服务器错误：{e}"))?;
    if let Some(data) = avatar {
        db.update_avatar(account_id, data).map_err(|e| format!("服务器错误：{e}"))?;
    }
    Ok(())
}
```

- [ ] **Step 5: auth.rs 测试模块**

`server/src/auth.rs` 文件末尾追加（argon2 默认参数在 debug 下每次哈希约百毫秒级，本模块约 9 次哈希，测试多跑几秒属正常）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Db {
        Db::open_in_memory()
    }

    #[test]
    fn register_then_login_and_resume() {
        let db = mem_db();
        let r = register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap();
        // 账号保留原始大小写、昵称默认 = 账号名
        assert_eq!(r.nickname, "Alice");
        assert_eq!(r.auth_token.len(), 32);
        assert!(!r.has_avatar);

        let l = login(&db, "Alice", "pw123456").unwrap();
        assert_eq!(l.account_id, r.account_id);
        assert_ne!(l.auth_token, r.auth_token, "每次登录发新 token");
        // 大小写敏感：小写形式是另一个账号，查不到
        assert_eq!(login(&db, "alice", "pw123456").unwrap_err(), "账号或密码错误");

        let s = resume(&db, &l.auth_token).unwrap();
        assert_eq!(s.account_id, r.account_id);
        assert_eq!(s.nickname, "Alice");
        assert_eq!(s.auth_token, l.auth_token);
    }

    #[test]
    fn register_error_cases() {
        let db = mem_db();
        assert_eq!(register(&db, None, "alice", "pw123456", "x").unwrap_err(), "未开放注册");
        assert_eq!(
            register(&db, Some("code-1"), "alice", "pw123456", "wrong").unwrap_err(),
            "邀请码错误"
        );
        assert_eq!(
            register(&db, Some("code-1"), "ab", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "alice中文", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "_alice", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "alice", "12345", "code-1").unwrap_err(),
            "密码至少 6 位"
        );
        // 大小写敏感：Alice 与 alice 是两个独立账号，可各自注册
        register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap();
        register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        // 完全一致（含大小写）才报已存在
        assert_eq!(
            register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap_err(),
            "账号已存在"
        );
    }

    #[test]
    fn login_unified_error() {
        let db = mem_db();
        register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        assert_eq!(login(&db, "alice", "wrong-pw").unwrap_err(), "账号或密码错误");
        assert_eq!(login(&db, "nobody", "pw123456").unwrap_err(), "账号或密码错误");
        assert_eq!(resume(&db, "不存在的token").unwrap_err(), "登录已过期");
    }

    #[test]
    fn apply_profile_validates_and_updates() {
        let db = mem_db();
        let r = register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        let img = vec![9u8; 1024];

        apply_profile(&db, r.account_id, "阿信", Some(&img)).unwrap();
        let acc = db.find_account("alice").unwrap();
        assert_eq!(acc.nickname, "阿信");
        assert!(acc.has_avatar);
        assert_eq!(db.get_avatar(r.account_id).unwrap(), img);

        // 只改昵称：avatar = None 不动头像
        apply_profile(&db, r.account_id, "信哥", None).unwrap();
        assert_eq!(db.find_account("alice").unwrap().nickname, "信哥");
        assert_eq!(db.get_avatar(r.account_id).unwrap(), img);

        assert_eq!(apply_profile(&db, r.account_id, "", None).unwrap_err(), "昵称不合法（1-24 字符）");
        assert_eq!(
            apply_profile(&db, r.account_id, &"名".repeat(25), None).unwrap_err(),
            "昵称不合法（1-24 字符）"
        );
        let big = vec![0u8; AVATAR_MAX + 1];
        assert_eq!(
            apply_profile(&db, r.account_id, "阿信", Some(&big)).unwrap_err(),
            "头像过大（上限 64KB）"
        );
    }

    #[test]
    fn password_hash_roundtrip() {
        let phc = hash_password("pw123456").unwrap();
        assert!(phc.starts_with("$argon2"));
        assert!(verify_password("pw123456", &phc));
        assert!(!verify_password("wrong", &phc));
        assert!(!verify_password("pw123456", "不是PHC字符串"));
    }
}
```

- [ ] **Step 6: room.rs——成员账号字段、五元组与真随机 UDP token**

`server/src/room.rs` 按以下 7 处修改；测试模块整体替换。

修改 1（import）：
```rust
// 原文
use echoroom_protocol::messages::TcpMessage;
// 替换为
use echoroom_protocol::messages::{MemberInfo, TcpMessage};
```

修改 2（JoinOk.members 类型）：
```rust
// 原文
    /// 加入前已在房间的成员：(uid, 昵称, 是否静音, 流位图)
    pub members: Vec<(u16, String, bool, u8)>,
// 替换为
    /// 加入前已在房间的成员（五元组：含 has_avatar）
    pub members: Vec<MemberInfo>,
```

修改 3（Member 加字段）：
```rust
// 原文
pub struct Member {
    pub uid: u16,
    pub nickname: String,
    pub token: u32,
// 替换为
pub struct Member {
    pub uid: u16,
    /// 持久账号 id（AvatarRequest 查库用）
    pub account_id: i64,
    pub nickname: String,
    pub token: u32,
    /// 是否已上传头像（客户端懒加载标记）
    pub has_avatar: bool,
```

修改 4（Room 结构体与 new/next_token——删 LCG，UDP token 换真随机）：
```rust
// 原文
pub struct Room {
    members: HashMap<u16, Member>,
    next_uid: u16,
    /// 简单确定性伪随机（学习用途：LCG；不引入 rand 依赖）
    rng_state: u64,
    /// 订阅表：订阅者 uid → 目标 uid（一人最多订阅一人，覆盖式）
    subscriptions: HashMap<u16, u16>,
}

impl Room {
    pub fn new() -> Room {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Room { members: HashMap::new(), next_uid: 1, rng_state: seed | 1, subscriptions: HashMap::new() }
    }

    fn next_token(&mut self) -> u32 {
        // LCG（Numerical Recipes 常数）
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.rng_state >> 32) as u32
    }
// 替换为
pub struct Room {
    members: HashMap<u16, Member>,
    next_uid: u16,
    /// 订阅表：订阅者 uid → 目标 uid（一人最多订阅一人，覆盖式）
    subscriptions: HashMap<u16, u16>,
}

impl Room {
    pub fn new() -> Room {
        Room { members: HashMap::new(), next_uid: 1, subscriptions: HashMap::new() }
    }
```

修改 5（join 签名与实现）：
```rust
// 原文
    pub fn join(&mut self, nickname: String, tx: Sender<Vec<u8>>) -> Result<JoinOk, JoinErr> {
        if self.members.len() >= ROOM_CAPACITY {
            return Err(JoinErr::Full);
        }
        let members: Vec<(u16, String, bool, u8)> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), m.muted, m.streams))
            .collect();
        let uid = self.next_uid;
        self.next_uid = self.next_uid.wrapping_add(1).max(1);
        let token = self.next_token();
        self.members.insert(
            uid,
            Member {
                uid,
                nickname,
                token,
                muted: false,
                streams: 0,
                tx,
                udp_addr: None,
                last_seen: Instant::now(),
            },
        );
        Ok(JoinOk { uid, token, members })
    }
// 替换为
    pub fn join(
        &mut self,
        nickname: String,
        account_id: i64,
        has_avatar: bool,
        tx: Sender<Vec<u8>>,
    ) -> Result<JoinOk, JoinErr> {
        if self.members.len() >= ROOM_CAPACITY {
            return Err(JoinErr::Full);
        }
        let members: Vec<MemberInfo> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), m.muted, m.streams, m.has_avatar))
            .collect();
        let uid = self.next_uid;
        self.next_uid = self.next_uid.wrapping_add(1).max(1);
        // UDP 会话 token：真随机（每连接一次性凭证）
        let token = rand::RngCore::next_u32(&mut rand::rngs::OsRng);
        self.members.insert(
            uid,
            Member {
                uid,
                account_id,
                nickname,
                token,
                has_avatar,
                muted: false,
                streams: 0,
                tx,
                udp_addr: None,
                last_seen: Instant::now(),
            },
        );
        Ok(JoinOk { uid, token, members })
    }
```

修改 6（新增 3 个方法，紧跟 `set_muted` 之后插入）：
```rust
    /// 更新成员昵称（SetProfile 成功后同步在线状态）；成员不存在 → false
    pub fn update_nickname(&mut self, uid: u16, nickname: &str) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.nickname = nickname.to_string();
            true
        } else {
            false
        }
    }

    /// 更新头像标记；成员不存在 → false
    pub fn set_has_avatar(&mut self, uid: u16, has: bool) -> bool {
        if let Some(m) = self.members.get_mut(&uid) {
            m.has_avatar = has;
            true
        } else {
            false
        }
    }

    /// 成员对应的账号 id（AvatarRequest 查库用）
    pub fn account_id_of(&self, uid: u16) -> Option<i64> {
        self.members.get(&uid).map(|m| m.account_id)
    }
```

修改 7（测试模块整体替换——所有 join 调用加 `account_id`/`has_avatar` 参数、四元组断言改五元组、`LoginReject` 改 `AuthReject`、新增 1 个测试）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use echoroom_protocol::messages::{TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
    use std::sync::mpsc;

    #[test]
    fn join_until_capacity_then_reject() {
        let mut room = Room::new();
        let mut rxs = Vec::new();
        for i in 0..ROOM_CAPACITY {
            let (tx, rx) = mpsc::channel();
            let ok = room.join(format!("user{i}"), i as i64 + 1, false, tx);
            assert!(ok.is_ok());
            rxs.push(rx);
        }
        let (tx, _rx) = mpsc::channel();
        assert_eq!(room.join("第7人".into(), 7, false, tx), Err(JoinErr::Full));
    }

    #[test]
    fn join_returns_existing_members_and_leaves_removes() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let ok1 = room.join("A".into(), 1, false, tx1).unwrap();
        assert!(ok1.members.is_empty());
        let (tx2, _r2) = mpsc::channel();
        let ok2 = room.join("B".into(), 2, true, tx2).unwrap();
        assert_eq!(ok2.members, vec![(ok1.uid, "A".to_string(), false, 0, false)]);
        assert!(room.leave(ok1.uid));
        assert!(!room.leave(ok1.uid)); // 再删为 false
    }

    #[test]
    fn broadcast_skips_except_and_delivers() {
        let mut room = Room::new();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        room.broadcast(Some(a.uid), &TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into(), has_avatar: false });
        assert!(rx1.try_recv().is_err(), "A 不应收到（被排除）");
        let bytes = rx2.try_recv().expect("B 应收到");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::MemberJoin { uid: b.uid, nickname: "B".into(), has_avatar: false });
    }

    #[test]
    fn udp_addr_mapping_and_expiry() {
        use std::time::Duration;
        let mut room = Room::new();
        let (tx, _rx) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx).unwrap();
        let addr: std::net::SocketAddr = "127.0.0.1:5555".parse().unwrap();
        assert!(!room.validate_token(a.uid, 999));
        assert!(room.validate_token(a.uid, a.token));
        room.set_udp(a.uid, addr);
        assert_eq!(room.member_by_addr(addr), Some(a.uid));
        assert_eq!(room.others_with_udp(a.uid).len(), 0);
        // 超时 0：立即过期
        let expired = room.clear_udp_expired(Duration::ZERO);
        assert_eq!(expired, vec![a.uid]);
        assert_eq!(room.member_by_addr(addr), None);
    }

    #[test]
    fn send_to_delivers_only_target() {
        let mut room = Room::new();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        let _b = room.join("B".into(), 2, false, tx2).unwrap();
        room.send_to(a.uid, &TcpMessage::AuthReject { reason: "测试".into() });
        let bytes = rx1.try_recv().expect("A 应收到定向消息");
        let (msg, _) = echoroom_protocol::tcp::try_decode(&bytes).unwrap().unwrap();
        assert_eq!(msg, TcpMessage::AuthReject { reason: "测试".into() });
        assert!(rx2.try_recv().is_err(), "B 不应收到定向消息");
    }

    #[test]
    fn set_muted_updates_member_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        assert!(room.set_muted(a.uid, true));
        assert!(!room.set_muted(999, true)); // 不存在的成员
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        // 后加入者应看到 A 处于静音
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), true, 0, false)]);
    }

    #[test]
    fn stream_bitmap_set_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), 1, false, tx1).unwrap();
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, true), Some(1));
        assert_eq!(room.set_stream(a.uid, STREAM_CAMERA, true), Some(3));
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, false), Some(2));
        assert_eq!(room.set_stream(999, STREAM_SCREEN, true), None, "成员不存在");
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), false, 2, false)]);
    }

    #[test]
    fn subscribe_unsubscribe_and_viewers() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), 1, false, txa).unwrap();
        let b = room.join("B".into(), 2, false, txb).unwrap();
        let addr_b: std::net::SocketAddr = "127.0.0.1:6001".parse().unwrap();
        room.set_udp(b.uid, addr_b);
        assert!(!room.subscribe(b.uid, b.uid), "不能订阅自己");
        assert!(!room.subscribe(b.uid, 999), "目标不存在");
        assert!(room.subscribe(b.uid, a.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid]);
        assert_eq!(room.subscription_of(b.uid), Some(a.uid));
        assert_eq!(room.subscribers_with_udp(a.uid), vec![addr_b]);
        // 覆盖式：C 订阅 A 后再改订阅 B
        let (txc, _rc) = mpsc::channel();
        let c = room.join("C".into(), 3, false, txc).unwrap();
        assert!(room.subscribe(c.uid, a.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid, c.uid], "两人在看，按 uid 升序");
        assert!(room.subscribe(c.uid, b.uid));
        assert_eq!(room.viewers_of(a.uid), vec![b.uid]);
        assert_eq!(room.viewers_of(b.uid), vec![c.uid]);
        assert!(room.unsubscribe(c.uid));
        assert!(!room.unsubscribe(c.uid), "重复退订为 false");
        assert!(room.viewers_of(b.uid).is_empty());
    }

    #[test]
    fn leave_cleans_subscriptions_both_ways() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), 1, false, txa).unwrap();
        let b = room.join("B".into(), 2, false, txb).unwrap();
        room.subscribe(b.uid, a.uid); // B 看 A
        room.subscribe(a.uid, b.uid); // A 看 B（互看）
        room.leave(b.uid);
        assert!(room.viewers_of(a.uid).is_empty(), "B 离开后 A 的观众列表清空");
        assert_eq!(room.subscription_of(a.uid), None, "A 看 B 的订阅记录被清");
        assert!(!room.unsubscribe(a.uid));
    }

    #[test]
    fn update_nickname_avatar_flag_and_account_id() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("old".into(), 42, true, tx1).unwrap();
        assert_eq!(room.account_id_of(a.uid), Some(42));
        assert_eq!(room.account_id_of(999), None);
        assert!(room.update_nickname(a.uid, "新名字"));
        assert!(!room.update_nickname(999, "x")); // 不存在的成员
        assert!(room.set_has_avatar(a.uid, false));
        assert!(!room.set_has_avatar(999, true));
        // 后加入者看到的五元组应为更新后的昵称与头像标记
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), 2, false, tx2).unwrap();
        assert_eq!(b.members, vec![(a.uid, "新名字".to_string(), false, 0, false)]);
    }
}
```

- [ ] **Step 7: 服务器 tcp.rs——三路认证与资料消息处理**

`server/src/tcp.rs` 按以下 5 处修改。

修改 1（import 加 auth/db）：
```rust
// 原文
use crate::room::Room;
// 替换为
use crate::auth;
use crate::db::Db;
use crate::room::Room;
```

修改 2（serve 签名与线程参数）：
```rust
// 原文
pub fn serve(listener: TcpListener, room: Arc<Mutex<Room>>) -> std::io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let room = room.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s, room) {
                        eprintln!("[tcp] 连接结束: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[tcp] accept 错误: {e}"),
        }
    }
    Ok(())
}
// 替换为
pub fn serve(
    listener: TcpListener,
    room: Arc<Mutex<Room>>,
    db: Arc<Db>,
    invite: Option<String>,
) -> std::io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let room = room.clone();
                let db = db.clone();
                let invite = invite.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s, room, db, invite) {
                        eprintln!("[tcp] 连接结束: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[tcp] accept 错误: {e}"),
        }
    }
    Ok(())
}
```

修改 3（handle_conn 头部：认证 + join）：
```rust
// 原文
fn handle_conn(mut stream: TcpStream, room: Arc<Mutex<Room>>) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    // ---- 登录（第一条消息必须是 Login）----
    let (nickname, buf_rest) = read_first_login(&mut stream)?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let join = room.lock().unwrap().join(nickname.clone(), tx);
    let ok = match join {
        Ok(ok) => ok,
        Err(_) => {
            let reject = tcp::encode(&TcpMessage::LoginReject { reason: "房间已满（6人）".into() });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    println!("[tcp] {nickname}(uid={}) 加入 {peer}", ok.uid);
// 替换为
fn handle_conn(
    mut stream: TcpStream,
    room: Arc<Mutex<Room>>,
    db: Arc<Db>,
    invite: Option<String>,
) -> std::io::Result<()> {
    let peer = stream.peer_addr()?;
    // ---- 认证（第一条消息必须是 Register / Login / Resume 之一）----
    let (first, buf_rest) = read_first_auth(&mut stream, &db, invite.as_deref())?;
    let auth_result = match first {
        FirstAuth::Ok(r) => r,
        FirstAuth::Reject(reason) => {
            let reject = tcp::encode(&TcpMessage::AuthReject { reason });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let join = room.lock().unwrap().join(
        auth_result.nickname.clone(),
        auth_result.account_id,
        auth_result.has_avatar,
        tx,
    );
    let ok = match join {
        Ok(ok) => ok,
        Err(_) => {
            let reject = tcp::encode(&TcpMessage::AuthReject { reason: "房间已满（6人）".into() });
            stream.write_all(&reject)?;
            return Ok(());
        }
    };
    println!("[tcp] {}(uid={}) 加入 {peer}", auth_result.nickname, ok.uid);
```

修改 4（LeaveGuard 构造 + LoginOk 组装，members 含自己）：
```rust
// 原文
    let guard = LeaveGuard { room: room.clone(), uid: ok.uid, nickname: nickname.clone() };
    // LoginOk（定向）+ MemberJoin（广播给其他人）
    {
        let login_ok = TcpMessage::LoginOk { uid: ok.uid, token: ok.token, members: ok.members.clone() };
        stream.write_all(&tcp::encode(&login_ok))?;
        let room = room.lock().unwrap();
        room.broadcast(Some(ok.uid), &TcpMessage::MemberJoin { uid: ok.uid, nickname: nickname.clone() });
    }
// 替换为
    let guard = LeaveGuard { room: room.clone(), uid: ok.uid, nickname: auth_result.nickname.clone() };
    // LoginOk（定向）+ MemberJoin（广播给其他人）
    {
        // members 含自己：客户端需要自己初始的静音/流位图/头像标记
        let mut members = ok.members.clone();
        members.push((ok.uid, auth_result.nickname.clone(), false, 0, auth_result.has_avatar));
        let login_ok = TcpMessage::LoginOk {
            uid: ok.uid,
            udp_token: ok.token,
            auth_token: auth_result.auth_token.clone(),
            members,
        };
        stream.write_all(&tcp::encode(&login_ok))?;
        let room = room.lock().unwrap();
        room.broadcast(
            Some(ok.uid),
            &TcpMessage::MemberJoin {
                uid: ok.uid,
                nickname: auth_result.nickname.clone(),
                has_avatar: auth_result.has_avatar,
            },
        );
    }
```

修改 5（读循环新增 SetProfile / AvatarRequest 两个分支，插在 `RequestKeyframe` 分支之后）：
```rust
// 原文
                        TcpMessage::RequestKeyframe { target, .. } => {
                            let room = room.lock().unwrap();
                            // 转发给目标（uid 替换为请求者，供流主识别）
                            room.send_to(target, &TcpMessage::RequestKeyframe { uid: ok.uid, target });
                        }
                        _ => {}
// 替换为
                        TcpMessage::RequestKeyframe { target, .. } => {
                            let room = room.lock().unwrap();
                            // 转发给目标（uid 替换为请求者，供流主识别）
                            room.send_to(target, &TcpMessage::RequestKeyframe { uid: ok.uid, target });
                        }
                        TcpMessage::SetProfile { nickname, avatar } => {
                            match auth::apply_profile(&db, auth_result.account_id, &nickname, avatar.as_deref()) {
                                Ok(()) => {
                                    let mut room = room.lock().unwrap();
                                    room.update_nickname(ok.uid, &nickname);
                                    if avatar.is_some() {
                                        room.set_has_avatar(ok.uid, true);
                                    }
                                    room.broadcast(None, &TcpMessage::ProfileChanged { uid: ok.uid, nickname });
                                }
                                Err(reason) => {
                                    // 已进房：AuthReject 语义为"资料更新失败"，仅定向提示不断开
                                    let room = room.lock().unwrap();
                                    room.send_to(ok.uid, &TcpMessage::AuthReject { reason });
                                }
                            }
                        }
                        TcpMessage::AvatarRequest { uid: target } => {
                            // 两段取锁：先短锁房间拿 account_id，再查库（避免跨锁嵌套）
                            let account_id = room.lock().unwrap().account_id_of(target);
                            if let Some(account_id) = account_id {
                                let data = db.get_avatar(account_id).unwrap_or_default();
                                let room = room.lock().unwrap();
                                room.send_to(ok.uid, &TcpMessage::AvatarData { uid: target, data });
                            }
                        }
                        _ => {}
```

修改 6（`read_first_login` → `read_first_auth` + `FirstAuth` 枚举，整个函数替换）：
```rust
// 原文（整个函数，从注释行到闭合大括号）
/// 读缓冲直到解出第一条 Login 消息；返回 (昵称, 剩余未消费字节)
fn read_first_login(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match tcp::try_decode(&buf) {
            Ok(Some((TcpMessage::Login { nickname }, n))) => {
                buf.drain(..n);
                return Ok((nickname, buf));
            }
            Ok(Some(_)) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "首条消息必须是 Login"));
            }
            Ok(None) => {
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "连接在登录前关闭"));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(e) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")));
            }
        }
    }
}
// 替换为
/// 首条消息认证结果
enum FirstAuth {
    Ok(auth::AuthResult),
    Reject(String),
}

/// 读缓冲直到解出第一条 Register/Login/Resume 并完成认证校验；
/// 返回 (认证结果, 剩余未消费字节)
fn read_first_auth(
    stream: &mut TcpStream,
    db: &Db,
    invite: Option<&str>,
) -> std::io::Result<(FirstAuth, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match tcp::try_decode(&buf) {
            Ok(Some((msg, n))) => {
                buf.drain(..n);
                let result = match msg {
                    TcpMessage::Register { account, password, invite: user_invite } => {
                        auth::register(db, invite, &account, &password, &user_invite)
                    }
                    TcpMessage::Login { account, password } => auth::login(db, &account, &password),
                    TcpMessage::Resume { auth_token } => auth::resume(db, &auth_token),
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "首条消息必须是 Register/Login/Resume",
                        ));
                    }
                };
                let first = match result {
                    Ok(r) => FirstAuth::Ok(r),
                    Err(reason) => FirstAuth::Reject(reason),
                };
                return Ok((first, buf));
            }
            Ok(None) => {
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "连接在认证前关闭"));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(e) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")));
            }
        }
    }
}
```

（`LeaveGuard`、`notify_viewers` 保持原样不动。）

- [ ] **Step 8: main.rs——参数解析与 DB 初始化**

`server/src/main.rs` 全文件替换为：

```rust
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
```

- [ ] **Step 9: sim_clients.rs——认证适配（带邀请码注册，已存在回退登录）**

`server/src/bin/sim_clients.rs` 按以下 4 处修改。

修改 1（文件头注释与 main 参数）：
```rust
// 原文
//! 模拟客户端：登录 + 定时公屏 + UDP 语音模拟 + 收包统计。
// 替换为
//! 模拟客户端：注册/登录 + 定时公屏 + UDP 语音模拟 + 收包统计。
//! 用法：sim_clients [addr] [n] [seconds] [invite]
//! 带 invite：先尝试注册（同名已存在则自动回退登录）；不带：直接登录
//! （要求账号已注册过，否则认证被拒后退出）。
```

main 中参数与线程：
```rust
// 原文
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    let mut handles = Vec::new();
    for i in 0..n {
        let addr = addr.clone();
        handles.push(std::thread::spawn(move || sim_one(&addr, i, seconds)));
    }
// 替换为
    let addr = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:9000".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let invite = args.get(4).cloned();

    let mut handles = Vec::new();
    for i in 0..n {
        let addr = addr.clone();
        let invite = invite.clone();
        handles.push(std::thread::spawn(move || sim_one(&addr, i, seconds, invite)));
    }
```

修改 2（sim_one 头部：首消息改 Register/Login）：
```rust
// 原文
fn sim_one(addr: &str, index: usize, seconds: u64) {
    let name = format!("sim{index}");
    let mut stream = TcpStream::connect(addr).expect("连接失败");
    stream.write_all(&tcp::encode(&TcpMessage::Login { nickname: name.clone() })).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap(); // 循环节拍：20ms

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
// 替换为
fn sim_one(addr: &str, index: usize, seconds: u64, invite: Option<String>) {
    let name = format!("sim{index}");
    let account = name.clone(); // 账号需符合 [A-Za-z0-9][A-Za-z0-9_]{2,19}，sim0/sim1… 合法
    let password = format!("pw-{index}-123456");
    let mut stream = TcpStream::connect(addr).expect("连接失败");
    let first = match &invite {
        Some(code) => TcpMessage::Register {
            account: account.clone(),
            password: password.clone(),
            invite: code.clone(),
        },
        None => TcpMessage::Login { account: account.clone(), password: password.clone() },
    };
    stream.write_all(&tcp::encode(&first)).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap(); // 循环节拍：20ms

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut fallback_done = false; // 注册撞名后是否已回退登录（防死循环）
```

修改 3（循环加 'main 标签）：
```rust
// 原文
    let start = Instant::now();
    let mut last_chat = Instant::now();
    let mut last_voice = Instant::now();
    let mut last_heartbeat = Instant::now();

    loop {
// 替换为
    let start = Instant::now();
    let mut last_chat = Instant::now();
    let mut last_voice = Instant::now();
    let mut last_heartbeat = Instant::now();

    'main: loop {
```

修改 4（LoginOk 字段重命名 + AuthReject 回退逻辑 + MemberJoin 解构）：
```rust
// 原文
                match msg {
                    TcpMessage::LoginOk { uid, token, .. } => {
                        my_uid = uid;
                        println!("[{name}] LoginOk uid={uid}");
                        let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
                        sock.connect(addr).unwrap();
                        sock.set_nonblocking(true).unwrap();
                        sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token })).unwrap();
                        udp_sock = Some(sock);
                    }
                    TcpMessage::Chat { uid, text } => println!("[{name}] 收到 uid={uid}: {text}"),
                    TcpMessage::MemberJoin { uid, nickname } => println!("[{name}] +{nickname}(uid={uid})"),
                    TcpMessage::MemberLeave { uid } => println!("[{name}] -uid={uid}"),
                    other => println!("[{name}] {other:?}"),
                }
// 替换为
                match msg {
                    TcpMessage::LoginOk { uid, udp_token, .. } => {
                        my_uid = uid;
                        println!("[{name}] LoginOk uid={uid}");
                        let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
                        sock.connect(addr).unwrap();
                        sock.set_nonblocking(true).unwrap();
                        sock.send(&udp::encode(uid, 0, &UdpPacket::Register { token: udp_token })).unwrap();
                        udp_sock = Some(sock);
                    }
                    TcpMessage::AuthReject { reason } => {
                        if invite.is_some() && reason == "账号已存在" && !fallback_done {
                            fallback_done = true;
                            println!("[{name}] 账号已存在，回退登录");
                            let login = TcpMessage::Login { account: account.clone(), password: password.clone() };
                            stream.write_all(&tcp::encode(&login)).unwrap();
                        } else {
                            eprintln!("[{name}] 认证失败：{reason}");
                            break 'main;
                        }
                    }
                    TcpMessage::Chat { uid, text } => println!("[{name}] 收到 uid={uid}: {text}"),
                    TcpMessage::MemberJoin { uid, nickname, .. } => println!("[{name}] +{nickname}(uid={uid})"),
                    TcpMessage::MemberLeave { uid } => println!("[{name}] -uid={uid}"),
                    other => println!("[{name}] {other:?}"),
                }
```

- [ ] **Step 10: 运行服务器包测试**

Run: `cargo test -p echoroom-server`
Expected: 全绿——db 5 个 + auth 5 个 + room 10 个（原 9 个更新 + 新增 `update_nickname_avatar_flag_and_account_id`）；argon2 相关 9 次哈希在 debug 下多跑几秒属正常。

```markdown
至此 server 恢复可测；client 仍断裂（Task 3 修复），`cargo test`（全 workspace）此时仍不可用。
```

---

### Task 3: 客户端 Rust——AuthMode 三态、token 持久化与新事件

**Files:**
- Modify: `client/src-tauri/src/config.rs`
- Modify: `client/src-tauri/src/net/tcp.rs`（全文件替换）
- Modify: `client/src-tauri/src/bridge.rs`
- Modify: `client/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes:
  - Task 1 协议类型：`LoginOk { uid, udp_token, auth_token, members }`（五元组、含自己）、`AuthReject`、`Register/Resume/SetProfile/ProfileChanged/AvatarRequest/AvatarData`
  - Task 2 服务器行为：未进房时 `AuthReject` = 认证失败（房间满同此）；已进房时定向 `AuthReject` = 资料更新失败（客户端用 `authenticated` 标志区分双语义）
- Produces（Task 4/5 前端依赖）：
  - invoke 命令：`auth_login(serverAddr, account, password)`、`auth_register(serverAddr, account, password, invite)`、`auto_connect()`、`set_profile(nickname, avatar)`（avatar = `number[] | null`，null 不改头像）、`avatar_request(uid)`；旧命令 `set_config`/`connect` 删除
  - 事件：`auth_ok`（无 payload：认证成功）、`auth_fail`（reason 字符串：已停止重连交还 UI）、`profile_error`（reason 字符串）、`profile_changed`（`{uid, nickname}`）、`avatar_data`（`{uid, data: number[]}`，空数组 = 无头像）；`members` 变五元组、`member_join` 变 `(uid, nickname, has_avatar)` 三元组
  - config.json 新字段：`account`、`auth_token`（旧 `nickname` 字段被 serde 忽略，兼容旧配置文件）

- [x] **Step 1: config.rs——账号与自动登录凭证字段**

`client/src-tauri/src/config.rs` 按以下 4 处修改。

修改 1（文件头注释）：
```rust
// 原文
//! 客户端配置：昵称、服务器地址与音量设置，持久化到 %APPDATA%\com.echoroom.dev\config.json。
// 替换为
//! 客户端配置：服务器地址、账号与自动登录凭证、音量设置，持久化到 %APPDATA%\com.echoroom.dev\config.json。
```

修改 2（结构体字段：删 nickname，加 account/auth_token）：
```rust
// 原文
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub nickname: String,
    pub server_addr: String,
// 替换为
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub server_addr: String,
    /// 登录账号（认证命令发起时保存；表单预填与自动登录显示用）
    #[serde(default)]
    pub account: String,
    /// 自动登录凭证（服务器签发；Resume 失效时被清除）
    #[serde(default)]
    pub auth_token: String,
```

修改 3（Default）：
```rust
// 原文
        Config {
            nickname: String::new(),
            server_addr: "127.0.0.1:9000".into(),
// 替换为
        Config {
            server_addr: "127.0.0.1:9000".into(),
            account: String::new(),
            auth_token: String::new(),
```

修改 4（测试模块整体替换——roundtrip 加账号字段断言、旧配置测试改断言 account/auth_token）：
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_volume_fields() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_roundtrip.json");
        let mut cfg = Config::default();
        cfg.account = "alice".into();
        cfg.auth_token = "0123456789abcdef0123456789abcdef".into();
        cfg.self_gain = 1.5;
        cfg.muted = true;
        cfg.peer_gains.insert("小林".into(), 0.5);
        cfg.screen_gain = 0.8;
        cfg.share_quality = "1080p15".into();
        cfg.share_audio = false;
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_config_without_volume_fields_loads_defaults() {
        let path = std::env::temp_dir().join("echoroom_cfg_test_old.json");
        // 旧版配置含 nickname 字段：serde 忽略未知字段，账号相关字段取默认空值
        std::fs::write(&path, r#"{"nickname":"老用户","server_addr":"127.0.0.1:9000"}"#).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.server_addr, "127.0.0.1:9000");
        assert!(loaded.account.is_empty());
        assert!(loaded.auth_token.is_empty());
        assert_eq!(loaded.self_gain, 1.0);
        assert!(!loaded.muted);
        assert!(loaded.peer_gains.is_empty());
        assert_eq!(loaded.screen_gain, 1.0);
        assert_eq!(loaded.share_quality, "720p30");
        assert!(loaded.share_audio);
        let _ = std::fs::remove_file(&path);
    }
}
```

- [x] **Step 2: net/tcp.rs——AuthMode 三态与全量重写**

`client/src-tauri/src/net/tcp.rs` 全文件替换为：

```rust
//! TCP 控制客户端：注册/登录/自动登录、公屏、成员与说话状态事件；断线自动重连（退避）。
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use echoroom_protocol::messages::{MemberInfo, TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
use echoroom_protocol::tcp;

use crate::bridge::{Bridge, ConnState};

/// 认证方式（连接首消息三选一；登录成功后的断线重连自动走 Resume）
#[derive(Clone, Debug)]
pub enum AuthMode {
    Login { account: String, password: String },
    Register { account: String, password: String, invite: String },
    /// 自动登录（有 auth_token 时的默认方式）
    Resume { auth_token: String },
}

/// UI → 网络线程的命令
pub enum NetCmd {
    SendChat(String),
    SetSpeaking(bool),
    SetMuted(bool),
    /// 订阅某人（None = 取消订阅）
    Subscribe(Option<u16>),
    /// 请求目标发关键帧
    RequestKeyframe(u16),
    /// 上报本端某路流开/停（kind = STREAM_*）
    SetStream { kind: u8, on: bool },
    /// 更新资料（昵称必填；avatar = None 表示不改头像）
    SetProfile { nickname: String, avatar: Option<Vec<u8>> },
    /// 请求某人的头像（懒加载）
    AvatarRequest(u16),
    Shutdown,
}

/// 网络线程句柄：命令通道 + 登录身份（uid/token，语音通道后续使用）
pub struct NetHandle {
    pub tx: Sender<NetCmd>,
    pub my_uid: Arc<AtomicU16>,
    pub my_token: Arc<AtomicU32>,
}

/// 重连退避（秒）：1 → 2 → 5 → 10（封顶）
const BACKOFF_SECS: [u64; 4] = [1, 2, 5, 10];

pub fn spawn(
    addr: String,
    mode: AuthMode,
    bridge: Bridge,
    shared: crate::audio::session::SharedAudio,
) -> NetHandle {
    let (tx, rx) = std::sync::mpsc::channel::<NetCmd>();
    let my_uid = Arc::new(AtomicU16::new(0));
    let my_token = Arc::new(AtomicU32::new(0));
    let (uid_c, tok_c) = (my_uid.clone(), my_token.clone());
    let tx_for_loop = tx.clone(); // 采集线程 VAD 的 SetSpeaking 命令经会话线程写 TCP
    std::thread::spawn(move || {
        run_loop(addr, mode, bridge, rx, uid_c, tok_c, tx_for_loop, shared)
    });
    NetHandle { tx, my_uid, my_token }
}

/// 会话结束原因
enum SessionEnd {
    /// 连接断开（服务端关闭或读错误）：从最短退避重新开始
    Disconnected,
    /// 被服务器拒绝（如房间满）：退避递增，避免高频重试
    Rejected,
    /// 认证失败（账号密码错误 / Resume 过期）：停止重连，交还 UI
    AuthFailed(String),
    /// 收到 Shutdown 命令：线程退出
    Shutdown,
}

/// 本轮连接的首消息：有 auth_token 一律走 Resume（登录成功后的重连同路径）
fn first_message(mode: &AuthMode, token: Option<&str>) -> TcpMessage {
    match token {
        Some(t) => TcpMessage::Resume { auth_token: t.to_string() },
        None => match mode {
            AuthMode::Login { account, password } => {
                TcpMessage::Login { account: account.clone(), password: password.clone() }
            }
            AuthMode::Register { account, password, invite } => TcpMessage::Register {
                account: account.clone(),
                password: password.clone(),
                invite: invite.clone(),
            },
            AuthMode::Resume { auth_token } => TcpMessage::Resume { auth_token: auth_token.clone() },
        },
    }
}

fn run_loop(
    addr: String,
    mode: AuthMode,
    bridge: Bridge,
    rx: Receiver<NetCmd>,
    my_uid: Arc<AtomicU16>,
    my_token: Arc<AtomicU32>,
    tx: Sender<NetCmd>,
    shared: crate::audio::session::SharedAudio,
) {
    let mut attempt = 0usize;
    // 自动登录凭证：初始来自 Resume 模式；Login/Register 成功后由 LoginOk 滚动更新
    let mut auth_token: Option<String> = match &mode {
        AuthMode::Resume { auth_token } => Some(auth_token.clone()),
        _ => None,
    };
    loop {
        bridge.emit_conn(if attempt == 0 { ConnState::Connecting } else { ConnState::Reconnecting });
        let used_resume = auth_token.is_some();
        let first = first_message(&mode, auth_token.as_deref());
        let result = match TcpStream::connect(&addr) {
            Ok(mut stream) => run_session(
                &mut stream, &addr, &first, &mut auth_token, &bridge, &rx, &my_uid, &my_token, &tx,
                &shared,
            ),
            Err(e) => {
                eprintln!("[net] 连接失败: {e}");
                Err(e)
            }
        };
        match result {
            Ok(SessionEnd::Shutdown) => return,
            Ok(SessionEnd::Disconnected) => {
                attempt = 0;
                bridge.emit_conn(ConnState::Reconnecting);
            }
            Ok(SessionEnd::Rejected) => bridge.emit_conn(ConnState::Reconnecting),
            Ok(SessionEnd::AuthFailed(reason)) => {
                eprintln!("[net] 认证失败: {reason}");
                if used_resume {
                    // Resume 失效（凭证被清 / 服务器换库）：清 token 回登录页
                    auth_token = None;
                    bridge.clear_auth_token();
                }
                bridge.emit_conn(ConnState::Rejected(reason.clone()));
                bridge.emit_auth_fail(reason);
                return; // 不自动重试，交还 UI 决定下一步
            }
            Err(e) => {
                eprintln!("[net] 会话结束: {e}");
                bridge.emit_conn(ConnState::Reconnecting);
            }
        }
        let wait = BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)];
        attempt += 1;
        if wait_or_shutdown(&rx, Duration::from_secs(wait)) {
            return;
        }
    }
}

/// 等待 `total` 时长，期间以 100ms 粒度轮询 Shutdown。返回 true = 收到 Shutdown。
fn wait_or_shutdown(rx: &Receiver<NetCmd>, total: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < total {
        match rx.try_recv() {
            Ok(NetCmd::Shutdown) => return true,
            _ => {} // 断线期间的发送/说话命令直接丢弃
        }
        std::thread::sleep(step);
        waited += step;
    }
    false
}

fn run_session(
    stream: &mut TcpStream,
    addr: &str,
    first: &TcpMessage,
    auth_token: &mut Option<String>,
    bridge: &Bridge,
    rx: &Receiver<NetCmd>,
    my_uid: &AtomicU16,
    my_token: &AtomicU32,
    tx: &Sender<NetCmd>,
    shared: &crate::audio::session::SharedAudio,
) -> std::io::Result<SessionEnd> {
    stream.write_all(&tcp::encode(first))?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut authenticated = false; // AuthReject 双语义：未认证 = 断开；已认证 = 资料错误提示
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        // 发：轮询 UI 命令写出
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                NetCmd::SendChat(text) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Chat { uid: 0, text }))?;
                }
                NetCmd::SetSpeaking(on) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Speaking { uid: 0, on }))?;
                }
                NetCmd::SetMuted(on) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on }))?;
                }
                NetCmd::Subscribe(Some(target)) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Subscribe { uid: 0, target }))?;
                }
                NetCmd::Subscribe(None) => {
                    stream.write_all(&tcp::encode(&TcpMessage::Unsubscribe { uid: 0 }))?;
                }
                NetCmd::RequestKeyframe(target) => {
                    stream.write_all(&tcp::encode(&TcpMessage::RequestKeyframe { uid: 0, target }))?;
                }
                NetCmd::SetStream { kind, on } => {
                    stream.write_all(&tcp::encode(&TcpMessage::StreamState { uid: 0, kind, on }))?;
                }
                NetCmd::SetProfile { nickname, avatar } => {
                    stream.write_all(&tcp::encode(&TcpMessage::SetProfile { nickname, avatar }))?;
                }
                NetCmd::AvatarRequest(uid) => {
                    stream.write_all(&tcp::encode(&TcpMessage::AvatarRequest { uid }))?;
                }
                NetCmd::Shutdown => return Ok(SessionEnd::Shutdown),
            }
        }
        // 收：50ms 超时轮询（超时 = 回去处理命令）
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(SessionEnd::Disconnected), // 服务端关闭
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
        // 解析（一次可能出多条，或只攒到半条）
        loop {
            match tcp::try_decode(&buf) {
                Ok(Some((msg, used))) => {
                    buf.drain(..used);
                    match msg {
                        TcpMessage::LoginOk { uid, udp_token, auth_token: fresh_token, members } => {
                            authenticated = true;
                            my_uid.store(uid, Ordering::Relaxed);
                            my_token.store(udp_token, Ordering::Relaxed);
                            // 凭证滚动：Login/Register 签发新 token，Resume 换取新 token
                            *auth_token = Some(fresh_token.clone());
                            bridge.save_auth_token(fresh_token);
                            bridge.emit_conn(ConnState::Connected);
                            // 重建 uid → 昵称映射（members 已含自己；重连场景先清空）
                            {
                                let mut names = shared.uid_names.lock().unwrap();
                                names.clear();
                                for (u, n, _, _, _) in &members {
                                    names.insert(*u, n.clone());
                                }
                            }
                            bridge.emit_self_uid(uid);
                            // 自己的 muted 取本地当前值（重连后保持界面与实际一致）
                            let my_muted = shared.self_muted.load(Ordering::Relaxed);
                            let all: Vec<MemberInfo> = members
                                .into_iter()
                                .map(|(u, n, m, s, h)| (u, n, if u == uid { my_muted } else { m }, s, h))
                                .collect();
                            bridge.emit_member_list(all);
                            bridge.emit_auth_ok();
                            // 重连后若本地处于静音，向新会话重新声明（否则服务器端 muted=false，别人看不到）
                            if my_muted {
                                stream.write_all(&tcp::encode(&TcpMessage::Mute { uid: 0, on: true }))?;
                            }
                            // 重连：把本地仍在推的流重新声明给新会话（服务器端状态随旧连接清零）
                            {
                                let bm = shared.my_streams.load(Ordering::Relaxed);
                                for kind in [STREAM_SCREEN, STREAM_CAMERA] {
                                    if bm & (1 << kind) != 0 {
                                        stream.write_all(&tcp::encode(&TcpMessage::StreamState { uid: 0, kind, on: true }))?;
                                    }
                                }
                            }
                            // 观众数随新会话归零（服务器接线后会推回真实值）
                            shared.viewer_count.store(0, Ordering::Relaxed);
                            // 启动音频链路（麦克风/编码/播放/VAD 上报；失败不影响文字聊天）
                            crate::bridge::start_audio(&bridge.app, uid, udp_token, addr.to_string(), tx.clone());
                        }
                        TcpMessage::AuthReject { reason } => {
                            if authenticated {
                                // 已进房：资料更新失败等（不断开）
                                bridge.emit_profile_error(reason);
                            } else {
                                // 未进房：认证失败（含房间满）→ 停止重连
                                return Ok(SessionEnd::AuthFailed(reason));
                            }
                        }
                        TcpMessage::MemberJoin { uid, nickname, has_avatar } => {
                            shared.uid_names.lock().unwrap().insert(uid, nickname.clone());
                            bridge.emit_member_join(uid, nickname, has_avatar)
                        }
                        TcpMessage::MemberLeave { uid } => {
                            shared.uid_names.lock().unwrap().remove(&uid);
                            bridge.emit_member_leave(uid)
                        }
                        TcpMessage::Chat { uid, text } => bridge.emit_chat(uid, text),
                        TcpMessage::Speaking { uid, on } => bridge.emit_speaking(uid, on),
                        TcpMessage::Muted { uid, on } => bridge.emit_muted(uid, on),
                        TcpMessage::StreamState { uid, kind, on } => bridge.emit_stream_state(uid, kind, on),
                        TcpMessage::Viewers { uids } => {
                            shared.viewer_count.store(uids.len() as u16, Ordering::Relaxed);
                            bridge.emit_viewer_count(&uids);
                        }
                        TcpMessage::RequestKeyframe { .. } => bridge.emit_request_keyframe(),
                        TcpMessage::ProfileChanged { uid, nickname } => {
                            shared.uid_names.lock().unwrap().insert(uid, nickname.clone());
                            bridge.emit_profile_changed(uid, nickname)
                        }
                        TcpMessage::AvatarData { uid, data } => bridge.emit_avatar_data(uid, data),
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    eprintln!("[net] 协议解码错误: {e:?}");
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "协议解码错误"));
                }
            }
        }
    }
}
```

- [x] **Step 3: bridge.rs——认证命令与事件出口**

`client/src-tauri/src/bridge.rs` 按以下 6 处修改。

修改 1（Bridge 事件：五元组 / 三元组）：
```rust
// 原文
    pub fn emit_member_list(&self, members: Vec<(u16, String, bool, u8)>) {
        let _ = self.app.emit("members", members);
    }
    pub fn emit_member_join(&self, uid: u16, nickname: String) {
        let _ = self.app.emit("member_join", (uid, nickname));
    }
// 替换为
    pub fn emit_member_list(&self, members: Vec<echoroom_protocol::messages::MemberInfo>) {
        let _ = self.app.emit("members", members);
    }
    pub fn emit_member_join(&self, uid: u16, nickname: String, has_avatar: bool) {
        let _ = self.app.emit("member_join", (uid, nickname, has_avatar));
    }
```

修改 2（Bridge 新增事件与 token 持久化，紧跟 `emit_request_keyframe` 之后插入）：
```rust
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
```

修改 3（删 `set_config` 与 `connect`，替换为认证命令与公共部分）：
```rust
// 原文（两个命令函数整体）
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
// 替换为
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
```

修改 4（connect_with_app 改签名：从 config 读服务器地址）：
```rust
// 原文
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
// 替换为
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
```

修改 5（新增 set_profile / avatar_request 命令，紧跟 `send_chat` 之后插入）：
```rust
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
```

（`is_web_url` 及其测试、其余命令保持原样不动。）

- [x] **Step 4: lib.rs——命令注册更新**

`client/src-tauri/src/lib.rs` 的 invoke_handler 列表替换：
```rust
// 原文
        .invoke_handler(tauri::generate_handler![
            bridge::get_config,
            bridge::set_config,
            bridge::connect,
            bridge::send_chat,
// 替换为
        .invoke_handler(tauri::generate_handler![
            bridge::get_config,
            bridge::auth_login,
            bridge::auth_register,
            bridge::auto_connect,
            bridge::send_chat,
            bridge::set_profile,
            bridge::avatar_request,
```

- [x] **Step 5: 全 workspace 测试与编译验证（断裂窗口关闭）**

Run: `cargo test`
Expected: 全绿——protocol（Task 1 测例）+ server（db/auth/room）+ client（config 2 个 + bridge is_web_url 2 个）；至此 Task 1 引入的跨包断裂全部修复。

Run: `cargo build`
Expected: workspace 编译通过（client bin `Echo` 链接成功；`cargo check -p echoroom-client` 亦可通过）。

---

### Task 4: 前端——登录 / 注册面板与完善资料弹窗（昵称）

**Files:**
- Modify: `client/ui/index.html`（setup-mask 结构替换 + 新增 profile-mask）
- Modify: `client/ui/app.js`（认证面板、资料弹窗、事件监听、init 尾部）
- Modify: `client/ui/style.css`（错误行 / 链接按钮）

**Interfaces:**
- Consumes: Task 3 的命令 `auth_login(serverAddr, account, password)` / `auth_register(serverAddr, account, password, invite)` / `auto_connect()` / `set_profile(nickname, avatar)`；事件 `auth_ok` / `auth_fail` / `profile_changed` / `profile_error`；`members` 五元组 / `member_join` 三元组；config 的 `server_addr` / `account` / `auth_token`
- Produces（Task 5 复用/扩展）：`showAuthPanel(mode, error)`、`showAuthError(msg)`、`openProfilePop(defaultName)`、`closeProfilePop()`、`showProfileError(msg)`、`profileSubmitting` 标志、`members` 条目中的 `hasAvatar` 字段

- [ ] **Step 1: index.html——认证面板替换 + 资料弹窗新增**

`client/ui/index.html` 中 `setup-mask` 整块（原 L43-50）替换为：

```html
  <div class="setup-mask hidden" id="setup-mask">
    <div class="setup-panel">
      <h2 id="setup-title">登录 Echo</h2>
      <input id="setup-server" type="text" maxlength="64" value="127.0.0.1:9000" placeholder="服务器地址 host:port" />
      <input id="setup-account" type="text" maxlength="20" placeholder="账号（3-20 位字母数字下划线，不能以 _ 开头）" autocomplete="off" />
      <input id="setup-password" type="password" maxlength="64" placeholder="密码（至少 6 位）" autocomplete="off" />
      <input id="setup-invite" type="text" maxlength="64" placeholder="邀请码" hidden />
      <div class="setup-error" id="setup-error" hidden></div>
      <button id="setup-submit">登录</button>
      <button class="setup-link" id="setup-toggle" type="button">没有账号？注册</button>
    </div>
  </div>

  <div class="setup-mask hidden" id="profile-mask">
    <div class="setup-panel">
      <h2>完善资料</h2>
      <input id="profile-nickname" type="text" maxlength="24" placeholder="昵称（1-24 字符）" />
      <div class="setup-error" id="profile-error" hidden></div>
      <button id="profile-submit">保存</button>
      <button class="setup-link" id="profile-skip" type="button">以后再说</button>
    </div>
  </div>
```

- [ ] **Step 2: app.js——状态变量与认证 / 资料弹窗逻辑**

状态区（`let lastViewerUids = [];` 之后）加：
```js
// C：认证与资料
let authMode = "login"; // "login" | "register"
let lastAuthAttempt = null; // 最近一次提交的意图（auth_ok 判断是否弹资料窗）
let profileSubmitting = false; // 资料提交中：等 profile_changed 广播回来才关弹窗
```

"首次启动：昵称 / 服务器设置面板"整段（`async function submitSetup()` 到 `for (const id of ["setup-nickname", "setup-server"]) ...` 结束）替换为：
```js
// ---- C：登录 / 注册面板 ----
const setupMask = el("setup-mask");

function showAuthError(msg) {
  const node = el("setup-error");
  node.hidden = !msg;
  node.textContent = msg;
}

function showAuthPanel(mode, error) {
  authMode = mode;
  el("setup-title").textContent = mode === "register" ? "注册 Echo" : "登录 Echo";
  el("setup-submit").textContent = mode === "register" ? "注册" : "登录";
  el("setup-toggle").textContent = mode === "register" ? "已有账号？登录" : "没有账号？注册";
  el("setup-invite").hidden = mode !== "register";
  showAuthError(error || "");
  setupMask.classList.remove("hidden");
}

async function submitAuth() {
  const serverAddr = el("setup-server").value.trim();
  const account = el("setup-account").value.trim();
  const password = el("setup-password").value;
  const invite = el("setup-invite").value.trim();
  if (!serverAddr || !account || !password) {
    showAuthError("请填写服务器地址、账号与密码");
    return;
  }
  const btn = el("setup-submit");
  btn.disabled = true; // 防止重复提交；成功经 auth_ok 复位、失败经 auth_fail 复位
  showAuthError("");
  lastAuthAttempt = authMode;
  try {
    if (authMode === "register") {
      await invoke("auth_register", { serverAddr, account, password, invite });
    } else {
      await invoke("auth_login", { serverAddr, account, password });
    }
  } catch (e) {
    // 命令层失败（如配置写盘失败）：立即复位；服务器拒绝经 auth_fail 事件回来
    showAuthError(String(e));
    btn.disabled = false;
  }
}

el("setup-submit").addEventListener("click", submitAuth);
el("setup-toggle").addEventListener("click", () => {
  showAuthPanel(authMode === "register" ? "login" : "register");
});
for (const id of ["setup-server", "setup-account", "setup-password", "setup-invite"]) {
  el(id).addEventListener("keydown", (e) => {
    if (e.key === "Enter") submitAuth();
  });
}

// ---- C：完善资料弹窗（昵称；头像在 Task 5 接入） ----
function showProfileError(msg) {
  const node = el("profile-error");
  node.hidden = !msg;
  node.textContent = msg;
}

function openProfilePop(defaultName) {
  showProfileError("");
  el("profile-nickname").value = defaultName || "";
  el("profile-mask").classList.remove("hidden");
  el("profile-nickname").focus();
}

function closeProfilePop() {
  el("profile-mask").classList.add("hidden");
  profileSubmitting = false;
  el("profile-submit").disabled = false;
}

function submitProfile() {
  const nickname = el("profile-nickname").value.trim();
  if (!nickname) {
    showProfileError("请填写昵称（1-24 字符）");
    return;
  }
  profileSubmitting = true;
  el("profile-submit").disabled = true;
  showProfileError("");
  invoke("set_profile", { nickname, avatar: null }).catch((e) => {
    // 命令层失败（未连接等）：复位；服务器校验失败经 profile_error 事件回来
    profileSubmitting = false;
    el("profile-submit").disabled = false;
    showProfileError(String(e));
  });
}

el("profile-submit").addEventListener("click", submitProfile);
el("profile-skip").addEventListener("click", closeProfilePop);
el("profile-nickname").addEventListener("keydown", (e) => {
  if (e.key === "Enter") submitProfile();
});
```

- [ ] **Step 3: app.js——事件监听更新与 init 尾部**

`members` 监听改为五元组解构（`hasAvatar` 存进成员对象，Task 5 渲染用）：
```js
// 原文
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname, muted, streams] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted, streams });
    }
    renderMembers();
    renderOnline();
// 替换为
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname, muted, streams, hasAvatar] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted, streams, hasAvatar });
    }
    renderMembers();
    renderOnline();
```

`member_join` 监听改为三元组解构：
```js
// 原文
  await listen("member_join", (e) => {
    const [uid, nickname] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false, streams: 0 });
// 替换为
  await listen("member_join", (e) => {
    const [uid, nickname, hasAvatar] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false, streams: 0, hasAvatar });
```

`conn` 监听之后追加认证 / 资料监听器：
```js
  // C：认证结果
  await listen("auth_ok", () => {
    el("setup-mask").classList.add("hidden");
    el("setup-submit").disabled = false;
    el("setup-password").value = "";
    showAuthError("");
    const wasRegister = lastAuthAttempt === "register";
    lastAuthAttempt = null;
    if (wasRegister) openProfilePop(el("setup-account").value.trim()); // 注册成功 → 弹「完善资料」
  });
  await listen("auth_fail", (e) => {
    el("setup-submit").disabled = false;
    showAuthPanel(lastAuthAttempt === "register" ? "register" : "login", String(e.payload));
    lastAuthAttempt = null;
  });
  await listen("profile_changed", (e) => {
    const { uid, nickname } = e.payload;
    const m = members.get(uid);
    if (m) m.nickname = nickname;
    renderMembers();
    renderOnline();
    if (profileSubmitting && uid === myUid) closeProfilePop(); // 自己保存成功：广播回来才关窗
  });
  await listen("profile_error", (e) => {
    if (!profileSubmitting) return;
    profileSubmitting = false;
    el("profile-submit").disabled = false;
    showProfileError(String(e.payload));
  });
```

init 尾部（`if (!cfg.nickname) { ... } else { await invoke("connect"); }` 整段）替换为：
```js
  // C：表单预填 + 自动登录（有 token 走 Resume；否则显示登录面板）
  el("setup-server").value = cfg.server_addr || "127.0.0.1:9000";
  if (cfg.account) el("setup-account").value = cfg.account;
  if (cfg.auth_token) {
    invoke("auto_connect").catch((e) => {
      console.error("自动登录失败:", e);
      showAuthPanel("login");
    });
  } else {
    showAuthPanel("login");
  }
```

- [ ] **Step 4: style.css——错误行与链接按钮**

`client/ui/style.css` 在 `.setup-panel button { ... }` 规则之后追加：
```css
/* ---- C：登录/注册与资料弹窗 ---- */
.setup-error {
  color: var(--err);
  font-size: 12px;
  text-align: center;
}
.setup-panel .setup-link {
  background: none;
  border: none;
  color: var(--accent);
  font-size: 12px;
  padding: 2px 0;
  cursor: pointer;
}
.setup-panel .setup-link:hover { text-decoration: underline; }
```

- [ ] **Step 5: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

```markdown
验收要点（手工，可延后到 Task 6 合并做）：首次启动显示登录面板；无账号 → 切注册（邀请码框出现）；注册成功 → 面板关闭并弹「完善资料」→ 保存后自己卡片昵称变化；重启客户端自动登录（不再显示登录面板）。
```

---

### Task 5: 前端——头像上传、懒加载与渲染

**Files:**
- Modify: `client/ui/index.html`（profile-mask 加头像行）
- Modify: `client/ui/app.js`（缓存 / 压缩 / 预览 / 提交 / 渲染 / 事件）
- Modify: `client/ui/style.css`（头像行与头像图样式）

**Interfaces:**
- Consumes: Task 4 的 `openProfilePop` / `closeProfilePop` / `showProfileError` / `profileSubmitting`；Task 3 的 `avatar_request(uid)` 命令与 `avatar_data` 事件、`set_profile(nickname, avatar)` 的 avatar 参数；`members` 五元组 / `member_join` 三元组中的 `hasAvatar`
- Produces: `avatarCache` / `ensureAvatar(uid, hasAvatar)` / `forceRequestAvatar(uid)`；头像 DOM 类 `.member-avatar-img`

- [ ] **Step 1: index.html——资料弹窗加头像行**

`client/ui/index.html` 的 profile-mask 中，`<h2>完善资料</h2>` 与 `<input id="profile-nickname" ...` 之间插入：
```html
      <div class="profile-avatar-row">
        <div class="profile-avatar-preview" id="profile-avatar-preview"></div>
        <button class="setup-link" id="profile-avatar-pick" type="button">选择图片</button>
        <input id="profile-avatar-file" type="file" accept="image/*" hidden />
      </div>
```

- [ ] **Step 2: app.js——头像缓存、压缩与懒加载函数**

状态区（`let profileSubmitting = false;` 之后）加：
```js
// C：头像
const avatarCache = new Map(); // uid → Blob URL；null = 已确认无头像
const avatarPending = new Set(); // 已请求未回复的 uid（防重入）
let pendingAvatar = null; // 资料弹窗待提交的新头像（压缩后 Uint8Array）
let previewUrl = null; // 弹窗预览的临时 Blob URL（避免泄漏）
```

`buildCard` 之前（渲染工具区）加：
```js
// ---- C：头像懒加载（members 五元组带 has_avatar；AvatarData 空数据 = 无头像） ----
function ensureAvatar(uid, hasAvatar) {
  if (avatarCache.has(uid)) return; // 命中缓存（含 null = 已确认无头像）
  if (!hasAvatar) {
    avatarCache.set(uid, null); // 无头像：不再请求
    return;
  }
  if (avatarPending.has(uid)) return;
  avatarPending.add(uid);
  invoke("avatar_request", { uid }).catch((e) => {
    avatarPending.delete(uid);
    console.error("请求头像失败:", e);
  });
}

// 强制重拉（profile_changed 后头像可能变化，has_avatar 快捷路径不可信）
function forceRequestAvatar(uid) {
  const old = avatarCache.get(uid);
  if (old) URL.revokeObjectURL(old);
  avatarCache.delete(uid);
  avatarPending.delete(uid);
  avatarPending.add(uid);
  invoke("avatar_request", { uid }).catch((e) => {
    avatarPending.delete(uid);
    console.error("请求头像失败:", e);
  });
}
```

`buildCard` 的 avatar 构建替换：
```js
// 原文
  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  avatar.innerHTML = ICONS.avatar;
// 替换为
  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  const avatarUrl = avatarCache.get(uid);
  if (avatarUrl) {
    const img = document.createElement("img");
    img.className = "member-avatar-img";
    img.src = avatarUrl;
    img.draggable = false;
    avatar.appendChild(img);
  } else {
    avatar.innerHTML = ICONS.avatar;
  }
  ensureAvatar(uid, m.hasAvatar); // 懒加载（缓存命中 / 无头像时直接返回）
```

- [ ] **Step 3: app.js——弹窗头像接线（预览 / 选择 / 提交）**

`openProfilePop` 替换（Task 4 版 → 头像版）：
```js
// 原文
function openProfilePop(defaultName) {
  showProfileError("");
  el("profile-nickname").value = defaultName || "";
  el("profile-mask").classList.remove("hidden");
  el("profile-nickname").focus();
}
// 替换为
function openProfilePop(defaultName) {
  showProfileError("");
  pendingAvatar = null;
  el("profile-nickname").value = defaultName || "";
  // 头像预览：优先已缓存的当前头像，否则剪影
  const preview = el("profile-avatar-preview");
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
  el("profile-mask").classList.remove("hidden");
  el("profile-nickname").focus();
}
```

`closeProfilePop` 替换（加清理）：
```js
// 原文
function closeProfilePop() {
  el("profile-mask").classList.add("hidden");
  profileSubmitting = false;
  el("profile-submit").disabled = false;
}
// 替换为
function closeProfilePop() {
  el("profile-mask").classList.add("hidden");
  profileSubmitting = false;
  el("profile-submit").disabled = false;
  pendingAvatar = null;
  if (previewUrl) {
    URL.revokeObjectURL(previewUrl);
    previewUrl = null;
  }
}
```

`submitProfile` 的 invoke 行替换（带上头像）：
```js
// 原文
  invoke("set_profile", { nickname, avatar: null }).catch((e) => {
// 替换为
  invoke("set_profile", { nickname, avatar: pendingAvatar ? Array.from(pendingAvatar) : null }).catch((e) => {
```

`el("profile-nickname").addEventListener("keydown", ...)` 之后追加（压缩与文件选择）：
```js
// ---- C：头像压缩（浏览端缩 256×256 居中裁剪 JPEG q85；≤64KB 校验） ----
async function shrinkAvatar(file) {
  const bmp = await createImageBitmap(file);
  const size = Math.min(bmp.width, bmp.height);
  const sx = (bmp.width - size) / 2;
  const sy = (bmp.height - size) / 2;
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  canvas.getContext("2d").drawImage(bmp, sx, sy, size, size, 0, 0, 256, 256);
  bmp.close();
  const blob = await new Promise((res) => canvas.toBlob(res, "image/jpeg", 0.85));
  if (!blob) throw new Error("图片处理失败");
  if (blob.size > 64 * 1024) throw new Error("图片过大，请换一张更简单的图片");
  return new Uint8Array(await blob.arrayBuffer());
}

el("profile-avatar-pick").addEventListener("click", () => el("profile-avatar-file").click());
el("profile-avatar-file").addEventListener("change", async (e) => {
  const file = e.target.files[0];
  e.target.value = ""; // 允许重复选择同一文件
  if (!file) return;
  try {
    const bytes = await shrinkAvatar(file);
    pendingAvatar = bytes;
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
    const img = document.createElement("img");
    img.className = "member-avatar-img";
    img.src = previewUrl;
    img.draggable = false;
    el("profile-avatar-preview").replaceChildren(img);
    showProfileError("");
  } catch (err) {
    showProfileError(String(err && err.message ? err.message : err));
  }
});
```

- [ ] **Step 4: app.js——事件监听（头像数据与缓存清理）**

`profile_error` 监听之后追加：
```js
  await listen("avatar_data", (e) => {
    const { uid, data } = e.payload;
    avatarPending.delete(uid);
    const old = avatarCache.get(uid);
    if (old) URL.revokeObjectURL(old);
    if (data && data.length > 0) {
      const blob = new Blob([new Uint8Array(data)], { type: "image/jpeg" });
      avatarCache.set(uid, URL.createObjectURL(blob));
    } else {
      avatarCache.set(uid, null); // 确认无头像
    }
    renderMembers();
  });
```

`members` 监听开头加缓存作废（重连后 uid 可能被重新分配，避免串号）：
```js
// 原文
  await listen("members", (e) => {
    members.clear();
// 替换为
  await listen("members", (e) => {
    // 全量列表（首次/重连）：uid 可能被重新分配，头像缓存全部作废
    for (const url of avatarCache.values()) if (url) URL.revokeObjectURL(url);
    avatarCache.clear();
    avatarPending.clear();
    members.clear();
```

`member_leave` 监听加清理：
```js
// 原文
  await listen("member_leave", (e) => {
    const uid = e.payload;
    if (videoView.watchingUid === uid) videoView.end(); // 被观看者离开：订阅已随其退出失效，直接收尾
    members.delete(uid);
// 替换为
  await listen("member_leave", (e) => {
    const uid = e.payload;
    if (videoView.watchingUid === uid) videoView.end(); // 被观看者离开：订阅已随其退出失效，直接收尾
    const url = avatarCache.get(uid);
    if (url) URL.revokeObjectURL(url);
    avatarCache.delete(uid);
    avatarPending.delete(uid);
    members.delete(uid);
```

`profile_changed` 监听加强制重拉：
```js
// 原文
  await listen("profile_changed", (e) => {
    const { uid, nickname } = e.payload;
    const m = members.get(uid);
    if (m) m.nickname = nickname;
    renderMembers();
    renderOnline();
    if (profileSubmitting && uid === myUid) closeProfilePop(); // 自己保存成功：广播回来才关窗
  });
// 替换为
  await listen("profile_changed", (e) => {
    const { uid, nickname } = e.payload;
    const m = members.get(uid);
    if (m) m.nickname = nickname;
    renderMembers();
    renderOnline();
    forceRequestAvatar(uid); // 头像可能同期更新：绕过 has_avatar 快捷路径强制重拉
    if (profileSubmitting && uid === myUid) closeProfilePop(); // 自己保存成功：广播回来才关窗
  });
```

- [ ] **Step 5: style.css——头像行与头像图**

`client/ui/style.css` 的 `.setup-panel .setup-link:hover { text-decoration: underline; }` 之后追加：
```css
.profile-avatar-row {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
}
.profile-avatar-preview {
  width: 72px;
  height: 72px;
  border-radius: 50%;
  overflow: hidden;
  background: #15181f;
  color: #333a49;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.profile-avatar-preview svg { width: 40px; height: 40px; }
.member-avatar-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
```

- [ ] **Step 6: 语法校验**

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

```markdown
验收要点（手工，可延后到 Task 6 合并做）：弹窗选图 → 预览显示裁剪后圆形头像 → 保存后自己卡片与左栏头像出现；对方客户端懒加载拉到同一头像；不设头像的成员显示剪影且不重复发请求。
```

---

### Task 6: 全量验收与 0.2.0 构建交付

**Files:**
- Modify: `client/src-tauri/tauri.conf.json`（版本号）
- 产物：`Echo.exe` + `Echo_0.2.0_x64-setup.exe`

**Interfaces:**
- Consumes: Task 1–5 的全部功能；验收依赖两台客户端（或一台客户端 + sim_clients）
- Produces: 0.2.0 安装包与免安装 exe（交付物）

- [ ] **Step 1: 版本号升级**

`client/src-tauri/tauri.conf.json`：
```json
// 原文
  "version": "0.1.10",
// 替换为
  "version": "0.2.0",
```

- [ ] **Step 2: 全量测试与构建**

Run: `cargo test`
Expected: 全绿（protocol + server + client 全部测例）。

Run: `cargo build`
Expected: workspace 编译通过。

Run: `node --check client/ui/app.js`
Expected: 无输出（语法通过）。

- [ ] **Step 3: 服务器 + 双客户端手工验收（7 条）**

先启动服务器（终端 A）：
```powershell
cd E:\pro\EchoRoom; cargo run -p echoroom-server -- 9000 --invite echo-2026
```
Expected: 打印 `[auth] 数据库: data\echoroom.db`、`[auth] 邀请码已配置，注册开放`、`Echo server listening on 0.0.0.0:9000 (tcp+udp)`；`E:\pro\EchoRoom\data\echoroom.db` 文件生成（cargo run 的 CWD 是 workspace 根，`--data` 相对路径基于 CWD）。

| # | 场景 | 操作 | 期望 |
|---|------|------|------|
| 1 | 注册 + 资料弹窗 | 客户端 A：注册新账号（邀请码 echo-2026） | 成功后自动进房并弹「完善资料」；填昵称 + 选头像保存 → 自己卡片立即更新 |
| 2 | 自动登录 | 关闭并重启客户端 A | 不再显示登录面板，直接进房（Resume；服务器日志可见新连接） |
| 3 | 错误路径 | 依次试：错邀请码 / 已存在账号 / 5 位密码 / 登录错误密码 | 面板分别显示对应文案且不关闭；按钮恢复可点 |
| 4 | 双账号 | 客户端 B 注册另一账号并进房 | 双方卡片互见（含头像懒加载）；两人语音正常 |
| 5 | 服务器重启 | Ctrl+C 停掉终端 A 服务器后原命令重启 | 客户端 A/B 自动重连（Resume）无感，聊天/语音恢复 |
| 6 | 旧版客户端 | 用 0.1.x 旧 exe 连接新服务器 | 旧客户端被断开或认证失败（无法进房），服务器日志正常、其他成员不受影响 |
| 7 | B 功能回归 | 聊天链接标蓝可点 / 音量 0–400% / 静音广播 / 投屏与观看 / 进出音效 | 全部正常（B 交付功能不回归） |

- [ ] **Step 4: sim_clients 冒烟（可选但推荐）**

终端 B：
```powershell
cd E:\pro\EchoRoom; cargo run -p echoroom-server --bin sim_clients -- 127.0.0.1:9000 3 5 echo-2026
```
Expected: 3 个模拟客户端注册成功（可再跑一次验证"账号已存在 → 回退登录"路径）；收到 LoginOk 与 UDP RegisterAck；退出时打印各 uid 语音包统计。

- [ ] **Step 5: 桌面构建与产物拷贝**

```powershell
cd E:\pro\EchoRoom\client\src-tauri; cargo tauri build
```
Expected: 构建成功，产物（cargo 使用 workspace 共享 target 目录，在仓库根）：
- `E:\pro\EchoRoom\target\release\Echo.exe`
- `E:\pro\EchoRoom\target\release\bundle\nsis\Echo_0.2.0_x64-setup.exe`

```powershell
Copy-Item 'E:\pro\EchoRoom\target\release\Echo.exe' "$env:USERPROFILE\Desktop" -Force; Copy-Item 'E:\pro\EchoRoom\target\release\bundle\nsis\Echo_0.2.0_x64-setup.exe' "$env:USERPROFILE\Desktop" -Force
```
Expected: 桌面出现 `Echo.exe` 与 `Echo_0.2.0_x64-setup.exe`。

```markdown
交付说明：0.2.0 为协议破坏性升级——所有客户端必须同步升级；服务器升级后旧客户端无法进房（预期行为）。服务器部署需带 `--invite <码>` 才能开放注册；`--data <目录>` 指定数据库位置（默认 ./data）。
```

---

## 计划完成后的执行提示

- 执行顺序严格按 Task 1 → 6；Task 1–2 期间跨包编译断裂属预期（见 Global Constraints），不要中途打补丁
- 每个任务结束运行其验证命令，全绿后才进入下一任务
- 手工验收（Task 6 Step 3）需要服务器 + 两台客户端（可用 sim_clients 替代其一）
