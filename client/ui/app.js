// EchoRoom 前端：tauri 事件 → DOM 渲染；操作经 invoke 发给 Rust。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---- 状态 ----
const members = new Map(); // uid → { nickname, speaking }

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

// ---- 渲染：成员（顶部格子 + 左栏在线名单共用同一数据） ----
function sortedMembers() {
  return [...members.entries()].sort((a, b) => a[0] - b[0]); // 按 uid 升序 = 进入顺序
}

function renderMembers() {
  const box = el("members");
  box.innerHTML = "";
  for (const [uid, m] of sortedMembers()) {
    const div = document.createElement("div");
    div.className = "member" + (m.speaking ? " speaking" : "");
    div.dataset.uid = uid;
    div.textContent = m.nickname;
    box.appendChild(div);
  }
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
  await listen("members", (e) => {
    members.clear();
    for (const [uid, nickname] of e.payload) members.set(uid, { nickname, speaking: false });
    renderMembers();
    renderOnline();
  });
  await listen("member_join", (e) => {
    const [uid, nickname] = e.payload;
    members.set(uid, { nickname, speaking: false });
    renderMembers();
    renderOnline();
  });
  await listen("member_leave", (e) => {
    members.delete(e.payload);
    renderMembers();
    renderOnline();
  });
  await listen("chat", (e) => appendMessage(e.payload.uid, e.payload.text));
  await listen("speaking", (e) => {
    const { uid, on } = e.payload;
    const m = members.get(uid);
    if (m) m.speaking = on;
    const card = document.querySelector(`.member[data-uid="${uid}"]`);
    if (card) card.classList.toggle("speaking", on);
  });
  await listen("conn", (e) => setConn(e.payload));

  const cfg = await invoke("get_config");
  if (!cfg.nickname) {
    el("setup-mask").classList.remove("hidden");
    el("setup-nickname").focus();
  } else {
    await invoke("connect"); // 打开即自动连接
  }
}

init().catch((e) => console.error("初始化失败:", e));
