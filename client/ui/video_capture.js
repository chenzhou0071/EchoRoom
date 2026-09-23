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
      configured: false, // 编码器是否已 configure（首帧必配，不依赖尺寸恰好变化）
      n: 0,
      lastEncTs: 0, // R9：上次送入编码器的时刻（按档位帧率丢帧用）
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
        // R9：按档位目标帧率丢帧——WebCodecs 的 framerate 仅是码控提示，编码器不会自动丢帧。
        // 间隔不足目标周期×0.9 的帧丢弃（不画不编）；forceKey 帧（首帧/换档/观众到达）不节流，保证起步低延迟
        const nowTs = performance.now();
        if (!session.forceKey && nowTs - session.lastEncTs < (1000 / session.spec.framerate) * 0.9) {
          frame.close();
          continue;
        }
        session.lastEncTs = nowTs;
        // 首帧 / 档位切换后：按帧比例定 canvas 与编码尺寸（首帧必配置；尺寸相等也不能跳过配置）
        const { w, h } = targetSize(kind, frame);
        if (!session.configured || session.canvas.width !== w || session.canvas.height !== h) {
          session.canvas.width = w;
          session.canvas.height = h;
          configureEncoder(session, w, h);
          session.configured = true;
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
    // 用户从系统共享条点"停止共享"（守卫：只停当前会话——切换源后旧流的监听不得误杀新会话）
    stream.getVideoTracks()[0].addEventListener("ended", () => {
      if (sessions.get(STREAM_SCREEN)?.stream === stream) stop(STREAM_SCREEN);
    });
    await invoke("report_stream", { kind: STREAM_SCREEN, on: true }).catch(() => {});
    await invoke("set_share_active", { active: true }).catch(() => {});
  }

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

  // R4：切换投屏源——重开系统选择器选择新窗口/应用；新流就位后原子替换会话。
  // 不报 off/on：服务器流状态保持，观众端由新码流首帧 IDR 无缝续播；预览由调用方就地换源。
  async function switchScreen() {
    const old = sessions.get(STREAM_SCREEN);
    if (!old) return false;
    let stream;
    try {
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate: { ideal: 30 } },
        audio: false,
      });
    } catch (e) {
      console.warn("[video] 切换投屏源取消/失败:", e.message);
      return false; // 保留原流
    }
    sessions.delete(STREAM_SCREEN);
    old.stopped = true;
    try {
      await old.reader.cancel();
    } catch {}
    try {
      old.encoder.close();
    } catch {}
    old.stream.getTracks().forEach((t) => t.stop());
    await startPipeline(STREAM_SCREEN, stream);
    stream.getVideoTracks()[0].addEventListener("ended", () => {
      if (sessions.get(STREAM_SCREEN)?.stream === stream) stop(STREAM_SCREEN);
    });
    return true;
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
    window.videoView?.stopPreview?.(kind); // R2：预览中该路停止 → 视图同步移除；全停则退出
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
    setCameraDevice,
    restartCamera,
    switchScreen,
    stop,
    setQuality,
    forceKeyframe,
    setViewerCount,
    isActive: (kind) => sessions.has(kind),
    getStream: (kind) => sessions.get(kind)?.stream ?? null, // R2：自预览取流
  };
})();
