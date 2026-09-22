# MIME Parsing

Uses `mail-parser` crate in `src-tauri/crates/cxmail-email/src/email/parser.rs`.

## Input

Raw RFC822 message bytes from IMAP `FETCH BODY[]`.

## Extraction

From each message, extract:

| Field | Source | Notes |
|-------|--------|-------|
| subject | MIME Subject header | Decoded from RFC2047 (mail-parser handles this) |
| from_name | From header display name | May be empty |
| from_email | From header email | Always present |
| to_list | To header | JSON array of `{name, email}` |
| cc_list | Cc header | JSON array of `{name, email}`, may be empty |
| date | Date header | Parse to ISO 8601 |
| message_id | Message-ID header | For threading |
| in_reply_to | In-Reply-To header | For threading |
| references | References header | JSON array of Message-IDs, for threading |
| plain_text | text/plain part | Prefer from multipart/alternative |
| html_body | text/html part | Raw HTML before sanitization |
| snippet | First ~200 chars of plain_text | For message list preview |
| attachments | Non-inline MIME parts | Metadata only: filename, content-type, size |
| inline_images | MIME parts with Content-ID | For cid: references in HTML |

## Multipart Handling

```
multipart/mixed
├── multipart/alternative
│   ├── text/plain          ← plain_text
│   └── text/html           ← html_body
├── image/png (inline)      ← inline image (Content-ID present)
└── application/pdf         ← attachment
```

- For `multipart/alternative`: extract both text/plain and text/html. Prefer HTML for display, fall back to plain text.
- For `multipart/mixed`: separate inline parts (Content-Disposition: inline with Content-ID) from attachments.
- For `multipart/related`: contains HTML body that references inline images via `cid:` URLs.

## HTML Sanitization

After extracting html_body, pass through ammonia before storing as `sanitized_html`.

See `specs/security.md` for the full ammonia configuration.

## Output Structs

```rust
pub struct ParsedMessage {
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub date: String,                    // ISO 8601
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub plain_text: Option<String>,
    pub html_body: Option<String>,       // Raw
    pub sanitized_html: Option<String>,  // ammonia-cleaned
    pub snippet: Option<String>,         // First ~200 chars
    pub attachments: Vec<AttachmentMeta>,
    pub size_bytes: u64,
}

pub struct EmailAddress {
    pub name: Option<String>,
    pub email: String,
}

pub struct AttachmentMeta {
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    pub content_id: Option<String>,      // For inline images
    pub is_inline: bool,
}
```
