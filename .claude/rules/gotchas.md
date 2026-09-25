# Known Gotchas

Organized by category. 65 items, condensed format. Original numbering preserved (gaps intentional).

> Pruned 2026-08-17: investigation narratives and measurements trimmed; the rules, invariants, and patterns are all still here. Full histories: git history of this file (pre-prune) and the Recent Learnings section of `CLAUDE.md`.

## Index

| # | Issue | Category |
|---|-------|----------|
| 1 | Cargo env must be sourced | Environment |
| 2 | XOAUTH2 SASL must be raw, not base64 | Backend |
| 3 | IMAP hangs without spawn_blocking | Backend |
| 4 | OAuth2 auth code must be URL-decoded | Backend |
| 5 | File-based credential store for dev | Environment |
| 6 | macOS Keychain unreliable for dev builds | Environment |
| 7 | RFC 2047 subjects can crash display | Backend |
| 8 | Separate click vs collapse on account items | Frontend |
| 9 | ammonia panics on `style` in tags + clean_content_tags | Backend |
| 10 | IMAP dead sockets hang without timeouts | Backend |
| 11 | DB Mutex blocks fetchBody during sync | Backend |
| 12 | Two Tauri dev instances share SQLite causing lock contention | Environment |
| 13 | Old MCP drafts collapse paragraph spacing; UI re-save doesn't normalize | Backend |
| 14 | Inline `cid:` images need parse-time resolution; re-caching a body must not clobber six metadata columns | Backend |
| 15 | Amazon (and others) leak literal `\"` JSON-escapes into Subject headers | Backend |
| 16 | Option-click "Copy Claude Prompt" — Radix Trigger swallows onContextMenu; use onOpenChange | Frontend |
| 17 | Radix `<Trigger asChild>` may not forward your `onContextMenu` to the wrapped child | Frontend |
| 18 | Vite HMR doesn't re-init module-level code; full reload (Cmd+R) needed when debugging | Frontend |
| 19 | Tauri production builds ship without DevTools — use `npm run tauri dev` for UI debugging | Environment |
| 20 | Tauri sync commands run on Tokio worker threads; AppKit calls need `run_on_main_thread` | Backend |
| 21 | Detected meeting events: join URL goes in `description`, NOT `location`; dtstart Z-contract; sync-only detection needs v38 backfill | Backend |
| 22 | Styled `layout="card"` email → Gmail folds the body behind "Show trimmed content" | Backend |
| 23 | ISO timestamp column compared lexicographically against `datetime('now')` fires late | Backend |
| 24 | "Open in Claude" spawns duplicate Ghostty windows — `open -na` new instance restores prior session | Backend |
| 25 | Drafts carry attachment METADATA only — editing a draft must reload the bytes or re-save silently drops files | Backend |
| 26 | Search is FTS5 kept in sync by DB triggers (v40) — invariants that keep it correct | Database |
| 27 | `spellcheck` attribute alone gives no squiggles in WKWebView — a user default is the real switch | Frontend |
| 28 | A plain release build locks the owner out behind the 1.0 license gate | Environment |
| 29 | The app CSP is inherited by the email `srcdoc` iframe — an inline script there is blocked, clipping every email to 200px | Frontend |
| 30 | `messages.message_id` is stored in THREE spellings — exact-match lookups silently miss and misreport why | Database |
| 31 | Keychain prompts: THREE conditions gate a prompt-free read, and the partition sweep only writes one of them | Environment |
| 32 | ReadingPane fetched the selected message from the WRONG account outside unified/group views — "UID not found" on a message that was cached | Frontend |
| 33 | Editable content in a ProseMirror node view needs an UNEDITABLE root — nested `contenteditable="true"` is one editing host, not two, and the first keystroke eats the node | Frontend |
| 34 | Omitting `layout` on an MCP draft flattens *designed* HTML at compose-open — and the 2026-07-30 bullets broke because they were table rows, not lists | Backend |
| 35 | Foreground handoff spawns a whole second Ghostty APP — AppleScript `new window with configuration` avoids it, and its `command` is shell-SPLIT | Backend |
| 36 | `MailRule` is wider than the engine that runs it — three ways to save a rule that does nothing, and one that eats the whole mailbox | Backend |
| 37 | Raw headers live in their OWN table because `message_bodies` doubles as the body-cache flag — and there is no backfill, by design | Database |
| 38 | Draining an async-imap response stream is NOT checking the response — a tagged NO/BAD never becomes an item | Backend |
| 39 | Every reply CXMail sent for three months had NO `In-Reply-To` — invisible because Gmail threads by subject when References is absent | Backend |
| 40 | Subject-fallback threading needs a reply-prefix gate, or 379 identical notifications become one thread | Database |
| 41 | Generic IMAP: STARTTLS is deferred because `BufReader::into_inner` eats buffered bytes, SPECIAL-USE can't be *requested*, and `LIMIT 1` with no `ORDER BY` picked folders at random | Backend |
| 42 | `vitest` reporting "21 errors, no tests" is the WRONG NODE, not a broken dependency — and every corroborating check agrees with the wrong answer | Environment |
| 43 | A derived voice profile can only DESCRIBE — "always" had nowhere to live, and a hedge faithful to a mixed corpus leaves the agent free to do the wrong thing | Database |
| 21b | The event detector missed an unlabeled date line, could not parse `*Thursday, …*` at all, and had a latent char-boundary PANIC | Backend |
| 44 | A Zoom link cannot be a `gcal_events` column — every Google sync would null it, and four deleters would orphan the meeting invisibly | Database |
| 45 | `u8 as char` is a LATIN-1 decode — it double-encoded every smart quote in the inbox preview, and the body next to it was fine | Backend |
| 46 | `believed_open` was never a socket count — it read 14 for an account holding 2 sockets, and it lied convincingly because it correlated with the symptom | Backend |
| 47 | A pinned rule that says "never" cannot be kept by a prompt — the advisory fires AFTER the write, and the derived profile argues the opposite in the same context window | Backend |
| 49 | `--add-dir` is VARIADIC — putting it before the prompt swallows it, and the session opens silent and empty | Backend |
| 48 | Bare text in a `<td>` has no styling hook — but the editor does NOT wrap it in a margin-carrying paragraph, and the fix must not restyle it either | Backend |
| 50 | Google records NO "was this invitation sent" fact — an event can look published, with guests attached, that nobody ever received | Database |
| 51 | Window dragging is an ATTRIBUTE plus an ACL GRANT — `-webkit-app-region` is a no-op and `core:window:default` does not grant `start-dragging` | Frontend |
| 52 | The glass window is TWO halves that fail invisibly apart — and one opaque root element defeats the whole thing while every number is correct | Frontend |
| 53 | `selectedAccountId` is not "an account is selected" — the auto-select effect fills it in every special view and inbox group | Frontend |
| 54 | "All accounts" was never a query over `accounts` — hiding one means a predicate in every aggregate, and `[]`→`undefined` collapse turns an empty folder into All Inboxes | Database |
| 55 | Opening a draft is an IMAP round trip with no identity — every click that lands during it opened another window, and a double-click is three of them | Frontend |
| 56 | Reopening a draft re-downloaded its attachments from IMAP every time — bytes we had in hand at save time — and awaited a LOGOUT that Google never answers | Backend |
| 57 | A stale draft UID silently FORKED the draft — `UID STORE` ignores nonexistent UIDs — and MCP `edit_draft` expunged the old revision before it appended the new one. Closed by v60: `drafts(draft_id → current_uid)` + a claim CAS before any APPEND | Backend |
| 58 | MCP draft writing never called a model — the writer IS the calling agent, so "swap in Gemini" is a new authorship path (`instruction` → `agy`), not an `inference.rs` provider | Backend |
| 59 | The blur is a CGS Gaussian we choose, not Apple's material — the window boots OPAQUE until first paint, three AppKit edges (0.01 alpha, `invalidateShadow`, radius 0 first), and Reduce Transparency is ours to honour now | Frontend |
| 60 | A per-area transparency cannot be a subtree `--alpha-*` override — `@theme` tokens resolve at `:root` — so the email body's dial is a solved veil, and its cap is the clamp at 0 | Frontend |
| 61 | A thread's members come from EVERY folder — so your own replies printed twice and an unsent draft printed as sent, and the count had the same blind spot | Database |
| 62 | A draft saved in Gmail's editor loses CXMail's `email-signature`/`cx-quote` classes — reopened in compose, the signature table flattened and a second signature landed below the quote | Frontend |
| 63 | Send-as: an address on your own Sent mail is PROOF, a delivery header is only a SUGGESTION, and a same-domain `To:` filter offers strangers as aliases | Database |
| 64 | The in-app chat is `claude -p` with CXMail as the permission host — three flags carry the security, and `--allowedTools` is ADDITIVE | Backend |

---

## Environment

### 1. Cargo env must be sourced
**Symptom**: `cargo`/`tauri` not found. **Solution**: `source "$HOME/.cargo/env"` first — first line of any build script.
**Non-obvious trigger**: any wrapper process that doesn't inherit a login shell — `secret run -k … -- npm run tauri build`, `env -i`, launchd, scrubbed CI. Dies on `failed to run 'cargo metadata' … No such file or directory (os error 2)`, which reads like a broken `secret`/`tauri` install, not PATH. Source the cargo env **outside** the wrapper.

### 5. File-based credential store for dev
Dev builds use an encrypted file store, not the Keychain: `~/Library/Application Support/com.cxmail.app/credentials.dat`. Pattern: `src-tauri/crates/cxmail-core/src/keychain/file_store.rs`.

### 6. macOS Keychain unreliable for dev builds
Unsigned dev builds can't reliably access the Keychain (prompts, denials, silent failures) — dev uses file_store; Keychain is production. Pattern: `src-tauri/crates/cxmail-core/src/keychain/mod.rs`.

---

## Backend

### 2. XOAUTH2 SASL must be raw, not base64
Pass `user=email\x01auth=Bearer token\x01\x01` raw — async-imap does the base64. Double-encoding = "invalid credentials". Pattern: `src-tauri/crates/cxmail-email/src/email/imap.rs`.

### 3. IMAP requires tokio-util compat layer ~~(was: spawn_blocking)~~
async-imap uses futures-io traits; bridge with `tokio_util::compat::TokioAsyncReadCompatExt` — manual greeting read + `async_imap::Client::new(buf_stream)`. Pattern: `src-tauri/crates/cxmail-email/src/email/imap.rs` connect functions.

### 4. OAuth2 auth code must be URL-decoded
The callback's auth code is URL-encoded; decode before token exchange. Pattern: `src-tauri/crates/cxmail-email/src/email/oauth2.rs`.

### 7. RFC 2047 subjects can crash display
Decode all MIME encoded-words (`=?UTF-8?B?...?=`) at parse time, before storing. Pattern: `src-tauri/crates/cxmail-email/src/email/parser.rs`.

---

## Frontend

### 8. Separate click vs collapse on account items
Click on account name selects; click on chevron toggles collapse — never one handler for both. Pattern: `src/components/layout/Sidebar.tsx`.

---

## Backend (continued)

### 9. ammonia panics on `style` in allowed tags
Don't add `<style>` to `tags` (it's in `clean_content_tags` — adding to both panics, and the panic on a sync worker looks like "Syncing…" stuck forever). Inline styles via `add_generic_attributes(&["style"])`. `sanitize_html()` and **every** `parse_message` call wrapped in `catch_unwind`. Pattern: `src-tauri/crates/cxmail-email/src/email/parser.rs`.

### 10. IMAP dead sockets need timeouts
TCP can stay "alive" while data stops (NAT timeout, LB reset) — without timeouts the app freezes. 15s on connect, 45s per-account in `sync_all_inboxes`, `catch_unwind` on parse. Pattern: `src-tauri/crates/cxmail-email/src/email/imap.rs`, `commands/messages.rs`.

### 11. DB Mutex must use small lock scopes
Sync holding the mutex for insert + classify + bodies + thread counts in one scope blocks `fetchBody` ("Loading message..." stuck). Separate scopes per phase. Pattern: `src-tauri/src/commands/messages.rs`.

### 12. Kill old Tauri dev before restarting
Two instances share the same SQLite file → write-lock contention, sync never completes. `pkill -f "target/debug/cxmail"` before restart; `busy_timeout(5s)` as net.

### 13. Old MCP drafts have collapsed paragraph spacing; UI re-save doesn't normalize
Pre-fix `body_to_html` wrapped the whole body in one `<p>` with `<br/>`s; the UI's `edit_draft` builds raw MIME from `OutgoingEmail` and never calls `body_to_html`, so the broken structure round-trips unchanged. Heal by regenerating the draft via `mcp__cxmail__compose_draft`/`edit_draft` with fresh plain text. Pattern: `src-tauri/crates/cxmail-mcp/src/mcp/server.rs::body_to_html`.

### 14. Inline `cid:` images: resolved at parse time; self-heal uses targeted UPDATE
`parse_message` rewrites `<img src="cid:...">` to `data:` URIs from `multipart/related` image parts (cid normalized per RFC 2392; 2 MB per image / 5 MB total, over-cap left as `cid:`). Cached messages self-heal once via the `cid_resolved` flag.

**Critical — the "don't use `insert_body` here" family.** `insert_body` is `INSERT OR REPLACE`: REPLACE deletes the row and re-inserts, nulling **six** columns (`summary`, `summary_model`, `to_json`, `cc_json`, `bcc_json`, `attachment_metadata_checked`).

| Writer | Function | Why |
|---|---|---|
| Frontend cache-HIT cid self-heal | `update_sanitized_html_after_cid_repair` | targeted 2-column UPDATE |
| MCP quoted-history write-back (#30) | `upsert_body_preserving_metadata` | `ON CONFLICT DO UPDATE` of body columns only; FTS-trigger-safe (#26); refuses to overwrite non-blank with blank |
| Fresh ingest (sync, import, local draft) | `insert_body` | correct — the row is genuinely new |

Pattern: `email/parser.rs::inline_cid_images`, `db/messages.rs`. No "show images" UI applies — `cid:` is not a network fetch.

### 15. Amazon (and others) leak literal `\"` JSON-escapes into Subject headers
Sender-side templating puts JSON-escaped text in Subject; mail-parser faithfully stores it (Subject is unstructured text). `normalize_unstructured_header` does a conservative unescape (`\"`→`"`, `\\`→`\`, other `\X` left intact), applied at both ingress points (`parse_message` and `parse_fetch_to_header`); v32 backfilled. **Scoped to `subject` only** — `from_name` is structured and already de-quoted. Pattern: `email/parser.rs`, `email/imap.rs`, `db/schema.rs::migrate_v32_unescape_subjects`.

---

## Frontend (continued)

### 16. Option-click "Copy Claude Prompt" — read Option in Radix's `onOpenChange`, NOT in your own `onContextMenu`
Radix `<ContextMenu.Trigger asChild>` doesn't reliably fire your `onContextMenu` (see #17), so reading modifier state there is dead code even though WKWebView delivers Option keydown fine. Fix: module-level `isOptionPressed` tracker (window keydown/keyup/blur — module-level because the component renders per virtualized row), snapshot it in `onOpenChange(open=true)`, live-update the label while open via a `useEffect`, and read the ref in `onSelect`. The live-update re-introduces a small release-Option-before-click race — accepted tradeoff.
**Lesson**: when the user says "this worked a week ago," trust that — prove which signal is dead with a `console.log` before redesigning. The 6-hour multi-signal rebuild was wrong; the actual bug was a 5-minute Radix prop-merge issue.
Pattern: `src/components/mail/EmailContextMenu.tsx`.

### 17. Radix `<Trigger asChild>` may not forward your `onContextMenu` to the wrapped child
Radix's prop-merging can short-circuit your handler for `contextmenu` in particular — the menu opens (Radix's handler ran) but yours never fires. Use `onOpenChange` on `<ContextMenu.Root>` for open/close signals; if you truly need the DOM event, drop `asChild`.

### 18. Vite HMR doesn't re-init module-level code; do a full reload when debugging
Fast Refresh re-runs components/hooks, NOT module top-level statements (`installX()` calls, `let moduleVar`). Edits there don't take effect until Cmd+R; old listeners may persist or duplicate. Verify new code is live with a distinctive `console.log` at module top.

### 19. Tauri production builds ship without DevTools — use `npm run tauri dev` for UI debugging
Tauri 2 only enables WebInspector in debug builds; prod strips it (`devtools` feature not set). Right-click → Inspect Element only exists in dev.

### 20. Tauri sync commands run on Tokio worker threads; AppKit calls need `run_on_main_thread`
Sync commands dispatch on the worker pool; main-thread-only AppKit calls (`+[NSEvent modifierFlags]`, `NSApplication.shared`) return garbage/zeroes or crash from there. Make the command async and route AppKit work via `app.run_on_main_thread` + a oneshot channel.

### 21. Detected meeting invites: join URL belongs in `description` (NOT `location`); dtstart Z-contract; detection is sync-only
Three couplings between `detect_events.rs` and the frontend:
1. `extractMeetingLink` scans `location → description → raw_ics`. `location` is **rendered** (raw URL = ugly MapPin line); `description` is **not displayed** but is scanned — it's what lights the "Join meeting" button. Put the join URL in `description`, never `location`.
2. dtstart is a tagged contract: recognized named US zone → true UTC `"%Y-%m-%dT%H:%M:%SZ"` (chrono-tz, per-date DST); unknown tz but parseable time → **floating** (no `Z`); no time → date-only all-day. Mixing forms shifts displayed times.
3. Heuristic detection runs **only at sync-ingest** (confidence ≥ 0.6, only when no `text/calendar` part) — cached mail needs a backfill migration (v38 did this, idempotent, preserves `dismissed`).
Two-signal gate: host-anchored provider URL AND (invite phrase OR labeled date/time); regexes mirror `meetingLink.ts` exactly. Pattern: `email/detect_events.rs`, `db/calendar.rs::insert_detected`, `src/lib/meetingLink.ts`.

---

### 21b. …and three more ways it silently got the date wrong (2026-08-10)
A real recruiting email (`*Thursday, August 13, 2026:*` on its own line, time on the next) was filed as all-day on the arrival date. Four defects:
1. **`find_labeled_datetime` only inspects lines that START with a label.** Fixed by `find_unlabeled_datetime`, pairing a date line with a time on the same/next non-empty line; labeled still wins.
2. **The date line couldn't parse at all**: `try_parse_month_day_year` required `words[0]` to BE the month. `strip_date_decoration` removes emphasis markers, trailing colon, leading weekday (boundary-checked).
3. **Read the tz from the line the TIME came from, never the document** — signatures carry `⏱️ PST` while the meeting is `ET`. `render_dtstart` takes the zone source as a parameter.
4. **`&s[..10]` guarded by `s.len() >= 10` is a PANIC** on multi-byte chars (a `──────` separator line). A length check is not a boundary check — use `s.get(..10)`, never a length-guarded slice. The #1 fix made this latent panic reachable; two siblings hardened at the same time (`parse_reference_date` runs for every synced message).
Plus a display bug: `new Date("2026-08-10")` parses as UTC midnight → renders the previous evening locally. Guard `dtstart.length === 10 → "All day"` in **both** card and calendar view.
**Migration is refresh-only** (`refresh_detected`, scoped `source='detected'`, preserves `dismissed`): `insert_detected` early-returns existing rows, so a detector fix can't reach stored mail otherwise — and a first draft that also *inserted* added 41 junk rows from historical GCal notification mail (v38's `NOT EXISTS` guard is what prevents heuristic-beside-ICS duplicates).
**Probe a detector migration against a copy of the real mailbox before shipping**: `sqlite3 live.db ".backup probe.db"`, stamp `schema_version` back, run `initialize`, diff. That caught both the panic and the junk rows; every unit test passed in both cases.
Pattern: `email/detect_events.rs`, `db/calendar.rs::refresh_detected`, `db/schema.rs::migrate_v53_recorrect_detected_events`, `CalendarEventCard.tsx::formatDateTime`.

---

### 22. Styled `layout="card"` emails make Gmail collapse the body behind "Show trimmed content"
The card chrome's full-width page background around a centered white card trips Gmail's trim heuristic — the message renders as an empty dark box behind "•••" (dark mode especially). The sent HTML is clean; the fold is Gmail's client-side view. **Don't use `card` for normal correspondence/replies** — plain body or `rich` without a page background. Reserve `card` for standalone designed email. Pattern: `mcp/server.rs::wrap_card`/`render_styled`.

### 23. ISO timestamp column compared lexicographically against `datetime('now')` fires late
Frontend/ICS store ISO-8601 (`2026-06-03T13:00:00.000Z`); `datetime('now')` returns space-format. On a TEXT column that's a byte compare — at index 10, `T` sorts after space, so anything due *today* (UTC) doesn't fire until UTC midnight. **Wrap both operands**: `datetime(col) <= datetime('now')`. **Do NOT change the stored format** (space-format would parse as local in JS and shift display). Wrapping defeats the covering indexes — acceptable, these tables are tiny.
**Testing trap**: past/future fixtures (2020/2999) don't catch this; the regression fixture must share today's UTC date but be in the past (`date('now') || 'T00:00:00.001Z'`).
Applied in `db/{scheduled,snoozed,followup,calendar}.rs::list_due`/range scans. Non-bug caveat: scheduled sending only runs while the app is open and the Mac awake (30s loop in `lib.rs`; the helper only does IDLE).

### 24. "Open in Claude" opens duplicate Ghostty windows — largely superseded by #35
AppleScript `new window with configuration` (see #35) is now the primary route; the `open -na` line is the fallback. The fallback keeps two hard-won facts: `-n` is required (an already-running instance ignores `open -a … --args -e`), and a new instance **restores the previous session's saved windows**, so `--window-save-state=never` must be passed per-launch (verified: 3 windows without, 1 with). Verification trick: count windows per *instance* via System Events on the new PID. Pattern: `commands/claude_handoff.rs`.

### 25. Drafts carry attachment METADATA only — editing a draft must reload the bytes, or re-save silently drops the files
`MessageDetail.attachments` is metadata only (no bytes); `save_draft`/`edit_draft` rebuild MIME purely from `OutgoingEmail.attachments`, so opening a draft without reloading bytes means the next save permanently drops its files. `openDraftForEdit` (both call sites) calls `fetch_outgoing_attachments` (IMAP-fetch + base64, **filters `is_inline`** so cid: body images aren't duplicated); on error, open without them — never block the modal. Pattern: `email/parser.rs::extract_all_attachments`, `commands/compose.rs::fetch_outgoing_attachments`, `ComposeModal.tsx` chips.

### 26. Search is FTS5 kept in sync by DB triggers (v40) — invariants that keep it correct
`messages_fts` (rowid = `messages.id`; porter stemming; prefix `'2 3'`), eight triggers, so every writer (app, helper, MCP) indexes automatically and raw `DELETE FROM messages` evicts in the same transaction. `db::search::verify_or_rebuild` at startup repairs drift and never panics.
**Invariants — breaking any silently desyncs search:**
1. **Never enable `PRAGMA recursive_triggers`** — `insert_body`'s `INSERT OR REPLACE` relies on the implicit delete NOT firing the delete trigger. Pinned by test.
2. **`messages` stays INSERT OR IGNORE** — REPLACE churns `messages.id` (the FTS rowid) and orphans rows.
3. **Never write `messages_fts` directly** — only `db::search::rebuild()`.
4. **User keywords go through `build_match_expr`** — it quotes every token; raw interpolation into MATCH = silent "No results".
5. **The UPDATE trigger is scoped** (`OF subject, from_name, from_email, to_list, cc_list`) so read/flag flips don't rewrite FTS rows; new indexed columns must be added to the `OF` list.
6. **Guard `messages_fts` access on table existence** (stale pre-v40 DB reachable via the MCP's non-fatal migration path — see `search_emails`' LIKE fallback).
Snippet contract: `search()` wraps matches in U+E000/U+E001; frontend splits to `<mark>` (plain text nodes only); MCP strips them. Pattern: `db/schema.rs::migrate_v40_fts5`, `db/search.rs`, `email/search_parse.rs`, `email/server_search.rs`.

### 27. The `spellcheck` attribute alone produces NO squiggles in WKWebView — the user default is the real switch
WKWebView's continuous spell check defaults OFF and the HTML attribute can't enable it. Both "obvious" native APIs are absent — verified at runtime: `setContinuousSpellCheckingEnabled:` (NSTextView API) and the `_`-prefixed WebKit SPI both fail `respondsToSelector:`; sending unguarded = NSException crash. Fix: write `WebContinuousSpellCheckingEnabled` + `WebGrammarCheckingEnabled` user defaults in `.setup()` (checker state initializes lazily on first focus, so setup-time is early enough; persists in the app domain). Keep the HTML attributes — they scope *which* fields get checked (atomic blocks are `contentEditable=false` → no squiggles in signatures/quotes, correctly). Pattern: `src-tauri/src/spellcheck_macos.rs`.

### 28. A plain release build locks the OWNER out behind the 1.0 license gate
All licensing is `#[cfg(not(debug_assertions))]` — invisible in dev, and a plain `npm run tauri build` swapped into `/Applications` greets you with "Activate CXMail". Owner builds: `CXMAIL_OWNER_LICENSE=owner npm run tauri build` (`owner_bypass_status()` reads `option_env!` and short-circuits before the Keychain read and remote validation). **Never** export it in shell profiles or customer CI.
**⚠️ `strings … | grep OWNER` returns 0 on a WORKING owner build** — since the size profile (`opt-level="s"` + thin LTO) the 5-byte literal is materialized as instruction immediates. Verify via differential build (env var must change the binary hash) or just launch it: inbox = bypass active.
A bare Keychain key insert isn't enough — `refresh_license_status()` re-validates remotely and a `{valid:false}` response *deletes* the stored key.
**Owner builds deliberately do NOT check for updates** — `checkForAppUpdate` early-returns on `maskedKey === "OWNER"`. Not a bug: a published build has no bypass, so accepting an update locks you out of your own mail. Guard fails open (a status-read error must never stop customer updates); pinned by `updater.test.ts` (`vi.stubEnv('PROD', true)` is load-bearing). It does not protect against replacing the app by hand. The better long-term fix: comp a real `CXM-` key (`npx tsx scripts/issue-cxmail-license.ts` in cxventures) and run the customer build.
Pattern: `commands/license.rs::owner_bypass_status`, `src/lib/updater.ts`, `LicenseGate.tsx`.

### 29. The app CSP is inherited by the email `srcdoc` iframe — an inline script there is silently blocked, clipping every email to 200px
A `srcdoc` iframe inherits the embedder's CSP (`script-src 'self'`, no `'unsafe-inline'`), so the inline frame script that posts `cxmail-frame-height` was refused — every body clipped at the initial 200px, plus dead links, no remote-image unblocking, no image context menu (one script owns all four). Fix: keep the script in `public/email-frame.js`, inject by `src=` — `'self'` covers it.
**Do NOT use a `'sha256-…'` hash allowlist**: WebKit ignores hash allowances for inline scripts in sandboxed srcdoc frames (Chromium honors them) — it would pass any Chromium check and ship broken to WKWebView. Verify CSP changes by driving real WebKit (Playwright).
Expensive-to-spot because the CSP shipped one build before it took effect, and release builds have no DevTools (#19). Regression guard: `EmailFrameCsp.test.ts` (mutation-tested). Pattern: `EmailFrame.tsx`, `public/email-frame.js`, `tauri.conf.json`.

---

## Database (continued)

### 30. `messages.message_id` is stored in THREE spellings — an exact-match lookup silently misses and then misreports why
The column holds bracketed `<id@host>` (IMAP sync), **bare** `id@host` (old `draft_local.rs`/`import.rs` ingress), and **HTML-escaped** `&lt;id@host&gt;` (agent-copied off a rendered surface); `reference_ids` has the same split plus garbage. Exact-match reply lookups degraded to a false "original body not cached" — and the agent's paste workaround puts the signature below the quote.
**Fix, three layers, all required:**
1. Read side bracket-insensitive: `WHERE message_id IN (?2, ?3)` binding both variants (`message_id_match_variants`). **Never `trim(message_id,'<>') = ?`** — a function on the column defeats `idx_messages_message_id` (~29k-row scan). Column not migrated; read-side normalization is safer.
2. Ingress normalizes `message_id`/`in_reply_to` and re-brackets `references` tokens.
3. Structural: `compose_draft`/`edit_draft` accept `reply_to_folder` + `reply_to_uid` — coordinates win over `reply_to_message_id`; half a pair is `invalid_params`.
**Fail loudly, and placement is load-bearing**: quote resolution runs strictly BEFORE the IMAP APPEND in both handlers. (`edit_draft` used to expunge the old draft *before* appending, so anything fallible in that window lost the draft outright; since 2026-08-25 it appends first — #57 — and the rule is now "nothing fallible after the APPEND", because a failure there leaves a duplicate.) Six typed skip reasons, each naming the remedy and banning the paste workaround. IMAP-fetched bodies write back via `upsert_body_preserving_metadata` (#14). All read tools emit normalized IDs plus `Folder:`/`UID:`.
**Do NOT touch `format_quoted_history`** — byte-contract with `buildQuotedEmailHtml`/`QuotedBlock`/`splitTrailingQuote`; test `quoted_history_matches_frontend_blockquote_contract` is the tripwire — if a refactor forces an edit there, stop. (Known cosmetic: `inline_styles.rs:67` gives our quote a second border bar; fix belongs there, not in `format_quoted_history`.)
Pattern: `email/message_id.rs`, `db/messages.rs`, `mcp/server.rs` (`resolve_reply_target`, `plan_quote`, …).

---

## Environment (continued)

### 31. Keychain prompts: THREE conditions gate a prompt-free read, and the partition sweep only writes one of them

**Symptom**: "CXMail / cxmail-helper wants to use your confidential information…" keeps returning; *Allow* doesn't stop it; survives a clean partition sweep and correct signing.

| # | Condition | Who writes it |
|---|---|---|
| 1 | Reader signed `TeamIdentifier=CCYV5HQZCM` | `codesign` (ship.md step 2b) |
| 2 | Item's **partition list** has `teamid:CCYV5HQZCM` | macOS, automatically, at creation by a signed process |
| 3 | Item's **ACL trusted-application list** names the reading binary | **only** `SecKeychainItemCreateFromContent`'s `initialAccess`, **at creation** |

**The prompt is gated by (3).** `SecKeychainAddGenericPassword` (what `keyring` calls) sets the ACL to the creating binary alone; no `security` subcommand can change it later (`set-generic-password-partition-list` writes (2), a different ACL entry), and post-creation ACL changes need `change_acl` authorization no binary holds — creation is the only prompt-free moment. A clean partition sweep is NOT a fix.
**Allow vs Always Allow**: *Allow* grants one read and writes nothing (prompt returns at next token refresh). *Always Allow* appends the binary's designated requirement — permanent for a Developer ID binary, but an ad-hoc binary has no DR so macOS pins a `cdhash` that dies on the next build.

**Fix (in-app, deliberately)**: `keychain::macos_acl` hand-declares the four Security.framework calls `security-framework` 2.11.1 lacks and creates items with an ACL naming every CXMail binary. `store_credential` branches on existence (modify-in-place preserves the ACL — `keyring`'s `set_generic_password` is find-then-modify, so **token refreshes do NOT disturb ACLs**; only the add branch mints a fresh single-binary ACL). `repair_keychain_acls()` runs once at launch behind a `.done` marker — it must run **inside the signed app**, the only process trusted on every item (a shell script would fire one dialog per item). `trusted_binary_paths()` returns empty outside a `.app` bundle on purpose (an unsigned build would mint cdhash-pinned entries that die on its own rebuild). The loose `target/release/cxmail-mcp` needs no separate ACL entry: trusted apps are stored as designated requirements, and ship.md signs it with the same identifier + team as the bundled copy.

**Sign `cxmail-mcp` after EVERY rebuild** (ship.md 2b): sign a copy and `mv` into place — never in place (other sessions may be executing that inode). Necessary but not sufficient on its own.

**`npx tauri dev` = prompt storm**: the ad-hoc debug binary fails (1) and (3), and `lib.rs`'s 30s tick reads 2 Keychain-only calendar keys × 8 Gmail accounts, forever; *Deny* writes nothing so it re-asks. Fix is one codesign of the debug binary — **`--identifier com.cxmail.app` is load-bearing** (a bare Mach-O defaults the identifier to the filename, producing a valid signature that doesn't satisfy the installed app's requirement, so it still prompts). Use `scripts/dev-signed.sh` (build → sign → verify → launch; every Rust rebuild destroys the signature). Do NOT "fix" by mirroring calendar tokens into `credentials.dat` — that's XOR obfuscation, not encryption.

**Diagnosing a fresh report, in order**: (1) `bash scripts/audit-keychain-acls.sh` — ⚠️ it wraps `security dump-keychain -a`, which can **silently truncate**; check the item count before believing rows. (2) Binary column dirty → in-app repair hasn't run: quit, delete the `.done` marker, relaunch, grep log for `keychain ACL repair:`. (3) Partition column dirty → `scripts/fix-keychain-partitions.sh` (**Chris runs it** — needs login password; uses `while read`, not `mapfile`, because macOS bash 3.2). (4) Both clean and still prompting → the reader is a binary the ACL doesn't name; check its `codesign -dv` team + identifier.
**Don't identify the requester from the dialog's name** — tauri-build stamps the same `CFBundleName = CXMail` into every binary, and the dialog names the item's label (`cxmail` for all items). The *icon* is the tell: real icon = bundled app, generic exec icon = loose Mach-O. Correlate with the app log and token expiry.
Pattern: `keychain/macos_acl.rs` (+ `--ignored` FFI self-test), `keychain/macos.rs`, `keychain/mod.rs::repair_keychain_acls`, `scripts/audit-keychain-acls.sh`, `scripts/dev-signed.sh`, ship.md step 2b. Related: #6, #28.

---

## Frontend (continued)

### 32. "Not found: Message UID N not found" on a message we have cached — the reader asked the wrong account
`ReadingPane` derived the fetch account from the view (`needsMessageLookup ? selectedMessageAccountId : selectedAccountId`); Needs You is neither unified inbox nor a group, so the message's own `account_id` was discarded for whatever account was selected last → wrong `(account_id, folder, uid)` triple → cache MISS → IMAP fetch into a mailbox that never held that UID. The error names a UID that exists with a healthy cached body, sending you toward cache-corruption theories; the null-account variant fails silently and leaves a stale error painted.
**Prove it in one grep**: `grep "<uid>" CXMail.log` shows the fetch's account; `SELECT account_id FROM messages WHERE uid=…` shows the real one — two different ids is the bug. (A `NotFound` reaching the UI proves the MISS branch, i.e. wrong triple, not cache.)
**Fix**: a selection carrying its own `account_id` always wins; view-derived is fallback only — `selectedMessageAccountId ?? (needsMessageLookup ? null : selectedAccountId)`, mirroring the folder rule. Safe because every `setSelectedMessage` call site passes the row's `account_id`. Any future cross-account view inherits this bug class — pass `(uid, account_id, folder)` and let the selection win. Regression: `ReadingPaneAccountResolution.test.tsx` (mutation-tested). Pattern: `ReadingPane.tsx`.

### 33. Editable content inside a ProseMirror node view needs an UNEDITABLE root — otherwise the first keystroke replaces the whole node
Two facts, lethal together: `contenteditable="true"` nested inside another is **one editing host** (focus stays on the PM root), and `stopEvent` is dispatched by event *target* — keyboard events target the focused PM root, so `stopEvent` is never consulted. PM's selection is a `NodeSelection` on the atom, so the first character "replaces the selected node": the whole styled card gone.
**Fix**: node-view root `contenteditable="false"`, editable child one level down (`data-cx-html-body`) — an editable child of an uneditable parent IS a genuine editing host. The wrapper is node-view-only; `renderHTML` never emits it.
**Three co-requirements, none optional**: `setAttribute("contenteditable", …)` never the property (PM only skips its stamp when the *attribute* exists; jsdom's property doesn't reflect); the `html` attribute goes stale — debounced push + **`flushHtmlBlockEdits()` synchronously in `buildOutgoingEmail()`** (the single serialize choke point; moving it ships pre-edit HTML); save/restore the DOM range around the sync dispatch and skip rewriting `innerHTML` on the echo (`syncedHtml`), or the caret is yanked mid-word.
**Toolbar**: PM's selection is stale while the caret is in a block — `isEditingHtmlBlock()` routes formatting to `document.execCommand`; buttons need `onMouseDown={e => e.preventDefault()}` or the click blurs the host.
**The same `stopEvent` kills TipTap's KEYMAP, so Cmd+B/I/U are dead inside a block — and macOS does not fill the gap.** ProseMirror never sees the keystroke, so `Mod-b` never runs; and Cocoa has no Cmd+B binding of its own (bold in a native editing host comes from a *Format menu item*, which is why Safari's contenteditable bolds and a WKWebView app with no such menu does nothing). Result: type freely, format never — the toolbar buttons were the only way to bold a word in a designed email, since `format()` already routes them to `document.execCommand`. `nativeCommandForShortcut` binds b/i/u on the host to the same commands, `preventDefault`s so a native binding can't toggle it straight back off, and calls `markDirty()` — formatting that never reaches the `html` attribute never gets sent. It also maps ⇧⌘S/⇧⌘X strike, ⇧⌘7/⇧⌘8 lists (by `e.code` — Shift turns `7` into `&`) and ⌘\ remove-formatting, mirroring `lib/composeShortcuts.ts` for the plain editor; change the two together. ⌘K is NOT here: the compose editor's wrapper claims it for both plain text and blocks (opens `LinkButton`), and `useKeyboardShortcuts` skips the palette when the event is already `defaultPrevented`. That handler also stops at every message action while typing — held-⌘ letters used to star (⌘S), mute (⌘M) and re-select (⌘J) the selected message from inside the composer. Mutation-tested (drop the listener, or the `markDirty`, and named tests fail). Still unfixed, deliberately: the toolbar's `active` states read `editor.isActive(...)`, i.e. ProseMirror's stale state, so the B button never lights up inside a block.

**jsdom cannot catch this class** (no real focus, `rangeCount: 0` — all 13 unit tests passed against the broken version); it took Chromium via Playwright. Unit tests pin the structure (`root=false` + `host=true`). Signature/quoted blocks stay sealed on purpose — only `HtmlBlock` passes `editable: true`. Pattern: `src/lib/composeNodes.ts`, `ComposeModal.tsx`.

---

## Backend (continued)

### 34. Omitting `layout` flattens a *designed* MCP draft when the user opens it in compose
**The MCP flattens nothing at draft time** — omitted-`layout` HTML survives ammonia and lands on IMAP intact. The flattening happens at **compose-open**: TipTap StarterKit has no node for `<div>`/`<table>`, so those are stripped to bare paragraphs (pinned by a negative-control test). `layout="card"`/`"rich"` is just the opt-in to the `div.cx-html-block` preservation wrapper.
**Two symmetric failure modes**: designed HTML (tables/inline styles) with `layout` omitted → design silently destroyed at compose-open; `layout="rich"` on trivial markup → structure needlessly locked in compose (#33: words retype fine, structure doesn't). A real `<ul><li>` survives without `layout` — the 2026-07-30 bullets broke because they were **table rows**. So: don't build bullets out of tables, don't reach for `rich` on plain prose — not "always pass `layout`".
**Guards**: `validate_layout` rejects unrecognized values at the **top** of both handlers — in `edit_draft` before the delete+expunge, per #30's rule; `layout_advisory` warns in both directions (omitted + `style=`/`<table` → will be stripped; `rich` + trivial markup → locks structure), silent for simple markup and for `card`. Verify stored structure with `preview_email_html` (`read_email`'s plain-text echo cannot settle it). Related: #22 (`card` is wrong for correspondence regardless). Pattern: `mcp/server.rs::{validate_layout, layout_advisory, wrap_preserve}`.

### 35. "Open in Claude" spawned a whole second Ghostty APP — and the fixing AppleScript has a shell-splitting trap
`open -na` forces a new Ghostty *application* per handoff (own Dock icon each time). #24 wrongly wrote off AppleScript after testing the bare verb: Ghostty 1.3 ships an sdef, and `new window with configuration {initial working directory:…, command:…, wait after command:…}` carries everything `-e` did. `open_in_running_ghostty` tries AppleScript, **falls back to `open -na` on any failure** (fail-open by design).
**The trap**: `command` is **shell-SPLIT** (Ghostty runs it through `bash -c`), `initial working directory` is applied raw — two adjacent fields, opposite rules. The scratch dir has a space, so an unquoted command execs `~/Library/Application` and paints a red launch failure. `ghostty_command` shell-quotes the command; the cwd must stay raw. **Reusable lesson**: a probe for path-handling must use the shape of the real path (the dev probe had no spaces and passed repeatedly against a string that could never work).
**Hardened runtime needs TWO grants**: `com.apple.security.automation.apple-events` in Entitlements.plist AND `NSAppleEventsUsageDescription` in Info.plist. Missing either is **invisible** (refused event ≈ Ghostty not answering → silent permanent fallback). Both only exist in a real `tauri build`. Declined consent sticks until `tccutil reset AppleEvents com.cxmail.app`.
Smaller traps: `new tab` answers -1708 with no args at all (looks like your syntax error); guard on `is running` (Launch Services, doesn't launch) because cold start restores saved windows — #24's problem — so cold start belongs to the fallback. Pattern: `commands/claude_handoff.rs`, `Info.plist`, `Entitlements.plist`. Requires a full rebuild; source-only checks prove nothing for plist changes.

### 36. `MailRule` is wider than the engine that runs it — three ways to save a rule that does nothing, and one that silently eats the whole mailbox
`MailRule` is permissive JSON (`field`/`operator`/`action_type` bare Strings, validated nowhere); `evaluate()` degrades bad input to "never matches". Traps:

| # | Trap | Why |
|---|---|---|
| 1 | `move`/`delete` actions | accepted, **never executed** (classification path skips destructive IMAP ops) |
| 2 | `body` conditions | rules run at classification time against headers only — body is always `""` |
| 3 | **empty conditions list** | vacuously true → matches **every message**; blank condition `value` too (`"".contains("")`) |
| 4 | unknown field/operator/category | silently never-matches / files under an invisible category |

`to` was a fifth trap, **fixed** (both call sites passed `""` while `to_list` was populated) — note pre-existing abandoned `to` rules start firing after upgrade.
**Validation lives at both boundaries, duplicated deliberately**: Rust (`validate_rule_conditions`/`validate_rule_actions`/`mail_rule_warnings` in `mcp/server.rs`) and the TS mirror (`src/lib/mailRuleValidation.ts`, used by RulesManager) — **`RULE_FIELDS`/`RULE_OPERATORS`/`RULE_ACTIONS`/`RULE_CATEGORIES` exist in two files and must change together.** `list_mail_rules` annotates legacy rows with `warnings`.
**`preview_mail_rule`** = read-only dry run through the real `evaluate()` — the only way to tell "dead rule" from "hasn't synced yet".
**`apply_mail_rule`** load-bearing points: `dry_run` defaults **TRUE**; **one matcher** (`scan_rule_matches` behind both preview and apply — do not add a second); re-validates the stored rule (`rule_apply_blockers` = warnings + blank-value check) and refuses UI-made/legacy bad rules — an apply tool must not bypass the guards; deliberately does NOT reuse `reclassify_inbox_messages` (whole-classifier blast radius + gotcha #11 risk); chunked transactions (CHUNK=200) because the app holds the same SQLite file.
**Flags are pushed to IMAP** (T26): `apply_server_flags` reconciles FROM the server each full sync, so local-only `is_read` writes get reverted — the apply pushes `\Seen`/`\Flagged` via the same `store_flags` as `mark_as_read`, strictly **after** the transactions commit with the DB connection dropped; a failed push never fails the apply (reports `imap_push_failed`). Category stays local by nature.
⚠️ **Verifying over stdio: keep stdin OPEN** — rmcp treats EOF as shutdown; `subprocess.run(input=…)` kills the server mid-push after the DB writes commit, which reads exactly like "the flags reverted again".
Removed a duplicate caller-less Tauri command trio (`commands::settings::{list,create,delete}_rule`) — don't reintroduce a second set. Pattern: `mcp/server.rs`, `db/rules.rs::evaluate`, `commands/messages.rs::classify_headers`.

### 37. Raw headers live in their OWN table because `message_bodies` doubles as the body-cache flag — and there is no backfill, by design
`message_headers` (v46), keyed `(account_id, folder_name, uid)`, `ON DELETE CASCADE`. A `raw_headers` column on `message_bodies` breaks the feature: that table doubles as the "is the body cached?" record (`get_cached_body_uids` drives prefetch; `get_body` returning `Some` makes both body fetch paths skip IMAP), so a header-only row with NULL `plain_text` suppresses prefetch and makes `read_email` call an empty body a cache hit. Pinned by `storing_headers_does_not_look_like_a_cached_body`.
**The sync FETCH is a named header subset — deliberately not widened** (full headers = megabytes across the mailbox). Headers ride along **free** wherever a full raw RFC822 already passes through `persist_parsed_metadata`; otherwise a lazy one-UID `BODY.PEEK[HEADER]`. **`.PEEK` is load-bearing** — a plain fetch marks unread mail read (verified live).
**No backfill** — the block is unreconstructible from stored data; already-synced mail has no headers until someone asks (one small round trip, then cached; both surfaces report `(cached)` vs `(fetched from IMAP)`).
**Two parsers, not interchangeable**: `extract_raw_headers` (full message; requires the blank-line terminator, else stores nothing) vs `header_block_to_string` (server header block; must NOT require a terminator).
**`filter_header_lines` must be folding-aware** — the DMARC verdict usually sits on an RFC 5322 continuation line; a naive prefix filter returns line one, looks right, loses the answer.
Privacy: separate `read_email_source` tool, not a `raw` flag; `raw_headers` stays out of `MessageDetail`. The UI surface was **removed 2026-08-05** (unused; and it rendered broken — **portal any modal that can mount inside a transformed ancestor**, `position: fixed` resolves against the transform). The MCP tool is the whole feature and earned it: it proved #39 on the wire. Pattern: `db/schema.rs::migrate_v46_message_headers`, `db/messages.rs`, `email/parser.rs`, `email/imap.rs::fetch_raw_headers`, `mcp/server.rs`.

### 38. Draining an async-imap response stream is NOT checking the response — a tagged NO/BAD never becomes an item
`uid_store`'s stream is `take_while`-terminated on the tagged completion — **the status line ends the stream and is dropped inside the library**, so a refused STORE looked like success. Measured live: a forced tagged `BAD` yields **0 items** from the drain; the "inspect the drained items" fix finds nothing.
**Fix**: `session.run_command_and_check_ok("UID STORE {uid} +FLAGS ({flags})")` — byte-identical wire traffic, and it surfaces `Error::No`/`Error::Bad`. **Rule**: in async-imap, a helper returning `Stream<Item = Result<T>>` gives untagged data only; if you need the verdict, use a `check_ok`-shaped call.
**There was a second instance grep nearly missed** — `move_message`'s COPY+DELETE fallback had its own inline drain spelled differently; **search for `uid_store` itself, not a drain idiom**. Consequence there was worst: refused `\Deleted` + EXPUNGE = message silently *duplicated* instead of moved.
Rejections now surface: `apply_mail_rule` counters are acknowledgement-based; `edit_draft`'s MCP path aborts (loud) instead of leaving a duplicate draft; app-side `mark_as_read` still only logs. Re-verify with an `examples/` probe using a malformed flag atom (`"((( "`) — a **nonexistent UID is not a usable probe**, Gmail answers OK. Pattern: `email/imap.rs::store_flags` + its 8 call sites.

### 39. Every reply CXMail sent for three months carried NO `In-Reply-To` — and Gmail hid it
Gmail falls back to subject+participant threading when `References` is absent, so threads looked fine everywhere except CXMail itself (which has no fallback) — surfaced only when nudges needed "newest message in thread" and found our sends rooting to themselves. Confirmed **on the wire** with `read_email_source` (#37): sent `Re:` messages with no `In-Reply-To`/`References` at all.
**Two independent holes — fixing either alone changes nothing**: (1) `compose_draft` accepted a `Re:` subject with no reply target; (2) `MessageDetail` had no `in_reply_to`, so `openDraftForEdit` *couldn't* pass it — drafts rebuild MIME from compose props, so absent means deleted; hole 2 would have silently undone hole 1's fix.
**Fixes**: `get_message_headers` returns normalized `(message_id, references, in_reply_to)` (#30's spellings must not reach outgoing headers); **`buildDraftComposeProps` is the single builder** for reopen props — it existed as two copies and every round-trip field had been forgotten at least once; do not re-inline it. `validate_reply_threading` rejects `Re:` with no target — after `resolve_reply_target`, before any IMAP work (before `edit_draft`'s expunge, per #30). Scoped to `Re:` — a `Fwd:` is legitimately a new thread.
**Lessons**: the parsed DB column was right but not *evidence* — read the bytes when a header is in question; and the first (plausible) diagnosis blamed only the reopen path, disproved by one query. History stays fragmented (headers can't be added to delivered mail); nudges fill in as correct replies accumulate. A subject-fallback in `compute_thread_root_id` was deferred, not rejected. Pattern: `db/messages.rs::get_message_headers`, `src/lib/draftCompose.ts`, `mcp/server.rs::validate_reply_threading`.

---

## Database (continued)

### 40. Subject-fallback threading fuses recurring notifications unless it requires a reply prefix
"Same normalized subject + shared participant + time window" also describes every recurring notification: the first stitch pass re-rooted 6,101 messages (379 Synology alerts as one thread). Requiring the orphan's subject to carry a reply prefix (`has_reply_prefix`: `Re:`, `Fwd:`, `AW:`, `re[2]:`, …) took it to 122 real repairs — exactly the population #39 stranded.
**Three more guards, each load-bearing** (pinned by `stitch_tests`): only orphans are eligible (a `References`/`In-Reply-To` header is a fact, a subject match is a guess); threading flows forward only, compared by *position* in a `datetime(date) ASC` query — never raw string compare, that's #23; generic subjects refused outright (`normalize_subject_for_threading` returns `None`, not an empty key, for "Hi"/"Thanks"/<8 chars).
**`reset_first`** recomputes every orphan's root from its own headers before stitching — a corrected rule *undoes* a previous bad run (how v49 repaired v48 in place). Leave it **off** for the per-sync 30-day-window call, which would otherwise strip stitches anchored outside the window.
**Verify on real data** — every unit test passed against the broken version. After any threading change:
```sql
SELECT substr(subject,1,42), COUNT(*) FROM messages WHERE thread_root_id IS NOT NULL
 GROUP BY account_id, thread_root_id ORDER BY 2 DESC LIMIT 10;
```
A top row in the hundreds = over-merging. Pattern: `db/messages.rs::stitch_orphan_threads`, `email/message_id.rs`, `db/schema.rs::migrate_v48/v49`.

---

## Backend (continued)

### 41. Generic IMAP: three traps that each look like a small thing and are not
**1. IMAP STARTTLS is deliberately NOT implemented — the reason is data loss, not effort.** `ImapSession` is a concrete TLS-stream type (~40 signatures go generic otherwise), and the upgrade has a silent-corruption trap: the greeting is read through a `BufReader`, and **`BufReader::into_inner()` discards its buffered bytes** — a server that pipelines `CAPABILITY` after the greeting loses those bytes exactly as the connection goes secure, desyncing the parser commands later. If ever implemented: drain the buffer explicitly, never `into_inner()` a non-empty one. Every preset is 993 implicit, so nothing shipped is blocked. **`TlsMode` has no `Plain` variant, load-bearing** — the moment the enum can describe plaintext, someone surfaces it as a checkbox. Same reasoning keeps Proton Bridge out (self-signed cert needs pinning in BOTH stacks — native-tls for IMAP, rustls for SMTP); if wanted, ship a distinct pinning provider, never a trust toggle.
**2. SPECIAL-USE can be RECEIVED but never REQUESTED** — async-imap's `list()` emits a bare `LIST "" *` and `parse_names` is `pub(crate)`, so name heuristics are mandatory. They match the **last hierarchy segment, whole** — a substring match makes `Sent to Legal` the Sent folder, and `db::nudges` computes off Sent, so the follow-up lane goes quiet with no error (that negative is the load-bearing test). `\Noselect` skipped in the generic path only. **SPECIAL-USE is NOT applied to gmail/icloud/outlook** — Gmail's `\Flagged`/vendor attrs mean `starred`/`important` there, and six call sites read `folder_type`.
**3. `LIMIT 1` with no `ORDER BY` picked special folders at random** — and the duplicate case is reachable (Outlook accepts both "Deleted Items" and "Trash" as trash). `db::folders::folder_of_type` is the single accessor, ordered `special_use DESC, LENGTH(name), name` (server declaration > shallower mailbox > stable tiebreak); `fallback_folder_name` is the single fallback table (only the sync loop knew iCloud's Sent is "Sent Messages"). **Verify a `sent_folder_for_account` change against the real mailbox** — nudges depend on it and a resolution change empties that lane silently (the 2026-08-05 consolidation was proven a no-op: 36/36 resolutions identical).
**Adjacent finds**: `mail-send`'s builder default timeout is **3600s** — every send had a latent hour hang; now 30s. `add_imap_account` validates SMTP as well as IMAP (two trust stores — IMAP success is not evidence about SMTP; `add_icloud_account` still checks IMAP only). `remove_account` used to leave up to seven keychain entries behind (`credential_keys_for` now enumerates, test-pinned). `accounts.imap_port` is i32 — `as u16` would wrap 70000→4464; `port_from_i32` range-checks.
Pattern: `email/providers.rs`, `email/imap.rs`, `db/folders.rs`, `commands/auth.rs`, `src/lib/mailPresets.ts` (TS mirror, drift fails a Rust test).

---

## Environment (continued)

### 42. `vitest` dying with "21 errors, no tests" is the wrong Node, not a broken dependency
`require() of ES Module …` from the vitest fork pool = the shell is on Node 20; this project needs v24.11.1 (no `.nvmrc`, nothing corrects you):
```bash
export PATH="$HOME/.nvm/versions/node/v24.11.1/bin:$PATH"
```
Costs an hour because the error names a real ESM-only dep chain and `npm ls` confirms it — every check agrees with the wrong conclusion. Tells: `git log -1 package-lock.json` predates tests that demonstrably ran after it; `cargo test` + `tsc` fine while vitest is dead is the shape of an environment problem. Stashing your changes proves the failure isn't your code — it proves nothing about your environment (constant across both runs). Related: #1.

---

## Database (continued)

### 43. A derived voice profile can only DESCRIBE — so "always" had nowhere to live
Chris wants Sam Ellis always addressed "Bro. Ellis"; the extracted profile faithfully hedged because the sent corpus genuinely mixes ("Bro. Ellis" twice, "Sam" once). **A statistical description of past behaviour cannot express "always"** — retraining/re-extracting can't fix it, and `extract_recipient_profile(force=true)` rebuilds `profile_json` wholesale, destroying any hand edit. Fix (v51): `voice_pinned_rules` — user-authored, absolute, permanent, never inside `profile_json`. Scoped `account` or `recipient`; account rules returned first so the specific rule reads last and wins.
**Easy to undo by accident:**
1. **No FK to `voice_profiles_recipient`** — a cascade reintroduces the bug: evicting a cached profile is a cache operation, not permission to forget what the user said. FK to `accounts` only. (Test-pinned.)
2. **A skill instruction is not enforcement** — `compose_draft`/`edit_draft` are the obligatory path, so they append a `⚠ PINNED RULES` note. Advisory, not refusal: no check can decide whether prose honours a rule, but it can make it impossible to miss. `get_voice_profile` returns `pinned_rules` top-level (not merged into `profile_json`).
3. **`get_voice_profile` no longer errors on rules-with-no-cached-profile** (`pinned-rules-only`) — the old hard-fail would hide rules for exactly the recipients nobody extracted yet.
**Scope/address contradictions rejected in both directions** (they fail invisibly and oppositely: account-scope + address → applies to everyone; recipient-scope − address → no one; a rule pinned to a *name* stores fine and never matches). Caps: 500 chars/rule, 20/scope — rules ride in every draft's prompt context. Pattern: `db/voice_pinned_rules.rs`, `db/schema.rs::migrate_v51`, `mcp/server.rs`, `email/ai.rs::build_voice_context_full`. Related: #36, #30.

### 44. A Zoom link cannot live on `gcal_events` — the sync nulls it, and four deleters orphan the meeting invisibly
Context: `create_calendar_event` gained `conference: "zoom"` (DB v52); Meet stays the default, Zoom is opt-in per call.
**Why not a column**: (1) `upsert_remote_event` is the sole INSERT and is driven entirely by Google's payload — every sync tick would null a `zoom_meeting_id`; the only column that survives (`pending_notify`) does so via an explicit correlated `COALESCE((SELECT …))` rescue inside the INSERT. (2) Four paths DELETE from `gcal_events` (`sweep_full_sync`, `purge_old_cancelled`, `delete_local`, the CASCADEs) — the id dies with the row and a real meeting keeps running with nothing knowing. So: side table `zoom_meetings`, **no FKs at all** (evicting a mirror row is a cache op, not permission to forget — one step past #43); an unlinked row is a tombstone `list_orphans` finds, counted in settings. **Keyed on the remote triple** `(account_id, gcal_calendar_id, gcal_event_id)`, not `gcal_events.id` — that id is AUTOINCREMENT and a routine sweep-then-repull would orphan an id-keyed link. Tripwire test asserts `gcal_events` has NO `zoom_meeting_id` column, so "simplifying" back fails immediately.
**Change detection is a string compare on a byte contract**: `gcal_events.dtstart` is always `…Z` (`normalize_event_times` → `to_rfc3339_opts(Secs, true)`), byte-identical to Zoom's `start_time`, so comparing against `pushed_topic/pushed_start/pushed_duration` (what Zoom is *believed* to hold, advanced only on 2xx) needs zero tz maths — and yields: no PATCH on a mere touch, idempotence, self-retry, renames that send `topic` without `start_time`. Break the `…Z` contract and it silently pushes every tick.
**Mirror-driven reconciler**: the common reschedule is a drag in GCal's web UI, which no local code observes; `zoom_sync::plan_for` reads the mirror on the background tick (after `sync_all`). **Positive-evidence rule**: never delete a Zoom meeting because a mirror row is *absent* (absence is ambiguous — sweep/purge/removal/fresh DB); deletion needs an explicit tombstone (`delete_pending`), `status='cancelled'`, or a targeted GET answering 404/410; unverifiable rows park `unverified`, never auto-deleted. The guarded regression: clear the sync token → sweep deletes every mirror row → nothing may be deleted on Zoom. (Mutation-tested: absence-means-delete fails.)
**Record predates object**: `begin_create` writes a durable `creating` row before Zoom is asked; `mark_delete_pending` before the delete call. Corollaries: revert the tombstone if the Google delete then fails, or the reaper kills a live meeting; a failed-insert compensation must NOT drop the local row when the Zoom delete also fails (the row is what makes the reaper retry); on a transport sentinel (status 0), probe with `get_event` and if that fails too, **leave everything alone**.
**`AppError::ZoomApi` is its own variant** — three places match `CalendarApi(404|410)` and respond by deleting the **Google** mirror row; a Zoom 404 reaching those arms deletes the wrong thing (test-pinned).
**`start_url` is structurally unreachable**: `ZoomMeeting` has no such field; serde drops it at the parse boundary — the host-authenticated URL cannot reach a log, row, error, or tool result. Stronger than a review convention.
**Join URL goes in `description` only** (per #21), in a fenced `[cxmail:zoom]…[/cxmail:zoom]` block asserted byte-for-byte on both sides (drift = no Join button, which looks like the meeting was never created). Frontend needed zero changes — `meetingLink.test.ts` staying green untouched is the proof.
**Two adjacent fixes**: `update_calendar_event` sets `description` wholesale, so the MCP refuses when the zoom block would be dropped (UI path untouched — the textarea shows what's being replaced); the invite approval card hardcoded `"Meet": hangout_link.unwrap_or("None")` — now a Zoom-first `Conferencing` row.
**Deliberately not wired**: recurring + all-day refused at the boundary; Meet↔Zoom switch unsupported (delete and recreate); the dormant `local_new`/`local_deleted` push arms in `gcal_sync`, if resurrected without touching `zoom_sync`, create no Zoom meeting / leave one behind; past meetings retire (`state='retired'`), never deleted — deleting a completed Zoom meeting can destroy its cloud recording. No Zoom-side sweep (needs an unapproved scope); create bodies carry `agenda: "CXMail:<gcal_event_id>"` for future correlation.
**Operational**: Server-to-Server OAuth app, five account-global keys `zoom:cxmail:*`. **Scope names are the granular `:admin` variants** (`meeting:write:meeting:admin` etc.) — the plain spellings in every doc do not exist on the Scopes tab, `:master` is wrong. Watch the first live create: if `POST /users/me/meetings` is rejected under an admin-scoped token, address the owner's email instead of `me`. Keys are **deliberately absent from `credential_keys_for`** (removing one Gmail account must not delete Zoom for the rest; test-pinned). `save_zoom_credentials` validates before persisting and unwinds all five on partial write; the Test button reports the **granted scope string** — the only thing distinguishing "scopes never added" from "bad secret".
Pattern: `email/zoom.rs`, `email/zoom_sync.rs`, `db/zoom.rs`, `db/schema.rs::migrate_v52`, `commands/zoom.rs`, `email/gcal.rs`, `mcp/server.rs`, `ZoomSettings.tsx`. Related: #21, #23, #31, #43.

### 45. `u8 as char` is a Latin-1 decode — it double-encoded every snippet, and the body sitting next to it was fine
`html_to_plain_text` did `out.push(bytes[i] as char)` — that cast maps a byte to the same code point, i.e. **Latin-1** — so each UTF-8 byte of `“` became its own char and was re-encoded (`E2 80 9C` → `C3A2 C280 C29C`). Two of three resulting chars are invisible C1 controls, so it reads as "a weird `â`" rather than an encoding bug. Only the **snippet** was hit, and only on the HTML-extractor fallback (the norm for newsletters whose `text/plain` is CSS); `mail_parser` decodes parts correctly, so subject and body beside it were clean — that contrast is the diagnostic.
**One query settles it** (and rules out charset-header/MIME/mail-parser theories): compare `hex(substr(snippet,…))` vs `hex(substr(plain_text,…))` for the same uid — two encodings of the same character is the bug.
**Fix**: accumulate **bytes**, decode once (`Vec<u8>` + `from_utf8_lossy`). Splitting a char is impossible — the tag state machine transitions only on ASCII `<`/`>`. **The identical bug was in `commands::messages::percent_decode`** (mailto unsubscribe), fixed together. **Grep `as char` after touching any decoder** — the only correct uses here are `(b as char).to_digit(16)` on known-ASCII hex.
**v54 repair**: snippets are written once at ingest and never recomputed, so the code fix reaches nothing — `repair_double_encoded_utf8` repairs in place (the transform is the exact inverse; a quarter of the rows have no cached body to re-derive from). **Three guards carry the whole safety**: every char ≤ U+00FF, those bytes **strictly** valid UTF-8, result differs. `from_utf8_lossy` there would mangle genuine Latin-1 (`Café` → replacement chars) instead of declining — the lossy mutation fails the migration test. Idempotent (repaired text holds U+201C, failing guard 1).
**It runs per PIECE, not per string** — both reasons found only by probing a copy of the real mailbox: `clean_snippet` truncates at 200 bytes on a *char* boundary, stranding a mojibake lead byte (tolerated only when `Utf8Error::error_len() == None` at end-of-string); and it decodes HTML entities *after* the mangling, so one correct char beside corrupted bytes must not reject the row. 391 → 417 repaired; C1 controls 407 → 19; the last 19 are unrecoverable (the continuation byte was an NBSP that whitespace-collapsing turned into a plain space — the information is gone) and all invisible preheader padding. The repair re-runs `strip_invisible_chars` so repaired rows match what the fixed code now produces.
**Probe a data migration against a copy of the real mailbox** (same rule as #21b) — every unit test passed against the flawed whole-string version, and the probe also proved three visually-identical diffs were shredded emoji, not corruption.
Pattern: `email/parser.rs::{html_to_plain_text, repair_double_encoded_utf8, flush_latin1_piece, strip_invisible_chars}`, `commands/messages.rs::percent_decode`, `db/schema.rs::migrate_v54`. `messages.snippet` is not an FTS5 column (#26).

### 46. A counter that INFERS live sockets will lie — and it lies most convincingly when it correlates with the symptom

**Symptom**: "Failed to save draft: IMAP error: Connection timed out for chris@cxventures.io (gmail)", ~50% of that account's connects timing out. The session ledger reported `believed_open=14` for that account — reads as textbook Gmail slot starvation (cap is 15). **It was not 14**: `lsof -nP -iTCP -a -p <app>` showed 11 sockets to `:993` for the whole process, all nine accounts. The counter was measuring something that is not a socket.

**Two independent reasons it could never have been one**: (1) `email/idle.rs` reaches `disconnect` on NO error path — every `?` in `run_idle_session` abandons the session, so each reconnect incremented `opened` and nothing else (`bin/helper.rs` identical). (2) Even a genuinely unaccounted session is not an open socket — dropping an `ImapSession` drops its `TcpStream` and closes the fd; only the *server-side* slot is ever in question.

**Why it was believable — the part worth keeping**: the IDLE loop reconnects hardest on the account failing most, so the phantom count rose fastest on exactly the sick account and climbed monotonically with uptime. A number that is wrong *and* correlated with the symptom survives a sanity check.

**Fix**: `abandoned` counts sessions ending with no LOGOUT attempted; `slot_suspect` = `dropped + abandoned` is the only number that speaks to the server-side theory; `unaccounted` is a self-check on the LEDGER (should read 0, says nothing about Gmail). Accounting rides on `imap::SessionGuard` (RAII) — a call at each error site is the arrangement that produced the hole. Construct the guard only on the `Ok` branch of `connect_for_account` (a failed connect never incremented `opened`; counting it pushes `unaccounted` negative).

**Do not diagnose TCP with ICMP.** `ping6` to `imap.gmail.com` fails 100% from 1240-byte payloads up — the exact signature of a PMTU black hole, and it's wrong: a real TLS session sending a 1410-byte command was answered in 0.01s over IPv6. Google simply declines large ICMPv6 echo. Test the protocol you are debugging:
```python
s = ssl.create_default_context().wrap_socket(
        socket.create_connection(("imap.gmail.com", 993)), server_hostname="imap.gmail.com")
s.recv(4096)                                  # greeting
s.sendall(b"a2 XPAD " + b"X"*1400 + b"\r\n")  # >1280B in one write
s.recv(4096)                                  # a reply here means large writes are fine
```

**`uptime_secs()` is AWAKE seconds, not wall clock** — built on `Instant`, which doesn't advance during sleep; check against `ps -o etime` before concluding the process restarted.

**The restart settled it (2026-08-14)**: a fresh process, counters at zero, still timed out on `chris@cxventures.io` — ten minutes later `slot_suspect=1` (cannot exhaust a 15-cap) and 0 OK / 5 timeouts, while the other eight accounts went 24 OK / 1 timeout on the same network. The failure is **account-specific at Google**, not a leak and not the network — and the ledger's own founding hypothesis was wrong; the old instrumentation had been supplying its own confirming evidence. (`unaccounted` behaved exactly as designed: 8 across healthy accounts = 8 `lsof` sockets, one long-lived IDLE each.)

**Still open, deliberately**: IDLE abandons sessions *without attempting LOGOUT* — hygiene, not urgent, now that the leak theory is refuted; `slot_suspect` climbing on an otherwise healthy account is the signal that would make it urgent again.

**Pattern**: `email/imap.rs` (`SessionLedger`, `note_abandoned`, `SessionGuard`), `email/idle.rs::run_idle_session`, `bin/helper.rs`, `lib.rs` (periodic ledger line). Guard tests are mutation-tested (no-op `Drop` fails). Related: #10, #12.


### 47. A pinned rule that says "never" cannot be kept by a prompt — enforce it at the write boundary

**Symptom**: drafts keep coming back with em-dashes even though an account-scoped pinned rule says "Never use em-dashes", the rule is stored correctly, and `pinned_rule_advisory` prints it on every draft.

**Two reasons, neither fixable by rewording:**
1. **The advisory arrives after the write.** It rides in the *tool result* of `compose_draft`/`edit_draft`, so the draft is on IMAP before the model is reminded. It buys a follow-up `edit_draft`, not compliance.
2. **The derived profile argues the other way, in the same context window.** Every stored profile described the real past habit — `"punctuation_style": "Uses em-dashes, parentheses, bullets…"` — and `ai::build_voice_context_full` renders the *recipient* profile as the style to MATCH. A statistical description and an absolute rule were being weighed against each other, and the description usually won. (Same root shape as #43: derived describes, pinned commands.)

**The fix is `email::dashes::normalize_dashes`, applied where nothing can skip it.** `enforce_dash_rule` in `mcp/server.rs` runs in both draft handlers; every prose generator in `ai.rs` goes through `call_inference_prose`. Because it sits at the write boundary rather than in a prompt, it also catches text the model never authored — a paste, or (the case that prompted it) an **external voice-rewrite service** handing prose back with the dashes reinstated. That is the argument for the boundary over the prompt: you cannot prompt a service you do not own.

| Situation | Result |
|---|---|
| opens a line, follows a closed tag or an opening delimiter | dropped (`<p>— Chris` → `<p>Chris`) |
| ends a line, precedes a tag or punctuation | dropped |
| digit both sides | ` to ` (`2010—2020`, `$3,500 — $7,200`) |
| follows punctuation | one space |
| anything else | `, ` |

**Never a hyphen** — the rule bans that stand-in explicitly, and it is what every naive de-AI script reaches for.

**Placement is load-bearing in two directions:**
- **Authored text only** (`params.subject`/`params.body`), *before* `render_body`, the signature, and the quote are attached. `format_quoted_history` output is a byte-contract with the frontend (#30) and is the other person's prose. Pinned by `the_quoted_original_is_out_of_the_dash_rules_reach`, which also asserts that normalizing the *composed* string WOULD damage the quote — so the ordering requirement can't quietly stop being true.
- **Above the IMAP work in both handlers** — in `edit_draft` that means above the delete+expunge, per #30.

**Four things it must not touch, each a way to be "correct" and destructive:** tag interiors (a `layout:"rich"` body's `href`/`style`), `<style>`/`<script>` bodies, dashes inside a URL, and an **unspaced** en-dash (`2010–2020` is a range; only the spaced form is the parenthetical dash). And **never normalize the pinned-rule text itself** — the rule names the character it forbids, so normalizing it produces "Never use em-dashes (,)".

**HTML entity spellings are the trap a literal replace misses**: `&mdash;`, `&#8212;`, `&#x2014;`, `&horbar;`, `&ndash;`… An agent writing designed HTML emits them routinely, and `body.replace('—', ", ")` sees none of them.

**v55 scrubs the stored profiles**, because a code fix reaches no stored data (#45's lesson) — a profile is written once and only rebuilt on a forced re-extraction. `scrub_dash_claims` removes the claim clause-by-clause and **returns None — leave it exactly as it was — whenever it cannot do so cleanly**; a mangled profile would be fed into every future draft with nothing to catch it. `voice_examples` is deliberately untouched: verbatim excerpts of mail actually sent are evidence, and rewriting the record of what someone wrote is a different thing from governing what gets written next. The seeds are gone from `voice.rs` too — the extraction schema hint led with `"em-dashes, semicolons…"`, and the learning prompt's example of a rule to append to future system prompts was, verbatim, `'Use em-dashes instead of colons.'`

**⚠ The migration's first draft addressed rows by `id`. `voice_profiles_recipient` and `voice_archetypes` have COMPOSITE primary keys and no `id` column at all** — but the test fixture invented one, so the unit test passed while the migration would have failed on every real database. Address by `rowid` (none of the three is WITHOUT ROWID). Same lesson as #21b/#45: **probe a data migration against a copy of the real mailbox** (`.backup`, run `initialize`, diff) — that is what proved all four live profiles scrub correctly and re-run clean.

**⚠ Verifying an MCP change from a Claude session: your own `mcp__cxmail__*` tools are bound to the binary that was running when the session started.** Calling them after a rebuild exercises the OLD code and looks like the fix didn't work. Rebuild, codesign (ship.md 2b, #31), then drive the new binary yourself over stdio — keeping stdin OPEN, per #36.

Pattern: `email/dashes.rs`, `mcp/server.rs::enforce_dash_rule`, `email/ai.rs::{call_inference_prose, NO_LONG_DASH_RULE}`, `email/voice.rs::parse_voice_extraction_response`, `db/schema.rs::migrate_v55_scrub_dash_claims_from_voice_profiles`. Related: #43, #30, #45.


---

### 48. Bare text in a `<td>` cannot be styled — and the wrapper that fixes it must render as nothing

An agent writing designed email (`layout: "card"`/`"rich"`) keeps producing a styled cell whose copy is a **bare text node** — a direct child of the `<td>`, in no element:

```html
<td style="font-size:15px;color:#334155">
  <div style="font-weight:bold">TL;DR</div>
  The growth is now a trend, not a hope.
</td>
```

Every per-paragraph declaration the author wrote for that line — `margin`, `font-size`, `color` — applies to nothing, because there is no element to carry it, and nothing reports a problem. `email::loose_text::normalize_cell_text` wraps each bare **inline run** in a `<p>` before `wrap_preserve`, on both styled paths, making it unrepresentable rather than documented (same posture as `validate_pinned_rule`).

**⚠ The mechanism this was originally blamed on is not real, and it was checked three ways.** The theory was "TipTap has no node for loose text, so it wraps it in a paragraph carrying the editor's default margins." It does not:
1. Real TipTap round-trip (`setContent` → `getHTML`) with `HtmlBlock` returns the cell **verbatim** — the block is an atom, so ProseMirror never parses its interior into schema nodes.
2. Real Chromium: bare text and the wrapped version render **identically** — same `td` height, same font-size, same color, gap 0. Source pretty-printing does not paint as a first-line indent either: the collapsed leading space sits at the start of a line box and is stripped.
3. Real Chromium `contenteditable`: typing into the bare run, and pressing Enter in it, insert text and a `<br>` — no element wrapper.
So whatever produced the 2026-08-18 gap-and-indent report, it was not this. The normalizer is still right (it gives the line a styling hook and trims stray whitespace), but **do not describe it as the fix for a rendering gap** — it is provably layout-neutral, which is the property that makes it safe to apply to every designed email.

**The wrapper's four `inherit` declarations are load-bearing.** `apply_inline_font_styles` runs after this pass and prepends `font-family:Arial…;font-size:13px;color:#222222` to every `<p>`. A plain `<p style="margin:0">` would therefore **restyle** the text it just adopted — a 15px slate paragraph inside a designed card comes out 13px `#222222`, and a `font-size:0` spacer cell grows a line box. `font-family`/`font-size`/`line-height`/`color:inherit` are not matched by `OUR_DECL_RE`, so they survive the merge and win by coming last. Tripwire: `wrapper_survives_font_pass_without_restyling_the_text`.

**Scope limits, each deliberate:** `<td>`/`<th>` only (loose text in a `<div>` already forms an anonymous block that inherits correctly; text directly in `<table>`/`<tr>` is foster-parented out by html5ever before this pass sees it, and a `<p>` there would be too). The unit wrapped is the **whole inline run**, not each text node — wrapping the text nodes of `Hello <b>world</b> again` separately turns one sentence into three blocks. A run with no bare text is left byte-identical, and `&nbsp;`-only runs count as whitespace so the `<td style="height:4px">&nbsp;</td>` spacer idiom is untouched.

**The pass never loses a byte**: it is a hand-rolled scanner (no new dependency; `ammonia` already re-serializes downstream), every token is re-emitted in source order, and the only edits are the inserted wrapper and **ASCII** whitespace trimmed off the ends of a wrapped run — NBSP is content, not indentation. Malformed input degrades to "changed nothing important", pinned by `unclosed_and_stray_tags_do_not_lose_content`.

`layout_advisory` speaks up only when a wrapped run shared its cell with a block element — the shape where the author styled one line and left the next bare. A cell that is nothing but text is normalized silently; warning there would fire on most designed email and train callers to ignore the note (#34's reasoning).

Pattern: `email/loose_text.rs`, `mcp/server.rs::{render_styled, layout_advisory}`, `email/inline_styles.rs::OUR_DECL_RE`, `src/lib/composeNodes.ts::HtmlBlock`. Related: #33, #34, #22.


---

### 49. "Open in Claude" lands in a repo now — and `--add-dir` is variadic, so flag order decides whether the prompt exists

The handoff used to `cd` into its own scratch directory, so Claude arrived with the email and no project. It now `cd`s into a repo resolved from the message (`db::claude_repos`, DB v56) and passes the scratch dir with `--add-dir` so `email.json` and `./attachments/` stay readable from there.

**`claude --add-dir <directories...>` is declared VARIADIC.** With the flag first, `claude --add-dir <scratch> "$(cat prompt)"` hands the prompt to `--add-dir` as a *second directory*. Proven on the real CLI: under `--print` it answers `Error: Input must be provided either through stdin or as a prompt argument`. **Interactively — which is how the handoff runs — there is no error at all**: a normal-looking Claude window opens with no prompt, which reads as "the handoff is broken" and survives a manual smoke test as "it opened, looks fine". So: **positional prompt first, `--add-dir` last**, and do not tidy the flags to the front. Pinned + mutation-tested by `a_mapped_repo_becomes_the_cwd_and_the_scratch_dir_stays_readable`.
Second consequence of the `cd`: **the prompt file must be read by absolute path**. `$(cat prompt.txt)` was relative to the scratch dir that is no longer the cwd, and would expand to nothing — the same silent empty session by a different route.

**The two prompts carry the repo differently, and that asymmetry is the point.** The launched handoff is *already* in the repo, so its prompt states where it is. `get_claude_prompt` (Option-click → Copy Claude Prompt) is pasted into a session that is somewhere else entirely, so it carries an actual **shell-quoted `cd`** — the path is not hypothetically spacey, `Application Support` is not the only such directory. Both take the same existence split (`split_on_existence`) so they cannot disagree: a mapping whose directory is gone is **named but never `cd`-ed into**, since sending someone's live session into a missing directory turns a stale mapping into an error in their terminal. Pinned in four directions (usable / quoted / missing / unmapped).

**Resolution is four scopes, most specific first**, each skipped when it has no repo mapped so an unmapped group can't shadow the account behind it: a mapped `contact` (address or bare domain) → a `group` whose **rules** match → the `account` → a group that merely **contains** that account → `default` → none, meaning the old scratch-dir behaviour. Contact outranks group because a group rule can be as loose as `subject contains "Northwind"`, while a domain names a party.

**The contact layer reads recipients as well as the sender**, minus the user's own addresses. Sent mail names the client in `To`, and `dana@ironside.example` writing about Northwind with Northwind people on cc is Northwind work — both verified against the real mailbox. Own addresses are excluded because they sit on both sides of your own mail and a contact row keyed on one would match everything; keying on your own address is what the `account` scope is for.

**A domain must match on a label boundary** — `domain == contact || domain.ends_with(".{contact}")`. A plain `contains`/suffix-of-text check makes `notharborline.example` a client. The trap it guards against is real: two unrelated organisations can share a name prefix (`bluestonepresents.example` vs `bluestoneadvisors.example`), and a resemblance match would file one's mail into the other's repo. Note also that a company's *mail* domain can differ from its web host (`harborline.example` vs `harborlineweb.example`) — map the domain the mail comes from.

**`claude_repos` cascades on BOTH foreign keys, and the group one is not tidiness.** `inbox_groups.id` is a plain INTEGER PRIMARY KEY, so SQLite reissues a deleted group's id to the next group created — an orphan mapping would silently reattach to an unrelated group. (Contrast `zoom_meetings` (#44), where a missing row is *ambiguous* and the orphan must survive as a tombstone; here the row's whole meaning is the group it names.) Test-pinned.

**One matcher, not two** (#36's rule again): `build_group_filter` was split into `build_account_conditions` + `build_rule_conditions` so the rules could be evaluated alone, and a test pins the composed SQL **byte-for-byte** — that builder backs the sidebar's group unread counts, and a reordering there would shift them with nothing to notice.

**A mapped repo that isn't there fails OPEN**: it lands in the scratch dir, says so in the prompt, and returns `missing_repo_path` so the toast says so too. Mappings are only ever made by the user, in Settings → Claude repos; v56 creates the table and seeds nothing.

**The landing is otherwise invisible** — the Ghostty window looks identical wherever it `cd`ed — so `ResolvedRepo.source` names the row that decided it, in the toast and in the seed prompt. Verify a mapping against real mail with `cargo run -p cxmail-db --example resolve_repo_probe -- <copy.db>` (a `.backup` copy, never the live file — #12).

Pattern: `db/claude_repos.rs`, `db/inbox_groups.rs::{build_rule_conditions, groups_with_rules_matching}`, `db/schema.rs::migrate_v56_claude_repos`, `commands/claude_handoff.rs::build_launch_script`, `commands/claude_repos.rs`, `ClaudeRepoSettings.tsx`. Related: #24, #35, #36, #44.


---

### 50. Google records no "was this invitation sent" fact — so an event can look published, with guests attached, that nobody ever received

**Symptom**: attendees keep asking for the meeting link for a meeting that is plainly on your calendar with their name on it.

**Adding an attendee is a field write; notifying is a separate side effect.** It is governed by `sendUpdates`, whose **API default is `none`** — and every CXMail path passed `SendUpdates::None` except the two explicit send paths (`commands/calendar.rs` create/patch/**delete**, so cancellations did not reach attendees either). Google's web UI forces the same choice with a *"send invitation emails?"* modal; the API just defaults to silence.

**Why it is invisible, and this is the load-bearing part:** an attendee object carries exactly five fields — `email`, `displayName`, `organizer`, `self`, `responseStatus` — and **none records whether an invitation was emailed**. `responseStatus` is stamped `needsAction` the instant an attendee is attached, so *"invited, hasn't replied"* and *"never told, has no idea"* are byte-identical on the organizer's calendar. The calendar was never lying; it has no way to express the difference.

**The Google/Microsoft asymmetry explains who suffers.** A Google-Calendar attendee can receive the event through Google's own backend with no email at all (measured: 5 of 9 externally-organized events had no invitation email anywhere in the mailbox). An **M365/Exchange** attendee has no such path — iMIP-over-email is the only cross-org transport in existence, so `sendUpdates=none` means they get *nothing*. Exchange's Calendar Attendant *does* auto-add (as **Tentative**), but only from a message already delivered to the Inbox. So Google-hosted contacts look fine while M365 clients silently get nothing — check MX before theorizing (`northwind.example` → M365, `ironside.example` → Proofpoint).

**Diagnosing delivery without an admin console:** a `responseStatus` other than `needsAction` is *proof of receipt* — and it is conclusive specifically for **non-Google** recipients, who have no silent-add path. A reply on an `Invitation:`/`Updated invitation:` thread is proof too. Google-sent invitations always carry a `text/calendar` part plus an `invite.ics` attachment; absence of a bounce is weak evidence, presence is strong.

**The fix is a ledger, because the fact cannot be read back — only recorded.** `invite_notifications` (v57), written in `gcal_invite::deliver_approved_invite` and `commands::calendar::send_google_calendar_invites`, **strictly after the 2xx** so a row means "Google took the send" and nothing weaker. It exists because `clear_invite_approval` destroys the approval snapshot on every terminal path — right for single-use approval, but it shredded the only receipt at the moment it was earned.

- **Side table on the remote triple, NO foreign keys** — `zoom_meetings`' shape for `zoom_meetings`' reasons (#44): `upsert_remote_event` is driven wholly by Google's payload so a column would be nulled every sync tick, and four paths DELETE from `gcal_events`. A ledger row whose event was swept is a tombstone: the event going away does not unsend mail. Tripwire test asserts it is not a `gcal_events` column.
- **No backfill, deliberately** (#37's situation — unreconstructible). Everything historical reads `unknown`. On the real mailbox that is 100 `responded` / 39 `unknown`, which is honest *and* immediately useful, because `responded` is the strong signal.
- **`derive` is the one matcher** (#36) — four states resolved in Rust and rendered verbatim; the TS side decides nothing. `event_has_ledger` is what separates `UNSENT` from `UNKNOWN`: if we have ever notified anyone for this event, absence from the ledger is evidence rather than ignorance. `pending_notify` alone also yields `UNSENT`.
- **`notified_by_event` degrades to empty when the table is missing** — the MCP migrates non-fatally, and this read hangs off `list_unified_by_range`, so erroring would refuse to draw the whole calendar. Empty → `unknown`, which is the honest rendering anyway.
- **⚠ Never label `sent` as "received".** We know Google accepted the request; we know nothing about bounces, spam filing, or tenant quarantine. Only `responded` is receipt. Pinned by `attendeeDelivery.test.ts` — a label containing "received" on any other state fails the build. `unknown` is likewise never counted in the "not notified" badge; most of the calendar is unknown and a badge that cries wolf gets ignored (#34's reasoning).

**Verify against real mail, never fixtures alone**: `cargo run -p cxmail-db --example invite_delivery_probe -- <copy.db> [filter]` on a `.backup` copy (#12).

**Cancelling and declining were the same bug.** `delete_event` passed `SendUpdates::None`, so a meeting vanished from the organizer's calendar and stayed on everyone else's — worse than the create path, because the guests who *were* told are exactly the ones left holding a dead slot. `cancellation_should_notify` now decides: **notify unless the ledger positively says nobody was ever told**, i.e. only an event where every non-self guest is `UNSENT` stays silent. **`UNKNOWN` counts as "might know"** — most of a pre-v57 calendar is unknown, and silence there just recreates the bug; the self attendee is excluded, or a solo event would notify and a real one could be silenced. Mutation-tested (always-silent and unknown-means-never-told both fail). The 403 branch — where "delete" degrades to *declining as an attendee* — takes `All` **unconditionally** and not the ledger rule: there we are not the organizer, they demonstrably know about their own event, and telling them we are not coming is the entire content of the action. The dormant `local_deleted` arm in `gcal_sync` got the same rule so it is not a landmine for whoever resurrects it.

**A cancellation is outbound mail, so it is gated like all the other outbound mail.** The first cut of the delete fix shipped `SendUpdates::All` un-gated — one click, cancellation notices to a client's whole team, no undo — which is out of step with every other path in the app (compose has undo-send, invitations have the approval card). `delete_google_calendar_event` now takes an **explicit, required `notify: bool`**; there is no implicit default that quietly emails anyone. The UI arms an inline `role="alertdialog"` in the event panel naming exactly who Google will mail (**all non-self attendees, not just the ledgered ones** — understating the blast radius is the one direction this must not be wrong in) and offers *Keep event* / *Delete and notify* / *Delete without notifying*, with the silent option always a deliberate second choice. When `cancellation_notifies` is false the gate still confirms the destructive act but **must not claim guests are being told**.

**`cancellation_send_updates(requested, delivery)` = `requested && cancellation_should_notify(delivery)`, and the asymmetry is the invariant**: a caller can always choose silence and can never force mail the rule would not send. That is what makes honouring a frontend boolean compatible with architecture invariant #6 — it can only ever *reduce* the blast radius. Both directions test-pinned; `cancellation_notifies` rides on `UnifiedCalendarEvent` so the dialog never re-derives the rule.

Still open, deliberately: no bounce correlation (a DSN naming an attendee is the only true *did-not-receive* signal available).

Pattern: `db/invite_notifications.rs`, `db/schema.rs::migrate_v57_invite_notifications`, `db/gcal.rs::list_unified_by_range`, `email/gcal_invite.rs`, `commands/calendar.rs`, `src/lib/attendeeDelivery.ts`, `CalendarView.tsx`. Related: #21, #37, #44, #36, #34.


---

---

### 51. Window dragging is an ATTRIBUTE *and* an ACL grant — and each layer fails invisibly on its own

**Symptom:** the window only moves by grabbing the extreme top edge — "the thin
bezel." Everywhere inside the webview a drag does nothing. `titleBarStyle:
"Overlay"` means the webview covers the titlebar, so that bezel is the sliver of
native titlebar it does not cover.

**Layer 1 — the ACL, which is the one that is invisible.** Tauri's injected
`drag.js` ends at `invoke('plugin:window|start_dragging')`, an ordinary
ACL-gated command. `core:window:default` grants 27 **read-only getters**
(`allow-inner-size`, `allow-is-focused`, …) plus `allow-internal-toggle-maximize`
— *not* `allow-start-dragging`. So correct markup produces a drag that is
**authorized away**: nothing in the DOM is wrong, nothing is logged, the window
just refuses to move. `capabilities/default.json` must list
`core:window:allow-start-dragging` explicitly. Two tells that you are on this
layer and not the markup one: double-click-to-maximize on the same strip
**works** while dragging does not (`internal_toggle_maximize` is granted by
default and `start_dragging` is not), and nothing in the DOM looks wrong because
nothing in the DOM *is* wrong.

**Layer 2 — the markup, where `-webkit-app-region: drag` is a decoy.** That is an
**Electron** property; WKWebView does not implement it and a webview cannot move
its host `NSWindow` on its own, so the declaration is silently inert. Only
`data-tauri-drag-region` counts, and **bare is not `deep`**: `isDragRegion` does
`return el === composedPath[0]` for a bare attribute, so only a *direct* hit on
that exact element drags and every child is a dead spot — which reads as
"dragging works in some places," not as a bug. `"deep"` accepts any descendant;
`"false"` blocks a subtree.

**Buttons and inputs need no opt-out.** `isClickableElement` already excludes
`A/BUTTON/INPUT/SELECT/TEXTAREA/LABEL/SUMMARY`, `contenteditable`, a real
`tabindex`, and the interactive ARIA roles. Do not invent a `.no-drag` class.

**Verify in two places — each check catches only its own layer.** For the ACL,
read `src-tauri/gen/schemas/capabilities.json` **after a build** and confirm
`start-dragging` resolved into it; the source capability file listing it is not
proof the build picked it up, and capabilities are compiled by `tauri-build`, so
a frontend hot-reload will never see the change. For the markup, port
`isDragRegion` out of
`~/.cargo/registry/src/*/tauri-<ver>/src/window/scripts/drag.js` and probe real
rendered nodes — asserting the attribute is merely *present* proves nothing,
since bare and `deep` look identical in the DOM. Both are pinned and
mutation-tested by `windowDrag.test.tsx` (flip `deep`→bare and exactly one test
fails; drop the grant and exactly the other one does).

`user-select: none` on the region is **not** what makes this work — Tauri calls
`preventDefault()` when it accepts the drag. It only suppresses the I-beam and
any selection flicker in the frame before that; form fields opt back in because
`user-select` inherits.

**A drag region SWALLOWS `mousedown` for the whole app.** `drag.js` calls
`stopImmediatePropagation()` (citing tauri#2549) from a **bubble**-phase listener
on `document`, so any bubble listener on `window` — or on `document`, registered
later — never sees a click that landed on the drag strip. Any click-outside
handler that must dismiss on a click in the titlebar has to register in the
**capture** phase (`addEventListener("mousedown", fn, true)`). Same
`preventDefault()` means such a click does **not** move focus, so a popover that
reopens on `onFocus` goes dead after one dismissal. Not currently a problem in
CXMail — the drag strip is a 32px band with one span and one button — but it is
the trap that arrives the moment a region grows.

Pattern: `src-tauri/capabilities/default.json`, `AppLayout.tsx` (titlebar strip),
`src/styles/globals.css`, `src/components/layout/__tests__/windowDrag.test.tsx`.
Ported from cxtasks' gotcha #15, where the same two layers broke together.

---

### 52. The translucent window is TWO halves, and each is invisible without the other

CXMail's window is glass: a transparent Tauri window with a desktop blur behind
the webview (a CGS Gaussian since 2026-09-10, #59 — an `NSVisualEffectView`
before that), tinted by alpha-aware pane tokens.
Ported from cxtasks (its gotcha #22), which is where the material and the
failure modes were argued out. What follows is what is *specific to cxmail*.

**Three things are required and any one missing looks like one of the other
two is broken.** (1) `"transparent": true` on the window in `tauri.conf.json`
(needs `macOSPrivateApi`, already on). (2) a blur behind the webview —
`glass_macos::enable`, armed by the frontend after first paint (#59), with
`vibrancy_macos` (`UnderWindowBackground`, `Active`, radius `None`) as its
fallback. (3) `body
{ background: transparent }`; an opaque body paints straight over the effect
view and the blur is present, correct, and completely invisible.
**`backdrop-filter` is not an option and never was** — it blurs an element's
*backdrop root*, which for anything in the page is the page, so on a
transparent window it blurs a transparent nothing and the desktop shows through
perfectly sharp. Same class of mistake as #51's `-webkit-app-region`: an
incantation belonging to a different layer.

**The root element must paint NOTHING.** `AppLayout`'s root carried `bg-base`,
and one opaque layer across the window means every pane composites over *that*
rather than over the blur — so turning the sidebar's alpha down moves it
toward the content's colour instead of toward the desktop, and "much more
transparent" renders as "slightly less contrast". The numbers are all correct
and the setting reads as doing nothing. Each pane paints its own background;
the titlebar strip, `Sidebar`, the message-list column, the reading-pane
column, the full-screen-calendar column and `StatusBar` tile the window between
them. **Check this first whenever a transparency change has no visible effect.**

**Which tokens carry alpha is a decision, not a detail.** `base` · `sidebar` ·
`surface` are panes and are alpha-aware. `base-solid` · `surface-solid` ·
`elevated` · `input` are opaque, because they back things that FLOAT OVER the
window: a Radix menu or dialog portals to the document root, so its backdrop is
the transparent window itself and a translucent one is text on raw wallpaper —
and even a non-portaled modal stacks over an already-translucent pane, which is
two sheets of glass. 25 call sites were moved across (every modal, menu,
popover, floating window and the docked composer). **The two `-solid` tokens are
the same triplets as their alpha-aware twins**, which is what keeps the ported
look byte-identical *and* makes an alpha-aware child inside a solid parent a
no-op — `AccountSetup` renders both full-screen and inside the add-account
modal and needed no edit. `SettingsDialog` is opaque for the same reason, and
deliberately does not dim the app behind it: every control in it changes how the
window looks, so a scrim would black out the exact surface being adjusted.
A new `bg-*` utility naming an undeclared token emits **no rule at all**;
`tailwindTokens.test.ts` has `bg` in its always-fails tier, so both new tokens
are covered.

**⚠ cxtasks' sidebar curve is WRONG for this palette, and copying it inverts the
bar into a trench.** cxtasks boosts the slider ×1.9 before applying it to the
sidebar and gives the bar its own low floor. That works because its bar is 42
against a content of 24 — 1.75× lighter. CXMail's is 35 against 28, only 1.25×,
and at that ratio the boosted bar composites *darker* than the content it sits
on (measured by the regression test at t=0.25 over black: bar 20.0, content
22.8). The constraint is `35·a_bar > 28·a_pane`, i.e. the sidebar's alpha may
lead the pane's by less than 0.2; `SIDEBAR_LEAD` is 0.12. Expressed as a
fraction OF the pane alpha, never as a scaled result — `pane * 0.5` leaves the
sidebar half-transparent with the slider at zero, so the opaque end would not be
opaque. Light inverts the ordering (its bar is painted *darker* than its
content) and is far less constrained. `windowTransparency.test.ts` composites
the real palette — read out of `globals.css`, not duplicated — over black and
white backdrops at eight slider positions, so moving `--bg-sidebar` closer to
`--bg-primary` fails the test that says the constant needs revisiting.

**`Math.min(1, Math.max(0, NaN))` is `NaN`, and a NaN alpha paints a pane fully
transparent** — the window vanishes and the text floats on the desktop. cxtasks
guards this where it reads localStorage; cxmail's read site is zustand
`persist` rehydration, which validates nothing, so a corrupt or hand-edited
`cxmail-ui` entry lands straight in the store. `clampTransparency` guards the
choke point both entry paths pass through, and falls back to the default rather
than 0 so corrupt storage behaves like no storage. Found by the test, not by
review.

**The appearance pin was load-bearing while the blur was a material, and still
is for the fallback.** `NSVisualEffectMaterial` renders against the *window's*
`NSAppearance`, which follows the SYSTEM setting unless something pins it — so a
dark UI on a light-mode Mac tinted its glass light and read washed out. The CGS
blur (#59) carries no tint and reads no appearance, so on the primary path the
pin is back to being about native chrome; it still governs the `vibrancy_macos`
fallback. `appearance_macos::set_appearance` pins it in `.setup()`, before
anything could install that fallback, and `applyTheme` re-asserts it through
`set_native_appearance`. Main-thread-only (#20). Free side-effect: right-click
NSMenus and `<select>` popups follow CXMail's theme, not the system's.

**The blur is no longer installed once and left alone** — that was the
material's contract, and it made the slider pure CSS. `glass_macos` takes a
radius and the radius is the slider's second output (#59), so `applyWindow` now
reaches AppKit too — but only through `applyGlass`, which compares against the
last values sent (a full drag is at most ~38 IPC calls), and the CSS write still
comes first and is never gated on the native one: a failed call can leave the
blur stale, never strand the window half-transparent. Only the fallback keeps
the install-once rule (`install_fallback_once`), because
`vibrancy_macos::apply` inserts another effect view on every call.

**The pre-hydrate fallback is not optional.** `:root { --alpha-*: 1 }` in
`globals.css` — without it the alphas are invalid at computed-value time before
`applyTheme` runs and the first frame is bare glass with floating
text. `main.tsx` calls `applyTheme` before `createRoot` (zustand `persist`
rehydrates synchronously during import, so `getState()` is already correct
there), which is what stops the window snapping from solid to glass in front of
you at every launch.

**Verifying this needs the real window.** Playwright renders the webview only,
so a transparent window screenshots as flat black there and proves nothing — a
signed dev window plus `screencapture` is the only honest check, and it means
quitting the installed app first (#12: two instances share the SQLite file).
Capture a REGION (`screencapture -R` with the bounds from
`CGWindowListCopyWindowInfo`), never `-l <windowid>`: a window capture renders the
window alone, without the desktop the WindowServer composites behind it, so it
cannot show the blur either. The recipe and the measured numbers are in #59.

Pattern: `src-tauri/src/glass_macos.rs` (+ `vibrancy_macos.rs` fallback),
`src-tauri/src/appearance_macos.rs`, `commands/system.rs::set_native_appearance`,
`lib.rs` `.setup()`, `tauri.conf.json`, `src/styles/globals.css`,
`src/stores/uiStore.ts::{transparencyToAlphas, applyWindow, applyGlass, applyTheme}`,
`SettingsDialog.tsx`, `AppLayout.tsx`. Related: #59, #20, #12, #51, #19.


---

### 53. `selectedAccountId` is not "an account is selected" — it is filled in every special view and inbox group

**Symptom**: the sidebar account row you clicked never showed as selected (it had no
selected state at all — the grey was `hover:`, which leaves with the pointer and is
near-invisible on the dark sidebar, so dark mode gave no feedback whatsoever).

The obvious fix — `selectedAccountId === account.id` — lights up the FIRST account
under Needs You and under every inbox group. `Sidebar`'s auto-select effect runs
`setSelectedAccount(accounts[0].id)` whenever `!isUnifiedInbox && !selectedAccountId`,
and `setSpecialView`/`setSelectedGroup` produce exactly that state, so the id is
populated in views that have nothing to do with an account. "The list is showing this
account" needs all four terms:
`!isUnifiedInbox && specialView === null && selectedGroupId === null && selectedAccountId === account.id`.
`FolderTree.isThisAccount` reads the id alone and has the same latent leak (harmless
today only because its folder rows also require `selectedFolder`, which those views null).

Selected rows carry `aria-current="true"`, which is what the test queries — the
tint is `${account.color}26`, FolderTree's treatment, so an account and its folders
share one rule. Pinned + mutation-tested by `SidebarAccountSelection.test.tsx` (each
dropped term fails exactly one test). Related: #32 (the reading pane made the mirror
mistake — trusting the view over the message's own account).

---

## Database (continued)

### 54. "All accounts" was never a query over `accounts` — so hiding one is a predicate in every aggregate, not a filter in one place

`accounts.hidden_from_aggregates` (v58) keeps an account out of All Inboxes, the
account folders, inbox groups, Needs You, nudges, unscoped search, the category
counts and the dock badge. No aggregated query ever read the `accounts` table —
"all accounts" was literally "every row in `messages`" — so there was no one place
to enforce it. Each aggregate embeds `db::accounts::visible_in_aggregates_sql(expr)`
(gotcha #36: one matcher), pinned by a source-reading tripwire in `accounts.rs`
that fails if any of the six modules spells the column itself or stops calling the
helper. Rules that are easy to undo:

- **`NOT EXISTS`, so absence fails open.** A message whose account row is missing
  stays visible: hidden is the explicit state. Corollary: **any fixture that
  exercises one of these queries now needs an `accounts` table** — `search.rs`'s
  and `inbox_groups.rs`'s hand-built schemas each gained a two-column one, or
  28 existing search tests die with `no such table: accounts`.
- **`search` applies the rule only when `account_ids` is `None`; every list/count
  query applies it always.** `list_all_inboxes(Some(ids))` only ever means an
  account folder — still an aggregate — so a hidden member is skipped even when
  named. `search` also serves explicit contexts (the bar inside the hidden account,
  an MCP call naming it), and naming the account is the one way its mail is
  reachable by search. `server_search` mirrors the same split for its IMAP fan-out.
- **Do NOT pre-filter the ids in `MessageList`/`CategoryTabs`.** Both collapse
  `ids.length > 0 ? ids : undefined`, and `undefined` means *every* account — a
  folder whose members are all hidden would flip into All Inboxes. The DB applies
  the predicate to `Some(ids)`, so those stay as they were. The one place the
  frontend does derive ids, `lib/searchScope.ts`, returns `[]` as `[]` for that
  case (test-pinned) and the backend reads `Some([])` as nothing.
- **The committed search scope is stored (`mailStore.searchScopeIds`), never
  re-derived.** Inside a hidden account the scope is that account; a chip removal
  or load-more that recomputed it from the account folder would widen to
  visible-only and the results would vanish.
- **Two sums never touch the DB** — "All Inboxes" and the account-folder header in
  `Sidebar.tsx` add up `foldersByAccount` — and each needed its own filter.
- **`needs_you::load_correspondence` is deliberately untouched.** It is the global
  "who has replied to me" set and also feeds single-account classification; a
  hidden account's own inbox must classify exactly as before.
- **The badge is only refreshed on new mail / mark-read**, so the toggle command
  calls `notify::update_badge` itself, with the DB guard dropped first.
- Found on the way: `server_search`'s final local re-run used `filters`, whose
  `account_ids` the frontend never sets (scope arrives as a separate argument), so
  a scoped server search merged unscoped rows. Folded in at the top of the command.

Notifications stay on `notify_enabled`, on purpose — hiding and muting are
different questions. Probe the real effect on a `.backup` copy (#12) with
`cargo run -p cxmail-db --example hidden_scope_probe -- <copy.db> <email>`.
Pattern: `db/accounts.rs::{visible_in_aggregates_sql, list_hidden}`,
`db/schema.rs::migrate_v58_account_hidden_from_aggregates`, `db/{messages,
categories, inbox_groups, needs_you, nudges, search}.rs`,
`commands/accounts.rs::set_account_hidden_from_aggregates`,
`commands/messages.rs::server_search`, `mcp/server.rs::hidden_accounts_note`,
`src/lib/searchScope.ts`, `Sidebar.tsx`, `AccountBadge.tsx`. Related: #36, #53, #32.

---

## Frontend (continued)

### 55. Opening a draft is an IMAP round trip with no identity — every click that lands during it opened another compose window

**Symptom**: a draft is slow to open (body + attachment bytes over IMAP), the user
sees nothing and clicks again, and five copies of the same draft cascade across the
screen — each with its own autosave. A double-click alone is THREE opens (click,
click, dblclick), so this was the common path, not an edge.

Two things were missing and either alone leaves the bug alive:
1. **The store had no notion of "this thing is already open."** `openWindow` always
   appended. Now a window may carry a `key` (`draftWindowKey` / `emailWindowKey`,
   both on the `(accountId, folder, uid)` triple), and `openWindow` with a key that
   is already open restores + raises that window and returns its id.
2. **The opener was two copies with no in-flight guard** — `MessageList` and
   `MessageListItem` each had an `openDraftForEdit` (same shape as the
   `buildDraftComposeProps` duplication in #39). `lib/draftCompose.ts::openDraftForEdit`
   is the one opener: already open → focus, no fetch; already opening → join the
   pending promise; else fetch and open *with the key*. The in-flight slot is released
   in `finally`, so a failed open does not poison the retry.

**The key must FOLLOW the draft, or the fix lasts until the first autosave.**
`edit_draft` expunges and re-appends, so the UID changes on every save; a window keyed
at open time goes stale and the next click on the Drafts row opens a second copy.
`ComposeModal` reports `draftContextState` through `onDraftRefChange`, and
`FloatingWindowManager` re-keys the window (`setWindowKey`). Fresh composes gain an
identity on their first save the same way.

**The re-key must be a true no-op when unchanged.** The effect runs on the window
manager's renders, the manager re-renders on every store write (each drag frame), and a
write of an identical key would re-render the manager — a loop. `setWindowKey` returns
the *same state object* when nothing changes, which zustand 5 treats as no update
(`Object.is` check in `setState`); the callback is read through a ref in `ComposeModal`
so its inline identity is not a dependency. Both pinned; mutation-tested
(`windowStore.test.ts`, `openDraft.test.ts` — drop either guard and the named tests fail).

Known stale-row hole, pre-existing: between a save and the `sync` that refreshes the
Drafts list, the list still shows the expunged UID, and clicking *that* row is a
different key. Fixing the list's staleness is the fix there, not a second key.

Pattern: `src/stores/windowStore.ts`, `src/lib/draftCompose.ts::openDraftForEdit`,
`ComposeModal.tsx::onDraftRefChange`, `FloatingWindowManager.tsx`. Related: #39, #25.

---

## Backend (continued)

### 56. Reopening a draft re-downloaded its attachments from IMAP every time — and waited 5 s for a LOGOUT on top

**Symptom**: double-clicking a draft takes many seconds to open a compose window. On
2026-08-25 it was ~25 s for an 8.8 MB draft (a 6.4 MB video) on `chris@cxventures.io`.
The log splits it exactly: body **cache HIT in 1 s**; then `fetch_outgoing_attachments`
opened a fresh IMAP connection (one connect **timed out at 15 s**, gotcha #46's
account-specific Google trouble), FETCHed the **entire message** (`fetch_body: uid=739
returned 8800112 bytes`, ~4 s), then **awaited `imap::disconnect`** — a LOGOUT that timed
out at 5 s in every open that day — before returning.

**The regression is `caf42cc` (2026-06-18), gotcha #25's fix.** Reopening a draft must
reload its real attachments (the compose window rebuilds the MIME on every save and drops
any attachment it lacks the bytes for), and that reload went to IMAP. Invisible on a 40 KB
PDF; brutal on a video. Before it, a draft open was the body cache hit alone.

**The bytes were never far away.** `persist_local_draft` receives the full raw MIME at save
time — from the app's `save_draft`/`edit_draft` AND both MCP draft paths — and kept only the
body and attachment *metadata*. v59 `draft_attachment_blobs` keeps the non-inline parts too
(`draft_local::draft_blobs_from_raw`, the same inline filter the IMAP path applies), keyed on
the draft's triple with an `ON DELETE CASCADE` FK to `messages` — the eviction strategy,
since the UID moves on every autosave and `edit_draft` deletes the old row via `delete_uids`.
`fetch_outgoing_attachments` reads it first; the IMAP path is the fallback for drafts saved
before v59 or by another client, and it **back-fills the cache**, so even those are local
from the second open on. No backfill migration: same situation as #37, and the fallback
does it lazily. Over 50 MB per draft it declines to cache (`MAX_CACHED_BYTES_PER_DRAFT`).

**Rules that are easy to undo:**
- **Empty means "not cached", never "no attachments".** The reader is only called when the
  metadata says the draft has real attachments, so absence is unambiguous — keep it that way;
  a caller that reads blobs to *decide whether* there are attachments will conclude "none"
  for every pre-v59 draft.
- **`replace` is delete-then-insert in one transaction**, so a re-persist (sync raced the
  save, or a fallback open) cannot accumulate duplicates. `insert_attachments` does the same.
- **The LOGOUT is `tokio::spawn`ed after the FETCH, not awaited.** The bytes are the answer;
  the logout is hygiene. Awaiting it put its 5 s timeout on the critical path of every open.
  The other ~20 `let _ = imap::disconnect(...).await` sites in `commands/` are the same shape —
  `fetch_message_body`'s IMAP-miss path included — and were deliberately left alone here
  (scope), but they are where the next "why does opening X take 5 s longer than it should"
  will be found.

**Probe a schema change against a copy of the real mailbox** (#21b/#45):
`sqlite3 live.db ".backup probe.db"` → `cargo run -p cxmail-db --example migrate_probe -- probe.db`
→ confirm `schema_version` and that a re-run is a no-op. Never the live file (#12).

Pattern: `db/draft_blobs.rs`, `db/schema.rs::migrate_v59_draft_attachment_blobs`,
`email/draft_local.rs::{persist_local_draft, draft_blobs_from_raw}`,
`commands/compose.rs::fetch_outgoing_attachments`. Related: #25, #46, #55, #37.

---

### 57. A stale draft UID silently FORKED the draft — and MCP `edit_draft` expunged before it appended

**Symptom**: an agent edits a draft it created and Drafts ends up with two copies; the tool result says "Draft UID 739 replaced with UID 741"; the open compose window never reloads; the agent starts probing nearby UIDs to find "the current one".

**Two independent defects, fixed 2026-08-25.**
1. **Delete-then-append.** MCP `edit_draft` marked `\Deleted`, EXPUNGEd, and only then built + APPENDed the replacement, so any failure in between (attachment read, MIME build, APPEND, the network) destroyed the draft outright. #30/#34/#47 had been *working around* that window ("nothing fallible between the expunge and the re-APPEND") rather than closing it; the app's `commands::compose::edit_draft` always appended first. Both now do APPEND → resolve new UID → retire old. A failure past the APPEND leaves a duplicate — visible, recoverable, and **reported in the tool result** (`dup_note`), never swallowed; the local row is evicted only if the server actually let the old UID go.
2. **A stale UID is not an error at the IMAP layer.** RFC 9051 §6.4.9: UID commands skip nonexistent UIDs, so `UID STORE 739 +FLAGS (\Deleted)` on an already-expunged 739 is answered OK (#38's "a nonexistent UID is not a usable probe", seen from the other side). The old handler "deleted" nothing and APPENDed a second draft. The composer's `draft-updated` listener matches `old_uid === its current uid`, so it was never told. UIDs go stale constantly — every save mints a new one, the compose window's autosave included, and the app never tells the MCP about UIDs it mints.

**The preflight is a defense, not exclusion.** `edit_draft` runs `imap::uid_exists` (SELECT + `UID SEARCH UID n`) before the APPEND and returns `stale_draft_error` — nothing written; the message says so and names the remedy. The local `messages` row is *advisory*: its absence is the signature of an app-side re-save (`compose::edit_draft` evicts it), so the message says that; the server decides. It guards against the world outside CXMail's writers (a delete in another client, a mailbox that changed generation). The TOCTOU between check and APPEND is closed by v60, below.

**v60 — the draft has an identity, and writers claim it (same day).** `db::drafts` = `drafts(draft_id, account_id, folder_name, uidvalidity, current_uid, claim_token, claim_expires_at)`, **no FK to `messages`** (the UID row is evicted on every save; the draft outlives all its revisions — test-pinned). The id travels in the MIME as `X-CXMail-Draft-Id` (`draft_local::DRAFT_ID_HEADER`), written by every writer (`compose::{save_draft, edit_draft}` via `build_draft_raw`, MCP `compose_draft`/`edit_draft`) and read back by the folder sync (added to the `HEADER.FIELDS` FETCH subset — one name, cheap; `sync::apply_header_batch` calls `drafts::link`). Rules that are easy to undo:
- **`link` is forward-only within a `(uidvalidity, folder)`**: UIDs ascend inside a generation, so the highest UID ever seen IS the latest revision, and every writer and the sync can feed it in any order — including the window where both revisions are on the server. A new generation or folder resets the pointer (a UID means nothing outside the mailbox generation that issued it — which is why `uidvalidity` is a column). Test-pinned: the old revision arriving after the new one does not regress.
- **`claim` is one UPDATE — a compare-and-set on `current_uid`** plus an expiry check on the token. Two writers naming the same revision cannot both win; the loser gets `Moved{current_uid}` or `Busy{expires_at}` *before any IMAP work*. Both writers claim: MCP `edit_draft` (tripwire pins `db::drafts::claim(` before `imap::connect_for_account(`) and the app's `compose::edit_draft` (resolves the composer's `uid` through `find_by_uid`, so the frontend needed no change beyond the optional `draft_id` on `SavedDraftRef`). A short expiring token (`CLAIM_TTL_SECS` = 120), never a SQLite transaction held across IMAP awaits (#11). `advance` moves the pointer and clears only *our* token; a writer that outlived an expired claim cannot drag the pointer backwards. Every failure past the claim releases it.
- **`expected_uid` is the revision token** — no separate `revision` or content hash. `edit_draft(draft_id, expected_uid?)`: a mismatch returns `draft_moved_error` carrying the current revision's subject/recipients from the local cache (honest about "not synced yet"); omitting it replaces whatever is current. `uid` stays as a DEPRECATED path for one release: resolved through `find_by_uid` (claimed like any other), or, for a pre-v60 draft with no row, the IMAP preflight alone — and the reply carries the draft_id to use from then on. `resolve_edit_target` rejects `draft_id`+`uid` together and `expected_uid` with a bare `uid`.
- **On the app side a conflict is a `Draft conflict:` error and nothing else** — the composer already keeps its dirty content on a failed save (the saved-hash only advances on success), adopts the live UID from the bridge's `draft-updated` event, and its next autosave lands on that revision. The "Reload / Keep mine" banner stays the arbiter for unsaved human edits. Cut on purpose: a bidirectional socket routing edits through the live TipTap document (two edit paths for one operation).
- **The handler future must stay `Send`** (#30's note): the IMAP section is an `async` block that captures no `&Connection` — `local_row_present` and the signature are read synchronously before it.
- **Not the RFC Message-ID**: both writers mint a fresh one per save because `find_uid_by_message_id` is how the new UID is discovered (async-imap 0.10.4's `append` reads the tagged line and discards APPENDUID), and Gmail dedupes APPENDs by Message-ID. Capturing APPENDUID (raw APPEND) would remove the 3×1 s search-and-retry — still deferred. A draft edited in Gmail's web UI loses the header and degrades to `unknown_draft_error` ("address it by uid once"), never to a wrong match. No backfill: pre-v60 drafts get a row lazily on their next save; the migration is `CREATE TABLE IF NOT EXISTS` (probed on a `.backup` of the real mailbox, v59→v60, re-run no-op). Rows for drafts that were sent or deleted are left as tombstones — harmless, since the preflight refuses a UID that is gone — and no sweep is built.

**`UID EXPUNGE`, not `EXPUNGE`.** A mailbox-wide EXPUNGE after marking one UID also purges everything *another client* had marked `\Deleted` in that folder. `imap::uid_expunge` sends `UID EXPUNGE n` through `run_command_and_check_ok` — NOT the library's `uid_expunge` stream, which would swallow the tagged BAD (#38) — and falls back to mailbox-wide only on BAD (the answer of a server without UIDPLUS); a NO propagates. Applied to the two draft-replace sites only. The other `imap::expunge` callers (delete paths, the MOVE fallback) look the same by grep and were **not** read or changed — verify each before touching.

**Providers, probed 2026-08-25**: none of Gmail / Outlook / iCloud advertise `REPLACE` (RFC 8508), so a REPLACE branch is not worth building; Outlook advertises UIDPLUS pre-auth, Gmail post-auth.

Tripwires (source-reading, mutation-tested): `edit_draft_checks_the_old_uid_then_appends_then_deletes` (claim < connect, preflight < APPEND < delete), `edit_draft_expunges_only_the_old_uid`; contracts: `edit_draft_target_is_draft_id_xor_uid`, `conflict_messages_say_nothing_changed_and_name_the_next_step`, `stale_draft_error_says_nothing_changed_and_names_the_remedy`; `db/drafts.rs` tests (forward-only link, CAS, release, expiry, reclaim); `sync::apply_header_batch_links_drafts_forward_only`. Verified live over stdio against the rebuilt binary: compose → edit by draft_id → the stale `expected_uid` and the stale `uid` both refused with nothing written → deprecated `uid` path succeeds and names the draft_id → edit with no `expected_uid` replaces the current revision → exactly one draft on the server throughout.

Pattern: `db/drafts.rs`, `db/schema.rs::migrate_v60_drafts`, `mcp/server.rs::{edit_draft, resolve_edit_target, stale_draft_error, draft_moved_error}`, `email/imap.rs::{uid_exists, uid_expunge}`, `email/sync.rs::apply_header_batch`, `email/draft_local.rs::DRAFT_ID_HEADER`, `commands/compose.rs::{save_draft, edit_draft, replace_draft_revision}`. Related: #30, #38, #55, #11.

---

### 58. MCP draft prose never came from a model — the writer IS the calling agent, so "use Gemini for writing" is a new authorship path, not a provider swap

**The question that started it**: can Gemini (on the Antigravity subscription, headless) write emails in place of Claude? The instinctive answer — add a provider to `inference.rs` — is wrong for the path that matters: `compose_draft`/`edit_draft` take `subject`/`body` as parameters and the MCP's ONE inference call site is voice-profile extraction. The prose is authored by the calling agent before the tool is ever invoked; there is no model on that path to swap. `inference.rs` only drives the in-app buttons (Reply with AI, Adjust Tone, proofread).

**The fix is an `instruction` parameter on both draft tools** (`body` XOR `instruction`): the MCP itself assembles the voice context, shells out to Antigravity's `agy` CLI headless, and feeds the generated prose through the UNCHANGED pipeline — `enforce_dash_rule`, advisories, quote, signature, APPEND — so every boundary guard that catches agent-authored text catches this too (#47's whole point). Default model `gemini-3.7-flash-high` (`external_writer::DEFAULT_WRITER_MODEL`); any `agy models` id works, `--effort` maps to the (High)/(Medium)/(Low) suffixes.

**Rules that are easy to undo:**
- **The containment argv IS the injection posture, and it is test-pinned**: `--mode plan` (read-only), an EMPTY per-call temp cwd (plan mode can still read — give it nothing), `--disable-slash-commands` (mail content must not expand skills), `--output-format json` (a typed envelope, so a warning line on stdout can never become an email body), stdin closed (a permission prompt fails instead of hanging), never `--dangerously-skip-permissions`. `args_pin_plan_mode_and_disabled_slash_commands` fails if any is dropped.
- **`resolve_authored_source` runs before ANY other work** (#30's ordering) and every refusal says "nothing was written" and names the remedy (#57's convention). `layout`/`is_html=true` are REFUSED with `instruction`, not ignored — silently dropping a layout is how a designed email ships flattened (#34, from the other direction). `writer_model` without `instruction` is refused too: it is confusion, not a no-op.
- **One voice assembler** (#36): `build_voice_ctx_for_recipient` moved from `commands/ai.rs` into `email::ai` so the writer and the in-app AI get IDENTICAL context. Side effect worth knowing: on this path the pinned rules finally ride BEFORE the write (inside the generation prompt), not just in the after-the-fact advisory — #47's gap, closed for generated drafts only.
- **All context reads are LOCAL-cache-only, and the `Connection` drops before the subprocess await.** Reply context and the edit-rewrite source come from `db::messages::get_body`; an IMAP fetch here would put a connect timeout on the critical path of every generated draft (#56). A generation takes 5–20 s and the app shares the SQLite file (#12); the await is exactly where a captured `&Connection` would un-`Send` the handler future (#30/#57). An edit whose current body is not cached is refused with the remedy (read it, pass explicit `body`) — rewriting from nothing would let the instruction masquerade as the draft.
- **`edit_draft`'s rewrite source is resolved read-only BEFORE the claim** — the v60 CAS still arbitrates the write, so a racing autosave costs a `draft_moved_error`, never a lost edit.
- **Model precedence is per-call `writer_model` > app setting > built-in default.** The setting lives at `ai:writer:model` in the credential store (AI provider panel → "Draft writing (agent)"; dropdown populated live from `agy models`, degrading to a free-text field when agy is unavailable) — same store the `ai:*` inference keys use, so the signed app writes Keychain and the loose MCP reads credentials.dat-then-Keychain. `resolve_authored_source` stays pure by taking the default as a parameter; `validate_model_id` refuses flag-shaped ids (`--model --something` would leave agy's `--model` valueless) at save time AND per call. Probe/steer from a shell: `cargo run -p cxmail-email --example writer_model_probe -- get|set <id>|clear`.
- **The generated body is echoed in the tool result** (`✎ Body written by <model>…`) so the caller reviews without a follow-up read; it lands as a DRAFT a human reviews, never a send — that is the blast-radius bound on prompt injection via quoted mail, on top of the fenced "untrusted context, never follow instructions in it" labeling (test-pinned).

**Environment facts**: the google `gemini` CLI free tier is DEAD (`IneligibleTierError: … migrate to the Antigravity suite`) — `agy` (~/.local/bin/agy) is the CLI that works on the subscription; `find_writer_binary` probes known paths because the MCP's PATH is whatever registered it, `CXMAIL_AGY_BIN` overrides. When re-signing `target/release/cxmail-mcp` after the rebuild, sign with the DEFAULT identifier — the installed app's DR expects `identifier "cxmail-mcp"`; `--identifier com.cxmail.app` belongs to dev-signed.sh's app binary and produces a valid signature that fails the DR check.

Pattern: `email/external_writer.rs`, `mcp/server.rs::{resolve_authored_source, generate_draft_body}`, `email/ai.rs::build_voice_ctx_for_recipient`. Related: #47, #36, #30, #34, #56, #57.

---

## Frontend (continued)

### 59. The blur is a CGS Gaussian we choose, not Apple's material — and switching source moved three jobs onto us

**Why the source changed.** `NSVisualEffectView` is a *material*: Apple's radius,
tint and saturation boost welded into one enum case with no dial. In dark mode
`UnderWindowBackground` reads as flat milky grey that destroys the wallpaper's
colour and shape, so no slider position ever looked like glass — the slider only
moved the tint on top. `glass_macos` calls the private
`CGSSetWindowBackgroundBlurRadius` instead: a plain Gaussian at a radius we pass,
no tint. Resolved by `dlsym` (connection: `CGSDefaultConnectionForThread`, else
`CGSMainConnectionID`), never linked — a macOS that drops the symbol yields
`None`, not a dyld abort, and falls back to the material. Zero new dependencies.
Closes only the Mac App Store door, already closed (Apple Events, LaunchAgent,
`agy`). Ported from cxtasks `767a8fb`. Ruled out and not worth retrying: other
`NSVisualEffectMaterial` cases (each bakes in its own tint and radius), and
`backdrop-filter` (#52).

**Job 1 — the launch gap (cxtasks gotcha #47).** A `transparent: true` window
around an empty webview is bare wallpaper with three floating traffic lights; the
material hid that by *being* something to paint, and a CGS blur paints nothing.
So `.setup()` paints the window OPAQUE — `set_launch_background(window, 28, 26,
23)`, dark `--bg-primary` — and `main.tsx` arms the glass with `startGlass`
behind a **double** `requestAnimationFrame` (one frame fires before the composite
often enough to show the flash; a `setTimeout` is a guess at what rAF measures).
`glassReady` keeps every earlier `applyWindow` CSS-only. **Cold-start only**, so
it survives every hot-reload iteration. The first commit always paints something:
`LicenseGate`'s checking state is a full-screen `bg-base` pane, so arming after
two frames cannot land on an empty webview in a release build either. The launch
colour now exists three times (CSS token, `THEME_BASE_RGB`, `lib.rs`);
`glass.test.ts` reads all three and fails on drift.

**Job 2 — the window's own edges, each a round in cxtasks.** Clear the
background to alpha **0.01, never 0**: fully clear plus a shadow makes AppKit
chamfer the corners, a hairline notch of desktop across each one.
`invalidateShadow()` after every opacity flip, or the window keeps the shadow it
computed while opaque and draws a second, misaligned edge. Turning glass OFF sets
the radius to **0 before** the background goes opaque; the other order leaves a
frame where an opaque window carries a blur region, painted as a grey halo
outside the frame. And `windowNumber` is ≤ 0 until the window is ordered in —
passing it blurs some *other* window, so it is guarded.

**Job 3 — Reduce Transparency is ours to honour now.** The material went opaque
on its own; nothing in the CGS path reads the setting, so switching source
silently dropped an accessibility behaviour. `reduce_transparency()` is read fresh
every time, `set_state` refuses glass while it is on, and
`install_accessibility_observer` watches
`NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` (the block is
leaked on purpose — the notification centre holds it by pointer) and emits
`reduce-transparency-changed`. The frontend pins the **effective** transparency
to 0 — window AND CSS tint, since an opaque window under translucent panes lands
the sidebar a shade off its token — and never moves the stored slider, so turning
the setting off restores the user's window. `get_reduce_transparency` WAITS for
the main thread (oneshot) instead of reading on the Tokio worker as cxtasks does:
that is #20's shape, and a zeroed BOOL there is a silently ignored setting.
**Testing it:** `defaults write com.apple.universalaccess reduceTransparency
-bool true` edits the plist WITHOUT posting the notification, so the live
observer never fires that way — but a fresh process reads the new value, so test
the hydrate path with it and restore with `defaults delete` (the key does not
exist by default). The live flip needs the real System Settings toggle and is
**still unverified**.

**Three deliberate departures from cxtasks.** The fallback material installs
ONCE (`install_fallback_once`) — cxtasks calls it on every non-radius
`set_state`, i.e. every theme toggle, and `vibrancy_macos::apply` stacks a new
view each time. `get_reduce_transparency` hops to the main thread (above). And
`transparencyToBlurRadius` goes through `clampTransparency`, so a corrupt value
means the DEFAULT radius exactly as it means the default alphas — cxtasks maps
NaN to the floor, which here would pair default-translucent panes with the
thinnest blur. (A NaN radius would reach `invoke` as JSON `null`, which serde
refuses for a `u8`: the blur would silently vanish.)

**The curve is CXMail's.** Same 10–48 band as cxtasks (past ~48 the wallpaper
turns to smoke; under 10 a translucent window over a sharp desktop reads as a
fault), but CXMail's 0.3 default lands on **21**, not cxtasks' 24 — message
bodies render straight onto the glass (`EmailFrame` is transparent), and on the
real window 21 kept the backdrop reading as shapes without competing with text.
Integer by construction, so `applyGlass`'s `lastSent` guard caps a full drag at
~38 IPC calls. `transparencyToAlphas`' floors are untouched.

**Verified on the real window, 2026-09-10** (signed debug binary, region
captures over the same backdrop, `bands.py`-style mean/stddev):

| State | Reading corner | Sidebar gutter |
|---|---|---|
| dark, default 0.3 (radius 21) | mean 74, stddev 1.42 | stddev 3.52 |
| dark, 91% (radius 44 via live `set_blur_radius`) | mean 174, stddev 3.80 | stddev 6.04 |
| RT on, slider still 91% | **mean 26.00, stddev 0.00** — exactly dark `--bg-primary` | 32.3 — exactly `--bg-sidebar` |
| RT deleted, relaunched | byte-identical to the 91% row | byte-identical |

Behind the sidebar and reading pane the backdrop reads as shapes (text bands, a
disc, a rectangle); the corners are clean. Chris's install is **light**, where the
0.3 default lets only 13.5% of the desktop through (`PANE_FLOOR`) — the change is
subtle there by design; raise the slider to see it.

Pattern: `src-tauri/src/glass_macos.rs`, `commands/glass.rs`, `vibrancy_macos.rs`
(fallback only), `lib.rs` `.setup()`,
`src/stores/uiStore.ts::{transparencyToBlurRadius, THEME_BASE_RGB, applyGlass, startGlass}`,
`main.tsx`, `src/stores/__tests__/glass.test.ts`. Related: #52, #20, #12, #31.

### 60. A per-area transparency cannot be a subtree override — `@theme` tokens resolve at `:root` — so the email body's dial is a solved veil

**The trap.** `--color-base: rgb(var(--bg-primary) / var(--alpha-pane))` lives in
`@theme`, which Tailwind emits on `:root`, and `bg-base` is `background-color:
var(--color-base)`. A custom property's `var()`s are substituted where it is
DECLARED; descendants inherit the finished colour. So setting `--alpha-pane` on a
subtree changes nothing, silently. Verified in WebKit 26.4: a `bg-base` element
with `--alpha-pane: 1` inline still computes to `rgba(28, 26, 23, 0.4)`.
(Re-declaring `--color-base` on the subtree would work, but a scope is the wrong
shape for a body that sits ON the pane rather than replacing it.)

**The fix is a veil.** The message body paints one extra layer in the SAME colour
as the surface beneath it, with an alpha solved so the stack lets through exactly
the desktop a single surface at the email's transparency would:
`veil = (target − under) / (1 − under)`. The same colour on both layers is what
makes the composite exact rather than a tint. `emailVeil.test.ts` composites it
over black and white at every pair of eight slider positions. There are two veils:
- `bg-email-veil`, pane-coloured, on `ReadingPane`'s body scroll container.
- `bg-email-veil-card`, surface-coloured and solved against the card's two-layer
  coverage, on a thread card's `ExpandedBody`. A pane-coloured veil on a card
  turns it two-tone.

The scope is the body only, and the header rows stay on the window's glass. That
was Chris's call, over the whole-pane option.

**The cap is arithmetic, not a rule.** The rule is "never more see-through than
the window". A veil can only add coverage, so an email dial set glassier than the
window solves to a negative alpha, and `solveVeil`'s clamp at 0 turns that into
exactly the window. An explicit `Math.min(email, window)` was written first and
then removed: mutation testing showed it was dead code that looked load-bearing.
Dropping the *clamp* is what fails the cap test. In Settings the label reads
"capped at N%" rather than leave a slider that looks broken, and the right end
reads "Same as window", not "Glass".

**Upgrading is invisible by construction.** Persist v2 seeds `emailTransparency`
from the stored window value, and equal dials solve to a veil of exactly 0.

Two limits:
- The blur stays window-wide: one CGS blur per window (#59). The dial controls how
  much blurred desktop reaches the text, not how blurred it is.
- Reduce Transparency pins both dials, so over an opaque window the veils are 0.

Pattern: `src/stores/uiStore.ts::{emailVeilAlphas, solveVeil, applyWindow}`,
`globals.css` (`--color-email-veil*`, `:root` fallback 0), `ReadingPane.tsx`,
`ThreadMessageCard.tsx::ExpandedBody`, `SettingsDialog.tsx`,
`src/stores/__tests__/emailVeil.test.ts`. Related: #52, #59.

---

### 61. A thread's members come from EVERY folder — so replies printed twice and an unsent draft printed as sent

`db::messages::get_thread` matches on `account_id` + the thread key with **no
folder predicate**, which is correct (a conversation spans Inbox, Sent and
everything else) and was rendering two different bugs at once:

* **Gmail mirrors every message into `[Gmail]/All Mail`**, so each of your own
  replies came back as two rows and the thread stack drew two identical cards.
* **An unsent draft is a row like any other.** A reply started in August and
  never sent sat in the thread between two real messages, styled identically
  to them — a false record of what the other person has been told, in the one
  place you would go to check.

The counting queries had the identical blind spot, three copies of it:
`messages::list_by_folder`, `messages::list_all_inboxes` and
`inbox_groups::list_by_group` each did `COUNT(*)` over every folder, so the
list row's badge (`thread_count + 1`) counted mirrors and drafts as messages.

**`folders::is_draft_sql` and `folders::folder_rank_sql` are the one matcher**
(gotcha #36), with a source-reading tripwire in `folders.rs` that fails if any
db module spells `folder_type = 'drafts'` itself. Both are `EXISTS`-shaped, so
an **unsynced folder list reads as ordinary mail** — the failure direction that
shows a message rather than hiding one behind a label that may be wrong.

**The rank ORDERS the dedupe; it must never filter.** 0 = ordinary (INBOX,
Sent, labels), 1 = drafts, 2 = archive; `ROW_NUMBER() PARTITION BY
COALESCE(NULLIF(message_id,''), 'uid:'||folder||':'||uid)` keeps the lowest.
Two tiers are load-bearing in opposite directions:
* **archive last, and only as a tiebreak.** On iCloud and Outlook — and on
  anything you archived out of a Gmail inbox — the archive folder holds the
  ONLY copy. Filtering on rank instead of ordering deletes those messages from
  their own threads.
* **drafts below ordinary.** A draft that was *sent* but whose Drafts row has
  not been expunged yet shares its Message-ID with the Sent copy; the safe
  reading of that pair is "this went out", not "still unsent".

**The `uid:` fallback in the key is not decoration.** `PARTITION BY` treats
NULLs as equal to each other, so a bare `message_id` key merges every id-less
message in a thread into one card. It takes TWO such rows to catch — a test
with one NULL passes against the broken version.

**`thread_draft_count` is a separate field, and the reason is the gate.**
`thread_count` is what the badge prints, so drafts must not be in it — but
`ReadingPane`'s thread-loading gate reads the same number, and simply
excluding drafts takes "one message plus the reply you started" to zero, so
the draft card would never render at all. Badge reads `thread_count`, gate
reads both. Conversely `thread_aggregate_columns(Some("?2"))` exempts drafts
**in the folder being listed**: browsing Drafts, the rows on screen ARE
drafts, and excluding them reports a count missing the message you are looking
at. All-Inboxes and the inbox groups are INBOX-scoped and pass `None`.
Free fix along the way: `has_unread_any` takes the same exclusion, so a draft
saved unread (6 of them in the real mailbox) stops bolding its whole thread.

**A draft card cannot expand, deliberately.** A read-only body is the one view
of an unfinished reply nobody wants, and rendering one is exactly what made it
look sent; the click goes to `openDraftForEdit`, which is idempotent under the
repeated clicks a slow IMAP open invites (#55).

**`isDraftFolder` was already two divergent copies** — `MessageList` consulted
the synced folder list alone, `MessageListItem` also accepted the two
well-known names — so before `foldersByAccount` loaded the same row selected
as a read-only message on click and opened the composer on double-click. One
`src/lib/draftFolder.ts` now, superset behaviour, before the thread card could
become a third.

**Two fixtures needed a `folders` table added** (the #54 lesson again): any
hand-built test schema that exercises a list query now has two predicates
reading tables it may not have declared.

Verify on real mail, never fixtures alone: `cargo run -p cxmail-db --example
thread_membership_probe -- <copy.db> [subject]` on a `.backup` copy (#12). It
prints a thread's raw rows and its members side by side. The reported thread
went 6 rows → 4 members with the badge reading 3.

Still carrying the same defect, deliberately out of scope: the MCP's
`read_thread` (`get_thread_with_ids`) is a separate query with no dedupe and
no draft marking — it prints `Folder:[Gmail]/Drafts`, so the fact is *there*,
but an agent has to notice it.

Pattern: `db/folders.rs::{is_draft_sql, folder_rank_sql}`,
`db/messages.rs::{get_thread, thread_aggregate_columns}`,
`db/inbox_groups.rs`, `src/lib/draftFolder.ts`, `ThreadMessageCard.tsx`,
`ReadingPane.tsx::{memberIsDraft, handleToggleMember}`. Related: #36, #54,
#55, #30.

---

### 62. A draft another client saved has lost CXMail's markers — reopened as-is, TipTap flattened it and the signature step appended a second copy below the quote (T186)

**Symptom**: a reply draft reopened in compose and saved came back with the signature table as bare paragraphs AND a fresh signature table after the quoted history; the quote's older tables flattened too.

**Not a CXMail→CXMail bug.** The intermediate draft (UID 988) had Message-ID `@mail.gmail.com` and `<div dir="ltr">` — it was saved from **Gmail's editor**, which keeps neither `class="email-signature"` nor `class="cx-quote"` (and `data-cx-*` never survive ammonia anyway; `class` is what re-claims the atomic nodes). Check the Message-ID domain first before theorizing about CXMail's save path. With no markers: the whole draft was body → StarterKit has no table node → paragraphs; `placeSignature` saw no marker → appended; the quote was plain body, so "append" meant after it.

**Fix (`lib/draftRecovery.ts`), applied only to reopened drafts (`draftContext`):**
- `recoverForeignDraftHtml` — when the HTML carries NO CXMail markers: the trailing top-level quote (`splitTrailingQuote`) becomes a `cx-quote` block again, and every outermost table outside a protected block is wrapped in `cx-html-block`. A marked draft is returned untouched on purpose, so gotcha #34's designed-HTML-without-`layout` behaviour is unchanged.
- `placeSignature` (the component's one-time signature step, extracted pure so the test drives the real thing): a `cx-html-block` whose text equals the account signature's text is re-marked as the signature; if the signature text is anywhere else in the body (already flattened by an earlier bad save) nothing is appended; otherwise it is inserted **before** the quote. Text comparison drops ALL whitespace (`<p>CX</p><p>Ventures</p>` vs `<div>CX</div>\n<div>Ventures</div>`), and blockquotes are excluded — an older quoted message signed by the same person must not count.

**Test the stored artefact, not the editor**: `draftRecovery.test.ts` loops open → signature step → edit → save (with `data-*` stripped like ammonia) three times and counts signature text copies outside the quote — counting only `div.email-signature` missed a recovered table sitting beside an appended signature (mutation-found). Verified against the real 988 body + real identity signature: 1 signature, a table, directly above the quote, quote tables intact across 3 saves. Drafts already damaged before the fix (e.g. 991) are not repaired — the no-duplicate rule just stops them getting worse.

Pattern: `src/lib/draftRecovery.ts`, `ComposeModal.tsx` (`initialContent`, signature effect), `src/lib/quoteToggle.ts::splitTrailingQuote`. Related: #34, #13, #55.

---

### 63. Send-as: Sent mail is proof, delivery headers are a suggestion, `To:` is neither (T207)

**No API lists aliases over IMAP.** Gmail's `users.settings.sendAs.list` *does* accept the `https://mail.google.com/` grant CXMail already holds (an earlier note said it needed `gmail.settings.basic` — wrong), but it is Gmail-only and not wired; iCloud has no public API. So `db::identities::send_as_addresses` = the account's own address + configured `identities` rows + **every `From:` on the account's own Sent folder** (`sent_from_addresses`, derived at read time, flagged `from_sent`).

**The trust split is the whole design:**
- **Sent mail = proof.** The provider's submission server accepted that From (Gmail rewrites one it won't send as) and nobody else can put mail in your Sent folder. Sent folder ONLY — a Drafts row was never submitted, and Gmail's All Mail holds everyone's mail.
- **Delivery headers = suggestion.** `Delivered-To` / `X-Original-To` / iCloud's `Original-recipient: rfc822;addr` are written by the receiving server, but a sender can forge any header, so `suggest_aliases` only offers; a person confirms.
- **`To:`/`Cc:` = neither.** The first `suggest_aliases` filtered same-domain `To:` addresses, which on a shared domain offered ~20 other `@gmail.com` co-recipients as `cxrobx@gmail.com`'s aliases. Plus-tags of a listed address (`cxrobx+aa-e2e-…`) are sub-addresses, also skipped.

**`Original-recipient` carries an address-type prefix** (`rfc822;`), so `extract_addresses` takes the text after the last `;` — without it the header parses to `rfc822;alias@…` and matches nothing (mutation-tested).

**Removal needs a tombstone** (`send_as_hidden`, v64): a found-in-Sent alias has no row to delete and re-derives on the next read. `remove_send_as` deletes any row AND tombstones; `create_identity` clears it. The UI removes through `remove_send_as`, never `identities.delete`.

**`messages.delivered_to` (v64) is a column, not a `message_headers` row** — that table means "the full block is cached" (#37). Sync fills it from its named header subset (`DELIVERED-TO X-ORIGINAL-TO ORIGINAL-RECIPIENT`, via `sync::record_delivered_to`, NULL-only); no backfill, so older mail relies on cached full blocks. The reply default reads To → Cc → `delivered_to` → cached block.

Verify on a `.backup` copy: `cargo run -p cxmail-db --example send_as_probe -- <copy.db>`. On 2026-09-23 it found `rileyprime@icloud.com` from uids 78/79 in iCloud Sent with nothing configured, and suggested only `admin@artistadvisory.io` and `hello@cxventures.io`.

Pattern: `db/identities.rs::{send_as_addresses, sent_from_addresses, append_sent_detected, remove_send_as, suggest_aliases, delivery_header_addresses}`, `db/schema.rs::migrate_v64_send_as_evidence`, `email/imap.rs::parse_fetch_to_header`, `email/sync.rs::record_delivered_to`, `commands/settings.rs::remove_send_as`, `SendAsSection.tsx`. Related: #36, #37, #30.

---

### 64. The in-app chat is `claude -p` with CXMail as the permission host — three flags carry the security, and `--allowedTools` is additive

**Shape.** One long-lived `claude -p --input-format stream-json --output-format stream-json` process per conversation (`commands::chat`), driven by the pure `email::chat_agent` (argv, tiers, MCP config, standing instructions, parser). `--permission-prompt-tool stdio` makes the CLI send a `can_use_tool` control request on stdout and wait; we answer `allow` (echoing the input as `updatedInput`) or `deny` (the message becomes the tool result). This is the protocol the Agent SDKs speak, not a published contract — so the parser is tested on CLI 2.1.282's exact shapes, and a 30 s watchdog stops a session whose `initialize` handshake never answers rather than let it run ungated.

**The chat acts on mail only through `cxmail-mcp`**, so every guard there applies for free (send disabled, `confirmed=true`, dash rule, v60 draft claim). Do not add chat-only mail tools in the app.

**Three flags, each load-bearing, all pinned in `chat_agent` tests:**
1. `--tools Read,Grep,Glob,Skill` — *replaces* the built-in set. Bash/Write/Edit/WebFetch/subagents are not denied, they are absent; an email that says "run this" has nothing to run it with.
2. `--permission-mode manual` — the user's `settings.json` defaults to `auto`, where a classifier, not the user, approves calls. Without the pin an unlisted tool may never reach the host.
3. `--strict-mcp-config` — cxmail plus servers passed through **by name** from `~/.claude.json` (`vault`), so no machine path lands in the repo. ~10× faster spawn (cxtasks' measurement) and no browser/Drive control reachable from mail content.

**`--allowedTools` is ADDITIVE over `~/.claude/settings.json`** (cxtasks measured a "read-only" run executing an approved `Bash(...)`). `--tools` contains that for built-ins; for MCP it means an `mcp__cxmail__*` allow rule in settings would silently skip the ask tier. None exists today. The auto tier is `AUTO_CXMAIL_TOOLS` — reads, `compose_draft`/`edit_draft` (a draft is never sent), `send_calendar_invites` (only *requests*; the approval card is its gate). Everything else asks, including tools added to the MCP later — the safe direction for that list to be stale in. Test-pinned: no mutating tool is ever in it.

**Project context is lighter than it looks.** Every mapped repo that exists is passed as `--add-dir` (variadic — last, #49), which makes it readable without prompts but does NOT load its `CLAUDE.md`, rules or skills. The standing instructions make the model call `resolve_project_repo` (the MCP face of `claude_repos::resolve_for_message`, one matcher #36) and read that `CLAUDE.md` first. A chat opened *from an email* starts in that email's repo and gets the full load.

**The permission card must be checkable by a person.** Raw input (`account_id` UUID, `uid 830`) is not; the app resolves ids to account address + subject + sender (`permission_context`, a read-only connection so a sync's mutex cannot stall the question, #11) and folds the exact call under *Details*. A failed lookup degrades to the raw card — it never blocks the question.

**"Open draft" parses two MCP result strings** (`extract_draft_ref`); a reworded result yields no button, never a wrong draft. Tripwire on the MCP side: `draft_result_wording_the_chat_panel_parses_is_unchanged`.

**Lifecycle.** The writer task owns stdin; dropping it is the EOF that ends the CLI — which also covers CXMail quitting. Measured: with a permission question pending, the CLI took ~10 s to exit after the app was killed, and its MCP children went with it (no ppid-1 orphans). Every event carries the session generation, so a late line from a stopped chat cannot land in the next one. *Continue in terminal* stops the process and waits for it to exit **before** `claude --resume` in Ghostty — two processes appending to one session file corrupt it — and resumes from the same cwd, because sessions are filed under the directory they started in.

**Environment.** User hooks stay on (the secret-exposure guard among them) with `CLAUDE_HOOK_SOUND=0`/`CLAUDE_HOOK_BANNER=0`; the child gets the login shell's PATH (hooks need `jq`/`sqlite3`). The `cxmail` server is the binary bundled beside the app when running from a `.app` (Keychain-ACL-trusted, #31), else the registered loose `target/release/cxmail-mcp` — so in dev, an MCP tool change needs that binary rebuilt with the Google client and signed (ship.md steps 2/2b) before the chat can see it.

**Verify headless** with `cargo run -p cxmail-email --example chat_probe -- <copy.db> "<message>" [--allow]` — the exact launch against the real CLI and MCP; every permission is denied unless `--allow`.

Pattern: `email/chat_agent.rs`, `commands/chat.rs`, `mcp/server.rs::{resolve_project_repo, project_repo_payload}`, `src/stores/chatStore.ts`, `src/lib/chatEvents.ts`, `src/components/chat/ChatPanel.tsx`. Related: #31, #36, #47, #49, #11.

---

## Lifecycle Management

- **SUPERSEDED**: When a gotcha is resolved, mark it: `## #N: [Title] ~~SUPERSEDED~~`
- **Merging**: If two gotchas share a root cause, merge and note consolidated numbers
- **Pruning**: When gotchas exceed 30 items or 15k chars, prune SUPERSEDED entries older than 90 days. Full-file prune 2026-08-17: narrative/forensics condensed across all entries; full text in git history (pre-prune) and CLAUDE.md Recent Learnings.
- **Numbering**: Original numbers are permanent — gaps are intentional. Never renumber.
- **Categories**: Environment, Database, Backend, Frontend, Security, Deployment, External APIs
