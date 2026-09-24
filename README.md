# EchoRoom — 实时语音聊天室

一个轻量的桌面实时语音聊天室：**单房间、最多 6 人、允许同时说话**，带公屏文字与成员状态展示。
打开客户端即连（自动登录），核心链路——网络收发与转发、抖动缓冲、混音、采集与播放——全部亲手实现，
仅 Opus 编解码使用成熟库。语音之外还补齐了**投屏 / 摄像头、账号系统与设置系统**。

> 个人学习与技术研究项目：用于走通"麦克风 → 网络 → 扬声器"的实时音频链路，同时也真的在小圈子里日常使用。

## 环境要求

- **Windows 10 / 11**（投屏"共享系统声音"需要 Windows 11，其余功能 Windows 10 可用）
- **Rust 工具链**（rustup + MSVC 生成工具）
- **tauri CLI v2**（`cargo tauri` 命令，用于开发与打包）

## 架构一览

```
┌─ 客户端（tauri + WebView UI）───────────────────────────┐
│  采集线程  麦克风 → DSP 链 → VAD → Opus → UDP           │
│  播放线程  UDP → 抖动缓冲 → 解码 → 混音 → WASAPI         │
│  TCP 线程  登录/注册、成员、公屏、说话与静音、订阅关系      │
│  WebView  界面 + 投屏/摄像头采集与编解码（WebCodecs）      │
└─────────────────────────────────────────────────────────┘
              ↕ TCP（控制，可靠） + UDP（语音/视频，实时）
┌─ 服务端（无界面 Rust 程序）──────────────────────────────┐
│  TCP：房间管理、账号（SQLite）、公屏与状态广播              │
│  UDP：单 socket 主循环——按来源地址反查成员，原样转发        │
│       （不解码、不混音；视频只转发给订阅者）               │
└─────────────────────────────────────────────────────────┘
```

- **服务器只做"管道"**：语音包不解码、不加工，CPU 开销趋近于零；所有音频理解（抖动缓冲、混音）都在客户端；
- **双通道分工**：TCP 承载控制（有序可靠），UDP 承载语音（容忍丢包，靠 Opus 丢包隐藏补音）；
- **`protocol` 是单一事实来源**：客户端与服务端共用同一份二进制报文编解码（零第三方依赖）。

## 运行

```powershell
# 客户端（开发 / 打包；在 client/src-tauri 目录执行）
cd client/src-tauri
cargo tauri dev      # 开发运行
cargo tauri build    # 打包：target/release/Echo.exe 与 NSIS 安装包 Echo_<版本>_x64-setup.exe

# 服务端（本地联调）
cargo run -p echoroom-server --bin echoroom-server -- --data ./data
```

部分链路自测无需启动服务器——两端内置了若干验证工具（客户端：`send_wav` 发音机器人、`loopback_test` 自听直通等；服务端：`sim_clients` 模拟客户端压测）。

## 功能开关（构建前配置）

音效 / 主题 / 背景图三类功能可以**在打包前整体裁掉**：编辑 `client/ui/features.js` 后重新打包即可，关闭的功能在产物中界面不显示、行为不生效：

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

| 关闭的开关 | 表现 |
|------------|------|
| `sound` | 设置页"音效"分类消失；不播放任何进出房提示音（含默认音效） |
| `theme` | 外观页"主题"区不显示；固定使用默认主题（不读取配置） |
| `background` | 外观页"背景图"区不显示；不铺自定义背景 |
| `theme` + `background` 同关 | "外观"分类整个消失 |
| 三项全关 | 设置页只剩 账号 / 设备 两类 |

新增开关 = 加一行配置 + 在界面层与行为层各接一处短路；文件缺失或写错时**默认全部开启**（不崩溃）。

## 服务器部署

```powershell
# 构建（任一平台交叉编译亦可，产物无运行时依赖）
cargo build --release -p echoroom-server
# 产物：target/release/echoroom-server
```

启动参数：

| 参数 | 说明 |
|------|------|
| `--data <目录>` | 数据目录（SQLite 账号库与头像文件），默认 `./data`，启动自动创建 |
| `--invite <码>` | 注册邀请码；**未配置时拒绝一切注册**（公网安全默认） |

- 监听端口默认为 **9000，TCP + UDP 同一端口**，防火墙需放行两条规则；
- Linux 上建议用 systemd 常驻（`Restart=always` 开机自启 + 崩溃拉起），例如：

```ini
[Unit]
Description=EchoRoom server
After=network.target

[Service]
ExecStart=/opt/echoroom/target/release/echoroom-server --data /opt/echoroom/data --invite <邀请码>
Restart=always

[Install]
WantedBy=multi-user.target
```

## 用法

1. 服务端启动后，客户端首次打开**填写服务器地址**（`主机:端口`），注册（需邀请码）或登录；
2. 之后启动**自动登录进房**；顶栏显示连接状态，断线自动重连（无需重新输密码）；
3. 主界面：成员卡片网格 + 公屏。点自己卡片上的按钮控制**静音 / 投屏 / 摄像头**；点他人卡片的音量按钮调节**对自己而言的播放音量**；
4. 他人头像出现"纱 + 按钮"表示正在投屏/开摄像头，**点击进入观看**（右下角全屏；摄像头与投屏同开时摄像头为右下角小窗，可拖动与缩放）；
5. 顶栏齿轮进入**设置页**：账号（改资料 / 退出登录）、设备（麦克风/扬声器/摄像头，**热切换、语音不断**）、音效（进出房提示音，可换包或关闭）、外观（主题、自定义背景图）。

## 功能特性

- **实时语音**：≤6 人同时说话；Opus 编码；采集链自带 DSP——高通滤波、RNNoise 自适应降噪、
  咔哒瞬态抑制（DeClicker）、低频冲击抑制（PopLimiter，治"噗"式喷气）、软限幅与响度整平；
- **公屏文字** + 说话状态高亮（卡片发光）；
- **个体音量**：自己的"音量"= 采集增益，他人的"音量"= 播放增益（按昵称记忆，重启保留）；
- **手动静音**：不发语音包 + 向全房间广播静音状态（他人卡片显示静音图标）；
- **进出房音效**：进场/离场提示音，支持多套音效包与"无提示音"（重连不重播）；
- **投屏 + 摄像头**：WebCodecs H.264 编解码，三档画质（720p30 / 1080p15 / 1080p30）；
  **订阅式按需传输**——有人看才推流，无人观看自动停；订阅即时请求关键帧（秒开）；
  Windows 11 可共享系统声音（自动排除 EchoRoom 自身的声音，避免回声）；
- **账号系统**：注册（邀请码）/ 登录 / 自动登录（token 24h 滑动过期）；昵称与头像存储于服务器；
- **断线自愈**：TCP 自动重连（退避 1s→2s→5s→10s）；UDP 心跳保活 + 超时后自动重新注册；
- **可裁剪的构建**：音效 / 主题 / 背景图可在打包前整体关闭（见《功能开关》一节）；
- **测试**：`cargo test --workspace`——95 个单元测试全部通过（protocol 12 / server 24 / client 59）。

## 已知限制

- **单房间（服务器即房间）、最多 6 人**——纯 UDP 转发架构的规模上限（转发带宽随人数平方增长）；
- **无回声消除（AEC）**——设计假设戴耳机使用；外放场景可能产生回声；
- **传输未加密**（UDP/TCP 明文）——仅建议在信任的小圈子里使用，未来计划 DTLS/TLS；
- **投屏共享声音仅 Windows 11**（系统级进程回路 API 限制），Windows 10 上该开关置灰；
- 视频无自适应码率（发送端手动切档）；抖动缓冲水位固定，未做弱网自适应；
- 服务器不存储任何音频/视频内容（纯转发），仅账号与头像落盘。

## 项目结构

```
protocol/               共享协议 crate：TCP/UDP 报文编解码（零第三方依赖）
server/                 服务端（无界面）
  src/main.rs           入口与参数（--data / --invite）
  src/room.rs           房间状态：成员、静音/流状态、订阅关系
  src/tcp.rs            控制通道：认证、成员管理、公屏、状态广播
  src/udp.rs            语音/视频转发：按来源地址反查成员
  src/auth.rs|db.rs     账号业务与 SQLite 持久化（argon2id 密码哈希）
  src/bin/sim_clients.rs  模拟客户端（压测/验证工具）

client/                 客户端（tauri）
  src-tauri/src/
    audio/              音频链：设备枚举、采集、降噪、DSP、抖动缓冲、混音、播放、VAD、Opus、屏幕声音
    net/                TCP 控制 + UDP 语音/视频收发
    bridge.rs           tauri 命令与事件（前后端接线）
    config.rs           本地配置（%APPDATA%\com.echoroom.dev\config.json）
    indicator.rs        投屏提示条的窗口隐藏
    bin/                验证工具：loopback_test / mic_probe / mic_record / send_wav
  ui/                   前端（纯静态 HTML/CSS/JS）
    app.js / style.css / index.html   主界面、设置页、音效
    video_capture.js / video_view.js  投屏/摄像头采集编码与观看
    features.js         功能开关（构建前配置）
    sounds/             音效包目录

docs/superpowers/      设计文档与实现计划（每个子系统均有 spec 与 plan）
scripts/               辅助脚本
```

## 声明

### 使用目的

- 本项目为**个人学习与技术研究用途**，是一个非盈利的个人项目；开发初衷是学习与实践：
  实时音频链路（采集 DSP、Opus、抖动缓冲、混音）、实时网络编程（UDP 转发、心跳保活、断线自愈）、
  Windows 音频 API（WASAPI 采集/播放/进程回路）与桌面应用（tauri）开发。
- 项目中的代码与设计文档均为学习记录，欢迎参考与交流。

### 隐私

- 语音与视频**不在服务器留存**：服务器仅做内存转发，不解码、不录音、不落盘；
- 账号数据仅保存在**你自建的服务器**上（SQLite：账号、密码哈希、昵称、头像），不经过任何第三方服务；
- 客户端本地仅保存连接配置与音量/主题等偏好（`%APPDATA%\com.echoroom.dev\`）。

### 安全提醒

- 音视频与控制消息**均为明文传输**（未加密）：适合朋友之间自建的信任环境；
  请勿将其用于传输敏感信息的场景。

### 其他

- 本软件按"现状（as-is）"提供，作者不对使用本软件产生的任何后果承担责任。

## 致谢

本项目站在众多开源工作的肩膀上，特别感谢：

- **[Tauri](https://tauri.app/)** ——应用框架：窗口、IPC、打包均基于它实现；
- **[Opus](https://opus-codec.org/) / [audiopus](https://crates.io/crates/audiopus)** ——低延迟音频编解码；
- **[wasapi](https://crates.io/crates/wasapi) / [windows-rs](https://github.com/microsoft/windows-rs)** ——
  Windows 音频（采集/播放/进程回路）与系统 API 绑定；
- **[nnnoiseless](https://crates.io/crates/nnnoiseless)** ——纯 Rust 的 RNNoise 降噪实现；
- **[rusqlite](https://crates.io/crates/rusqlite)**（SQLite）、**[argon2](https://crates.io/crates/argon2)**、
  **[rand](https://crates.io/crates/rand)** ——服务端账号存储与密码哈希；
- **Rust 生态**：`anyhow`、`serde`、`hound` 等——感谢这些库的作者与维护者。

以及所有为开源社区持续贡献代码与文档的开发者们。
