# EchoRoom 计划2 · B 子项目设计：视频子系统（投屏 + 摄像头）

**日期**：2026-09-18
**状态**：设计已确认（brainstorming 完成，逐节获批）
**前置**：T12（抖动缓冲）、T13（混音+软限幅）、A 子项目（UI 卡片/个体音量/进出音效）已完成
**上位文档**：`2026-09-18-echoroom-plan2-overview.md`（本文件是其子项目 B 的细化）

## 背景与目标

计划2 的第二个子项目：让房间内任何人可以把「屏幕/窗口」或「摄像头」推流给其他成员观看，向 Oopz 的视频体验靠拢。

**已确认的需求基线**（沿用总纲 + 本轮 brainstorming 修正）：

- 采集源两条：屏幕共享、摄像头；二者是**独立的两路流**（独立采集、独立编码、独立订阅传输），共用同一条"采集 → 编码 → 传输 → 渲染"管线形态
- **订阅式按需传输**：点击才开始接收；同时最多看一路（同时最多订阅一个人）；切换 = 换订阅，关闭 = 停收
- 服务器延续"只转发"原则：维护订阅关系，只为订阅者转发；无人观看时通知发送端停止推流（省带宽省 CPU）
- **投屏可带系统声音**：默认开启、投屏者可随时关；声音中必须**排除 EchoRoom 自身发出的声音**（否则观看者会重复听到房间内其他成员的语音形成回声）
- 观看 UI 由 B 实现（A 阶段只做了按钮禁用态与占位）

## 范围

**包含**：投屏与摄像头采集、H.264 编码、订阅式传输协议、服务器订阅转发、观看视图（单路 / 双路小窗 / 全屏）、屏幕声音（采集→传输→播放）、画质三档切换、Win10 兼容性处理。

**不包含**（明确划出）：

- Win10 的共享声音（置灰 + 提示"需要 Windows 11"，见 §5.4）
- 多路同屏观看（仍限一路；"最多看一路"的约束由 UI 层保证）
- 本地自预览（不看自己的流）
- 自适应码率（用发送端手动三档代替）
- 屏幕音频独立音量控制（并入现有混音，固定 100%）
- 观看中"关闭小窗 / 小窗切主画面"操作（双路即组合显示，如需再加）

## 1. 总体架构与数据流

**发送端（流主）**：

```
屏幕/窗口 ──getDisplayMedia──┐
摄像头 ────getUserMedia──────┼→ <canvas> 缩放 → WebCodecs H264 编码   [WebView]
                             │         (浏览器内核，可硬件加速)
                             │              ↓ Tauri IPC（二进制）
系统声音 ─WASAPI 进程回路────→ Opus 48kHz 立体声编码                   [Rust]
   (排除 EchoRoom 进程树)                    ↓
                              UDP 分片（≤1150B/包）→ 服务器
```

**服务器**：只转发。维护两张小表：流状态（谁在推哪路）、订阅关系（谁在看谁）。UDP 视频/屏幕音频包**只转发给订阅者**；语音包行为不变（全员转发）。

**观看端**：

```
UDP 重组(H264 帧) ─[IPC]→ WebCodecs 解码 → <canvas> 渲染               [WebView]
屏幕音频(Opus) → 立体声解码 → 并入混音器（与各成员语音合流）→ 扬声器    [Rust]
```

## 2. 关键技术决策记录

| 决策 | 结论 | 理由 |
|------|------|------|
| 视频技术路线 | **全 WebView**：getDisplayMedia/getUserMedia 采集 + WebCodecs 编解码 + canvas 渲染 | 硬骨头全部交给 Chromium 内核；零新增 Rust 采集/编码依赖；与项目"webview 原生能力优先"风格一致。风险由 spike 前置验证（§7），失败退 MJPEG |
| 屏幕声音采集 | **Rust WASAPI 进程回路**（ActivateAudioInterfaceAsync + EXCLUDE_TARGET_PROCESS_TREE，排除自身进程树） | 浏览器 API 做不到进程级排除；此 API 为 Zoom/Teams 同款方案。**需 Windows 11（build 22000+ / 官方样例标称 20348+）** |
| 带宽预算 | 服务器出口 15-20Mbps：单路 ≤4Mbps × 最重 3 观看 ≈ 12Mbps ✅；**2核2G 现有服务器无需升级** | 服务器只做 UDP 转发，CPU 量为 memcpy 级 |
| 画质档位 | 三档：720p@30（~2Mbps）/ 1080p@15（~2.5Mbps）/ 1080p@30（~4Mbps，人少时用）；摄像头固定 720p@30（~1-1.5Mbps） | 覆盖"代码文字"与"视频流畅"两类场景；发送端随时可切 |
| 传输通道 | 复用现有 UDP 端口与包头（10 字节），新增包类型 | 免开新端口、复用注册/心跳/防伪机制 |
| 屏幕音频默认 | 默认开启，投屏面板可关 | 用户明确的完整体验诉求 |

## 3. 协议改动

### 3.1 TCP 控制消息（`protocol/src/messages.rs`）

现有 1-9 号消息不变；**成员元组扩展**：`(uid, nickname, muted)` → `(uid, nickname, muted, streams)`，其中 `streams: u8` 位图（bit0 = 投屏中，bit1 = 摄像头中）。影响 `LoginOk` / `Members` 同步（新加入者立即可见谁在推流）。

新增消息：

| 号 | 消息 | 方向 | 载荷 | 说明 |
|----|------|------|------|------|
| 10 | `StreamState` | C→S 上报（uid 填 0）+ S→C 广播（含流主本人） | `{uid, kind(0=屏幕,1=摄像头), on}` | 流主上报开始/停止推流，服务器广播 → 卡片"纱+按钮"出现/消失；与 Muted 同模式 |
| 11 | `Subscribe` | C→S | `{target: u16}` | 订阅某人的全部流（覆盖式：先清旧订阅） |
| 12 | `Unsubscribe` | C→S | `{}`（uid 字段填 0） | 退订当前订阅 |
| 13 | `ViewerCount` | S→C（定向给流主） | `{n: u16}` | 观看人数变化；0→1 启动推流，1→0 停止推流 |
| 14 | `RequestKeyframe` | C→S，服务器转发 S→C | `{uid: 请求者, target: 流主}` | 订阅后立即发一次（秒开）；解码端连续丢帧时补发 |

### 3.2 UDP 包类型（`protocol/src/udp.rs`）

现有 1-5 号不变。新增：

| 号 | 包 | 载荷 | 转发规则 |
|----|----|------|---------|
| 6 | `VideoChunk` | `[kind u8][flags u8][frame_seq u16][chunk_idx u8][chunk_count u8] + data(≤1150B)` | 只发给订阅者 |
| 7 | `ScreenAudio` | Opus 包（48kHz 立体声） | 只发给订阅者 |

- `flags`：bit0 = keyframe（IDR），bit1 = last_chunk（帧末片）
- `frame_seq` u16 循环，比较用 wrapping 差值；一帧上限 255 片（≈293KB，远超 4Mbps 下关键帧峰值）
- 包总长 ≤ 1166B（6B 载荷头 + 1150B 数据 + 10B 包头）< 现有 `MAX_PACKET` 1200 ✅

### 3.3 分片与重组策略

- **发送**：完整编码帧按 ≤1150B 切片逐包发出；同一帧的片共享 `frame_seq`
- **重组（接收端）**：按 `frame_seq` 开帧缓冲，乱序等待；**任何一片丢失 → 整帧丢弃**（不重传，时效优先），等下一 `frame_seq`；收到新 `frame_seq` 时旧缓冲未完成也直接丢弃
- **解码连续失败**（如 3 帧）或收到首个 IDR 前 → 发 `RequestKeyframe`；编码端收到即下一个编码帧 `keyFrame: true`
- **自然关键帧间隔**：2 秒
- 语音 `Voice` 与屏幕音频 `ScreenAudio` 均为小包，丢包按各自链路既有策略（PLC / 静音）处理，不重传

## 4. 服务器改动

**`server/src/room.rs`**：

- `Member` 增加 `streams: u8` 位图
- `Room` 增加 `subscriptions: HashMap<u16, u16>`（订阅者 → 目标）
- 新方法：`set_stream(uid, kind, on) -> Option<u8>`（返回新位图）、`subscribe(sub, target)`（覆盖式）、`unsubscribe(sub)`、`subscribers_of(target) -> Vec<SocketAddr>`（仅取有 UDP 地址者）、`viewer_count(target) -> usize`
- `leave` 清理：成员离开时同时清"他作为订阅者的记录"与"订阅了他的记录"

**`server/src/tcp.rs`**：

- `Subscribe` → 记订阅 + 给目标发 `ViewerCount`（计数含新订阅者）
- `Unsubscribe` / 断线清理 → 给原目标发 `ViewerCount`
- `RequestKeyframe` → 转发给 target（带请求者 uid）
- `StreamState` 广播：流主开/停流时（含发给流主本人）；流主停止某路流时若订阅者仍订着他，不强制退订（观看端按 StreamState 处理）
- 成员离开时清理其订阅两向记录（作为订阅者、作为被订阅者）；订阅者端依赖既有 `MemberLeave` 广播退出观看视图

**`server/src/udp.rs`**：

- `VideoChunk` / `ScreenAudio` → 用来源地址反查真实 uid（沿用防伪），转发给 `subscribers_of(uid)`（两人都已有 udp 地址时）
- `Voice` 转发逻辑保持不动

## 5. 客户端实现

### 5.1 发送端（流主）

- **采集/编码（WebView，`client/ui/video.js`）**：
  - 屏幕：`getDisplayMedia({video})`（优先系统选择器；若 WebView2 选择器不可用，spike 后改用 `additionalBrowserArgs` 自动选屏）
  - 摄像头：`getUserMedia({video: 1280×720@30})`
  - 帧流：`MediaStreamTrackProcessor` → `<canvas>` 缩放（色域/尺寸统一到目标档位）→ `VideoEncoder`（H.264，`latencyMode: 'realtime'`，bitrate 按档位）
  - 编码输出 `EncodedVideoChunk` → `invoke` 二进制送 Rust（spike 确认通道形态）
- **传输（Rust，`net/udp.rs`）**：收 IPC 帧 → 分片发送；`kind` 区分两路
- **推流启停（ViewerCount 驱动）**：采集保持（track 不关，避免重弹选择器），编码与发送仅在 `ViewerCount ≥ 1` 时运行；0 时销毁编码器
- **屏幕声音（Rust，新 `audio/screen_capture.rs`）**：投屏开始且开关开启时启动 WASAPI 进程回路捕获（EXCLUDE 自身进程树）→ 48kHz float32 立体声 → i16 → Opus 128kbps 立体声（`opus.rs` 新增立体声封装，Mono 路径不动）→ `ScreenAudio` 包，同样受 ViewerCount 控制启停
- **关键帧**：自然 2s 间隔 + `request_keyframe` 事件（JS 侧 `encode({keyFrame: true})`）

### 5.2 观看端

- **订阅（Rust `net/tcp.rs`）**：进入观看 → `Subscribe{target}`；退出/切换 → `Unsubscribe` 再订阅；订阅成功后**立即发一次 `RequestKeyframe`**（秒开）
- **接收重组（Rust `net/udp.rs`）**：`VideoChunk` 按 §3.3 重组 → 完整帧经 IPC Channel 送 WebView；`ScreenAudio` → 立体声解码 → 混音器新增一路（当前订阅流主的屏幕音频，单路）
- **解码渲染（WebView `video.js`）**：`VideoDecoder`（首个 IDR 后 configure 完成）→ `VideoFrame` → 绘制到观看 canvas；连续解码失败 → invoke 请求关键帧并 reset
- **混音（Rust `audio/session.rs`）**：屏幕音频作为"当前观看流"的附加输入并入播放混音；声道按现有播放链路适配（必要时降混）

### 5.3 画质档位与拖拽小窗

- 三档参数（编码侧 `VideoEncoder.configure`）：`720p30`（1280×720@30, 2Mbps）、`1080p15`（1920×1080@15, 2.5Mbps）、`1080p30`（1920×1080@30, 4Mbps）
- 切换 = `reconfigure` + canvas 尺寸调整；观看端自动适配画面尺寸（解码分辨率变化由解码端自然处理，重发关键帧保底）
- **小窗**（摄像头，双路同开时）：默认贴主画面右下角；`比例锁死` 的拖角缩放（16:9 基准，最小 120×67.5、最大 480×270）；左键拖动移位（限制在主画面范围内）；全屏时小窗规则相同

### 5.4 Windows 版本兼容性

- **Windows 11（build 22000+）**：完整功能（进程回路捕获为 Win11 引入的能力）
- **Windows 10**：投屏/摄像头视频功能不受影响；"共享声音"开关**置灰**，hover 提示"需要 Windows 11（系统限制）"
- 检测：Rust 侧读取系统构建号（`RtlGetVersion`，无需应用 manifest），通过事件告知前端是否支持共享声音

## 6. UI 与交互

### 6.1 发送端（自己卡片）

- 投屏 / 摄像头按钮点击启用（移除禁用态与"即将推出"提示）
- **激活态**：淡蓝底 `rgba(59,130,246,0.15)` + 图标 `#3b82f6`（A 已定义样式直接接线）；头像区左上角小角标"投屏中 / 摄像头中"
- **投屏中点击投屏按钮** → 弹小面板（沿用量 popover 风格）：三档画质单选 + "共享声音"开关 + "停止投屏"
- **摄像头中点击** → 直接停止（无面板）
- 档位与"共享声音"偏好持久化到 config（`share_quality: "720p30" | "1080p15" | "1080p30"`，`share_audio: bool`）

### 6.2 观看视图

- **入口（对他人卡片）**：该成员有任一流 → 头像覆盖"纱"（半透明遮罩）+ 中央半透明按钮（hover 提示"点击观看"）；三种状态（只投屏 / 只摄像头 / 都有）入口形态一致
- **单路观看**：卡片网格隐藏，画面**高 230px 固定 + 宽按视频比例**（16:9 → 408px）+ 水平居中；窗口拉伸画面尺寸不变
- **双路观看**：投屏为主画面（同上尺寸规则），摄像头为**右下角小窗**（见 §5.3；全屏/非全屏均如此）
- **控制件**：右下角全屏按钮（窗口全屏，画面按屏幕等比放大、小窗规则不变）；左上角关闭按钮（×）→ 退出观看恢复卡片网格
- **自动退出**：对方停止推流（`StreamState off`）或离开房间（`MemberLeave`）→ 观看视图退出，回卡片网格
- **说话蓝框/静音图标**等卡片既有视觉逻辑不受观看态影响（网格隐藏时自然不可见）

## 7. spike 验证计划（计划的第一个任务，半天内出结论）

| # | 验证项 | 通过标准 | 失败退路 |
|---|--------|---------|---------|
| 1 | `getDisplayMedia` 在 WebView2 | 选择器唤起且可用（或自动选屏可用） | `additionalBrowserArgs` 加 `--auto-select-desktop-capture-source` |
| 2 | `VideoEncoder` H.264 | 1080p 15/30fps 出帧，CPU 占用可接受，码率贴近目标 | 退 MJPEG 方案（canvas.toBlob） |
| 3 | `MediaStreamTrackProcessor` | 帧流稳定，窗口最小化后仍持续（配合反节流 flags） | 定时器轮询 + flags |
| 4 | Tauri IPC 二进制吞吐 | 30fps × ~100KB 双向无堆积 | `tauri::ipc::Channel` / 自定义协议优化 |
| 5 | WASAPI 进程回路（EXCLUDE 自身） | 系统音乐可录、EchoRoom 语音确认被排除 | 共享声音仅限验证通过的环境并提示 |
| 6 | 反节流 flags | 最小化/后台时编码与 IPC 不暂停 | 提示"投屏时请勿最小化" |

## 8. 文件改动清单

**协议（`protocol/src/`）**：`lib.rs`（新常量：分片大小、屏幕音频码率等）、`messages.rs`（5 新 TCP 消息 + 2 新 UDP 包 + 成员元组扩展）、`tcp.rs`、`udp.rs`（编解码 + 测试）

**服务器（`server/src/`）**：`room.rs`（流状态 + 订阅表 + 方法 + 单测）、`tcp.rs`（新消息处理 + ViewerCount）、`udp.rs`（订阅转发）

**客户端 Rust（`client/src-tauri/src/`）**：
- `Cargo.toml`：新增 `windows` crate（WASAPI 进程回路所需 COM API）
- `net/tcp.rs`：NetCmd 扩展（Subscribe/Unsubscribe/RequestKeyframe/StreamState 上报）、新消息处理、事件出口
- `net/udp.rs`：VideoChunk 分片发送 / 重组接收 / ScreenAudio 收发
- `audio/screen_capture.rs`（新）：WASAPI 进程回路采集（EXCLUDE 自身进程树）
- `audio/opus.rs`：立体声编解码封装（Mono 不动）
- `audio/session.rs`：屏幕音频发送接线 + 接收端解码/混音接线
- `bridge.rs`：新 invoke 命令（send_video_frame / subscribe / unsubscribe / request_keyframe / set_share_quality / set_share_audio）+ 下行 IPC Channel（解码帧）+ 事件（stream_state / viewer_count）
- `config.rs`：`share_quality` / `share_audio` 字段
- `tauri.conf.json`：`additionalBrowserArgs` 追加（spike 结论）

**前端（`client/ui/`）**：`video.js`（新：采集/编码/解码/观看视图）、`app.js`（接线：入口、按钮、面板）、`index.html` + `style.css`（观看视图、小窗、角标、面板）

## 9. 测试与验收

**Rust 单测**：

- `udp.rs`：VideoChunk 编解码往返；分片重组（乱序到达、丢片丢整帧、新帧打断旧帧）；ScreenAudio 往返
- `messages.rs`/`tcp.rs`：新消息编解码往返；成员元组（含 streams 位图）往返
- `room.rs`：订阅/覆盖订阅/退订；`subscribers_of` 只含已注册 UDP 的订阅者；成员离开清理两向记录；`viewer_count` 计数

**双客户端手动验收**：

1. A 开投屏 → B 卡片出现"纱 + 中央按钮" → 点击观看，画面正常（比例正确、清晰）
2. 三档画质切换：B 观看到的画面分辨率/流畅度随之变化；切换时画面自动恢复（关键帧）
3. 摄像头：单独观看 → 直接看摄像头画面；与投屏同开 → 主画面 + 右下角小窗（拖角缩放比例锁死、拖动移位；全屏下同规则）
4. 全屏按钮与关闭按钮行为正确，退出恢复卡片网格
5. **屏幕音频**：A 放音乐 → B 能听到；B/C 说话时确认投屏声中**没有**他们的语音（回声排除验证，Win11 关键项）；"共享声音"关闭后 B 听不到
6. 切换观看对象（A 流 → C 流）流畅，无残留画面/声音
7. A 停止投屏 → B 观看视图自动退出回网格
8. **无人观看不推流**：A 开投屏但无人订阅时，发送端 CPU/上行无视频流量；B 点开后立即起流（订阅即请求关键帧，秒开）
9. Win10 环境（如有）：共享声音开关置灰并提示"需要 Windows 11"；投屏画面功能正常

## 10. 明确不做（YAGNI）

- Win10 共享系统声音（置灰 + 提示）
- 多路同屏观看（同时最多一路，UI 层约束）
- 本地自预览、画面截图/录制
- 自适应码率、网络自适应画质
- 屏幕音频独立音量、音画时间轴同步（无严格唇音同步需求）
- 观看中的小窗开关/切换主画面（双路即组合，如需后补）
- 视频加密/鉴权（延续项目现状：token 机制仅绑定 UDP 通道）
