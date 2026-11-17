#!/bin/bash
# Configure only the existing native push transport; never copies/prints a key.
# Usage: remote/configure-push.sh /absolute/AuthKey_XXX.p8 KEY_ID TEAM_ID
set -euo pipefail
[ "$#" = 3 ] || { echo 'Usage: configure-push.sh /absolute/key.p8 KEY_ID TEAM_ID' >&2; exit 1; }
HERE="$(cd "$(dirname "$0")" && pwd)"
STATE="$HOME/Library/Application Support/com.cxmail.remote"
[ -f "$1" ] || { echo 'APNs key file is missing' >&2; exit 1; }
mkdir -p "$STATE"
chmod 700 "$STATE"
xcrun swiftc -O "$HERE/apns.swift" -o "$STATE/cxmail-apns"
/usr/bin/python3 - "$STATE/apns.json" "$1" "$2" "$3" <<'PY'
import json,os,sys
path,key,kid,team=sys.argv[1:]
if not os.path.isabs(key): sys.exit('Use an absolute key path')
fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
with os.fdopen(fd,'w') as f: json.dump(dict(keyPath=key,keyID=kid,teamID=team),f)
os.chmod(path,0o600)
PY
echo 'Apple Push transport configured. Restart CXMail Remote after updating its signed helper.'
