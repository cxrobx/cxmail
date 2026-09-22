# Implementation Plan

## Phase 1: Foundation (Single Gmail Account)

- [x] Scaffold the Tauri 2 + React 19 + TypeScript project. Set up Cargo.toml with all Rust crates from specs/architecture.md. Set up package.json with all frontend deps. Configure Vite, Tailwind CSS 4, tsconfig with @ path alias. Verify `npm run tauri dev` launches an empty window.
- [x] Create SQLite schema and migration system in src-tauri/src/db/schema.rs. Create all tables from specs/database.md (accounts, folders, messages, message_bodies, attachments, sync_state, schema_version). Create indexes. Wire into Tauri setup hook so DB initializes on launch. Verify DB creates at ~/Library/Application Support/com.cxmail.app/cxmail.db.
- [x] Implement unified error types in src-tauri/src/error.rs using thiserror. Create AppError enum covering: database, imap, oauth, keychain, parse errors. Implement From conversions and Serialize for IPC.
- [x] Implement AppState in src-tauri/src/state.rs. Wrap rusqlite Connection in Mutex. Register as Tauri managed state.
- [x] Implement macOS Keychain wrapper in src-tauri/src/keychain/macos.rs using keyring crate (apple-native feature). Functions: store_credential, get_credential, delete_credential. Handle not-found and access-denied errors. Test with `security find-generic-password -s "cxmail"`.
- [x] Implement OAuth2 PKCE flow in src-tauri/src/email/oauth2.rs following specs/oauth2.md. Generate PKCE code_verifier + code_challenge. Start loopback HTTP server on random port. Build Google auth URL. Exchange code for tokens. Store tokens in Keychain. Implement token refresh.
- [x] Implement auth IPC commands in src-tauri/src/commands/auth.rs. start_oauth2 opens browser via tauri-plugin-shell. handle_oauth2_callback exchanges code and stores account. Wire commands into Tauri builder.
- [x] Implement IMAP client wrapper in src-tauri/src/email/imap.rs following specs/imap.md. Connect to imap.gmail.com:993 with TLS. Authenticate with XOAUTH2 SASL. Implement folder listing (LIST "" "*"). Map Gmail special folders to folder_type. Implement clean disconnect (LOGOUT).
- [x] Implement message header fetching in the IMAP client. FETCH ENVELOPE + FLAGS + INTERNALDATE + RFC822.SIZE + BODY.PEEK[HEADER.FIELDS (MESSAGE-ID IN-REPLY-TO REFERENCES)]. Extract all fields. Paginate by UID range.
- [x] Implement message body fetching in the IMAP client. FETCH BODY[] for full RFC822 message. Pass to parser module.
- [x] Implement MIME parser in src-tauri/src/email/parser.rs following specs/parser.md. Use mail-parser crate. Extract all ParsedMessage fields. Handle multipart/alternative, multipart/mixed, multipart/related. Extract attachment metadata.
- [x] Implement HTML sanitization in parser.rs using ammonia following specs/security.md. Configure allowlists for tags, attributes, CSS properties. Strip scripts/iframes/event handlers. Rewrite links with target="_blank" rel="noopener". Replace remote image src with data-original-src placeholder.
- [x] Implement database CRUD in src-tauri/src/db/ — accounts.rs (insert, list, delete), folders.rs (upsert, list by account), messages.rs (insert batch, list paginated by folder, get by uid, update flags). All queries parameterized.
- [x] Implement folder IPC commands in src-tauri/src/commands/folders.rs (list_folders, sync_folders) and message IPC commands in src-tauri/src/commands/messages.rs (fetch_messages, fetch_message_body, sync_folder, mark_as_read, mark_as_unread). Wire all commands into Tauri builder.
- [x] Create typed TypeScript IPC wrappers in src/lib/tauri.ts and TypeScript types in src/types/email.ts matching specs/ipc.md exactly. Create src/lib/utils.ts with cn() helper.
- [x] Build AccountSetup.tsx component. Provider selection (Gmail button, disabled iCloud/Outlook buttons for Phase 2). "Connect with Google" triggers OAuth2 flow. Loading spinner while waiting for callback. Success confirmation. Error display with retry.
- [x] Build Zustand stores: accountStore.ts, mailStore.ts, uiStore.ts following specs/frontend.md. Persist uiStore to localStorage.
- [x] Build AppLayout.tsx — three-pane layout with resizable panels. Custom draggable title bar with data-tauri-drag-region. Sidebar on left, message list and reading pane on right (split vertically). Show AccountSetup if no accounts configured, otherwise show mail layout.
- [x] Build Sidebar.tsx + FolderTree.tsx + AccountBadge.tsx. Account badge shows avatar (initials) and email. FolderTree renders folder hierarchy recursively with unread count badges. Click folder updates mailStore.selectedFolder. Wire to useFolders hook.
- [x] Build MessageList.tsx + MessageListItem.tsx with @tanstack/react-virtual for virtual scrolling. Each row shows from, subject, date (relative), snippet. Bold for unread. Blue dot indicator. Paperclip for attachments. Star for flagged. Pagination on scroll. Wire to useMessages hook.
- [x] Build ReadingPane.tsx + EmailFrame.tsx. ReadingPane shows message header (from, to, cc, date, subject). EmailFrame renders sanitized HTML in sandboxed iframe (sandbox="allow-same-origin", srcdoc). Intercept link clicks and open in system browser. Show "Images blocked" banner. Auto-resize iframe to content height.
- [x] Implement incremental sync following specs/sync.md. On app launch, sync INBOX. Check UIDVALIDITY. Fetch new messages since last_uid. Update sync_state. Background timer every 5 minutes. Emit sync-progress and sync-complete events.
- [x] Build StatusBar.tsx. Shows connection status (green/red dot), last synced time, sync progress bar during active sync. Listen to Tauri events via useTauriEvent hook.
- [x] Implement mark-as-read. When user selects a message, debounce 1 second, then call mark_as_read IPC. Updates both local SQLite and IMAP server flags (STORE +FLAGS \Seen). Update mailStore and folder unread count.
- [x] Build shared components: EmptyState.tsx (centered icon + message), LoadingSpinner.tsx, ErrorBoundary.tsx. Apply to all views.

## Phase 2: Core Email Client

- [x] Add iCloud account support (app-specific password, IMAP LOGIN auth, separate setup flow)
- [x] Add Outlook account support (Microsoft OAuth2, XOAUTH2 SASL, separate OAuth2 config)
- [x] Multi-account UI: account switcher in sidebar, per-account folder trees, account color coding
- [x] Unified inbox toggle: virtual folder combining all accounts' inboxes
- [x] Compose modal/pane with rich text editor (tiptap or Plate). To/Cc/Bcc fields with autocomplete from local cache.
- [x] Send email via SMTP using mail-send crate. Gmail: XOAUTH2 auth. iCloud: LOGIN auth.
- [x] Reply, forward, reply-all with quoted original content
- [x] Save drafts to IMAP Drafts folder
- [x] Conversation threading using JWZ algorithm on In-Reply-To/References headers
- [x] Full-text search with tantivy index. Index subject, from, body on sync. Search UI with query syntax.
- [x] Attachment download via Tauri dialog (save panel). Inline image rendering (decode from MIME, data URL).
- [x] Attachment compose: drag-and-drop + file picker. Encode as MIME multipart.
- [x] Folder operations: move (IMAP MOVE), archive, delete (move to Trash), star/unstar. Batch operations on multi-select.
- [x] macOS native notifications via tauri-plugin-notification. Badge count on dock icon.

## Phase 3: Power Features

- [x] Command palette (Cmd+K). Commands: navigate folders, search, compose, settings, switch account. Fuzzy matching.
- [x] Customizable keyboard shortcuts. Defaults: j/k navigate, e archive, # delete, r reply, a reply-all, f forward, / search, c compose. Stored in JSON config.
- [x] Mail rules engine: conditions (from, to, subject contains, body contains) + actions (move, tag, mark read, auto-reply). Rules stored in SQLite. Evaluated on sync.
- [x] Multiple identities/aliases. Send-as support. Per-account HTML signatures.
- [x] Density modes: comfortable, compact, ultra-compact. CSS variable adjustments.
- [x] Theme system: CSS custom properties, dark/light toggle, user-loadable themes from ~/.config/cxmail/themes/

## Phase 4: MCP Server

- [x] Embed MCP server using rmcp crate. stdio + SSE transports. Register in Tauri app lifecycle.
- [x] Implement open-tier tools: search_emails, read_email, read_thread, list_folders, list_accounts
- [x] Implement confirm-tier tools: compose_draft, reply_draft, move_email, archive_email, flag_email
- [x] Implement approve-tier tools: send_email, delete_email with UI approval modal
- [x] Build MCP approval modal component. Shows action details. Approve/Edit/Deny buttons. 60s timeout to reject.
- [x] Test with Claude CLI: add to ~/.claude/mcp_servers.json and verify all tools work

## Phase 5: Plugin System

- [x] Embed rquickjs (QuickJS) runtime. Sandboxed isolates. No default filesystem/network access.
- [x] Plugin API: inject cxmail global object matching specs/plugins.md
- [x] Plugin manifest loading from ~/.config/cxmail/plugins/
- [x] Plugin permission system: declared in manifest, approved on install
- [x] Trigger system: on_new_email, on_send, on_folder_change, on_startup, on_schedule
- [x] Example plugin: auto-tagger

## Phase 6: Polish

- [x] JMAP support via jmap-client crate (alternative transport behind same API)
- [ ] ~~PGP encryption via sequoia-openpgp~~ — **removed 2026-08-05.** This was
  never as complete as the line claimed: there was no key-management UI and no
  sign/verify, only generate/encrypt/decrypt/import behind four IPC commands
  with zero frontend callers. `sequoia-openpgp` is LGPL-2.0-or-later, and
  static linking into a proprietary binary carries a relink obligation
  (LGPL §6) — the only copyleft dependency in the tree, held for dead code.
  Restore from git history if PGP is ever wanted for real; a dynamically
  linked or separate-process design would keep the licence clean.
- [x] Thunderbird import: parse mbox files, map to internal format
- [x] Auto-updater: Tauri built-in updater with code signing, GitHub Releases feed
- [x] Local Bayesian spam filter trained on user's corpus
