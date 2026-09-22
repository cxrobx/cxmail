#!/bin/bash
# Refresh the vendored ISPDB snapshot from upstream.
#
# The data is bundled rather than fetched from autoconfig.thunderbird.net at
# runtime — see src-tauri/resources/ispdb/README.md for why. That trade buys
# offline support and removes a third-party runtime dependency, at the cost of
# needing this script periodically. Run it before a release.
#
# The files are redistributed UNMODIFIED under MPL-2.0. Do not hand-edit them:
# modifying an MPL file makes it a modified work and triggers source-availability
# obligations that vendoring an untouched snapshot does not.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$REPO/src-tauri/resources/ispdb"
UPSTREAM="https://github.com/thunderbird/autoconfig.git"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Cloning $UPSTREAM"
git clone --depth 1 --quiet "$UPSTREAM" "$TMP/autoconfig"
COMMIT="$(cd "$TMP/autoconfig" && git rev-parse HEAD)"

if [ ! -d "$TMP/autoconfig/ispdb" ]; then
  echo "!!  Upstream layout changed: no ispdb/ directory. Aborting." >&2
  exit 1
fi

COUNT="$(find "$TMP/autoconfig/ispdb" -name '*.xml' | wc -l | tr -d ' ')"
if [ "$COUNT" -lt 50 ]; then
  # A near-empty clone would silently gut autodiscovery rather than fail loudly.
  echo "!!  Only $COUNT config files upstream — refusing to replace the snapshot." >&2
  exit 1
fi

echo "==> Replacing snapshot ($COUNT files, commit ${COMMIT:0:12})"
rm -f "$DEST"/*.xml
cp "$TMP/autoconfig/ispdb"/*.xml "$DEST/"
cp "$TMP/autoconfig/LICENSE" "$DEST/LICENSE"

# Keep the recorded provenance honest — it is the only record of what we shipped.
sed -i '' "s|^- \*\*Snapshot commit:\*\* .*|- **Snapshot commit:** \`$COMMIT\`|" "$DEST/README.md"

echo "==> Done. Review with: git diff --stat src-tauri/resources/ispdb"
echo "    Then re-run: cargo test -p cxmail autoconfig"
