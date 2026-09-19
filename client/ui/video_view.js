// 观看视图：Rust 下行帧 → VideoDecoder（每路独立）→ canvas。
// 预览模式（R2）：本地采集流 → <video> 直接渲染（不经服务器，0 延迟）。
// 单路：高 230px 按比例居中；双路：投屏主画面 + 摄像头右下角小窗（可拖可缩、比例锁死）。
// 容器：#view（覆盖 #members 区域）内 #view-stage / #view-close / #view-fs / #view-settings / #view-viewers。
window.videoView = (() => {
  const { invoke, Channel } = window.__TAURI__.core;
  const STREAM_SCREEN = 0;
  const STREAM_CAMERA = 1;
  const DEC_CFG = { codec: "avc1.640028", avc: { format: "annexb" }, optimizeForLatency: true };

  let mode = null; // null | "watch"（看他人）| "preview"（自预览）
  let watchingUid = null; // 当前观看者（payload 带 uid 用于过滤切换残留）
  let keyTimer = null; // 秒开重试定时器
  let lastReqAt = 0; // request_keyframe 节流
  let lastViewers = 0; // R1：最近一次观看人数（视图显隐时复用）
  const gotKey = {}; // kind → 是否已见到关键帧（未见到前不喂 delta 帧）
  const decoders = {}; // kind → VideoDecoder
  const canvases = {}; // kind → canvas（观看模式）
  const previewVideos = {}; // kind → <video>（预览模式；srcObject = 本地采集流）

  const stage = () => document.getElementById("view-stage");
  const viewEl = () => document.getElementById("view");

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

  // 主/小窗布局：screen 存在时 camera 是小窗，否则都是主画面（观看/预览共用）
  function applyLayout() {
    const els = Object.assign({}, canvases, previewVideos);
    const hasScreen = !!els[STREAM_SCREEN];
    for (const key of Object.keys(els)) {
      const kind = Number(key);
      const el = els[key];
      const pip = kind === STREAM_CAMERA && hasScreen;
      el.classList.toggle("pip", pip);
      el.classList.toggle("main", !pip);
    }
  }

  // 小窗：左键拖动移位；右下角 20×20 区域拖拽缩放（宽 120–480px，比例由固有尺寸锁死）
  function makePipInteractive(canvas) {
    let dragMode = null;
    let sx = 0, sy = 0, sl = 0, st = 0, sw = 0;
    canvas.addEventListener("pointerdown", (e) => {
      if (!canvas.classList.contains("pip")) return;
      const r = canvas.getBoundingClientRect();
      const nearCorner = e.clientX > r.right - 20 && e.clientY > r.bottom - 20;
      dragMode = nearCorner ? "resize" : "move";
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
      if (!dragMode) return;
      const dx = e.clientX - sx;
      const dy = e.clientY - sy;
      if (dragMode === "move") {
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
      if (!dragMode) return;
      dragMode = null;
      try {
        canvas.releasePointerCapture(e.pointerId);
      } catch {}
    };
    canvas.addEventListener("pointerup", stopDrag);
    canvas.addEventListener("pointercancel", stopDrag);
  }

  // R1：观看人数徽标（× 右侧；0 或视图未开时隐藏）
  function setViewerCount(n) {
    lastViewers = n;
    const el = document.getElementById("view-viewers");
    const show = n > 0 && !viewEl().hidden;
    el.hidden = !show;
    if (show) el.textContent = n + " 人在看";
  }

  // R2：右下角按钮显隐——观看=⛶；预览且有投屏=⚙（#view.preview 同时控制小窗避让样式）
  function refreshButtons() {
    const isPreview = mode === "preview";
    viewEl().classList.toggle("preview", isPreview);
    document.getElementById("view-fs").hidden = isPreview;
    document.getElementById("view-settings").hidden = !(isPreview && previewVideos[STREAM_SCREEN]);
  }

  async function watch(uid) {
    await end();
    mode = "watch";
    watchingUid = uid;
    let ch;
    try {
      ch = new Channel(onFrame);
      await invoke("watch_start", { ch, target: uid });
    } catch (e) {
      console.warn("[view] watch_start 失败:", e);
      mode = null;
      watchingUid = null;
      return;
    }
    viewEl().hidden = false;
    refreshButtons();
    setViewerCount(lastViewers); // 视图刚显示：重评估徽标
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
    const wasWatch = mode === "watch";
    mode = null;
    watchingUid = null;
    if (wasWatch) {
      try {
        await invoke("watch_stop");
      } catch {}
    }
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
    for (const k of Object.keys(previewVideos)) {
      previewVideos[k].srcObject = null;
      previewVideos[k].remove();
      delete previewVideos[k];
    }
    document.getElementById("view-viewers").hidden = true;
    refreshButtons();
    viewEl().hidden = true;
  }

  // R2：自预览——把本地采集流挂进视图（不经服务器；重复调用追加/替换某一路）
  async function preview(kind, stream) {
    if (mode !== "preview") await end(); // 与观看互斥（预览中追加第二路则不清场）
    mode = "preview";
    const old = previewVideos[kind];
    if (old) {
      old.srcObject = null;
      old.remove();
      delete previewVideos[kind];
    }
    const v = document.createElement("video");
    v.className = "view-canvas";
    v.autoplay = true;
    v.muted = true;
    v.playsInline = true;
    v.srcObject = stream;
    if (kind === STREAM_CAMERA) makePipInteractive(v);
    stage().appendChild(v);
    previewVideos[kind] = v;
    v.play().catch(() => {});
    applyLayout();
    viewEl().hidden = false;
    refreshButtons();
    setViewerCount(lastViewers); // 视图刚显示：重评估徽标
  }

  // R2：预览中某路停止（video_capture.stop 通知）；两路全停 → 退出视图
  function stopPreview(kind) {
    const v = previewVideos[kind];
    if (v) {
      v.srcObject = null;
      v.remove();
      delete previewVideos[kind];
    }
    if (mode !== "preview") return;
    if (Object.keys(previewVideos).length === 0) {
      end();
    } else {
      applyLayout();
      refreshButtons();
    }
  }

  // 对方某路流停止（观看模式）：移除对应画面；两路都没了 → 退出观看
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
  document.getElementById("view-close").addEventListener("click", () => {
    if (mode === "preview") {
      // R2：× = 停止共享（单路停那路；双路全部停止；视图退出由停止流程驱动）
      const vc = window.videoCapture;
      if (vc?.isActive(STREAM_SCREEN)) vc.stop(STREAM_SCREEN);
      if (vc?.isActive(STREAM_CAMERA)) vc.stop(STREAM_CAMERA);
    } else {
      end(); // 观众：退订退出
    }
  });
  document.getElementById("view-fs").addEventListener("click", () => toggleFullscreen());
  // #view-settings（⚙）点击由 app.js 接线打开投屏设置面板（Task 10）

  return {
    watch,
    end,
    onStreamOff,
    preview,
    stopPreview,
    setViewerCount,
    toggleFullscreen,
    get watchingUid() {
      return watchingUid;
    },
  };
})();
