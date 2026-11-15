# CXMail Remote

A thin web client that puts the Mac's CXMail on a phone, over Tailscale only.
It is not a second mail client: there is no sync, no IMAP and no credentials here.
Sending goes through CXMail's own signed binary, after you review the draft.

The native SwiftUI phone companion in [`../ios/`](../ios/README.md) uses this same
API, authentication and exact-draft send review. The web client remains supported.

| Does | How |
|---|---|
| Inbox (Primary / Unread / Starred / Updates / Promotions / Social / All), per account | reads `cxmail.db` **read-only** (`mode=ro` + `PRAGMA query_only`) |
| Search | FTS5, with a line-for-line port of `build_match_expr` (gotcha #26; `test_server.py` pins parity) |
| Read a message and its thread | stored ammonia-sanitised HTML, or the signed helper's sanitized preview when the body is not cached; fetched HTML stays in a bounded 15-minute memory cache, never written to SQLite by this service. No-script CSP and blocked remote images apply to both paths. MCP diagnostic headers are never displayed as body text. |
| Archive, star, read/unread, move | calls the shared `cxmail-mcp` HTTP server (`127.0.0.1:18704`), which does the IMAP work and tells the app over the bridge socket |
| Reply, reply all, new message | `compose_draft`, then optionally **Review & send** |
| Drafts tab, **send** | `cxmail-mcp remote preview-draft` shows every From/To/Cc/Bcc and a SHA-256 of the stored draft; `cxmail-mcp remote send-draft` refetches it, refuses if the bytes changed, sends through the compose window's `smtp::send_email`, then expunges that one draft UID. Send needs two taps |
| Native **Schedule send** | the same reviewed draft reference + digest, queued durably by this LaunchAgent; an atomic claim invokes the signed `send-draft` once when due. The phone can be closed. View outcomes or cancel pending sends from the clock in Drafts. |
| Home Screen app | `apple-touch-icon` PNG, `viewport-fit=cover`, locked zoom, a floating glass tab bar (Inbox · Unread · Drafts · Sent · Search; New is the pencil top right) like Telly's, a pushed reader with a bottom action toolbar, edge-swipe back and pull to refresh (Safari's own gestures are gone in a standalone app) |
| Long-press a message (right-click on a Mac) | Open in Claude (T3), Ask Claude in Chat, Reply, Reply all, Archive, Star, Mark read/unread, Move to…, Move to Trash (`delete_email` without `permanent`, so recoverable) |
| **Open in Claude** | `t3 thread start` in the repo `resolve_project_repo` maps the sender to (the desktop's resolver), else the home project, with the desktop handoff's prompt. Uses `~/.t3/bin/t3` first: Homebrew's copy can lag the T3 server's database |
| **Claude chat bubble** | `cxmail-mcp remote chat [<account> <folder> <uid>]`: the desktop chat panel's own launch (`chat::assemble_launch`), flags (gotcha #64) and permission gate (`chat::pump_stdout`), spoken as JSON lines. One chat at a time; the phone long-polls `/api/chat/events`. Its sessions live in `claude-chat-remote`, apart from the panel's. Stopped after an hour without a poll, and whenever this server goes away (EOF) |

It needs CXMail running on the same Mac (for sync), the `cxmail-mcp` HTTP LaunchAgent (for changes), and a CXMail build that has the `remote` subcommand (for sending; an older build answers "Ship the update on the Mac first").

## Run

```bash
remote/install.sh             # LaunchAgent com.cx.cxmail-remote on 127.0.0.1:18706 + Tailscale Serve HTTPS :18706
remote/install.sh uninstall   # stop it and remove the Serve entry
/usr/bin/python3 remote/test_server.py   # tests (fixture DB + a schema check against the real one, read-only)
```

Standard-library Python only (`/usr/bin/python3`, 3.9): no venv, nothing to break. Log: `~/Library/Logs/cxmail-remote.log`.

## Draft freshness

Draft list metadata comes from the mini's read-only cache. If the signed helper
confirms a particular revision no longer exists on IMAP, preview/send returns
HTTP 410 with `code: draft_gone`. The native app removes that exact reference
from its cached lists and refreshes Drafts instead of retrying an obsolete UID.
Confirmed missing revisions are excluded in the service's private
`missing-drafts.json` cache across restarts. The exclusion is scoped to the
account, folder, UID, cached message identity/date and UIDVALIDITY; a new mailbox
generation or changed cached revision can appear again. A successful fresh
preview also clears it. No mailbox row or server message is deleted. Connection,
account and digest errors never exclude a draft. New revisions still arrive
through the mini's normal folder sync; replacements are never guessed by subject.

## Companion schedules

The native review has a floating glass send capsule: paper plane → final **Send
now** confirmation; chevron → **Send now**, **Schedule send…**, or **Scheduled
sends**. Scheduling shows all recipients, including Bcc, a date/time picker in
the phone's timezone, and a separate final approval. Pending schedules can be
cancelled; the draft remains in Drafts.

`scheduled.py` keeps a private, separate SQLite queue at
`~/Library/Application Support/com.cxmail.remote/scheduled.sqlite` (0600).
The mailbox database remains read-only. This queue stores references, approved
digests, due times, subject and To summary; no bodies, Bcc addresses or credentials.
It is separate from desktop compose-window schedules. The resident service scans
every 10 seconds, including after waking; the mini must be awake and online to
deliver. An edited/deleted draft is refused by the signed helper at delivery.
No send is automatically retried: an error or a restart during a claimed send
shows **Check Sent before retrying**. A lost schedule-creation reply cannot queue
the same draft twice. A pending/claimed schedule blocks an immediate phone send
until cancelled. The worker is never started by fixture tests.

Authenticated, CSRF-protected routes: `GET /api/scheduled`, `POST /api/schedule`
(`account`, `folder`, `uid`, `digest`, timezone-aware `send_at`), and
`POST /api/scheduled/cancel` (`id`). GET needs tailnet identity; both POST routes
also require the JSON/header guard. No scheduling or send tool is added to MCP.

## Security

- Bound to `127.0.0.1`; reachable only through Tailscale Serve.
- Every request must carry a `Tailscale-User-Login` in `CXMAIL_REMOTE_LOGINS` (set by `install.sh` to this Mac's own Tailscale user). Serve adds that header; a request without it (a tagged device, a direct local hit) gets 403. The server refuses to start with the list empty.
- **The MCP still cannot send.** The send path is two subcommands of the signed `cxmail-mcp` binary, not MCP tools, so an agent working through the MCP has no send (`send_email` stays refused). A process running as you could run the binary directly, as it could drive the app itself: the boundary is the MCP surface, not the user account. The subcommands only send a message from the account's Drafts folder, and only if it is byte-for-byte what you reviewed: a draft edited anywhere since gets refused. The Keychain trusts the binary by its signature, which is why it has to ship inside the app (`/ship`).
- POSTs need `X-CXMail-Remote: 1` and a JSON body, which a cross-site form can't send without a preflight this server never answers.
- `CXMAIL_REMOTE_DEV=1` turns the login check off for a localhost-only instance that agents and tests drive. Never give one a Serve entry, and point its `CXMAIL_MCP_URL` at a dead port unless you mean to change real mail (opening a message marks it read). Send routes additionally require an isolated `CXMAIL_SCHEDULE_DB`; dev instances never start a scheduling worker. Fixture tests also set a fake `CXMAIL_SEND_BIN`.

## Not here, on purpose

- **Sending drafts with attachments** (refused with "send it from the Mac"), opening attachments (names only), **delete**, **compose formatting**.
- **Undo send.** Two taps is the guard; there is no delay window.
