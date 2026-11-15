#!/bin/bash
# Sequential, seeded simulator matrix. No real-mail mutations or sends.
set -euo pipefail
cd "$(dirname "$0")/.."
COMPACT=${1:?Usage: bash scripts/verify.sh compact-simulator-udid wide-ipad-simulator-udid}
WIDE=${2:?Pass an iPad simulator for the exact open-Duo viewport}
OUT=${CXMAIL_BUILD_DIR:-/private/tmp/cxmail-ios-dd}
RUN=$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
xcodegen generate
for PROFILE in compact wide; do
  SIM=$COMPACT; ONLY=(-skip-testing:CXMailUITests/CXMailUITests/testExactOpenDuoViewport -skip-testing:CXMailUITests/CXMailUITests/testPortraitInboxDrawer -skip-testing:CXMailUITests/CXMailUITests/testDockChatResizeAndPopOut)
  if [ "$PROFILE" = wide ]; then SIM=$WIDE; ONLY=(-only-testing:CXMailUITests/CXMailUITests/testExactOpenDuoViewport -only-testing:CXMailUITests/CXMailUITests/testPortraitInboxDrawer -only-testing:CXMailUITests/CXMailUITests/testDockChatResizeAndPopOut); fi
  LOG="$OUT/$PROFILE-$RUN.log"
  xcodebuild -project CXMail.xcodeproj -scheme CXMail -configuration Debug \
    -destination "platform=iOS Simulator,id=$SIM" -derivedDataPath "$OUT" \
    -resultBundlePath "$OUT/$PROFILE-$RUN.xcresult" CODE_SIGNING_ALLOWED=NO "${ONLY[@]}" test > "$LOG" 2>&1 || {
      tail -80 "$LOG"; exit 1;
    }
  rg 'Executed .* tests|TEST SUCCEEDED' "$LOG"
done
/usr/bin/python3 ../remote/test_server.py
PREFLIGHT="$HOME/.claude/skills/ios-design/scripts/preflight.sh"
[ ! -x "$PREFLIGHT" ] || "$PREFLIGHT" "$PWD" --scale 8,12,16 --strict
