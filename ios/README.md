# CXMail for iPhone

A native SwiftUI companion for CXMail Remote. The Mac remains the mail engine:
credentials, IMAP sync, search, sending and Claude stay there. This app uses the
existing authenticated, tailnet-only HTTPS API; there are no mailbox credentials
or new authentication services on the phone. The web client remains available.

## Build

Requires Xcode 27.1+ for the Duo APIs and XcodeGen. Runs on iOS 18+.

1. Start the existing service with `remote/install.sh` from the repository root.
2. In `ios/`, run `bash scripts/configure.sh`. This reads the local Tailscale
   hostname and creates ignored `Config/Local.xcconfig`, then generates the project.
3. For signing, set `CXMAIL_DEVELOPMENT_TEAM` when configuring, or select your
   team in Xcode. Open `CXMail.xcodeproj`, select your device, and build.
   `project.yml` is the project source; regenerate after adding files.

The phone needs Tailscale connected under an allowed user. HTTPS certificate
validation is never bypassed. There is no LAN listener, alternate public endpoint,
or new server to keep running.

For direct Duo installation, `bash scripts/device.sh` discovers the physical Duo
by product type, builds Release, installs and launches. If Xcode has no signed-in
account, set `CXMAIL_ASC_KEY_PATH`, `CXMAIL_ASC_KEY_ID`, and
`CXMAIL_ASC_ISSUER_ID` to an existing provisioning key. It stays outside this repo.
`bash scripts/device.sh build` builds without installing.

## Behavior

- Inbox categories, Unread, Drafts, Sent, per-account selection and search.
- Native message navigation, swipe/context actions, archive, star, read/unread,
  move and recoverable Trash; linked conversation messages.
- Plain-text composing/reply/reply-all, local recovery, save on the mini, and
  review-before-send. From/To/Cc/Bcc and body are fetched from the mini; sending
  requires separate confirmation and the exact reviewed digest. No automatic
  send retry after an ambiguous connection failure.
- If a draft was edited, sent or removed elsewhere, an authoritative missing
  revision clears that obsolete reference from all phone caches and refreshes
  Drafts. Choose the current draft for a fresh review. Network failures retain
  the draft and its retry action; locally typed replies are preserved.
- Compact glass send controls, Send now and Schedule send with an explicit final
  confirmation. The mini persists schedules while the phone is closed; the clock
  in Drafts opens pending sends, cancellation and delivery outcomes.
- Claude chat on the mini with streaming, tool details, permission requests,
  interruption and new-chat controls. Closing the sheet pauses polling without
  killing the conversation. Returning resumes it. This shares the server's one
  remote chat slot with the PWA.
- Recent inboxes and up to 100 opened messages are available offline. Local mail
  and recovered replies have iOS data protection and are excluded from backups.
  No offline mail mutations or phone-side send queue; failures are explicit.

HTML is already sanitized by the mini, then rendered in an ephemeral WKWebView
with email JavaScript disabled, a restrictive CSP, blocked remote images and
external links handed to the OS. Opted-in images are not persisted as an opt-in.
Original HTML colors are preserved, including white email paper in dark mode.
Responsive mail keeps its natural size. Fixed-width tables and other overflowing
layouts scale down to the available reader width, including after rotation or a
pane resize. A constant app-authored measurement runs in WebKit’s isolated client
world; it does not enable email scripts or change network permissions.
When a body is missing locally, the mini fetches sanitized HTML through its
existing signed helper. MCP metadata is never treated as message content. A
missing body gets a retry state, and the old metadata-bearing phone cache is
discarded without touching recovered replies.

Native Liquid Glass on iOS 26+ provides the account/filter surface and compact
floating action capsules; earlier systems use a material fallback. The list is
dense and unboxed. The icon uses the actual desktop CXMail artwork, adapted to
the opaque iOS mask by `scripts/icon.swift`.

## Appearance

Settings → Appearance offers System, Light and Dark. The choice applies immediately
to native app screens and sheets and persists on this device; System follows iOS.
Email documents retain their sender’s original colors. UI fixtures use a separate
preference so tests cannot change the normal app setting.

Inbox rows have a full rectangular tap target. Pending read acknowledgements do
not disable navigation or scrolling; mutation controls retain their pending guard.

## Sender image settings

The small sender icon opens authentication details: Domain authenticated, failed,
or unavailable. This is From-domain authentication, not brand/content verification.
The image-settings menu can always trust or stop trusting that exact sender using
the mini's shared desktop allowlist. Automatic loading additionally requires fresh
passing authentication and no Medium/High risk/Junk flag. Unsupported providers,
missing evidence and offline views retain manual image loading. Show images is
one-time consent; unblocked remote-image bodies are never persisted for offline use.

## Scheduled send

From a reviewed draft, use the chevron next to the paper plane → **Schedule
send…**. Choose a preset or custom date/time, review all recipients including Bcc,
then confirm. The schedule uses the phone's timezone. The mini must be awake and
online; a sleeping mini picks up the send after waking. Changes to the reviewed
draft stop delivery. Pending schedules can be cancelled without deleting the
draft. An uncertain outcome requires checking Sent before any manual retry.
These companion schedules are separate from desktop compose-window schedules.

## Chat

The mailbox has separate glass chat and compose icons. The chat uses T3's input
and transcript patterns: a multiline composer with send/stop inside its lower
edge, quiet user bubbles, formatted assistant headings/lists/quotes/code, copy
actions and collapsed tool activity. Mail permissions remain explicit.

The header can dock chat beside mail on the open Duo, with the email on the left
and chat on the right. Drag the divider or use its accessibility adjustment to
resize; both panes retain at least 300pt. While a vertical fold is active the
division follows the hinge instead. Pop-out returns chat to its floating window
and restores the normal mail layout. A narrow window also returns to floating
chat. Conversation, permission state, unsent input and split preference are kept
above these presentation changes.

Tap the composer's context label to choose “Your mail” or “Current email”. Current
email follows the message open in the reader and shows its subject. Choosing a
scope changes the next request without restarting Claude or clearing history or
unsent text. If no message is open, email context is unavailable; an existing
email-scoped draft requires selecting a message or choosing Your mail before send.
The native client supplies the exact account/folder/UID reference per turn, keeping
that transport context out of the user's visible message bubble. Prior turns remain
in the conversation; this selector does not erase history or change mail permissions.

## Duo layout

The layout uses available geometry, not a device-name or size-class guess.
Closed upright (382 usable points) and closed sideways use mailbox-to-message navigation.
Open tall starts with a full-width reader and a closed inbox drawer. The sidebar
button opens the inbox over the message; selecting an email closes it again.
Open wide becomes inbox + reader. Flat, the inbox takes about 38% of the usable
width, leaving most of the space for the email. On iOS 27.1+, an active division
region supplies the fold gap and pane boundary while the device is partly folded.
Outdated division geometry during rotation is rejected when a pane cannot fit.
The native split container owns each column's safe area, so the status column
does not reserve another blank strip inside the inbox. System safe areas protect
the asymmetric status column and home indicator. Inbox filters begin with “All”.
Selection, composer and chat state sit above layout changes.

## Verification

```sh
bash scripts/verify.sh <compact-duo-simulator-udid> <ipad-simulator-udid>
```

The compact suite covers reading rendered HTML, reply recovery after relaunch,
compose/review/confirmed-send, stale-draft refusal, Claude permission denial,
search, empty and offline states, rotation, and a fictional fixed-width email’s
right edge and footer. Native WebKit tests assert fitted bounds, unchanged
responsive/wide scaling, disabled email scripts and the image CSP. The separate iPad run constrains
the host to the Duo's 951×669 wide or 669×951 tall window and injects the measured
side/top and bottom safe areas, plus the active 40-point fold in the wide profile.
The iPad retains its own system chrome. Tests compare actual rendered email bounds;
native navigation backgrounds can extend behind both columns. This is a controlled
geometry test, not a claim of physical hinge automation.
Unit tests cover API boundaries, identity, recipient handling, timeout contracts,
cache persistence, optimistic rollback, no automatic send retries and chat events.
Fixture API shapes are taken from `remote/server.py`, `remote/test_server.py` and
the signed helper's preview contract. Fictional data uses example domains only.

Tests launch with `--uitesting`; this selects an in-process fake transport and a
separate cache. No fixture request can reach real mail. `--reset` clears only the
fixture cache. These switches are compiled out of Release builds.

Use simulator appearance and content-size settings for light/dark and large-text
captures. Screenshots and recordings are attached to the test result bundles;
build products/results live in `/private/tmp/cxmail-ios-dd` by default.

Initial verification 2026-11-15: 12 unit tests, 7 compact UI flows, the exact open-width UI
profile, and all 32 existing remote API regressions passed. Light/dark and XXXL
captures were inspected; the reply/recovery flow was recorded. A signed Release
build was installed on the physical Duo and loaded the live mini mailbox.
Physical hinge transitions and sustained device frame rate remain manual checks.

The polish pass adds body-load failure/retry, realistic missing-snippet density,
and flat-Duo reader-width UI checks, plus four API regressions for uncached HTML,
plain-text envelope removal, empty cached bodies and explicit fetch errors.
It passes 12 unit checks, 9 compact UI flows, the partial-fold/flat open-width
flow, and 36 API regressions. Build 2 uses CXMail's actual icon and native glass.

Build 3 replaces the manual split with native column containment, puts “All”
first, and adds the initially closed portrait inbox drawer. Verification covers
12 unit checks, 10 compact UI flows, both open-window profiles and 36 API checks.
The open profiles assert the actual fold boundary, clear division, full-width
portrait reader, 400pt drawer, selection and explicit open/close controls.
Light/dark, XXXL text and recorded drawer transitions were inspected. The signed
Release update was installed on the Duo; a physical UI run and automatic launch
were blocked by the phone being locked.

Build 4 adds the T3-style chat composer/transcript, separate icon actions and
resizable mail/chat docking. Verification passed 14 unit checks, 11 compact UI
flows, 3 open-window flows and 36 API checks. Extended large-text checks retain
an existing conversation and draft through docking, divider dragging, keyboard
input and pop-out, and verify the restored mail toolbar. Light/dark captures and
recorded transitions were inspected. The signed Release update was installed
and launched on the Duo.

Build 5 adds selectable per-turn mail/current-email context. The 16 unit checks,
compact context/history/draft checks and 36 API regressions pass. A simulator
startup failure passed on retry. The fresh open-layout rerun was interrupted by
the build guard when the mini's machine load exceeded 150; it needs repeating
when the machine is quiet. The signed update was installed and launched on the
Duo without bypassing the guard.

Build 6 replaces the full-width review CTA with compact glass send/options
controls and adds persistent companion scheduling, date/time selection and
cancellation. Verification passed 16 unit checks, 13 compact UI flows and four
open-window flows (the new schedule flow passed after correcting its iPad test
orientation and nested tab target), plus 45 fixture API/reliability checks.
Light/XXXL and dark schedule flows passed; their captures and the recorded flow
were inspected. The live authenticated HTTPS queue endpoint was verified without
mutating mail. The signed Release update was installed and its build number
confirmed on the Duo. Launch was refused because the phone was locked; open
CXMail manually after unlocking.

Build 7 handles confirmed missing draft revisions with an inline Drafts update,
invalidates their phone caches and refreshes the list. Connection failures remain
retryable and preserve the draft; local composer recovery is unaffected. The
service persists cache-only exclusions and invalidates them when the cached
revision or UIDVALIDITY changes. Verification passed 18 unit checks, 15 compact
UI flows, five open-window flows and 49 fixture API checks. Light/XXXL recovery
and the recorded transitions were inspected. Three failing live references were
confirmed obsolete and excluded without sending, editing or deleting any mail.
The signed Release build 7 was installed on the Duo.

Build 8 shares sender image trust with desktop and adds From-domain authentication
indicators. Automatic images require current Google authentication, exact sender
trust and no Medium/High risk/Junk flag; the final body-response permission and
request generations prevent stale decisions from enabling images after revocation.
Verification passed 21 unit checks, 17 compact UI flows (one unrelated orientation
case skipped), five existing wide flows, and the two sender-policy flows on both
compact and wide simulators. The 58 API checks, 615 frontend checks, 997 Rust checks,
real-browser request blocking, read-only cached-mail compatibility probe and design
preflight passed. Screenshots were inspected. After explicit swap approval, both
signed builds were installed on the mini and physical Duo. Desktop app, helper,
shared MCP and remote service were restarted and their installed binaries checked.
The live authenticated tailnet HTTPS API returned the domain-authentication policy
and matching final image-permission header without mail/trust mutations. Duo build
8 was confirmed installed; launch was refused because the phone was locked, so
CXMail must be opened after unlocking. The previous desktop bundle is preserved
in a private local rollback directory.

Build 9 fits fixed-width email layouts to the actual native reader width and
refits after rotation or pane resizing, including the scaled document height and
padding. Responsive emails retain their normal size. Verification passed 25 unit
checks, 18 compact UI flows (one unrelated orientation case skipped), eight wide
UI flows, all 58 API checks and design preflight. The additional light/XXXL and
normal dark footer checks passed; captures confirm the complete email can scroll
clear of the floating toolbar. Fitting uses constant host code in the isolated
client world while email JavaScript remains disabled and the image CSP is intact.
The signed Release build 9 was installed and launched on the physical Duo.

Build 10 adds a device-local System/Light/Dark preference in Settings and makes
blank row space tappable. Pending mark-read acknowledgements no longer disable
opening mail or scrolling the reader. A delayed-acknowledgement fixture checks a
single blank-area tap, reading and reopening. Validation passed 25 unit checks,
20 compact UI flows (one unrelated orientation case skipped), eight existing wide
flows and the new appearance/persistence flow on wide. The wide appearance test
was rerun after correcting it to open the initially hidden Inbox sidebar. The
additional XXXL appearance flow, all 58 API checks and design preflight passed.
Light/dark captures were inspected; fixture preferences are separate from normal
app preferences. Email documents retain their original colors. Signed Release
build 10 was installed and launched on the physical Duo.

Build 11 stabilizes search navigation against keyboard-reduced height and adds
hybrid native search. Fast keyword matches appear before the shared Rust engine
returns BM25/document/semantic rank fusion, Gmail-copy deduplication and stable
cursor pagination. Stale query responses are rejected during the debounce window.
Meaning-search coverage/failure is visible; exact phrases/exclusions remain exact.
“Most recent XRF meeting notes” treats the opening phrase as ordering intent.
The local vault embedding service is supported without a new external provider.

Validation: the full compact suite (25 unit/WebKit checks and 20 UI flows, with one
unrelated orientation skip), ten wide UI flows, and a final 27-unit/three-search-flow
run passed. The actual-window and constrained-width search tests retain typed text
and focus. Both before-fix simulator variants also passed, so this does not claim
a reproduction of the physical Duo's reported flicker; confirm on the installed
Duo. The signed Release build 11 was installed and launched successfully. Remote
API, Rust workspace, frontend checks/build and the fiction-only live local
embedding relevance/null-case probe passed. The approved independent review identified passage-exclusion leakage and
queued-search starvation; build 12 corrects both.

Build 12 keeps results visible while typing, uses the same Rust engine for the
fast keyword pass, supersedes obsolete queued queries, retries busy search once,
and refreshes expired ranked cursors without marking the mini offline. The
keyboard-height guard applies only to undocked Search; docked chat retains normal
keyboard avoidance and its test measures the composer above the software keyboard.
Model cold starts have a bounded allowance. Whole-email exclusions, latest exact
matches, malformed helper results and smaller local indexing batches are covered
by the review follow-up checks. Final checks passed: 30 unit/WebKit checks, 20 compact UI flows (one unrelated
orientation skip), 11 wide flows, and a final light/XXXL search run. The revised
docked-chat test proves the composer remains above the software keyboard. All 64
remote API checks, 1,004 Rust workspace checks, frontend tests/build and the design
preflight passed. A live fictional 16-passage test verifies shared two-at-a-time
GPU submission, and the optimized synthetic 44k-vector scan took 0.78 seconds.
The authenticated typing-burst probe completed the final query with hybrid
results; the unrelated probe returned none. Indexing resumed beyond its former
GPU-failing batch. Captures were inspected; the simulator recording contained
blank compositor frames, so physical transition/frame-rate behavior remains a
manual check. Signed build 12 was installed; automatic launch was refused because
the Duo was locked. Open CXMail after unlocking. The desktop compatibility bundle
was installed after explicit approval. App, mail helper and shared MCP were
restarted from `/Applications`; the phone service now uses its bundled helper.
Installed executable hashes and signatures matched the prepared bundle. Prior
bundles were preserved for rollback. Final live keyword/hybrid/no-match checks
passed. As indexing expanded, a weak semantic-only hit at 0.6535 crossed the
initial gate; the shared model-specific gate was tightened to 0.70 and verified
against seven fictional positive cases and 36 unrelated pairs, including long
mail and similar-topic distractors. The approved desktop update was rebuilt and
reinstalled with that correction. Physical Duo typing remains the manual check.

Build 13 adds a keyboard-collapse icon beside Settings in the Search account bar.
It is visible while the search field has focus and resigns that focus without
cancelling search, clearing the query or discarding results. Tapping the field
resumes typing. Native search does not surface SwiftUI's keyboard accessory
items reliably, so the control lives in the safe, persistent account bar instead.
The keyboard-collapse UI check proves visibility/hit testing, keyboard dismissal,
query/result retention and refocusing on both compact and wide profiles.
Native results default to newest days first, with relevance retained within a day;
the service applies the order before cursor pagination.

The broad checks also reproduced a pre-existing Duo composer issue against the
previous RootView: focusing its fields could remove Review and Cancel from the
sheet. Moving its keyboard-dismiss action from the
keyboard accessory to the navigation toolbar restores those actions. The compose
regression retains its complete recipient review and explicit fictional send,
and now checks dismissal, body retention, refocusing and Review hit testing too.

Build 13 validation passed: 30 unit/WebKit checks, the compact profile's 20 UI
checks (one orientation-only skip), all 11 wide-profile cases across the initial
run and focused reruns, and all 68 API checks. The shared iPad's keyboard failures
also reproduced with the previous RootView. An isolated iPad passed docked chat;
after a build-guard load interruption, both wide-search checks passed with the
software keyboard explicitly enabled, including XXXL text and keyboard collapse
in both the real wide window and the fixed Duo viewport. `verify.sh` now disables
the simulator hardware-keyboard connection temporarily and restores its previous
preference on exit; its shell syntax and the same setup/restore sequence were
checked. The design preflight passed and the signed build was installed on Duo.

Build 14 removes the always-visible cache-age footer ("Updated …" / "Saved …")
from mailboxes and Search. Cache timestamps remain internal; connection errors
and their retry action still appear when needed.
Validation passed for all 30 unit/WebKit checks, 20 compact UI checks (one
orientation-only skip), 10 of the 11 wide UI checks including both search and
keyboard-collapse flows, and all 68 API checks. The existing docked-chat fixture
keyboard-position assertion failed again, as it did against the previous
RootView; this patch leaves that test and chat layout unchanged. The Search
captures were inspected to confirm the footer is gone and results are retained.
The signed build was produced in the laptop's Aqua session using a temporary
copy of the mini's Xcode 27.2 (the laptop's default 27.0 lacks `reservedRegions`),
verified again on the mini, and installed/launched on Duo. The laptop checkout
and installed Xcode were unchanged; the temporary job and build tools were
removed afterward. The design preflight passed.

Build 15 moves Search's keyboard-dismiss action from the account strip into a
native trailing toolbar item, allowing the Duo to place it in the right rail.
Search keeps toolbar content visible while focused. Other devices use their
native navigation-bar placement. The Duo search regression checks the action is
right of the account controls, above the keyboard and hittable; all search
touch checks also prove dismissal preserves the query/results without opening a
message underneath the rail. The actual wide window repeats those checks with
XXXL text. The fixed Duo viewport retains continuous-typing/navigation coverage;
its top toolbar can be panned offscreen by iPad's keyboard, so toolbar hit testing
belongs in the actual resizable window.
The dedicated Duo rail test and full compact profile passed (30 unit/WebKit
checks, 20 UI checks and one orientation-only skip). Ten of 11 wide cases passed
across the initial run and focused reruns, including native-wide XXXL dismissal
and the appearance case after a SpringBoard launch crash. The pre-existing
docked-chat fixture position assertion remains failing. All 68 API checks and
the design preflight passed; captures were inspected in both appearances. Signed
build 15 was installed and launched on Duo without interrupting other test runs.

Build 16 gives the open split window’s detail navigator ownership of Search’s
keyboard-dismiss action, so it appears on the trailing side of the whole window
instead of the left mailbox header. Search focus and keyboard visibility live
above the adaptive branches. Floating Chat and Compose actions hide while the
software keyboard is exposed and return after dismissal. Compact and native-wide
search regressions check the trailing position, hidden/restored actions and
retained query/results. Full-screen captures avoid Xcode’s rotated app-window
crop and were inspected alongside the wide flow recording. The compact search,
native-wide XXXL search and fixed Duo typing checks passed, as did the compact
send/schedule regression and design preflight. Full verify runs were interrupted
by simulator startup/host load failures; they are not a clean full-matrix pass.
The 68 API checks had one timing-sensitive chat failure that passed in isolation.
The signed build was created from an isolated snapshot excluding the concurrent
draft-link work, verified, then installed and launched on Duo.

## Current boundaries

Attachments are listed; opening them or sending drafts containing them still
requires the Mac. No rich-text editor, share extension or
undo-send delay in this version. Draft creation matches the current API's To/Cc
surface; review still shows Bcc on drafts created elsewhere. Drafts are never
silently sent or discarded when connectivity changes.

## Agent draft links

Agent-created drafts can open directly in the native review screen through HTTPS
universal links. Deploy/configure the metadata host and Associated Domains profile
before installing: see [draft links](../docs/draft-links.md). The mini remains the
only mail engine, and every send still requires recipient review and confirmation.

## Smart notifications

Settings → Smart notifications enables native Apple Push alerts and manages a
separate exact-address trusted sender list. New mail from those addresses alerts
without a model call; other mail uses the mini's privacy-gated Luna Decisions
classification for reply/action need and urgency. Known junk and risk flags,
hidden accounts and read mail are excluded. Existing mail is never backfilled.
Generic alerts hide sender/subject; tapping opens the exact message. Turning off
also disables delivery on the mini; offline changes show an error until synced.
The mini must be awake and syncing. See `../remote/README.md` for setup, initial
thresholds, caps, and best-effort delivery limits. The signing profile must include
Push Notifications for this app ID. No phone-side provider key is added.
