---
paths:
  - "src-tauri/**/*.rs"
  - "src-tauri/Cargo.toml"
  - "src-tauri/crates/*/Cargo.toml"
---

# Backend Patterns

## Tech Stack
Rust, Tauri 2, Tokio, rusqlite (bundled SQLite incl. FTS5 search), async-imap, mail-send, ammonia, rquickjs

## Crate Layout

`src-tauri/` is a **Cargo workspace**, not a single package. Know which crate a
file is in before you edit it — `crate::` means something different in each.

| Crate | Path | Holds | Links Tauri |
|---|---|---|---|
| `cxmail-core` | `crates/cxmail-core/` | `AppError`, `LockExt`, `secrets`, `keychain/`, `EventSink`/`AppCtx`, the MCP socket `bridge`, and `mail/` — the pure helpers `db` and `email` both need | no |
| `cxmail-db` | `crates/cxmail-db/` | `db/` — migrations (v58), message store, FTS5 | no |
| `cxmail-email` | `crates/cxmail-email/` | `email/` — IMAP, SMTP, OAuth2, parser, autoconfig, AI. Owns `build.rs` + `resources/ispdb/` | no |
| `cxmail-mcp` | `crates/cxmail-mcp/` | `mcp/server.rs` | no |
| `cxmail` (app) | `src/` | `lib.rs`, `commands/` (190 IPC commands), `idle.rs`, `notify*`, `plugins/`, `tracker/`, **all four `bin/` targets** | yes |

Dependencies point one way: **`core <- db <- email <- mcp <- app`**. Every crate
under `crates/` is lib-only.

Two rules that keep this from rotting:

1. **Never add `tauri` to a member crate.** Four of five are provably Tauri-free
   (`cargo tree -p cxmail-db | grep -c tauri` → 0) and that is the entire payoff —
   `cargo test -p cxmail-db` compiles 83 dependencies instead of 751. Code that
   needs to emit takes a `cxmail_core::EventSink`; code that needs the connection
   *and* a way to emit takes a `cxmail_core::AppCtx`. `email::oauth2` and
   `email::gcal_invite` are the worked examples.
2. **Never move a `[[bin]]` out of the app package.** Tauri's bundler enumerates
   that package's bin targets, and a binary missing from
   `cxmail.app/Contents/MacOS/` is one `keychain::macos_acl` silently skips —
   permanent Keychain prompts, no error (gotcha #31). It also keeps every
   `--bin cxmail-*` command in the repo working with no `-p`.

`lib.rs` re-exports each moved module at its historical path (`pub use
cxmail_db::db;` and friends), so `cxmail_lib::db::…` still resolves from the
bins and examples. Add to the shim rather than re-pathing call sites.

`cargo test --workspace` — plain `cargo test` at the root tests only the app
package and would stay green while covering none of the extracted crates.

## Command Modules

IPC commands in `src-tauri/src/commands/` — app package, each file a domain:
- `accounts.rs` — CRUD, sync triggers
- `ai.rs` — AI writing assistant (generate reply, rewrite, tone adjust)
- `auth.rs` — OAuth2 flow initiation/completion
- `calendar.rs` — Calendar event parsing, RSVP
- `categories.rs` — Email category management
- `chat.rs` — The in-app Claude chat: one `claude -p` process per conversation, CXMail answering its permission prompts (pure half: `email::chat_agent`; see gotcha #64)
- `compose.rs` — Send (with undo delay), save draft, attachments
- `folders.rs` — List, create, rename, delete
- `followup.rs` — Follow-up reminder CRUD
- `import.rs` — Mbox import
- `messages.rs` — Fetch, mark read/unread, move, delete, flag, search, unsubscribe
- `plugins.rs` — Plugin lifecycle (install, enable, trigger)
- `schedule.rs` — Scheduled send CRUD
- `settings.rs` — App settings
- `snooze.rs` — Snooze/unsnooze messages
- `summarize.rs` — AI email summarization
- `templates.rs` — Email template CRUD
- `tracking.rs` — Open tracking config and pixel management

## Error Handling

- `thiserror` for typed domain errors (`AppError` enum)
- `anyhow` for ad-hoc errors in internal logic
- All commands return `Result<T, AppError>` — serialized to frontend
- Pattern: `src-tauri/crates/cxmail-core/src/error.rs`

## Database Layer

`cxmail-db`'s `db/` modules mirror command domains. Schema migrations in `db/schema.rs` (v58).

Six migrations call helpers that live in `cxmail-core::mail` (`detect_events`, `dashes::scrub_profile_json`, `text::repair_double_encoded_utf8`, `text::normalize_unstructured_header`, `date::normalize_date_to_iso8601`, `message_id::compute_thread_root_id`). That is not incidental — those helpers were in `email/`, and the migrations needing them were half of what made `db` and `email` mutually dependent. Probe the chain against a copy of a real database with `cargo run -p cxmail-db --example migrate_probe -- <copy.db>`.

Tables: accounts, folders, messages, message_bodies, message_headers (verbatim RFC 5322 header blocks, v46 — see gotcha #37), attachments, sync_state, mail_rules, identities, snoozed_messages, scheduled_emails, tracking_pixels, tracking_config, sender_categories, followup_reminders, email_templates, calendar_events, unsubscribed_senders, account_groups, needs_you_dismissals, nudge_dismissals (per-thread, per-lane, v47 — see gotcha #40), voice_pinned_rules (human-authored absolute writing rules, v51 — kept OUT of `voice_profiles_recipient.profile_json` so a forced rebuild can't erase them; see gotcha #43), zoom_meetings (which Zoom meeting backs which Google Calendar event, v52 — a SIDE table keyed on the remote `(account_id, gcal_calendar_id, gcal_event_id)` triple with **no foreign keys**, because a column on `gcal_events` would be nulled by `upsert_remote_event`'s remote-driven ON CONFLICT and destroyed by four deleters; see gotcha #44), claude_repos (which repo an "Open in Claude" handoff lands in, v56 — four scopes resolved most-specific-first; **both foreign keys cascade**, and the `inbox_groups` one is load-bearing because that table's ids are reused after a delete, so an orphan would silently reattach to an unrelated group; see gotcha #49), invite_notifications (which attendees we actually asked Google to notify, v57 — Google records **no** such fact on the event, and `responseStatus` reads `needsAction` whether someone was invited or was never told, so the ledger has to be ours; another SIDE table on the remote triple with **no foreign keys**, for `zoom_meetings`' reasons, written strictly after the 2xx and never backfilled — history honestly reads `unknown`; see gotcha #50), messages_fts (FTS5, trigger-maintained — never write directly).

### Voice: derived vs pinned

Two stores, and the split is load-bearing. `voice_profiles_recipient.profile_json` is LLM-authored, statistical and rebuildable (`extract_recipient_profile(force=true)` overwrites it). `voice_pinned_rules` is user-authored, absolute and permanent — scoped `account` (every message) or `recipient` (one address), read by `db::voice_pinned_rules::list_effective` in account-then-recipient order so the specific rule wins. Both reach a drafting agent: in-app through `email::ai::build_voice_context_full`'s `<pinned-rules>` block (emitted first), over the MCP through `get_voice_profile`'s top-level `pinned_rules` field, `list_voice_rules`, and the `⚠ PINNED RULES` note `compose_draft`/`edit_draft` append. Never merge the two.

**A pinned rule that says *never* needs an enforcement point, not just a louder prompt.** The `⚠ PINNED RULES` note rides in the tool *result*, so it lands after the draft is already on IMAP, and a derived profile that accurately describes the banned habit argues against it in the same context window. `email::dashes::normalize_dashes` is the deterministic half for the em-dash rule: `mcp::server::enforce_dash_rule` runs it in both draft handlers (on `params.subject`/`params.body` only, above the IMAP work and before the signature and quote are attached), and every prose generator in `email::ai` goes through `call_inference_prose`. v55 scrubs the claim out of stored profiles. See gotcha #47.

### Nudges

`db::nudges::list` builds both stalled-conversation lanes — "Sent Nd ago. Follow up?" and "Received Nd ago. Reply?" — from one question asked of the newest message in each thread: ours, or theirs. The filter is the feature: the naive rule nudges 299 of 317 recent sends because cold outreach is outbound mail that never gets a reply. Both lanes gate on `Correspondence::repliers` (people who have answered us at least once), NOT on `CORRESPONDENCE_THRESHOLD`, which outbound volume alone satisfies. The reply lane additionally defers to `email::needs_you::evaluate` so the automation vocabulary has one definition. See gotcha #40 for the threading repair that makes the follow-up lane non-empty.

### Search

`db::search::search(conn, &SearchFilters, limit, offset, prefix)` is the single engine (BM25 + recency, snippets with U+E000/U+E001 highlight markers, porter stemming, prefix indexes). User keywords must go through `db::search::build_match_expr` — never interpolate raw text into a MATCH. Deterministic operator parsing lives in `cxmail-email`'s `email/search_parse.rs`; provider-native server-side query builders in `email/server_search.rs`. FTS invariants: gotcha #26.

### Account Group Filtering

`fetch_unified_inbox` and `get_category_counts` accept optional `account_ids: Vec<String>` for scoping queries to a subset of accounts. The frontend derives these from `selectedAccountGroup` → matching account IDs. DB functions use dynamic `IN (?)` placeholders with indexed params.

**Hidden accounts (`accounts.hidden_from_aggregates`, v58).** Every aggregate query — `list_all_inboxes` (with *and* without ids: an account folder is still an aggregate), `count_total_inbox_unread`, both `count_unread_by_category_*`, `list_messages_for_group` / `count_unread_for_group`, `needs_you::list`, `nudges::list`, and `search::search` when `account_ids` is `None` — embeds `db::accounts::visible_in_aggregates_sql(expr)`, the one predicate (`NOT EXISTS`, so a missing account row fails open). `search` is `None`-only because naming the account is the one way its mail is searchable; the frontend must therefore never collapse an empty id list to `undefined` (see gotcha #54). Single-account queries (`list_by_folder`, per-account `count_unread_by_category`) are untouched — clicking the account is the way in.

## Email Protocol Handling

- **IMAP**: `async-imap` via `tokio_util::compat` bridge (see Gotcha #3). 15s connect timeout, 45s per-account sync timeout. Single entry point `imap::connect_for_account(&Account)`; only the generic `imap` provider reads host/port/security from the DB row.
- **Autodiscovery**: `email::autoconfig::discover()` — the ISPDB table is generated by
  `cxmail-email`'s own `build.rs` into that crate's `OUT_DIR`; `build.rs` and
  `resources/ispdb/` must stay in the same crate as `autoconfig.rs`, since `OUT_DIR` is per-crate — presets → bundled ISPDB (vendored MPL-2.0, 969 domains) → the user's own `autoconfig.<domain>` over HTTPS → DNS SRV → MX. Never connects, never sees the password. Full reference: [`docs/mail-providers.md`](../../docs/mail-providers.md).
- **SMTP**: `mail-send` with provider-aware auth (OAuth2 or password)
- **Parser**: `mail-parser` + ammonia sanitization + RFC 2047 decoding. All `parse_message` calls wrapped in `catch_unwind` (see Gotcha #9).

## Sync Resilience

- `connect_for_provider` has 15s timeout to prevent dead-socket hangs (Gotcha #10)
- `sync_all_inboxes` has 45s per-account timeout; one hung account doesn't block others
- DB lock scopes are kept small — insert, classify, bodies, thread counts in separate scopes (Gotcha #11)
- `sanitize_html` falls back to escaped plain text on panic (Gotcha #9)
- Background sync in `AppLayout.tsx` guards against overlapping calls

## Undo Send Pattern

Send with delay uses tokio::select! on timer vs cancellation channel:
1. Frontend calls `send_email` with `delay_seconds`
2. Backend stores `send_id -> cancel_tx` in `AppState.pending_sends`
3. Timer expiry: sends via SMTP, emits "send-completed"
4. Cancel signal: emits "send-cancelled"
Pattern: `src-tauri/src/commands/compose.rs` (app package)

## Plugin System

QuickJS (rquickjs) sandboxed runtime. Plugins defined by manifest.json with triggers (on_message_received, etc.). Runtime in `src-tauri/src/plugins/runtime.rs` (app package).
