# CXMail Specifications

macOS-first email client. Rust (Tauri 2) backend, React 19 + TypeScript frontend.

## Lookup Table

| Spec | File | Search Hints |
|------|------|-------------|
| Architecture Overview | specs/architecture.md | tauri, rust, react, typescript, project structure, tech stack, crates, dependencies, directory layout, module |
| SQLite Schema | specs/database.md | sqlite, rusqlite, tables, messages, folders, accounts, sync_state, migrations, schema, database, cache, storage |
| IMAP Client | specs/imap.md | imap, async-imap, xoauth2, sasl, folders, fetch, uid, uidvalidity, sync, idle, mailbox, envelope, flags |
| OAuth2 Flow | specs/oauth2.md | oauth, pkce, google, gmail, token, refresh, keychain, loopback, callback, credentials, authentication, login |
| MIME Parsing | specs/parser.md | mime, mail-parser, html, sanitize, ammonia, multipart, attachment, content-type, body, headers, rfc2822 |
| Security Model | specs/security.md | ammonia, xss, script, iframe, sandbox, csp, remote images, tracking pixel, keychain, keyring, tls, rustls, credential, encryption |
| IPC Commands | specs/ipc.md | tauri command, invoke, frontend, backend, state, appstate, api, handler, ipc |
| Frontend Components | specs/frontend.md | react, component, zustand, store, layout, sidebar, message list, reading pane, radix, tailwind, lucide |
| MCP Server | specs/mcp.md | mcp, tool, search_emails, read_email, compose_draft, send_email, permission, approve, claude, ai, agent |
| Plugin System | specs/plugins.md | plugin, rquickjs, quickjs, manifest, runtime, extension, hook, scripting, javascript |
| Sync Engine | specs/sync.md | sync, incremental, uidnext, uidvalidity, background, offline, cache, polling, idle, push |
| MCP Send-Approval Bridge | specs/mcp-send-approval-bridge.md | mcp, send_email, approval, bridge, hook, recipient, standalone, approve tier, disabled by design, socket |
| Scheduled-Send Reliability | specs/scheduled-send-reliability.md | scheduled, send, schedule, launchd, helper, daemon, sleep, wake, datetime, claim, double-send, retry, gotcha 23 |

## Conventions

- **Package manager**: npm (not pnpm/yarn)
- **UI components**: Radix UI primitives + Tailwind CSS + Lucide React icons
- **State management**: Zustand (not Redux)
- **Animations**: Framer Motion
- **CSS utility**: cn() from clsx + tailwind-merge
- **Error handling (Rust)**: thiserror for typed errors, anyhow for ad-hoc
- **Async (Rust)**: tokio runtime (Tauri's default)
- **IPC pattern**: `#[tauri::command] async fn -> Result<T, AppError>`
- **DB**: rusqlite with bundled SQLite, migrations in schema.rs
- **Credentials**: keyring crate (apple-native feature), NEVER in SQLite or logs
- **TLS**: rustls only, no OpenSSL, no plaintext fallback
- **HTML email**: ammonia sanitization before storage, sandboxed iframe for display
- **Remote images**: blocked by default (tracking pixel protection)
- **Links in email**: always open in system browser, never in-app
