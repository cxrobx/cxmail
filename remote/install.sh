#!/bin/bash
# Install (or reinstall) CXMail Remote on this Mac as a LaunchAgent, and publish
# it to the tailnet over HTTPS with Tailscale Serve.
#
#   remote/install.sh            install / restart
#   remote/install.sh uninstall  stop it and remove the Serve entry
#
# The only login allowed in is this Mac's own Tailscale user, read at install
# time and written to the LaunchAgent (never to the repo).
set -euo pipefail

LABEL=com.cx.cxmail-remote
PORT=${CXMAIL_REMOTE_PORT:-18706}
HERE="$(cd "$(dirname "$0")" && pwd)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/cxmail-remote.log"
TS=$(command -v tailscale || echo /Applications/Tailscale.app/Contents/MacOS/Tailscale)

if [[ "${1:-}" == "uninstall" ]]; then
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  "$TS" serve --https="$PORT" off 2>/dev/null || true
  echo "CXMail Remote removed."
  exit 0
fi

SEARCH_BIN=${CXMAIL_SEARCH_BIN:-/Applications/CXMail.app/Contents/MacOS/cxmail-mcp}
[[ -x "$SEARCH_BIN" ]] || { echo "Missing signed search helper: $SEARCH_BIN" >&2; exit 1; }
# A standalone send helper can be updated without replacing the desktop app.
# Preserve an existing override when reinstalling the Remote service.
SEND_BIN=${CXMAIL_SEND_BIN-$(/usr/libexec/PlistBuddy -c 'Print :EnvironmentVariables:CXMAIL_SEND_BIN' "$PLIST" 2>/dev/null || true)}
[[ -z "$SEND_BIN" || -x "$SEND_BIN" ]] || { echo "Missing signed send helper: $SEND_BIN" >&2; exit 1; }

LOGIN=${CXMAIL_REMOTE_LOGINS:-$("$TS" status --json | /usr/bin/python3 -c '
import json, sys
s = json.load(sys.stdin)
print(s["User"][str(s["Self"]["UserID"])]["LoginName"])')}
[[ -n "$LOGIN" ]] || { echo "Could not read the Tailscale login." >&2; exit 1; }

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>/usr/bin/python3</string><string>$HERE/server.py</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>CXMAIL_SEARCH_BIN</key><string>$SEARCH_BIN</string>
    <key>CXMAIL_REMOTE_PORT</key><string>$PORT</string>
    <key>CXMAIL_REMOTE_LOGINS</key><string>$LOGIN</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict>
</plist>
EOF
if [[ -n "$SEND_BIN" ]]; then
  plutil -insert EnvironmentVariables.CXMAIL_SEND_BIN -string "$SEND_BIN" "$PLIST"
  plutil -convert xml1 "$PLIST"
fi

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
"$TS" serve --bg --https="$PORT" "$PORT" >/dev/null

HOST=$("$TS" status --json | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))')
echo "CXMail Remote: https://$HOST:$PORT"
