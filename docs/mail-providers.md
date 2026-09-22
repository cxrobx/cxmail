# Mail providers, generic IMAP, and autodiscovery

How CXMail connects to a mail server, how it works out *which* server, and the
decisions behind both. Covers the `imap` provider (schema v50) and the
autoconfig cascade.

---

## 1. Providers

| Provider | Auth | Servers come from | Credential key |
|---|---|---|---|
| `gmail` | OAuth2 XOAUTH2 | hardcoded | `gmail:{email}:access/refresh/expires` |
| `outlook` | OAuth2 XOAUTH2 | hardcoded | `outlook:{email}:access/refresh/expires` |
| `icloud` | password (`LOGIN`) | hardcoded | `icloud:{email}:password` |
| **`imap`** | password (`LOGIN`) | **the `accounts` row** | `imap:{email}:password` |

`imap` is the only provider that reads `imap_host` / `imap_port` /
`imap_security` / `imap_username` off the database. **The other three keep
their literals on purpose** — so a corrupted or tampered row cannot redirect an
existing Gmail account at someone else's server.

`imap::connect_for_account(&Account)` is the single entry point. There is no
`connect_for_provider` — it was deleted rather than kept as a wrapper, so a
missed call site is a compile error rather than an account that silently
cannot connect.

### Why the generic provider exists

Two reasons, and the second is the strategic one:

1. The storefront already advertised "anything that speaks standard IMAP and
   SMTP". It didn't work.
2. **Google caps the `cxmail` Cloud project at 100 lifetime OAuth consents.**
   Lifting that needs restricted-scope verification plus a recurring CASA
   assessment. An **app-password connection is not OAuth** — it consumes no
   cap slots, shows no unverified-app warning, and needs no Google approval.

Gmail therefore ships as an app-password *preset* as well as an OAuth provider.
OAuth remains the default in the picker.

> Note the limit of that argument: app passwords are themselves Google's to
> revoke, and Workspace admins can disable them. The durable version of "don't
> depend on Google" is making non-Google providers frictionless — which is what
> autodiscovery below is for.

---

## 2. Transport security

`providers::TlsMode` has exactly two variants: `Implicit` (TLS from the first
byte — 993/465) and `StartTls` (upgrade in band — 143/587).

**There is deliberately no `Plain` variant, and adding one is not a small
change.** The moment the enum can describe an unencrypted connection, something
surfaces it as a checkbox and the TLS invariant becomes optional. A server that
cannot do TLS is one CXMail declines to talk to.

Port handling (`providers::security_for_port`):

| Port | Result |
|---|---|
| 993, 465 | implicit |
| 143, 587 | STARTTLS |
| 25 | **refused** — unencrypted server-to-server relay |
| 110, 995 | **refused** — POP3; CXMail is an IMAP client |
| anything else | no answer — the caller must have an explicit choice, never a guess |

**IMAP STARTTLS is not implemented** (gotcha #41). Measured cost against the
bundled provider database: **2 providers out of 120**. SMTP STARTTLS works
normally.

Two TLS stacks, which is not obvious and matters: **IMAP is native-tls, SMTP is
rustls + webpki-roots.** They do not share a trust store, so an IMAP success is
not evidence that sending works — which is why `add_imap_account` validates
both before saving.

---

## 3. Autodiscovery

`email::autoconfig::discover(email)` answers "where does this address get its
mail?" so a user with a custom domain doesn't need to know their own hostname.

Tiers, first hit wins:

| # | Tier | `source` | Network? |
|---|---|---|---|
| 0 | Curated presets (`providers::PRESETS`, 9 providers) | `preset` | no |
| 1 | Bundled ISPDB — 163 files, **969 domains** | `ispdb` | no |
| 2 | `autoconfig.<domain>` + `<domain>/.well-known/…` | `autoconfig` | user's own domain |
| 3a | DNS SRV, RFC 6186 | `srv` | DNS |
| 3b | MX → retry tiers 1–2 against the operator | `mx` | DNS |

**Every tier is local or talks only to the user's own domain.** Nothing
contacts a third party at runtime.

Tier 0 exists on top of the much larger tier 1 because the presets carry
app-password `hint` text the ISPDB has no field for — and `fastmail.com` isn't
in the ISPDB at all.

### The ISPDB is vendored, not fetched

`src-tauri/resources/ispdb/` holds an **unmodified MPL-2.0 snapshot** of the
[Thunderbird autoconfig database](https://github.com/thunderbird/autoconfig),
embedded at compile time by `build.rs`.

It is deliberately **not** fetched from `autoconfig.thunderbird.net`:

| | Live fetch | Vendored |
|---|---|---|
| Legal basis | unstated service terms | **MPL-2.0, explicitly redistributable** |
| Third party can break it | yes, silently | no |
| Speed | network round trip | local (1.3 ms to build the index, then 167 ns) |
| Offline | no | **yes** |

MPL-2.0 permits redistributing unmodified files inside a larger proprietary
work; obligations attach only to *modified* MPL files. **Do not hand-edit those
files** — editing one makes it a modified work and triggers source-availability
obligations that an untouched snapshot does not. Refresh with
`scripts/sync-ispdb.sh`, which re-clones upstream, refuses a suspiciously small
result, and updates the recorded commit.

One config file routinely covers several domains via `<domain>` elements, so
the index is built from **both** the filename and those elements — 163 files
resolve 969 domains. Filename-only lookup would discard ~6× the coverage.

### Parsing rules (Mozilla `clientConfig`)

Each is pinned by a test in `email/autoconfig.rs`:

- `socketType` `plain` → **refused**. `TlsMode` cannot represent it.
- The schema allows **multiple** server entries, so **implicit TLS is preferred
  over STARTTLS in either order** — we cannot do IMAP STARTTLS.
- `password-encrypted` (CRAM-MD5) and `OAuth2` → **refused**. Accepting them
  produces settings that look right and fail at login.
- `%EMAILADDRESS%` → `None` (authenticate as the address).
  `%EMAILLOCALPART%` → the local part, which is what `accounts.imap_username`
  exists for.
- A literal username that simply **is** the address → `None`. Tier 2 passes
  `?emailaddress=`, and some providers (verified against Migadu) substitute the
  template server-side; storing the result literally would freeze a copy of the
  address that stops tracking the account if the email is corrected.
- `type="pop3"` entries → skipped.

### Security model

**Discovery never connects and never sees the password.** It returns settings
the setup form *displays*.

- Tier 2 is **HTTPS only**. Thunderbird historically also tried plain HTTP,
  which lets anyone on the path return a config pointing at their own server
  and harvest the credential. There is no http fallback.
- DNS is spoofable without DNSSEC, so anything found by tiers 2–3 **reveals the
  server-settings panel** — the user sees the hostname before a password is
  sent to it. Only curated presets keep the collapsed two-field flow.
- Security is submitted only when the port on screen still matches what was
  discovered; if the user retyped the port, the backend re-derives it rather
  than trusting a stale value.

### Try it without credentials

```bash
cargo run --example discover -- chris@cxventures.io someone@posteo.de
```

Performs no authentication and sends no credential.

---

## 4. Folder classification

For `imap` accounts only: **RFC 6154 SPECIAL-USE first, name heuristics as a
mandatory fallback.**

The fallback is not a nicety. `async-imap`'s `list()` emits a bare
`LIST "" *` and offers no way to request SPECIAL-USE (`parse_names` is
`pub(crate)`), so servers may *volunteer* attributes but we can never ask.

- Attributes resolve by **fixed precedence**, not the order the server listed
  them, so a mailbox carrying two classifies identically everywhere.
- Heuristics match the **last hierarchy segment, whole**. A substring match
  makes `Sent to Legal` the account's Sent folder — and `db::nudges` computes
  "my last message in this thread" out of Sent, so the follow-up lane would go
  quiet with no error.
- `\Noselect` rows are skipped: Dovecot/Courier emit hierarchy placeholders
  that would otherwise fail every `SELECT`.
- **Not applied to gmail/icloud/outlook.** Gmail marks `[Gmail]/Starred`
  `\Flagged` and `[Gmail]/Important` with a vendor extension, and those map to
  `starred` / `important` — not RFC 6154 concepts. Letting the attribute win
  would silently rewrite `folder_type` for accounts that already work, and six
  call sites read that column.

Special folders resolve through `db::folders::folder_for_account` — synced
folder list first, `fallback_folder_name` second (for the window before the
first folder sync). The lookup is ordered `special_use DESC, LENGTH(name),
name`; see gotcha #41 for why an unordered `LIMIT 1` was a real hazard.

---

## 5. Adding a provider preset

1. Add a `MailPreset` to `providers::PRESETS` (`src-tauri/crates/cxmail-email/src/email/providers.rs`).
2. Mirror it in `src/lib/mailPresets.ts`. **These two must change together** —
   `providers::tests::ts_mirror_matches_the_rust_table` parses the TS file and
   fails on drift.
3. `cargo test -p cxmail providers` — the suite checks id/domain uniqueness and
   that each preset's port agrees with its declared security.

Only add a preset when the provider needs `hint` text or isn't in the ISPDB.
Otherwise tier 1 already covers it.

---

## Related

- Gotcha **#41** — STARTTLS deferral, the SPECIAL-USE limitation, the
  `LIMIT 1` determinism hazard, and why `tauri dev` needs a signed binary.
- Gotcha **#31** — Keychain ACLs; `imap:{email}:password` is subject to them.
- `.claude/rules/architecture.md` — invariants, including the two-TLS-stack note.
