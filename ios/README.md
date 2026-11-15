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

## Duo layout

The layout uses available geometry, not a device-name or size-class guess.
Closed upright (382 usable points), closed sideways and open tall stay one pane.
Open wide becomes inbox + reader. On iOS 27.1+, the measured division region,
including an inactive fold when flat, supplies the gap and pane boundary.
Outdated division geometry during rotation is rejected when a pane cannot fit.
System safe areas protect the asymmetric status column and home indicator.
Selection, composer and chat state sit above layout changes.

## Verification

```sh
bash scripts/verify.sh <compact-duo-simulator-udid> <ipad-simulator-udid>
```

The compact suite covers reading rendered HTML, reply recovery after relaunch,
compose/review/confirmed-send, stale-draft refusal, Claude permission denial,
search, empty and offline states, and rotation. The separate iPad run constrains
the app to the open Duo's exact 867×635 usable viewport with its 40-point fold.
That is a controlled geometry test, not a claim of physical hinge automation.
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

Verified 2026-11-15: 12 unit tests, 7 compact UI flows, the exact open-width UI
profile, and all 32 existing remote API regressions passed. Light/dark and XXXL
captures were inspected; the reply/recovery flow was recorded. A signed Release
build was installed on the physical Duo and loaded the live mini mailbox.
Physical hinge transitions and sustained device frame rate remain manual checks.

## Current boundaries

Attachments are listed; opening them or sending drafts containing them still
requires the Mac. No rich-text editor, push notifications, share extension or
undo-send delay in this version. Draft creation matches the current API's To/Cc
surface; review still shows Bcc on drafts created elsewhere. Drafts are never
silently sent or discarded when connectivity changes.
