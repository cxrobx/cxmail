#!/bin/bash
# Install a dedicated signed classifier without replacing or quitting CXMail.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
STATE="$HOME/Library/Application Support/com.cxmail.remote"
BINARY="$HERE/../src-tauri/target/release/cxmail-mcp"
REFERENCE=/Applications/CXMail.app/Contents/MacOS/cxmail-mcp
mkdir -p "$STATE"
chmod 700 "$STATE"
# Copy then sign and atomically rename; existing processes keep their inode.
IDENTIFIER=$(codesign -dv "$REFERENCE" 2>&1 | sed -n 's/^Identifier=//p')
REQ=$(codesign -d -r- "$REFERENCE" 2>&1 | sed -n 's/^designated => //p')
[ -n "$IDENTIFIER" ] && [ -n "$REQ" ]
cp "$BINARY" "$STATE/cxmail-notifications.staging"
codesign --force --timestamp --identifier "$IDENTIFIER" \
  --sign 'Developer ID Application: Christopher Robinson (CCYV5HQZCM)' "$STATE/cxmail-notifications.staging"
codesign --verify -R="$REQ" "$STATE/cxmail-notifications.staging"
mv -f "$STATE/cxmail-notifications.staging" "$STATE/cxmail-notifications"
echo 'Signed notification classifier installed; running CXMail left open.'
