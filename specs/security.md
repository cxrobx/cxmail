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

Default: **block all remote images** (prevents tracking pixels).

### Implementation
1. During sanitization, rewrite all `<img src="https://...">` to `<img data-original-src="https://..." src="">` with a placeholder
2. Frontend shows "Images blocked" banner with "Load images" button
3. On "Load images" click:
   - Frontend sends IPC to Rust
   - Rust fetches images via reqwest, returns as data URLs
   - Frontend updates iframe content with real images
4. Per-sender allowlist stored in SQLite (Phase 3)

### What this prevents
- Tracking pixels: sender knows when/where you opened the email
- IP address leakage to sender
- Cookie-based tracking across emails
- Exploit delivery via malicious image URLs

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
