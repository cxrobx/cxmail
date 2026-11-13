"use strict";
// CXMail Remote. Plain JS, no build step. Routes: #/ (list), #/m/<acct>/<folder>/<uid> (reader).

const $ = (s) => document.querySelector(s);
const VIEWS = [
  ["primary", "Primary"], ["unread", "Unread"], ["starred", "Starred"], ["drafts", "Drafts"],
  ["updates", "Updates"], ["promotions", "Promotions"], ["social", "Social"], ["all", "All"],
];
const state = { view: "primary", account: "", q: "", rows: [], accounts: [], done: false, loading: false, current: null };
try { state.view = localStorage.getItem("cxr.view") || "primary"; state.account = localStorage.getItem("cxr.acct") || ""; } catch (e) {}

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
function toast(msg) {
  const t = $("#toast"); t.textContent = msg; t.classList.add("show");
  clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove("show"), 2600);
}
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

// ─── list ────────────────────────────────────────────────
function renderTabs() {
  const nav = $("#tabs"); nav.replaceChildren();
  for (const [id, label] of VIEWS) {
    nav.append(h("button", { class: "tab", "aria-pressed": String(!state.q && state.view === id), onclick: () => {
      state.view = id; state.q = ""; $("#q").value = ""; save(); load(true);
    } }, label));
  }
}
function save() { try { localStorage.setItem("cxr.view", state.view); localStorage.setItem("cxr.acct", state.account); } catch (e) {} }

function renderList(err) {
  const main = $("#list"); main.replaceChildren();
  if (err) main.append(h("div", { class: "banner" }, err));
  if (!state.rows.length && !state.loading && !err) {
    main.append(h("div", { class: "empty" }, state.q ? "No matches." : "Nothing here."));
  }
  for (const m of state.rows) {
    main.append(h("a", { class: "msg" + (m.read ? "" : " unread"), href: route(m) },
      h("div", { class: "top" }, h("span", { class: "dot" }), h("span", { class: "from" }, who(m)),
        m.starred ? h("span", { class: "star", "aria-label": "starred" }, "★") : null,
        m.attachments ? h("span", { "aria-label": "has attachments" }, "📎") : null,
        h("span", { class: "date" }, when(m.date))),
      h("div", { class: "subj" }, m.subject),
      h("div", { class: "snip" }, m.snippet)));
  }
  if (state.loading) main.append(h("div", { class: "status" }, "Loading…"));
  else if (state.rows.length && !state.done) main.append(h("button", { class: "more", onclick: () => load(false) }, "Load more"));
}

async function load(reset) {
  if (state.loading) return;
  if (reset) { state.rows = []; state.done = false; }
  state.loading = true; renderTabs(); renderList();
  const p = new URLSearchParams();
  if (state.q) p.set("q", state.q); else p.set("view", state.view);
  if (state.account) p.set("account", state.account);
  if (!reset && state.rows.length) p.set("before", state.rows[state.rows.length - 1].date);
  let err = null;
  try {
    const rows = await api((!state.q && state.view === "drafts" ? "/api/drafts?" : "/api/messages?") + p);
    state.rows = reset ? rows : state.rows.concat(rows);
    state.done = rows.length < 50;
  } catch (e) { err = e.message; }
  state.loading = false; renderList(err);
}

// ─── reader ──────────────────────────────────────────────
async function openMessage(account, folder, uid) {
  document.body.classList.add("reading");
  const box = $("#reader");
  box.replaceChildren(h("div", { class: "status" }, "Opening…"));
  window.scrollTo(0, 0);
  let m;
  try { m = await api("/api/message?account=" + encodeURIComponent(account) + "&folder=" + encodeURIComponent(folder) + "&uid=" + uid); }
  catch (e) { box.replaceChildren(backBar(), h("div", { class: "banner" }, e.message)); return; }
  state.current = m;
  renderReader(m, m.images_trusted);
  if (!m.read) {
    m.read = true; patchRow(m, { read: true });
    api("/api/action", { action: "read", account: m.account, folder: m.folder, uid: m.uid }).catch(() => {});
  }
}
function backBar() { return h("button", { class: "act", onclick: () => history.length > 1 ? history.back() : (location.hash = "#/") }, "‹ Inbox"); }

function renderReader(m, images) {
  const box = $("#reader");
  const head = h("div", { class: "rhead" },
    h("div", { class: "row" }, backBar(), h("span", { style: "flex:1" }),
      h("span", { class: "meta" }, (state.accounts.find((a) => a.id === m.account) || {}).email || "")),
    h("h1", {}, m.subject),
    h("div", { class: "meta" }, h("b", {}, who(m)), m.from_name ? " <" + m.from_email + ">" : "", " · ", new Date(m.date).toLocaleString()),
    m.to.length ? h("div", { class: "meta" }, "To: ", m.to.map(fmtAddr).join(", ")) : null,
    m.cc.length ? h("div", { class: "meta" }, "Cc: ", m.cc.map(fmtAddr).join(", ")) : null,
    h("div", { class: "actions" },
      h("button", { class: "act primary", onclick: () => act("archive") }, "Archive"),
      h("button", { class: "act", onclick: () => act(m.starred ? "unstar" : "star") }, m.starred ? "★ Unstar" : "☆ Star"),
      h("button", { class: "act", onclick: () => act("unread") }, "Mark unread"),
      h("button", { class: "act", onclick: openMove }, "Move"),
      h("button", { class: "act", onclick: () => openDraft("reply") }, "Reply"),
      (m.to.length + m.cc.length > 1) ? h("button", { class: "act", onclick: () => openDraft("all") }, "Reply all") : null,
    ));
  const parts = [head];
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
    parts.push(h("div", { class: "thread" }, h("h2", {}, "In this conversation"),
      m.thread.map((t) => h("a", { class: "msg" + (t.read ? "" : " unread"), href: route(t) },
        h("div", { class: "top" }, h("span", { class: "dot" }), h("span", { class: "from" }, who(t)), h("span", { class: "date" }, when(t.date))),
        h("div", { class: "snip" }, t.snippet)))));
  }
  box.replaceChildren(...parts);
}

function patchRow(m, fields) {
  const r = state.rows.find((x) => x.account === m.account && x.folder === m.folder && x.uid === m.uid);
  if (r) Object.assign(r, fields);
}
function dropRow(m) { state.rows = state.rows.filter((x) => !(x.account === m.account && x.folder === m.folder && x.uid === m.uid)); }

async function act(action, extra) {
  const m = state.current; if (!m) return;
  try {
    await api("/api/action", Object.assign({ action, account: m.account, folder: m.folder, uid: m.uid }, extra || {}));
  } catch (e) { toast("Didn't work: " + e.message); return; }
  if (action === "archive" || action === "move") {
    dropRow(m); toast(action === "archive" ? "Archived" : "Moved to " + extra.to_folder);
    location.hash = "#/";
  } else if (action === "star" || action === "unstar") {
    m.starred = action === "star"; patchRow(m, { starred: m.starred }); renderReader(m, false); toast(m.starred ? "Starred" : "Unstarred");
  } else if (action === "unread") {
    patchRow(m, { read: false }); toast("Marked unread"); location.hash = "#/";
  }
}

// ─── review & send ───────────────────────────────────────
async function openReview(account, folder, uid) {
  document.body.classList.add("reading");
  const box = $("#reader");
  box.replaceChildren(h("div", { class: "status" }, "Fetching the draft from the server…"));
  window.scrollTo(0, 0);
  let p;
  try { p = await api("/api/draft-preview?account=" + encodeURIComponent(account) + "&folder=" + encodeURIComponent(folder) + "&uid=" + uid); }
  catch (e) { box.replaceChildren(h("div", { class: "rhead" }, backBar()), h("div", { class: "banner" }, e.message)); return; }
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
  const send = h("button", { class: "act primary send", disabled: !p.sendable }, "Send");
  send.addEventListener("click", async () => {
    if (!armed) {
      armed = true; send.textContent = "Tap again to send to " + total + (total === 1 ? " person" : " people");
      send.classList.add("armed");
      armT = setTimeout(() => { armed = false; send.textContent = "Send"; send.classList.remove("armed"); }, 4000);
      return;
    }
    clearTimeout(armT); send.disabled = true; send.textContent = "Sending…";
    try {
      const r = await api("/api/send", { account: p.account_id, folder: p.folder, uid: p.uid, digest: p.digest });
      toast(r.draft_removed ? "Sent" : (r.note || "Sent"));
      state.rows = []; location.hash = "#/";
    } catch (e) {
      send.disabled = false; send.textContent = "Send"; send.classList.remove("armed"); armed = false;
      toast("Not sent: " + e.message);
    }
  });
  const head = h("div", { class: "rhead" },
    h("div", { class: "row" }, backBar(), h("span", { style: "flex:1" }), h("span", { class: "meta" }, "Review before sending")),
    h("h1", {}, p.subject || "(no subject)"),
    h("div", { class: "rcpts" },
      rcptBlock("From", [p.from]),
      rcptBlock("To", p.to), rcptBlock("Cc", p.cc), rcptBlock("Bcc", p.bcc, true)),
    p.blocked_reason ? h("div", { class: "banner" }, p.blocked_reason) : null,
    h("div", { class: "actions" }, send));
  const parts = [head];
  if (!images) parts.push(h("div", { class: "imgnote" }, "Remote images are blocked. ", h("button", { onclick: () => renderReview(p, true) }, "Show images")));
  const f = h("iframe", { class: "body", title: "Draft body", sandbox: "allow-same-origin allow-popups allow-popups-to-escape-sandbox", referrerpolicy: "no-referrer",
    src: "/api/draft-body?account=" + encodeURIComponent(p.account_id) + "&folder=" + encodeURIComponent(p.folder) + "&uid=" + p.uid + "&digest=" + p.digest + (images ? "&images=1" : "") });
  f.addEventListener("load", () => { try { f.style.height = Math.max(200, f.contentDocument.documentElement.scrollHeight) + "px"; } catch (e) { f.style.height = "70vh"; } });
  parts.push(f);
  if (p.attachments.length) parts.push(h("div", { class: "attach" }, "📎 ", p.attachments.join(", ")));
  box.replaceChildren(...parts);
}

// ─── move ────────────────────────────────────────────────
async function openMove() {
  const m = state.current; const list = $("#folders");
  list.replaceChildren(h("div", { class: "status" }, "Loading folders…"));
  $("#moveSheet").classList.add("open");
  try {
    const fs = await api("/api/folders?account=" + encodeURIComponent(m.account));
    list.replaceChildren(...fs.filter((f) => f.name !== m.folder && f.type !== "drafts").map((f) =>
      h("button", { class: "folder", onclick: () => { closeSheets(); act("move", { to_folder: f.name }); } }, f.label)));
  } catch (e) { list.replaceChildren(h("div", { class: "banner" }, e.message)); }
}

// ─── drafts ──────────────────────────────────────────────
let draftCtx = null;
function openDraft(kind) {
  const m = kind ? state.current : null;
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
  $("#draftTitle").textContent = m ? (kind === "all" ? "Reply all (draft)" : "Reply (draft)") : "New draft";
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
  } catch (e) { toast("Not saved: " + e.message); }
  btn.disabled = false; btn.textContent = label;
});

function closeSheets() { document.querySelectorAll(".sheet").forEach((s) => s.classList.remove("open")); }
$("#dCancel").addEventListener("click", closeSheets);
$("#mCancel").addEventListener("click", closeSheets);
document.querySelectorAll(".sheet").forEach((s) => s.addEventListener("click", (e) => { if (e.target === s) closeSheets(); }));
$("#compose").addEventListener("click", () => openDraft(null));

// ─── wiring ──────────────────────────────────────────────
let qT;
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
  if (!state.rows.length) load(true); else renderList();
}
window.addEventListener("hashchange", onRoute);

(async function init() {
  renderTabs();
  try {
    state.accounts = await api("/api/accounts");
    const sel = $("#acct");
    for (const a of state.accounts) sel.append(h("option", { value: a.id }, a.email + (a.hidden ? " (hidden)" : "")));
    if (!state.accounts.some((a) => a.id === state.account)) state.account = "";
    sel.value = state.account;
  } catch (e) { renderList(e.message + ". Is CXMail running on the mini?"); return; }
  onRoute();
})();
