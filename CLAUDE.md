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
| Search | SQLite FTS5 (`messages_fts`, trigger-maintained, in cxmail.db) |
| State | Zustand (accountStore, mailStore, uiStore) |

## Golden Commands

```bash
bash scripts/dev-signed.sh                   # Dev mode — USE THIS, not `tauri dev`
source "$HOME/.cargo/env" && npx tauri dev   # ⚠️ ad-hoc binary: Keychain dialog every 30s (gotcha #31)
source "$HOME/.cargo/env" && npm run tauri build  # Production build (CUSTOMER — gates on license)
npm run dev                                   # Frontend only (Vite)
cd src-tauri && cargo check                   # Check Rust compiles
cd src-tauri && cargo test --workspace        # Run Rust tests (all crates)
```

**Important:** Always `source "$HOME/.cargo/env"` before cargo/tauri commands.

**Dev mode:** use `scripts/dev-signed.sh`. Plain `npx tauri dev` runs an ad-hoc-signed
`target/debug/cxmail`, which fails two of gotcha #31's three conditions — and since
`lib.rs` polls `is_gcal_connected` for every Gmail account every 30 seconds, that is a
Keychain dialog storm *Deny* cannot stop. The script signs the debug binary with the
Developer ID (`--identifier com.cxmail.app` — the filename default does NOT satisfy the
installed app's requirement), verifies it against that requirement, and refuses to launch
if it doesn't match. Every Rust rebuild drops the signature, which is why this is a script
and not a one-time step. It needs a Developer ID signing identity in your keychain
(`IDENTITY` in the script); without one, `npx tauri dev` works and prompts instead.

**Updater signing:** `createUpdaterArtifacts` is enabled, so any build without `TAURI_SIGNING_PRIVATE_KEY` fails at the end with *"A public key has been found, but no private key"*. The `.app` is still built and usable — only the `.sig`/`latest.json` are missing — so this is easy to ignore until a release silently ships unpublishable. The key's public half must stay byte-identical to `plugins.updater.pubkey`, which `scripts/verify-updater-signature.mjs` checks.

## Critical Invariants (DO NOT BREAK)

1. **No credentials in SQLite** — encrypted file store (dev) or macOS Keychain (production)
2. **HTML always sanitized** — ammonia processes all HTML before storage and display
3. **Email rendered in sandbox** — iframe with `sandbox="allow-same-origin"`, no scripts
4. **TLS enforced** — no plaintext fallback (IMAP native-tls, SMTP rustls — two stacks)
5. **IPC validates all input** — Rust commands never trust frontend data

Full list with patterns in `.claude/rules/architecture.md`.

## Documentation Index

| File | Purpose | Loaded |
|------|---------|--------|
| `.claude/rules/architecture.md` | Tech stack, invariants, patterns | Always |
| `.claude/rules/gotchas.md` | Known issues & workarounds | Always |
| `.claude/rules/frontend.md` | React/UI patterns | Path: `src/**` |
| `.claude/rules/backend.md` | Rust/Tauri patterns | Path: `src-tauri/**` |
| `docs/mail-providers.md` | Providers, generic IMAP, autodiscovery, folder classification | On demand |
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
| `~/Library/Application Support/com.cxmail.app/cxmail.db` | SQLite database (includes the FTS5 search index) |
| `~/Library/Application Support/com.cxmail.app/credentials.dat` | Encrypted credentials (dev) |

## Conventions

- Package manager: npm
- Rich text editor: TipTap (compose modal)
- CSS utility: `cn()` from clsx + tailwind-merge
- Rust errors: thiserror (typed) + anyhow (ad-hoc)
- IPC: `#[tauri::command] async fn -> Result<T, AppError>`
- DB migrations: `db/schema.rs` (currently at v59)

## Environment

- Rust 1.94.1 (rustup), Tauri CLI 2.10.1
- Node.js v24.11.1, npm 11.6.2
