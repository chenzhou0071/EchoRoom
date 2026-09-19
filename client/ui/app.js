// EchoRoom 前端：tauri 事件 → DOM 渲染；操作经 invoke 发给 Rust。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---- 状态 ----
const members = new Map(); // uid → { nickname, speaking, muted }
let myUid = null; // 自己的 uid（self_uid 事件，先于 members 到达）
let volState = { self_gain: 1.0, muted: false, peer_gains: {} }; // Rust 侧音量真值的本地副本

// ---- 图标（feather 风格内联 SVG，currentColor 随按钮状态变色）----
const ICONS = {
  mic: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  micOff: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="1" y1="1" x2="23" y2="23"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6"/><path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23"/><line x1="12" y1="19" x2="12" y2="23"/></svg>',
  volume: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>',
  screen: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
  cam: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="23 7 16 12 23 17 23 7"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/></svg>',
  avatar: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-5.33 0-9 2.67-9 6v2h18v-2c0-3.33-3.67-6-9-6z"/></svg>',
};

// ---- 小工具 ----
const el = (id) => document.getElementById(id);

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

function buildCard(uid, m) {
  const isSelf = uid === myUid;
  const div = document.createElement("div");
  div.className = "member" + (m.speaking ? " speaking" : "") + (!isSelf && m.muted ? " muted" : "");
  div.dataset.uid = uid;

  const avatar = document.createElement("div");
  avatar.className = "member-avatar";
  avatar.innerHTML = ICONS.avatar;
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
volPop.innerHTML = '<div class="val"></div><input type="range" min="0" max="200" step="1" />';
document.body.appendChild(volPop);
const volVal = volPop.querySelector(".val");
const volSlider = volPop.querySelector('input[type="range"]');
let volTarget = null; // { uid, nickname } 当前面板目标

function openVolPop(anchor, uid, nickname) {
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

document.addEventListener("click", (e) => {
  if (!e.target.closest(".vol-pop") && !e.target.closest(".mbtn-vol")) closeVolPop();
});

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
  body.textContent = text;
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

// ---- 首次启动：昵称 / 服务器设置面板 ----
async function submitSetup() {
  const nickname = el("setup-nickname").value.trim();
  const serverAddr = el("setup-server").value.trim();
  if (!nickname || !serverAddr) return;
  try {
    await invoke("set_config", { nickname, serverAddr }); // 保存后 Rust 侧自动发起连接
    el("setup-mask").classList.add("hidden");
  } catch (e) {
    console.error("保存配置失败:", e);
  }
}

el("setup-connect").addEventListener("click", submitSetup);
for (const id of ["setup-nickname", "setup-server"]) {
  el(id).addEventListener("keydown", (e) => {
    if (e.key === "Enter") submitSetup();
  });
}

// ---- 事件接线与启动 ----
async function init() {
  // 先注册监听器，再发起连接：保证事件不因时序竞态丢失
  await listen("self_uid", (e) => {
    myUid = e.payload;
    renderMembers(); // 幂等：myUid 变化后重渲染（区分自己/他人卡片）
  });
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname, muted] of e.payload) {
      members.set(uid, { nickname, speaking: false, muted });
    }
    renderMembers();
    renderOnline();
    if (!everEntered) {
      everEntered = true;
      playSnd(sndIn); // 自己首次进入；之后的重连同步不播
    }
  });
  await listen("member_join", (e) => {
    const [uid, nickname] = e.payload;
    members.set(uid, { nickname, speaking: false, muted: false });
    renderMembers();
    renderOnline();
    playSnd(sndIn); // 别人进入
  });
  await listen("member_leave", (e) => {
    members.delete(e.payload);
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
  });
  // 观看端请求关键帧（秒开/解码恢复）→ 本端强制下一帧为 IDR
  await listen("request_keyframe", () => videoCapture.forceKeyframe());
  await listen("conn", (e) => setConn(e.payload));

  const cfg = await invoke("get_config");
  // 音量真值来自 Rust（config 持久化）：初始化本地副本
  volState = { self_gain: cfg.self_gain, muted: cfg.muted, peer_gains: cfg.peer_gains };
  if (!cfg.nickname) {
    el("setup-mask").classList.remove("hidden");
    el("setup-nickname").focus();
  } else {
    await invoke("connect"); // 打开即自动连接
  }
}

init().catch((e) => console.error("初始化失败:", e));
