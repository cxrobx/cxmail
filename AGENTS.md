# CXMail

macOS-first email client. Rust (Tauri 2) backend, React 19 + TypeScript frontend.

## Specs

All specifications live in `specs/`. The lookup table is `specs/README.md`. The implementation plan is `specs/plan.md`. Read these first.

## Tech Stack

| Layer | Technology |
|-------|------------|
| Desktop | Tauri 2 (macOS-private-api) |
| Frontend | React 19, TypeScript, Vite, Tailwind CSS 4, Radix UI |
| Backend | Rust, Tokio, rusqlite (SQLite), async-imap, mail-send |
| Search | Tantivy full-text index |
| State | Zustand (accountStore, mailStore, uiStore) |

## Golden Commands

```bash
source "$HOME/.cargo/env" && npx tauri dev   # Dev mode (hot reload)
source "$HOME/.cargo/env" && npm run tauri build  # Production build
npm run dev                                   # Frontend only (Vite)
cd src-tauri && cargo check                   # Check Rust compiles
cd src-tauri && cargo test                    # Run Rust tests
```

**Important:** Always `source "$HOME/.cargo/env"` before cargo/tauri commands.

## Critical Invariants (DO NOT BREAK)

1. **No credentials in SQLite** — encrypted file store (dev) or macOS Keychain (production)
2. **HTML always sanitized** — ammonia processes all HTML before storage and display
3. **Email rendered in sandbox** — iframe with `sandbox="allow-same-origin"`, no scripts
4. **TLS enforced** — native-tls, no plaintext fallback
5. **IPC validates all input** — Rust commands never trust frontend data

Full list with patterns in `.Codex/rules/architecture.md`.

## Documentation Index

| File | Purpose | Loaded |
|------|---------|--------|
| `.Codex/rules/architecture.md` | Tech stack, invariants, patterns | Always |
| `.Codex/rules/gotchas.md` | Known issues & workarounds | Always |
| `.Codex/rules/frontend.md` | React/UI patterns | Path: `src/**` |
| `.Codex/rules/backend.md` | Rust/Tauri patterns | Path: `src-tauri/**` |
| `specs/README.md` | Technical spec lookup table | On demand |
| `docs/README.md` | Full documentation index | On demand |
| `CHANGELOG.md` | Version history | On demand |

## Key Directories

| Path | Purpose |
|------|---------|
| `src/components/` | React components (layout/, mail/, accounts/, shared/) |
| `src/stores/` | Zustand state stores |
| `src/lib/tauri.ts` | Typed IPC wrappers |
| `src-tauri/src/commands/` | Tauri IPC command handlers (28 modules, 161 commands) — app package |
| `src-tauri/crates/cxmail-core/` | AppError, LockExt, secrets, keychain/, EventSink/AppCtx, MCP bridge, and the pure mail helpers `db` and `email` share |
| `src-tauri/crates/cxmail-db/` | SQLite schema, CRUD, FTS5 search (28 modules) |
| `src-tauri/crates/cxmail-email/` | IMAP, SMTP, OAuth2, parser, autoconfig, AI, categorize (29 modules) |
| `src-tauri/crates/cxmail-mcp/` | MCP server (`mcp/server.rs`) |

> `src-tauri/` is a Cargo **workspace**: the app package at `src-tauri/src/` plus four
> lib-only crates under `crates/`, depending downward as `core <- db <- email <- mcp <- app`.
> All four `[[bin]]` targets stay in the app package — moving one out un-bundles it and
> silently breaks the shared Keychain ACL. Test with `cargo test --workspace`.

## App Data

| Path | Contents |
|------|----------|
| `~/Library/Application Support/com.cxmail.app/cxmail.db` | SQLite database |
| `~/Library/Application Support/com.cxmail.app/credentials.dat` | Encrypted credentials (dev) |
| `~/Library/Application Support/com.cxmail.app/search_index/` | Tantivy search index |

## Conventions

- Package manager: npm
- Rich text editor: TipTap (compose modal)
- CSS utility: `cn()` from clsx + tailwind-merge
- Rust errors: thiserror (typed) + anyhow (ad-hoc)
- IPC: `#[tauri::command] async fn -> Result<T, AppError>`
- DB migrations: `db/schema.rs` (currently at v21)

## Environment

- Rust 1.94.1 (rustup), Tauri CLI 2.10.1
- Node.js v24.11.1, npm 11.6.2

## Recent Learnings

- 2026-03-31: Fixed critical sync hang — ammonia panics on `style` tag, IMAP dead sockets, DB mutex contention (DB v21)
- 2026-03-31: All `parse_message` calls need `catch_unwind` — certain HTML emails crash html5ever/ammonia
- 2026-03-31: Account groups, unsubscribe tracking, body prefetching, IMAP timeouts added
- 2026-03-29: Unsubscribe, compose attachments, email templates, and calendar integration (DB v7)
- 2026-03-29: Follow-up reminders wired into compose modal with desktop notifications
- 2026-03-29: Email categories, follow-up reminders, and AI writing menu added
- 2026-03-29: AI summarization, email tracking, snooze, undo send, scheduled view
- 2026-03-29: Created compound documentation infrastructure (rules, agents, changelog)
