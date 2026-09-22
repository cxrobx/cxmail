#!/usr/bin/env bash
# Set the PARTITION LIST on every `cxmail` Keychain item.
#
# ⚠️  THIS DOES NOT STOP AUTHORIZATION PROMPTS ON ITS OWN, and it used to claim
#     it did. Corrected 2026-07-30 — see gotcha #31. A prompt-free read needs
#     THREE things:
#
#       1. the reading binary is signed with TeamIdentifier=CCYV5HQZCM
#       2. the item's PARTITION LIST contains teamid:CCYV5HQZCM   <- this script
#       3. the item's ACL TRUSTED-APPLICATION LIST names that binary
#
#     The prompt is gated by (3) — ACL `entry 1`, `authorizations: decrypt …`.
#     No `security` subcommand can write it: `SecKeychainAddGenericPassword`
#     sets it to the creating binary alone, and changing it afterwards needs the
#     `change_acl` authorization, which no binary holds. An audit on 2026-07-30
#     found all 43 items already correct on (2) — including ones created after
#     the previous sweep, because macOS assigns `teamid:` automatically for a
#     signed creator — while 19 were missing `cxmail-helper` on (3). The sweep
#     had been fixing a non-problem.
#
#     (3) is handled inside the app now: `keychain::repair_keychain_acls()` runs
#     once at launch and recreates each item with every CXMail binary trusted,
#     and `keychain::macos_acl` mints new items that way from the start.
#
# SO WHEN DO YOU RUN THIS? Only as a fallback, if
# `bash scripts/audit-keychain-acls.sh` reports a MISSING partition — e.g. if a
# future macOS stops auto-assigning `teamid:` to app-created items. Check first;
# don't run it reflexively.
#
# RUN THIS YOURSELF — it needs your login password, which must never be pasted
# into a Claude session. Run it in your own terminal:
#     bash scripts/fix-keychain-partitions.sh
#
# NOTE ON -k: the password is passed to `security` as an argument, so it is
# briefly visible to `ps` on this machine. The alternative is one GUI dialog per
# item (40+). Single-user Mac, so the tradeoff is deliberate.
set -uo pipefail

TEAM_ID="CCYV5HQZCM"
SERVICE="cxmail"
PARTITIONS="teamid:${TEAM_ID},apple:"

echo "Collecting '${SERVICE}' Keychain items…"
# NOTE: macOS ships bash 3.2, which has no `mapfile` — read the list the
# portable way. `bash -n` will not catch a mapfile call here, it just runs empty.
ACCOUNTS=()
while IFS= read -r line; do
  [ -n "${line}" ] && ACCOUNTS+=("${line}")
done < <(
  security dump-keychain 2>/dev/null |
    awk -v svc="\"svce\"<blob>=\"${SERVICE}\"" '
      /"acct"<blob>=/ { acct = $0 }
      $0 ~ svc       { print acct }
    ' |
    sed 's/.*"acct"<blob>="\(.*\)"$/\1/' |
    sort -u
)

if [ "${#ACCOUNTS[@]}" -eq 0 ]; then
  echo "No items found for service '${SERVICE}'. Nothing to do."
  exit 0
fi

echo "Found ${#ACCOUNTS[@]} items. They will be granted: ${PARTITIONS}"
printf '  %s\n' "${ACCOUNTS[@]}"
echo

# Read once, never echoed, never leaves this shell.
read -rsp "login keychain password: " KEYCHAIN_PW
echo
if [ -z "${KEYCHAIN_PW}" ]; then
  echo "No password entered — aborting." >&2
  exit 1
fi

ok=0
fail=0
failed_items=()
for acct in "${ACCOUNTS[@]}"; do
  if security set-generic-password-partition-list \
      -S "${PARTITIONS}" -s "${SERVICE}" -a "${acct}" \
      -k "${KEYCHAIN_PW}" >/dev/null 2>&1; then
    ok=$((ok + 1))
  else
    fail=$((fail + 1))
    failed_items+=("${acct}")
  fi
done
unset KEYCHAIN_PW

echo "updated: ${ok}   failed: ${fail}"
if [ "${fail}" -gt 0 ]; then
  printf 'failed: %s\n' "${failed_items[@]}" >&2
  echo "A wrong password fails every item; anything else is usually a stale duplicate." >&2
  exit 1
fi

cat <<'EOF'

Partition lists updated. Quit and relaunch CXMail.

This does NOT by itself stop authorization prompts — the trusted-application
list is what gates them, and this script cannot touch it. Confirm with:

    bash scripts/audit-keychain-acls.sh

Every binary column must read clean. If one does not, the in-app repair has not
run: quit CXMail, delete the marker, and relaunch it.

    rm ~/Library/Application\ Support/com.cxmail.app/keychain-acl-repair-v1.done
EOF
