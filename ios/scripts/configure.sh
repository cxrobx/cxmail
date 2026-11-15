#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p Config
TS=$(command -v tailscale || echo /Applications/Tailscale.app/Contents/MacOS/Tailscale)
HOST=$("$TS" status --json | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))')
# xcconfig treats // as a comment, so split the slash pair.
printf 'CXMAIL_SERVER_URL = https:/$()/%s:18706\n' "$HOST" > Config/Local.xcconfig
[ -z "${CXMAIL_DEVELOPMENT_TEAM:-}" ] || printf 'CXMAIL_DEVELOPMENT_TEAM = %s\n' "$CXMAIL_DEVELOPMENT_TEAM" >> Config/Local.xcconfig
xcodegen generate
