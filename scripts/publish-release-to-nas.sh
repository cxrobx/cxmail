#!/usr/bin/env bash
set -euo pipefail

tag="${1:-}"
if [[ ! "$tag" =~ ^cxmail-v[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]]; then
  echo "Usage: $0 cxmail-vX.Y.Z" >&2
  exit 1
fi

release_tmp="$(mktemp -d)"
trap 'rm -rf "$release_tmp"' EXIT

gh release download "$tag" --repo cxrobx/cxmail-private \
  --pattern "*.dmg" \
  --pattern "*.tar.gz" \
  --pattern "*.zip" \
  --pattern "*.sig" \
  --pattern "latest.json" \
  --dir "$release_tmp"

dmg_path="$(find "$release_tmp" -maxdepth 1 -name '*.dmg' -type f -print -quit)"
test -n "$dmg_path"
test -f "$release_tmp/latest.json"

spctl --assess --type open --context context:primary-signature -v "$dmg_path"
xcrun stapler validate "$dmg_path"

# Point every platform at the tarball as WE will serve it.
#
# This cannot derive the filename from the URL already in the manifest.
# tauri-action writes GitHub **API asset** URLs — `…/releases/assets/505458001` —
# so the previous `split("/") | last` yielded the numeric asset id and produced
# `…/update/505458001`. The route validates the asset against
# `^[A-Za-z0-9._-]+\.(?:tar\.gz|zip)$`, rejects a bare number, and 404s. The
# manifest itself would still serve 200, so the break is invisible until a
# client tries to download the update it was just offered.
#
# Take the name from the artifact on disk instead. Refuse rather than guess if
# there is not exactly one — a future split-architecture build needs per-platform
# URLs, not whichever tarball `find` happened to return first.
tarball_count="$(find "$release_tmp" -maxdepth 1 -name '*.tar.gz' -type f | wc -l | tr -d ' ')"
if [[ "$tarball_count" -ne 1 ]]; then
  echo "Expected exactly one .tar.gz in the release, found $tarball_count." >&2
  echo "Multiple architectures need per-platform URLs — update this rewrite." >&2
  exit 1
fi
tarball_name="$(basename "$(find "$release_tmp" -maxdepth 1 -name '*.tar.gz' -type f -print -quit)")"

jq --arg url "https://cxventures.io/api/products/cxmail/update/$tarball_name" \
  '(.platforms[] | .url) |= $url' \
  "$release_tmp/latest.json" > "$release_tmp/latest.rewritten.json"
mv "$release_tmp/latest.rewritten.json" "$release_tmp/latest.json"

# The plugin resolves the platform BEFORE checking whether an update is needed and
# has no darwin-universal fallback, so a manifest without this key returns 200 and
# still dies with TargetsNotFound on every Apple Silicon Mac.
jq -e '.platforms."darwin-aarch64".url' "$release_tmp/latest.json" >/dev/null \
  || { echo 'Manifest has no platforms."darwin-aarch64" — refusing to publish.' >&2; exit 1; }

nas_root="${CXMAIL_NAS_RELEASE_ROOT:-/volume2/docker/cxventures/product-downloads}"
ssh nas mkdir -p "$nas_root/cxmail/updates"

# -O forces the legacy SCP protocol. OpenSSH 9+ made `scp` speak SFTP by default,
# and the Synology's sshd does not offer the sftp subsystem — so a plain scp dies
# with "subsystem request failed on channel 0 / scp: Connection closed" while
# `ssh nas` itself works fine, which makes it read like a network or auth fault
# rather than a protocol one.
scp -O "$dmg_path" "nas:$nas_root/cxmail/CXMail.dmg"

find "$release_tmp" -maxdepth 1 -type f \
  \( -name '*.tar.gz' -o -name '*.zip' -o -name '*.sig' -o -name 'latest.json' \) \
  -exec scp -O {} "nas:$nas_root/cxmail/updates/" \;

ssh nas test -r "$nas_root/cxmail/CXMail.dmg" -a -r "$nas_root/cxmail/updates/latest.json"
echo "Published $tag to the CXMail NAS release directory."

# Files landing on the NAS is not the same as the site serving them, and the
# difference is invisible: this script used to exit 0 here while the endpoint
# still 404'd. Fail loudly instead.
"$(dirname "$0")/verify-published-update.sh" "${tag#cxmail-v}"
