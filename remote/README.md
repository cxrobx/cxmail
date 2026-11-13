# CXMail Remote

A thin web client that puts the Mac's CXMail on a phone, over Tailscale only.
It is not a second mail client: there is no sync, no IMAP and no credentials here.
Sending goes through CXMail's own signed binary, after you review the draft.

| Does | How |
|---|---|
| Inbox (Primary / Unread / Starred / Updates / Promotions / Social / All), per account | reads `cxmail.db` **read-only** (`mode=ro` + `PRAGMA query_only`) |
| Search | FTS5, with a line-for-line port of `build_match_expr` (gotcha #26; `test_server.py` pins parity) |
| Read a message and its thread | the stored ammonia-sanitised HTML, served with a no-script CSP into a sandboxed iframe; remote images off unless the sender is in `trusted_image_senders` or you tap *Show images* |
| Archive, star, read/unread, move | calls the shared `cxmail-mcp` HTTP server (`127.0.0.1:18704`), which does the IMAP work and tells the app over the bridge socket |
| Reply, reply all, new message | `compose_draft`, then optionally **Review & send** |
| Drafts tab, **send** | `cxmail-mcp remote preview-draft` shows every From/To/Cc/Bcc and a SHA-256 of the stored draft; `cxmail-mcp remote send-draft` refetches it, refuses if the bytes changed, sends through the compose window's `smtp::send_email`, then expunges that one draft UID. Send needs two taps |
| Home Screen app | `apple-touch-icon` PNG, `viewport-fit=cover`, locked zoom, a floating glass tab bar (Inbox · Unread · Drafts · Search · New) like Telly's, a pushed reader with a bottom action toolbar, edge-swipe back and pull to refresh (Safari's own gestures are gone in a standalone app) |
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

## Security

- Bound to `127.0.0.1`; reachable only through Tailscale Serve.
- Every request must carry a `Tailscale-User-Login` in `CXMAIL_REMOTE_LOGINS` (set by `install.sh` to this Mac's own Tailscale user). Serve adds that header; a request without it (a tagged device, a direct local hit) gets 403. The server refuses to start with the list empty.
- **The MCP still cannot send.** The send path is two subcommands of the signed `cxmail-mcp` binary, not MCP tools, so an agent working through the MCP has no send (`send_email` stays refused). A process running as you could run the binary directly, as it could drive the app itself: the boundary is the MCP surface, not the user account. The subcommands only send a message from the account's Drafts folder, and only if it is byte-for-byte what you reviewed: a draft edited anywhere since gets refused. The Keychain trusts the binary by its signature, which is why it has to ship inside the app (`/ship`).
- POSTs need `X-CXMail-Remote: 1` and a JSON body, which a cross-site form can't send without a preflight this server never answers.
- `CXMAIL_REMOTE_DEV=1` turns the login check off for a localhost-only instance that agents and tests drive. Never give one a Serve entry, and point its `CXMAIL_MCP_URL` at a dead port unless you mean to change real mail (opening a message marks it read).

## Not here, on purpose

- **Sending drafts with attachments** (refused with "send it from the Mac"), opening attachments (names only), **delete**, **compose formatting**.
- **Undo send.** Two taps is the guard; there is no delay window.
