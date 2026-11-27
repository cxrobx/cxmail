# Draft links from agent sessions to CXMail

After `compose_draft` or `edit_draft`, include the returned **Open in CXMail** HTTPS
link in the session reply. The link names a stable draft UUID, not an IMAP UID.
It survives draft re-saves. On iPhone it opens the native review screen, where
recipients, rendered HTML and the reviewed digest use the existing review pipeline.
On Mac the HTTPS fallback hands off to the registered `cxmail:` handler, which
opens and focuses the linked draft in compose once its UUID has synced to that
Mac. The browser can require confirmation. Choose **Open Link** or **Open CXMail**
when prompted, or select **Open in CXMail** on the page to retry the handoff.
Opening a link never sends or approves a send.

The public metadata Worker in `integrations/draft-links` has no storage bindings,
mail endpoint, credentials or outbound requests. Its only successful routes are:

- `GET/HEAD /.well-known/apple-app-site-association`, associating `/draft/*`.
- `GET/HEAD /draft/<lowercase UUID>`, a generic fallback with a `cxmail://draft/…`
  button and app-open attempt. All syntactically valid IDs receive the same page;
  the public host never checks whether a draft exists.

No email subject, recipient, body, trusted mini address or authentication material
is included in the public page or link. UUIDs are identifiers, not capabilities:
the phone resolves through its own configured Tailscale server, whose normal
login check protects `/api/draft-link?id=…`. Resolution reads the shared `drafts`
table and checks active account, Drafts folder and matching UIDVALIDITY. It never
guesses by subject, invokes an MCP mutation or switches the phone's trusted server.
The existing signed preview helper verifies the actual current draft before review.
A later edit still invalidates the reviewed send digest.

## Deployment and configuration

1. Copy `integrations/draft-links/wrangler.example.toml` to ignored `wrangler.toml`.
   Configure a dedicated public hostname and `LINK_APP_ID` with the **signed**
   application identifier (team prefix plus bundle id). Do not add mail/storage bindings.
2. Deploy: `wrangler deploy --config integrations/draft-links/wrangler.toml` using
   the established credential wrapper for your environment.
3. Before installing the app, run:
   `python3 scripts/verify-draft-links.py https://<public-host> --app-id <team.bundle>`.
   This verifies both origin and Apple's CDN, with identity and gzip encoding.
4. Run `python3 scripts/configure-draft-links.py https://<public-host>`.
   It writes `draft-links.json` in CXMail's local application data and ignored
   `ios/Config/Links.xcconfig`. Hostnames/team configuration stay out of git.
5. Generate/build/install the iOS app following `ios/README.md`. The signing profile
   must include Associated Domains for the explicit app id. `project.yml` remains
   the project source; `Config/Build.xcconfig` includes local server/team settings
   and the optional public link host. The default `links.invalid` is unconfigured.
6. Rebuild the Mac MCP with its existing OAuth configuration and signing requirement
   (`.claude/commands/ship.md`). Restart the shared MCP so sessions use the new results.
   Restart `com.cx.cxmail-remote` between remote chat turns to load the resolver.

Missing/invalid MCP link configuration produces an explicit unavailable message
and the existing desktop link. Never invent a phone URL. A browser fallback can
require another Open tap when the source app or iOS doesn't dispatch automatically.

## Phone recovery and validation

A link received while composing, reviewing or chatting stays pending until that
window is closed and the user chooses Try again. Connection/index failures retain
the pending link and offer retry/dismissal. A newer link, dismissed link or changed
selection cannot be overwritten by an older in-flight resolution. Link lookup's
404 is deliberately **not** `draft_gone` and never evicts cached drafts. The normal
review screen alone handles authoritative missing revisions.

Run `node --test integrations/draft-links/test/links.test.js`,
`/usr/bin/python3 remote/test_server.py`, and `cargo test -p cxmail-mcp --lib`.
The iOS unit suite checks strict trusted URL parsing, current revision, no send,
offline retry, composer preservation and stale-response races. `DraftLinkUITests`
checks cold-start review, actual rendered HTML/Bcc, offline recovery and OS-level
HTTPS dispatch using fiction-only fixtures. Include those tests in both compact
and wide runs of `ios/scripts/verify.sh` before a phone install.
