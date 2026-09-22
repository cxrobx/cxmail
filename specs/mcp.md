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
  - Creates draft in IMAP Drafts folder
  - Opens compose UI in the app with the draft loaded
  - Does NOT send — user reviews first

edit_draft(draft_id: string, ...fields) → Draft
  - Updates an existing draft (recipients, subject, body, cc/bcc)
  - Opens compose UI
  - (There is no reply_draft tool — use compose_draft to create, edit_draft to revise)
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
