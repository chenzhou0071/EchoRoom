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
  log("[采集] 观察 30s。节流测试全自动：5 秒后应用自动最小化、15 秒自动恢复，观察 [可见性] hidden 期间帧数是否继续增长。");
  // 自动化节流测试：到点自动最小化（5s）/ 恢复（15s），Rust 侧延迟复查状态打印到 dev 控制台
  setTimeout(() => {
    log("[测试] 自动最小化窗口（5s 到点）");
    invoke("spike_minimize", { minimize: true }).catch((e) => log(`[测试] 最小化失败: ${e}`));
    invoke("spike_min_check", { afterMs: 600 }).catch(() => {});
  }, 5000);
  setTimeout(() => {
    log("[测试] 自动恢复窗口（15s 到点）");
    invoke("spike_minimize", { minimize: false }).catch((e) => log(`[测试] 恢复失败: ${e}`));
    invoke("spike_min_check", { afterMs: 600 }).catch(() => {});
  }, 15000);
  // 可见性变化打点：最小化→hidden，恢复→visible
  if (window.__visListener) document.removeEventListener("visibilitychange", window.__visListener);
  window.__visListener = () => {
    log(`[可见性] ${document.visibilityState} @ ${((performance.now() - t0) / 1000).toFixed(1)}s`);
  };
  document.addEventListener("visibilitychange", window.__visListener);
  let lastFrames = 0, lastT = t0;
  const timer = setInterval(() => {
    const now = performance.now();
    const dt = (now - t0) / 1000;
    const winDt = Math.max((now - lastT) / 1000, 0.001);
    const df = frames - lastFrames;
    lastFrames = frames;
    lastT = now;
    log(`[统计] ${dt.toFixed(1)}s：取帧 ${frames}（本窗 +${df} = ${(df / winDt).toFixed(1)}fps，${document.visibilityState}）→ 编码 ${chunks} 帧（${(bytes / 1024 / dt).toFixed(0)}KB/s，关键帧 ${keyframes}）`);
  }, 2000);
  setTimeout(() => {
    clearInterval(timer);
    log("[采集] 30s 观察结束（统计已停）。若刚才未测最小化：停止后重新点『2-4 采集』→ 跑 5 秒 → 最小化 10 秒 → 恢复，看 hidden 期间本窗帧数是否仍增长。");
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
