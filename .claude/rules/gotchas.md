# Known Gotchas — summary (always loaded)

One rule per gotcha. **The full entry — symptom, why, every guard, the test that pins it — is
in [`docs/gotchas.md`](../../docs/gotchas.md) under the same number. Read it before changing
code a rule names.** Numbers are permanent; gaps are intentional. New gotcha: write the full
entry there, then add its one line here.

## Environment
- **#1** `source "$HOME/.cargo/env"` before cargo/tauri — and *outside* wrappers like `secret run` (dies as `failed to run 'cargo metadata'`).
- **#5/#6** Dev builds use the encrypted `credentials.dat` file store; Keychain is production only.
- **#12** Never run two CXMail instances on one SQLite file — `pkill -f target/debug/cxmail` before restarting dev.
- **#19** Production builds have no DevTools; debug UI in `tauri dev`.
- **#28** Owner builds need `CXMAIL_OWNER_LICENSE=owner`; `strings | grep OWNER` reads 0 on a working build; owner builds skip update checks on purpose.
- **#31** Keychain prompts are gated by the item's ACL trusted-app list (set only at creation), not the partition list. Use `scripts/dev-signed.sh` (`--identifier com.cxmail.app`); sign `cxmail-mcp` after every rebuild (copy + `mv`). Audit with `scripts/audit-keychain-acls.sh` (it can truncate — check the count).
- **#42** vitest "21 errors, no tests" = Node 20. Use `~/.nvm/versions/node/v24.11.1/bin`.

## Backend
- **#2** XOAUTH2 SASL string goes raw; async-imap base64s it.
- **#3** async-imap needs the tokio-util compat layer.
- **#4** URL-decode the OAuth callback code.
- **#7** Decode RFC 2047 encoded-words at parse time.
- **#9** Never add `style` to ammonia `tags`; wrap `sanitize_html`/`parse_message` in `catch_unwind`.
- **#10** Every IMAP op needs a timeout (15 s connect, 45 s per account).
- **#11** Keep DB mutex scopes small; never hold a transaction across IMAP awaits.
- **#13** Old MCP drafts with collapsed paragraphs: regenerate through `compose_draft`/`edit_draft`.
- **#14** `insert_body` is INSERT OR REPLACE and nulls six columns — use the targeted UPDATE/upsert helpers on existing rows.
- **#15** `normalize_unstructured_header` unescapes `\"` in `subject` only.
- **#20** AppKit calls go through `run_on_main_thread` (sync commands run on Tokio workers).
- **#21/#21b** Detected meeting join URL goes in `description`, never `location`; dtstart Z-contract; read tz from the time's own line; use `s.get(..n)`, never a length-guarded slice; detector fixes need a refresh-only migration probed on a `.backup` copy.
- **#22** Don't use `layout="card"` for correspondence — Gmail folds it.
- **#23** Compare ISO timestamps as `datetime(col) <= datetime('now')`; don't change the stored format.
- **#24/#35** "Open in Claude": AppleScript `new window with configuration` first (shell-quote `command`, cwd raw), `open -na --window-save-state=never` fallback; needs both Apple Events grants.
- **#25** Drafts carry attachment metadata only — reopen must reload bytes or the next save drops them.
- **#33/#34** Designed MCP HTML needs `layout` to survive compose-open; don't build bullets from table rows; don't use `rich` for plain prose.
- **#36** `MailRule` validation lives in Rust and `mailRuleValidation.ts` (change both); empty conditions match everything; one matcher (`scan_rule_matches`); `apply_mail_rule` dry-runs by default; keep stdin open when probing over stdio.
- **#37** Raw headers live in `message_headers`, never on `message_bodies` (the body-cache flag); use `BODY.PEEK`; no backfill.
- **#38** Draining an async-imap stream never shows a tagged NO/BAD — use `run_command_and_check_ok`; grep `uid_store` itself.
- **#39** `Re:` drafts need a reply target (`validate_reply_threading`); `buildDraftComposeProps` is the single reopen-props builder.
- **#41** No IMAP STARTTLS (`BufReader::into_inner` drops bytes); no `TlsMode::Plain`; folder heuristics match the last segment whole; `folder_of_type` is the single accessor.
- **#44** Zoom links live in `zoom_meetings` (no FKs, remote-triple key), never a `gcal_events` column; never delete a Zoom meeting on absence; `start_url` is unrepresentable.
- **#45** `u8 as char` is a Latin-1 decode — accumulate bytes, decode once; grep `as char` after touching any decoder.
- **#46** `believed_open` is not a socket count; trust `lsof`. Test the protocol, not ICMP.
- **#47** Dash rule is enforced at the write boundary (`enforce_dash_rule`, `call_inference_prose`) on authored text only, before quote/signature; never touch tag interiors, URLs or unspaced en-dashes.
- **#48** `normalize_cell_text` wraps bare `<td>` text in a `<p>` whose four `inherit` declarations are load-bearing; it is layout-neutral, not a rendering fix.
- **#49** `--add-dir` is variadic: prompt first, `--add-dir` last; read the prompt file by absolute path.
- **#56** Draft attachment bytes are cached in `draft_attachment_blobs` (empty = not cached); spawn the LOGOUT, don't await it.
- **#57** `edit_draft` order: claim → preflight → APPEND → retire old UID with `UID EXPUNGE`. Drafts are addressed by `draft_id` + `expected_uid`; `drafts.link` is forward-only per `(uidvalidity, folder)`.
- **#58** MCP draft prose comes from the calling agent; `instruction` → `agy` is a separate authorship path with a test-pinned containment argv. Sign `cxmail-mcp` with its default identifier.
- **#64** In-app chat security is `--tools` + `--permission-mode manual` + `--strict-mcp-config`; `--allowedTools` is additive; CXTasks writes always ask.

## Database
- **#26** FTS5 by triggers: never `recursive_triggers`, `messages` stays INSERT OR IGNORE, never write `messages_fts` directly, keywords through `build_match_expr`.
- **#30** `message_id` exists in three spellings — match `IN (variants)`, never `trim()` on the column; nothing fallible after the APPEND; don't touch `format_quoted_history`; check `EXPLAIN QUERY PLAN` uses the lookup's own key.
- **#40** Subject-fallback threading requires a reply prefix; verify with the GROUP BY top-10 query on real data.
- **#43** Pinned voice rules are separate from `profile_json` and have no FK to profiles.
- **#50** Google has no "invitation sent" fact — `invite_notifications` records it after the 2xx; never label `sent` as "received"; cancellations need an explicit `notify`.
- **#54** Hidden accounts: every aggregate embeds `visible_in_aggregates_sql`; don't pre-filter ids in the frontend (`[]`→`undefined` means all).
- **#61** Thread members span folders: dedupe by rank (it orders, never filters); drafts counted separately (`thread_draft_count`).
- **#63** Send-as: Sent-folder `From:` is proof, delivery headers only suggest, `To:` is neither; removal tombstones in `send_as_hidden`.

## Frontend
- **#8** Account-row click selects; chevron collapses.
- **#16/#17** Radix `Trigger asChild` may swallow your `onContextMenu` — use `onOpenChange`.
- **#18** HMR doesn't re-run module-level code — Cmd+R.
- **#27** Spellcheck squiggles need the `WebContinuousSpellCheckingEnabled` user default as well as the attribute.
- **#29** The email `srcdoc` iframe inherits the CSP — scripts load from `public/email-frame.js`, never inline or hash-allowed.
- **#32** A selection's own `account_id` wins over the view's.
- **#33** Editable ProseMirror node views need an uneditable root; flush `flushHtmlBlockEdits()` in `buildOutgoingEmail()`; verify in a real browser, not jsdom.
- **#51** Window drag needs `data-tauri-drag-region="deep"` and the `core:window:allow-start-dragging` grant; `-webkit-app-region` is inert.
- **#52/#59/#60** Glass: root element paints nothing; floating UI uses `-solid` tokens; `clampTransparency` guards NaN; the blur is CGS (window boots opaque, Reduce Transparency is ours); per-area transparency is a solved veil; verify with region `screencapture`, not Playwright.
- **#53** "Account is selected" needs all four terms, not `selectedAccountId` alone.
- **#55** Compose windows carry an identity `key` that follows the draft's UID; `openDraftForEdit` is single-flight.
- **#62** Drafts saved by another client lose CXMail's markers — `recoverForeignDraftHtml` + `placeSignature`.
- **#65** Vault theme: `AppLayout`'s root carries the preference (`"vault"`), not the resolved theme; palettes are validated as `[u8; 3]`.

## Recurring rules
- **Probe any data migration on a `.backup` copy of the real mailbox** (#21b, #45, #47, #56) — unit tests have passed against broken versions every time.
- **One matcher per question** (#36, #49, #50, #54, #61) — reuse the helper, don't write a second query.
- **Your own `mcp__cxmail__*` tools run the binary from session start** (#47) — test a rebuilt MCP over stdio.
