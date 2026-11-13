# CXMail Remote

A thin web client that puts the Mac's CXMail on a phone, over Tailscale only.
It is not a second mail client: there is no sync, no IMAP and no credentials here.

| Does | How |
|---|---|
| Inbox (Primary / Unread / Starred / Updates / Promotions / Social / All), per account | reads `cxmail.db` **read-only** (`mode=ro` + `PRAGMA query_only`) |
| Search | FTS5, with a line-for-line port of `build_match_expr` (gotcha #26; `test_server.py` pins parity) |
| Read a message and its thread | the stored ammonia-sanitised HTML, served with a no-script CSP into a sandboxed iframe; remote images off unless the sender is in `trusted_image_senders` or you tap *Show images* |
| Archive, star, read/unread, move | calls the shared `cxmail-mcp` HTTP server (`127.0.0.1:18704`), which does the IMAP work and tells the app over the bridge socket |
| Reply, reply all, new message | `compose_draft`. **It never sends.** Drafts are sent from CXMail on the Mac |

It needs CXMail running on the same Mac (for sync) and the `cxmail-mcp` HTTP LaunchAgent (for changes).

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
- POSTs need `X-CXMail-Remote: 1` and a JSON body, which a cross-site form can't send without a preflight this server never answers.
- `CXMAIL_REMOTE_DEV=1` turns the login check off for a localhost-only instance that agents and tests drive. Never give one a Serve entry, and point its `CXMAIL_MCP_URL` at a dead port unless you mean to change real mail (opening a message marks it read).

## Not here, on purpose

- **Sending.** Needs the approval design in `specs/mcp-send-approval-bridge.md` and a spec review first.
- **Attachments** (names only), **delete**, **compose formatting**.
