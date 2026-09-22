#!/usr/bin/env bash
# Prove the updater endpoint actually serves a manifest an installed copy can use.
#
# This exists because the publish step is INVISIBLE when skipped: the build
# succeeds, the draft release appears, spctl and stapler pass, and nothing
# anywhere reports that latest.json never landed. The only symptom is an ERROR
# line in the customer's log, one launch at a time. 1.0.0 shipped exactly that
# way and nobody noticed for two weeks.
#
# Usage: scripts/verify-published-update.sh [expected-version]
#   CXMAIL_UPDATE_ENDPOINT overrides the URL (for staging).
set -euo pipefail

ENDPOINT="${CXMAIL_UPDATE_ENDPOINT:-https://cxventures.io/api/products/cxmail/update/latest.json}"
EXPECT_VERSION="${1:-}"

fail() { echo "FAIL: $*" >&2; exit 1; }

command -v jq >/dev/null 2>&1 || fail "jq is required (brew install jq)"

body="$(mktemp)"
trap 'rm -f "$body"' EXIT

# The manifest is served with Cache-Control max-age=60, and a publish that just
# landed can briefly still read as missing. Retry only the statuses that a
# propagation lag actually produces — never a 200, and never a 4xx other than 404.
status=""
for attempt in 1 2 3; do
  status="$(curl -sS -o "$body" -w '%{http_code}' --max-time 30 "$ENDPOINT")" \
    || fail "could not reach $ENDPOINT"
  case "$status" in
    204|404|5??) [[ $attempt -lt 3 ]] && sleep 5 && continue ;;
  esac
  break
done

case "$status" in
  200) ;;
  204) fail "endpoint returns 204 — no manifest is published. Run scripts/publish-release-to-nas.sh." ;;
  404) fail "endpoint returns 404 — nothing published, or the route is misconfigured. See RELEASING.md § Publishing the update manifest." ;;
  *)   fail "endpoint returned HTTP $status" ;;
esac

jq -e . "$body" >/dev/null 2>&1 || fail "manifest is not valid JSON"

version="$(jq -r '.version // empty' "$body")"
[[ -n "$version" ]] || fail "manifest has no .version"
if [[ -n "$EXPECT_VERSION" && "$version" != "$EXPECT_VERSION" ]]; then
  fail "manifest serves version $version, expected $EXPECT_VERSION"
fi

# tauri-plugin-updater 2.10.0 resolves the platform BEFORE it checks whether an
# update is even needed (get_urls() at updater.rs:536, the should_update branch
# at :538), and its only candidates are {os}-{arch}-{installer} and {os}-{arch}.
# There is NO darwin-universal fallback — a manifest keyed that way returns 200
# right here and still dies with TargetsNotFound on the client.
url="$(jq -r '.platforms."darwin-aarch64".url // empty' "$body")"
[[ -n "$url" ]] \
  || fail 'manifest has no platforms."darwin-aarch64" — every Apple Silicon client gets TargetsNotFound'
[[ -n "$(jq -r '.platforms."darwin-aarch64".signature // empty' "$body")" ]] \
  || fail 'platforms."darwin-aarch64".signature is empty — the client rejects an unsigned update'

jq -e '.platforms."darwin-x86_64".url' "$body" >/dev/null 2>&1 \
  || echo "WARNING: no platforms.\"darwin-x86_64\" — Intel Macs cannot update from this manifest." >&2

# Prefer HEAD, but fall back to a one-byte range GET: Next.js App Router
# synthesizes HEAD from the GET handler, and if that ever changes a 405 here
# would be a false failure. A server that ignores Range degrades to a full GET,
# which is correct but slow — hence the timeout.
asset_status="$(curl -sSL -o /dev/null -w '%{http_code}' --max-time 60 --head "$url" 2>/dev/null || echo 000)"
if [[ ! "$asset_status" =~ ^2 ]]; then
  asset_status="$(curl -sSL -o /dev/null -w '%{http_code}' --max-time 120 -r 0-0 "$url")" \
    || fail "could not reach the update asset at $url"
fi
[[ "$asset_status" == 200 || "$asset_status" == 206 ]] \
  || fail "update asset $url returned HTTP $asset_status"

echo "Update endpoint OK — serving $version, darwin-aarch64 asset reachable (HTTP $asset_status)."
