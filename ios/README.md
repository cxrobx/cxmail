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
- Claude chat on the mini with streaming, tool details, permission requests,
  interruption and new-chat controls. Closing the sheet pauses polling without
  killing the conversation. Returning resumes it. This shares the server's one
  remote chat slot with the PWA.
- Recent inboxes and up to 100 opened messages are available offline. Local mail
  and recovered replies have iOS data protection and are excluded from backups.
  No offline mail mutations or queued sends; failures are explicit.

HTML is already sanitized by the mini, then rendered in an ephemeral WKWebView
with JavaScript disabled, a restrictive CSP, blocked remote images and external
links handed to the OS. Opted-in images are not persisted as an opt-in.
Original HTML colors are preserved, including white email paper in dark mode.
When a body is missing locally, the mini fetches sanitized HTML through its
existing signed helper. MCP metadata is never treated as message content. A
missing body gets a retry state, and the old metadata-bearing phone cache is
discarded without touching recovered replies.

Native Liquid Glass on iOS 26+ provides the account/filter surface and compact
floating action capsules; earlier systems use a material fallback. The list is
dense and unboxed. The icon uses the actual desktop CXMail artwork, adapted to
the opaque iOS mask by `scripts/icon.swift`.

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
search, empty and offline states, and rotation. The separate iPad run constrains
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

## Current boundaries

Attachments are listed; opening them or sending drafts containing them still
requires the Mac. No rich-text editor, push notifications, share extension or
undo-send delay in this version. Draft creation matches the current API's To/Cc
surface; review still shows Bcc on drafts created elsewhere. Drafts are never
silently sent or discarded when connectivity changes.
