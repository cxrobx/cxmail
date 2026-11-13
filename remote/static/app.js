"use strict";
// CXMail Remote. Plain JS, no build step.
// Routes: #/ (list), #/m/<acct>/<folder>/<uid> (reader), #/send/<acct>/<folder>/<uid> (review & send).

const $ = (s) => document.querySelector(s);
// Inbox chips. The bottom tabs pick the section; chips only show on Inbox.
const CHIPS = [["primary", "Primary"], ["starred", "Starred"], ["updates", "Updates"], ["promotions", "Promotions"], ["social", "Social"], ["all", "All"]];
const TABS = [["inbox", "Inbox", "inbox"], ["unread", "Unread", "unread"], ["drafts", "Drafts", "drafts"], ["search", "Search", "search"], ["new", "New", "compose"]];
const TITLES = { inbox: "Inbox", unread: "Unread", drafts: "Drafts", search: "Search" };

// Lucide-style 24px stroke icons.
const ICONS = {
  inbox: '<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  unread: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-10 6L2 7"/><circle cx="19" cy="5" r="3" fill="currentColor"/>',
  drafts: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5z"/><path d="M14 2v6h6"/><path d="M10 13h4M10 17h4"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
  compose: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  back: '<path d="m15 18-6-6 6-6"/>',
  archive: '<rect x="2" y="3" width="20" height="5" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
  star: '<path d="m12 2 3.1 6.3 6.9 1-5 4.9 1.2 6.8L12 17.8 5.8 21l1.2-6.8-5-4.9 6.9-1z"/>',
  starOn: '<path fill="currentColor" d="m12 2 3.1 6.3 6.9 1-5 4.9 1.2 6.8L12 17.8 5.8 21l1.2-6.8-5-4.9 6.9-1z"/>',
  mail: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-10 6L2 7"/>',
  mailOpen: '<path d="M21.2 8.4c.5.38.8.97.8 1.6v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V10a2 2 0 0 1 .8-1.6l8-6a2 2 0 0 1 2.4 0l8 6Z"/><path d="m22 10-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 10"/>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  reply: '<path d="m9 17-5-5 5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>',
  replyAll: '<path d="m7 17-5-5 5-5"/><path d="m12 17-5-5 5-5"/><path d="M22 18v-2a4 4 0 0 0-4-4H7"/>',
  trash: '<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M10 11v6M14 11v6"/>',
  sparkle: '<path d="M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.13-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.13a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.13 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.13a.5.5 0 0 1-.96 0z"/>',
  send: '<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>',
  chat: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
  up: '<path d="M12 19V5"/><path d="m5 12 7-7 7 7"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor"/>',
};
function icon(name) {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("viewBox", "0 0 24 24"); s.setAttribute("fill", "none"); s.setAttribute("stroke", "currentColor");
  s.setAttribute("stroke-width", "2"); s.setAttribute("stroke-linecap", "round"); s.setAttribute("stroke-linejoin", "round");
  s.setAttribute("aria-hidden", "true");
  s.innerHTML = ICONS[name] || "";
  return s;
}

const state = { tab: "inbox", chip: "primary", account: "", q: "", rows: [], accounts: [], done: false, loading: false, current: null, seq: 0 };
try {
  state.chip = localStorage.getItem("cxr.chip") || "primary";
  state.account = localStorage.getItem("cxr.acct") || "";
  state.tab = localStorage.getItem("cxr.tab") || "inbox";
  if (state.tab === "search" || state.tab === "new") state.tab = "inbox";
} catch (e) {}

// ─── helpers ─────────────────────────────────────────────
function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) el.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids.flat()) if (k != null && k !== false) el.append(k.nodeType ? k : String(k));
  return el;
}
async function api(path, body) {
  const opt = body ? { method: "POST", headers: { "Content-Type": "application/json", "X-CXMail-Remote": "1" }, body: JSON.stringify(body) } : {};
  const r = await fetch(path, opt);
  let data = null;
  try { data = await r.json(); } catch (e) {}
  if (!r.ok) throw new Error((data && data.error) || ("HTTP " + r.status));
  return data;
}
let toastT;
function toast(msg, ms) {
  const t = $("#toast"); t.textContent = msg; t.classList.add("show");
  clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove("show"), ms || 2600);
}
function haptic() { try { navigator.vibrate && navigator.vibrate(10); } catch (e) {} }
function when(iso) {
  const d = new Date(iso), now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const days = (now - d) / 864e5;
  if (days < 6) return d.toLocaleDateString([], { weekday: "short" });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { month: "short", day: "numeric" });
  return d.toLocaleDateString([], { year: "2-digit", month: "numeric", day: "numeric" });
}
const who = (m) => m.from_name || m.from_email || "(unknown)";
const fmtAddr = (a) => (a.name ? a.name + " <" + a.email + ">" : a.email);
const refQ = (m) => "account=" + encodeURIComponent(m.account) + "&folder=" + encodeURIComponent(m.folder) + "&uid=" + m.uid;
const route = (m) => (m.draft ? "#/send/" : "#/m/") + encodeURIComponent(m.account) + "/" + encodeURIComponent(m.folder) + "/" + m.uid;
const sameMsg = (a, b) => a && b && a.account === b.account && a.folder === b.folder && a.uid === b.uid;
function save() { try { localStorage.setItem("cxr.chip", state.chip); localStorage.setItem("cxr.acct", state.account); localStorage.setItem("cxr.tab", state.tab); } catch (e) {} }

// ─── chrome: tab bar, chips, title ───────────────────────
function renderChrome() {
  const bar = $("#tabbar"); bar.replaceChildren();
  for (const [id, label, ic] of TABS) {
    const b = h("button", { class: "tbtn", "aria-current": state.tab === id ? "page" : false, onclick: () => selectTab(id) }, icon(ic), label);
    bar.append(b);
  }
  $("#title").textContent = TITLES[state.tab] || "Inbox";
  const chips = $("#chips"); chips.replaceChildren();
  chips.style.display = state.tab === "inbox" && !state.q ? "" : "none";
  for (const [id, label] of CHIPS) {
    chips.append(h("button", { class: "chip", "aria-pressed": String(state.chip === id), onclick: () => { state.chip = id; save(); load(true); } }, label));
  }
}
function selectTab(id) {
  haptic();
  if (id === "new") return openDraft(null);
  if (location.hash && location.hash !== "#/") location.hash = "#/";
  if (id === "search") {
    state.tab = "search"; renderChrome(); window.scrollTo(0, 0);
    const q = $("#q"); q.focus();
    if (!state.q) { state.rows = []; renderList(); }
    return;
  }
  const again = state.tab === id;
  state.tab = id; state.q = ""; $("#q").value = ""; save();
  if (again) window.scrollTo({ top: 0, behavior: "smooth" });
  load(true);
}
function currentView() {
  if (state.tab === "unread") return "unread";
  if (state.tab === "drafts") return "drafts";
  return state.chip;
}

// ─── list ────────────────────────────────────────────────
function renderList(err) {
  const box = $("#rows"); box.replaceChildren();
  if (err) box.append(h("div", { class: "banner" }, err));
  if (!state.rows.length && !state.loading && !err) {
    box.append(h("div", { class: "empty" }, state.tab === "search" && !state.q ? "Search every account." : state.q ? "No matches." : "Nothing here."));
  }
  for (const m of state.rows) box.append(rowEl(m));
  if (state.loading) box.append(h("div", { class: "status" }, "Loading…"));
  else if (state.rows.length && !state.done) box.append(h("button", { class: "more", onclick: () => load(false) }, "Load more"));
}
function rowEl(m) {
  const a = h("a", { class: "msg" + (m.read ? "" : " unread"), href: route(m) },
    h("div", { class: "top" }, h("span", { class: "dot" }), h("span", { class: "from" }, m.draft ? "Draft" : who(m)),
      m.starred ? h("span", { class: "star", "aria-label": "starred" }, "★") : null,
      m.attachments ? h("span", { "aria-label": "has attachments" }, "📎") : null,
      h("span", { class: "date" }, when(m.date))),
    h("div", { class: "subj" }, m.subject),
    h("div", { class: "snip" }, m.snippet));
  longPress(a, () => openMenu(m));
  return a;
}

async function load(reset) {
  const seq = ++state.seq;
  if (reset) { state.rows = []; state.done = false; }
  state.loading = true; renderChrome(); renderList();
  const p = new URLSearchParams();
  const view = currentView();
  if (state.q) p.set("q", state.q); else p.set("view", view);
  if (state.account) p.set("account", state.account);
  if (!reset && state.rows.length) p.set("before", state.rows[state.rows.length - 1].date);
  let err = null;
  try {
    if (state.tab === "search" && !state.q) { state.loading = false; renderList(); return; }
    const rows = await api((!state.q && view === "drafts" ? "/api/drafts?" : "/api/messages?") + p);
    if (seq !== state.seq) return; // a newer load won
    state.rows = reset ? rows : state.rows.concat(rows);
    state.done = rows.length < 50;
  } catch (e) { if (seq !== state.seq) return; err = e.message; }
  state.loading = false; renderList(err);
}

// ─── long press → action menu ────────────────────────────
// Touch: hold 450 ms without moving. Mouse: right-click. Either way the tap
// that ends the hold must not also open the message.
function longPress(el, fire) {
  let t = null, x = 0, y = 0, fired = false;
  const cancel = () => { clearTimeout(t); t = null; el.classList.remove("pressing"); };
  el.addEventListener("touchstart", (e) => {
    fired = false; x = e.touches[0].clientX; y = e.touches[0].clientY;
    el.classList.add("pressing");
    t = setTimeout(() => { fired = true; el.classList.remove("pressing"); haptic(); fire(); }, 450);
  }, { passive: true });
  el.addEventListener("touchmove", (e) => {
    if (Math.abs(e.touches[0].clientX - x) > 8 || Math.abs(e.touches[0].clientY - y) > 8) cancel();
  }, { passive: true });
  el.addEventListener("touchend", (e) => { cancel(); if (fired) e.preventDefault(); });
  el.addEventListener("touchcancel", cancel);
  el.addEventListener("click", (e) => { if (fired) { e.preventDefault(); fired = false; } });
  el.addEventListener("contextmenu", (e) => { e.preventDefault(); cancel(); fire(); });
}

function menuItem(ic, label, onclick, cls) {
  return h("button", { class: "mi" + (cls ? " " + cls : ""), onclick: () => { closeSheets(); onclick(); } }, icon(ic), label);
}
function openMenu(m) {
  const box = $("#menu");
  const multi = m.to ? m.to.length + m.cc.length > 1 : true;
  box.replaceChildren(...[
    h("div", { class: "peek" }, h("div", { class: "from" }, m.draft ? "Draft" : who(m)), h("div", { class: "subj" }, m.subject)),
    m.draft ? null : menuItem("sparkle", "Open in Claude (T3)", () => openInClaude(m), "claude"),
    m.draft ? null : menuItem("chat", "Ask Claude in Chat", () => askInChat(m), "claude"),
    m.draft ? null : h("div", { class: "msep" }),
    m.draft ? menuItem("send", "Review & send", () => { location.hash = route(m); }) : null,
    m.draft ? null : menuItem("reply", "Reply", () => replyTo(m, "reply")),
    m.draft || !multi ? null : menuItem("replyAll", "Reply all", () => replyTo(m, "all")),
    m.draft ? null : menuItem("archive", "Archive", () => act("archive", m)),
    m.draft ? null : menuItem(m.starred ? "starOn" : "star", m.starred ? "Unstar" : "Star", () => act(m.starred ? "unstar" : "star", m)),
    m.draft ? null : menuItem(m.read ? "mail" : "mailOpen", m.read ? "Mark unread" : "Mark read", () => act(m.read ? "unread" : "read", m)),
    m.draft ? null : menuItem("folder", "Move to…", () => openMove(m)),
    h("div", { class: "msep" }),
    menuItem("trash", "Move to Trash", () => act("trash", m), "danger"),
  ].filter(Boolean));
  $("#menuSheet").classList.add("open");
}

// ─── reader ──────────────────────────────────────────────
async function openMessage(account, folder, uid) {
  document.body.classList.add("reading");
  const box = $("#reader");
  box.replaceChildren(navbar(""), h("div", { class: "status" }, "Opening…"));
  window.scrollTo(0, 0);
  let m;
  try { m = await api("/api/message?account=" + encodeURIComponent(account) + "&folder=" + encodeURIComponent(folder) + "&uid=" + uid); }
  catch (e) { box.replaceChildren(navbar(""), h("div", { class: "banner" }, e.message)); return; }
  state.current = m;
  renderReader(m, m.images_trusted);
  if (!m.read) {
    m.read = true; patchRow(m, { read: true });
    api("/api/action", { action: "read", account: m.account, folder: m.folder, uid: m.uid }).catch(() => {});
  }
}
function goBack() { if (history.length > 1) history.back(); else location.hash = "#/"; }
function navbar(right) {
  return h("div", { class: "navbar" },
    h("button", { class: "back", onclick: goBack, "aria-label": "Back" }, icon("back"), TITLES[state.tab] || "Inbox"),
    right ? h("span", { class: "meta" }, right) : null);
}
function tool(ic, label, onclick, disabled) {
  return h("button", { onclick, disabled: !!disabled, "aria-label": label }, icon(ic), label);
}

function renderReader(m, images) {
  const box = $("#reader");
  const acct = (state.accounts.find((a) => a.id === m.account) || {}).email || "";
  const parts = [navbar(acct),
    h("div", { class: "rmeta" },
      h("h1", {}, m.subject),
      h("div", { class: "meta" }, h("b", {}, who(m)), m.from_name ? " <" + m.from_email + ">" : ""),
      h("div", { class: "meta" }, new Date(m.date).toLocaleString()),
      m.to.length ? h("div", { class: "meta" }, "To: ", m.to.map(fmtAddr).join(", ")) : null,
      m.cc.length ? h("div", { class: "meta" }, "Cc: ", m.cc.map(fmtAddr).join(", ")) : null)];
  if (m.has_html) {
    if (!images) parts.push(h("div", { class: "imgnote" }, "Remote images are blocked. ", h("button", { onclick: () => renderReader(m, true) }, "Show images")));
    const f = h("iframe", { class: "body", title: "Message body", sandbox: "allow-same-origin allow-popups allow-popups-to-escape-sandbox",
      referrerpolicy: "no-referrer", src: "/api/body?" + refQ(m) + (images ? "&images=1" : "") });
    f.addEventListener("load", () => {
      try { const d = f.contentDocument; f.style.height = Math.max(200, d.documentElement.scrollHeight) + "px"; } catch (e) { f.style.height = "70vh"; }
    });
    parts.push(f);
  } else {
    parts.push(h("pre", { class: "body" }, m.plain_text || "(no body)"));
  }
  if (m.attachment_names.length) parts.push(h("div", { class: "attach" }, "📎 ", m.attachment_names.join(", "), " (open on the Mac)"));
  if (m.thread.length) {
    parts.push(h("div", { class: "thread" }, h("h2", {}, "In this conversation"), m.thread.map(rowEl)));
  }
  const multi = m.to.length + m.cc.length > 1;
  parts.push(h("nav", { class: "toolbar", "aria-label": "Message actions" },
    tool("archive", "Archive", () => act("archive", m)),
    tool(m.starred ? "starOn" : "star", m.starred ? "Unstar" : "Star", () => act(m.starred ? "unstar" : "star", m)),
    tool("folder", "Move", () => openMove(m)),
    tool("sparkle", "Claude", () => claudeChoice(m)),
    tool(multi ? "replyAll" : "reply", "Reply", () => (multi ? openMenuReply(m) : replyTo(m, "reply")))));
  box.replaceChildren(...parts);
}
function openMenuReply(m) {
  const box = $("#menu");
  box.replaceChildren(h("div", { class: "peek" }, h("div", { class: "from" }, "Reply to"), h("div", { class: "subj" }, m.subject)),
    menuItem("reply", "Reply to " + who(m), () => replyTo(m, "reply")),
    menuItem("replyAll", "Reply all", () => replyTo(m, "all")));
  $("#menuSheet").classList.add("open");
}

function patchRow(m, fields) {
  for (const r of state.rows) if (sameMsg(r, m)) Object.assign(r, fields);
}
function dropRow(m) { state.rows = state.rows.filter((x) => !sameMsg(x, m)); }

const DONE = { archive: "Archived", trash: "Moved to Trash", star: "Starred", unstar: "Unstarred", read: "Marked read", unread: "Marked unread" };
async function act(action, m, extra) {
  try {
    await api("/api/action", Object.assign({ action, account: m.account, folder: m.folder, uid: m.uid }, extra || {}));
  } catch (e) { toast("Didn't work: " + e.message); return; }
  haptic();
  const reading = sameMsg(state.current, m);
  if (action === "archive" || action === "move" || action === "trash") {
    dropRow(m); toast(action === "move" ? "Moved to " + extra.to_folder : DONE[action]);
    if (reading) goBack(); else renderList();
  } else if (action === "star" || action === "unstar") {
    m.starred = action === "star"; patchRow(m, { starred: m.starred });
    if (reading) renderReader(m, false); else renderList();
    toast(DONE[action]);
  } else {
    m.read = action === "read"; patchRow(m, { read: m.read });
    toast(DONE[action]);
    if (reading && action === "unread") goBack(); else renderList();
  }
}

function claudeChoice(m) {
  const box = $("#menu");
  box.replaceChildren(h("div", { class: "peek" }, h("div", { class: "from" }, "Claude"), h("div", { class: "subj" }, m.subject)),
    menuItem("chat", "Ask Claude in Chat", () => askInChat(m), "claude"),
    menuItem("sparkle", "Open in Claude (T3 thread)", () => openInClaude(m), "claude"));
  $("#menuSheet").classList.add("open");
}

// ─── Open in Claude → a T3 thread on the mini ────────────
async function openInClaude(m) {
  toast("Starting a Claude thread in T3…", 20000);
  try {
    const r = await api("/api/claude-thread", { account: m.account, folder: m.folder, uid: m.uid });
    const where = r.mapped ? r.project.split("/").pop() : "home";
    toast("Started in T3 (" + where + "): " + r.title + ". Open the T3 app.", 5000);
  } catch (e) { toast("Couldn't start it: " + e.message, 5000); }
}

// ─── Claude chat ─────────────────────────────────────────
// The mini runs CXMail's own chat bridge (the desktop panel's launch and
// permission gate); this is the panel. Event handling mirrors the desktop's
// applyChatEvent (src/lib/chatEvents.ts).
const chat = { id: 0, seq: 0, items: [], alive: false, busy: false, polling: false, subject: "" };
let itemN = 0;
const nid = (p) => p + (++itemN);
const TOOL_NAMES = {
  search_emails: "Searching mail", read_email: "Reading an email", read_thread: "Reading the thread", read_email_source: "Reading headers",
  preview_email_html: "Previewing", resolve_project_repo: "Finding the project", compose_draft: "Writing a draft", edit_draft: "Editing the draft",
  list_calendar_events: "Checking the calendar", search_vault: "Searching notes", related_notes: "Finding related notes",
  file_task: "Filing a task", list_tasks: "Listing tasks", get_task: "Reading a task", search_tasks: "Searching tasks",
  Read: "Reading a file", Grep: "Searching files", Glob: "Listing files", Skill: "Using a skill",
};
function toolLabel(name) {
  const short = name.replace(/^mcp__[^_]+(?:_[^_]+)*?__/, "");
  return TOOL_NAMES[short] || short.replace(/_/g, " ");
}
function finishStreaming() {
  const last = chat.items[chat.items.length - 1];
  if (last && last.kind === "assistant" && last.streaming) last.streaming = false;
}
function applyEvent(ev) {
  const last = chat.items[chat.items.length - 1];
  switch (ev.type) {
    case "ready": chat.alive = true; break;
    case "text_delta":
      if (last && last.kind === "assistant" && last.streaming) last.text += ev.text;
      else chat.items.push({ kind: "assistant", id: nid("a"), text: ev.text, streaming: true });
      break;
    case "assistant_text":
      if (last && last.kind === "assistant" && last.streaming) { last.text = ev.text; last.streaming = false; }
      else chat.items.push({ kind: "assistant", id: nid("a"), text: ev.text, streaming: false });
      break;
    case "tool_use":
      finishStreaming();
      if (!chat.items.some((i) => i.kind === "tool" && i.id === ev.id))
        chat.items.push({ kind: "tool", id: ev.id, name: ev.name, input: ev.input, status: "running", result: null, open: false });
      break;
    case "tool_result":
      for (const i of chat.items) if (i.kind === "tool" && i.id === ev.tool_use_id) { i.status = ev.is_error ? "error" : "done"; i.result = ev.text; }
      break;
    case "permission_request":
      finishStreaming();
      chat.items.push({ kind: "permission", id: ev.request_id, toolName: ev.tool_name, displayName: ev.display_name, input: ev.input,
        context: ev.context || null, rememberable: ev.rememberable !== false, state: "pending" });
      haptic();
      break;
    case "turn_done":
      finishStreaming(); chat.busy = false;
      if (ev.is_error && ev.subtype !== "success") chat.items.push({ kind: "notice", id: nid("n"), text: turnError(ev.subtype), tone: "error" });
      break;
    case "notice": chat.items.push({ kind: "notice", id: nid("n"), text: ev.text, tone: "info" }); break;
    case "exited": {
      finishStreaming();
      for (const i of chat.items) if (i.kind === "permission" && i.state === "pending") i.state = "expired";
      const crashed = chat.alive && ev.code !== 0 && ev.code !== null && ev.code !== undefined;
      const tail = (ev.stderr_tail || "").trim().split("\n").slice(-3).join("\n");
      if (crashed) chat.items.push({ kind: "notice", id: nid("n"), text: "Claude stopped unexpectedly (exit " + ev.code + ")." + (tail ? "\n" + tail : ""), tone: "error" });
      chat.alive = false; chat.busy = false;
      break;
    }
  }
}
function turnError(subtype) {
  if (subtype === "error_max_turns") return "Claude hit its turn limit. Ask it to continue.";
  if (subtype === "error_during_execution") return "Something went wrong during that turn.";
  return "That turn ended with an error (" + subtype + ").";
}

// Minimal, safe formatting: **bold** and `code`, as DOM nodes, never HTML.
function richText(text) {
  const out = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`)/g;
  let at = 0, m;
  while ((m = re.exec(text))) {
    if (m.index > at) out.push(document.createTextNode(text.slice(at, m.index)));
    const tok = m[0];
    out.push(tok[0] === "`" ? h("code", {}, tok.slice(1, -1)) : h("b", {}, tok.slice(2, -2)));
    at = m.index + tok.length;
  }
  if (at < text.length) out.push(document.createTextNode(text.slice(at)));
  return out;
}

function permCard(p) {
  const ctx = p.context;
  const card = h("div", { class: "perm" },
    h("h4", {}, "Allow " + (p.displayName || toolLabel(p.toolName)) + "?"),
    ctx && ctx.account ? h("div", { class: "ctx" }, "Account: ", h("b", {}, ctx.account), ctx.folder ? " · " + ctx.folder : "") : null,
    ctx && ctx.messages && ctx.messages.length ? h("div", { class: "ctx" },
      ctx.messages.map((mm) => h("div", {}, "• ", h("b", {}, mm.subject), " from " + mm.from)),
      ctx.message_count > ctx.messages.length ? h("div", {}, "…and " + (ctx.message_count - ctx.messages.length) + " more") : null) : null,
    h("details", {}, h("summary", {}, "Details"), h("pre", {}, JSON.stringify(p.input, null, 2))));
  if (p.state === "pending") {
    const answer = async (allow, remember) => {
      try {
        await api("/api/chat/answer", { request_id: p.id, allow, remember });
        p.state = allow ? (remember ? "allowed-always" : "allowed") : "denied";
        if (allow) chat.busy = true;
      } catch (e) { toast(e.message); p.state = "expired"; }
      renderChat();
    };
    card.append(h("div", { class: "btns" },
      h("button", { class: "act primary", onclick: () => answer(true, false) }, "Allow"),
      p.rememberable ? h("button", { class: "act", onclick: () => answer(true, true) }, "Allow for this chat") : null,
      h("button", { class: "act", onclick: () => answer(false, false) }, "Deny")));
  } else {
    const said = { allowed: "Allowed", "allowed-always": "Allowed for this chat", denied: "Denied", expired: "No longer waiting" }[p.state];
    card.append(h("div", { class: "state" }, said));
  }
  return card;
}

function renderChat() {
  const log = $("#chatlog");
  const nearBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 80;
  log.replaceChildren();
  if (!chat.items.length) {
    log.append(h("div", { class: "chatempty" }, h("b", {}, chat.subject ? "Ask about this email" : "Ask Claude"),
      chat.subject ? "“" + chat.subject + "”. Claude can read it and the thread, search your mail and notes, and draft a reply. Anything that changes mail asks you first."
        : "It runs on the Mac mini with your CXMail tools. Search, summarize, draft replies. Anything that changes mail asks you first."));
  }
  for (const i of chat.items) {
    if (i.kind === "user") log.append(h("div", { class: "bub user" }, ...richText(i.text)));
    else if (i.kind === "assistant") log.append(h("div", { class: "bub assistant" + (i.streaming ? " streaming" : "") }, ...richText(i.text)));
    else if (i.kind === "tool") {
      const st = i.status === "running" ? "⏳" : i.status === "error" ? "⚠️" : "✓";
      log.append(h("div", { class: "toolline" }, h("span", { class: "st" }, st),
        h("div", {}, h("button", { onclick: () => { i.open = !i.open; renderChat(); } }, toolLabel(i.name)),
          i.open && i.result ? h("pre", {}, i.result) : null)));
    } else if (i.kind === "permission") log.append(permCard(i));
    else if (i.kind === "notice") log.append(h("div", { class: "notice" + (i.tone === "error" ? " error" : "") }, i.text));
  }
  if (nearBottom) log.scrollTop = log.scrollHeight;
  const sendBtn = $("#chatSend");
  sendBtn.replaceChildren(icon(chat.busy ? "stop" : "up"));
  sendBtn.classList.toggle("stop", chat.busy);
  sendBtn.setAttribute("aria-label", chat.busy ? "Stop" : "Send");
  $("#chatSub").textContent = chat.subject ? "About: " + chat.subject : chat.alive ? "Your mail, on the Mac mini" : "Not running";
  const fab = $("#chatfab");
  fab.classList.toggle("waiting", chat.items.some((i) => i.kind === "permission" && i.state === "pending"));
  fab.classList.toggle("busy", chat.busy);
}

async function pollChat() {
  if (chat.polling) return;
  chat.polling = true;
  try {
    while (chat.id) {
      let r;
      try { r = await api("/api/chat/events?chat=" + chat.id + "&after=" + chat.seq); }
      catch (e) { await new Promise((ok) => setTimeout(ok, 3000)); continue; }
      if (r.reset) { chat.alive = false; chat.busy = false; renderChat(); break; }
      for (const o of r.events) {
        if (o.event) applyEvent(o.event);
        else if (o.error) chat.items.push({ kind: "notice", id: nid("n"), text: o.error, tone: "error" });
      }
      chat.seq = r.seq;
      renderChat();
      if (!r.alive && !r.events.length) { chat.alive = false; chat.busy = false; renderChat(); break; }
    }
  } finally { chat.polling = false; }
}

async function startChat(m) {
  try {
    const r = await api("/api/chat/start", m ? { account: m.account, folder: m.folder, uid: m.uid } : {});
    Object.assign(chat, { id: r.chat, seq: 0, items: [], alive: true, busy: false, subject: m ? m.subject : "" });
    renderChat();
    pollChat();
    return true;
  } catch (e) { toast("Couldn't start the chat: " + e.message, 5000); return false; }
}
function openChat() {
  document.body.classList.add("chatting");
  renderChat();
  setTimeout(() => { const log = $("#chatlog"); log.scrollTop = log.scrollHeight; }, 0);
}
async function askInChat(m) {
  openChat();
  await startChat(m);
  $("#chatInput").focus();
}
$("#chatfab").addEventListener("click", async () => {
  haptic(); openChat();
  if (!chat.id || !chat.alive) await startChat(null);
});
$("#chatClose").addEventListener("click", () => { document.body.classList.remove("chatting"); $("#chatInput").blur(); });
$("#chatNew").addEventListener("click", async () => { haptic(); await startChat(null); $("#chatInput").focus(); });
$("#chatInput").addEventListener("input", (e) => { const t = e.target; t.style.height = "auto"; t.style.height = Math.min(140, t.scrollHeight) + "px"; });
$("#chatForm").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  if (chat.busy) { api("/api/chat/interrupt", {}).catch(() => {}); return; }
  const input = $("#chatInput"); const text = input.value.trim();
  if (!text) return;
  if (!chat.id || !chat.alive) { if (!(await startChat(null))) return; }
  chat.items.push({ kind: "user", id: nid("u"), text }); chat.busy = true;
  input.value = ""; input.style.height = "";
  renderChat();
  const log = $("#chatlog"); log.scrollTop = log.scrollHeight;
  try { await api("/api/chat/send", { text }); }
  catch (e) { chat.busy = false; chat.items.push({ kind: "notice", id: nid("n"), text: "Not sent: " + e.message, tone: "error" }); renderChat(); }
});

// ─── review & send ───────────────────────────────────────
async function openReview(account, folder, uid) {
  document.body.classList.add("reading");
  const box = $("#reader");
  box.replaceChildren(navbar("Review"), h("div", { class: "status" }, "Fetching the draft from the server…"));
  window.scrollTo(0, 0);
  let p;
  try { p = await api("/api/draft-preview?account=" + encodeURIComponent(account) + "&folder=" + encodeURIComponent(folder) + "&uid=" + uid); }
  catch (e) { box.replaceChildren(navbar("Review"), h("div", { class: "banner" }, e.message)); return; }
  renderReview(p, false);
}

function rcptBlock(label, list, warn) {
  if (!list.length) return null;
  return h("div", { class: "rcpt" + (warn ? " warn" : "") }, h("span", { class: "rlabel" }, label),
    h("div", {}, list.map((a) => h("div", { class: "addr" }, a.name ? a.name + " " : "", h("b", {}, a.email)))));
}

function renderReview(p, images) {
  const box = $("#reader");
  const total = p.to.length + p.cc.length + p.bcc.length;
  let armed = false, armT;
  const send = h("button", { class: "sendbtn", disabled: !p.sendable }, icon("send"), "Send");
  const label = (t) => { send.replaceChildren(icon("send"), t); };
  send.addEventListener("click", async () => {
    if (!armed) {
      armed = true; haptic(); label("Tap again to send to " + total + (total === 1 ? " person" : " people"));
      send.classList.add("armed");
      armT = setTimeout(() => { armed = false; label("Send"); send.classList.remove("armed"); }, 4000);
      return;
    }
    clearTimeout(armT); send.disabled = true; label("Sending…");
    try {
      const r = await api("/api/send", { account: p.account_id, folder: p.folder, uid: p.uid, digest: p.digest });
      haptic(); toast(r.draft_removed ? "Sent" : (r.note || "Sent"));
      state.rows = []; location.hash = "#/";
    } catch (e) {
      send.disabled = false; label("Send"); send.classList.remove("armed"); armed = false;
      toast("Not sent: " + e.message, 5000);
    }
  });
  const parts = [navbar("Review before sending"),
    h("div", { class: "rmeta" },
      h("h1", {}, p.subject || "(no subject)"),
      h("div", { class: "rcpts" },
        rcptBlock("From", [p.from]),
        rcptBlock("To", p.to), rcptBlock("Cc", p.cc), rcptBlock("Bcc", p.bcc, true)),
      p.blocked_reason ? h("div", { class: "banner" }, p.blocked_reason) : null)];
  if (!images) parts.push(h("div", { class: "imgnote" }, "Remote images are blocked. ", h("button", { onclick: () => renderReview(p, true) }, "Show images")));
  const f = h("iframe", { class: "body", title: "Draft body", sandbox: "allow-same-origin allow-popups allow-popups-to-escape-sandbox", referrerpolicy: "no-referrer",
    src: "/api/draft-body?account=" + encodeURIComponent(p.account_id) + "&folder=" + encodeURIComponent(p.folder) + "&uid=" + p.uid + "&digest=" + p.digest + (images ? "&images=1" : "") });
  f.addEventListener("load", () => { try { f.style.height = Math.max(200, f.contentDocument.documentElement.scrollHeight) + "px"; } catch (e) { f.style.height = "70vh"; } });
  parts.push(f);
  if (p.attachments.length) parts.push(h("div", { class: "attach" }, "📎 ", p.attachments.join(", ")));
  parts.push(h("nav", { class: "toolbar", "aria-label": "Send" }, send));
  box.replaceChildren(...parts);
}

// ─── move ────────────────────────────────────────────────
async function openMove(m) {
  const list = $("#folders");
  list.replaceChildren(h("div", { class: "status" }, "Loading folders…"));
  $("#moveSheet").classList.add("open");
  try {
    const fs = await api("/api/folders?account=" + encodeURIComponent(m.account));
    list.replaceChildren(...fs.filter((f) => f.name !== m.folder && f.type !== "drafts").map((f) =>
      h("button", { class: "folder", onclick: () => { closeSheets(); act("move", m, { to_folder: f.name }); } }, f.label)));
  } catch (e) { list.replaceChildren(h("div", { class: "banner" }, e.message)); }
}

// ─── drafts / compose ────────────────────────────────────
let draftCtx = null;
async function replyTo(m, kind) {
  // From the list the row has no recipients yet; load them first.
  if (!m.to) {
    try { m = await api("/api/message?" + refQ(m)); } catch (e) { toast(e.message); return; }
  }
  openDraft(kind, m);
}
function openDraft(kind, msg) {
  const m = kind ? (msg || state.current) : null;
  const sel = $("#dAcct"); sel.replaceChildren(...state.accounts.map((a) => h("option", { value: a.id }, a.email)));
  sel.value = m ? m.account : (state.account || (state.accounts[0] || {}).id || "");
  sel.disabled = !!m;
  let to = [], cc = [];
  if (m) {
    to = [m.from_email];
    if (kind === "all") {
      const me = (m.account_email || "").toLowerCase();
      cc = m.to.concat(m.cc).map((a) => a.email).filter((e) => e && e.toLowerCase() !== me && e.toLowerCase() !== (m.from_email || "").toLowerCase());
    }
  }
  $("#draftTitle").textContent = m ? (kind === "all" ? "Reply all" : "Reply") : "New message";
  $("#dTo").value = to.join(", ");
  $("#dCc").value = [...new Set(cc)].join(", ");
  $("#dSubj").value = m ? (/^re:/i.test(m.subject) ? m.subject : "Re: " + m.subject) : "";
  $("#dBody").value = "";
  draftCtx = m ? { folder: m.folder, uid: m.uid } : null;
  $("#draftSheet").classList.add("open");
  setTimeout(() => (m ? $("#dBody") : $("#dTo")).focus(), 50);
}
const splitAddrs = (s) => s.split(/[,;\s]+/).map((x) => x.trim()).filter(Boolean);
let draftThenReview = false;
$("#dReview").addEventListener("click", () => { draftThenReview = true; });
$("#dSave").addEventListener("click", () => { draftThenReview = false; });
$("#draftForm").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const review = draftThenReview;
  const btn = review ? $("#dReview") : $("#dSave"); const label = btn.textContent;
  btn.disabled = true; btn.textContent = "Saving…";
  try {
    const body = { account: $("#dAcct").value, to: splitAddrs($("#dTo").value), cc: splitAddrs($("#dCc").value),
      subject: $("#dSubj").value, body: $("#dBody").value };
    if (draftCtx) { body.reply_folder = draftCtx.folder; body.reply_uid = draftCtx.uid; }
    const res = await api("/api/draft", body);
    closeSheets();
    if (review && res.draft) location.hash = "#/send/" + encodeURIComponent(res.draft.account) + "/" + encodeURIComponent(res.draft.folder) + "/" + res.draft.uid;
    else toast(review ? "Saved to Drafts. Open it under Drafts to send." : "Saved to Drafts.");
  } catch (e) { toast("Not saved: " + e.message, 5000); }
  btn.disabled = false; btn.textContent = label;
});

function closeSheets() { document.querySelectorAll(".sheet").forEach((s) => s.classList.remove("open")); }
$("#dCancel").addEventListener("click", closeSheets);
$("#mCancel").addEventListener("click", closeSheets);
document.querySelectorAll(".sheet").forEach((s) => s.addEventListener("click", (e) => { if (e.target === s) closeSheets(); }));

// ─── gestures a Home Screen app loses from Safari ────────
// Edge swipe back: start within 24px of the left edge on a pushed screen.
(function edgeSwipe() {
  let sx = 0, sy = 0, on = false;
  const page = () => $("#reader");
  document.addEventListener("touchstart", (e) => {
    on = document.body.classList.contains("reading") && !document.body.classList.contains("chatting") && e.touches[0].clientX < 24;
    sx = e.touches[0].clientX; sy = e.touches[0].clientY;
  }, { passive: true });
  document.addEventListener("touchmove", (e) => {
    if (!on) return;
    const dx = e.touches[0].clientX - sx, dy = Math.abs(e.touches[0].clientY - sy);
    if (dy > 40 && dx < 30) { on = false; page().style.transform = ""; return; }
    page().style.transform = "translateX(" + Math.max(0, dx) + "px)";
  }, { passive: true });
  document.addEventListener("touchend", (e) => {
    if (!on) return; on = false;
    const dx = e.changedTouches[0].clientX - sx;
    page().style.transition = "transform .2s";
    page().style.transform = dx > 80 ? "translateX(100%)" : "";
    setTimeout(() => { page().style.transition = ""; page().style.transform = ""; if (dx > 80) goBack(); }, 200);
  });
})();

// Pull to refresh on the list, from the very top.
(function pullToRefresh() {
  const ptr = $("#ptr"); let sy = 0, pulling = false, dist = 0;
  document.addEventListener("touchstart", (e) => {
    pulling = !document.body.classList.contains("reading") && !document.body.classList.contains("chatting") && window.scrollY <= 0 && !$(".sheet.open");
    sy = e.touches[0].clientY; dist = 0;
  }, { passive: true });
  document.addEventListener("touchmove", (e) => {
    if (!pulling) return;
    dist = e.touches[0].clientY - sy;
    if (dist <= 0) { ptr.style.height = "0"; return; }
    ptr.style.transition = "none";
    ptr.style.height = Math.min(64, dist / 2.2) + "px";
    ptr.textContent = dist > 120 ? "Release to refresh" : "Pull to refresh";
  }, { passive: true });
  document.addEventListener("touchend", () => {
    if (!pulling) return; pulling = false;
    ptr.style.transition = ""; ptr.style.height = "0";
    if (dist > 120) { haptic(); load(true); }
  });
})();

// ─── wiring ──────────────────────────────────────────────
let qT;
$("#q").addEventListener("focus", () => { if (state.tab !== "search") { state.tab = "search"; renderChrome(); } });
$("#q").addEventListener("input", (e) => { clearTimeout(qT); qT = setTimeout(() => { state.q = e.target.value.trim(); load(true); }, 300); });
$("#acct").addEventListener("change", (e) => { state.account = e.target.value; save(); load(true); });
$("#refresh").addEventListener("click", () => load(true));
document.addEventListener("visibilitychange", () => { if (!document.hidden && !document.body.classList.contains("reading")) load(true); });

function onRoute() {
  const m = location.hash.match(/^#\/(m|send)\/([^/]+)\/([^/]+)\/(\d+)$/);
  closeSheets();
  if (m && m[1] === "send") return openReview(decodeURIComponent(m[2]), decodeURIComponent(m[3]), Number(m[4]));
  if (m) return openMessage(decodeURIComponent(m[2]), decodeURIComponent(m[3]), Number(m[4]));
  document.body.classList.remove("reading"); state.current = null;
  if (!state.rows.length) load(true); else { renderChrome(); renderList(); }
}
window.addEventListener("hashchange", onRoute);

(async function init() {
  renderChrome();
  try {
    state.accounts = await api("/api/accounts");
    const sel = $("#acct");
    for (const a of state.accounts) sel.append(h("option", { value: a.id }, a.email + (a.hidden ? " (hidden)" : "")));
    if (!state.accounts.some((a) => a.id === state.account)) state.account = "";
    sel.value = state.account;
  } catch (e) { renderList(e.message + ". Is CXMail running on the mini?"); return; }
  onRoute();
})();
