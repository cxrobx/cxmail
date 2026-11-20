#!/bin/bash
# Sequential, seeded simulator matrix. No real-mail mutations or sends.
set -euo pipefail
cd "$(dirname "$0")/.."
COMPACT=${1:?Usage: bash scripts/verify.sh compact-simulator-udid wide-ipad-simulator-udid}
WIDE=${2:?Pass an iPad simulator for the exact open-Duo viewport}
/usr/bin/python3 scripts/simulator_policy.py simulator-policy.json "compact=$COMPACT" "wide=$WIDE"
OUT=${CXMAIL_BUILD_DIR:-/private/tmp/cxmail-ios-dd}
RUN=$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
# iPad search can retain hardware-keyboard focus with no software keyboard.
# Exercise the actual on-screen keyboard, then restore the user's preference.
CXMAIL_PREVIOUS_HARDWARE_KEYBOARD=$(defaults read com.apple.iphonesimulator ConnectHardwareKeyboard 2>/dev/null || true)
restore_keyboard_preference() {
  if [ -n "$CXMAIL_PREVIOUS_HARDWARE_KEYBOARD" ]; then
    case "$CXMAIL_PREVIOUS_HARDWARE_KEYBOARD" in
      1|true|YES) defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool true ;;
      0|false|NO) defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool false ;;
    esac
  else
    defaults delete com.apple.iphonesimulator ConnectHardwareKeyboard >/dev/null 2>&1 || true
  fi
}
trap restore_keyboard_preference EXIT
defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool false
xcodegen generate
for PROFILE in compact wide; do
  SIM=$COMPACT; ONLY=(-skip-testing:CXMailUITests/CXMailUITests/testSearchKeyboardInActualWideWindow -skip-testing:CXMailUITests/CXMailUITests/testWideSearchKeepsKeyboardAndTypedQuery -skip-testing:CXMailUITests/CXMailUITests/testExactOpenDuoViewport -skip-testing:CXMailUITests/CXMailUITests/testPortraitInboxDrawer -skip-testing:CXMailUITests/CXMailUITests/testDockChatResizeAndPopOut)
  if [ "$PROFILE" = wide ]; then SIM=$WIDE; ONLY=(-only-testing:CXMailUITests/DraftLinkUITests -only-testing:CXMailUITests/CXMailUITests/testSearchKeyboardInActualWideWindow -only-testing:CXMailUITests/CXMailUITests/testWideSearchKeepsKeyboardAndTypedQuery -only-testing:CXMailUITests/CXMailUITests/testExactOpenDuoViewport -only-testing:CXMailUITests/CXMailUITests/testPortraitInboxDrawer -only-testing:CXMailUITests/CXMailUITests/testDockChatResizeAndPopOut -only-testing:CXMailUITests/CXMailUITests/testCompactSendToolbarScheduleAndCancel -only-testing:CXMailUITests/CXMailUITests/testDraftAttachmentPreviewAndExpiredReview -only-testing:CXMailUITests/CXMailUITests/testEditSavedDraftFromReview -only-testing:CXMailUITests/CXMailUITests/testComposerShowsSignatureBelowWritingArea -only-testing:CXMailUITests/CXMailUITests/testMissingDraftRefreshesListAndOpensCurrentDraft -only-testing:CXMailUITests/CXMailUITests/testTrustedSenderImagesAndAuthenticationDetails -only-testing:CXMailUITests/CXMailUITests/testFailedAuthenticationCannotTrustSenderAutomatically -only-testing:CXMailUITests/CXMailUITests/testFixedWidthEmailFitsClosedReaderAndRotation -only-testing:CXMailUITests/CXMailUITests/testAppearanceSelectionPersistsAcrossRelaunch); fi
  LOG="$OUT/$PROFILE-$RUN.log"
  CX_IOS_SIMULATOR_PROFILE="$PROFILE" xcodebuild -project CXMail.xcodeproj -scheme CXMail -configuration Debug \
    -destination "platform=iOS Simulator,id=$SIM" -derivedDataPath "$OUT" \
    -resultBundlePath "$OUT/$PROFILE-$RUN.xcresult" CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- "${ONLY[@]}" test > "$LOG" 2>&1 || {
      tail -80 "$LOG"; exit 1;
    }
  rg 'Executed .* tests|TEST SUCCEEDED' "$LOG"
done
/usr/bin/python3 ../remote/test_server.py
PREFLIGHT="$HOME/.claude/skills/ios-design/scripts/preflight.sh"
[ ! -x "$PREFLIGHT" ] || "$PREFLIGHT" "$PWD" --scale 8,12,16 --strict
