# Changelog

All notable changes to CXMail are documented here.

## Unreleased

### Added
- **Chat with Claude, inside CXMail.** Press `⌘L` (or click **Claude** in the title bar) and ask in plain words — *"write a follow-up to Nick"*, *"what needs a reply from me today?"*. It runs on your own Claude Code sign-in (`claude` CLI), so there is no API key to set up.
  - **It reads what you would read.** It searches your mail, reads threads, and looks up which project folder a correspondent belongs to (the same links as *Open in Claude*, set in Settings → Claude repos), then reads that project's notes before writing.
  - **It writes drafts, never sends.** A finished draft shows an **Open draft** button that puts it in the compose window for you to review and send.
  - **Anything that changes your mail asks first.** Archiving, moving, deleting, flagging, rules, groups and calendar changes show a card in the chat — *"Archive this email?"* with the subject, sender and account — and wait for **Allow**, **Allow for this chat** or **Deny**. It cannot run commands or edit files at all.
  - **Ask about one email:** right-click it → *Ask Claude in Chat*, or `⌘K` → *Ask Claude About This Email*. The chat starts in that email's project folder.
  - **Continue in terminal** hands the conversation to a Claude Code session in Ghostty, picking up exactly where the chat left off.
- **Hide an account from All Inboxes & groups.** Right-click an account → *Hide from All Inboxes & groups*. A hidden account's mail leaves every cross-account surface at once — All Inboxes, its account folder, rule-based inbox groups, Needs You, nudges, search run from anywhere but inside it, the category-tab counts and the dock badge — and comes back with *Show in All Inboxes & groups* (database schema v58). Built for a warm-up mailbox whose traffic is noise everywhere except in its own inbox.
  - **Clicking the account is the only way in.** Its inbox, folders and category tabs are untouched, and a search typed while inside it searches that account. Sync and IMAP IDLE continue as before, so the mail is already there when you look.
  - **Hidden wins over group membership.** An account added to an inbox group explicitly, or one whose mail a group's rules would match, contributes nothing to that group while hidden; the group editor labels such an account rather than offering a checkbox that does nothing. The sidebar row carries a small glyph whose tooltip lists what the account is hidden from.
  - **Notifications are a separate switch.** *Mute notifications* still governs whether new mail in the account alerts you; hiding does not change it.
  - Claude's `search_emails` skips hidden accounts unless `account_ids` names them, and its result says which accounts it skipped; `list_accounts` shows the flag.
  - Also fixed on the way: a search scoped to an account folder no longer has its server-side fallback merge in rows from every other account.
- **A Settings window, and a translucent "glass" look for dark mode.** The window now sits on a real macOS blur of whatever is behind it, the way Notes and Finder do, instead of being a flat opaque rectangle.
  - **Settings** opens from the gear beside Compose in the title bar, or from the command palette (`⌘K` → "Settings"). It holds Theme, Density and the new Window transparency dial. Theme and density were previously reachable only by knowing to search the command palette for them.
  - **Transparency is a slider, not a set of presets**, because how much is right depends entirely on the wallpaper behind the window. Dragging it left all the way returns the window to fully opaque — that end of the range is a real destination, not a disabled state.
  - **The sidebar runs slightly glassier than the mail itself**, so the rail reads as chrome laid over the work rather than a gutter cut out of it. Menus, dialogs, the composer and every pop-out window stay fully opaque — they float over the window rather than being part of it, and a see-through menu is text on a wallpaper.
  - **Light mode holds more opacity at the same setting.** Dark text loses to a bright wallpaper much faster than light text does, so the same slider position deliberately means less transparency there. The Settings panel says so.
  - Native menus, sheets and scrollbars now follow CXMail's own theme instead of the system's — so a dark CXMail on a Mac set to Light mode no longer erupts a light-grey right-click menu.
  - **The blur keeps what is behind the window recognisable.** It is a true blur whose strength follows the transparency slider — the more see-through, the more blur — rather than macOS's standard frosted material, which turned any wallpaper into the same flat grey. It honours **System Settings → Accessibility → Display → Reduce transparency**: with that on the window is fully opaque whatever the slider says, and turning it off gives you back exactly the window you had.
  - **The message you are reading has its own transparency.** Settings → *Email transparency* holds the body of an email more opaque than the rest of the window, because that is where text sits directly on the glass and a busy wallpaper starts to cost readability. It never makes the email more see-through than the window (drag it past the window's value and it says *capped*), and it starts where your window already is, so nothing changes until you move it.

### Fixed
- **Stray `â` characters in the inbox preview line.** A message from Tim Ferriss previewed as `âA writerâand, I believe, generally all personsâmust` where the sender had written `“A writer—and, I believe, generally all persons—must`. Smart quotes, em dashes, accented letters and emoji were all affected — anywhere the preview text had to be pulled out of a message's HTML, which is most newsletters (database schema v54).
  - **Only the one-line preview was ever wrong.** The subject on the same row, and the message itself when opened, were always correct — the text was being mangled on its way into the preview, not on its way into CXMail.
  - **Previews already stored are repaired on first launch**, not just new mail. 417 of them in this mailbox. A preview is written once when a message arrives and never recalculated, so without this they would have stayed wrong forever.
  - The repair is deliberately cautious and leaves anything it cannot repair with certainty exactly as it is: genuinely accented text like *Café au lait* is untouched, and 19 previews consisting of invisible spacer characters are beyond recovery — their readable text was already correct.
  - The same fault was fixed in the unsubscribe link handler, where it could have garbled a non-English subject line.
- **Detected meeting times are read correctly again.** A meeting invite whose schedule looks like this —

  ```
  *Thursday, August 13, 2026:*
  12:30pm - 1:00pm ET - Hiring Manager Screen
  ```

  was being filed as an **all-day event on the day the email arrived**, and the reading-pane card showed a different (also wrong) time than the calendar did. It is now read as Thursday 13 August, 12:30–1:00pm Eastern, with the end time included (database schema v53).
  - Three separate things were wrong: nothing looked for a time on the line *below* a date; a date line styled bold by the sender (`*Thursday, …*`) could not be parsed at all; and the time zone was liable to be taken from the sender's signature — these emails say `PST` at the bottom while the meeting is `ET`, which would have moved the meeting three hours.
  - **Meeting invites already in your mailbox are corrected on first launch**, not just new ones. Events you had dismissed stay dismissed, and real calendar invitations are never touched — only CXMail's own guesses are rewritten.
  - All-day events no longer display a day early with an invented time.
  - Also fixed a crash: an email containing a line of `──────────` could make detection fail outright on that message, because a character-count check was mistaken for a safe text boundary. Two more instances of the same mistake were found and fixed, one of which ran on every message that syncs.

### Added
- **"Open in Claude" now opens in the right repo.** Right-clicking an email and choosing *Open in Claude* used to start a session in a scratch folder containing only that email — Claude had the mail and no project. It now starts in the repo the email belongs to, so its `CLAUDE.md`, its code and its history are already there (database schema v56).
  - **The repo is worked out from who the email involves**, in this order: a correspondent you have mapped (an address, or a whole domain like `northwind.example`) → an inbox group whose rules match the message → the account it arrived in → a group that account belongs to → a default. The first match wins; anything unmatched opens in the scratch folder exactly as before.
  - **Correspondent mappings read the recipients too, not just the sender.** A reply you sent to a client lands in the same repo as their original, and an email *from* someone else with the client on cc still lands there.
  - **Mappings are yours to set**, in a new panel: the terminal icon at the top of the sidebar. Type a path or pick a folder; empty the field to remove a mapping. A path that isn't there any more is flagged in the panel, and a handoff that hits one still opens — in the scratch folder, and it tells you why.
  - A toast names the repo it opened in and the rule that chose it, since the terminal window itself looks the same wherever it landed.
  - **Option-click → *Copy Claude Prompt* carries the repo too**, as a ready-to-run `cd` line, so a prompt pasted into a Claude session you already had open moves it to the right project. A mapping whose folder has moved is named but never `cd`-ed into.
  - The email's own material (`email.json`, downloaded attachments) is still readable from the repo.
  - Mappings are made in Settings → Claude repos.

- **Zoom meetings on calendar events.** A calendar event Claude creates can now be backed by a real Zoom meeting instead of a Google Meet link, for the case Meet cannot cover: sharing your screen *with computer audio* on macOS, which a browser has no way to do. The meeting is created on Zoom, its join link is written into the event, and CXMail's "Join meeting" button opens it (database schema v52).
  - **Google Meet is still the default and nothing about existing events changes.** Zoom is requested per call, through the CXMail MCP (`create_calendar_event` with `conference: "zoom"`). The "Add Google Meet" checkbox in CXMail's own event window is untouched.
  - **The meeting follows the event for its whole life.** Move the event, rename it, make it longer, or delete it, and the Zoom meeting is updated or removed to match — *including when you drag the event around in Google Calendar's own website*, which nothing in CXMail is told about. A rename changes only the title; the meeting's time is left alone.
  - **It will not delete a meeting it is unsure about.** If CXMail loses its cached copy of an event — which happens routinely, for example after a full calendar re-sync — it asks Google whether the event still exists rather than assuming it was deleted. A meeting it cannot check on is left alone and listed for you in the Zoom settings panel. Meetings that have already happened are never deleted, because deleting one on Zoom also destroys its cloud recording and attendance report.
  - Rescheduling an event *and* rewriting its description in the same MCP call is refused rather than silently dropping the join link out of the description.
  - The approval card shown before invitations go out now names the actual conferencing for the meeting — it previously always said "Meet", and read "Meet: None" on a Zoom event.
  - Set it up in Calendar → the video icon: a Zoom **Server-to-Server OAuth** app's Account ID, Client ID and Client Secret. The credentials are checked against Zoom before they are saved, and the Test button reports the scopes Zoom actually granted — the one thing that distinguishes "the app has no permissions" from "wrong secret", since both otherwise look identical. Not required unless you use Zoom.
- **Pinned writing rules — a way to say "always".** You can now state a permanent instruction for how your mail is written, and it is obeyed on every draft: *always address Sam as "Bro. Ellis", never "Sam"*, or *never open with "Hope you're well"*. Rules are set through the CXMail MCP (`set_voice_rule` / `list_voice_rules` / `delete_voice_rule`) and apply either to one correspondent or to every message sent from an account (database schema v51).
  - **They are stored apart from the learned voice profile, and cannot be erased by it.** The learned profile is a description of how you have written in the past, rebuilt from your sent mail; rebuilding it — even forcibly — leaves your rules untouched. This is the difference the feature exists for: a profile built from mail that opens "Bro. Ellis" twice and "Sam" once can only report that you do both, which leaves an assistant free to pick the wrong one.
  - Rules override the learned profile everywhere it is used: CXMail's own AI writing tools, and any assistant working through the MCP. Where an account-wide rule and a rule for one person disagree, the more specific one wins.
  - Drafting through the MCP now reports the rules that apply to the message it just created, so a rule cannot be silently missed.
  - A rule pinned to a name rather than an email address, or one whose scope contradicts its recipient, is refused with the reason — each of those would otherwise be stored happily and then quietly never apply.

## 1.0.0 — 2026-08-07

### Added
- **Automatic server setup for any mail domain.** CXMail now works out where a mail address gets its mail, so connecting a custom domain or a self-hosted server usually needs nothing but the address and a password. It tries, in order: nine curated providers, a bundled database covering 969 more domains, configuration published by your own mail domain, and finally your domain's DNS records — including following the MX record to whoever actually hosts the mail.
  - The provider database is **bundled, not fetched**, so setup is instant, works offline, and does not depend on anyone else's server staying up. Refreshed with each release.
  - Discovery never connects and never sees your password. Anything found over the network is shown in the server-settings panel first, so you can see the hostname before a credential is sent to it.

- **Generic IMAP/SMTP accounts.** A fourth provider, `imap`, connects any mailbox that speaks standard IMAP and SMTP with an app password — Fastmail, Zoho, Yahoo, GMX, Yandex, AOL, mail.com, a self-hosted server, or a custom domain (database schema v50). Presets cover nine known providers by email domain, so a recognized address needs only an address and a password; anything else asks for server settings. OAuth remains the default path for Gmail and Outlook.
  - **Gmail is available by app password too**, and that is the point of shipping this now: an app-password connection is not OAuth, so it consumes none of the Google Cloud project's 100-consent lifetime cap, shows no unverified-app warning, and needs no Google review.
  - Setup validates **both** transports before saving the account and deletes the stored password if either fails. IMAP uses native-tls and SMTP uses rustls, so they do not share a trust store — an IMAP success is not evidence that sending works.
  - Folder detection prefers the server's RFC 6154 SPECIAL-USE attributes and falls back to name matching (last hierarchy segment, matched whole) for servers that do not publish them.
  - Connections are always encrypted. Ports 993/465 use implicit TLS and 587 uses STARTTLS; port 25 and the POP3 ports are refused by name with the reason. IMAP STARTTLS (port 143) is not supported yet.

- Commercial activation with encrypted macOS Keychain storage, server-side validation, revocation support, and a 30-day offline grace period.
- Bring-your-own inference support for OpenAI, Anthropic, and local or hosted OpenAI-compatible providers, including connection testing and encrypted API-key storage.
- Needs You, an on-device action queue for messages that appear to need a reply, decision, follow-up, or document review.
- Durable per-recipient voice learning groups sent mail by local style and semantic similarity, incrementally assigns new messages, and balances profile extraction across long-lived groups instead of using only the newest messages (database schema v44).
- Signed automatic updates. Signing is verified working end to end — a CI-built artifact validates against the embedded public key. The delivery endpoint is implemented and begins serving as soon as this release's artifacts are published to it; until then it correctly reports that there is no update rather than erroring.

### Removed
- PGP encryption. It was four IPC commands with no user interface and no callers — nothing in the app could reach it. Its library (`sequoia-openpgp`) was the only copyleft dependency in the project, carrying obligations that complicate commercial distribution, so it was removed rather than kept for unreachable code. The application binary is 1.7 MB smaller as a result.

### Fixed
- **A failed update check left no trace.** Release builds ship without developer tools, so the one place the failure was written — the browser console — could not be read by anyone, including us. Update-check and update-install failures now go to the application log alongside everything else.
- **Removing an account left its credentials in the Keychain.** Mail, folders and messages were deleted, but up to seven credential entries per account survived — including the separately connected Google Calendar grant. They are now removed with the account.
- **A send could hang for an hour with no signal.** The SMTP client was using its library's default one-hour timeout; it is now 30 seconds, matching the rest of the app.
- Archive, delete and draft-save now use the account's actual synced folder list, falling back to the built-in provider names only before the first folder sync. Previously the destination was resolved from a hardcoded table at ten separate places, which disagreed about where iCloud keeps sent mail, and could pick nondeterministically between two candidate folders.

- Recipient voice profiles no longer forget older writing modes when the latest-message window rolls forward. Refreshes merge the cached profile with representatives from every durable style group, raw drafting examples use the same diverse selection, and pre-v44 profiles lazily rebuild into the new store. Voice extraction also accepts valid provider JSON that omits the optional `voice_examples` field, while preserving the last good cache and logging refresh failures.
- **MCP reply quoting silently failed and misreported why.** Drafting a reply through `compose_draft` / `edit_draft` returned *"quoted history unavailable — original body not cached"* and produced a draft with no quoted history — which pushed agents to paste the thread into `body`, landing the user's signature *below* the quote. The message was factually false: the body was cached. `messages.message_id` is stored in three spellings — bracketed `<id@host>` from the IMAP sync (29,257 rows), bare `id@host` minted by `draft_local.rs` / `import.rs` (16 rows), and HTML-escaped `&lt;id@host&gt;` when an agent copies an ID off a rendered surface — and both reply lookups exact-matched it. Any mismatch degraded to "not cached" *and* silently corrupted the emitted `References` header.
  - Lookups are now bracket-insensitive via `message_id IN (?2, ?3)` — deliberately not `trim(message_id,'<>')`, which would defeat `idx_messages_message_id` and scan ~29k rows per reply. The column is not migrated; the two ingress points normalize instead.
  - `compose_draft` / `edit_draft` accept **`reply_to_folder` + `reply_to_uid`**, the same `(account_id, folder, uid)` key the app itself quotes from and the coordinates every read tool already prints. Coordinates win over `reply_to_message_id`; half a pair is `invalid_params`. Both params are optional, so existing callers are unaffected.
  - A quote that cannot be built is now a **hard error** naming the reason and the remedy, raised before the IMAP APPEND and before `edit_draft`'s delete+expunge — so no stray draft is created and no original is destroyed. Success is confirmed explicitly with `+ quoted original`. Deliberate opt-outs (`quote_original=false`, an already-quoted body) stay silent no-ops.
  - Six distinct skip reasons replace the single static string. The swallowed IMAP-connect error (`.ok()?`) and ignored `select_folder` result (`let _ =`) are now matched, logged, and disconnected — the old select-failure path leaked the session — and `parse_message` is wrapped in `catch_unwind` per gotcha #9.
  - `get_reply_source` preferred any folder with a body *row*, including a blank one; it now prefers a non-blank body. An IMAP-fetched body is cached via the new `upsert_body_preserving_metadata` (`INSERT … ON CONFLICT DO UPDATE`), which preserves the six columns `insert_body`'s `INSERT OR REPLACE` would null — `summary`, `summary_model`, `to_json`, `cc_json`, `bcc_json`, `attachment_metadata_checked` — and refuses to overwrite a good body with a blank one.
  - `read_email`'s IMAP-miss branch handed back a bare Message-ID while its cached branch handed back a bracketed one, so the spelling an agent copied depended on cache state. All read tools now emit normalized IDs plus `Folder:` / `UID:`; `read_thread` gained the folder it was missing.
  - The `/cxmail:reply` skill documents the coordinates path and gains an error-recovery step. `format_quoted_history` — byte-contract with `src/lib/utils.ts` and TipTap's `QuotedBlock` — is untouched; its contract test passes unmodified. +32 tests (257 → 289). See gotcha #30.
- Email bodies rendered clipped to a 200px sliver, and email links, "Show images", and the image context menu silently stopped working. The tightened Content Security Policy below is inherited by the sandboxed `srcdoc` iframe used to render mail, which blocked the inline script that measures the message and reports its height. The script now loads from `public/email-frame.js`, so the policy stays strict. Pinned by a regression test; see gotcha #29.
- The updater public key embedded in `tauri.conf.json` did not match the signing key — the comment had lost a character and one byte of the key body was wrong, while the key ID still matched. Every signed update would have been rejected by every installed copy, with no error until the update silently failed to apply. `release-check.sh` now compares the embedded key against the signing key's public half, and `scripts/verify-updater-signature.mjs` validates a real signature end to end.
- Outlook reported "not configured" in any build that did not receive `CXMAIL_MS_CLIENT_ID` at compile time, because the fallback was an empty string. It now defaults to the production public client ID, matching how the Google client ID is handled.
- A `workflow_dispatch` release run could never finish: it addressed the draft release by branch name instead of the tag, failing *after* notarization and leaving the draft holding an un-stapled DMG.

### Changed
- Production credentials now use the macOS Keychain. Outlook uses a PKCE public client with a loopback callback and no client secret. Google is registered as a **Desktop app** client and ships a `client_secret` — not an oversight, and not a Web-application mislabel: Google's token endpoint requires the secret for Desktop clients too, on both the code exchange and every refresh, verified live twice. PKCE is additive for Google, not a substitute, and the "not applicable" carve-out in Google's docs names only Android/iOS/Chrome-app clients. Registering a second Desktop client does not avoid this and has already regressed once; see the comment on `GOOGLE_CLIENT_SECRET` in `src-tauri/src/secrets.rs`. Google treats an installed-app secret as non-confidential by design, so this is not the credential-leak class a Web-app secret would be.
- The macOS bundle builds universally for Apple silicon and Intel and is prepared for Developer ID signing, notarization, and stapling.
- Claude Sonnet 5 requests omit sampling parameters that the current Anthropic Messages API rejects and disable adaptive thinking for CXMail's short email-generation tasks.

### Security
- Tightened Content Security Policy, input validation, TLS-only remote inference URLs, local-only HTTP provider URLs, and paid-release gating.
- Preserved sanitized HTML storage and sandboxed email rendering with scripts disabled.

## 2026-06-02 — Fix ISO-vs-`datetime('now')` time-comparison bug (scheduled send, snooze, follow-up, calendar)

### Fixed
- **Time-based features fired at the wrong instant** (`src-tauri/src/db/{scheduled,snoozed,followup,calendar}.rs`). The frontend/ICS store timestamps as ISO-8601 (`2026-06-03T13:00:00.000Z` — capital `T`, `.000` ms, trailing `Z`), but the "is it due?" scans compared that TEXT column **lexicographically** against SQLite's `datetime('now')` (`2026-06-03 13:00:00` — space separator, no `T`/`Z`). At byte index 10, `T` (0x54) sorts after the space (0x20), so a row whose UTC date equals today never read as "due" until the UTC clock rolled to the next day. Net effect: a 9 AM (13:00 UTC) scheduled email would have fired at ~00:00 UTC the next day (~8 PM local), and calendar events earlier today lingered as "upcoming" until UTC midnight.
- **Fix**: wrap **both** operands in SQLite's `datetime()` so they normalize to the same comparable form — `datetime(send_at) <= datetime('now')`, `datetime(wake_at) <= datetime('now')`, `datetime(remind_at) <= datetime('now')`, `datetime(dtstart) >= datetime('now')`, and `datetime(dtstart) >= datetime(?) AND datetime(dtstart) < datetime(?)` for the calendar date-range scan. This extends the idiom already proven at `src-tauri/src/email/voice.rs:377` (`datetime(m.date) > datetime(?3)`). **Storage format is unchanged** (stays ISO) so the frontend display paths that rely on `new Date(iso)` parsing the trailing `Z` keep rendering correct local time. No schema change, no migration, `CURRENT_VERSION` stays v38.
- **Scope**: 5 query groups across 4 files — `scheduled::list_due`, `snoozed::list_due`, `followup::list_due`, `calendar::list_upcoming` (both account-scoped and all-account branches), `calendar::list_by_date_range` (both branches). `created_at`/`updated_at`/`fetched_at`/etc. (SQLite-native space format via `DEFAULT (datetime('now'))`) and `messages.date` search filters (ISO on both sides) were audited and left untouched.

### Added
- **Regression tests** (`#[cfg(test)] mod tests` in `scheduled.rs`, `snoozed.rs`, `followup.rs`, `calendar.rs`, in-memory SQLite, +9 tests → 180 total). The critical fixture inserts a row dated `date('now') || 'T00:00:00.001Z'` — today at the very start of the UTC day, always in the past yet sharing today's UTC date. A naive 2020/2999 past/future test does **not** catch this bug (it passes lexicographically even when broken); this fixture fails pre-fix and passes post-fix (verified by temporarily reverting the query). Calendar tests also assert all three `dtstart` storage shapes from gotcha #21 (`…Z`, floating `…THH:MM:SS`, date-only `YYYY-MM-DD`) survive `datetime()` without being silently dropped.

## 2026-05-29 — Plain-text meeting-invite calendar detection

### Added
- **Strategy 5 body-invite detection** (`src-tauri/src/email/detect_events.rs::detect_meeting_invite`): detects Zoom / Google Meet / Microsoft Teams / Webex / Jitsi invites pasted into the email *body* (no `text/calendar` MIME part — e.g. forwarded "Sent from my iPhone" invites). Two-signal gate to suppress promos: requires a host-anchored provider join URL **and** at least one of {an invite signature phrase, a parseable labeled date/time}. Webinar/training/workshop/course blasts require the explicit phrase signal and are confidence-capped at 0.7. Confidence tiers: URL=0.7, +datetime=0.85, +phrase+datetime=0.95. Wired into `detect_events()` after Strategy 4; `persist_parsed_metadata` (`src-tauri/src/commands/messages.rs`) inserts at the unchanged confidence ≥ 0.6 threshold. Join-URL regexes mirror `src/lib/meetingLink.ts` exactly (host-anchored, so a bare "zoom" mention won't match), compiled once via `OnceLock`.
- **Named-zone → UTC with per-date DST** (`parse_datetime_tz`, `match_tz` + `TZ_ALIASES`): parses "Month Day, Year HH:MM AM/PM <tz>", resolving Central/Eastern/Pacific/Mountain (full names, "… (US and Canada)" suffix, and 2–3-letter codes, plus UTC/GMT) to IANA zones via the new `chrono-tz = "0.10"` dep (`src-tauri/Cargo.toml`), then to true UTC honoring the DST offset for that calendar date (May Central 09:00 → 14:00Z; January → 15:00Z). Aliases are word-boundary matched (`contains_word`) so "ct"/"et" don't fire inside words like "contact" or "meeting".
- **dtstart emit contract**: recognized tz → UTC `"%Y-%m-%dT%H:%M:%SZ"`; tz unknown-or-absent but a time is present → floating `"%Y-%m-%dT%H:%M:%S"` (no trailing `Z`); no parseable time → `None`, so the caller falls back to a date-only all-day `"%Y-%m-%d"` (existing all-day behavior). DST-ambiguous hours fall back from `.single()` to `.earliest()`.
- **`insert_detected` description column** (`src-tauri/src/db/calendar.rs`): added a `description: Option<&str>` param and a `description` column to the INSERT; `DetectedEvent` (`detect_events.rs`) gained a `description` field (the four pre-existing constructors set it `None`).
- **v38 backfill migration** (`src-tauri/src/db/schema.rs::migrate_v38_backfill_detected_events`, `CURRENT_VERSION` 37 → 38): re-runs `detect_events` over every already-synced message that has no calendar event yet (`LEFT JOIN message_bodies`, `WHERE NOT EXISTS` a `calendar_events` row) and inserts results ≥ 0.6 via `insert_detected`. Keyset-paginated by `messages.id` (PAGE=1000, not OFFSET) so mid-scan inserts can't shift a page window; each page commits in its own `unchecked_transaction`. Idempotent — the `NOT EXISTS` filter plus `insert_detected`'s existing-row guard prevent duplicates and preserve any user-set `dismissed` flag. Per-row insert errors are logged and skipped so one bad row can't abort the run. Unit test asserts the sam@harborline.example Zoom invite backfills to dtstart `2026-05-29T14:00:00Z` and re-runs without duplicating.

### Changed
- **Join URL routed into `description`, not `location`**: the meeting join URL is stored in `description` so the frontend's `extractMeetingLink` (`src/lib/meetingLink.ts`, scan order location → description → raw_ics) lights up the accent-colored "Join meeting" button (`CalendarEventCard.tsx`) without rendering an ugly raw URL — `location` would surface as a visible MapPin line, while `description` is scanned but not displayed. **No frontend changes were needed.**

## 2026-05-04 — Gmail-style threaded conversations

### Added
- **List collapse**: each conversation appears as a single row in the inbox/folder views. The badge shows the full thread size (folder-agnostic) so a 2-message thread with one Inbox + one Sent member still displays "2" in Inbox.
- **Reader thread stack** (`ReadingPane.tsx`): multi-message threads render as a vertical stack of cards (older collapsed, latest expanded). Clicking a collapsed card lazy-loads its body via `fetchBody` (cached in a 30-entry FIFO map), expands inline. Single-message threads render unchanged.
- **Quote-trim toggle** (`src/lib/quoteToggle.ts` + `ThreadMessageCard.tsx`): a "•••" pill below an expanded card body hides any trailing quoted-reply blob. Detection covers `<blockquote>`, `div.gmail_quote`, and the "On … wrote:" lead-in (en/es/fr/ru). The toggle is a React button **outside** the iframe — `EmailFrame` is unchanged, preserving the iframe sandbox posture.
- **Thread fan-out for actions**: archive (`e`), delete (`#` and Trash button), context-menu "Move to Trash", and unsubscribe → "Move to Trash" all expand to every thread member in the displayed folder before issuing the IPC. Cross-folder Sent copies are intentionally untouched (Gmail parity). New helper `src/lib/threadActions.ts::uidsForThreadAction`.
- **Mark-read fan-out**: opening a thread marks every unread sibling in the displayed folder as read on the server, and clears the representative row's `thread_has_unread` flag locally.
- **Aggregated unread state**: `MessageSummary.thread_has_unread` boolean on every list row. The list-item bold/dot styling treats `!is_read || thread_has_unread` as unread, so a read latest with an unread older sibling still highlights the row.
- **DB schema v29 migration** (`src-tauri/src/db/schema.rs`): adds `thread_root_id TEXT` column + `idx_messages_thread_root` index. Backfill runs in Rust (paged) via the new `compute_thread_root_id` helper because a single-pass SQL `SUBSTR/INSTR` cannot parse RFC 5322 whitespace-separated `<id> <id>` references.
- **Message-ID parsing helpers** (`src-tauri/src/email/message_id.rs`): `parse_first_message_id` accepts both bracketed (IMAP raw header) and unbracketed (mail-parser-stripped) shapes; `compute_thread_root_id` derives a stable per-account thread key. 13 unit tests.
- **`get_thread_uids_in_folder` IPC** + TS wrapper `api.messages.threadUidsInFolder` (`src/lib/tauri.ts`).

### Changed
- **List query rewrite** (`db/messages.rs::list_by_folder`, `list_all_inboxes`; `db/inbox_groups.rs::list_messages_for_group`): now uses an `eligible → keys → aggregates → ranked` CTE chain. Partitions by `(account_id, group_key)` so unified inbox never collapses across accounts. `total_count` from a folder-agnostic aggregate so the badge reflects the full thread; folder-eligible representative chosen by `is_pinned DESC, datetime(date) DESC, uid DESC`. SQLite window functions (≥ 3.25) required.
- **Mbox import** (`src-tauri/src/email/import.rs`): now passes `parsed.references` (re-bracketed and joined) into `insert_batch`. Imported conversations now thread correctly.
- **Get-thread query** (`db/messages.rs::get_thread`): now matches `thread_root_id` first; legacy reference-chain match retained as a fallback.
- **MessageRow / MessageSummary**: added `thread_root_id` and `thread_has_unread` fields.
- **`insert_batch` self-computes `thread_root_id`** so sync.rs and import.rs callers don't need to know about the new column.
- **`update_thread_counts_with_conn`** now also calls `update_thread_root_ids` to keep the column current on legacy inserts.

### Fixed
- **SQL injection invariant** (`db/messages.rs::validated_category`): the category string interpolated into list query SQL is now validated against the `EmailCategory` enum. Pre-existing in touched code; addressed during the CTE rewrite.
- **Cached-body selection inheriting stale loading/error**: `loadBody` now clears `isLoading`/`error` on a cache hit so a previously-failed selection doesn't carry its error screen forward to the next message.
- **`thread_has_unread` not clearing in store**: `markMessageRead` now also sets `thread_has_unread: false` on the matched row, so opening a thread immediately drops the bold/dot styling even if the unread member was an older sibling.

### Security
- `EmailFrame.tsx` is **unchanged**. Quote toggling is parent-controlled (React state changes the iframe srcDoc), so the iframe sandbox attribute and `FRAME_SCRIPT` contents do not deepen the existing pre-spec divergence.

## 2026-03-31 — Unified Primary Inbox, Account Groups, Sync Fixes

### Added
- **Default unified primary inbox** — app starts in unified inbox with "Primary" category pre-selected
- **Account group inbox filtering** — click group name in sidebar to view combined inbox for that group only
- Account group–aware category tabs and message counts
- Icon generation script (`src-tauri/icons/generate_icon.py`) with baked squircle mask, all standard macOS iconset sizes
- 15s IMAP connection timeout on all providers (Gmail, iCloud, Outlook)
- 45s per-account timeout in `sync_all_inboxes` — one hung account doesn't block others
- `catch_unwind` around all `parse_message` calls with sanitizer fallback to escaped plain text
- `in_reply_to` index migration (v21) for faster thread count queries
- Account groups — group accounts in sidebar with context menu
- Unsubscribe sender tracking with toast notifications for new mail from unsubscribed senders
- Reading pane retry button on message fetch failure
- Body prefetching during sync (up to 10 per account) for faster message loading
- SQLite busy timeout (5s) for multi-process resilience
- DB schema v19–v21 (account groups, unsubscribed senders, in_reply_to index)

### Fixed
- **Critical**: ammonia HTML sanitizer panicked on `<style>` tag (in both `tags` and `clean_content_tags`), crashing the tokio worker thread and causing infinite "Syncing..." state
- **Critical**: IMAP connections had zero timeouts — dead sockets hung the entire app indefinitely
- **Critical**: `parse_message` panics on certain HTML emails (UIDs 99740, 99724, 229, 2074) crashed sync; now caught by `catch_unwind` with plain-text fallback
- Overlapping rows in virtualized message list — added `getItemKey`, `measureElement` ref, and dedup on append
- Shift-click multi-select broken in unified inbox (UID collisions across accounts) — keyed by `account_id-uid`
- DB Mutex held too long during sync blocked `fetchBody` — split into small lock scopes
- New accounts never got folders synced (Sidebar called `list()` instead of `sync()`)
- Background sync and manual "Sync All" could run concurrently, doubling IMAP connections
- Auto-select first account no longer fires when unified inbox is active

### Changed
- Default initial state: unified inbox + primary category (was: no selection)
- Unified inbox shown for 1+ accounts (was: 2+)
- Sidebar group click → select group inbox; chevron click → collapse/expand (separated concerns)

## 2026-03-29 — AI, Productivity & Compose Enhancements

### Added
- AI email summarization with configurable provider
- AI writing assistant menu in compose modal (generate reply, rewrite, tone adjust)
- Smart replies powered by AI
- Email open tracking with pixel injection
- Snooze messages UI with custom date/time picker
- Snoozed messages list view
- Undo send with configurable delay (default 5s)
- Scheduled send view with edit/cancel
- Email categories with AI-powered auto-categorization (primary, updates, social, promotions)
- Follow-up reminders for sent emails with desktop notifications
- One-tap unsubscribe — detects List-Unsubscribe headers, supports RFC 8058 one-click POST, browser, and mailto
- Email templates — save and apply reusable compose templates per account or globally
- Compose attachments — file picker, drag-and-drop, attachment list with remove, 25 MB limit
- Attachment reminder — warns when email body mentions attachments but none are attached
- Calendar integration — parses ICS/text-calendar invites, displays event cards, RSVP via email
- Remaining IPC wiring for all command modules
- DB schema v3–v7 migrations (categories, follow-ups, tracking, templates, calendar)

### Changed
- Default all accounts to collapsed state in sidebar

### Fixed
- Tracker Dockerfile configuration
- Reverted title bar icon change back to CXMail text

## Earlier Development

### Phase 6: Polish
- App icon iterations (envelope design, rounded rectangle, dark corners)
- RFC 2047 decoder fix for encoded subjects
- Separated account click from collapse behavior
- Account renaming with double-click to edit
- Comprehensive CLAUDE.md documentation

### Phase 5: Plugin System
- QuickJS sandboxed runtime for plugins
- Plugin manifest loading and trigger system
- Example auto-tagger plugin

### Phase 4: MCP Server
- rmcp-based MCP server with 12 tools
- 3 permission tiers: open, confirm, approve
- Standalone binary sharing SQLite database

### Phase 3: Power Features
- Attachment download and inline image rendering
- Folder operations (create, rename, delete)
- Full-text search with Tantivy *(replaced by SQLite FTS5 in 2026-07; Tantivy is no longer a dependency)*
- Conversation threading
- PGP encryption/decryption support *(removed 2026-08-05 — see Removed under 1.0.0)*
- Bayesian spam filter
- JMAP protocol support
- Email import (mbox format)
- Mail rules engine

### Phase 2: Compose & Multi-Account
- Compose modal with TipTap rich text editor
- SMTP send with Gmail XOAUTH2 and iCloud LOGIN auth
- Reply, forward, reply-all with quoted original
- Draft saving to IMAP Drafts folder
- Unified inbox combining all accounts
- Multi-account UI with per-account folder trees and color coding
- Outlook account support (Microsoft OAuth2)
- iCloud account support (app-specific passwords)

### Phase 1: Foundation
- Tauri 2 + React 19 + TypeScript project scaffold
- SQLite schema with migration system
- Unified error types and AppState
- macOS Keychain wrapper (later replaced by file_store for dev)
- OAuth2 PKCE flow for Gmail
- IMAP client with XOAUTH2 auth
- MIME parser and HTML sanitization (ammonia)
- Database CRUD operations
- Complete React frontend
- All IPC commands for accounts, folders, and messages

### Infrastructure
- OAuth2 debugging: token exchange logging, URL-decode auth code, event emission
- IMAP debugging: manual server greeting, tokio-native-tls compat layer, spawn_blocking for async-std
- Switched from macOS Keychain to file-based credential store for dev builds
