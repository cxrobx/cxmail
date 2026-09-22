# Architecture

## Tech Stack

| Layer | Technology |
|-------|-----------|
| App framework | Tauri 2 (Rust backend + system webview) |
| Backend language | Rust — a Cargo **workspace**, five crates |
| Frontend | React 19 + TypeScript + Vite |
| Styling | Tailwind CSS 4 |
| UI primitives | Radix UI |
| Icons | Lucide React |
| Animations | Framer Motion |
| Editor | TipTap (compose) |
| State management | Zustand |
| Local database | SQLite via rusqlite (bundled) |
| Full-text search | SQLite FTS5 (`messages_fts`, trigger-maintained) |
| Credentials | macOS Keychain via keyring crate (file store in dev) |
| IMAP | async-imap over native-tls |
| SMTP | mail-send over rustls |
| MIME parsing | mail-parser |
| HTML sanitization | ammonia |
| OAuth2 | oauth2 crate + reqwest |
| MCP server | rmcp (stdio transport) |
| Plugin runtime | rquickjs (QuickJS) |
| Tracking service | axum + tower-http |

## The workspace

`src-tauri/` is a Cargo workspace whose **root is the `cxmail` app package**.
The four crates under `crates/` are lib-only.

```
src-tauri/
├── Cargo.toml              # [workspace] members = ["crates/*"] + [package] cxmail
│                           # [workspace.dependencies] pins rusqlite/serde/chrono/... ONCE
├── build.rs                # tauri_build::build() only
├── tauri.conf.json         # resolved relative to the app package — do not move it
├── capabilities/ icons/ binaries/ Entitlements.plist
├── examples/               # discover, …
├── crates/
│   ├── cxmail-core/        # AppError · LockExt · secrets · keychain/ · EventSink/AppCtx
│   │                       #   · bridge (MCP socket) · mail/ (pure helpers)
│   ├── cxmail-db/          # db/ — 28 modules, schema migrations (v55), FTS5
│   │   └── examples/migrate_probe.rs
│   ├── cxmail-email/       # email/ — 29 modules: IMAP, SMTP, OAuth2, parser, sync,
│   │   ├── build.rs        #   autoconfig, AI, calendar
│   │   └── resources/ispdb/
│   └── cxmail-mcp/         # mcp/server.rs
└── src/                    # the cxmail app package
    ├── main.rs lib.rs
    ├── commands/           # 28 modules, 161 #[tauri::command] handlers
    ├── bin/                # mcp.rs · tracker.rs · helper.rs  (+ main.rs = cxmail)
    ├── idle.rs             # IMAP IDLE watchers
    ├── notify.rs notify_macos.rs spellcheck_macos.rs launchd.rs
    ├── plugins/            # QuickJS runtime
    └── tracker/            # axum open-tracking service
```

### Dependency direction

```
cxmail-core  <-  cxmail-db  <-  cxmail-email  <-  cxmail-mcp  <-  cxmail (app)
```

Strictly downward, and that took work: `db/` and `email/` used to import each
other (6 files one way, 13 the other). Every piece `db` wanted from `email` was
pure — threading keys, snippet cleanup, event detection, the Google Calendar
DTOs, date normalization, the `needs_you` verdict — so those sank into
`cxmail_core::mail` and the edge disappeared. `email` still depends on `db`;
`db` no longer depends on `email`.

**If `db` seems to need something from `email` again, the something is pure and
belongs in core.** Adding `cxmail-email` to `cxmail-db`'s dependencies
reintroduces the cycle the split exists to remove.

### What each crate is for

| Crate | Why it is separate |
|---|---|
| `cxmail-core` | The leaf everything shares. `AppError` alone is referenced from ~83 files; left in the app package it would re-couple every other crate to Tauri. `bridge` lives here rather than in `cxmail-mcp` because the *app* consumes it too — `lib.rs` listens for pokes from the standalone MCP process. |
| `cxmail-db` | Was already Tauri-free; moving it means a SQL migration can be tested without compiling the QuickJS C library. |
| `cxmail-email` | The 20K-line engine. Its only Tauri coupling was `oauth2` and `gcal_invite` wanting to emit and to reach `state.db`; both now take a `cxmail_core::AppCtx`. |
| `cxmail-mcp` | 8.4K lines in one file. Its own compilation unit now, so editing `commands/` no longer invalidates it. |
| `cxmail` (app) | Everything that genuinely needs Tauri: the 161 IPC commands, the tray/menu/window setup, macOS notifications, and all four binaries. |

### Two things that must not move

**All four `[[bin]]` targets stay in the app package.** `tauri.conf.json`
declares exactly one external binary (`binaries/cxmail-helper`), so
`cxmail-mcp` and `cxmail-tracker` reach `cxmail.app/Contents/MacOS/` only
because Tauri's bundler enumerates the app package's bin targets. A binary that
stops being bundled is one `keychain::macos_acl` **silently skips** when
building the trusted-binary list for shared Keychain items — no error, no log,
just a credential prompt on every read from then on. Keeping them put also means
every `--bin cxmail-*` invocation in the repo works with no `-p`, `ship.md` step
2b included.

**`target/release/cxmail-mcp` keeps its exact path.** It is the live `cxmail`
MCP server registered in `~/.claude.json`, and a broken stdio MCP fails
silently. Workspace members share the root `target/`, so as long as the
workspace root stays at `src-tauri/`, the path holds.

### Consequences, honestly

`cargo test -p cxmail-db` compiles 83 dependencies; the app compiles 751. None
of tauri, its 8 plugins, rquickjs, rmcp, axum, image or objc2 are in the first
number. 594 of the 623 backend tests now live in crates that do not compile
Tauri at all.

What did *not* get faster: `cargo build --release --bin cxmail-mcp`. A bin
compiles its own package's lib, the bins stay in the app package, and the app's
lib is the Tauri one. `tracker/Dockerfile` still installs GTK and WebKit to
build a 348-line axum server. Fixing that needs the `externalBin` sidecar route
and is a separate refactor.

### Working in the workspace

- `cargo test --workspace` — plain `cargo test` at the root tests only the app
  package and stays green while covering none of the extracted crates. CI passes
  `--workspace` for this reason.
- Add shared dependency versions to `[workspace.dependencies]` and reference
  them with `x.workspace = true`. A second `rusqlite` in the graph makes
  `Connection` a distinct type per crate, and the mismatch reads as a baffling
  trait error rather than a version conflict.
- `lib.rs` re-exports every moved module at its historical path (`pub use
  cxmail_db::db;`, `pub use cxmail_email::email;`, …) so `cxmail_lib::db::…`
  still resolves from the bins and examples. Extend the shim rather than
  re-pathing call sites.

## Frontend

```
src/
├── main.tsx App.tsx
├── components/     layout/ · mail/ · accounts/ · shared/
├── stores/         accountStore · mailStore · uiStore · windowStore  (Zustand)
├── hooks/          useKeyboardShortcuts · useSwipeAction · useUnsubscribe
├── lib/            tauri.ts (typed invoke wrappers) · mailPresets.ts (mirrors the
│                   Rust preset table; a Rust test fails on drift) · draft*/search*/
│                   snooze*/schedule* helpers · syncHealth.ts · utils.ts
├── types/          email.ts · ipc.ts
├── styles/         globals.css
└── mocks/ test/
```

## IPC and events

- **Commands**: `#[tauri::command] async fn name(...) -> Result<T, AppError>` in
  `commands/`, invoked through typed wrappers in `src/lib/tauri.ts`. `AppError`
  has a manual `impl Serialize` that collapses to the message only — internals
  are deliberately not exposed to the frontend.
- **Events**: the backend emits over `tauri::Emitter`. Code in `cxmail-email`
  cannot, so it takes a `cxmail_core::EventSink`; the app implements it as
  `TauriEvents`, and binaries with no window pass `NoopSink`. Events in use
  include `account-added`, `oauth-error`, `calendar-connected`,
  `calendar-oauth-error`, `calendar-invites-sent`, `idle-status`,
  `idle-new-mail`, `send-completed`, `send-cancelled`.

## Data flow

```
User clicks folder → mailStore.selectedFolder
  → invoke('fetch_messages', …)
    → commands/messages.rs (app)
      → cxmail-db: cached rows, FTS5 where searching
      → cxmail-email: IMAP fetch when stale, mail-parser, ammonia
      → cxmail-db: persist
    → MessagePage back to the frontend
  → MessageList renders virtualized

User clicks message → mailStore.selectedMessage
  → invoke('fetch_message_body', …) → sanitized HTML
  → EmailFrame renders it in a sandboxed iframe
  → invoke('mark_as_read', …) fires debounced
```

Sync also runs without the user: `idle.rs` holds IMAP IDLE per account and
emits `idle-new-mail`; `cxmail-helper` runs the same watch as a LaunchAgent
while the app is closed.
