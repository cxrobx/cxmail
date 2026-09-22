#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

APP_VERSION="$(node -p "require('./package.json').version")"
TAURI_VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"
CARGO_VERSION="$(sed -n 's/^version = \"\([^\"]*\)\"/\1/p' src-tauri/Cargo.toml | head -1)"

if [[ "$APP_VERSION" != "$TAURI_VERSION" || "$APP_VERSION" != "$CARGO_VERSION" ]]; then
  echo "Version mismatch: package=$APP_VERSION tauri=$TAURI_VERSION cargo=$CARGO_VERSION" >&2
  exit 1
fi

# The updater public key baked into tauri.conf.json must be byte-identical to the
# local key's public half. A corrupted pubkey (it has happened — a hand-transcribed
# value lost a character and flipped a byte) still builds, still signs, still
# publishes, and is only caught by the installed client rejecting every update.
# Skipped where the private key is absent (CI), which cannot verify either way.
PUBKEY_FILE="$HOME/.tauri/cxmail.key.pub"
if [[ -f "$PUBKEY_FILE" ]]; then
  if ! node -e "
    const fs = require('fs');
    const conf = require('./src-tauri/tauri.conf.json').plugins.updater.pubkey;
    const real = fs.readFileSync(process.argv[1], 'utf8').trim();
    if (conf !== real) {
      console.error('Updater pubkey mismatch: tauri.conf.json does not match ' + process.argv[1]);
      console.error('Updates signed with this key would be REJECTED by every install.');
      console.error('Fix: copy the file contents verbatim into plugins.updater.pubkey.');
      process.exit(1);
    }
  " "$PUBKEY_FILE"; then
    exit 1
  fi
  echo "Updater pubkey matches $PUBKEY_FILE"
else
  echo "WARNING: $PUBKEY_FILE not found — cannot verify the embedded updater pubkey." >&2
fi

# The publish step (RELEASING.md step 7) fails silently when skipped — 1.0.0
# shipped with a dead update endpoint and nothing anywhere reported it. This
# asserts the CURRENTLY published manifest is being served, which catches a
# skipped publish at the next release instead of never. It deliberately does
# not check the version about to be released; that one isn't published yet.
if [[ "${CXMAIL_SKIP_UPDATE_ENDPOINT_CHECK:-}" == "1" ]]; then
  echo "WARNING: update-endpoint check skipped (CXMAIL_SKIP_UPDATE_ENDPOINT_CHECK=1)." >&2
elif ! ./scripts/verify-published-update.sh; then
  cat >&2 <<'MSG'

The update endpoint is not serving a usable manifest, which means the PREVIOUS
release's publish step was skipped or failed — every installed copy is currently
unable to find updates.

Fix it before releasing again (docs/RELEASING.md § Publishing the update manifest),
or set CXMAIL_SKIP_UPDATE_ENDPOINT_CHECK=1 if nothing has ever been published yet.
MSG
  exit 1
fi

npm audit --audit-level=high
npm test -- --run
npm run build
source "$HOME/.cargo/env"
cargo test --manifest-path src-tauri/Cargo.toml

echo "CXMail $APP_VERSION is ready for the signed release workflow."
