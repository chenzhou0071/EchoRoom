# EchoRoom 计划2 · B 子项目（投屏 + 摄像头）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 EchoRoom 加入投屏（屏幕画面 + 系统声音）与摄像头视频：WebView 内 WebCodecs 采集编码、订阅式 UDP 转发、观看视图（单路画面 / 投屏主画面 + 摄像头小窗组合）。

**Architecture:** 全 WebView + WebCodecs 路线：前端 `getDisplayMedia`/`getUserMedia` 采集 → `MediaStreamTrackProcessor` 取帧 → canvas 缩放 → `VideoEncoder`(H.264 annexb) → Tauri 二进制 IPC → Rust UDP 分片（≤1150B/片）；观看端反向（UDP 重组 → IPC Channel → `VideoDecoder` → canvas）。屏幕声音独立走 Rust WASAPI 进程回路（EXCLUDE 自身进程树）→ 立体声 Opus → UDP。服务器新增订阅表：视频/屏幕音频只转发给订阅者；`Viewers`（观看者名单/人数）驱动推流启停（无人看不推）。

**Tech Stack:** Rust（protocol / server / client；新增 `windows` crate 做进程回路）+ Tauri v2（`withGlobalTauri`）+ 原生 HTML/CSS/JS（WebCodecs / MediaStreamTrackProcessor / OffscreenCanvas，无构建工具）。

## Global Constraints

- 目标平台 Windows：Win11（build ≥ 22000）完整功能；Win10 视频可用、"共享系统声音"置灰并提示"需要 Windows 11"
- TCP 消息 type：`StreamState`=10（双向）/ `Subscribe`=11 / `Unsubscribe`=12 / `Viewers`=13（uid 名单版，见修订 R1）/ `RequestKeyframe`=14；`LoginOk.members` 变四元组 `(u16, String, bool, u8)`（第四位 = 流位图）
- UDP 包 type：6=`VideoChunk`、7=`ScreenAudio`；帧分片数据 ≤ 1150B（包总长 = 10 头 + 6 载荷头 + ≤1150 < `MAX_PACKET` 1200）
- 流类型常量：`STREAM_SCREEN=0`、`STREAM_CAMERA=1`（u8 位图 bit0/bit1）；分片标志：`VF_KEYFRAME=0x01`、`VF_LAST=0x02`
- H.264 统一 `avc1.640028` + `avc: {format:"annexb"}`（带内 SPS/PPS，无 description）；自然关键帧 2s；订阅/解码失败时请求关键帧
- 画质档位：`720p30`（1280×720@30，2Mbps）/ `1080p15`（1920×1080@15，2.5Mbps）/ `1080p30`（1920×1080@30，4Mbps）；摄像头固定 720p 档（1.5Mbps）；canvas 尺寸按首帧宽高比推导（高度取档位值，宽度偶数对齐）
- config 新字段：`share_quality`（默认 `"720p30"`）、`share_audio`（默认 `true`）；旧配置文件无此字段时 serde default 兜底
- 屏幕音频：48kHz 立体声 Opus 128kbps（`SCREEN_OPUS_BITRATE`）；播放端降混单声道后并入现有混音器（现有播放链路为"单声道信号左右同源"）
- 观看视图：单路画面高 230px 固定 + 宽按比例水平居中；双路 = 投屏主画面 + 摄像头右下角小窗（默认宽 176px，缩放范围 120–480px，比例锁死、左键拖动移位）；左上角 × 关闭、右上角 ⛶ 全屏
- 订阅为"整人级"、覆盖式（一人同时最多订阅一人）；`ViewerCount` 0→1 启动推流、1→0 停止；屏幕音频/视频编码均受此门控
- 环境：Windows PowerShell（命令用 `;` 分隔）；测试命令 `cargo test -p <包名>`；每个任务结束时全 workspace 编译通过、已有测试全绿
- 前端无构建工具（原生 JS 直接放 `client/ui/`，各文件用 IIFE 挂 `window.` 命名空间避免全局冲突）；新前端模块：`ui/video_capture.js`、`ui/video_view.js`

## File Structure（全景）

**protocol**（协议）
- `protocol/src/messages.rs`：`TcpMessage` 加 10–14 五条；`UdpPacket` 加 `VideoChunk`/`ScreenAudio`；`STREAM_SCREEN`/`STREAM_CAMERA` 常量；`LoginOk.members` 四元组
- `protocol/src/tcp.rs`、`protocol/src/udp.rs`：编解码 + 往返测试
- `protocol/src/lib.rs`：`VIDEO_CHUNK_DATA`、`SCREEN_OPUS_BITRATE` 常量

**server**（服务器）
- `server/src/room.rs`：`Member.streams`、订阅表、`set_stream`/`subscribe`/`unsubscribe`/`subscription_of`/`subscribers_with_udp`/`viewer_count`；`leave` 清理两向订阅
- `server/src/tcp.rs`：4 条 C→S 消息处理 + ViewerCount 通知 + LeaveGuard 清理
- `server/src/udp.rs`：VideoChunk/ScreenAudio 按订阅转发（Voice 转发不变）

**client / src-tauri**（客户端 Rust）
- `src/config.rs`：`share_quality`/`share_audio`
- `src/audio/opus.rs`：`OpusEncStereo`/`OpusDecStereo`
- `src/audio/screen_capture.rs`（新）：WASAPI 进程回路采集 + `is_supported()`
- `src/audio/session.rs`：`SharedAudio` 加 `my_streams`/`viewer_count`；屏幕音频编码线程 + 播放端降混叠加；`AudioHandle` 加视频收发口与屏幕 PCM 入口
- `src/net/tcp.rs`：`NetCmd` 扩展 + 新消息分发 + 重连补报流状态
- `src/net/udp.rs`：`spawn_udp_voice` → `spawn_udp`（视频分片发送/重组接收、ScreenAudio 收发、`FrameAssembler`）
- `src/bridge.rs`：视频/订阅/watch 命令与事件、屏幕捕获生命周期、`send_video_frame`
- `src/lib.rs`：AppState 扩展 + invoke 注册
- `Cargo.toml`：`windows` 依赖；`tauri.conf.json`：反节流 browser args

**client / ui**（前端）
- `ui/video_capture.js`（新）：采集 + 编码 + 上行
- `ui/video_view.js`（新）：解码 + 渲染 + 小窗 + 全屏
- `ui/app.js`：按钮/角标/纱+入口/投屏面板/事件接线
- `ui/index.html`：script 引入 + 观看视图容器
- `ui/style.css`：角标/纱/面板/观看视图样式

---

### Task 1: Spike 验证（前置门槛）

**Files:**
- Modify: `client/src-tauri/tauri.conf.json`（反节流参数，正式保留）
- Modify: `client/src-tauri/Cargo.toml`（`cargo add windows`，正式保留）
- Create: `client/ui/spike.html`、`client/ui/spike.js`（临时，Task 11 删除）
- Create: `client/src-tauri/src/bin/screen_probe.rs`（临时，Task 11 删除）
- Modify: `client/src-tauri/src/lib.rs`（临时加两个 spike 命令，Task 11 删除）
- Create: `docs/superpowers/plans/2026-09-18-echoroom-plan2-b-spike.md`（结论记录）

**Interfaces:**
- Consumes: 无（独立验证任务）
- Produces（结论直接决定后续任务的写法，若与"预期"不符按备注列中的备选走，并把调整记录进 spike 笔记）：
  - A1 编解码：`VideoEncoder.isConfigSupported("avc1.640028" + annexb)` 三档全支持。备选：换 profile 串（`avc1.4D4028`/`avc1.42E028`）或降档位表
  - A2 取帧：`MediaStreamTrackProcessor` 正常出帧，且窗口最小化后不停滞（配合反节流参数）。备选：退回 `<video>` + `setInterval` 绘 canvas
  - A3 上行 IPC：`invoke(cmd, ArrayBuffer)` 在 Rust 端以 `tauri::ipc::InvokeBody::Raw` 到达且 100×16KB < 500ms。备选：参数改 base64 字符串
  - A4 下行 IPC：`tauri::ipc::Channel<Vec<u8>>` 在 JS 侧收到 `ArrayBuffer` 且 100×16KB < 1s。备选：`Channel<String>` + base64
  - A5 WASAPI 进程回路：捕获到系统播放声、且**不含本进程**（EchoRoom）的声音（`spike_probe.wav` 人工试听）
  - A6 混音格式：`GetMixFormat` 返回 48kHz / 32-bit float / 2ch（若采样率不同，正式实现以格式实测值为准并注明）

- [ ] **Step 1: 反节流浏览器参数**

`client/src-tauri/tauri.conf.json` 的 `additionalBrowserArgs` 追加三个禁用后台节流的开关（WebView 最小化/被遮挡时画面停止是 WebCodecs 推流的头号杀手）：

```json
        "additionalBrowserArgs": "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding"
```

- [ ] **Step 2: 添加 windows 依赖**

`windows` crate 只用于进程回路（`wasapi` 高层封装不支持 Application Loopback），Task 7 正式使用，本任务先装上供 probe 编译：

```powershell
cd E:\pro\EchoRoom\client\src-tauri; cargo add windows --features "Win32_Media_Audio,Win32_System_Com,Win32_System_Com_StructuredStorage,Win32_System_Variant,Win32_Foundation,Wdk_System_SystemServices"; cargo check
```

Expected: 依赖添加成功且 `cargo check` 通过（若某 feature 名不存在，按 cargo 报错提示调整——feature 名以 crate 实际为准）。

- [ ] **Step 3: spike 页面（前端三项验证）**

`client/ui/spike.html`（新建）：

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head><meta charset="UTF-8" /><title>B spike</title></head>
<body>
  <h3>B 子项目 spike（临时页面）</h3>
  <div>
    <button id="btn-codec">1. WebCodecs 配置支持</button>
    <button id="btn-screen">2-4. 采集/取帧/编码（30s）</button>
    <button id="btn-ipc-up">5. IPC 上行 100×16KB</button>
    <button id="btn-ipc-down">6. IPC 下行 100×16KB</button>
    <button id="btn-stop">停止采集</button>
  </div>
  <pre id="log" style="white-space:pre-wrap; font-size:12px"></pre>
  <script src="spike.js"></script>
</body>
</html>
```

`client/ui/spike.js`（新建）：

```js
// B 子项目 spike：WebCodecs / TrackProcessor / IPC 通道可行性（临时，Task 11 删除）
const { invoke, Channel } = window.__TAURI__.core;
const log = (s) => { const p = document.getElementById("log"); p.textContent += s + "\n"; p.scrollTop = p.scrollHeight; };
let currentStream = null;

// 1) WebCodecs 配置支持（三档）
async function checkCodecs() {
  const cfgs = [
    { name: "720p30", width: 1280, height: 720, framerate: 30, bitrate: 2_000_000 },
    { name: "1080p15", width: 1920, height: 1080, framerate: 15, bitrate: 2_500_000 },
    { name: "1080p30", width: 1920, height: 1080, framerate: 30, bitrate: 4_000_000 },
  ];
  for (const c of cfgs) {
    const r = await VideoEncoder.isConfigSupported({
      codec: "avc1.640028", width: c.width, height: c.height,
      bitrate: c.bitrate, framerate: c.framerate,
      avc: { format: "annexb" }, latencyMode: "realtime",
    });
    log(`[编码] ${c.name}: ${r.supported ? "支持 ✓" : "不支持 ✗"}`);
  }
  const rd = await VideoDecoder.isConfigSupported({ codec: "avc1.640028", avc: { format: "annexb" } });
  log(`[解码] avc1.640028 annexb: ${rd.supported ? "支持 ✓" : "不支持 ✗"}`);
}

// 2-4) 屏幕采集 → TrackProcessor 取帧 → 编码输出统计（30s）
async function screenTrial() {
  if (currentStream) return;
  try {
    currentStream = await navigator.mediaDevices.getDisplayMedia({
      video: { frameRate: { ideal: 30 } }, audio: false,
    });
  } catch (e) { log(`[采集] getDisplayMedia 失败/取消: ${e.message}`); return; }
  const vt = currentStream.getVideoTracks();
  log(`[采集] 视频轨 ${vt.length}，settings=${JSON.stringify(vt[0]?.getSettings() ?? {})}`);
  const processor = new MediaStreamTrackProcessor({ track: vt[0] });
  const reader = processor.readable.getReader();
  const canvas = new OffscreenCanvas(1280, 720);
  const ctx = canvas.getContext("2d");
  let frames = 0, chunks = 0, bytes = 0, keyframes = 0;
  const enc = new VideoEncoder({
    output: (chunk) => { chunks++; bytes += chunk.byteLength; if (chunk.type === "key") keyframes++; },
    error: (e) => log(`[编码错误] ${e.message}`),
  });
  enc.configure({
    codec: "avc1.640028", width: 1280, height: 720, bitrate: 2_000_000, framerate: 30,
    avc: { format: "annexb" }, latencyMode: "realtime",
  });
  const t0 = performance.now();
  const timer = setInterval(() => {
    const dt = (performance.now() - t0) / 1000;
    log(`[统计] ${dt.toFixed(1)}s：取帧 ${frames}（${(frames / dt).toFixed(1)}fps）→ 编码 ${chunks} 帧（${(bytes / 1024 / dt).toFixed(0)}KB/s，关键帧 ${keyframes}）`);
  }, 2000);
  setTimeout(() => {
    clearInterval(timer);
    log("[采集] 30s 观察结束——请测试：最小化窗口 10s，看上方统计是否继续增长（A2 节流验证）");
  }, 30000);
  while (true) {
    const { done, value: frame } = await reader.read();
    if (done) { log("[采集] TrackProcessor 结束"); break; }
    frames++;
    ctx.drawImage(frame, 0, 0, 1280, 720);
    frame.close();
    const vf = new VideoFrame(canvas, { timestamp: Math.round(performance.now() * 1000) });
    enc.encode(vf, { keyFrame: frames % 60 === 1 });
    vf.close();
  }
}

// 5) IPC 上行：raw body invoke（100 × 16KB）
async function ipcUp() {
  const buf = new Uint8Array(16 * 1024);
  const t0 = performance.now();
  for (let i = 0; i < 100; i++) {
    try { await invoke("spike_echo", buf.buffer); }
    catch (e) { log(`[IPC↑] 失败（API 形态不符）: ${e}`); return; }
  }
  log(`[IPC↑] 100×16KB raw body 用时 ${(performance.now() - t0).toFixed(1)}ms`);
}

// 6) IPC 下行：Channel 二进制（100 × 16KB）
async function ipcDown() {
  let count = 0, bytes = 0, first = "?";
  const t0 = performance.now();
  const ch = new Channel((data) => {
    count++;
    if (first === "?") first = data instanceof ArrayBuffer ? "ArrayBuffer" : data?.constructor?.name ?? typeof data;
    bytes += typeof data === "string" ? data.length : (data?.byteLength ?? 0);
  });
  await invoke("spike_feed", { ch });
  await new Promise((r) => setTimeout(r, 500));
  log(`[IPC↓] 收到 ${count} 条 / ${(bytes / 1024).toFixed(0)}KB，首包类型 ${first}，用时 ${(performance.now() - t0).toFixed(1)}ms`);
}

document.getElementById("btn-codec").addEventListener("click", checkCodecs);
document.getElementById("btn-screen").addEventListener("click", screenTrial);
document.getElementById("btn-ipc-up").addEventListener("click", ipcUp);
document.getElementById("btn-ipc-down").addEventListener("click", ipcDown);
document.getElementById("btn-stop").addEventListener("click", () => {
  if (currentStream) { currentStream.getTracks().forEach((t) => t.stop()); currentStream = null; log("[采集] 已停止"); }
});
```

- [ ] **Step 4: spike Rust 命令 + screen_probe**

`client/src-tauri/src/lib.rs` 临时追加两个命令（仅 spike 用，Task 11 删除）并注册进 `invoke_handler`：

```rust
// --- 临时 spike 命令（Task 11 删除）---
#[tauri::command]
fn spike_echo(request: tauri::ipc::Request) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(b) if b.len() == 16 * 1024 => Ok(()),
        tauri::ipc::InvokeBody::Raw(_) => Err("长度不符".into()),
        _ => Err("非 raw body（invoke 二进制参数未生效）".into()),
    }
}

#[tauri::command]
fn spike_feed(ch: tauri::ipc::Channel<Vec<u8>>) -> Result<(), String> {
    let payload = vec![7u8; 16 * 1024];
    for _ in 0..100 {
        ch.send(payload.clone()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

`client/src-tauri/src/bin/screen_probe.rs`（新建；WASAPI 进程回路最小验证，8 秒 → `spike_probe.wav`）：

```rust
//! spike 用：WASAPI 进程回路捕获（排除本进程树）8 秒 → spike_probe.wav（在 src-tauri 目录运行）。
//! 验证：播放一段音乐 + 开着 EchoRoom 说话，录出的 wav 应有音乐、不应有 EchoRoom 的声音。
//! 用法：cargo run --bin screen_probe
//!
//! 注：COM 细节（implement 宏形态 / PROPVARIANT 构造 / GetBuffer 签名）以所用 windows crate
//! 版本编译提示为准；本文件的流程与 API 调用顺序与微软官方 ApplicationLoopback 示例一致。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::{implement, Interface, Result as WinResult};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::StructuredStorage::{BLOB, PROPVARIANT};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    done: Arc<AtomicBool>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler {
    fn ActivateCompleted(&self, _op: Option<&IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
        self.done.store(true, Ordering::Release);
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? }

    // 1) 激活参数：进程回路 + 排除本进程树（EchoRoom 自身的声音不会被捕获）
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams = AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
        TargetProcessId: std::process::id(),
        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    };
    let blob = BLOB {
        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pBlobData: &mut params as *mut _ as *mut u8,
    };
    let mut prop = PROPVARIANT::default();
    // VT_BLOB 写入（crate 提供 From<&BLOB> 时可直接 `(&blob).into()`）
    prop.Anonymous.Anonymous.vt = VT_BLOB;
    prop.Anonymous.Anonymous.Anonymous.blob = blob;

    // 2) 异步激活虚拟设备
    let done = Arc::new(AtomicBool::new(false));
    let handler: IActivateAudioInterfaceCompletionHandler =
        ActivationHandler { done: done.clone() }.into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop),
            &handler,
        )?
    };
    while !done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut hr = windows::core::HRESULT(0);
    let mut raw: Option<windows::core::IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut raw)? }
    hr.ok()?;
    let client: IAudioClient = raw.unwrap().cast()?;

    // 3) 混音格式 + 初始化（loopback flag）
    let mix = unsafe { client.GetMixFormat()? }; // *mut WAVEFORMATEX（需 CoTaskMemFree）
    let fmt = unsafe { *mix };
    println!(
        "[probe] 混音格式: {}Hz {}ch {}bit tag=0x{:X}",
        fmt.nSamplesPerSec, fmt.nChannels, fmt.wBitsPerSample, fmt.wFormatTag
    );
    if fmt.wFormatTag != 3 {
        // 3 = WAVE_FORMAT_IEEE_FLOAT（本 probe 按 float32 读取；非 float 请在笔记中记录实际值）
        println!("[probe] 警告：混音格式不是 IEEE float32，读循环可能是噪音");
    }
    let init = unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 200_000, 0, mix, None)
    };
    unsafe { CoTaskMemFree(Some(mix as *const _ as *const _)) }
    init?;
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    unsafe { client.Start()? }

    // 4) 读循环：f32 交错 → i16 → wav（8 秒）
    let spec = hound::WavSpec {
        channels: fmt.nChannels,
        sample_rate: fmt.nSamplesPerSec,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create("spike_probe.wav", spec)?;
    let t_end = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < t_end {
        std::thread::sleep(Duration::from_millis(10));
        loop {
            let mut packet_frames: u32 = 0;
            unsafe { capture.GetNextPacketSize(&mut packet_frames)? }
            if packet_frames == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? }
            let n = frames as usize * fmt.nChannels as usize;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data.is_null() {
                let f = unsafe { std::slice::from_raw_parts(data as *const f32, n) };
                for &x in f {
                    wav.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16)?;
                }
            } else {
                for _ in 0..n {
                    wav.write_sample(0i16)?;
                }
            }
            unsafe { capture.ReleaseBuffer(frames)? }
        }
    }
    unsafe { client.Stop()? }
    wav.finalize()?;
    println!("[probe] 完成 → spike_probe.wav");
    unsafe { CoUninitialize() }
    Ok(())
}
```

- [ ] **Step 5: 运行验证**

1. `cd E:\pro\EchoRoom\client; cargo tauri dev`；在应用窗口打开前先手动访问 spike 页面（dev 模式下改 `index.html` 临时把 `<script>` 换为 spike 页，或直接在 devtools console 跑——二选一，记录所用方式）。

   最简做法：临时把 `index.html` 的脚本行改为 `<script src="spike.js"></script>`（Task 2 恢复），页面按钮逐个点：

   - 点击"1. WebCodecs 配置支持" → 三档编码 + 解码全部"支持 ✓"（A1）
   - 点击"2-4. 采集/取帧/编码" → 系统弹出共享选择器，选一个屏或窗口 → 统计行持续增长（取帧 ≈30fps、编码产出 KB/s 在 200-300KB/s 量级）（A2/A6 二合一观察）
   - 编码运行期间**最小化窗口 10 秒**再回来看统计 → 取帧计数继续增长 = 未被节流（A2 关键点）
   - 点击"5. IPC 上行" → 输出"用时 xxxms"且无"失败"（A3）
   - 点击"6. IPC 下行" → 首包类型 ArrayBuffer、100 条全收（A4）
2. 回到 app 与房间语音同时运行的状态，`cd E:\pro\EchoRoom\client\src-tauri; cargo run --bin screen_probe`：同时播放一段音乐并在 EchoRoom 里说话 8 秒 → 试听 `spike_probe.wav`：**有音乐、无房间语音、无自己说话**（A5）。若 wav 里出现房间声音 → 记录并停：反馈给用户重新决策共享声音方案。

- [ ] **Step 6: 记录结论**

创建 `docs/superpowers/plans/2026-09-18-echoroom-plan2-b-spike.md`，按此模板逐项记录（这是后续任务的"事实依据"）：

```markdown
# B 子项目 Spike 结论（2026-09-18）

| 项 | 结论 | 备注 |
|---|---|---|
| A1 编解码 avc1.640028+annexb | ✅/❌ | 若❌：实际可用串 = ___ |
| A2 TrackProcessor + 最小化不节流 | ✅/❌ | 若❌：退路 = video+定时器 |
| A3 上行 raw body invoke | ✅/❌ | 实测 100×16KB = ___ms |
| A4 下行 Channel ArrayBuffer | ✅/❌ | 实测 100×16KB = ___ms，首包类型 = ___ |
| A5 进程回路排除自身 | ✅/❌ | wav 试听：音乐 ___ / 房间声 ___ |
| A6 混音格式 | 48k/32f/2ch? | 实测：___Hz/___bit/___ch |

## 与计划的偏差（若有）
- （列出需要修改后续任务代码的点）
```

- [ ] **Step 7: 恢复 index.html**

把 `index.html` 的脚本行恢复为 `<script src="app.js"></script>`（spike 页面文件保留，Task 11 统一删除）。

---

### Task 2: 协议扩展——TCP 流消息（type 10–14）+ members 四元组
> ⚠ 修订 R1（见文末）：`ViewerCount { n }` 升级为 `Viewers { uids: Vec<u16> }`（名单版），实现与测试按 R1 调整。

**Files:**
- Modify: `protocol/src/messages.rs`
- Modify: `protocol/src/tcp.rs`
- Modify: `server/src/room.rs`（仅类型适配：`Member.streams` 字段 + join 输出四元组）
- Modify: `client/src-tauri/src/net/tcp.rs`（仅类型适配：解构四元组）
- Modify: `client/src-tauri/src/bridge.rs`（仅类型适配：`emit_member_list` 签名）

**Interfaces:**
- Consumes: 现有 `TcpMessage`、`tcp::encode/try_decode`（`[len u32][type u8][payload]`；u16/u32 大端、String=[len u16][utf8]、bool=u8）
- Produces（后续任务依赖）：
  - `messages::STREAM_SCREEN: u8 = 0`、`messages::STREAM_CAMERA: u8 = 1`
  - `TcpMessage::StreamState { uid: u16, kind: u8, on: bool }`（type 10，双向）
  - `TcpMessage::Subscribe { uid: u16, target: u16 }`（type 11，C→S）
  - `TcpMessage::Unsubscribe { uid: u16 }`（type 12，C→S）
  - `TcpMessage::ViewerCount { n: u16 }`（type 13，S→C 定向）
  - `TcpMessage::RequestKeyframe { uid: u16, target: u16 }`（type 14，C→S→S→C）
  - `TcpMessage::LoginOk.members: Vec<(u16, String, bool, u8)>`（第四位 = 流位图）
  - `server::room::Member.streams: u8`（初始 0）
  - `Bridge::emit_member_list(Vec<(u16, String, bool, u8)>)`

- [ ] **Step 1: 写失败测试（roundtrip 扩展）**

修改 `protocol/src/tcp.rs` 的 `roundtrip_all_variants`：`LoginOk` 样本改四元组，追加 5 条新消息：

```rust
    #[test]
    fn roundtrip_all_variants() {
        let samples = vec![
            TcpMessage::Login { nickname: "阿信".into() },
            TcpMessage::LoginOk {
                uid: 3,
                token: 0xDEAD_BEEF,
                members: vec![(1, "小K".into(), false, 0), (2, "你".into(), true, 0b11)],
            },
            TcpMessage::MemberJoin { uid: 5, nickname: "新来的".into() },
            TcpMessage::MemberLeave { uid: 2 },
            TcpMessage::Chat { uid: 1, text: "晚上开黑吗".into() },
            TcpMessage::Speaking { uid: 4, on: true },
            TcpMessage::Mute { uid: 0, on: true },
            TcpMessage::Muted { uid: 4, on: true },
            TcpMessage::StreamState { uid: 0, kind: 1, on: true },
            TcpMessage::StreamState { uid: 4, kind: 0, on: false },
            TcpMessage::Subscribe { uid: 0, target: 4 },
            TcpMessage::Unsubscribe { uid: 0 },
            TcpMessage::ViewerCount { n: 3 },
            TcpMessage::RequestKeyframe { uid: 0, target: 4 },
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
Expected: 编译失败（`LoginOk` 字段类型不匹配、5 个新变体不存在）

- [ ] **Step 3: 实现消息定义**

`protocol/src/messages.rs`——加流类型常量、`TcpMessage` 新变体（放在 `Muted` 之后）、`type_id` 补 5 条、`LoginOk` 注释更新：

```rust
/// 流类型常量（u8 位图 bit0 = 屏幕 / bit1 = 摄像头）
pub const STREAM_SCREEN: u8 = 0;
pub const STREAM_CAMERA: u8 = 1;
```

```rust
    /// S→C 广播静音状态（含发送者本人）
    Muted { uid: u16, on: bool },
    /// 双向：C→S（uid 填 0）上报本端开/停某路流；S→C 广播（uid 为流主）
    StreamState { uid: u16, kind: u8, on: bool },
    /// C→S（uid 填 0）：订阅某人的视频流（覆盖式）
    Subscribe { uid: u16, target: u16 },
    /// C→S（uid 填 0）：取消订阅
    Unsubscribe { uid: u16 },
    /// S→C 定向（发给流主）：当前订阅人数
    ViewerCount { n: u16 },
    /// 双向：C→S（uid 填 0）请求目标发关键帧；S→C 转发（uid 为请求者）
    RequestKeyframe { uid: u16, target: u16 },
    /// S→C：房间满等拒绝原因
    LoginReject { reason: String },
```

`type_id` 追加（保持现有风格）：

```rust
            TcpMessage::StreamState { .. } => 10,
            TcpMessage::Subscribe { .. } => 11,
            TcpMessage::Unsubscribe { .. } => 12,
            TcpMessage::ViewerCount { .. } => 13,
            TcpMessage::RequestKeyframe { .. } => 14,
```

`LoginOk` 成员注释更新为 `(uid, 昵称, 是否静音, 流位图)`。

- [ ] **Step 4: 实现编解码**

`protocol/src/tcp.rs`——`encode` 的 `LoginOk` 分支加 streams 字节、新增 5 个分支：

```rust
        TcpMessage::LoginOk { uid, token, members } => {
            payload.extend_from_slice(&uid.to_be_bytes());
            payload.extend_from_slice(&token.to_be_bytes());
            payload.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for (uid, name, muted, streams) in members {
                payload.extend_from_slice(&uid.to_be_bytes());
                put_str(&mut payload, name);
                payload.push(if *muted { 1 } else { 0 });
                payload.push(*streams);
            }
        }
```

```rust
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
        TcpMessage::ViewerCount { n } => payload.extend_from_slice(&n.to_be_bytes()),
```

`try_decode`——type 2 成员循环加 `streams`、新增 5 个分支：

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
                let streams = field!(r.u8());
                members.push((m_uid, name, muted, streams));
            }
            TcpMessage::LoginOk { uid, token, members }
        }
```

```rust
        10 => {
            let uid = field!(r.u16());
            let kind = field!(r.u8());
            TcpMessage::StreamState { uid, kind, on: field!(r.u8()) != 0 }
        }
        11 => {
            let uid = field!(r.u16());
            TcpMessage::Subscribe { uid, target: field!(r.u16()) }
        }
        12 => TcpMessage::Unsubscribe { uid: field!(r.u16()) },
        13 => TcpMessage::ViewerCount { n: field!(r.u16()) },
        14 => {
            let uid = field!(r.u16());
            TcpMessage::RequestKeyframe { uid, target: field!(r.u16()) }
        }
```

- [ ] **Step 5: 全 workspace 类型适配**

`server/src/room.rs`：
- `Member` 加字段（放 `muted` 之后）：
  ```rust
      /// 流位图：bit0 = 投屏（屏幕）、bit1 = 摄像头
      pub streams: u8,
  ```
- `join()` 内 `members` 映射加第四位、`Member` 初始化加 `streams: 0`：
  ```rust
        let members: Vec<(u16, String, bool, u8)> = self
            .members
            .values()
            .map(|m| (m.uid, m.nickname.clone(), m.muted, m.streams))
            .collect();
  ```
  ```rust
                muted: false,
                streams: 0,
                tx,
  ```
- `JoinOk.members` 类型同步改 `Vec<(u16, String, bool, u8)>`；模块内两个测试的断言补第四位：
  - `join_returns_existing_members_and_leaves_removes`：`assert_eq!(ok2.members, vec![(ok1.uid, "A".to_string(), false, 0)]);`
  - `set_muted_updates_member_and_join_reports_it`：`assert_eq!(b.members, vec![(a.uid, "A".to_string(), true, 0)]);`

`client/src-tauri/src/net/tcp.rs`：
- `uid_names` 重建循环改 `for (u, n, _, _) in &members`
- `all.push((uid, nickname.to_string(), my_muted))` 改为 `all.push((uid, nickname.to_string(), my_muted, 0))`（重连补报流状态在 Task 5 实现）

`client/src-tauri/src/bridge.rs`：`emit_member_list` 参数改 `Vec<(u16, String, bool, u8)>`。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p echoroom-protocol -p echoroom-server`
Expected: 全绿（含四元组 roundtrip 与 server room 断言）

Run: `cargo build`（workspace 根）
Expected: 全 workspace 编译通过

---

### Task 3: 协议扩展——UDP 视频/屏幕音频包（type 6–7）

**Files:**
- Modify: `protocol/src/messages.rs`
- Modify: `protocol/src/udp.rs`
- Modify: `protocol/src/lib.rs`

**Interfaces:**
- Consumes: 现有 `UdpPacket`、`udp::encode/decode`（10 字节头：magic 0xEC45 / ver 1 / type / uid u16 / seq u32，大端；`MAX_PACKET=1200`）
- Produces（后续任务依赖）：
  - `udp::VF_KEYFRAME: u8 = 0x01`、`udp::VF_LAST: u8 = 0x02`
  - `UdpPacket::VideoChunk { kind: u8, flags: u8, frame_seq: u16, chunk_idx: u8, chunk_count: u8, data: Vec<u8> }`（type 6）
  - `UdpPacket::ScreenAudio { opus: Vec<u8> }`（type 7）
  - `lib::VIDEO_CHUNK_DATA: usize = 1150`、`lib::SCREEN_OPUS_BITRATE: i32 = 128_000`

- [ ] **Step 1: 写失败测试**

`protocol/src/udp.rs` 的 `roundtrip_all_variants` 追加两个样本，并新增一个"满片不越界"测试：

```rust
            UdpPacket::VideoChunk {
                kind: 0,
                flags: 0x03,
                frame_seq: 65535,
                chunk_idx: 2,
                chunk_count: 5,
                data: vec![0x5A; 1150],
            },
            UdpPacket::ScreenAudio { opus: vec![0xEE; 320] },
```

```rust
    #[test]
    fn video_chunk_max_size_within_packet_limit() {
        let bytes = encode(
            1,
            0,
            &UdpPacket::VideoChunk {
                kind: 1,
                flags: 0,
                frame_seq: 0,
                chunk_idx: 0,
                chunk_count: 1,
                data: vec![0u8; crate::VIDEO_CHUNK_DATA],
            },
        );
        assert!(bytes.len() <= MAX_PACKET, "len={} 超上限", bytes.len());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-protocol`
Expected: 编译失败（`VideoChunk`/`ScreenAudio` 变体与 `VIDEO_CHUNK_DATA` 不存在）

- [ ] **Step 3: 实现消息定义与常量**

`protocol/src/messages.rs`——`UdpPacket`（现有 `Voice`/`Heartbeat` 之后）加两个变体 + `type_id`：

```rust
    /// 双向：视频分片（kind = STREAM_*；flags bit0 = 关键帧、bit1 = 末片；按 frame_seq + chunk_idx 重组）
    VideoChunk { kind: u8, flags: u8, frame_seq: u16, chunk_idx: u8, chunk_count: u8, data: Vec<u8> },
    /// S→C：屏幕声音 Opus 包（48kHz 立体声，20ms）
    ScreenAudio { opus: Vec<u8> },
```

```rust
            UdpPacket::VideoChunk { .. } => 6,
            UdpPacket::ScreenAudio { .. } => 7,
```

`protocol/src/lib.rs`——追加常量：

```rust
/// 视频分片数据上限（包总长 = 10 字节头 + 6 字节载荷头 + 数据 ≤ 1166 < MAX_PACKET）
pub const VIDEO_CHUNK_DATA: usize = 1150;
/// 屏幕声音 Opus 码率（立体声）
pub const SCREEN_OPUS_BITRATE: i32 = 128_000;
```

- [ ] **Step 4: 实现编解码**

`protocol/src/udp.rs`——加标志常量 + `encode` 两个分支 + `decode` 两个分支：

```rust
/// 视频分片标志：bit0 = 关键帧（IDR）、bit1 = 该帧末片
pub const VF_KEYFRAME: u8 = 0x01;
pub const VF_LAST: u8 = 0x02;
```

```rust
        UdpPacket::VideoChunk { kind, flags, frame_seq, chunk_idx, chunk_count, data } => {
            out.push(*kind);
            out.push(*flags);
            out.extend_from_slice(&frame_seq.to_be_bytes());
            out.push(*chunk_idx);
            out.push(*chunk_count);
            out.extend_from_slice(data);
        }
        UdpPacket::ScreenAudio { opus } => out.extend_from_slice(opus),
```

```rust
        6 => {
            if payload.len() < 6 {
                return Err(DecodeError::TooShort);
            }
            UdpPacket::VideoChunk {
                kind: payload[0],
                flags: payload[1],
                frame_seq: u16::from_be_bytes([payload[2], payload[3]]),
                chunk_idx: payload[4],
                chunk_count: payload[5],
                data: payload[6..].to_vec(),
            }
        }
        7 => UdpPacket::ScreenAudio { opus: payload.to_vec() },
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p echoroom-protocol`
Expected: 全绿（含新样本 roundtrip 与满片尺寸测试）

---

### Task 4: 服务器——订阅表与流状态
> ⚠ 修订 R1（见文末）：ViewerCount 发送点全部改为 `Viewers`（发流主+该流全部订阅者）；新增 `Room::viewers_of`。

**Files:**
- Modify: `server/src/room.rs`
- Modify: `server/src/tcp.rs`
- Modify: `server/src/udp.rs`

**Interfaces:**
- Consumes: Task 2/3 的协议定义；现有 `Room::broadcast/send_to/leave/member_by_addr/touch`
- Produces（后续任务依赖）：
  - `Room::set_stream(uid: u16, kind: u8, on: bool) -> Option<u8>`（返回新位图；成员不存在 → None）
  - `Room::subscribe(sub: u16, target: u16) -> bool`（覆盖式；目标不存在或订阅自己 → false）
  - `Room::unsubscribe(sub: u16) -> bool`
  - `Room::subscription_of(sub: u16) -> Option<u16>`
  - `Room::subscribers_with_udp(target: u16) -> Vec<SocketAddr>`
  - `Room::viewer_count(target: u16) -> usize`
  - `Room::leave` 语义扩展：同时清理该 uid 的两向订阅记录

- [ ] **Step 1: 写失败测试（room 三个新测试 + 适配）**

在 `server/src/room.rs` 测试模块顶部补 `STREAM_SCREEN/STREAM_CAMERA` 到 use，并追加三个测试：

```rust
    use echoroom_protocol::messages::{TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
```

```rust
    #[test]
    fn stream_bitmap_set_and_join_reports_it() {
        let mut room = Room::new();
        let (tx1, _r1) = mpsc::channel();
        let a = room.join("A".into(), tx1).unwrap();
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, true), Some(1));
        assert_eq!(room.set_stream(a.uid, STREAM_CAMERA, true), Some(3));
        assert_eq!(room.set_stream(a.uid, STREAM_SCREEN, false), Some(2));
        assert_eq!(room.set_stream(999, STREAM_SCREEN, true), None, "成员不存在");
        let (tx2, _r2) = mpsc::channel();
        let b = room.join("B".into(), tx2).unwrap();
        assert_eq!(b.members, vec![(a.uid, "A".to_string(), false, 2)]);
    }

    #[test]
    fn subscribe_unsubscribe_and_viewer_count() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), txa).unwrap();
        let b = room.join("B".into(), txb).unwrap();
        let addr_b: std::net::SocketAddr = "127.0.0.1:6001".parse().unwrap();
        room.set_udp(b.uid, addr_b);
        assert!(!room.subscribe(b.uid, b.uid), "不能订阅自己");
        assert!(!room.subscribe(b.uid, 999), "目标不存在");
        assert!(room.subscribe(b.uid, a.uid));
        assert_eq!(room.viewer_count(a.uid), 1);
        assert_eq!(room.subscription_of(b.uid), Some(a.uid));
        assert_eq!(room.subscribers_with_udp(a.uid), vec![addr_b]);
        // 覆盖式：C 订阅 A 后再改订阅 B
        let (txc, _rc) = mpsc::channel();
        let c = room.join("C".into(), txc).unwrap();
        assert!(room.subscribe(c.uid, a.uid));
        assert_eq!(room.viewer_count(a.uid), 1);
        assert!(room.subscribe(c.uid, b.uid));
        assert_eq!(room.viewer_count(a.uid), 0);
        assert_eq!(room.viewer_count(b.uid), 1);
        assert!(room.unsubscribe(c.uid));
        assert!(!room.unsubscribe(c.uid), "重复退订为 false");
        assert_eq!(room.viewer_count(b.uid), 0);
    }

    #[test]
    fn leave_cleans_subscriptions_both_ways() {
        let mut room = Room::new();
        let (txa, _ra) = mpsc::channel();
        let (txb, _rb) = mpsc::channel();
        let a = room.join("A".into(), txa).unwrap();
        let b = room.join("B".into(), txb).unwrap();
        room.subscribe(b.uid, a.uid); // B 看 A
        room.subscribe(a.uid, b.uid); // A 看 B（互看）
        room.leave(b.uid);
        assert_eq!(room.viewer_count(a.uid), 0, "B 离开后 A 的观众数清零");
        assert_eq!(room.subscription_of(a.uid), None, "A 看 B 的订阅记录被清");
        assert!(!room.unsubscribe(a.uid));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-server`
Expected: 编译失败（`set_stream`/`subscribe` 等方法不存在）

- [ ] **Step 3: 实现 Room 数据层**

`server/src/room.rs`——`Room` 结构加订阅表（`rng_state` 之后）：

```rust
    /// 订阅表：订阅者 uid → 目标 uid（一人最多订阅一人，覆盖式）
    subscriptions: HashMap<u16, u16>,
```

`Room::new()` 初始化加 `subscriptions: HashMap::new(),`。

`impl Room` 内（`others_with_udp` 之后）追加方法：

```rust
    /// 设置某路流的开/停；返回新位图（成员不存在 → None）
    pub fn set_stream(&mut self, uid: u16, kind: u8, on: bool) -> Option<u8> {
        let m = self.members.get_mut(&uid)?;
        let bit = 1u8 << kind;
        if on {
            m.streams |= bit;
        } else {
            m.streams &= !bit;
        }
        Some(m.streams)
    }

    /// 订阅（覆盖式：直接改写映射）；目标不存在或订阅自己 → false
    pub fn subscribe(&mut self, sub: u16, target: u16) -> bool {
        if sub == target || !self.members.contains_key(&target) {
            return false;
        }
        self.subscriptions.insert(sub, target);
        true
    }

    /// 解除订阅；返回是否存在被解除的订阅
    pub fn unsubscribe(&mut self, sub: u16) -> bool {
        self.subscriptions.remove(&sub).is_some()
    }

    /// 订阅者 → 目标（离开清理 / 覆盖切换通知用）
    pub fn subscription_of(&self, sub: u16) -> Option<u16> {
        self.subscriptions.get(&sub).copied()
    }

    /// 目标的订阅者中已注册 UDP 地址的列表（视频/屏幕音频的转发目标）
    pub fn subscribers_with_udp(&self, target: u16) -> Vec<SocketAddr> {
        self.subscriptions
            .iter()
            .filter(|(_, t)| **t == target)
            .filter_map(|(s, _)| self.members.get(s)?.udp_addr)
            .collect()
    }

    /// 目标的订阅人数
    pub fn viewer_count(&self, target: u16) -> usize {
        self.subscriptions.values().filter(|t| **t == target).count()
    }
```

`leave()` 改为同时清理两向订阅（uid 作为订阅者、作为目标）：

```rust
    pub fn leave(&mut self, uid: u16) -> bool {
        self.subscriptions.remove(&uid); // 他看别人的记录
        self.subscriptions.retain(|_, t| *t != uid); // 别人看他的记录
        self.members.remove(&uid).is_some()
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p echoroom-server`
Expected: 全绿（含新增三测试；旧测试 `join_until_capacity_then_reject` 等不受影响）

- [ ] **Step 5: TCP 接线（消息处理 + LeaveGuard）**

`server/src/tcp.rs` 读循环 match 中，`TcpMessage::Mute` 分支之后追加四个分支：

```rust
                        TcpMessage::StreamState { kind, on, .. } => {
                            let mut room = room.lock().unwrap();
                            if room.set_stream(ok.uid, kind, on).is_some() {
                                room.broadcast(None, &TcpMessage::StreamState { uid: ok.uid, kind, on });
                                // 把当前观众数直接告知流主：0 = 先不编码，1 起才开始推
                                let n = room.viewer_count(ok.uid) as u16;
                                room.send_to(ok.uid, &TcpMessage::ViewerCount { n });
                            }
                        }
                        TcpMessage::Subscribe { target, .. } => {
                            let mut room = room.lock().unwrap();
                            // 覆盖式：切换订阅前先让旧目标的人数回落
                            if let Some(old) = room.subscription_of(ok.uid) {
                                room.unsubscribe(ok.uid);
                                let n = room.viewer_count(old) as u16;
                                room.send_to(old, &TcpMessage::ViewerCount { n });
                            }
                            if room.subscribe(ok.uid, target) {
                                let n = room.viewer_count(target) as u16;
                                room.send_to(target, &TcpMessage::ViewerCount { n });
                            }
                        }
                        TcpMessage::Unsubscribe { .. } => {
                            let mut room = room.lock().unwrap();
                            if let Some(old) = room.subscription_of(ok.uid) {
                                room.unsubscribe(ok.uid);
                                let n = room.viewer_count(old) as u16;
                                room.send_to(old, &TcpMessage::ViewerCount { n });
                            }
                        }
                        TcpMessage::RequestKeyframe { target, .. } => {
                            let room = room.lock().unwrap();
                            // 转发给目标（uid 替换为请求者，供流主识别）
                            room.send_to(target, &TcpMessage::RequestKeyframe { uid: ok.uid, target });
                        }
```

`LeaveGuard::drop` 改为清理后通知相关目标：

```rust
impl Drop for LeaveGuard {
    fn drop(&mut self) {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner()); // 锁 poisoned 时也尽力清理
        // 离开者若正在观看别人：先取出目标 uid，离开后通知该目标的人数回落
        let watching = room.subscription_of(self.uid);
        if room.leave(self.uid) {
            room.broadcast(None, &TcpMessage::MemberLeave { uid: self.uid });
            println!("[tcp] {}(uid={}) 离开", self.nickname, self.uid);
        }
        if let Some(target) = watching {
            let n = room.viewer_count(target) as u16;
            room.send_to(target, &TcpMessage::ViewerCount { n });
        }
    }
}
```

- [ ] **Step 6: UDP 代理（按订阅转发）**

`server/src/udp.rs` 的 match 中，`UdpPacket::Voice` 分支之后追加两个分支（转发模式与 Voice 相同，只是目标换成订阅者）：

```rust
                UdpPacket::VideoChunk { kind, flags, frame_seq, chunk_idx, chunk_count, data } => {
                    let (real_uid, targets) = {
                        let mut room = room.lock().unwrap();
                        let Some(real_uid) = room.member_by_addr(from) else {
                            continue;
                        };
                        room.touch(real_uid);
                        (real_uid, room.subscribers_with_udp(real_uid))
                    };
                    // 只发给订阅者；uid 用反查值（防伪造）
                    let out = udp::encode(
                        real_uid,
                        frame_seq as u32,
                        &UdpPacket::VideoChunk { kind, flags, frame_seq, chunk_idx, chunk_count, data },
                    );
                    for t in targets {
                        let _ = socket.send_to(&out, t);
                    }
                }
                UdpPacket::ScreenAudio { opus } => {
                    let (real_uid, targets) = {
                        let mut room = room.lock().unwrap();
                        let Some(real_uid) = room.member_by_addr(from) else {
                            continue;
                        };
                        room.touch(real_uid);
                        (real_uid, room.subscribers_with_udp(real_uid))
                    };
                    let out = udp::encode(real_uid, 0, &UdpPacket::ScreenAudio { opus });
                    for t in targets {
                        let _ = socket.send_to(&out, t);
                    }
                }
```

- [ ] **Step 7: 编译与全量测试**

Run: `cargo build; cargo test -p echoroom-server -p echoroom-protocol`
Expected: 编译通过、测试全绿

---

### Task 5: 客户端 TCP 接线 + 共享状态基建
> ⚠ 修订 R1（见文末）：收到 `Viewers { uids }` → `viewer_count = uids.len()` + emit `viewer_count`（payload = uid 数组）。

**Files:**
- Modify: `client/src-tauri/src/net/tcp.rs`
- Modify: `client/src-tauri/src/audio/session.rs`（仅 `SharedAudio` 加两字段）
- Modify: `client/src-tauri/src/bridge.rs`
- Modify: `client/src-tauri/src/lib.rs`（invoke 注册）

**Interfaces:**
- Consumes: Task 2 的协议消息；`SharedAudio`（`Arc` 原子/锁容器，`Clone` 共享）；`NetHandle.tx: Sender<NetCmd>`；现有 `Bridge` emit 风格
- Produces（后续任务依赖）：
  - `NetCmd::Subscribe(Option<u16>)`（Some=订阅、None=退订）
  - `NetCmd::RequestKeyframe(u16)`
  - `NetCmd::SetStream { kind: u8, on: bool }`
  - `SharedAudio.my_streams: Arc<AtomicU8>`（本端流位图；LoginOk 重连时按它补报）
  - `SharedAudio.viewer_count: Arc<AtomicU16>`（LoginOk 归零；收到 ViewerCount 更新）
  - `Bridge::emit_stream_state(uid, kind, on)` / `emit_viewer_count(n)` / `emit_request_keyframe()`
  - invoke 命令：`subscribe(target: Option<u16>)`、`report_stream(kind, on)`、`request_keyframe(target)`

- [ ] **Step 1: NetCmd 扩展 + 发送分支**

`client/src-tauri/src/net/tcp.rs` 顶部 use 改：

```rust
use echoroom_protocol::messages::{TcpMessage, STREAM_CAMERA, STREAM_SCREEN};
```

`NetCmd` 扩展（`SetMuted` 之后、`Shutdown` 之前）：

```rust
    /// 订阅某人（None = 取消订阅）
    Subscribe(Option<u16>),
    /// 请求目标发关键帧
    RequestKeyframe(u16),
    /// 上报本端某路流开/停（kind = STREAM_*）
    SetStream { kind: u8, on: bool },
```

`run_session` 发送轮询 match 中 `SetMuted` 分支之后追加：

```rust
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
```

- [ ] **Step 2: 接收分支 + 重连补报**

`run_session` 接收 match 中 `TcpMessage::Muted` 分支之后追加：

```rust
                        TcpMessage::StreamState { uid, kind, on } => bridge.emit_stream_state(uid, kind, on),
                        TcpMessage::ViewerCount { n } => {
                            shared.viewer_count.store(n, Ordering::Relaxed);
                            bridge.emit_viewer_count(n);
                        }
                        TcpMessage::RequestKeyframe { .. } => bridge.emit_request_keyframe(),
```

`LoginOk` 处理块内——在"重连后若本地处于静音，向新会话重新声明"之后追加：

```rust
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
```

（Task 2 在此处补的 `all.push((uid, nickname.to_string(), my_muted, 0))` 保持不变：成员列表里自己的流位图由前端以 `stream_state` 事件为准。）

- [ ] **Step 3: SharedAudio 加字段**

`client/src-tauri/src/audio/session.rs`：

顶部 use 补 `AtomicU8, AtomicU16`（合并进现有 `use std::sync::atomic::{...}` 行）。

`SharedAudio` 结构体（`uid_names` 之后）：

```rust
    /// 本端流位图镜像（bit0 = 投屏、bit1 = 摄像头；重连补报用）
    pub my_streams: Arc<AtomicU8>,
    /// 当前订阅人数（屏幕音频推流门控；LoginOk 归零）
    pub viewer_count: Arc<AtomicU16>,
```

`SharedAudio::new()` 里补初始化：

```rust
            my_streams: Arc::new(AtomicU8::new(0)),
            viewer_count: Arc::new(AtomicU16::new(0)),
```

- [ ] **Step 4: Bridge 事件与命令**

`client/src-tauri/src/bridge.rs`——`Bridge` impl 内追加三个 emit：

```rust
    pub fn emit_stream_state(&self, uid: u16, kind: u8, on: bool) {
        let _ = self.app.emit("stream_state", serde_json::json!({ "uid": uid, "kind": kind, "on": on }));
    }
    pub fn emit_viewer_count(&self, n: u16) {
        let _ = self.app.emit("viewer_count", n);
    }
    pub fn emit_request_keyframe(&self) {
        let _ = self.app.emit("request_keyframe", ());
    }
```

文件末尾（`start_audio` 之前或之后）追加三个命令：

```rust
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
```

- [ ] **Step 5: invoke 注册**

`client/src-tauri/src/lib.rs` 的 `invoke_handler` 列表追加：

```rust
            bridge::subscribe,
            bridge::report_stream,
            bridge::request_keyframe,
```

- [ ] **Step 6: 编译与全量测试**

Run: `cargo build; cargo test`
Expected: 全 workspace 编译通过、现有测试全绿

---

### Task 6: 客户端 UDP 传输——视频分片/重组 + 屏幕音频收发

**Files:**
- Modify: `client/src-tauri/src/net/udp.rs`
- Modify: `client/src-tauri/src/audio/session.rs`（调用处适配 + `AudioHandle` 加字段）

**Interfaces:**
- Consumes: Task 3 的 `VideoChunk`/`ScreenAudio`/`VF_KEYFRAME`/`VF_LAST`/`VIDEO_CHUNK_DATA`；现有 `spawn_udp_voice` 的线程结构（`sock.try_clone` 多发送线程、单接收线程）
- Produces（后续任务依赖）：
  - `net::udp::VideoOut { kind: u8, keyframe: bool, data: Vec<u8> }`（送出一帧）
  - `net::udp::VideoIn { uid: u16, kind: u8, keyframe: bool, data: Vec<u8> }`（重组完成帧）
  - `net::udp::UdpTx { tx_pcm, tx_video, tx_screen_audio }`（三个 `SyncSender`）
  - `net::udp::UdpRx { rx_voice, rx_video, rx_screen_audio }`
  - `net::udp::spawn_udp(server_addr: String, uid: u16, token: u32, stop: Arc<AtomicBool>) -> anyhow::Result<(UdpTx, UdpRx)>`（原 `spawn_udp_voice` 更名，签名不变）
  - `net::udp::FrameAssembler::push(kind, flags, seq, idx, count, data) -> Option<(u8, bool, Vec<u8>)>`
  - `AudioHandle.video_tx: SyncSender<VideoOut>`、`AudioHandle.video_rx: Arc<Mutex<Receiver<VideoIn>>>`

- [ ] **Step 1: 写 FrameAssembler 失败测试**

`client/src-tauri/src/net/udp.rs` 文件末尾新建测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_in_order() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 7, 0, 3, b"aa").is_none());
        assert!(a.push(0, 0, 7, 1, 3, b"bb").is_none());
        let (kind, key, data) = a.push(0, VF_KEYFRAME | VF_LAST, 7, 2, 3, b"cc").unwrap();
        assert_eq!((kind, key), (0, true));
        assert_eq!(data, b"aabbcc");
    }

    #[test]
    fn assembles_out_of_order() {
        let mut a = FrameAssembler::new();
        assert!(a.push(1, VF_KEYFRAME, 9, 2, 3, b"cc").is_none());
        assert!(a.push(1, VF_KEYFRAME, 9, 0, 3, b"aa").is_none());
        let (kind, key, data) = a.push(1, VF_KEYFRAME, 9, 1, 3, b"bb").unwrap();
        assert_eq!((kind, key), (1, true));
        assert_eq!(data, b"aabbcc");
    }

    #[test]
    fn drops_incomplete_frame_on_new_seq() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 1, 0, 3, b"x").is_none()); // seq=1 缺片
        assert!(a.push(0, 0, 2, 0, 2, b"y").is_none()); // seq=2 开始 → seq=1 被丢弃
        let (_, key, data) = a.push(0, 0, 2, 1, 2, b"z").unwrap();
        assert!(!key);
        assert_eq!(data, b"yz");
    }

    #[test]
    fn ignores_invalid_chunk() {
        let mut a = FrameAssembler::new();
        assert!(a.push(0, 0, 1, 0, 0, b"x").is_none(), "count=0");
        assert!(a.push(0, 0, 1, 3, 3, b"x").is_none(), "idx>=count");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-client`
Expected: 编译失败（`FrameAssembler` 不存在）

- [ ] **Step 3: 实现类型 + FrameAssembler + spawn_udp**

`client/src-tauri/src/net/udp.rs` 全文件改造：

顶部注释与 use 更新：

```rust
//! UDP 通道：语音 / 视频分片 / 屏幕音频的收发线程与视频帧重组。
//! 服务器按来源地址反查 uid，且只转发给订阅者（server/src/udp.rs），本模块只负责收发与注册维持。
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use echoroom_protocol::messages::UdpPacket;
use echoroom_protocol::udp::{self, VF_KEYFRAME, VF_LAST};
use echoroom_protocol::{FRAME_SAMPLES, HEARTBEAT_INTERVAL_MS, VIDEO_CHUNK_DATA};
```

类型定义与重组器（文件前部，`spawn_udp` 之前）：

```rust
/// 视频送出一帧（前端编码产物 → UDP 分片）
pub struct VideoOut {
    pub kind: u8,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// 重组完成的一帧（UDP → 前端解码）
pub struct VideoIn {
    pub uid: u16,
    pub kind: u8,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// UDP 送入口集合
pub struct UdpTx {
    /// 语音 PCM（960 样本块）
    pub tx_pcm: SyncSender<Vec<i16>>,
    /// 视频帧
    pub tx_video: SyncSender<VideoOut>,
    /// 屏幕音频 Opus 包
    pub tx_screen_audio: SyncSender<Vec<u8>>,
}

/// UDP 出口集合
pub struct UdpRx {
    /// 语音：(from_uid, seq, opus)
    pub rx_voice: Receiver<(u16, u32, Vec<u8>)>,
    /// 视频：已重组完整帧
    pub rx_video: Receiver<VideoIn>,
    /// 屏幕音频：(from_uid, opus)
    pub rx_screen_audio: Receiver<(u16, Vec<u8>)>,
}

/// 视频帧重组器：按 frame_seq 收集分片，凑齐即产出完整帧。
/// 策略：新 frame_seq 到达即弃掉未完成的旧帧（丢片丢整帧，不重传）。
pub struct FrameAssembler {
    cur: Option<Partial>,
}

struct Partial {
    seq: u16,
    kind: u8,
    keyframe: bool,
    chunks: Vec<Option<Vec<u8>>>,
    have: usize,
}

impl FrameAssembler {
    pub fn new() -> Self {
        FrameAssembler { cur: None }
    }

    /// 送入一个分片；若凑齐当前帧则返回 `(kind, keyframe, 完整 annexb 数据)`
    pub fn push(
        &mut self,
        kind: u8,
        flags: u8,
        seq: u16,
        idx: u8,
        count: u8,
        data: &[u8],
    ) -> Option<(u8, bool, Vec<u8>)> {
        if count == 0 || idx >= count {
            return None; // 非法分片
        }
        if self.cur.as_ref().map(|p| p.seq != seq).unwrap_or(true) {
            // 新帧：丢弃未完成的旧帧
            self.cur = Some(Partial {
                seq,
                kind,
                keyframe: flags & VF_KEYFRAME != 0,
                chunks: vec![None; count as usize],
                have: 0,
            });
        }
        let p = self.cur.as_mut().unwrap();
        if p.chunks.len() != count as usize {
            self.cur = None; // 同 seq 片数矛盾：整帧放弃
            return None;
        }
        let slot = &mut p.chunks[idx as usize];
        if slot.is_none() {
            *slot = Some(data.to_vec());
            p.have += 1;
        }
        if p.have == p.chunks.len() {
            let p = self.cur.take().unwrap();
            let mut out = Vec::new();
            for c in p.chunks {
                out.extend_from_slice(&c.unwrap());
            }
            return Some((p.kind, p.keyframe, out));
        }
        None
    }
}
```

`spawn_udp_voice` 更名与扩展——函数签名与开头：

```rust
/// 启动 UDP 收发线程组（语音 + 视频分片 + 屏幕音频）。
///
/// - `stop`：与音频管线共享的停止旗标
/// - 返回 `(送入口集合, 出口集合)`
pub fn spawn_udp(
    server_addr: String,
    uid: u16,
    token: u32,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<(UdpTx, UdpRx)> {
    let (tx_pcm, rx_pcm) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
    let (tx_voice, rx_voice) = std::sync::mpsc::sync_channel::<(u16, u32, Vec<u8>)>(256);
    let (tx_video, rx_video) = std::sync::mpsc::sync_channel::<VideoOut>(8);
    let (vod_tx, vod_rx) = std::sync::mpsc::sync_channel::<VideoIn>(8);
    let (tx_screen_audio, rx_screen_audio) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
    let (sa_tx, sa_rx) = std::sync::mpsc::sync_channel::<(u16, Vec<u8>)>(64);

    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(&server_addr)?; // connect 后 send/recv 只面向服务器
    let sock_send = sock.try_clone()?;
```

原有语音发送线程保持不变（`rx_pcm` → `OpusEnc` → `UdpPacket::Voice`）。其后追加视频发送线程与屏幕音频发送线程：

```rust
    // 视频发送线程：帧 → 分片 → UDP（每片带 kind/flags/frame_seq[idx]/count）
    {
        let stop = stop.clone();
        let sock = sock_send.try_clone()?;
        std::thread::spawn(move || {
            let mut frame_seq: u16 = 0;
            while !stop.load(Ordering::Relaxed) {
                let f = match rx_video.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if f.data.is_empty() {
                    continue;
                }
                let count = f.data.len().div_ceil(VIDEO_CHUNK_DATA);
                if count > 255 {
                    eprintln!("[udp] 帧过大（{}B，{} 片）丢弃", f.data.len(), count);
                    continue;
                }
                for (i, chunk) in f.data.chunks(VIDEO_CHUNK_DATA).enumerate() {
                    let mut flags = 0u8;
                    if f.keyframe {
                        flags |= VF_KEYFRAME;
                    }
                    if i + 1 == count {
                        flags |= VF_LAST;
                    }
                    let pkt = udp::encode(
                        uid,
                        frame_seq as u32,
                        &UdpPacket::VideoChunk {
                            kind: f.kind,
                            flags,
                            frame_seq,
                            chunk_idx: i as u8,
                            chunk_count: count as u8,
                            data: chunk.to_vec(),
                        },
                    );
                    let _ = sock.send(&pkt);
                }
                frame_seq = frame_seq.wrapping_add(1);
            }
        });
    }

    // 屏幕音频发送线程：Opus 包 → UDP
    {
        let stop = stop.clone();
        let sock = sock_send.try_clone()?;
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let opus = match rx_screen_audio.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let pkt = udp::encode(uid, 0, &UdpPacket::ScreenAudio { opus });
                let _ = sock.send(&pkt);
            }
        });
    }
```

接收线程的 match 追加两个分支（`Voice` 与 `RegisterAck` 之间），循环外先建重组器：

```rust
            let mut va = FrameAssembler::new();
```

```rust
                        Ok((from_uid, _, UdpPacket::VideoChunk { kind, flags, frame_seq, chunk_idx, chunk_count, data })) => {
                            if let Some((k, key, full)) =
                                va.push(kind, flags, frame_seq, chunk_idx, chunk_count, &data)
                            {
                                let _ = vod_tx.try_send(VideoIn { uid: from_uid, kind: k, keyframe: key, data: full });
                            }
                        }
                        Ok((from_uid, _, UdpPacket::ScreenAudio { opus })) => {
                            let _ = sa_tx.try_send((from_uid, opus));
                        }
```

函数末尾返回（替换原 `Ok((tx_pcm, rx_voice))`）：

```rust
    Ok((
        UdpTx { tx_pcm, tx_video, tx_screen_audio },
        UdpRx { rx_voice, rx_video: vod_rx, rx_screen_audio: sa_rx },
    ))
```

- [ ] **Step 4: session.rs 适配 + AudioHandle 扩展**

`client/src-tauri/src/audio/session.rs`：

`AudioHandle` 定义加字段（需在文件顶部 use 区补 `use std::sync::mpsc::SyncSender;` 与 `use std::sync::Mutex;`，若尚未导入）：

```rust
pub struct AudioHandle {
    pub stop: Arc<AtomicBool>,
    /// 视频帧送入口（前端 send_video_frame 命令写入；UDP 线程消费）
    pub video_tx: SyncSender<crate::net::udp::VideoOut>,
    /// 视频帧接收端（观看时由 bridge 的 watch 线程消费）
    pub video_rx: Arc<Mutex<Receiver<crate::net::udp::VideoIn>>>,
}
```

`spawn_audio_pipeline` 开头（原 `let (tx_pcm, rx_voice) = ...spawn_udp_voice(...)?;` 处）替换为：

```rust
    let (udp_tx, udp_rx) = crate::net::udp::spawn_udp(server_addr, uid, token, stop.clone())?;
    let tx_pcm = udp_tx.tx_pcm.clone();
    let rx_voice = udp_rx.rx_voice;
```

（`udp_rx.rx_screen_audio` 在 Task 7 接入播放线程；`udp_rx.rx_video`、`udp_tx` 的其余部分用于函数末尾。）

函数末尾 `Ok(AudioHandle { stop })` 替换为：

```rust
    Ok(AudioHandle {
        stop,
        video_tx: udp_tx.tx_video,
        video_rx: Arc::new(Mutex::new(udp_rx.rx_video)),
    })
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p echoroom-client`
Expected: 全绿（FrameAssembler 四个测试 + 既有 opus/mixer/session 测试）；`cargo build` 亦通过

注：接收线程 `buf` 尺寸 `[0u8; 2048]` 保持现状即可（最大视频包 1166B < 2048）。

---

### Task 7: 屏幕音频 Rust 全链路——立体声 Opus + WASAPI 进程回路 + 采集/混音接线

**Files:**
- Modify: `client/src-tauri/src/audio/opus.rs`
- Modify: `client/src-tauri/src/config.rs`
- Create: `client/src-tauri/src/audio/screen_capture.rs`
- Modify: `client/src-tauri/src/audio/mod.rs`
- Modify: `client/src-tauri/src/audio/session.rs`
- Modify: `client/src-tauri/src/bridge.rs`
- Modify: `client/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的 `windows` 依赖与 probe 验证结论；Task 3 的 `SCREEN_OPUS_BITRATE`；Task 6 的 `UdpTx.tx_screen_audio`、`UdpRx.rx_screen_audio`、`SharedAudio.viewer_count`（Task 5）
- Produces（后续任务依赖）：
  - `audio::opus::OpusEncStereo`（`new() -> Result<Self, audiopus::Error>`；`encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, audiopus::Error>`，输入 1920 交错样本）
  - `audio::opus::OpusDecStereo`（`decode(&mut self, frame: Option<&[u8]>, out: &mut [i16]) -> Result<usize, audiopus::Error>`，输出 1920 交错样本）
  - `audio::screen_capture::is_supported() -> bool`
  - `audio::screen_capture::spawn_screen_capture(tx: SyncSender<Vec<i16>>) -> anyhow::Result<ScreenCaptureHandle>`（块 = 1920 交错样本；句柄 Drop 即停止）
  - `Config.share_quality: String`（默认 `"720p30"`）、`Config.share_audio: bool`（默认 true）
  - `AudioHandle.screen_pcm_tx: SyncSender<Vec<i16>>`
  - invoke 命令：`set_share_active(active)`、`set_share_audio(on)`、`set_share_quality(quality)`、`screen_audio_supported()`
  - 事件：`share_audio`（on: bool）

- [ ] **Step 1: 写失败测试（立体声 Opus + config 字段）**

`client/src-tauri/src/audio/opus.rs` 测试模块追加：

```rust
    #[test]
    fn stereo_roundtrip_preserves_signal() {
        // 左 440Hz、右 880Hz（交织），幅度 12000
        let mut input = vec![0i16; 1920];
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            input[2 * i] = (f32::sin(2.0 * std::f32::consts::PI * 440.0 * t) * 12000.0) as i16;
            input[2 * i + 1] = (f32::sin(2.0 * std::f32::consts::PI * 880.0 * t) * 12000.0) as i16;
        }
        let mut enc = OpusEncStereo::new().unwrap();
        let mut dec = OpusDecStereo::new().unwrap();
        let packet = enc.encode(&input).unwrap();
        assert!(!packet.is_empty());
        let mut out = vec![0i16; 1920];
        let n = dec.decode(Some(&packet), &mut out).unwrap();
        assert_eq!(n, 1920, "解码应输出整帧（双声道）");
        let left: Vec<i16> = (0..960).map(|i| out[2 * i]).collect();
        let right: Vec<i16> = (0..960).map(|i| out[2 * i + 1]).collect();
        assert!(rms(&left) > 12000.0 * 0.3, "左声道能量过低: {}", rms(&left));
        assert!(rms(&right) > 12000.0 * 0.3, "右声道能量过低: {}", rms(&right));
    }
```

`client/src-tauri/src/config.rs` 测试更新：

- `roundtrip_with_volume_fields` 内加：
  ```rust
        cfg.share_quality = "1080p15".into();
        cfg.share_audio = false;
  ```
- `old_config_without_volume_fields_loads_defaults` 内加：
  ```rust
        assert_eq!(loaded.share_quality, "720p30");
        assert!(loaded.share_audio);
  ```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p echoroom-client`
Expected: 编译失败（`OpusEncStereo`/`OpusDecStereo` 不存在、Config 无新字段）

- [ ] **Step 3: 实现立体声 Opus**

`client/src-tauri/src/audio/opus.rs`——use 行补 `SCREEN_OPUS_BITRATE`、`OpusDec` 之后追加两个类型：

```rust
use echoroom_protocol::{FRAME_SAMPLES, OPUS_BITRATE, SCREEN_OPUS_BITRATE};
```

```rust
/// 立体声编码器：屏幕声音 PCM（48000Hz、20ms、1920 交错样本）→ Opus 包
pub struct OpusEncStereo(Encoder);

impl OpusEncStereo {
    pub fn new() -> Result<Self, audiopus::Error> {
        let mut e = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        e.set_bitrate(Bitrate::BitsPerSecond(SCREEN_OPUS_BITRATE))?;
        Ok(Self(e))
    }

    /// 编码一帧（pcm.len() 必须为 FRAME_SAMPLES * 2，左右交错）
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, audiopus::Error> {
        debug_assert_eq!(pcm.len(), FRAME_SAMPLES * 2);
        let mut out = vec![0u8; 1024]; // 128kbps×20ms = 320B，1024 有余量
        let n = self.0.encode(pcm, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
}

/// 立体声解码器：Opus 包 → 屏幕声音 PCM（1920 交错样本）
pub struct OpusDecStereo(Decoder);

impl OpusDecStereo {
    pub fn new() -> Result<Self, audiopus::Error> {
        Ok(Self(Decoder::new(SampleRate::Hz48000, Channels::Stereo)?))
    }

    /// 解码到 out（长度须为 FRAME_SAMPLES * 2）；返回解码样本数（正常 = 1920）
    pub fn decode(&mut self, frame: Option<&[u8]>, out: &mut [i16]) -> Result<usize, audiopus::Error> {
        debug_assert_eq!(out.len(), FRAME_SAMPLES * 2);
        self.0.decode(frame, out, false)
    }
}
```

- [ ] **Step 4: 实现 config 字段**

`client/src-tauri/src/config.rs`：

```rust
fn default_true() -> bool {
    true
}

fn default_share_quality() -> String {
    "720p30".into()
}
```

```rust
    /// 投屏画质档位（"720p30" / "1080p15" / "1080p30"）
    #[serde(default = "default_share_quality")]
    pub share_quality: String,
    /// 投屏时是否共享系统声音（Win11+）
    #[serde(default = "default_true")]
    pub share_audio: bool,
```

`Default for Config` 加：

```rust
            share_quality: "720p30".into(),
            share_audio: true,
```

- [ ] **Step 5: 实现 screen_capture.rs**

`client/src-tauri/src/audio/mod.rs`——`pub mod playback;` 之后插入（保持字母序）：

```rust
pub mod screen_capture;
```

`client/src-tauri/src/audio/screen_capture.rs`（新建）：

```rust
//! WASAPI 进程回路（Application Loopback）屏幕声音采集：排除本进程树的系统混音。
//! 仅 Windows 11（build ≥ 22000）支持；Win10 上 `is_supported()` 返回 false（UI 置灰）。
//! 输出：立体声 i16 交错、每 960 帧（1920 样本）一块经 SyncSender 送出。
//!
//! 注：COM 细节（implement 宏形态 / PROPVARIANT 构造 / GetBuffer 签名）以所用 windows crate
//! 版本编译提示为准；流程与 API 调用顺序与微软官方 ApplicationLoopback 示例一致
//! （与 Task 1 的 screen_probe.rs 相同，正式版只增加了停止旗标与分块送出）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, Sender};
use std::sync::Arc;
use std::time::Duration;

use windows::core::{implement, Interface, Result as WinResult};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::Foundation::WAVEFORMATEX;
use windows::Win32::System::Com::StructuredStorage::{BLOB, PROPVARIANT};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

const CHUNK_SAMPLES: usize = 1920; // 960 帧 × 2 声道（20ms @48kHz）

/// 采集句柄：Drop 即停止
pub struct ScreenCaptureHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for ScreenCaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 系统是否支持进程回路（Windows 11 build 22000+）
pub fn is_supported() -> bool {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut vi = OSVERSIONINFOW::default();
    vi.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let ok = unsafe { RtlGetVersion(&mut vi).is_ok() };
    ok && vi.dwBuildNumber >= 22000
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    done: Arc<AtomicBool>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler {
    fn ActivateCompleted(&self, _op: Option<&IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
        self.done.store(true, Ordering::Release);
        Ok(())
    }
}

/// 启动采集线程；返回前等待初始化完成（失败直接报错，句柄 Drop 停止采集）。
pub fn spawn_screen_capture(tx: SyncSender<Vec<i16>>) -> anyhow::Result<ScreenCaptureHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    std::thread::spawn(move || match init_capture() {
        Ok((client, capture, fmt)) => {
            let _ = ready_tx.send(Ok(()));
            capture_loop(&client, &capture, &fmt, &stop_thread, &tx);
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
        }
    });
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(ScreenCaptureHandle { stop }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow::anyhow!("屏幕采集线程提前退出")),
    }
}

/// 激活虚拟设备 + 初始化（流程与 screen_probe 相同）
fn init_capture() -> anyhow::Result<(IAudioClient, IAudioCaptureClient, WAVEFORMATEX)> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? }

    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams = AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
        TargetProcessId: std::process::id(),
        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    };
    let blob = BLOB {
        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pBlobData: &mut params as *mut _ as *mut u8,
    };
    let mut prop = PROPVARIANT::default();
    prop.Anonymous.Anonymous.vt = VT_BLOB;
    prop.Anonymous.Anonymous.Anonymous.blob = blob;

    let done = Arc::new(AtomicBool::new(false));
    let handler: IActivateAudioInterfaceCompletionHandler =
        ActivationHandler { done: done.clone() }.into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop),
            &handler,
        )?
    };
    while !done.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut hr = windows::core::HRESULT(0);
    let mut raw: Option<windows::core::IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut raw)? }
    hr.ok()?;
    let client: IAudioClient = raw.unwrap().cast()?;

    let mix = unsafe { client.GetMixFormat()? };
    let fmt = unsafe { *mix };
    if fmt.nChannels != 2 || fmt.nSamplesPerSec != 48000 || fmt.wFormatTag != 3 {
        println!(
            "[screen] 混音格式 {}Hz {}ch tag=0x{:X}（预期 48000/2/float32；非预期时按 2ch 交错读取，可能出现异常）",
            fmt.nSamplesPerSec, fmt.nChannels, fmt.wFormatTag
        );
    }
    let init = unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 200_000, 0, mix, None)
    };
    unsafe { CoTaskMemFree(Some(mix as *const _ as *const _)) }
    init?;
    client
        .GetService::<IAudioCaptureClient>()
        .map(|cap| (client, cap, fmt))
        .map_err(|e| e.into())
}

/// 读循环：f32 交错 → i16 → 满 1920 样本即送出一块（队列满丢块，不积压）
fn capture_loop(
    client: &IAudioClient,
    capture: &IAudioCaptureClient,
    fmt: &WAVEFORMATEX,
    stop: &AtomicBool,
    tx: &SyncSender<Vec<i16>>,
) {
    if let Err(e) = unsafe { client.Start() } {
        eprintln!("[screen] Start 失败: {e}");
        return;
    }
    println!("[screen] 进程回路采集已启动（排除本进程树）");
    let mut pending: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES * 2);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5));
        loop {
            let mut packet_frames: u32 = 0;
            if unsafe { capture.GetNextPacketSize(&mut packet_frames) }.is_err() || packet_frames == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }.is_err() {
                break;
            }
            let n = frames as usize * fmt.nChannels as usize;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data.is_null() {
                let f = unsafe { std::slice::from_raw_parts(data as *const f32, n) };
                for &x in f {
                    pending.push((x.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            } else {
                pending.resize(pending.len() + n, 0);
            }
            let _ = unsafe { capture.ReleaseBuffer(frames) };
            while pending.len() >= CHUNK_SAMPLES {
                let block: Vec<i16> = pending.drain(..CHUNK_SAMPLES).collect();
                let _ = tx.try_send(block); // 满则丢块
            }
        }
    }
    let _ = unsafe { client.Stop() };
    unsafe { CoUninitialize() }
    println!("[screen] 进程回路采集已停止");
}
```

- [ ] **Step 6: session 接线（编码线程 + 播放端叠加 + AudioHandle）**

`client/src-tauri/src/audio/session.rs`：

(a) `AudioHandle` 再加一字段：

```rust
    /// 屏幕 PCM 送入口（屏幕捕获线程写入；编码线程消费）
    pub screen_pcm_tx: SyncSender<Vec<i16>>,
```

(b) 采集线程块之后、播放线程之前，插入编码线程：

```rust
    // 屏幕音频编码线程：屏幕 PCM（1920 交错样本）→ Opus → UDP（无人观看时丢弃不编码）
    let (screen_pcm_tx, screen_pcm_rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
    {
        let stop = stop.clone();
        let viewer_count = shared.viewer_count.clone();
        let tx_screen_audio = udp_tx.tx_screen_audio.clone();
        std::thread::spawn(move || {
            let mut enc = match crate::audio::opus::OpusEncStereo::new() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[audio] 屏幕音频编码器创建失败: {e}");
                    return;
                }
            };
            while !stop.load(Ordering::Relaxed) {
                let pcm = match screen_pcm_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if pcm.len() != FRAME_SAMPLES * 2 {
                    continue;
                }
                if viewer_count.load(Ordering::Relaxed) == 0 {
                    continue; // 0 观众：不推流
                }
                match enc.encode(&pcm) {
                    Ok(opus) => {
                        let _ = tx_screen_audio.try_send(opus);
                    }
                    Err(e) => eprintln!("[audio] 屏幕音频编码失败: {e}"),
                }
            }
        });
    }
```

(c) 播放线程闭包——状态变量（`let mut scratch = vec![0i16; FRAME_SAMPLES];` 之后）：

```rust
    // 屏幕音频：解码器 + 单声道降混队列（每播放周期消费）
    let mut scr_dec: Option<crate::audio::opus::OpusDecStereo> = None;
    let mut scr_buf: std::collections::VecDeque<i16> = std::collections::VecDeque::new();
```

闭包内、收语音块（`while let Ok((uid, seq, opus)) = rx_voice.try_recv() { ... }`）之后插入：

```rust
        // 收屏幕音频：解码 → 降混单声道 → 入队（上限 2s 防积压）
        while let Ok((_uid, opus)) = udp_rx.rx_screen_audio.try_recv() {
            if scr_dec.is_none() {
                scr_dec = crate::audio::opus::OpusDecStereo::new().ok();
            }
            if let Some(dec) = scr_dec.as_mut() {
                let mut st = vec![0i16; FRAME_SAMPLES * 2];
                if dec.decode(Some(&opus), &mut st).is_ok() {
                    for i in 0..FRAME_SAMPLES {
                        scr_buf.push_back(((st[2 * i] as i32 + st[2 * i + 1] as i32) / 2) as i16);
                    }
                    while scr_buf.len() > FRAME_SAMPLES * 100 {
                        scr_buf.pop_front();
                    }
                }
            }
        }
```

填充循环内、`acc.finalize(&mut scratch[..want]);` 之前插入：

```rust
            // 屏幕声音叠加（固定 1.0 增益；所有输出段共用同一队列）
            let avail = scr_buf.len().min(want);
            if avail > 0 {
                let seg: Vec<i16> = scr_buf.drain(..avail).collect();
                acc.add_scaled(&seg, 1.0);
            }
```

(d) 函数末尾 `Ok(AudioHandle { ... })` 补字段：

```rust
    Ok(AudioHandle {
        stop,
        video_tx: udp_tx.tx_video,
        video_rx: Arc::new(Mutex::new(udp_rx.rx_video)),
        screen_pcm_tx,
    })
```

- [ ] **Step 7: bridge 接线（AppState + 同步函数 + 4 命令）**

`client/src-tauri/src/bridge.rs`：

`AppState` 追加字段：

```rust
    /// 本端是否正在投屏（屏幕流激活；驱动屏幕捕获生命周期）
    pub sharing: std::sync::atomic::AtomicBool,
    /// 屏幕捕获会话（Drop 即停止）
    pub screen_cap: Mutex<Option<crate::audio::screen_capture::ScreenCaptureHandle>>,
```

文件末尾追加：

```rust
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

/// 投屏开/停（前端在投屏开始/结束时调用；联动屏幕声音采集）
#[tauri::command]
pub fn set_share_active(state: State<AppState>, active: bool) {
    state.sharing.store(active, std::sync::atomic::Ordering::Relaxed);
    sync_screen_capture(&state, active);
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
```

- [ ] **Step 8: lib.rs 初始化与注册**

`client/src-tauri/src/lib.rs`：

- `AppState` 初始化块补：
  ```rust
            sharing: std::sync::atomic::AtomicBool::new(false),
            screen_cap: std::sync::Mutex::new(None),
  ```
- `invoke_handler` 追加：
  ```rust
            bridge::set_share_active,
            bridge::set_share_audio,
            bridge::set_share_quality,
            bridge::screen_audio_supported,
  ```

- [ ] **Step 9: 运行测试确认通过**

Run: `cargo test -p echoroom-client; cargo build`
Expected: 全绿（立体声 roundtrip、config 新旧字段测试）+ workspace 编译通过

---

### Task 8: 前端——采集与编码（video_capture.js）

**Files:**
- Create: `client/ui/video_capture.js`
- Modify: `client/src-tauri/src/bridge.rs`（`send_video_frame` 命令）
- Modify: `client/src-tauri/src/lib.rs`（注册）
- Modify: `client/ui/index.html`（script 引入）

**Interfaces:**
- Consumes: Task 6 的 `AudioHandle.video_tx`（经 `send_video_frame` 命令）、`NetCmd` 链路、Task 7 的 `set_share_active`、Task 5 的 `report_stream`；spike 结论 A1/A2/A3
- Produces（后续任务依赖）：
  - invoke `send_video_frame`：raw body 载荷 `[kind u8][keyframe u8][annexb data...]`
  - `window.videoCapture`：`{ STREAM_SCREEN, STREAM_CAMERA, startScreen(), startCamera(), stop(kind), setQuality(name), forceKeyframe(), setViewerCount(n), isActive(kind) }`

- [ ] **Step 1: Rust 命令 send_video_frame**

`client/src-tauri/src/bridge.rs` 末尾追加：

```rust
/// 前端编码帧上行（Tauri 原始请求体：[kind u8][keyframe u8][annexb data...]）
#[tauri::command]
pub fn send_video_frame(state: State<AppState>, request: tauri::ipc::Request) -> Result<(), String> {
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
```

`lib.rs` 注册：`bridge::send_video_frame,`

- [ ] **Step 2: video_capture.js**

`client/ui/video_capture.js`（新建；完整实现，无本地预览——画面预览在观看端）：

```js
// 视频采集与编码：屏幕（getDisplayMedia）/ 摄像头（getUserMedia）→ WebCodecs H.264（annexb）
// → invoke raw body 上行。挂载为 window.videoCapture。
// 门控：viewerCount == 0 时不编码（无人观看不推流）；forceKey 时强制关键帧。
window.videoCapture = (() => {
  const { invoke } = window.__TAURI__.core;
  const STREAM_SCREEN = 0;
  const STREAM_CAMERA = 1;

  // 画质档位（屏幕；摄像头固定 720p 档）
  const QUALITY = {
    "720p30": { height: 720, framerate: 30, bitrate: 2_000_000 },
    "1080p15": { height: 1080, framerate: 15, bitrate: 2_500_000 },
    "1080p30": { height: 1080, framerate: 30, bitrate: 4_000_000 },
  };
  const CAMERA_QUALITY = { height: 720, framerate: 30, bitrate: 1_500_000 };

  let quality = "720p30";
  let viewerCount = 0; // 由 app.js 在 viewer_count 事件中同步

  // kind → 会话 { kind, stream, reader, encoder(null 待首帧), canvas, ctx, spec, forceKey, n, stopped }
  const sessions = new Map();

  function specOf(kind) {
    return kind === STREAM_SCREEN ? QUALITY[quality] : CAMERA_QUALITY;
  }

  // 目标编码尺寸：高取档位值，宽按帧宽高比推导（偶数对齐，16:10 屏幕不失真）
  function targetSize(kind, frame) {
    const spec = specOf(kind);
    const ar = frame.displayWidth / frame.displayHeight;
    const w = Math.max(2, Math.round((spec.height * ar) / 2) * 2);
    return { w, h: spec.height };
  }

  function configureEncoder(session, w, h) {
    const spec = session.spec;
    session.encoder.configure({
      codec: "avc1.640028",
      width: w,
      height: h,
      bitrate: spec.bitrate,
      framerate: spec.framerate,
      avc: { format: "annexb" },
      latencyMode: "realtime",
    });
  }

  async function startPipeline(kind, stream) {
    const track = stream.getVideoTracks()[0];
    const processor = new MediaStreamTrackProcessor({ track });
    const reader = processor.readable.getReader();
    const canvas = new OffscreenCanvas(1280, 720);
    const session = {
      kind,
      stream,
      reader,
      canvas,
      ctx: canvas.getContext("2d"),
      encoder: null,
      spec: specOf(kind),
      forceKey: true, // 首帧必为关键帧
      n: 0,
      stopped: false,
    };
    session.encoder = new VideoEncoder({
      output: (chunk) => {
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        const buf = new Uint8Array(2 + data.length);
        buf[0] = kind;
        buf[1] = chunk.type === "key" ? 1 : 0;
        buf.set(data, 2);
        invoke("send_video_frame", buf.buffer).catch(() => {});
      },
      error: (e) => console.warn(`[video] kind=${kind} 编码错误:`, e.message),
    });
    sessions.set(kind, session);

    (async () => {
      while (!session.stopped) {
        let item;
        try {
          item = await session.reader.read();
        } catch {
          break;
        }
        if (item.done || session.stopped) break;
        const frame = item.value;
        // 无人观看且非强制关键帧：跳过编码（省 CPU/带宽；观看者到达时 forceKey 重开）
        if (viewerCount === 0 && !session.forceKey) {
          frame.close();
          continue;
        }
        // 首帧 / 档位切换后：按帧比例定 canvas 与编码尺寸
        const { w, h } = targetSize(kind, frame);
        if (session.canvas.width !== w || session.canvas.height !== h) {
          session.canvas.width = w;
          session.canvas.height = h;
          configureEncoder(session, w, h);
          session.forceKey = true;
        }
        session.ctx.drawImage(frame, 0, 0, w, h);
        frame.close();
        if (session.encoder.state !== "configured") continue;
        const vf = new VideoFrame(session.canvas, { timestamp: Math.round(performance.now() * 1000) });
        const natural = session.n++ % (session.spec.framerate * 2) === 0; // 自然关键帧 ≈2s
        session.encoder.encode(vf, { keyFrame: session.forceKey || natural });
        session.forceKey = false;
        vf.close();
      }
    })();
  }

  async function startScreen() {
    if (sessions.has(STREAM_SCREEN)) return;
    let stream;
    try {
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate: { ideal: 30 } },
        audio: false, // 系统声音由 Rust WASAPI 进程回路采集（浏览器无法排除房间自身声音）
      });
    } catch (e) {
      console.warn("[video] 屏幕采集取消/失败:", e.message);
      return;
    }
    await startPipeline(STREAM_SCREEN, stream);
    // 用户从系统共享条点"停止共享"
    stream.getVideoTracks()[0].addEventListener("ended", () => stop(STREAM_SCREEN));
    await invoke("report_stream", { kind: STREAM_SCREEN, on: true }).catch(() => {});
    await invoke("set_share_active", { active: true }).catch(() => {});
  }

  async function startCamera() {
    if (sessions.has(STREAM_CAMERA)) return;
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        video: { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } },
      });
    } catch (e) {
      console.warn("[video] 摄像头开启失败/被拒:", e.message);
      return;
    }
    await startPipeline(STREAM_CAMERA, stream);
    await invoke("report_stream", { kind: STREAM_CAMERA, on: true }).catch(() => {});
  }

  async function stop(kind) {
    const s = sessions.get(kind);
    if (!s) return;
    sessions.delete(kind);
    s.stopped = true;
    try {
      await s.reader.cancel();
    } catch {}
    try {
      s.encoder.close();
    } catch {}
    s.stream.getTracks().forEach((t) => t.stop());
    await invoke("report_stream", { kind, on: false }).catch(() => {});
    if (kind === STREAM_SCREEN) {
      await invoke("set_share_active", { active: false }).catch(() => {});
    }
  }

  function setQuality(name) {
    if (!QUALITY[name]) return;
    quality = name;
    const s = sessions.get(STREAM_SCREEN);
    if (s) {
      s.spec = QUALITY[name];
      s.forceKey = true; // 下一帧循环里按新档位重配编码器（尺寸可能变化 → 必须出新 IDR）
      s.canvas.width = 1; // 触发尺寸重算分支
    }
  }

  function forceKeyframe() {
    for (const s of sessions.values()) s.forceKey = true;
  }

  function setViewerCount(n) {
    viewerCount = n;
    // 0 → 有人看：保证下一帧是 IDR，让观看端能立即起步
    if (n > 0) forceKeyframe();
  }

  return {
    STREAM_SCREEN,
    STREAM_CAMERA,
    startScreen,
    startCamera,
    stop,
    setQuality,
    forceKeyframe,
    setViewerCount,
    isActive: (kind) => sessions.has(kind),
  };
})();
```

- [ ] **Step 3: index.html 引入**

`client/ui/index.html` 在 `<script src="app.js"></script>` 之前插入：

```html
  <script src="video_capture.js"></script>
```

- [ ] **Step 4: 手动验证（单机）**

1. 启动本地服务器（`cargo run -p echoroom-server`）+ `cargo tauri dev`
2. devtools console 执行 `videoCapture.startScreen()` → 选一个窗口共享
3. 观察 Rust 端 stdout：应出现 `[video] 上行帧 #0`、`#150`、`#300`…（≈30fps → 每 5 秒一条）
4. `videoCapture.setViewerCount(2)` 前后对比：设为 0 时上行帧日志应停止（`videoCapture.setViewerCount(0)`），重新设为 1 后恢复
5. `videoCapture.stop(0)` → 日志停止；服务器 stdout 无异常
6. `videoCapture.startCamera()` 重复上述（摄像头另有 rust 日志）

---

### Task 9: 前端——解码与观看视图（video_view.js）
> ⚠ 修订 R1（见文末）：`video_view.js` 增加 `setViewerCount(n)`——左上角 × 右侧显示"👀 N 人在看"。

**Files:**
- Create: `client/ui/video_view.js`
- Modify: `client/src-tauri/src/bridge.rs`（`watch_start`/`watch_stop` + `AppState.watching`）
- Modify: `client/src-tauri/src/lib.rs`（初始化 + 注册）
- Modify: `client/ui/index.html`（视图容器 + script）
- Modify: `client/ui/style.css`（观看视图样式）

**Interfaces:**
- Consumes: Task 6 的 `AudioHandle.video_rx`；Task 5 的 `subscribe`/`request_keyframe`；spike 结论 A4
- Produces（后续任务依赖）：
  - invoke `watch_start(ch: Channel<Vec<u8>>, target: u16)`：订阅 + 启动下行线程（新调用替换旧线程）
  - invoke `watch_stop()`：退订 + 停线程
  - 下行帧载荷：`[uid u16 BE][kind u8][keyframe u8][annexb data...]`
  - `window.videoView`：`{ watch(uid), end(), onStreamOff(kind), toggleFullscreen(), get watchingUid() }`

- [ ] **Step 1: Rust 观看线程**

`client/src-tauri/src/bridge.rs`：

`AppState` 追加字段：

```rust
    /// 观看线程停止旗标（None = 未在观看）
    pub watching: Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
```

文件末尾追加：

```rust
/// 开始观看某人：订阅 + 建视频下行线程（新调用会替换旧观看线程）。
/// 下行帧格式：[uid u16 BE][kind u8][keyframe u8][annexb data]
#[tauri::command]
pub fn watch_start(
    state: State<AppState>,
    ch: tauri::ipc::Channel<Vec<u8>>,
    target: u16,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    // 停旧观看线程
    if let Some(old) = state.watching.lock().unwrap().take() {
        old.store(true, Ordering::Relaxed);
    }
    // 先订阅（服务器开始把 target 的流转给本端）
    {
        let slot = state.net.lock().unwrap();
        let h = slot.as_ref().ok_or("未连接")?;
        h.tx.send(NetCmd::Subscribe(Some(target))).map_err(|e| e.to_string())?;
    }
    // 取视频接收端（音频管线启动后才有）
    let rx = {
        let audio = state.audio.lock().unwrap();
        let Some(ah) = audio.as_ref() else {
            return Err("音频管线未启动".into());
        };
        ah.video_rx.clone()
    };
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
                    if ch.send(payload).is_err() {
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
```

`lib.rs`：`AppState` 初始化补 `watching: std::sync::Mutex::new(None),`；invoke 注册补：

```rust
            bridge::watch_start,
            bridge::watch_stop,
```

- [ ] **Step 2: video_view.js**

`client/ui/video_view.js`（新建）：

```js
// 观看视图：Rust 下行帧 → VideoDecoder（每路独立）→ canvas。
// 单路：高 230px 按比例居中；双路：投屏主画面 + 摄像头右下角小窗（可拖可缩、比例锁死）。
// 容器：#view（覆盖 #members 区域）内 #view-stage / #view-close / #view-fs。
window.videoView = (() => {
  const { invoke, Channel } = window.__TAURI__.core;
  const STREAM_SCREEN = 0;
  const STREAM_CAMERA = 1;
  const DEC_CFG = { codec: "avc1.640028", avc: { format: "annexb" }, optimizeForLatency: true };

  let watchingUid = null; // 当前观看者（null = 未观看；payload 带 uid 用于过滤切换残留）
  let keyTimer = null; // 秒开重试定时器
  let lastReqAt = 0; // request_keyframe 节流
  const gotKey = {}; // kind → 是否已见到关键帧（未见到前不喂 delta 帧）
  const decoders = {}; // kind → VideoDecoder
  const canvases = {}; // kind → canvas

  const stage = () => document.getElementById("view-stage");

  function requestKey() {
    if (watchingUid === null) return;
    const now = Date.now();
    if (now - lastReqAt < 1000) return;
    lastReqAt = now;
    invoke("request_keyframe", { target: watchingUid }).catch(() => {});
  }

  function ensureDecoder(kind) {
    if (decoders[kind]) return decoders[kind];
    const canvas = ensureCanvas(kind);
    const dec = new VideoDecoder({
      output: (frame) => {
        if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
          canvas.width = frame.displayWidth;
          canvas.height = frame.displayHeight;
        }
        canvas.getContext("2d").drawImage(frame, 0, 0, canvas.width, canvas.height);
        frame.close();
      },
      error: (e) => {
        console.warn(`[view] kind=${kind} 解码器错误:`, e.message);
        gotKey[kind] = false;
        try {
          decoders[kind].close();
        } catch {}
        delete decoders[kind];
        requestKey();
      },
    });
    dec.configure(DEC_CFG);
    decoders[kind] = dec;
    return dec;
  }

  function ensureCanvas(kind) {
    if (canvases[kind]) return canvases[kind];
    const c = document.createElement("canvas");
    c.className = "view-canvas";
    if (kind === STREAM_CAMERA) makePipInteractive(c);
    stage().appendChild(c);
    canvases[kind] = c;
    applyLayout();
    return c;
  }

  function onFrame(buf) {
    const u8 = new Uint8Array(buf);
    if (u8.length < 5) return;
    const uid = (u8[0] << 8) | u8[1];
    const kind = u8[2];
    const key = u8[3] !== 0;
    if (uid !== watchingUid) return; // 切换订阅的残留帧
    if (!gotKey[kind] && !key) return; // 等关键帧再起步（delta 帧解不出）
    gotKey[kind] = true;
    try {
      ensureDecoder(kind).decode(
        new EncodedVideoChunk({
          type: key ? "key" : "delta",
          timestamp: Math.round(performance.now() * 1000),
          data: u8.subarray(4),
        })
      );
    } catch (e) {
      console.warn("[view] decode 调用失败:", e);
      gotKey[kind] = false;
      requestKey();
    }
  }

  // 主/小窗布局：screen 存在时 camera 是小窗，否则都是主画面
  function applyLayout() {
    const hasScreen = !!canvases[STREAM_SCREEN];
    for (const key of Object.keys(canvases)) {
      const kind = Number(key);
      const c = canvases[key];
      const pip = kind === STREAM_CAMERA && hasScreen;
      c.classList.toggle("pip", pip);
      c.classList.toggle("main", !pip);
    }
  }

  // 小窗：左键拖动移位；右下角 20×20 区域拖拽缩放（宽 120–480px，比例由固有尺寸锁死）
  function makePipInteractive(canvas) {
    let mode = null;
    let sx = 0, sy = 0, sl = 0, st = 0, sw = 0;
    canvas.addEventListener("pointerdown", (e) => {
      if (!canvas.classList.contains("pip")) return;
      const r = canvas.getBoundingClientRect();
      const nearCorner = e.clientX > r.right - 20 && e.clientY > r.bottom - 20;
      mode = nearCorner ? "resize" : "move";
      sx = e.clientX;
      sy = e.clientY;
      sl = r.left;
      st = r.top;
      sw = r.width;
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch {}
      e.preventDefault();
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!mode) return;
      const dx = e.clientX - sx;
      const dy = e.clientY - sy;
      if (mode === "move") {
        const sr = stage().getBoundingClientRect();
        const cr = canvas.getBoundingClientRect();
        let nl = Math.min(Math.max(sl + dx - sr.left, 0), sr.width - cr.width);
        let nt = Math.min(Math.max(st + dy - sr.top, 0), sr.height - cr.height);
        canvas.style.right = "auto";
        canvas.style.bottom = "auto";
        canvas.style.left = nl + "px";
        canvas.style.top = nt + "px";
      } else {
        const w = Math.min(480, Math.max(120, sw + dx));
        canvas.style.width = w + "px";
      }
      e.preventDefault();
    });
    const stopDrag = (e) => {
      if (!mode) return;
      mode = null;
      try {
        canvas.releasePointerCapture(e.pointerId);
      } catch {}
    };
    canvas.addEventListener("pointerup", stopDrag);
    canvas.addEventListener("pointercancel", stopDrag);
  }

  async function watch(uid) {
    await end();
    watchingUid = uid;
    let ch;
    try {
      ch = new Channel(onFrame);
      await invoke("watch_start", { ch, target: uid });
    } catch (e) {
      console.warn("[view] watch_start 失败:", e);
      watchingUid = null;
      return;
    }
    document.getElementById("view").hidden = false;
    // 秒开：立即请求关键帧；1.5s 内未收到任何关键帧则重试（最多 3 次）
    requestKey();
    let tries = 0;
    keyTimer = setInterval(() => {
      if (Object.values(gotKey).some(Boolean) || tries >= 3) {
        clearInterval(keyTimer);
        keyTimer = null;
        return;
      }
      tries++;
      requestKey();
    }, 1500);
  }

  async function end() {
    if (keyTimer) {
      clearInterval(keyTimer);
      keyTimer = null;
    }
    if (watchingUid === null) return;
    watchingUid = null;
    try {
      await invoke("watch_stop");
    } catch {}
    for (const k of Object.keys(decoders)) {
      try {
        decoders[k].close();
      } catch {}
      delete decoders[k];
    }
    for (const k of Object.keys(canvases)) {
      canvases[k].remove();
      delete canvases[k];
    }
    for (const k of Object.keys(gotKey)) delete gotKey[k];
    document.getElementById("view").hidden = true;
  }

  // 对方某路流停止：移除对应画面；两路都没了 → 退出观看
  function onStreamOff(kind) {
    if (canvases[kind]) {
      canvases[kind].remove();
      delete canvases[kind];
    }
    if (decoders[kind]) {
      try {
        decoders[kind].close();
      } catch {}
      delete decoders[kind];
    }
    delete gotKey[kind];
    if (Object.keys(canvases).length === 0) {
      end();
    } else {
      applyLayout();
    }
  }

  async function toggleFullscreen() {
    const w = window.__TAURI__.window.getCurrentWindow();
    const cur = await w.isFullscreen();
    await w.setFullscreen(!cur);
    document.getElementById("view-fs").classList.toggle("on", !cur);
  }

  // 按钮接线（DOM 已在 body 尾部就绪）
  document.getElementById("view-close").addEventListener("click", () => end());
  document.getElementById("view-fs").addEventListener("click", () => toggleFullscreen());

  return {
    watch,
    end,
    onStreamOff,
    toggleFullscreen,
    get watchingUid() {
      return watchingUid;
    },
  };
})();
```

- [ ] **Step 3: index.html 容器与引入**

`client/ui/index.html`：

`#members` 改为包含视图容器：

```html
      <section class="members" id="members">
        <section class="view" id="view" hidden>
          <div class="view-stage" id="view-stage"></div>
          <button class="view-btn view-close" id="view-close" title="关闭">×</button>
          <button class="view-btn view-fs" id="view-fs" title="全屏">⛶</button>
        </section>
      </section>
```

script 引入顺序（app.js 之前、video_capture.js 之后）：

```html
  <script src="video_capture.js"></script>
  <script src="video_view.js"></script>
  <script src="app.js"></script>
```

- [ ] **Step 4: style.css 观看视图样式**

`client/ui/style.css` 末尾追加（并确认 `.members` 有 `position: relative;`，没有则补进其现有规则）：

```css
/* ---- B：观看视图 ---- */
.view {
  position: absolute;
  inset: 0;
  z-index: 20;
  background: rgba(15, 23, 42, 0.6);
  overflow: hidden;
}
.view[hidden] {
  display: none;
}
.view-stage {
  position: absolute;
  inset: 0;
}
.view-canvas.main {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  height: 230px;
  width: auto;
  border-radius: 8px;
  background: #000;
}
.view-canvas.pip {
  position: absolute;
  right: 12px;
  bottom: 12px;
  width: 176px;
  height: auto;
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
  cursor: move;
  touch-action: none;
  background: #000;
}
.view-btn {
  position: absolute;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  background: rgba(15, 23, 42, 0.7);
  color: #e2e8f0;
  font-size: 16px;
  cursor: pointer;
  z-index: 21;
}
.view-btn:hover {
  background: rgba(30, 41, 59, 0.95);
}
.view-close {
  left: 10px;
  top: 10px;
}
.view-fs {
  right: 10px;
  top: 10px;
}
.view-fs.on {
  background: #3b82f6;
}
```

- [ ] **Step 5: 手动验证（双客户端）**

1. 客户端 A（另一台机或打包版）：开投屏（Task 8 的 `videoCapture.startScreen()`）；客户端 B（dev）：`cargo tauri dev`，devtools 执行 `videoView.watch(<A的uid>)`（uid 从 `members` 事件或 app.js 状态里看）
2. 预期：B 出现画面，高 230 居中；A 端 stdout 出现"上行帧 #..."恢复增长（ViewerCount 0→1 触发编码）
3. A 再开摄像头（`videoCapture.startCamera()`）→ B 端出现右下角小窗；拖动小窗移位、拖右下角缩放（比例不变）
4. B 的 `videoView.onStreamOff(0)`（或让 A 停投屏）→ 小窗升为单路主画面；A 全停 → B 自动退出视图
5. 全屏按钮 → 窗口全屏、小窗形态不变；关闭按钮 → 退出

---

### Task 10: 前端 UI 接线——卡片纱/角标、发送端按钮与投屏面板（app.js + style.css）
> ⚠ 修订 R1（见文末）：`viewer_count` 分发扩展 + 投屏面板"正在观看"名单行（uids → 昵称）。

**Files:**
- Modify: `client/ui/app.js`
- Modify: `client/ui/style.css`

**Interfaces:**
- Consumes: Task 5/7/8/9 的全部事件与命令——事件 `stream_state`（{uid,kind,on}）、`viewer_count`（n）、`request_keyframe`（()）、`share_audio`（on）；invoke `report_stream`/`set_share_quality`/`set_share_audio`/`screen_audio_supported`；`window.videoCapture`、`window.videoView`（script 加载顺序保证：capture → view → app）
- Produces（Task 11 验收依赖）：完整可交互入口——他人卡片"纱+播放按钮"进入观看、自己卡片投屏/摄像头按钮、投屏设置面板（档位/声音/停止）

- [ ] **Step 1: 状态与图标（app.js 三处小改）**

(a) `members` 值结构加 `streams`：

原：
```js
const members = new Map(); // uid → { nickname, speaking, muted }
```
新：
```js
const members = new Map(); // uid → { nickname, speaking, muted, streams }
```

(b) `volState` 初始化行（`// Rust 侧音量真值的本地副本` 那行）之后追加：

```js
// B：投屏设置本地副本（Rust config 为真值）
let shareQuality = "720p30";
let shareAudioOn = true;
let screenAudioOk = false; // Win11 22000+；init 时查询
```

(c) ICONS 中 `cam` 与 `avatar` 之间插入 play 图标：

```js
  play: '<svg viewBox="0 0 24 24" fill="currentColor"><polygon points="6 4 20 12 6 20 6 4"/></svg>',
```

- [ ] **Step 2: buildCard 改造**

(a) 头像块替换：

原：
```js
  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  avatar.innerHTML = ICONS.avatar;
  div.appendChild(avatar);
```
新：
```js
  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  avatar.innerHTML = ICONS.avatar;
  // 他人正在直播：纱（中央播放按钮，再点退出观看）+ 角标
  if (!isSelf && m.streams) {
    const veil = document.createElement("div");
    veil.className = "member-veil";
    const watch = document.createElement("button");
    watch.className = "watch-btn";
    watch.title = "观看";
    watch.innerHTML = ICONS.play;
    watch.addEventListener("click", (e) => {
      e.stopPropagation();
      if (videoView.watchingUid === uid) videoView.end();
      else videoView.watch(uid);
    });
    veil.appendChild(watch);
    avatar.appendChild(veil);
    const badge = document.createElement("div");
    badge.className = "member-badge";
    badge.textContent = m.streams === 3 ? "投屏+摄像头" : m.streams & 1 ? "投屏中" : "摄像头中";
    avatar.appendChild(badge);
  }
  div.appendChild(avatar);
```

(b) 自己卡片的投屏/摄像头按钮（替换 disabled 占位版）：

原：
```js
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
```
新：
```js
  if (isSelf) {
    const screenOn = videoCapture.isActive(videoCapture.STREAM_SCREEN);
    const screenBtn = document.createElement("button");
    screenBtn.className = "mbtn mbtn-screen" + (screenOn ? " active acc" : "");
    screenBtn.title = screenOn ? "投屏设置" : "开始投屏";
    screenBtn.innerHTML = ICONS.screen;
    screenBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const btn = e.currentTarget; // await 后 currentTarget 失效，先取节点
      if (videoCapture.isActive(videoCapture.STREAM_SCREEN)) {
        // 投屏中：按钮 = 面板开关
        if (sharePop.classList.contains("show")) closeSharePop();
        else openSharePop(btn);
      } else {
        // 未投屏：先起投屏（系统选择器），成功后弹设置面板
        await videoCapture.startScreen();
        renderMembers(); // 重建卡片（旧节点已脱离 DOM，不能用作定位锚点）
        const fresh = document.querySelector(".mbtn-screen");
        if (fresh && videoCapture.isActive(videoCapture.STREAM_SCREEN)) openSharePop(fresh);
      }
    });
    btns.appendChild(screenBtn);

    const camOn = videoCapture.isActive(videoCapture.STREAM_CAMERA);
    const camBtn = document.createElement("button");
    camBtn.className = "mbtn" + (camOn ? " active acc" : "");
    camBtn.title = camOn ? "关闭摄像头" : "开启摄像头";
    camBtn.innerHTML = ICONS.cam;
    camBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (videoCapture.isActive(videoCapture.STREAM_CAMERA)) {
        await videoCapture.stop(videoCapture.STREAM_CAMERA);
      } else {
        await videoCapture.startCamera();
      }
      renderMembers();
    });
    btns.appendChild(camBtn);
  }
```

说明：`active acc` 复用现有蓝色激活态样式（`.mbtn.active.acc`）；`mbtn-screen` 类供"点外部关面板"识别。

- [ ] **Step 3: 投屏设置面板（sharePop）**

在 volPop 段末尾（`document.addEventListener("click", ...)` 关闭逻辑之前）插入整段：

```js
// ---- 投屏设置面板（单例；投屏中点击卡片投屏按钮开关） ----
const sharePop = document.createElement("div");
sharePop.className = "share-pop";
sharePop.innerHTML = `
  <div class="sp-title">投屏设置</div>
  <label class="sp-row"><input type="radio" name="sp-quality" value="720p30" />720p 30fps</label>
  <label class="sp-row"><input type="radio" name="sp-quality" value="1080p15" />1080p 15fps</label>
  <label class="sp-row"><input type="radio" name="sp-quality" value="1080p30" />1080p 30fps</label>
  <label class="sp-row"><input type="checkbox" id="sp-audio" />共享系统声音<span class="sp-hint" id="sp-audio-hint"></span></label>
  <button class="sp-stop" id="sp-stop">停止投屏</button>
`;
document.body.appendChild(sharePop);

function applyShareAudioUi() {
  const cb = sharePop.querySelector("#sp-audio");
  cb.checked = shareAudioOn && screenAudioOk;
  cb.disabled = !screenAudioOk;
  sharePop.querySelector("#sp-audio-hint").textContent = screenAudioOk ? "" : "（需 Win11）";
}

function openSharePop(anchor) {
  for (const r of sharePop.querySelectorAll('input[name="sp-quality"]')) {
    r.checked = r.value === shareQuality;
  }
  applyShareAudioUi();
  const r = anchor.getBoundingClientRect();
  sharePop.classList.add("show");
  const w = sharePop.offsetWidth;
  const h = sharePop.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - w - 8);
  let top = r.bottom + 6;
  if (top + h > window.innerHeight - 8) top = r.top - h - 6;
  sharePop.style.left = left + "px";
  sharePop.style.top = top + "px";
}

function closeSharePop() {
  sharePop.classList.remove("show");
}

for (const r of sharePop.querySelectorAll('input[name="sp-quality"]')) {
  r.addEventListener("change", () => {
    if (!r.checked) return;
    shareQuality = r.value;
    videoCapture.setQuality(shareQuality); // 下一帧起按新档位重配编码器并出新 IDR
    invoke("set_share_quality", { quality: shareQuality }).catch((e) => console.error("保存投屏档位失败:", e));
  });
}

sharePop.querySelector("#sp-audio").addEventListener("change", (e) => {
  shareAudioOn = e.target.checked;
  invoke("set_share_audio", { on: shareAudioOn }).catch((e) => console.error("切换共享声音失败:", e));
});

sharePop.querySelector("#sp-stop").addEventListener("click", async () => {
  closeSharePop();
  await videoCapture.stop(videoCapture.STREAM_SCREEN);
  renderMembers();
});
```

关闭监听扩展——原：
```js
document.addEventListener("click", (e) => {
  if (!e.target.closest(".vol-pop") && !e.target.closest(".mbtn-vol")) closeVolPop();
});
```
新：
```js
document.addEventListener("click", (e) => {
  if (!e.target.closest(".vol-pop") && !e.target.closest(".mbtn-vol")) closeVolPop();
  if (!e.target.closest(".share-pop") && !e.target.closest(".mbtn-screen")) closeSharePop();
});
```

- [ ] **Step 4: 事件接线**

(a) `members` 事件解构四元组——原：
```js
    for (const [uid, nickname, muted] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted });
    }
```
新：
```js
    for (const [uid, nickname, muted, streams] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted, streams });
    }
```

(b) `member_join`——原：`members.set(uid, { nickname, speaking: false, muted: false });`
新：`members.set(uid, { nickname, speaking: false, muted: false, streams: 0 });`

(c) `member_leave` 整块替换——原：
```js
  await listen("member_leave", (e) => {
    members.delete(e.payload);
    renderMembers();
    renderOnline();
    playSnd(sndOut); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）
  });
```
新：
```js
  await listen("member_leave", (e) => {
    const uid = e.payload;
    if (videoView.watchingUid === uid) videoView.end(); // 被观看者离开：订阅已随其退出失效，直接收尾
    members.delete(uid);
    renderMembers();
    renderOnline();
    playSnd(sndOut); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）
  });
```

(d) `volume` 监听之后、`conn` 监听之前插入四个新监听：

```js
  await listen("stream_state", (e) => {
    const { uid, kind, on } = e.payload;
    const m = members.get(uid);
    if (m) m.streams = on ? m.streams | (1 << kind) : m.streams & ~(1 << kind);
    renderMembers();
    // 自己屏幕流结束（含系统共享条"停止共享"）：收起投屏面板
    if (uid === myUid && kind === videoCapture.STREAM_SCREEN && !on) closeSharePop();
    // 正在观看的人停了一路：移除对应画面（两路全停则视图自动退出）
    if (videoView.watchingUid === uid && !on) videoView.onStreamOff(kind);
  });
  await listen("viewer_count", (e) => videoCapture.setViewerCount(e.payload));
  await listen("request_keyframe", () => videoCapture.forceKeyframe());
  await listen("share_audio", (e) => {
    shareAudioOn = e.payload;
    applyShareAudioUi();
  });
```

(e) `conn` 监听替换——原：
```js
  await listen("conn", (e) => setConn(e.payload));
```
新：
```js
  await listen("conn", (e) => {
    setConn(e.payload);
    if (e.payload === "reconnecting") {
      videoView.end(); // 连接断开：正在观看的流必然中断
      videoCapture.setViewerCount(0); // 观众数随旧会话清零；重连后由服务器推回
    }
  });
```

(f) init 读配置——原：
```js
  const cfg = await invoke("get_config");
  // 音量真值来自 Rust（config 持久化）：初始化本地副本
  volState = { self_gain: cfg.self_gain, muted: cfg.muted, peer_gains: cfg.peer_gains };
```
新：
```js
  const cfg = await invoke("get_config");
  // 音量真值来自 Rust（config 持久化）：初始化本地副本
  volState = { self_gain: cfg.self_gain, muted: cfg.muted, peer_gains: cfg.peer_gains };
  // B：投屏配置（档位应用给采集模块；声音开关/平台能力供面板显示）
  shareQuality = cfg.share_quality || "720p30";
  videoCapture.setQuality(shareQuality);
  shareAudioOn = cfg.share_audio !== false;
  screenAudioOk = await invoke("screen_audio_supported").catch(() => false);
  applyShareAudioUi();
```

（注意执行顺序：Step 3 的 sharePop 段落在文件顶部区域、init 在文件末尾，`applyShareAudioUi`/`closeSharePop` 均为函数声明已提升；`sharePop` 常量在 init 执行前已完成初始化。）

- [ ] **Step 5: style.css**

(a) `.member-avatar` 规则补 `position: relative;`（新增绝对定位子元素需要）：

原：
```css
.member-avatar {
  height: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #15181f;
  color: #333a49;
  flex-shrink: 0;
}
```
新：
```css
.member-avatar {
  position: relative;
  height: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #15181f;
  color: #333a49;
  flex-shrink: 0;
}
```

(b) 文件末尾追加：

```css
/* ---- B：成员卡片流角标 / 观看纱 ---- */
.member-badge {
  position: absolute;
  left: 6px;
  top: 6px;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  color: #dbeafe;
  background: rgba(59, 130, 246, .82);
}
.member-veil {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(15, 23, 42, .38);
}
.member-veil:hover { background: rgba(15, 23, 42, .58); }
.watch-btn {
  width: 44px;
  height: 44px;
  border: none;
  border-radius: 50%;
  background: rgba(15, 23, 42, .7);
  color: #e2e8f0;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.watch-btn svg { width: 20px; height: 20px; }

/* ---- B：投屏设置面板 ---- */
.share-pop {
  position: fixed;
  z-index: 30;
  width: 172px;
  padding: 10px 12px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, .45);
  display: none;
  flex-direction: column;
  gap: 6px;
}
.share-pop.show { display: flex; }
.share-pop .sp-title { font-size: 12px; color: var(--text-dim); }
.share-pop .sp-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: #cbd0dc;
  cursor: pointer;
}
.share-pop .sp-hint { font-size: 11px; color: var(--err); }
.share-pop .sp-stop {
  margin-top: 2px;
  padding: 6px;
  border: none;
  border-radius: 8px;
  background: rgba(248, 113, 113, .15);
  color: var(--err);
  font-size: 12px;
  cursor: pointer;
}
.share-pop .sp-stop:hover { background: rgba(248, 113, 113, .28); }
```

- [ ] **Step 6: 手动验证**

单客户端（1–6）+ 双客户端（7–9）：

1. `cargo tauri dev` + 本地服务器；连接后自己卡片：投屏/摄像头按钮可点（不再是"即将推出"禁用态）
2. 点投屏 → 系统选择器选屏 → 按钮变蓝，自动弹出设置面板（三个档位、声音勾选、停止投屏）
3. 再点投屏按钮 → 面板关闭；再点 → 重开（出现在按钮下方）；点击页面其他区域 → 关闭
4. 面板切"1080p15" → devtools 无报错；勾/取消"共享系统声音" → Rust stdout 出现"[screen] 屏幕声音采集启动/停止"
5. 点摄像头 → 按钮变蓝；再点 → 复原（无面板）
6. 面板"停止投屏" → 按钮复原；或从系统共享条"停止共享" → 同样复原且面板自动关闭
7. 双客户端：对方卡片出现"投屏中"角标 + 纱（中央播放按钮）；点击进入观看；再点纱退出观看
8. 双客户端：观看中对方全停 → 观看视图自动退出；双路时停一路 → 对应画面消失、另一路升为主画面
9. 断线重连：断连 → "重连中…"；恢复后成员角标按服务器最新状态重绘（本端若仍在投屏，按钮保持蓝色）

---

### Task 11: 全量验收与 spike 清理
> ⚠ 修订 R1（见文末）：验收表追加第 11、12 条（观看人数/名单联动）。

**Files:**
- Delete: `client/ui/spike.html`、`client/ui/spike.js`、`client/src-tauri/src/bin/screen_probe.rs`
- Modify: `client/src-tauri/src/lib.rs`（删除 spike 命令与注册）
- 可选删除: `client/src-tauri/spike_probe.wav`（若 spike 时生成过）

**Interfaces:**
- Consumes: Task 1–10 的全部产出
- Produces: 无临时物、全绿、可交付的 B 子项目

- [ ] **Step 1: 全量测试与构建**

Run: `cargo test; cargo build`
（工作目录 `E:\pro\EchoRoom`；PowerShell 用 `;` 分隔）

Expected:
- `echoroom-protocol`：TCP 新消息 roundtrip、UDP VideoChunk/ScreenAudio roundtrip 与满片尺寸测试全绿
- `echoroom-server`：room 订阅/观看数/离开清理三个新测试全绿
- `echoroom-client`：FrameAssembler 四个测试、立体声 Opus roundtrip、config 新旧字段测试全绿
- workspace 编译通过

- [ ] **Step 2: 端到端验收（双客户端 + 服务器）**

准备：本机 `cargo run -p echoroom-server`；客户端 A = 另一台机器或打包版（同机双开 dev 会共享配置目录，勿同机同模式双开）；客户端 B = `cargo tauri dev`。

| # | 操作 | 预期 |
|---|------|------|
| 1 | B 连入 | 成员卡片正常；无流成员无角标、无纱 |
| 2 | A 开投屏 | A 按钮变蓝；B 端 A 卡片出现"投屏中"角标+纱；A stdout 无上行帧日志（人数 0 门控） |
| 3 | B 点 A 的纱 | B 出现观看视图（高 230 居中）；A stdout 开始出现 `[video] 上行帧 #0/150/...` |
| 4 | A 开摄像头 | B 变为"投屏主画面 + 右下角摄像头小窗"；小窗可拖移、右下角拖拽缩放 120–480px 且比例不变 |
| 5 | A 面板切"1080p15" | B 画面 1–2 秒内更新分辨率（重配+新 IDR）；切回 720p30 同样生效 |
| 6 | A 勾"共享系统声音"并摆音乐 | B 能听到系统音乐；B 说话时 A 的共享声音里听不到 B 的语音（进程回路排除 EchoRoom 自身，无回环） |
| 7 | B 退出观看 | A 的观众数回落；无人观看时 A stdout 上行帧日志即停（门控关） |
| 8 | B 从看 A 切到看 C（C 为第三个客户端，也在投屏） | A 的观众数回落、C 升 1；B 画面为 C 的流，无 A 的残留画面/声音 |
| 9 | A 系统共享条"停止共享" | A 按钮复原、面板自动关闭；B 观看视图自动退出；A 重开投屏后 B 重看正常 |
| 10 | 断线重连 | 停服务器 5 秒再启：双方"重连中…"→"已连接"；B 观看自动退出；A 仍在投屏时按钮保持蓝色，恢复后其他端重新看到角标；B 重新点纱可秒开画面 |

- [ ] **Step 3: 清理 spike 临时物**

1. 删除三个临时文件（见 Files 节）
2. `lib.rs`：删除 `spike_echo`/`spike_feed` 两个函数（含 `// --- 临时 spike 命令（Task 11 删除）---` 注释块）与 `invoke_handler` 中的两行注册
3. 若 `client/src-tauri/spike_probe.wav` 存在一并删除
4. 保留项：client 的 `windows` crate 依赖（Task 7 的 screen_capture.rs 依赖）、tauri.conf.json 的反节流 flags（投屏后台保活依赖）

- [ ] **Step 4: 清理后回归**

Run: `cargo test; cargo build`
Expected: 全绿；`cargo tauri dev` 能启动，投屏/观看入口仍在（无对 spike 的残留引用）

- [ ] **Step 5: 完成清单确认**

- 发送端：投屏（档位/系统声音/停止面板）+ 摄像头（直接开关）；无人观看不编码
- 观看端：纱入口 → 单路主画面 / 双路主+小窗（拖移、缩放、全屏）；关键帧请求秒开
- 链路：UDP 分片与重组、自然/请求 IDR、人数门控、重连补报流状态
- 范围外（后续子项目）：真实头像（C 阶段）、多人多路观看、录制回放

---

## 修订记录（执行期新增）

### R1（2026-09-19）：观看人数显示与"谁在看"名单

**需求（用户确认）**：
- 观看视图（点纱出现的画面窗口）：左上角 × 按钮右侧悬浮显示"👀 N 人在看"（N = 当前观看该流的总人数，各观众看到的一致）
- 投屏者的投屏设置面板（sharePop，仅投屏中显示）：新增只读一行"正在观看：小K、阿信"（无人在看 → "暂无"）
- 不新增"流主预览自己流"的入口（观看视图仍仅观看者可用）

**协议升级（替代 Task 2 的 `ViewerCount { n: u16 }`，type 13 不变）**：
- `TcpMessage::Viewers { uids: Vec<u16> }`；载荷 = `[count u16][uid u16 × count]`
- 语义 = 该流当前全部订阅者 uid 列表（含刚订阅者本人；人数 N = `uids.len()`）
- 收件人 = 流主 + 该流全部订阅者（同内容定向发送）
- 改动点：`messages.rs`（变体、type_id 注释）、`tcp.rs`（encode/decode 与 roundtrip 样本）

**服务器（Task 4 增量）**：
- 新增 `Room::viewers_of(target: u16) -> Vec<u16>`（按 uid 升序，保证稳定）
- 统一触发时机：流 T 的订阅集合任一变化后（Subscribe 新订阅、覆盖切换旧/新目标、Unsubscribe、LeaveGuard 离开、T 的 StreamState 上报初始值）→ 向 `{T} ∪ viewers_of(T)` 每人发送 `Viewers { uids: viewers_of(T) }`
- 原 `ViewerCount { n }` 的全部发送点按此替换

**客户端 Rust（Task 5 增量）**：
- 收到 `Viewers { uids }` → `shared.viewer_count.store(uids.len())`（屏幕音频/视频门控语义不变）+ `bridge.emit_viewer_count(uids)`（前端事件 payload = uid 数组）

**前端（Task 9/10 增量）**：
- `video_view.js`：新增 `setViewerCount(n)`——正在观看时在窗口左上角（× 右侧）显示"👀 N 人在看"；退出/切换观看时清除；双路视图挂主画面窗口
- `app.js` `listen("viewer_count")`：`const uids = e.payload; videoCapture.setViewerCount(uids.length); videoView.setViewerCount(uids.length); updateShareViewers(uids);`
- `updateShareViewers(uids)`：缓存最近名单；投屏面板打开时渲染"正在观看：…"（uid → members 昵称表，缺失显示 uid；空数组 → "暂无"）
- `style.css`：角落徽标（半透明底、小字号）与面板名单行样式

**验收增量（Task 11 验收表追加）**：
| 11 | B 点 A 的纱 | A 投屏面板显示"正在观看：B"；B 观看视图角落显示"👀 1 人在看" |
| 12 | C 也看 A；随后 B 退出 | B、C 角落均显示"👀 2 人在看"；A 面板显示"正在观看：B、C"；B 退出后 C 变"👀 1 人在看"、A 面板剩"正在观看：C" |

**执行时机**：协议层升级在 Task 4 开始前完成（作为 Task 4 的 Step 0 或独立小提交）；前端两项随 Task 9/10 原步骤一并实现。

---

