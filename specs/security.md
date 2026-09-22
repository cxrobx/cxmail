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

Email HTML is rendered in an `<iframe>` with strict sandboxing:

```html
<iframe
  sandbox="allow-same-origin"
  srcdoc={sanitizedHtml}
  style="border: none; width: 100%; height: 100%;"
/>
```

### What `sandbox="allow-same-origin"` means
- NO JavaScript execution
- NO form submission
- NO popups/new windows
- NO plugins
- YES same-origin access (needed for srcdoc to work)

### Link Interception
- Listen for click events on the iframe
- Intercept all `<a>` clicks
- Open URLs in system browser via `@tauri-apps/plugin-shell` open()
- Never navigate the iframe to an external URL

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
