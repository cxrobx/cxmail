#!/usr/bin/env python3
"""Parse `security dump-keychain -a` on stdin and report, per Keychain item,
which CXMail binaries can read it without a prompt.

Driven by scripts/audit-keychain-acls.sh — see that file for the warning about
`dump-keychain -a` raising dialogs and truncating its own output.

Gotcha #31: a prompt-free read needs the reader signed with the team ID, the
item's PARTITION LIST to carry `teamid:CCYV5HQZCM`, AND the item's ACL
TRUSTED-APPLICATION LIST to name that binary. Only the last one gates the
prompt, and no `security` subcommand can write it.
"""
import re
import sys

SERVICE = "cxmail"
BINARIES = ["cxmail-helper", "cxmail-mcp", "cxmail-tracker"]
TEAM_PARTITION = "teamid:CCYV5HQZCM"
YES, NO = "✓", "✗"

# The ACL entry that gates reading is the one holding `decrypt`; it runs until
# the next `entry N:` or the next record.
DECRYPT_ENTRY = re.compile(
    r"authorizations \(\d+\): decrypt.*?(?=\n    entry |\nkeychain:|\Z)", re.S
)
TRUSTED_APP = re.compile(r"^\s+\d+: (\S.*?) \((?:OK|status [^)]*)\)$", re.M)
PARTITION = re.compile(
    r"authorizations \(1\): partition_id\s*\n\s*don't-require-password\s*\n\s*description: ([^\n]*)"
)
ACCOUNT = re.compile(r'"acct"<blob>="([^"]*)"')


def parse(text):
    needle = '"svce"<blob>="%s"' % SERVICE
    rows = []
    for chunk in re.split(r"^keychain: ", text, flags=re.M):
        if needle not in chunk:
            continue
        m = ACCOUNT.search(chunk)
        account = m.group(1) if m else "<unknown>"

        entry = DECRYPT_ENTRY.search(chunk)
        body = entry.group(0) if entry else ""
        names = TRUSTED_APP.findall(body)
        # `applications: <null>` means the entry trusts every application.
        any_app = "applications: <null>" in body

        p = PARTITION.search(chunk)
        partition = p.group(1).strip() if p else "(none)"

        app_ok = any_app or any(
            n.rstrip("/").endswith(".app") or n.endswith("/MacOS/cxmail") for n in names
        )
        per_binary = {b: any_app or any(b in n for n in names) for b in BINARIES}
        rows.append((account, app_ok, per_binary, partition))
    return rows


def main():
    rows = parse(sys.stdin.read())
    if not rows:
        print("No items found — the dump was empty or truncated (see the script's warning).")
        return 1

    print('%d "%s" item(s)\n' % (len(rows), SERVICE))
    labels = [b.replace("cxmail-", "") for b in BINARIES]
    header = "%-52s %4s %s  %s" % (
        "item",
        "app",
        " ".join("%8s" % l for l in labels),
        "partition",
    )
    print(header)
    print("-" * len(header))

    broken = {b: 0 for b in BINARIES}
    missing_partition = 0
    for account, app_ok, per_binary, partition in sorted(rows):
        for b in BINARIES:
            if not per_binary[b]:
                broken[b] += 1
        if TEAM_PARTITION not in partition:
            missing_partition += 1
        print(
            "%-52s %4s %s  %s"
            % (
                account[:52],
                YES if app_ok else NO,
                " ".join("%8s" % (YES if per_binary[b] else NO) for b in BINARIES),
                "ok" if TEAM_PARTITION in partition else "MISSING",
            )
        )

    print()
    for b in BINARIES:
        print("%-16s %s" % (b, "clean" if not broken[b] else "%d item(s) will PROMPT" % broken[b]))
    print(
        "%-16s %s"
        % (
            "partition lists",
            "clean"
            if not missing_partition
            else "%d missing %s (run fix-keychain-partitions.sh)"
            % (missing_partition, TEAM_PARTITION),
        )
    )
    return 0 if not any(broken.values()) and not missing_partition else 2


if __name__ == "__main__":
    sys.exit(main())
