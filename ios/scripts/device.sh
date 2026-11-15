#!/bin/bash
# Direct device install; provisioning keys are external and are never copied.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${CXMAIL_BUILD_DIR:-/private/tmp/cxmail-ios-dd}
mkdir -p "$OUT"
[ -f Config/Local.xcconfig ] || { echo 'Run bash scripts/configure.sh first.' >&2; exit 1; }
xcodegen generate
DEVICES=$(mktemp)
trap 'rm -f "$DEVICES"' EXIT
xcrun devicectl list devices --json-output "$DEVICES" >/dev/null
read -r UDID DEVICE < <(/usr/bin/python3 - "$DEVICES" <<'PY'
import json,sys
devices=json.load(open(sys.argv[1]))['result']['devices']
duos=[d for d in devices if d.get('hardwareProperties',{}).get('productType')=='iPhone19,4' and d.get('hardwareProperties',{}).get('reality')=='physical']
if len(duos)!=1: sys.exit('Connect exactly one physical Duo, or install the built app with devicectl yourself.')
d=duos[0]; print(d['hardwareProperties']['udid'], d['identifier'])
PY
)
AUTH=()
if [ -n "${CXMAIL_ASC_KEY_PATH:-}" ]; then
  : "${CXMAIL_ASC_KEY_ID:?Set CXMAIL_ASC_KEY_ID}"
  : "${CXMAIL_ASC_ISSUER_ID:?Set CXMAIL_ASC_ISSUER_ID}"
  AUTH=(-authenticationKeyPath "$CXMAIL_ASC_KEY_PATH" -authenticationKeyID "$CXMAIL_ASC_KEY_ID" -authenticationKeyIssuerID "$CXMAIL_ASC_ISSUER_ID")
fi
xcodebuild -project CXMail.xcodeproj -scheme CXMail -configuration Release \
  -destination "platform=iOS,id=$UDID" -derivedDataPath "$OUT" \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration ${AUTH[@]+"${AUTH[@]}"} build > "$OUT/device-build.log" 2>&1 || {
  rg 'error:|BUILD FAILED' "$OUT/device-build.log"; exit 1;
}
rg 'BUILD SUCCEEDED' "$OUT/device-build.log"
[ "${1:-}" = build ] && exit 0
APP="$OUT/Build/Products/Release-iphoneos/CXMail.app"
xcrun devicectl device install app --device "$DEVICE" "$APP" || xcrun devicectl device install app --device "$DEVICE" "$APP"
xcrun devicectl device process launch --device "$DEVICE" app.cxmail.remote || echo 'Installed. If the phone is locked, open CXMail by hand.'
