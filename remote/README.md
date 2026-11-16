# CXMail Remote

A thin web client that puts the Mac's CXMail on a phone, over Tailscale only.
It is not a second mail client: there is no sync, no IMAP and no credentials here.
Sending goes through CXMail's own signed binary, after you review the draft.

The native SwiftUI phone companion in [`../ios/`](../ios/README.md) uses this same
API, authentication and exact-draft send review. The web client remains supported.

| Does | How |
|---|---|
| Inbox (Primary / Unread / Starred / Updates / Promotions / Social / All), per account | reads `cxmail.db` **read-only** (`mode=ro` + `PRAGMA query_only`) |
| Native search | Fast FTS5 results, then `/api/search`: shared Rust operator/date parsing, weighted BM25, attachment passages and local semantic retrieval merged with reciprocal rank fusion. Ranked snapshots keep pagination stable; account visibility and semantic opt-out are rechecked. |
| Read a message and its thread | stored ammonia-sanitised HTML, or the signed helper's sanitized preview when the body is not cached; fetched HTML stays in a bounded 15-minute memory cache, never written to SQLite by this service. No-script CSP and blocked remote images apply to both paths. The human-reader adapter verifies and removes the MCP's matching-nonce envelope and excludes server-note blocks before parsing the preview/header format. MCP diagnostic headers and agent safety markers are never displayed as body text; AI consumers keep the original fenced output. |
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

## Sender authentication and image trust

The native app, web reader and desktop share `trusted_image_senders` on the mini.
`image_policy` on `/api/message` comes from the same Rust implementation as desktop;
its automatic permission requires explicit sender trust, passing Google receiver
DMARC evidence for this message's From domain, and no Medium/High risk/Junk flag.
Missing evidence or an older/unavailable helper leaves images blocked. The legacy
`images_trusted` flag now carries this gated permission, not address membership.

The human action `POST /api/image-trust` takes an exact account/folder/UID and a
boolean `trusted`; it invokes the signed helper once, never adds an MCP tool and
never writes through this service's read-only mailbox connection. `/api/body`
rechecks `images=auto`; `images=1` is the explicit per-message override. The final
`X-CXMail-Images-Allowed` response header binds native WKWebView permission to
that body response, so a stale metadata decision cannot override a revocation. HTTPS
placeholders are restored only after loading is allowed. Email scripts remain
blocked. Authentication is domain identity evidence, not a content-safety verdict
or Gmail's BIMI blue check. See `../specs/security.md` for the trust boundary.

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

## Native hybrid search

`GET /api/search?q=...&account=...` returns `results`, `semantic_status` and
`next_cursor`. Follow `cursor` with the same query/account for an immutable ranked
snapshot (five minutes, 20 snapshots maximum). Queries are limited to 2,000
characters. Initial retrieval has two concurrent slots, a bounded queue and an 18-second helper
budget; the native app already displays keyword matches while it runs. Expired
cursors require a refreshed search. Existing `/api/messages?q=...` remains the
fast keyword and legacy web-client path.

The signed `cxmail-mcp remote search` path opens the mailbox read-only and skips
startup migrations. `CXMAIL_SEARCH_BIN` can select the separately signed live
helper when the desktop bundle has not been swapped; `remote/install.sh` persists
that path. No send operation or new MCP tool is added. Exact quoted/negative
queries use the whole-message FTS lane; explicit metadata/account filters constrain every lane.
“Most recent”, “latest” and “newest” at the beginning request date ordering over
matching candidates. Retrieval is bounded to 500 candidates per lane; source
coverage remains cached mail and extracted attachment text, not uncached IMAP.

`local-vault` uses the existing loopback-only EmbeddingGemma 2 service on :11436,
with its actual Ollama-compatible `/api/embed` protocol and distinct document/query
task prefixes. Its vectors use a separate endpoint/model fingerprint. A 0.70
minimum cosine gate replaces the inappropriate generic 0.35 cutoff for this model.
Run `python3 scripts/eval-mail-retrieval.py` from the repo root for the live,
fiction-only positive/null-case check. This small check is a smoke benchmark, not
proof of relevance for every real-mail query. No judgment/reranking API is enabled.

After explicit local activation (`remote enable-local-search`), the production
remote service runs bounded local indexing passes (two passages/batch, 25-second
budget, five-second catch-up pause; two-minute idle/error pause). It uses the same
per-account opt-in and compare-and-set vector writes as desktop. Prepare-reply
settings remain unchanged. Hidden accounts are not background-indexed. The worker
is disabled in fixture/dev servers and refuses remote embedding providers. The
service shows partial coverage until catch-up completes; the phone remains usable
through provider outages. Restarting the remote service interrupts remote chat, so
do it between chat turns. Tests never activate indexing against the real mailbox.

Native requests include a per-launch client id and increasing generation. Queued
obsolete generations are rejected before a helper is started. A busy queue returns
`429 / search_busy`; the native client retries once. Expired snapshots return
`410 / search_expired` and refresh page one rather than declaring the mini offline.
The fast native keyword pass uses `mode=keyword` through the same Rust parser,
filters and deduplication as hybrid search. The legacy web endpoint is unchanged.

Exact exclusions are message-wide: passage retrieval is disabled for strict
queries so a forbidden term in a later passage cannot leak through an earlier
matching passage. “Most recent” places dated lexical matches ahead of looser
semantic-only suggestions. A cold model has a twelve-second query budget; keyword
results remain available while it loads. Local-vault model names are validated
and fixed in settings so task profiles/cutoffs cannot silently drift.

The 44,117-passage/768-dimension synthetic scale probe reads ~374 MB of serialized
vectors and measured ~0.78 seconds in optimized Rust on this mini. Re-run with
`cargo test --release -p cxmail-email full_mailbox_vector_scan_budget --lib -- --ignored --nocapture`
from `src-tauri/` (use the normal OAuth build environment). It is deliberately
excluded from ordinary tests. This measures the vector lane, not full cold-query
latency. Two long passages fit the local GPU where sixteen exhausted its memory;
input rejections are isolated, while network/provider failures remain pending.
The two-passage GPU bound lives in the shared embedding client too, so the
desktop's larger logical batches cannot bypass it. The explicit fictional live
regression is `cargo test -p cxmail-email local_vault_long_batch_is_split_before_gpu_submission --lib -- --ignored`.

Activating `local-vault` requires the updated desktop bundle as well as the signed
companion helper: older desktop builds cannot decode that new service choice.
The helper can be deployed first for the phone, but finish the desktop bundle
swap through the repository's shipping gate to restore its intelligence Settings.

## Search result previews

The native search list displays the same saved message preview as Inbox. If it is
missing, the service uses a bounded excerpt from the already-cached plain body;
if neither exists, the preview stays empty. Retrieval snippets are never used as
human previews: FTS may select its matching recipient column, whose storage is
JSON, and semantic/document hits describe passages rather than the mail's opening.
Ranking, matched references and cursor ordering are unaffected. No body fetch or
mail mutation is performed to populate list previews. `remote/test_server.py`
reproduces the real FTS recipient-JSON behavior and covers saved-preview,
cached-body and missing-body cases. The deployed keyword and hybrid paths were
checked against canonical message text for 50 live results each, without opening
or changing mail. This service-side fix requires refreshing Search, not a new iOS
build.
