#!/usr/bin/env python3
"""CXMail Remote: a thin, tailnet-only web client over the Mac mini's CXMail.

Reads come straight from the running app's SQLite database, opened read-only.
Every change (archive, flag, move, draft) goes through the shared cxmail-mcp
HTTP server, so IMAP stays the source of truth and the app hears about it over
its bridge socket. Nothing here sends mail: drafts land in Drafts and are sent
from CXMail on the Mac.

Standard library only (macOS /usr/bin/python3 is 3.9), so there is no venv to
break. See remote/README.md for the LaunchAgent and Tailscale Serve setup.
"""

import json
import os
import re
import sqlite3
import subprocess
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
DB_PATH = os.environ.get(
    "CXMAIL_DB",
    os.path.expanduser("~/Library/Application Support/com.cxmail.app/cxmail.db"),
)
MCP_URL = os.environ.get("CXMAIL_MCP_URL", "http://127.0.0.1:18704/mcp")
PORT = int(os.environ.get("CXMAIL_REMOTE_PORT", "18706"))
# Tailscale Serve adds Tailscale-User-Login for tailnet users. Only these
# logins get in. Set in the LaunchAgent, never in the repo.
ALLOWED_LOGINS = {
    s.strip().lower()
    for s in os.environ.get("CXMAIL_REMOTE_LOGINS", "").split(",")
    if s.strip()
}
# Dev mode skips the login check, for a localhost-only instance that agents
# and tests drive. Never give a dev instance a Serve entry.
DEV = os.environ.get("CXMAIL_REMOTE_DEV") == "1"
# The only thing here that can send mail: CXMail's own signed binary, through
# its `remote` subcommands (never an MCP tool). It sends a draft only after a
# person has reviewed it, and only if it hasn't changed since.
SEND_BIN = os.environ.get("CXMAIL_SEND_BIN", "/Applications/CXMail.app/Contents/MacOS/cxmail-mcp")

CATEGORIES = ("primary", "updates", "promotions", "social")
PAGE = 50


# ─── Database (read-only) ──────────────────────────────────────────────


def db():
    conn = sqlite3.connect(
        "file:{}?mode=ro".format(urllib.parse.quote(DB_PATH)), uri=True, timeout=5
    )
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA query_only = ON")
    return conn


def addr_list(raw):
    """messages.to_list / cc_list are JSON [{email, name}]."""
    if not raw:
        return []
    try:
        items = json.loads(raw)
    except ValueError:
        return []
    out = []
    for it in items if isinstance(items, list) else []:
        if isinstance(it, dict) and it.get("email"):
            out.append({"email": it["email"], "name": it.get("name")})
    return out


def accounts(conn):
    rows = conn.execute(
        "SELECT id, email, display_name, color, hidden_from_aggregates "
        "FROM accounts WHERE is_active = 1 ORDER BY sort_order, email"
    ).fetchall()
    return [
        {
            "id": r["id"],
            "email": r["email"],
            "name": r["display_name"],
            "color": r["color"],
            "hidden": bool(r["hidden_from_aggregates"]),
        }
        for r in rows
    ]


def folder_of_type(conn, account_id, ftype):
    r = conn.execute(
        "SELECT name FROM folders WHERE account_id = ? AND folder_type = ? LIMIT 1",
        (account_id, ftype),
    ).fetchone()
    return r["name"] if r else None


def row_summary(r):
    return {
        "account": r["account_id"],
        "folder": r["folder_name"],
        "uid": r["uid"],
        "subject": r["subject"] or "(no subject)",
        "from_name": r["from_name"],
        "from_email": r["from_email"],
        "date": r["date"],
        "snippet": r["snippet"] or "",
        "read": bool(r["is_read"]),
        "starred": bool(r["is_flagged"]),
        "attachments": bool(r["has_attachments"]),
        "category": r["category"],
    }


SUMMARY_COLS = (
    "m.account_id, m.folder_name, m.uid, m.subject, m.from_name, m.from_email, "
    "m.date, m.snippet, m.is_read, m.is_flagged, m.has_attachments, m.category"
)


def list_messages(conn, view, account, before):
    accts = accounts(conn)
    if account:
        ids = [a["id"] for a in accts if a["id"] == account]
    else:
        ids = [a["id"] for a in accts if not a["hidden"]]
    if not ids:
        return []
    inbox_pairs = []
    for aid in ids:
        name = folder_of_type(conn, aid, "inbox") or "INBOX"
        inbox_pairs.append((aid, name))
    where = ["(" + " OR ".join(["(m.account_id = ? AND m.folder_name = ?)"] * len(inbox_pairs)) + ")"]
    args = [x for pair in inbox_pairs for x in pair]
    if view == "primary":
        where.append("(m.category = 'primary' OR m.category IS NULL)")
    elif view in CATEGORIES:
        where.append("m.category = ?")
        args.append(view)
    elif view == "unread":
        where.append("m.is_read = 0")
    elif view == "starred":
        where.append("m.is_flagged = 1")
    # "all": no extra filter
    where.append("COALESCE(m.category, '') <> 'junk'")
    if before:
        where.append("m.date < ?")
        args.append(before)
    sql = (
        "SELECT {} FROM messages m WHERE {} ORDER BY m.date DESC LIMIT {}".format(
            SUMMARY_COLS, " AND ".join(where), PAGE
        )
    )
    return [row_summary(r) for r in conn.execute(sql, args).fetchall()]


def fts_query(text, prefix=True):
    """A line-for-line port of cxmail-db's `build_match_expr` (gotcha #26), so
    the phone and the desktop answer the same query the same way: "phrases",
    a leading - negates, every token double-quoted so FTS syntax never leaks,
    the last positive token a prefix. test_server.py pins the parity cases."""
    chars = text
    n = len(chars)
    i = 0
    tokens = []
    while i < n:
        while i < n and chars[i].isspace():
            i += 1
        if i >= n:
            break
        negated = False
        if chars[i] == "-" and i + 1 < n and not chars[i + 1].isspace():
            negated = True
            i += 1
        if i < n and chars[i] == '"':
            i += 1
            start = i
            while i < n and chars[i] != '"':
                i += 1
            phrase = chars[start:i].strip()
            if i < n:
                i += 1
            if phrase:
                tokens.append((phrase, negated))
        else:
            start = i
            while i < n and not chars[i].isspace():
                i += 1
            raw = chars[start:i]
            if any(c.isalnum() for c in raw):
                tokens.append((raw, negated))
    pos = [t for t, neg in tokens if not neg]
    negs = [t for t, neg in tokens if neg]
    if not pos:
        return None
    parts = []
    for idx, t in enumerate(pos):
        term = '"{}"'.format(t.replace('"', '""'))
        if prefix and idx == len(pos) - 1:
            term += "*"
        parts.append(term)
    expr = " AND ".join(parts)
    for t in negs:
        expr += ' NOT "{}"'.format(t.replace('"', '""'))
    return expr


def search(conn, q, account, before):
    fq = fts_query(q)
    if not fq:
        return []
    accts = accounts(conn)
    ids = [a["id"] for a in accts if (a["id"] == account if account else not a["hidden"])]
    if not ids:
        return []
    where = ["messages_fts MATCH ?", "m.account_id IN ({})".format(",".join("?" * len(ids)))]
    args = [fq] + ids
    # Search mail people kept, not the bins.
    where.append(
        "m.folder_name NOT IN (SELECT name FROM folders f WHERE f.account_id = m.account_id "
        "AND f.folder_type IN ('trash', 'spam', 'drafts', 'important', 'starred'))"
    )
    if before:
        where.append("m.date < ?")
        args.append(before)
    sql = (
        "SELECT {} FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid "
        "WHERE {} ORDER BY m.date DESC LIMIT {}".format(SUMMARY_COLS, " AND ".join(where), PAGE)
    )
    return [row_summary(r) for r in conn.execute(sql, args).fetchall()]


def get_message(conn, account, folder, uid):
    r = conn.execute(
        "SELECT m.*, b.plain_text, b.sanitized_html IS NOT NULL AS has_html, "
        "b.id IS NOT NULL AS cached "
        "FROM messages m LEFT JOIN message_bodies b USING (account_id, folder_name, uid) "
        "WHERE m.account_id = ? AND m.folder_name = ? AND m.uid = ?",
        (account, folder, uid),
    ).fetchone()
    if not r:
        return None
    msg = row_summary(r)
    msg.update(
        {
            "message_id": r["message_id"],
            "to": addr_list(r["to_list"]),
            "cc": addr_list(r["cc_list"]),
            "cached": bool(r["cached"]),
            "has_html": bool(r["has_html"]),
            "plain_text": r["plain_text"] if not r["has_html"] else None,
        }
    )
    msg["attachment_names"] = [
        a["filename"]
        for a in conn.execute(
            "SELECT filename FROM attachments WHERE account_id = ? AND folder_name = ? "
            "AND message_uid = ? AND is_inline = 0",
            (account, folder, uid),
        ).fetchall()
        if a["filename"]
    ]
    # The rest of the conversation, oldest first. Members span folders and a
    # Gmail message sits in INBOX *and* All Mail (gotcha #61), so dedupe by
    # Message-ID, keeping the copy in the best-ranked folder.
    thread = []
    if r["thread_root_id"]:
        rank = {"inbox": 0, "sent": 1, "archive": 2}
        best = {}
        for t in conn.execute(
            "SELECT {}, m.message_id, f.folder_type FROM messages m "
            "LEFT JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name "
            "WHERE m.account_id = ? AND m.thread_root_id = ? "
            "AND COALESCE(f.folder_type, '') NOT IN ('trash', 'spam', 'drafts') "
            "ORDER BY m.date ASC LIMIT 200".format(SUMMARY_COLS),
            (account, r["thread_root_id"]),
        ).fetchall():
            key = t["message_id"] or (t["folder_name"], t["uid"])
            score = rank.get(t["folder_type"], 3)
            if key not in best or score < best[key][0]:
                best[key] = (score, t)
        for _, t in sorted(best.values(), key=lambda st: st[1]["date"]):
            if t["message_id"] and t["message_id"] == r["message_id"]:
                continue
            if t["folder_name"] == folder and t["uid"] == uid:
                continue
            thread.append(row_summary(t))
    msg["thread"] = thread[-30:]
    acct = conn.execute("SELECT email FROM accounts WHERE id = ?", (account,)).fetchone()
    msg["account_email"] = acct["email"] if acct else None
    # Remote images: off unless the sender is on the desktop's allowlist
    # (architecture invariant 4).
    msg["images_trusted"] = bool(
        r["from_email"]
        and conn.execute(
            "SELECT 1 FROM trusted_image_senders WHERE sender_email = ?",
            (r["from_email"].lower(),),
        ).fetchone()
    )
    return msg


def list_drafts(conn, account):
    accts = accounts(conn)
    ids = [a["id"] for a in accts if (a["id"] == account if account else not a["hidden"])]
    rows = []
    for aid in ids:
        name = folder_of_type(conn, aid, "drafts")
        if not name:
            continue
        rows += [
            row_summary(r)
            for r in conn.execute(
                "SELECT {} FROM messages m WHERE m.account_id = ? AND m.folder_name = ? "
                "ORDER BY m.date DESC LIMIT {}".format(SUMMARY_COLS, PAGE),
                (aid, name),
            ).fetchall()
        ]
    rows.sort(key=lambda r: r["date"], reverse=True)
    for r in rows:
        r["draft"] = True
    return rows[:PAGE]


def get_html(conn, account, folder, uid):
    r = conn.execute(
        "SELECT sanitized_html FROM message_bodies WHERE account_id = ? AND folder_name = ? AND uid = ?",
        (account, folder, uid),
    ).fetchone()
    return r["sanitized_html"] if r else None


def folders(conn, account):
    return [
        {"name": r["name"], "label": r["display_name"] or r["name"], "type": r["folder_type"]}
        for r in conn.execute(
            "SELECT name, display_name, folder_type FROM folders WHERE account_id = ? "
            "AND name NOT IN ('[Gmail]') ORDER BY CASE folder_type WHEN 'inbox' THEN 0 "
            "WHEN 'archive' THEN 1 WHEN 'other' THEN 3 ELSE 2 END, name",
            (account,),
        ).fetchall()
    ]


# ─── cxmail-mcp client (writes) ────────────────────────────────────────


class McpError(Exception):
    pass


class Mcp:
    """Minimal streamable-HTTP MCP client: one session, re-opened on failure."""

    def __init__(self, url):
        self.url = url
        self.sid = None
        self.n = 0
        self.lock = threading.Lock()

    def _post(self, body):
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.sid:
            headers["Mcp-Session-Id"] = self.sid
        req = urllib.request.Request(self.url, json.dumps(body).encode(), headers)
        with urllib.request.urlopen(req, timeout=90) as resp:
            self.sid = resp.headers.get("Mcp-Session-Id") or self.sid
            raw = resp.read().decode("utf-8", "replace")
        out = None
        for line in raw.splitlines():
            if line.startswith("data:") and line[5:].strip():
                out = json.loads(line[5:])
        if out is None and raw.strip().startswith("{"):
            out = json.loads(raw)
        return out

    def _rpc(self, method, params):
        self.n += 1
        return self._post({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})

    def _open(self):
        self.sid = None
        self._rpc(
            "initialize",
            {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "cxmail-remote", "version": "1"},
            },
        )
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def call(self, tool, args):
        with self.lock:
            for attempt in (0, 1):
                try:
                    if not self.sid:
                        self._open()
                    res = self._rpc("tools/call", {"name": tool, "arguments": args})
                    break
                except (urllib.error.URLError, OSError, ValueError) as e:
                    self.sid = None
                    if attempt:
                        raise McpError("cxmail-mcp is not answering ({}). Is CXMail running on the mini?".format(e))
            if res is None:
                raise McpError("empty reply from cxmail-mcp")
            if "error" in res:
                raise McpError(res["error"].get("message", "cxmail-mcp error"))
            result = res.get("result", {})
            text = "\n".join(c.get("text", "") for c in result.get("content", []))
            if result.get("isError"):
                raise McpError(text or "cxmail-mcp refused")
            return text


MCP = Mcp(MCP_URL)


class Pending:
    """Changes this server made that the app's DB hasn't caught up with yet.

    cxmail-mcp changes IMAP and pings the app, but nothing writes the change
    into cxmail.db until the app's next IDLE/sync pass. Without this, a reload
    right after Archive brings the message back. Entries expire once the DB
    agrees, or after TTL seconds as a backstop."""

    TTL = 900

    def __init__(self):
        self.lock = threading.Lock()
        self.items = {}  # (account, folder, uid) -> (expires, {"gone"|"read"|"starred": value})

    def note(self, key, **fields):
        import time

        with self.lock:
            exp, cur = self.items.get(key, (0, {}))
            cur = dict(cur, **fields)
            self.items[key] = (time.time() + self.TTL, cur)

    def apply(self, rows):
        import time

        now = time.time()
        with self.lock:
            for k in [k for k, (exp, _) in self.items.items() if exp < now]:
                del self.items[k]
            if not self.items:
                return rows
            out = []
            for r in rows:
                key = (r["account"], r["folder"], r["uid"])
                if key not in self.items:
                    out.append(r)
                    continue
                _, f = self.items[key]
                if f.get("gone"):
                    continue
                r = dict(r)
                for name in ("read", "starred"):
                    if name in f:
                        if r[name] == f[name]:
                            del f[name]  # the DB caught up
                        else:
                            r[name] = f[name]
                if not f:
                    del self.items[key]
                out.append(r)
            return out


PENDING = Pending()
EFFECT = {
    "archive": {"gone": True},
    "move": {"gone": True},
    "star": {"starred": True},
    "unstar": {"starred": False},
    "read": {"read": True},
    "unread": {"read": False},
}

class SendError(Exception):
    pass


def run_send_bin(*args):
    """Run `cxmail-mcp remote ...`; returns its JSON or raises SendError."""
    try:
        p = subprocess.run(
            [SEND_BIN, "remote"] + [str(a) for a in args],
            capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL,
            env=dict(os.environ, CXMAIL_MCP_IDLE_GRACE_SECS="0"),
        )
    except FileNotFoundError:
        raise SendError("CXMail isn't installed at {}".format(SEND_BIN))
    except subprocess.TimeoutExpired:
        raise SendError("CXMail didn't answer within 2 minutes. Check Drafts and Sent before trying again.")
    out = (p.stdout or "").strip().splitlines()
    if not out:
        # A CXMail build from before the `remote` subcommand starts its stdio
        # MCP server instead, sees the closed stdin and exits silently.
        raise SendError("This CXMail build can't send from the phone yet. Ship the update on the Mac first.")
    try:
        data = json.loads(out[-1]) if out else {}
    except ValueError:
        data = {}
    if p.returncode != 0 or "error" in data:
        msg = data.get("error") or (p.stderr or "").strip()[-300:] or "exit {}".format(p.returncode)
        if msg.startswith("usage:"):
            msg = "This CXMail build can't send from the phone yet. Ship the update on the Mac first."
        raise SendError(msg)
    return data


class PreviewCache:
    """Reviewed drafts, so the review screen's body iframe doesn't refetch."""

    def __init__(self):
        self.lock = threading.Lock()
        self.items = {}

    def put(self, p):
        import time
        with self.lock:
            now = time.time()
            self.items = {k: v for k, v in self.items.items() if v[0] > now}
            self.items[(p["account_id"], p["folder"], p["uid"], p["digest"])] = (now + 900, p)

    def get(self, account, folder, uid, digest):
        import time
        with self.lock:
            hit = self.items.get((account, folder, uid, digest))
            return hit[1] if hit and hit[0] > time.time() else None


PREVIEWS = PreviewCache()
_SAVED = re.compile(r"Draft saved to (.+?) \(UID (\d+)\)")

ACTIONS = {
    "archive": lambda m: ("archive_email", {"account_id": m["account"], "folder": m["folder"], "uid": m["uid"]}),
    "star": lambda m: ("flag_email", dict(_flag(m), flag="starred")),
    "unstar": lambda m: ("flag_email", dict(_flag(m), flag="unstarred")),
    "read": lambda m: ("flag_email", dict(_flag(m), flag="read")),
    "unread": lambda m: ("flag_email", dict(_flag(m), flag="unread")),
    "move": lambda m: (
        "move_email",
        {"account_id": m["account"], "from_folder": m["folder"], "to_folder": m["to_folder"], "uid": m["uid"]},
    ),
}


def _flag(m):
    return {"account_id": m["account"], "folder": m["folder"], "uid": m["uid"]}


def _emails(v):
    if not isinstance(v, list):
        return []
    return [str(x).strip() for x in v if isinstance(x, str) and "@" in x and len(x) < 320][:50]


# ─── HTTP ──────────────────────────────────────────────────────────────

PAGE_CSP = (
    "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; "
    "script-src 'self'; frame-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
)


def body_csp(images):
    # The email itself: no scripts, ever. Remote images (tracking pixels) only
    # when asked for, like the desktop app.
    img = "data: cid: https:" if images else "data: cid:"
    return (
        "default-src 'none'; img-src {}; style-src 'unsafe-inline'; font-src data:; "
        "frame-ancestors 'self'; base-uri 'none'; form-action 'none'; sandbox allow-same-origin "
        "allow-popups allow-popups-to-escape-sandbox".format(img)
    )


BODY_WRAP = (
    "<!doctype html><html><head><meta charset='utf-8'>"
    "<meta name='viewport' content='width=device-width,initial-scale=1'>"
    "<base target='_blank'><meta name='referrer' content='no-referrer'>"
    "<style>html,body{{margin:0;padding:0;background:#fff;color:#111;"
    "font:15px/1.45 -apple-system,system-ui,sans-serif;overflow-wrap:anywhere}}"
    "body{{padding:12px}}img{{max-width:100%;height:auto}}table{{max-width:100%!important}}"
    "pre{{white-space:pre-wrap}}</style></head><body>{}</body></html>"
)


class Handler(BaseHTTPRequestHandler):
    server_version = "cxmail-remote"
    sys_version = ""

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    # Auth ---------------------------------------------------------------
    def _allowed(self):
        if DEV:
            return True
        login = (self.headers.get("Tailscale-User-Login") or "").strip().lower()
        return bool(login) and login in ALLOWED_LOGINS

    def _send(self, code, body, ctype="application/json", extra=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body)
        if isinstance(body, str):
            body = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", ctype + ("; charset=utf-8" if "charset" not in ctype else ""))
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def _err(self, code, msg):
        self._send(code, {"error": msg})

    # Routing ------------------------------------------------------------
    def do_GET(self):
        if not self._allowed():
            return self._err(403, "not allowed")
        url = urllib.parse.urlparse(self.path)
        q = {k: v[-1] for k, v in urllib.parse.parse_qs(url.query).items()}
        try:
            if url.path in ("/", "/index.html"):
                return self._static("index.html", "text/html", {"Content-Security-Policy": PAGE_CSP})
            if url.path == "/app.js":
                return self._static("app.js", "text/javascript")
            if url.path == "/icon.svg":
                return self._static("icon.svg", "image/svg+xml")
            if url.path == "/manifest.webmanifest":
                return self._static("manifest.webmanifest", "application/manifest+json")
            if url.path == "/api/accounts":
                with db() as c:
                    return self._send(200, accounts(c))
            if url.path == "/api/messages":
                with db() as c:
                    if q.get("q"):
                        rows = search(c, q["q"], q.get("account"), q.get("before"))
                    else:
                        rows = list_messages(c, q.get("view", "primary"), q.get("account"), q.get("before"))
                    return self._send(200, PENDING.apply(rows))
            if url.path == "/api/message":
                ref = self._ref(q)
                with db() as c:
                    msg = get_message(c, *ref)
                applied = PENDING.apply([msg]) if msg else []
                if not applied:
                    return self._err(404, "message not found (archived or moved since?)")
                msg = dict(msg, **{k: applied[0][k] for k in ("read", "starred")})
                msg["thread"] = PENDING.apply(msg["thread"])
                if not msg["cached"]:
                    # Not on the mini yet: let the MCP fetch it from IMAP.
                    try:
                        msg["plain_text"] = MCP.call(
                            "read_email", {"account_id": ref[0], "folder": ref[1], "uid": ref[2]}
                        )
                    except McpError as e:
                        msg["plain_text"] = "Body not cached on the mini, and fetching it failed: {}".format(e)
                return self._send(200, msg)
            if url.path == "/api/body":
                ref = self._ref(q)
                with db() as c:
                    html = get_html(c, *ref)
                if html is None:
                    return self._err(404, "no HTML body")
                return self._send(
                    200,
                    BODY_WRAP.format(html),
                    "text/html",
                    {"Content-Security-Policy": body_csp(q.get("images") == "1")},
                )
            if url.path == "/api/drafts":
                with db() as c:
                    return self._send(200, PENDING.apply(list_drafts(c, q.get("account"))))
            if url.path == "/api/draft-preview":
                ref = self._ref(q)
                try:
                    p = run_send_bin("preview-draft", *ref)
                except SendError as e:
                    return self._err(502, str(e))
                PREVIEWS.put(p)
                p = dict(p)
                p.pop("sanitized_html", None)
                return self._send(200, p)
            if url.path == "/api/draft-body":
                ref = self._ref(q)
                p = PREVIEWS.get(ref[0], ref[1], ref[2], q.get("digest", ""))
                if not p:
                    return self._err(404, "review expired; reopen the draft")
                if p.get("sanitized_html"):
                    html = p["sanitized_html"]
                else:
                    html = "<pre>{}</pre>".format(
                        (p.get("plain_text") or "").replace("&", "&amp;").replace("<", "&lt;")
                    )
                return self._send(200, BODY_WRAP.format(html), "text/html",
                                  {"Content-Security-Policy": body_csp(q.get("images") == "1")})
            if url.path == "/api/folders":
                with db() as c:
                    return self._send(200, folders(c, q.get("account", "")))
            return self._err(404, "not found")
        except ValueError as e:
            return self._err(400, str(e))
        except sqlite3.Error as e:
            return self._err(503, "database unavailable: {}".format(e))

    def do_POST(self):
        if not self._allowed():
            return self._err(403, "not allowed")
        # CSRF: a cross-site form can't set this header or a JSON content type
        # without a preflight, and this server never answers preflights.
        if self.headers.get("X-CXMail-Remote") != "1" or not (
            self.headers.get("Content-Type", "").startswith("application/json")
        ):
            return self._err(400, "bad request")
        try:
            n = int(self.headers.get("Content-Length", "0"))
            if n > 512 * 1024:
                return self._err(413, "too large")
            data = json.loads(self.rfile.read(n) or b"{}")
            url = urllib.parse.urlparse(self.path)
            if url.path == "/api/action":
                m = {
                    "account": str(data.get("account", "")),
                    "folder": str(data.get("folder", "")),
                    "uid": int(data.get("uid")),
                    "to_folder": str(data.get("to_folder", "")),
                }
                act = ACTIONS.get(data.get("action"))
                if not act or not m["account"] or not m["folder"]:
                    return self._err(400, "unknown action")
                if data.get("action") == "move" and not m["to_folder"]:
                    return self._err(400, "move needs to_folder")
                tool, args = act(m)
                result = MCP.call(tool, args)
                PENDING.note((m["account"], m["folder"], m["uid"]), **EFFECT[data["action"]])
                return self._send(200, {"ok": True, "result": result})
            if url.path == "/api/draft":
                to = _emails(data.get("to"))
                if not data.get("account") or not to:
                    return self._err(400, "a draft needs an account and at least one recipient")
                args = {
                    "account_id": str(data["account"]),
                    "to": to,
                    "subject": str(data.get("subject", ""))[:998],
                    "body": str(data.get("body", "")),
                    "is_html": False,
                }
                cc = _emails(data.get("cc"))
                if cc:
                    args["cc"] = cc
                if data.get("reply_folder") and data.get("reply_uid") is not None:
                    args["reply_to_folder"] = str(data["reply_folder"])
                    args["reply_to_uid"] = int(data["reply_uid"])
                result = MCP.call("compose_draft", args)
                saved = _SAVED.search(result)
                return self._send(200, {
                    "ok": True,
                    "result": result,
                    "draft": {"account": args["account_id"], "folder": saved.group(1), "uid": int(saved.group(2))}
                    if saved else None,
                })
            if url.path == "/api/send":
                ref = (str(data.get("account", "")), str(data.get("folder", "")), int(data.get("uid")))
                digest = str(data.get("digest", ""))
                if not ref[0] or not ref[1] or len(digest) != 64:
                    return self._err(400, "account, folder, uid and the reviewed digest are required")
                try:
                    out = run_send_bin("send-draft", ref[0], ref[1], ref[2], digest)
                except SendError as e:
                    return self._err(502, str(e))
                PENDING.note(ref, gone=True)
                return self._send(200, dict(out, ok=True))
            return self._err(404, "not found")
        except (ValueError, TypeError) as e:
            return self._err(400, "bad request: {}".format(e))
        except McpError as e:
            return self._err(502, str(e))

    # Helpers ------------------------------------------------------------
    def _ref(self, q):
        if not q.get("account") or not q.get("folder") or not q.get("uid"):
            raise ValueError("account, folder and uid are required")
        return q["account"], q["folder"], int(q["uid"])

    def _static(self, name, ctype, extra=None):
        with open(os.path.join(HERE, "static", name), "rb") as f:
            self._send(200, f.read(), ctype, extra)


def main():
    if not DEV and not ALLOWED_LOGINS:
        sys.exit("CXMAIL_REMOTE_LOGINS is empty: refusing to start with nobody allowed in.")
    if not os.path.exists(DB_PATH):
        sys.exit("No CXMail database at {}".format(DB_PATH))
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    sys.stderr.write("cxmail-remote on 127.0.0.1:{}{}\n".format(PORT, " (DEV: no login check)" if DEV else ""))
    srv.serve_forever()


if __name__ == "__main__":
    main()
