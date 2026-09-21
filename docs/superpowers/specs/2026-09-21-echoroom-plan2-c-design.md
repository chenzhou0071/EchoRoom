# EchoRoom 计划2 · C 子项目设计：账号系统（登录/注册 + 头像）

**日期**：2026-09-21
**状态**：设计已确认（brainstorming 完成，逐节获批）
**前置**：A（UI 改版 + 个体音量）、B（视频子系统）已完成
**上位文档**：`2026-09-18-echoroom-plan2-overview.md`（本文件是其子项目 C 的细化）

## 背景与目标

计划2 的第三个子项目：引入账号体系，让"身份"从一次性的昵称输入升级为持久账号——昵称与头像存储在服务器、登录即可找回身份，同时向 Oopz 的完成度靠拢。

**本轮 brainstorming 确认的需求基线**：

- 三个目标并重：**身份稳定 + 头像**（换设备/重装后仍是"你"）、**防冒名**（账号+密码保证身份唯一）、**完成度与体验**（正式登录/注册流程）
- **无游客模式**：彻底移除"输入昵称即连接"，必须账号进入（原 `Login{nickname}` 删除）
- 注册流程：**账号 + 密码 + 邀请码** → 注册成功即进房 → 客户端弹「完善资料」窗（昵称 + 头像，可跳过）
- **账号与昵称分离**：账号（登录凭证，全服唯一，统一小写存储/比较）；昵称（展示用，**允许重复**，默认=账号名）
- **头像 = 上传本地图片**：客户端缩图（256×256 居中裁剪、JPEG q85）后上传，服务器存储；不传则显示默认剪影
- **邀请码制**：服务器配置注册码；未配置时拒绝一切注册（"未开放注册"）

## 范围

**包含**：认证协议（注册/登录/自动登录）、服务器账号持久化（SQLite）、资料设置（昵称/头像）、头像分发与渲染、登录/注册/完善资料三块 UI、服务器配置入口（数据目录/邀请码）、自动登录与重连。

**不包含**（明确划出）：

- 登出/切换账号 UI（D 设置系统补入口；自动登录失效会回登录页兜底）
- 改密码、找回密码、邮箱/手机验证
- 登录限速与防爆破（6 人小服务器，YAGNI）
- 清除头像、头像裁剪编辑界面（只做居中裁剪自动处理）
- 多房间/大厅（登录与进房合一，延续单房间模型）

## 1. 总体架构与会话生命周期

**架构一句话**：账号持久化进 SQLite（服务器 `data/echoroom.db`）；在线成员仍是内存 `Room`；TCP 连接的首条消息由"直接发昵称"改为**三种认证之一**，认证成功即进房（登录与进房合一）。

```
客户端启动
 ├─ config 有 auth_token → 连接 → Resume{token} ─┬─ 成功 → 静默进房
 │                                               └─ 失败 → 登录页（提示"登录已过期"）
 ├─ 登录页：账号+密码 → Login ──→ 成功进房（下发新 auth_token 供下次自动登录）
 └─ 注册页：账号+密码+邀请码 → Register → 成功即进房（昵称暂为账号名）
       └─ 随即弹「完善资料」窗（昵称预填账号名 + 头像可选）
              └─ SetProfile → 服务器更新 + 广播 ProfileChanged → 全体卡片刷新
```

- **身份 token**（`auth_token`，自动登录用）与 **UDP token**（现有绑定校验用）是两套东西，命名显式区分
- 断线自动重连沿用现有逻辑，改为携带 `auth_token` 的 `Resume` 重连（无需重新输密码）

## 2. 关键技术决策记录

| 决策 | 结论 | 理由 |
|------|------|------|
| 存储引擎 | **SQLite（rusqlite，bundled）**，头像 BLOB 入库 | 用户选定；bundled 把 SQLite 编进二进制，服务器免装；事务安全、扩展性好 |
| 密码存储 | **argon2id**（`argon2` crate，PHC 字符串自带盐与参数） | 业界标准，纯 Rust；2C2G 上单次 ~20-50ms 可接受 |
| token 生成 | **真随机**（`rand` OsRng，16 字节 → 32 位 hex） | 现有 LCG 可预测，绝不能用于身份凭证；顺手把 UDP token 的 LCG 也换掉 |
| 多设备 | 每账号最多 **8 条** token，超出删最旧；同账号可多机同时在线（各自 uid） | "换设备"是账号系统的核心动机之一 |
| 认证与进房 | **合一**：认证成功即加入房间，单消息往返 | 单房间模型下两步没有收益 |
| 注册后体验 | **注册成功即进房**，资料弹窗盖在房间上；跳过则昵称=账号名 | 任何一步中断都有合法状态（账号已建、身份可用） |
| 头像传输 | **懒加载**：成员元组带 `has_avatar` 标记，前端按需 `AvatarRequest` 拉取 | 避免进房/成员事件广播大块二进制；换头像后重拉即刷新 |
| 头像限制 | 单张 **≤64KB**（客户端缩 256×256 JPEG q85 一般 10-30KB）；服务器二次校验 | 帧内 bytes 字段可控，渲染开销恒定 |
| 邀请码 | 服务器 `--invite` 参数；**未配置 = 拒绝一切注册** | 公网 IP 会被扫描机器人扫到，默认最安全 |
| 账号格式 | `[A-Za-z0-9_]{3,20}`，统一小写存储与比较 | 防 `Alice`/`alice` 抢注混淆；密码 6-64 字符、大小写敏感 |
| 昵称规则 | 1-24 字符，允许重复，默认=账号名 | 用户确认：昵称纯展示，账号才是身份 |

## 3. 协议改动（`protocol/src/messages.rs`）

### 3.1 TCP 消息（type id 1-14 已用）

| 号 | 消息 | 方向 | 载荷 | 说明 |
|----|------|------|------|------|
| 1 | `Login`（改造） | C→S | `{account, password}` | 替换原 `{nickname}`；登录成功即进房 |
| 2 | `LoginOk`（改造） | S→C | `{auth_token, uid, udp_token, members}` | 字段重命名：身份 token 与 UDP token 分离 |
| 7 | `AuthReject`（原名 LoginReject，改名复用） | S→C | `{reason}` | 注册/登录/Resume 三种失败共用 |
| 15 | `Register` | C→S | `{account, password, invite}` | 注册成功即进房（昵称=账号名） |
| 16 | `Resume` | C→S | `{auth_token}` | 自动登录 |
| 17 | `SetProfile` | C→S | `{nickname, avatar: Option<Vec<u8>>}` | `None`=不改头像；进房后调用 |
| 18 | `ProfileChanged` | S→C 广播 | `{uid, nickname}` | 昵称变更立即生效；头像由前端重拉 |
| 19 | `AvatarRequest` | C→S | `{uid}` | 懒加载头像 |
| 20 | `AvatarData` | S→C | `{uid, data}` | 无头像回空数据 |

**成员元组扩展**（全局）：`(uid, nickname, muted, streams)` → **加 `has_avatar: bool`** 共五项；`MemberJoin` 同步增加 `has_avatar` 字段。影响 `LoginOk.members` 及后续同步链路。

### 3.2 序列化扩展（`protocol/src/tcp.rs`）

- 新增 **bytes 字段**编码：`[len u32][原始字节]`（现有 `String` 的 u16 长度前缀不够 64KB 上限）
- 帧头 `[len u32][type]` 不变，可容纳头像单帧

### 3.3 首消息约束

- 服务器 `read_first_login` → **`read_first_auth`**：首条消息必须为 `Register` / `Login` / `Resume` 之一，其余按协议错误断开

## 4. 服务器改动

**`server/src/main.rs`**：新增参数 `--data <目录>`（默认 `./data`，DB 位于 `data/echoroom.db`，启动自动建目录/建表）、`--invite <码>`；构建时初始化 db 模块并注入 tcp 层。

**`server/src/db.rs`（新）**：rusqlite 封装（`Arc<Mutex<Connection>>` 单连接互斥，低频操作足够）：

```sql
CREATE TABLE IF NOT EXISTS accounts (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  account       TEXT NOT NULL UNIQUE,      -- 小写存储
  password_hash TEXT NOT NULL,             -- argon2 PHC 字符串
  nickname      TEXT NOT NULL,             -- 默认=账号名
  avatar        BLOB,                      -- NULL = 无头像
  created_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tokens (
  token      TEXT PRIMARY KEY,             -- 32 位 hex
  account_id INTEGER NOT NULL,             -- 每账号上限 8 条，超出删最旧
  created_at INTEGER NOT NULL
);
```

接口：`create_account` / `verify_login`（查账号 + argon2 verify）/ `insert_token`（含 8 条修剪）/ `find_by_token` / `update_nickname` / `update_avatar` / `get_avatar` / `has_avatar`。

**`server/src/auth.rs`（新）**：注册/登录/Resume/资料更新的业务逻辑与格式校验（账号、密码、昵称、邀请码、头像大小），错误原因文案：

- 注册/资料：`邀请码错误` / `账号已存在` / `账号格式不合法（3-20 位字母数字下划线）` / `密码至少 6 位` / `昵称不合法（1-24 字符）` / `未开放注册` / `头像过大（上限 64KB）`
- 登录：统一 `账号或密码错误`（不泄露账号是否存在）
- Resume：`登录已过期`（客户端回登录页）

**`server/src/room.rs`**：

- `Member` 增加 `account_id: i64`、`has_avatar: bool`
- 新方法：`update_nickname(uid, nickname)`、`set_has_avatar(uid, bool)`、`account_id_of(uid)`
- 成员元组（join 返回、broadcast 用）升级为五元组

**`server/src/tcp.rs`**：

- `read_first_auth`：三条认证路径 → 成功时组装 `LoginOk` 并进入现有读/写循环（后续消息处理不动）
- `SetProfile`：更新 DB（昵称/头像）→ 更新 `Room` 成员昵称/头像标记 → 广播 `ProfileChanged`
- `AvatarRequest`：按 uid 查 `account_id` → DB 取 BLOB → 定向回 `AvatarData`

## 5. 客户端改动

### 5.1 启动流程与 config（`client/src-tauri/src/config.rs`）

- 新增 `auth_token: String`（有则启动静默 Resume）、`account: String`（预填登录表单）
- 删除 `nickname` 字段（昵称真值在服务器；`server_addr` 保留）
- 启动：有 token → 连接 + `Resume` → 成功直接进房；失败/无 token → 显示登录页

### 5.2 Rust 层

- `net/tcp.rs`：连接后的**首消息模式**由 bridge 传入（`Login` / `Register` / `Resume` 三种）；新消息处理与事件出口（`auth_result` / `profile_changed` / `avatar_data`）
- `bridge.rs` 新命令：`auth_login {account, password}`、`auth_register {account, password, invite}`、`set_profile {nickname, avatar}`、`avatar_request {uid}`；重连逻辑改为携 token Resume

### 5.3 UI：登录/注册页（改造现 `setup-mask` 面板）

- 登录态：服务器地址 + 账号 + 密码 + [登录] + [注册] 切换
- 注册态：追加邀请码行 + [返回登录] 切换
- 面板内错误提示行（显示 `AuthReject.reason`）；连接中/失败状态沿用顶栏
- 提交时序：保存服务器地址（如变更）→ 建立连接 → TCP 就绪后发送对应认证首消息（Login/Register）

### 5.4 UI：完善资料弹窗与头像

- **完善资料弹窗**（新）：注册成功后立即弹出——昵称输入（预填账号名）+ 头像选择（`<input type=file accept=image/*>` → canvas 居中裁剪 256×256 → JPEG q85 → invoke 上传）；[完成] [跳过]
- **改资料入口**：点击自己卡片的名字可再次打开该弹窗（C 阶段即可改昵称/头像；入口在 D 可再挪）
- **头像渲染**：成员元组带 `has_avatar` → 前端对未缓存的 uid 逐个 `avatar_request` → 收到后建 Blob URL 缓存（会话内 Map）→ 卡片头像区由剪影换成真实头像；`ProfileChanged` 后头像缓存失效重拉
- 无头像/拉取失败 → 维持现有剪影占位

## 6. 错误与边界处理

| 场景 | 行为 |
|------|------|
| 注册/登录/Resume 失败 | `AuthReject{reason}` → 面板错误行/登录页提示；连接随后关闭 |
| 服务器重启 | DB 持久，token 仍有效 → 客户端自动重连（携 Resume）无感知；房间内存态清空与现状一致 |
| 多设备 | 同账号多机在线（各自 uid、uid 内昵称相同）；token 独立、上限 8/账号 |
| 头像超限 | 服务器二次校验 ≤64KB，超限拒绝并回错误 |
| 换头像/改昵称 | `ProfileChanged` 广播 → 昵称直接更新；头像缓存失效后重拉 |
| 注册中途断开 | 账号已建（昵称=账号名），重新登录即可；SetProfile 未提交无副作用 |
| 并发注册同名 | SQLite UNIQUE 约束兜底 → 返回"账号已存在" |
| 旧版客户端 | 首消息不符（旧 `Login{nickname}` 解码失败）→ 协议错误断开，服务器不崩溃；朋友圈统一更新新版 |

## 7. 测试与验收

**Rust 单测**：

- `protocol`：新消息编解码 round-trip（含 bytes 字段、成员五元组、`Option<Vec<u8>>`）
- `server/db.rs`：`:memory:` SQLite——建表、创建/查询账号、token 插入与 8 条修剪、资料更新、头像存取
- `server/auth.rs`：注册全链路（校验/查重/哈希）、登录成败、Resume 失效、昵称/账号格式边界
- `server/room.rs`：现有测试随五元组升级；新增 `update_nickname`/`account_id_of` 行为

**手工验收（双客户端）**：

1. 注册（正确邀请码）→ 进房 → 弹窗设昵称+头像 → 另一客户端看到新昵称与头像
2. 重启客户端 → 自动登录进房（无需输密码）
3. 错误路径：错密码 / 错邀请码 / 重复账号 / 未开放注册 → 各显示对应提示
4. 两账号双开互见昵称头像；同账号双开（多设备）各自正常在线
5. 点击自己名字 → 改昵称/头像 → 他人实时更新（昵称广播、头像重拉）
6. 服务器重启 → 客户端重连自动登录成功且房间状态正确
7. 旧版本客户端连接 → 被拒绝（预期行为）

**构建**：`node --check` + 全量 `cargo test` + `cargo tauri build`；客户端版本 **0.2.0**（协议破坏性变更）；服务器升级部署（含 `data/` 目录自动初始化）。

## 8. 文件改动清单

**协议（`protocol/src/`）**：`messages.rs`（认证消息改造 + 6 新消息 + 五元组）、`tcp.rs`（bytes 编解码 + 各消息序列化 + 测试）

**服务器（`server/src/`）**：`main.rs`（参数与初始化）、`db.rs`（新）、`auth.rs`（新）、`tcp.rs`（read_first_auth + SetProfile/AvatarRequest）、`room.rs`（account_id/has_avatar/update_nickname）、`Cargo.toml`（rusqlite bundled / argon2 / rand）

**客户端 Rust（`client/src-tauri/src/`）**：`config.rs`（auth_token/account，删 nickname）、`net/tcp.rs`（首消息模式 + 新消息）、`bridge.rs`（4 新命令 + 3 事件 + 重连改造）、`Cargo.toml`（如需）

**前端（`client/ui/`）**：`index.html` + `style.css`（登录/注册页改造、资料弹窗、头像样式）、`app.js`（认证流程接线、头像加载/缓存/渲染、弹窗逻辑）、`tauri.conf.json`（0.2.0）

## 9. 明确不做（YAGNI）

- 登出/切换账号 UI（D 设置系统补；失效自动回登录页兜底）
- 改密码 / 找回密码 / 邮箱验证
- 登录限速与防爆破、验证码
- 清除头像、头像裁剪编辑界面（仅居中裁剪自动）
- 多房间 / 大厅（维持登录即进房）
- 昵称唯一性/敏感词审核（昵称可重复，朋友圈自治）
