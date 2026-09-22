# Architecture Patterns

## Tech Stack

| Layer | Technology |
|-------|------------|
| Desktop | Tauri 2 (macOS-private-api) |
| Frontend | React 19, TypeScript, Vite |
| Backend | Rust, Tokio async runtime |
| Database | SQLite (rusqlite, bundled) |
| Search | SQLite FTS5 (`messages_fts`, trigger-maintained) |
| UI | Radix UI + Tailwind CSS 4 + Lucide React + Framer Motion |
| Editor | TipTap (compose modal) |
| State | Zustand (accountStore, mailStore, uiStore) |
| IMAP | async-imap with native-tls (implicit TLS only — see gotcha #41) |
| SMTP | mail-send (rustls) with provider-aware auth, implicit TLS or STARTTLS |
| Auth | OAuth2 PKCE (Gmail, Outlook), app passwords (iCloud) |
| Credentials | file_store (dev), macOS Keychain (production) |
| MCP | rmcp with stdio transport (`cxmail-mcp` crate; the *binary* stays in the app package) |
| Plugins | rquickjs (QuickJS) sandboxed runtime |
| Security | ammonia (HTML sanitization) |

## Directory Structure

```
cxmail/
├── src/                    # React frontend
│   ├── components/         # layout/, mail/, accounts/, shared/
│   ├── stores/             # Zustand: accountStore, mailStore, uiStore
│   ├── hooks/              # useKeyboardShortcuts, IPC wrappers
│   ├── lib/                # tauri.ts (typed IPC), utils.ts
│   └── types/              # TypeScript definitions
├── src-tauri/              # Rust backend — a CARGO WORKSPACE, root = the app package
│   ├── crates/             # lib-only members, no bin targets in any of them
│   │   ├── cxmail-core/    # AppError · LockExt · secrets · keychain/ · EventSink/AppCtx
│   │   │                   #   · mcp bridge · mail/ (the pure helpers db and email share)
│   │   ├── cxmail-db/      # db/ — schema migrations (v59), message store, FTS5
│   │   ├── cxmail-email/   # email/ — IMAP, SMTP, OAuth2, parser, autoconfig, AI,
│   │   │                   #   + its own build.rs (embed_ispdb) and resources/ispdb/
│   │   └── cxmail-mcp/     # mcp/server.rs — the MCP tool surface
│   └── src/                # the `cxmail` app package
│       ├── commands/       # Tauri IPC handlers (28 modules, 161 commands)
│       ├── bin/            # ALL FOUR binaries live here — see below
│       ├── idle.rs         # IMAP IDLE watchers (stayed for its notify coupling)
│       ├── notify*.rs      # macOS notifications (objc2)
│       ├── plugins/        # QuickJS plugin runtime
│       └── tracker/        # Email tracking service (Axum)
├── specs/                  # Technical specifications (13 files)
└── docs/                   # Reference documentation
```

**Dependency direction is strictly downward: `core <- db <- email <- mcp <- app`.**
`db` and `email` used to import each other; the shared pieces were sunk into
`cxmail-core::mail` to break that. Do not add a `cxmail-email` dependency to
`cxmail-db` — if `db` needs something from `email`, the something is pure and
belongs in core.

Four crates are provably Tauri-free (`cargo tree -p <crate> | grep -c tauri`
is 0 for core, db, email and mcp). That is what makes `cargo test -p cxmail-db`
compile 83 dependencies instead of 751, and it is worth protecting: a `tauri`
dependency added to any member undoes it silently.

**All four `[[bin]]` targets stay in the app package** (`cxmail`, `cxmail-mcp`,
`cxmail-tracker`, `cxmail-helper`), and this is load-bearing, not inertia.
Tauri's bundler enumerates the app package's bin targets — only
`binaries/cxmail-helper` is declared in `tauri.conf.json`, so `cxmail-mcp` and
`cxmail-tracker` reach `cxmail.app/Contents/MacOS/` by that route alone. A
binary that stops being bundled is one `keychain::macos_acl` **silently skips**
when it builds the trusted-binary list for shared Keychain items (`if
p.exists()`), and the result is a credential prompt on every read, forever
(gotcha #31). It also keeps every `--bin cxmail-*` invocation in the repo —
`ship.md` step 2b included — working with no `-p`.

Consequence, accepted deliberately: `cargo build --release --bin cxmail-mcp`
is **not** faster than before. A bin compiles its own package's lib, and the
app's lib is the Tauri one. The win is in test and rebuild scope, not release
link time. Making the aux binaries Tauri-free needs the `externalBin` sidecar
route and is a separate refactor.

## Critical Invariants (DO NOT BREAK)

1. **No credentials in SQLite** — Tokens/passwords go in encrypted file store (dev) or macOS Keychain (production). Never store secrets in the database.
   - Pattern: `src-tauri/crates/cxmail-core/src/keychain/`

2. **HTML always sanitized** — ammonia processes all HTML before storage and display. No raw HTML rendering.
   - Pattern: `src-tauri/crates/cxmail-email/src/email/parser.rs`

3. **Email rendered in sandbox** — iframe with `sandbox="allow-same-origin"`, no scripts allowed.
   - Pattern: `src/components/mail/EmailFrame.tsx`

4. **Remote images blocked** — Default off, per-sender allowlist. Prevents tracking pixels from third parties.

5. **TLS enforced** — no plaintext fallback, and `TlsMode` has no `Plain` variant to add one.
   **Two different TLS stacks, deliberately noted:** IMAP is native-tls (macOS keychain trust),
   SMTP is `mail-send` on **rustls + webpki-roots**. So a host whose certificate chains to a
   privately-installed CA connects over IMAP and fails over SMTP — which is why
   `add_imap_account` validates both. On the SMTP side `connect()` (never `connect_plain()`)
   returns `MissingStartTls` when a 587 server omits STARTTLS, so the invariant holds for free.

6. **IPC validates all input** — Rust commands never trust frontend data. All input validated server-side.
   - Pattern: `src-tauri/src/commands/*.rs`

7. **Links open externally** — Email links open in system browser via Tauri shell plugin, never in-app.

8. **RFC 2047 decoded** — MIME encoded-words in subjects/names are decoded before storage.
   - Pattern: `src-tauri/crates/cxmail-email/src/email/parser.rs`

## IPC Pattern

All frontend-to-backend communication uses Tauri's typed invoke:

- Rust side: `#[tauri::command] async fn name(...) -> Result<T, AppError>`
- Frontend side: Typed wrappers in `src/lib/tauri.ts` calling `invoke<T>("command_name", { args })`
- Errors: `thiserror` for typed errors, `anyhow` for ad-hoc; both serialize to `AppError` for IPC

## State Management

- **Frontend**: Zustand stores (not Redux). `uiStore` persisted to localStorage.
- **Backend**: `AppState` managed via Tauri, holds the DB handle, pending sends, plugin registry.
  `AppState.db` is an `Arc<Mutex<Connection>>` so a detached task can hold it without an
  `AppHandle` — see `cxmail_core::AppCtx`, which is what `email::oauth2` and
  `email::gcal_invite` take in place of one.
- **CSS utility**: `cn()` from clsx + tailwind-merge in `src/lib/utils.ts`

## Database

- SQLite via `rusqlite` (bundled), migrations in `cxmail-db`'s `db/schema.rs` (currently at v59).
  Both the app (`lib.rs`) and the MCP binary (`bin/mcp.rs`) run `initialize()` independently at startup
- App data at `~/Library/Application Support/com.cxmail.app/cxmail.db`
- Full-text search: FTS5 table `messages_fts` inside the same DB, kept in sync by triggers (v40). Single engine `db::search::search()` serves UI commands and the MCP binary; never write to `messages_fts` directly (see gotcha #26)

## Account Providers

| Provider | Auth | IMAP Host | Credential Key |
|----------|------|-----------|----------------|
| Gmail | OAuth2 XOAUTH2 | imap.gmail.com:993 | `gmail:{email}:access/refresh/expires` |
| iCloud | App password LOGIN | imap.mail.me.com:993 | `icloud:{email}:password` |
| Outlook | OAuth2 XOAUTH2 | outlook.office365.com:993 | `outlook:{email}:access/refresh/expires` |
| Generic (`imap`) | Password LOGIN | from `accounts.imap_host/port` | `imap:{email}:password` |

The generic `imap` provider is the only one that reads host/port/security/username off the
`accounts` row — the other three keep literals, so a corrupted row cannot redirect an existing
Gmail account. Presets in `cxmail-email`'s `email/providers.rs` (mirrored in `src/lib/mailPresets.ts`; a Rust test
fails on drift), with autodiscovery in `email/autoconfig.rs` — see
[`docs/mail-providers.md`](../../docs/mail-providers.md). **Gmail is a preset on purpose:** an app-password connection is not OAuth, so it
consumes none of the Cloud project's 100-consent lifetime cap. IMAP STARTTLS is not implemented
(gotcha #41); every preset is implicit TLS.
