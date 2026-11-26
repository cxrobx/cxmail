#!/bin/bash
# Install a new phone send helper (CXMAIL_SEND_BIN) for CXMail Remote.
#
#   remote/install-send-helper.sh <signed cxmail-mcp>
#
# The phone's draft review, edit and send run this helper, not the desktop app,
# so a helper built without the Google client works until a Gmail token needs
# renewing and then fails every review (gotcha #70). This refuses that binary,
# refuses one signed differently from the current helper (a Keychain prompt on
# every read, gotcha #31), keeps the old file in a dated rollback-send-* folder
# and swaps with mv, so a running send keeps the old file.
#
# Build the binary with the Google client first:
#   secret run -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET -- \
#     cargo build --release --bin cxmail-mcp
# then codesign it with the Developer ID (default identifier cxmail-mcp).
#
# The helper path comes from CXMAIL_SEND_BIN, else from the Remote LaunchAgent.
# With neither, Remote uses the desktop app's own cxmail-mcp and there is
# nothing to install.
set -euo pipefail

NEW=${1:?usage: remote/install-send-helper.sh <signed cxmail-mcp>}
PLIST="$HOME/Library/LaunchAgents/com.cx.cxmail-remote.plist"
DEST=${CXMAIL_SEND_BIN-$(/usr/libexec/PlistBuddy -c 'Print :EnvironmentVariables:CXMAIL_SEND_BIN' "$PLIST" 2>/dev/null || true)}
if [[ -z "$DEST" ]]; then
  echo "Remote has no separate send helper; it uses the desktop app's cxmail-mcp. Nothing to do."
  exit 0
fi
[[ -x "$NEW" ]] || { echo "Not an executable: $NEW" >&2; exit 1; }

# 1. It must be able to renew Gmail access.
google_ok() {
  "$1" remote build-info </dev/null 2>/dev/null | /usr/bin/python3 -c '
import json, sys
lines = sys.stdin.read().strip().splitlines()
try:
    sys.exit(0 if json.loads(lines[-1]).get("google_oauth") is True else 1)
except Exception:
    sys.exit(1)'
}
google_ok "$NEW" || {
  echo "REFUSED: $NEW has no Google sign-in client (or predates 'remote build-info')." >&2
  echo "Rebuild it under: secret run -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET -- cargo build --release --bin cxmail-mcp" >&2
  exit 1
}

# 2. It must be signed the way the Keychain already trusts.
if [[ -e "$DEST" ]]; then
  REQ=$(codesign -d -r- "$DEST" 2>/dev/null | sed -n 's/^designated => //p' || true)
  [[ -n "$REQ" ]] || { echo "REFUSED: can't read the current helper's signing requirement." >&2; exit 1; }
  codesign --verify -R="$REQ" "$NEW" 2>/dev/null || {
    echo "REFUSED: $NEW is not signed like the current helper (codesign it with the Developer ID, identifier cxmail-mcp)." >&2
    exit 1
  }
fi

# 3. Keep the old one, then swap.
DIR=$(dirname "$DEST")
if [[ -e "$DEST" ]]; then
  BACK="$DIR/rollback-send-$(date +%Y%m%d-%H%M%S)"
  mkdir "$BACK"
  cp -p "$DEST" "$BACK/"
  echo "Old helper kept in $BACK"
fi
cp "$NEW" "$DEST.new"
mv -f "$DEST.new" "$DEST"

google_ok "$DEST" || { echo "Installed helper failed its check; restore it from $BACK" >&2; exit 1; }
echo "Send helper installed: $DEST"
