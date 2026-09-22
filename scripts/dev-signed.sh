#!/bin/bash
# Dev loop that does NOT fire Keychain prompts.
#
# `npx tauri dev` is unusable on a machine with connected Google Calendar
# grants: it runs `target/debug/cxmail`, which cargo leaves ad-hoc
# ("linker-signed") with no TeamIdentifier. That fails two of gotcha #31's
# three conditions, and lib.rs's 30-second tick calls `is_gcal_connected` for
# every Gmail account — two Keychain-only keys each — so you get a dialog
# storm that Deny does not stop.
#
# The fix is the same one ship.md step 2b applies to cxmail-mcp: sign the loose
# binary with the Developer ID. The Keychain ACL stores a **designated
# requirement** (identifier + team OU), not a path, so a signed debug binary
# matches the entry the installed app already has — no new grants, no prompts.
#
# `--identifier com.cxmail.app` is load-bearing: for a bare Mach-O, codesign
# defaults the identifier to the FILENAME ("cxmail"), which does not satisfy
# the bundled app's requirement. Verified 2026-08-05.
#
# Why not just `tauri dev`? Every Rust rebuild re-links and drops the
# signature, so it must be re-applied after each build — which is what this
# script does. Frontend changes still hot-reload through Vite untouched.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IDENTITY="Developer ID Application: Christopher Robinson (CCYV5HQZCM)"
BIN="$REPO/src-tauri/target/debug/cxmail"

source "$HOME/.cargo/env"

# Two instances fighting over one SQLite file is gotcha #12.
if pgrep -if "/Applications/cxmail\.app/Contents/MacOS/cxmail$" >/dev/null 2>&1; then
  echo "==> Quitting the installed app (gotcha #12: shared SQLite file)"
  osascript -e 'quit app "cxmail"' || true
  sleep 2
fi
HELPER_PLIST="$HOME/Library/LaunchAgents/com.cxmail.app.helper.plist"
HELPER_WAS_LOADED=0
if launchctl list 2>/dev/null | grep -q com.cxmail.app.helper; then
  echo "==> Unloading the helper daemon (KeepAlive would respawn it)"
  launchctl unload "$HELPER_PLIST" || true
  HELPER_WAS_LOADED=1
fi

# Put it back on the way out. Without this every dev session ends with the
# user's background sync silently switched off — it stays off until they
# notice mail has stopped arriving, which is not a symptom that points here.
restore_helper() {
  if [ "$HELPER_WAS_LOADED" = "1" ]; then
    echo "==> Restoring the helper daemon"
    launchctl load "$HELPER_PLIST" 2>/dev/null || true
  fi
}
trap restore_helper EXIT

# A previous dev build is the same two-writers hazard and was never guarded.
if pgrep -f "src-tauri/target/debug/cxmail$" >/dev/null 2>&1; then
  echo "==> Killing a previous dev build (gotcha #12: shared SQLite file)"
  pkill -f "src-tauri/target/debug/cxmail$" || true
  sleep 1
fi

echo "==> Building debug binary"
# The Google OAuth client is not in the source tree (secrets.rs explains why);
# a dev build takes it from the Keychain so Gmail sign-in works locally.
( cd "$REPO/src-tauri" && secret run -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET -- cargo build --bin cxmail )

echo "==> Signing (sign a COPY and mv — never in place, gotcha #31)"
cp "$BIN" "$BIN.signing"
codesign --force --timestamp --identifier com.cxmail.app --sign "$IDENTITY" "$BIN.signing"
mv -f "$BIN.signing" "$BIN"

# Prove it satisfies the same ACL entry as the installed app. If this fails,
# the run WILL prompt — stop rather than subject the user to the storm.
REQ="$(codesign -d -r- /Applications/cxmail.app/Contents/MacOS/cxmail 2>/dev/null | sed -n 's/^designated => //p')"
if [ -n "$REQ" ]; then
  if codesign --verify -R="$REQ" "$BIN" 2>/dev/null; then
    echo "==> OK: matches the installed app's designated requirement (no prompts expected)"
  else
    echo "!!  Signed binary does NOT satisfy the installed app's requirement." >&2
    echo "!!  Running it would fire a Keychain dialog every 30s. Aborting." >&2
    exit 1
  fi
else
  echo "==> No installed app to compare against; skipping the requirement check"
fi

echo "==> Starting Vite (frontend hot-reloads; only Rust changes need a re-run)"
( cd "$REPO" && npm run dev ) &
VITE_PID=$!
trap 'kill $VITE_PID 2>/dev/null || true; restore_helper' EXIT

for _ in $(seq 1 40); do
  if curl -sf http://localhost:5173 >/dev/null 2>&1; then break; fi
  sleep 1
done

echo "==> Launching signed debug build"
cd "$REPO/src-tauri"
RUST_LOG=${RUST_LOG:-info} "$BIN"
