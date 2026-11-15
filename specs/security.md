# Security Model

## Credential Storage

All credentials stored in macOS Keychain via `keyring` crate with `apple-native` feature.

| Asset | Keychain service | Keychain account |
|-------|-----------------|------------------|
| OAuth2 access token | `cxmail` | `{provider}:{email}:access` |
| OAuth2 refresh token | `cxmail` | `{provider}:{email}:refresh` |
| Token expiry | `cxmail` | `{provider}:{email}:expires` |
| App-specific password (iCloud) | `cxmail` | `icloud:{email}:password` |

**INVARIANT**: No credentials are EVER stored in SQLite, log files, frontend-accessible locations, or environment variables.

## HTML Email Sanitization

Uses `ammonia` crate. Applied before storing in `message_bodies.sanitized_html`.

### Allowed Tags
```
p, div, span, a, img, br, hr,
b, i, strong, em, u, s, strike,
h1, h2, h3, h4, h5, h6,
ul, ol, li,
table, thead, tbody, tfoot, tr, th, td,
blockquote, pre, code,
sup, sub, small, big,
center, font
```

### Stripped Tags (removed entirely, including content for script/style)
```
script, style, iframe, object, embed, form, input, textarea,
select, button, applet, base, link, meta
```

### Allowed Attributes
```
a:       href, title (href must be http: or https: only)
img:     src, alt, width, height (src rewritten — see remote images below)
td/th:   colspan, rowspan, align, valign, width, height
table:   cellpadding, cellspacing, border, width
font:    color, size, face
*:       style (with CSS property allowlist), class, id, dir, lang
```

### CSS Property Allowlist (for style attribute)
```
color, background-color, background,
font-family, font-size, font-weight, font-style,
text-align, text-decoration, text-transform,
line-height, letter-spacing, word-spacing,
margin, margin-top, margin-right, margin-bottom, margin-left,
padding, padding-top, padding-right, padding-bottom, padding-left,
border, border-top, border-right, border-bottom, border-left,
border-color, border-width, border-style, border-collapse,
width, height, max-width, max-height, min-width, min-height,
display, vertical-align, float, clear,
list-style-type, list-style-position
```

### Blocked CSS
- `expression()` — IE CSS expressions (code execution)
- `url()` — except for `background-image` with `https:` only
- `-moz-binding`, `behavior` — legacy browser code execution
- `position: fixed/absolute` — prevents overlay attacks

### Link Rewriting
All `<a>` tags rewritten to:
```html
<a href="..." target="_blank" rel="noopener noreferrer">
```
Only `http:` and `https:` URLs allowed. `javascript:`, `data:`, `vbscript:` stripped.

## Remote Image Policy

Default: **block remote images**. The shared image-loading policy is implemented in
`cxmail-email::email::image_policy`, used by Tauri and the signed remote helper.

### Automatic loading

All three conditions are required for the current message:

1. The exact sender address is in `trusted_image_senders`, the mini's existing
   shared allowlist. Desktop and iOS write the same table; no domain wildcards.
2. From-domain authentication passes. Currently supported for Gmail accounts and
   generic IMAP accounts explicitly configured for `imap.gmail.com`. The policy
   checks the receiver's topmost `Authentication-Results: mx.google.com`, its
   receiving trace, an unambiguous single From mailbox matching the cached sender,
   and exactly one passing DMARC result for that same From domain. Missing,
   malformed, duplicate/ambiguous or unsupported evidence stays unknown.
3. The message is not in Junk/Trash and has no Medium/High local or previously
   stored risk warning. A LOW risk report alone never grants permission. Opening
   mail does not start a new model/inference request.

Only the configured receiving provider's results are trusted (RFC 8601). A
Google-looking header copied to an unrelated provider is not evidence. As with
other mail clients, this relies on the receiving provider's header handling;
mailbox imports/IMAP APPEND are not independently attested by these headers.

The discreet sender indicator says **Domain authenticated**, not Google verified,
brand verified or safe. It does not certify the person, links, attachments or image
bytes. Gmail's BIMI/VMC blue check is not reproduced. Non-Google providers retain
manual image loading until a reviewed provider-specific verifier is added.

### Rendering and explicit choices

The parser rewrites remote `<img>` URLs to `data-original-src` placeholders. The
desktop iframe also starts with a restrictive `img-src` CSP, covering CSS
backgrounds and any direct image URL the placeholder rewrite does not cover.
When images are allowed, a newly constructed frame permits HTTPS images and the
app-owned frame script restores placeholders. Email scripts remain stripped and
the frame never receives `allow-same-origin`.

The iOS/PWA service keeps its SQLite reads read-only. Its signed helper supports
`remote image-policy <account> <folder> <uid>` and explicit `trust-images` /
`untrust-images` actions, which are not MCP tools. POST trust changes require the
existing tailnet identity and JSON/CSRF guard. The `/api/body?images=auto` path
rechecks policy before enabling HTTPS images; `images=1` is explicit Show once.
The native client also requires the final `X-CXMail-Images-Allowed` response header
for automatic permission, because loading HTML through URLSession does not carry
HTTP CSP headers into WKWebView. In-flight decisions are superseded on refresh or
trust changes, so an older response cannot re-enable images after revocation.
Server and native WKWebView CSPs continue to deny email scripts.

Show once permits images for one message without changing sender trust. Trust
changes trigger a fresh policy check, never an immediate address-only grant.
Desktop rechecks on focus; phone refresh/reopen reads current shared trust.
Offline phone bodies never restore an automatic network-loading permission.

Blocking avoids sender image requests that disclose opens and IP addresses, and
reduces exposure to malicious remote images. It does not block embedded images,
scan image files, or replace OS/WebKit security updates. Once allowed, images are
fetched directly: there is no image proxy or claim of tracking prevention.

## Frontend Rendering Sandbox

The reading pane renders sanitized email HTML in a `srcdoc` iframe
(`src/components/mail/EmailFrame.tsx`):

```html
<iframe
  sandbox="allow-scripts allow-top-navigation-by-user-activation"
  srcDoc={frameDocument}
/>
```

### What this sandbox means
- **No `allow-same-origin`**: the frame has an opaque origin, so nothing in it can reach the
  app's DOM, storage, cookies or Tauri IPC.
- **Scripts run, but only CXMail's own.** ammonia strips every `<script>` and event handler
  from email HTML before storage, and the frame inherits the app CSP (`script-src 'self'`),
  which blocks inline script. The one script that runs is `public/email-frame.js`, loaded by
  URL (height reporting, selection feedback, image trust). Gotcha #29: it must stay an
  external file, never inline or hash-allowed. Test: `EmailFrameCsp.test.ts`.
- NO form submission, NO popups, NO plugins.
- Top-level navigation only on a real user click.

Hover previews in the message list use `sandbox=""` (no flags at all).

### Link Interception
- The frame document carries `<base target="_top">`, so a clicked link navigates the top frame.
- Tauri's `on_navigation` handler catches that navigation and opens the URL in the system
  browser; the app webview never navigates to an external URL.

### Agents (MCP)
What reaches an AI agent through the MCP, and what an agent may do with it, is a separate
boundary: fenced untrusted output, server-side file limits, client approval hooks and the
per-message risk check. See `specs/mcp.md` § Agent boundary.

## TLS / Network Security

- All IMAP connections use port 993 (IMAPS — TLS from connection start)
- All SMTP connections use port 587 with STARTTLS or port 465 (SMTPS)
- All HTTP requests (OAuth2 token exchange) use HTTPS only
- TLS via `async-native-tls` (uses macOS SecureTransport) for IMAP
- TLS via `rustls` (reqwest default) for HTTP
- No plaintext auth fallback — if TLS fails, connection fails
- Certificate validation enabled (OS trust store)

## IPC Security

- All Tauri IPC commands validate inputs on the Rust side
- Account IDs are UUIDs, validated with `uuid::Uuid::parse_str()` before any DB query
- Folder names checked against cached folder list (no arbitrary IMAP commands from frontend)
- Tauri 2 capabilities system restricts which IPC commands the webview can call
- Error types never leak internal details to frontend (no stack traces, no file paths)

## Filesystem Security

- SQLite DB in `~/Library/Application Support/com.cxmail.app/` (standard macOS app data location)
- No arbitrary filesystem access from frontend
- Attachment downloads use Tauri dialog plugin (user picks destination via save panel)
- No temp files with sensitive content — everything goes through Keychain or SQLite

## Attachment Text Extraction

PDF and DOCX text (search indexing, `parse_message`, the MCP's `read_attachment_text`) is
extracted by `cxmail-extract`, a bundled helper run under `sandbox-exec`: no network, no file
writes, no forking, no reads under the user's home folder, 5 s of CPU, a 512 MB allocator cap
(macOS does not enforce `RLIMIT_AS`) and a 10 s wall-clock kill. The parsers are memory-safe
Rust (`pdf-extract` has no `unsafe`, `lopdf` forbids it, deflate is `miniz_oxide`), so the
memory-corruption class behind native-renderer PDF CVEs does not apply; the helper contains
what Rust does not prevent. Measured: a 2 MB PDF holding a 2 GB deflate bomb aborts at the
memory cap in 0.09 s. Release builds never fall back to in-process parsing; a missing helper
means no text. The native macOS parsers (Spotlight's importer, Quick Look) still see any PDF
that is *saved to disk*, which is why downloads always ask and `read_attachment_text` is the
preferred route. Tests: `document_text::sandbox_tests` (the 10 s wall-clock test is `--ignored`).
