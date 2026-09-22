# Bundled ISPDB snapshot

Mail-provider autoconfiguration data from the Thunderbird `autoconfig` project,
vendored here rather than fetched from `autoconfig.thunderbird.net` at runtime.

- **Source:** https://github.com/thunderbird/autoconfig
- **Snapshot commit:** `52e19b4904720d88aaa3214e461f668cda67a389`
- **License:** Mozilla Public License 2.0 (see `LICENSE` in this directory)

## Why vendored and not fetched

MPL-2.0 explicitly permits redistributing this data inside a larger work, including
a proprietary one — obligations attach only to *modified* MPL files, and these are
unmodified. Calling Mozilla's public endpoint from a commercial product instead
relies on a service whose terms are unstated and which can rate-limit, block, or
retire the endpoint at any time, silently degrading account setup for paying users.

Vendoring is also faster (no network round trip), works offline, and keeps every
autodiscovery tier talking only to the user's own domain.

## Do not edit these files

They are a verbatim snapshot. Refresh with `scripts/sync-ispdb.sh`, which
re-clones upstream and updates the commit recorded above. Editing them in place
would make this a modified MPL work and trigger source-availability obligations.
