# EchoRoom 计划2 B 子项目 · Task 1 Spike 结论文档

> 日期：2026-09-19 ｜ 环境：Windows · Tauri 2 · WebView2 · windows crate 0.62.2 ｜ 执行人：Qoder（本地实测）

## A1–A6 结论表

| 编号 | 验证项 | 结论 | 证据 |
|---|---|---|---|
| A1 | WebCodecs H.264 编解码 | ✅ | 720p30 / 1080p15 / 1080p30 编码 + avc1.640028 annexb 解码全部 `isConfigSupported=true` |
| A2 | 取帧 + 窗口最小化不节流 | ✅ | 整屏采集 26–29fps 稳定；最小化期间（`is_minimized=true` 由 Rust 侧双重确认）帧率 29.1–29.5fps 零衰减，恢复后正常 |
| A3 | IPC 上行 raw body | ✅ | 100×16KB 二进制 invoke 用时 121–130ms（门槛 < 500ms） |
| A4 | IPC 下行 Channel 二进制 | ✅（修复后） | `Channel<InvokeResponseBody>` + `send(Raw(bytes))` → JS 收真 ArrayBuffer，100 条 / 1600KB 全量正确 |
| A5 | WASAPI 进程回路排除指定进程树 | ✅ | 受控实验：不排除时报警声 RMS **1779.9** → 排除后 **37.3**（低于环境底噪 72.3），报警声 100% 排除 |
| A6 | 采集格式策略 | ✅（结论明确） | **进程回路模式下 GetMixFormat 不可用**；必须手动指定 48000Hz/2ch/f32 + `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM \| AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` 自动转换 |

## 关键技术发现（Task 7 正式实现必须沿用）

### 1. GetMixFormat 在进程回路模式下不可用（严重）
- wasapi crate 文档明确记载：进程回路模式下 `get_mixformat` / `is_supported` / `get_device_period` 等不可用或返回错误值。
- windows-rs 裸调用下表现为 `STATUS_HEAP_CORRUPTION (0xC0000374)` 直接崩溃。
- **正确做法**：手动构造 `WAVEFORMATEX`（48000Hz / 2ch / 32bit float）+ Initialize 加 `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`，让系统自动转换来源格式。period 用 200_000（20ms）。

### 2. PROPVARIANT 的 Drop 陷阱（严重，windows-rs 特有）
- windows 0.62.2 在 extensions 里给 `PROPVARIANT` 实现了 `Drop` → 自动调 `PropVariantClear`。
- 对 `VT_BLOB`，`PropVariantClear` 会执行 `CoTaskMemFree(pBlobData)`；而我们的 `pBlobData` 指向**栈上的** `AUDIOCLIENT_ACTIVATION_PARAMS` → 释放栈指针 → 堆损坏。
- 表现极具迷惑性："报告点 ≠ 损坏点"（崩在退出时或任意后续堆分配处），且 **100% 复现、约 20 行最小复现可定位**。
- **修复**：`ActivateAudioInterfaceAsync` 调用后立刻 `std::mem::forget(prop)`（prop 内无堆内存，forget 是正确的）。

### 3. COM 释放顺序
- 必须先显式释放全部 COM 对象（capture → client → op → handler），**最后**才 `CoUninitialize()`；反之在已反初始化的公寓里 Release 有附加风险。

### 4. IPC 下行二进制必须用 `InvokeResponseBody::Raw`
- `Channel<Vec<u8>>::send(Vec<u8>)` 命中通用 `impl Serialize → IpcResponse`，字节被序列化成 **JSON 数字数组**（JS 收 plain Array、`byteLength=undefined`、"0KB"）。
- `Channel<InvokeResponseBody>::send(Raw(bytes))` 才走二进制：Rust 侧 ≥1KB 走 fetch 快路径（`application/octet-stream`），JS 侧 `response.arrayBuffer()` → 真 ArrayBuffer；<1KB 走 eval 直通（`new Uint8Array([...]).buffer`，也是 ArrayBuffer）。

### 5. 窗口最小化与 Page Visibility
- 反节流参数生效时，窗口最小化**不再触发** `visibilitychange`（`visibilityState` 保持 visible）。
- ⚠ 正式功能**不要**用 `visibilityState` 判断窗口是否最小化；最小化验证应以窗口状态 API（`is_minimized`）+ 帧率为准。

## 与计划文档的偏差记录

| 偏差 | 原因 | 处理 |
|---|---|---|
| probe 用 CLI 参数接收"被排除的 pid"（计划原文用自身 pid） | probe 是独立进程，必须传目标 pid；正式实现（Task 7）跑在 EchoRoom 进程内才用自身 pid | 已实现；Task 7 沿用正确语义 |
| `index.html` 整页替换为 spike 页（计划只改 script 行） | spike.js 需要按钮 DOM 结构 | 已在 Step 7 恢复为原文 |
| screen_probe 实测崩溃并修复（GetMixFormat / PROPVARIANT） | 见"关键发现"1、2 | 修复已固化在 probe 中，Task 7 必须沿用（含注释说明） |
| 新增临时 bin `probe_min.rs` | 二分定位崩溃的最小复现 | Task 11 删除 |
| probe 支持第二 CLI 参数（采集秒数，默认 8） | 调试便利（0 = 跳过读循环） | 保留于 probe |
| `lib.rs` 增加临时命令 `spike_echo` / `spike_feed` / `spike_minimize` / `spike_min_check` | IPC 验证 + 自动化节流测试 | Task 11 删除 |

## 临时文件清单（Task 11 需删除）

- `client/ui/spike.html`、`client/ui/spike.js`
- `client/src-tauri/src/bin/screen_probe.rs`、`client/src-tauri/src/bin/probe_min.rs`
- `client/src-tauri/src/lib.rs` 的 4 个 spike 命令 + `invoke_handler` 注册行
- 调试录音产物（`client/src-tauri/` 下）：`spike_probe.wav`、`r0_base.wav`、`r1_env_plus_alarm.wav`、`r2_alarm_excluded.wav`、`probe_a_excluded.wav`、`probe_b_control.wav`、`player_pid.txt`

## 正式保留项

- `client/src-tauri/tauri.conf.json`：`additionalBrowserArgs` 三个反节流参数（`--disable-background-timer-throttling` / `--disable-backgrounding-occluded-windows` / `--disable-renderer-backgrounding`）
- `client/src-tauri/Cargo.toml`：`windows 0.62.2`（features: `Win32_Media_Audio`, `Win32_System_Com`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`, `Win32_Foundation`, `Wdk_System_SystemServices`）+ `windows-core 0.62.2`

## 对后续 Task 的影响

- **Task 6/T9（IPC 链路）**：下发音频/视频帧必须使用 `Channel<InvokeResponseBody>` + `Raw`（不得用 `Channel<Vec<u8>>`）。
- **Task 7（WASAPI 采集）**：直接采用本 probe 的激活流程（含 forget(prop) 与手动格式 + AUTOCONVERTPCM 两个关键修复），排除目标 = EchoRoom 自身 pid（正式实现与采集同进程）。
- **T11（清理）**：按上方清单删除临时文件与命令。
