#!/usr/bin/env bash
# Audit which CXMail binaries can read each `cxmail` Keychain item WITHOUT a
# prompt. This is the diagnostic for gotcha #31.
#
# A prompt-free read needs THREE things, not two:
#   1. the reading binary is signed with TeamIdentifier=CCYV5HQZCM
#   2. the item's PARTITION LIST contains teamid:CCYV5HQZCM
#   3. the item's ACL TRUSTED-APPLICATION LIST names that binary
#
# `fix-keychain-partitions.sh` writes only (2). The prompt is gated by (3), and
# no `security` subcommand can write it — items are born trusting their creating
# binary alone. This script shows (2) and (3) side by side so "the sweep says
# 43/43" can never again be mistaken for "the prompts are fixed".
#
# ⚠️  `security dump-keychain -a` can return SILENTLY TRUNCATED output. Observed
#     2026-07-30: two runs minutes apart reported 43 items and then 19, and
#     invoking it repeatedly in a loop hung outright. The truncated run looked
#     entirely plausible — same format, sane-looking rows, just fewer of them.
#     **Check the reported item count against what you expect before trusting a
#     single line of the report.** Ctrl-C is safe; this script only reads.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Dumping Keychain ACLs — answer any dialogs that appear (Allow is enough)…" >&2
security dump-keychain -a 2>/dev/null | python3 "${HERE}/audit_keychain_acls.py"
