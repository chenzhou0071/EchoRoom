// EchoRoom 前端：tauri 事件 → DOM 渲染；操作经 invoke 发给 Rust。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---- 状态 ----
const members = new Map(); // uid → { nickname, speaking, muted, streams }
let myUid = null; // 自己的 uid（self_uid 事件，先于 members 到达）
let volState = { self_gain: 1.0, muted: false, peer_gains: {} }; // Rust 侧音量真值的本地副本
// B：投屏设置本地副本（Rust config 为真值）
let shareQuality = "720p30";
let shareAudioOn = true;
let screenAudioOk = false; // Win11 22000+；init 时查询
let lastViewerUids = []; // R1：最近一次观看名单（投屏面板"正在观看"行）

// C：认证与资料
let authMode = "login"; // "login" | "register"
let lastAuthAttempt = null; // 最近一次提交的意图（auth_ok 判断是否弹资料窗）
let profileSubmitting = false; // 资料提交中：等 profile_changed 广播回来才关弹窗

// C：头像
const avatarCache = new Map(); // uid → Blob URL；null = 已确认无头像
const avatarPending = new Set(); // 已请求未回复的 uid（防重入）
let pendingAvatar = null; // 资料弹窗待提交的新头像（压缩后 Uint8Array）
let previewUrl = null; // 弹窗预览的临时 Blob URL（避免泄漏）

// ---- 图标（feather 风格内联 SVG，currentColor 随按钮状态变色）----
const ICONS = {
  mic: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  micOff: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="1" y1="1" x2="23" y2="23"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6"/><path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  volume: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>',
  screen: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
  cam: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="23 7 16 12 23 17 23 7"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/></svg>',
  play: '<svg viewBox="0 0 24 24" fill="currentColor"><polygon points="6 4 20 12 6 20 6 4"/></svg>',
  avatar: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-5.33 0-9 2.67-9 6v2h18v-2c0-3.33-3.67-6-9-6z"/></svg>',
  gear: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
};

// ---- 小工具 ----
const el = (id) => document.getElementById(id);

// D：功能开关（features.js 提供；缺文件/缺项按全开）
const FEATURES = Object.assign({ sound: true, theme: true, background: true }, window.FEATURES || {});

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

function buildCard(uid, m) {
  const isSelf = uid === myUid;
  const div = document.createElement("div");
  div.className = "member" + (m.speaking ? " speaking" : "") + (!isSelf && m.muted ? " muted" : "");
  div.dataset.uid = uid;

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
  // 直播中的纱（中央播放按钮）：他人=观看对方流（再点退出）；自己=回到自预览（R5）
  if (m.streams) {
    const veil = document.createElement("div");
    veil.className = "member-veil";
    const watch = document.createElement("button");
    watch.className = "watch-btn";
    watch.title = isSelf ? "自预览" : "观看";
    watch.innerHTML = ICONS.play;
    watch.addEventListener("click", (e) => {
      e.stopPropagation();
      if (isSelf) {
        if (videoView.isPreviewing) {
          videoView.end();
          return;
        }
        // R8：全部活跃流一起进自预览（投屏主画面 + 摄像头小窗）
        enterSelfPreview();
      } else if (videoView.watchingUid === uid) {
        videoView.end();
      } else {
        videoView.watch(uid);
      }
    });
    veil.appendChild(watch);
    avatar.appendChild(veil);
  }
  // 他人直播角标（自己的状态由卡片按钮颜色体现）
  if (!isSelf && m.streams) {
    const badge = document.createElement("div");
    badge.className = "member-badge";
    badge.textContent = m.streams === 3 ? "投屏+摄像头" : m.streams & 1 ? "投屏中" : "摄像头中";
    avatar.appendChild(badge);
  }
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
    const screenOn = videoCapture.isActive(videoCapture.STREAM_SCREEN);
    const screenBtn = document.createElement("button");
    screenBtn.className = "mbtn mbtn-screen" + (screenOn ? " active acc" : "");
    screenBtn.title = screenOn ? "投屏设置" : "开始投屏";
    screenBtn.innerHTML = ICONS.screen;
    screenBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const btn = e.currentTarget; // await 后 currentTarget 失效，先取节点
      if (videoCapture.isActive(videoCapture.STREAM_SCREEN)) {
        // 投屏中（预览视图通常盖住卡片）：按钮 = 面板开关
        if (sharePop.classList.contains("show")) closeSharePop();
        else openSharePop(btn);
      } else {
        // 未投屏：起投屏（系统选择器）→ 成功即自动进入自预览（R2；不自动弹面板）
        await videoCapture.startScreen();
        renderMembers(); // 重建卡片（旧节点已脱离 DOM，不能用作定位锚点）
        if (videoCapture.isActive(videoCapture.STREAM_SCREEN)) enterSelfPreview(); // R8：连同另一路活跃流一起进
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
        await videoCapture.stop(videoCapture.STREAM_CAMERA); // 预览退出由流停止逻辑驱动（R2）
      } else {
        await videoCapture.startCamera();
        if (videoCapture.isActive(videoCapture.STREAM_CAMERA)) enterSelfPreview(); // R8：连同另一路活跃流一起进
      }
      renderMembers();
    });
    btns.appendChild(camBtn);
  }
  bar.appendChild(btns);
  div.appendChild(bar);
  return div;
}

function renderMembers() {
  const box = el("members");
  const view = el("view"); // 观看视图覆盖层：重建卡片时保留原节点（video_view.js 的引用与监听器不失效）
  box.innerHTML = "";
  if (view) box.appendChild(view);
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

// ---- 音量弹出面板（单例） ----
const volPop = document.createElement("div");
volPop.className = "vol-pop";
volPop.innerHTML = '<div class="val"></div><input type="range" min="0" max="400" step="1" />';
document.body.appendChild(volPop);
const volVal = volPop.querySelector(".val");
const volSlider = volPop.querySelector('input[type="range"]');
let volTarget = null; // { uid, nickname } 当前面板目标

function openVolPop(anchor, uid, nickname) {
  closeScreenVolPop(); // R4：与投屏音量面板互斥
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

// ---- R4：投屏（屏幕）音量弹出面板（观看端；单例，样式复用 .vol-pop） ----
el("view-vol").innerHTML = ICONS.volume;
const screenVolPop = document.createElement("div");
screenVolPop.className = "vol-pop screen-vol-pop";
screenVolPop.innerHTML = '<div class="val"></div><input type="range" min="0" max="400" step="1" />';
document.body.appendChild(screenVolPop);
const screenVolVal = screenVolPop.querySelector(".val");
const screenVolSlider = screenVolPop.querySelector('input[type="range"]');
let screenGain = 1.0; // Rust config 为真值，本地副本供弹层回显

function openScreenVolPop(anchor) {
  closeVolPop(); // 互斥：收掉成员音量面板
  screenVolSlider.value = String(Math.round(screenGain * 100));
  screenVolVal.textContent = screenVolSlider.value + "%";
  const r = anchor.getBoundingClientRect();
  screenVolPop.classList.add("show");
  const w = screenVolPop.offsetWidth;
  const h = screenVolPop.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - w - 8);
  let top = r.bottom + 6;
  if (top + h > window.innerHeight - 8) top = r.top - h - 6;
  screenVolPop.style.left = left + "px";
  screenVolPop.style.top = top + "px";
}

function closeScreenVolPop() {
  screenVolPop.classList.remove("show");
}

el("view-vol").addEventListener("click", (e) => {
  e.stopPropagation();
  if (screenVolPop.classList.contains("show")) closeScreenVolPop();
  else openScreenVolPop(e.currentTarget);
});

screenVolSlider.addEventListener("input", () => {
  screenVolVal.textContent = screenVolSlider.value + "%";
  const gain = Number(screenVolSlider.value) / 100;
  screenGain = gain;
  invoke("set_screen_gain", { gain }).catch((e) => console.error("设置投屏音量失败:", e));
});

// ---- 设置面板：投屏 + 摄像头（单例；预览视图右下角 ⚙ 或投屏中卡片按钮开关） ----
const sharePop = document.createElement("div");
sharePop.className = "share-pop";
sharePop.innerHTML = `
  <div class="sp-title">投屏</div>
  <div class="sp-screen-opts" id="sp-screen-opts">
    <label class="sp-row"><input type="radio" name="sp-quality" value="720p30" />720p 30fps</label>
    <label class="sp-row"><input type="radio" name="sp-quality" value="1080p15" />1080p 15fps</label>
    <label class="sp-row"><input type="radio" name="sp-quality" value="1080p30" />1080p 30fps</label>
    <label class="sp-row"><input type="checkbox" id="sp-audio" />共享系统声音<span class="sp-hint" id="sp-audio-hint"></span></label>
    <div class="sp-viewers">正在观看：<span id="sp-viewers">暂无</span></div>
    <button class="sp-switch" id="sp-switch">切换投屏窗口</button>
  </div>
  <button class="sp-stop" id="sp-stop">停止投屏</button>
  <div class="sp-sep"></div>
  <div class="sp-title">摄像头</div>
  <button class="sp-cam" id="sp-cam">开启摄像头</button>
`;
document.body.appendChild(sharePop);

// R1：投屏面板"正在观看"行（uid → 昵称；空 → 暂无）
function updateShareViewers(uids) {
  lastViewerUids = uids;
  sharePop.querySelector("#sp-viewers").textContent =
    uids.length === 0 ? "暂无" : uids.map((u) => nicknameOf(u)).join("、");
}

function applyShareAudioUi() {
  const cb = sharePop.querySelector("#sp-audio");
  cb.checked = shareAudioOn && screenAudioOk;
  cb.disabled = !screenAudioOk;
  sharePop.querySelector("#sp-audio-hint").textContent = screenAudioOk ? "" : "（需 Win11）";
}

// R3：内容变高后保持面板底边不越出视口下缘
function clampSharePop() {
  const h = sharePop.offsetHeight;
  const top = parseFloat(sharePop.style.top);
  if (Number.isNaN(top)) return; // 尚未定位过
  if (top + h > window.innerHeight - 8) {
    sharePop.style.top = Math.max(8, window.innerHeight - 8 - h) + "px";
  }
}

// R8：进入自预览——把当前所有活跃本地流逐路挂进视图（screen 主画面 + camera 小窗）。
// 修复「先开一路 → ←返回 → 再开另一路」只挂新流、另一路不显示的问题
function enterSelfPreview() {
  const sc = videoCapture.getStream(videoCapture.STREAM_SCREEN);
  if (sc) videoView.preview(videoCapture.STREAM_SCREEN, sc);
  const cm = videoCapture.getStream(videoCapture.STREAM_CAMERA);
  if (cm) videoView.preview(videoCapture.STREAM_CAMERA, cm);
}

// R3：面板按流状态刷新——未投屏时投屏区只留"开启投屏"按钮；两路按钮文案随开关状态
function syncSharePop() {
  const screenOn = videoCapture.isActive(videoCapture.STREAM_SCREEN);
  const camOn = videoCapture.isActive(videoCapture.STREAM_CAMERA);
  sharePop.querySelector("#sp-screen-opts").hidden = !screenOn;
  const stopBtn = sharePop.querySelector("#sp-stop");
  stopBtn.textContent = screenOn ? "停止投屏" : "开启投屏";
  stopBtn.classList.toggle("start", !screenOn);
  const camBtn = sharePop.querySelector("#sp-cam");
  camBtn.textContent = camOn ? "关闭摄像头" : "开启摄像头";
  camBtn.classList.toggle("danger", camOn);
  if (sharePop.classList.contains("show")) clampSharePop();
}

function openSharePop(anchor) {
  syncSharePop(); // 先按流状态定显隐/文案，再测尺寸定位
  for (const r of sharePop.querySelectorAll('input[name="sp-quality"]')) {
    r.checked = r.value === shareQuality;
  }
  applyShareAudioUi();
  updateShareViewers(lastViewerUids);
  const r = anchor.getBoundingClientRect();
  sharePop.classList.add("show");
  const w = sharePop.offsetWidth;
  const h = sharePop.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - w - 8);
  let top = r.bottom + 6;
  // R7：只朝下——永远从按钮下方展开；下缘超出视口时仅上顶最少距离（不整块翻到按钮上方遮画面）
  if (top + h > window.innerHeight - 8) top = Math.max(8, window.innerHeight - 8 - h);
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

// R4：切换投屏源（重开系统选择器；观众不掉线——服务器流状态不变，预览就地换源）
sharePop.querySelector("#sp-switch").addEventListener("click", async () => {
  const ok = await videoCapture.switchScreen();
  if (!ok) return; // 取消选择：保留原流
  if (videoView.isPreviewing) {
    videoView.preview(videoCapture.STREAM_SCREEN, videoCapture.getStream(videoCapture.STREAM_SCREEN));
  }
});

sharePop.querySelector("#sp-stop").addEventListener("click", async () => {
  if (videoCapture.isActive(videoCapture.STREAM_SCREEN)) {
    closeSharePop();
    await videoCapture.stop(videoCapture.STREAM_SCREEN); // 预览退出由流停止逻辑驱动（R2）
    renderMembers();
    return;
  }
  // R3：未投屏时同一按钮 = 开启投屏（与卡片投屏按钮同流程）
  await videoCapture.startScreen();
  renderMembers(); // 重建卡片（旧节点已脱离 DOM，不能用作定位锚点）
  if (videoCapture.isActive(videoCapture.STREAM_SCREEN)) enterSelfPreview(); // R8：连同另一路活跃流一起进
  syncSharePop();
});

// R3：面板内摄像头开关（自预览中卡片被盖时也能开关摄像头）
sharePop.querySelector("#sp-cam").addEventListener("click", async () => {
  if (videoCapture.isActive(videoCapture.STREAM_CAMERA)) {
    await videoCapture.stop(videoCapture.STREAM_CAMERA); // 预览退出由流停止逻辑驱动（R2）
  } else {
    await videoCapture.startCamera();
    if (videoCapture.isActive(videoCapture.STREAM_CAMERA)) enterSelfPreview(); // R8：连同另一路活跃流一起进
  }
  renderMembers();
  syncSharePop();
});

// R2：预览视图右下角 ⚙（#view-settings）→ 开关投屏设置面板（锚定该按钮，浮层贴右下）
document.getElementById("view-settings").addEventListener("click", (e) => {
  e.stopPropagation();
  if (sharePop.classList.contains("show")) closeSharePop();
  else openSharePop(e.currentTarget);
});

document.addEventListener("click", (e) => {
  if (!e.target.closest(".vol-pop") && !e.target.closest(".mbtn-vol")) closeVolPop();
  if (!e.target.closest(".screen-vol-pop") && !e.target.closest("#view-vol")) closeScreenVolPop();
  if (!e.target.closest(".share-pop") && !e.target.closest(".mbtn-screen")) closeSharePop();
});

// ---- R11：正文链接识别（http(s):// 与 www.*；尾部标点不吞；点击用系统浏览器打开） ----
const LINK_RE = /(?:https?:\/\/|www\.)[A-Za-z0-9\-._~:\/?#\[\]@!$&()*+,;=%]+/gi;
const LINK_TAIL_PUNCT = ".,;:!?)]}";

// 剥掉误吞的尾部标点（中文全角标点本就进不了 URL 字符集）；成对括号（如 /wiki/Foo_(bar)）不剥
function stripUrlTail(raw) {
  let url = raw;
  while (url.length > 0) {
    const c = url[url.length - 1];
    if (!LINK_TAIL_PUNCT.includes(c)) break;
    if (c === ")" && url.split("(").length >= url.split(")").length) break;
    if (c === "]" && url.split("[").length >= url.split("]").length) break;
    url = url.slice(0, -1);
  }
  return url;
}

// 正文按节点拼装（不走 innerHTML）：纯文本 textNode + 链接 <a.chat-link>
function linkify(text, into) {
  let last = 0;
  for (const m of text.matchAll(LINK_RE)) {
    const raw = m[0];
    const url = stripUrlTail(raw);
    if (!url) continue;
    if (m.index > last) into.appendChild(document.createTextNode(text.slice(last, m.index)));
    const a = document.createElement("a");
    a.className = "chat-link";
    a.textContent = url;
    const target = /^www\./i.test(url) ? "https://" + url : url; // www. 开头补协议
    a.addEventListener("click", () => {
      invoke("open_url", { url: target }).catch((e) => console.error("打开链接失败:", e));
    });
    into.appendChild(a);
    const tail = raw.slice(url.length);
    if (tail) into.appendChild(document.createTextNode(tail));
    last = m.index + raw.length;
  }
  if (last < text.length) into.appendChild(document.createTextNode(text.slice(last)));
}

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
  linkify(text, body); // R11：正文链接标蓝可点
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
  el("open-settings").hidden = !usable;
  if (!usable) closeSettings(); // 断线/被拒：设置页一并收起，交还登录/重连流程
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

function submitProfile() {
  const nickname = el("profile-nickname").value.trim();
  if (!nickname) {
    showProfileError("请填写昵称（1-24 字符）");
    return;
  }
  profileSubmitting = true;
  el("profile-submit").disabled = true;
  showProfileError("");
  invoke("set_profile", { nickname, avatar: pendingAvatar ? Array.from(pendingAvatar) : null }).catch((e) => {
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

// ---- D：设置页（顶栏 ⚙ 覆盖式；分类切换 + 版本号） ----
const settingsPage = el("settings-page");
const paneRefreshers = {}; // cat → 刷新钩子（各页面任务注册：paneRefreshers.device = refreshDevices 等）

function switchCat(cat) {
  for (const btn of settingsPage.querySelectorAll(".settings-cat")) {
    btn.classList.toggle("active", btn.dataset.cat === cat);
  }
  for (const pane of settingsPage.querySelectorAll(".settings-pane")) {
    pane.hidden = pane.dataset.pane !== cat;
  }
  const fn = paneRefreshers[cat];
  if (fn) fn();
}

function openSettings() {
  settingsPage.hidden = false;
  const active = settingsPage.querySelector(".settings-cat.active");
  switchCat(active ? active.dataset.cat : "account");
}

function closeSettings() {
  settingsPage.hidden = true;
}

el("open-settings").innerHTML = ICONS.gear;
el("open-settings").addEventListener("click", openSettings);
el("settings-back").addEventListener("click", closeSettings);
for (const btn of settingsPage.querySelectorAll(".settings-cat")) {
  btn.addEventListener("click", () => switchCat(btn.dataset.cat));
}

// D：功能开关——关闭的分类整个隐藏（features.js；默认全开时无副作用）
if (!FEATURES.sound) settingsPage.querySelector('.settings-cat[data-cat="sound"]').hidden = true;
if (!FEATURES.theme && !FEATURES.background) settingsPage.querySelector('.settings-cat[data-cat="appearance"]').hidden = true;

// ---- 事件接线与启动 ----
async function init() {
  // 先注册监听器，再发起连接：保证事件不因时序竞态丢失
  await listen("self_uid", (e) => {
    myUid = e.payload;
    renderMembers(); // 幂等：myUid 变化后重渲染（区分自己/他人卡片）
  });
  await listen("members", (e) => {
    // 全量列表（首次/重连）：uid 可能被重新分配，头像缓存全部作废
    for (const url of avatarCache.values()) if (url) URL.revokeObjectURL(url);
    avatarCache.clear();
    avatarPending.clear();
    members.clear();
    for (const [uid, nickname, muted, streams, hasAvatar] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted, streams, hasAvatar });
    }
    renderMembers();
    renderOnline();
    if (!everEntered) {
      everEntered = true;
      playSnd(sndIn); // 自己首次进入；之后的重连同步不播
    }
  });
  await listen("member_join", (e) => {
    const [uid, nickname, hasAvatar] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false, streams: 0, hasAvatar });
    renderMembers();
    renderOnline();
    playSnd(sndIn); // 别人进入
  });
  await listen("member_leave", (e) => {
    const uid = e.payload;
    if (videoView.watchingUid === uid) videoView.end(); // 被观看者离开：订阅已随其退出失效，直接收尾
    const url = avatarCache.get(uid);
    if (url) URL.revokeObjectURL(url);
    avatarCache.delete(uid);
    avatarPending.delete(uid);
    members.delete(uid);
    renderMembers();
    renderOnline();
    playSnd(sndOut); // 别人退出（自己退出不播：本客户端不会收到自己的 leave 广播）
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
  // R1/R2（T9 先行）：观看名单——人数驱动投屏门控（0→N 恢复编码+强制关键帧）与观看视图徽标
  await listen("viewer_count", (e) => {
    const uids = e.payload; // uid 数组，人数 = length
    videoCapture.setViewerCount(uids.length);
    videoView.setViewerCount(uids.length);
    updateShareViewers(uids); // R1：投屏面板"正在观看"行
  });
  // 观看端请求关键帧（秒开/解码恢复）→ 本端强制下一帧为 IDR
  await listen("request_keyframe", () => videoCapture.forceKeyframe());
  await listen("stream_state", (e) => {
    const { uid, kind, on } = e.payload;
    const m = members.get(uid);
    if (m) m.streams = on ? m.streams | (1 << kind) : m.streams & ~(1 << kind);
    renderMembers();
    // R3：面板管投屏+摄像头两路——随流状态刷新；两路全停（预览退出）才收面板
    if (uid === myUid) {
      const anyOn =
        videoCapture.isActive(videoCapture.STREAM_SCREEN) || videoCapture.isActive(videoCapture.STREAM_CAMERA);
      if (!anyOn) closeSharePop();
      else if (sharePop.classList.contains("show")) syncSharePop();
    }
    // 正在观看的人停了一路：移除对应画面（两路全停则视图自动退出）
    if (videoView.watchingUid === uid && !on) videoView.onStreamOff(kind);
  });
  await listen("share_audio", (e) => {
    shareAudioOn = e.payload;
    applyShareAudioUi();
  });
  await listen("conn", (e) => {
    setConn(e.payload);
    if (e.payload === "reconnecting") {
      videoView.end(); // 连接断开：正在观看的流必然中断
      videoCapture.setViewerCount(0); // 观众数随旧会话清零；重连后由服务器推回
    }
  });

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
    forceRequestAvatar(uid); // 头像可能同期更新：绕过 has_avatar 快捷路径强制重拉
    if (profileSubmitting && uid === myUid) closeProfilePop(); // 自己保存成功：广播回来才关窗
  });
  await listen("profile_error", (e) => {
    if (!profileSubmitting) return;
    profileSubmitting = false;
    el("profile-submit").disabled = false;
    showProfileError(String(e.payload));
  });
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

  const cfg = await invoke("get_config");
  // D：设置页版本号（取打包版本；失败保留静态占位）
  window.__TAURI__.app
    .getVersion()
    .then((v) => {
      el("settings-version").textContent = "v" + v;
    })
    .catch(() => {});
  // 音量真值来自 Rust（config 持久化）：初始化本地副本
  volState = { self_gain: cfg.self_gain, muted: cfg.muted, peer_gains: cfg.peer_gains };
  screenGain = cfg.screen_gain ?? 1.0; // R4：投屏音量（观看端）
  // B：投屏配置（档位应用给采集模块；声音开关/平台能力供面板显示）
  shareQuality = cfg.share_quality || "720p30";
  videoCapture.setQuality(shareQuality);
  shareAudioOn = cfg.share_audio !== false;
  screenAudioOk = await invoke("screen_audio_supported").catch(() => false);
  applyShareAudioUi();
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
}

init().catch((e) => console.error("初始化失败:", e));
