// T7 静态壳：假数据渲染（T8 将接入真实网络数据，替换 FAKE_* 数据源）。
(function () {
  const FAKE_MEMBERS = [
    { uid: 1, nickname: "阿信", speaking: true },
    { uid: 2, nickname: "小K", speaking: false },
    { uid: 3, nickname: "小林", speaking: false },
    { uid: 4, nickname: "小美", speaking: false },
    { uid: 5, nickname: "小北", speaking: false },
    { uid: 6, nickname: "老周", speaking: false },
  ];
  const FAKE_MESSAGES = [
    { uid: 1, time: "20:12", text: "晚上开黑吗？" },
    { uid: 2, time: "20:13", text: "在的，吃完饭就来" },
    { uid: 3, time: "20:15", text: "我八点到家" },
  ];
  const NICKNAMES = { 1: "阿信", 2: "小K", 3: "小林", 4: "小美", 5: "小北", 6: "老周" };

  function renderMembers(list) {
    const box = document.getElementById("members");
    box.innerHTML = "";
    for (const m of list) {
      const div = document.createElement("div");
      div.className = "member" + (m.speaking ? " speaking" : "");
      div.textContent = m.nickname;
      box.appendChild(div);
    }
  }

  function renderMessages(list) {
    const box = document.getElementById("chat");
    box.innerHTML = "";
    for (const m of list) {
      const div = document.createElement("div");
      div.className = "msg";
      const meta = document.createElement("div");
      meta.className = "msg-meta";
      const name = document.createElement("span");
      name.className = "name";
      name.textContent = (NICKNAMES[m.uid] || `uid=${m.uid}`) + "：";
      meta.appendChild(name);
      const time = document.createElement("span");
      time.className = "time";
      time.textContent = m.time;
      meta.appendChild(time);
      div.appendChild(meta);
      const text = document.createElement("div");
      text.className = "msg-text";
      text.textContent = m.text;
      div.appendChild(text);
      box.appendChild(div);
    }
  }

  function renderOnline(list) {
    const box = document.getElementById("online-list");
    box.innerHTML = "";
    for (const m of list) {
      const li = document.createElement("li");
      li.className = "online-item";
      li.textContent = m.nickname;
      box.appendChild(li);
    }
  }

  renderMembers(FAKE_MEMBERS);
  renderMessages(FAKE_MESSAGES);
  renderOnline(FAKE_MEMBERS);
})();
