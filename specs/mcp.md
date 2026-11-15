# MCP Server

Built-in MCP server so Claude can interact with email. Embedded in the Tauri Rust backend using `rmcp` crate. Shares the same database and IMAP connections as the UI.

## Transports

- **stdio**: For Claude CLI `--mcp-config` (primary)
- **SSE**: For remote MCP clients (secondary, Phase 4+)

## Permission Tiers

| Tier | Behavior | Tools |
|------|----------|-------|
| **Open** | No confirmation needed | search_emails, read_email, read_thread, list_folders, list_accounts |
| **Confirm** | UI shows toast notification, user can undo | compose_draft, edit_draft, move_email, archive_email, flag_email |
| **Approve** | UI shows modal, blocks until user clicks Approve/Deny | send_email, delete_email |

Approve tier timeout: 60 seconds. Default on timeout: **reject**.

> **What actually holds today (2026-11-15).** The tiers above describe the app-embedded
> design. The standalone `cxmail-mcp` (stdio, and the shared `--http` server) has no UI, so
> the Approve modal never runs there, and `confirmed=true` on the delete tools is a flag the
> agent sets itself. The real controls are in § Agent boundary below.

## Agent boundary

The MCP runs inside agent sessions that also have a shell, files and the web, and it hands
them mail written by strangers. Three layers, in `crates/cxmail-mcp/src/mcp/untrusted.rs`
and the client:

1. **Fenced output.** Every tool in `UNTRUSTED_OUTPUT` (anything that can carry a body,
   subject, sender name, attachment filename or invitation text) returns its text between
   `<<<UNTRUSTED_EMAIL_CONTENT nonce=…>>>` and `<<<END_UNTRUSTED_EMAIL_CONTENT nonce=…>>>`,
   a fresh random nonce per call, with marker-shaped text inside defanged. The convention
   is also sent as server `instructions` at initialize. Wrapping happens once, in the
   hand-written `call_tool` (not `#[tool_handler]`, which in rmcp-macros 1.3 always
   generates its own). Every tool must be listed in exactly one of `UNTRUSTED_OUTPUT` /
   `SERVER_AUTHORED_OUTPUT`; `every_tool_is_classified_for_untrusted_output` fails
   otherwise. Draft tools stay server-authored: the chat panel parses their result line.
2. **Server hard limits** (no agent can lift them): `download_attachment` never overwrites
   (`name (1).ext`, `create_new`), refuses dot-files, hidden directories and `~/Library`
   (except iCloud Drive and `CloudStorage`) including through symlinks, and sets
   `com.apple.quarantine` on what it saves. Draft `attachments` get the same path refusal,
   because a draft is APPENDed to the provider before anyone reviews it.
   `read_attachment_text` is the preferred way to read a document: the text comes out of the
   inert extractor (`document_text`, by the type the bytes sniff as), invisible Unicode is
   stripped, DOCX hidden/white/≤2 pt runs are surfaced in a separate section, and a
   server note (a `_meta`-marked block `wrap_result` leaves unfenced) gives the real type and
   flags from `email::attachment_safety`. PDF hidden text is not detectable and the note says
   so. `mode="summary"` sends redacted text through the triage gate's `final_check` to a
   tool-less model instead. `list_attachments` adds name-only flags the same way.
3. **Client approval.** Only the client can ask the human. For Claude Code, a PreToolUse
   hook on `mcp__cxmail__.*` with this policy: ask for `delete_email`,
   `bulk_delete_emails`, `move_email` into Trash/Junk/Spam, `apply_mail_rule` with
   `dry_run=false`, mail-rule and voice-rule writes, `delete_group`, `send_email`, and
   drafts with attachments; `download_attachment` always asks (with a warning for risky
   types) and denies what the server refuses. A second Bash hook asks before a shell
   command runs, opens, unpacks or `chmod +x`es a quarantined file, or strips the flag,
   since a shell never consults Gatekeeper. CXMail's in-app chat already routes every mutating tool
   to its own ask tier (`chat_agent::mutating_tools_are_never_auto_allowed`).
   `send_calendar_invites` needs the in-app approval card regardless.

4. **Per-message risk check.** On an agent's first contact with a message (`read_email`,
   `read_thread`, `list_attachments`) the server runs `email::risk::assess` and stores the
   report in `message_risk` (v66), keyed by account + bare Message-ID so a moved message is
   not re-assessed; `RULES_VERSION` invalidates stale reports. Sender standing ("first-time
   sender", from `needs_you::sender_trust`, cached 10 min in-process) is computed live and
   never stored. The result is a server note: level, then each signal. **A level only ever
   tightens:** LOW skips no approval and drops no warning. The approval hook reads the
   stored level and makes a download prompt from a MEDIUM/HIGH email louder.
   Calibrated on 3,000 real inbox messages (`cxmail-email` example `risk_probe`, aggregates
   only): first pass 62.6% medium, final 99.9% low. What the first pass got wrong is now
   written into the rules: hidden layout blocks and preheader padding are info; link text
   counts only when the whole text is a URL, and sponsor/tracker mismatches are info (a
   judgment for the model half, not a rule); a shared prefix is not a typosquat
   (`northwind` vs `northwind06`); invoicing platforms make display-name mismatches medium, not
   high; DMARC aggregate reports are not risky attachments; mentions of "system prompt"
   are info, but text that addresses an agent and tells it to act is medium, and anything
   instruction-like in HIDDEN text, or Unicode tag characters, is high.

   **Model half (Decisions, `email::risk_model`).** On first contact the triage gate's
   redacted payload is asked three predicates (phishing, AI-instructions, impersonation);
   the verdict is stored in `message_risk` under `MODEL_VERSION`, or the gate's withheld
   reason is. The handler awaits the call with no connection borrowed: `risk_prepare` (sync)
   → `risk_model_step` (async) → `risk_finish` (sync). Thresholds, measured by
   `cxmail-mcp --risk-eval --referee` against a tool-less Opus referee on 706 real messages
   (17 phishing): phishing ≥ 0.5 → MEDIUM (caught 12/17, flagged 4/648 others), ≥ 0.8 → HIGH
   (9/17, 0/648); AI-instructions ≥ 0.5 → HIGH (no real examples yet); impersonation is
   shown, never raised on (5% false positives). The rules alone caught 0/17. Off switch:
   credential `ai:risk:model` = `off`. Needs the OpenAI provider configured in CXMail.

Not covered: clients without a hook (Codex, other MCP hosts) get layers 1, 2 and 4 only.

## Tool Definitions

### Read Tools (Open Tier)

```
search_emails(query: string, account_ids?: string[], since?: string, until?: string, limit?: number) → SearchResult[]
  - SQLite FTS5 (`db::search::search`, the same engine as the UI), LIKE fallback on a pre-v40 DB
  - Query syntax: "from:boss after:2026-03-01 has:attachment subject:invoice"
  - Returns: uid, subject, from, date, snippet
  - Without account_ids, accounts hidden from aggregated views are SKIPPED and
    the result ends with a line naming them; pass their IDs to search them

read_email(account_id: string, folder: string, uid: number) → MessageDetail
  - Returns full message content (sanitized HTML + plain text + metadata)
  - Echoes Folder / UID / normalized Message-ID in the header block — feed the
    first two straight to compose_draft as reply_to_folder + reply_to_uid

read_thread(message_id: string) → MessageDetail[]
  - Returns all messages in conversation thread
  - Follows In-Reply-To and References headers
  - Each line carries Folder + UID + Message-ID (UIDs are folder-scoped, so the
    folder is required to address a message)

list_folders(account_id?: string) → Folder[]
  - Returns folder hierarchy with unread counts
  - If no account_id, returns all accounts' folders

list_accounts() → Account[]
  - Returns configured accounts (email, provider, status, open tracking,
    "Hidden from aggregates: yes/no")
```

### Write Tools (Confirm Tier)

```
compose_draft(to: string[], subject: string, body: string, account_id?: string) → Draft
  - Creates draft in IMAP Drafts folder; the running app's Drafts list refreshes
    (bridge `draft-created`), but no compose window opens — that is open_draft
  - Does NOT send — user reviews first
  - Result ends with the open_draft hint and the draft's `cxmail://draft/<draft_id>` link

edit_draft(draft_id: string, ...fields) → Draft
  - Updates an existing draft (recipients, subject, body, cc/bcc); an open
    compose window for it reloads (bridge `draft-updated`)
  - (There is no reply_draft tool — use compose_draft to create, edit_draft to revise)

open_draft(draft_id?: string, account_id?: string, uid?: number) → text
  - Opens that exact draft in the compose window and brings CXMail to the front
  - draft_id (stable across saves and across Macs) or account_id + uid
  - Resolves against this Mac's DB (retries ~3 s for a draft still syncing in),
    then sends bridge `open-draft` and activates the app by bundle PATH
    (never `open -b`: LaunchServices also knows the build output's bundle)
  - App not running → `open -a <bundle> cxmail://draft/<id>`, which launches it;
    the cold-start deep-link read opens the draft
  - Unknown draft → error, nothing opened

Deep link: `cxmail://draft/<draft_id>` or `cxmail://draft/<account_id>/<uid>`
(`src/lib/draftLink.ts`, resolved by the `resolve_draft_link` command).
```

#### Reply threading & quoted history

`compose_draft` and `edit_draft` share one reply contract. Assembly order in the
finished draft is **body → signature → quoted history**, matching
`ComposeModal.tsx`; that ordering is the point, so the agent must never paste
thread history into `body` (doing so lands the signature *below* the quote).

| Param | Type | Notes |
|---|---|---|
| `reply_to_folder` | `string?` | **Preferred**, with `reply_to_uid`. Folder as printed by `read_email` / `search_emails` / `read_thread`. |
| `reply_to_uid` | `number?` | **Preferred**, with `reply_to_folder`. IMAP UIDs are folder-scoped, so the pair is the smallest unambiguous key — and it's the same `(account_id, folder, uid)` key the app itself quotes from. |
| `reply_to_message_id` | `string?` | Fallback, used only when the pair is omitted. Accepts `<id@host>`, bare `id@host`, and HTML-escaped `&lt;id@host&gt;` (gotcha #30). |
| `quote_original` | `boolean?` | Default `true`. `false` opts out of the quote entirely. |

Resolution rules:

- Coordinates **win** over `reply_to_message_id` when both are supplied.
- Half a pair (`reply_to_folder` without `reply_to_uid`, or vice versa) is an
  `invalid_params` error — never a silent fallback.
- Threading headers (`In-Reply-To` / `References`) come from the same row being
  quoted. Both are emitted in canonical bracketed, space-separated form.
- A quote that cannot be built is a **hard error** naming the reason and the
  remedy (`NotInDb`, `NoLocalRow`, `ImapConnect`, `ImapSelect`, `ImapFetch`,
  `NoRenderableBody`) — not a quote-less draft. The error is raised before the
  IMAP APPEND and before `edit_draft`'s delete+expunge, so no draft is created
  and none is destroyed.
- Deliberate opt-outs are **not** errors: `quote_original=false`, and a `body`
  that already contains a `cx-quote` block.
- On success the tool result ends with `+ quoted original`.
- When the original body isn't cached it is fetched once over IMAP and written
  back via `upsert_body_preserving_metadata`, so later replies in the thread
  need no round-trip.

```

move_email(uid: number, from_folder: string, to_folder: string) → void
  - Moves message between folders
  - UI shows undo toast for 10 seconds

archive_email(uid: number) → void
  - Moves to archive folder

flag_email(uid: number, flag: "starred" | "unstarred" | "read" | "unread") → void
  - Updates message flags
```

### Dangerous Tools (Approve Tier)

```
send_email(draft_id: string) → SendResult
  - Actually sends the email via SMTP
  - UI shows full email preview in modal: recipients, subject, body
  - User must click [Approve] to send, [Edit] to modify, [Deny] to cancel
  - Blocks MCP response until user acts (60s timeout → reject)

delete_email(uid: number, permanent?: boolean) → void
  - permanent=false (default): moves to Trash
  - permanent=true: permanently deletes (EXPUNGE)
  - UI shows modal with message preview and permanent/trash option
```

## Approval Flow

```
1. MCP tool call arrives (e.g., send_email)
2. Rust MCP handler checks permission tier → "approve"
3. Rust emits Tauri event: "mcp-approval-request" with details
4. Frontend shows approval modal:
   ┌─────────────────────────────────────┐
   │  Claude wants to send an email      │
   │                                     │
   │  To: sarah@example.com              │
   │  Subject: Re: Project Timeline      │
   │  Body: [preview]                    │
   │                                     │
   │  [Deny]  [Edit]  [Approve]          │
   └─────────────────────────────────────┘
5. User clicks Approve → Rust sends the email → MCP returns success
6. User clicks Deny → MCP returns error "User denied"
7. User clicks Edit → Opens compose UI, MCP returns "User editing"
8. 60s timeout → MCP returns error "Approval timed out"
```

## MCP Server Registration

> Full walkthrough (prereqs, app-first dependency, troubleshooting): **[`docs/mcp-setup.md`](../docs/mcp-setup.md)**.

The server is a dedicated **`cxmail-mcp`** binary built from this crate
(`[[bin]] name = "cxmail-mcp"`, source at `src-tauri/src/bin/mcp.rs`). It speaks MCP over **stdio**
and takes **no flags** (it is argless — there is no `--stdio`, `--mcp`, or `--mcp-server` flag). It
shares the desktop app's SQLite database and credential store, so the app must be run and an account
added first.

Build it:

```bash
cd src-tauri && source "$HOME/.cargo/env" && cargo build --release --bin cxmail-mcp
# → src-tauri/target/release/cxmail-mcp
```

Register it one of two ways:

- **Project `.mcp.json`** (committed at the repo root) — open the repo in Claude Code and approve the
  `cxmail` server when prompted:
  ```json
  { "mcpServers": { "cxmail": { "command": "./src-tauri/target/release/cxmail-mcp" } } }
  ```
- **Global / manual** — `claude mcp add cxmail /abs/path/to/src-tauri/target/release/cxmail-mcp`
  (no args).
