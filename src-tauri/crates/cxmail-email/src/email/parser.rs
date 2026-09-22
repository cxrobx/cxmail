use crate::email::calendar as ical_util;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::{FixedOffset, TimeZone, Utc};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// The pure half of this module lives in `cxmail-core` so `cxmail-db` can reach
// it without depending on `cxmail-email` — that edge was the db/email import
// cycle. Re-exported here because these are still parser concepts: callers say
// `email::parser::clean_snippet`, and did before the split.
pub use cxmail_core::mail::text::{
    clean_snippet, html_to_plain_text, html_to_plain_text_public,
    normalize_unstructured_header, repair_double_encoded_utf8, AttachmentMeta,
};

const CID_INLINE_PER_IMAGE_CAP: usize = 2 * 1024 * 1024; // 2 MB
const CID_INLINE_TOTAL_CAP: usize = 5 * 1024 * 1024; // 5 MB

/// Upper bound on a stored raw header block. Real ones run 1–8 KB; a long
/// mailing-list `Received` chain can reach ~30 KB. 256 KB is far above anything
/// legitimate and exists so a malformed message that never produces a blank
/// line can't push a multi-megabyte body into `message_headers`.
pub const RAW_HEADER_CAP: usize = 256 * 1024;

/// Split the verbatim RFC 5322 header block off a raw message.
///
/// Byte-level on purpose: this must be the bytes the server sent, not a
/// re-serialization of parsed fields, or the whole point (auditing
/// `Authentication-Results` / `Received` / DKIM) is lost. Handles CRLF and
/// bare-LF terminators, since IMAP literals are CRLF but locally-appended
/// drafts and mbox imports are not. Returns `None` when there is no blank-line
/// terminator (not a message) or the block exceeds `RAW_HEADER_CAP`.
pub fn extract_raw_headers(raw: &[u8]) -> Option<String> {
    let end = find_header_block_end(raw)?;
    if end > RAW_HEADER_CAP {
        log::warn!(
            "extract_raw_headers: header block of {} bytes exceeds cap, discarding",
            end
        );
        return None;
    }
    let text = String::from_utf8_lossy(&raw[..end]).trim_end().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Normalize an IMAP `BODY[HEADER]` response into stored form.
///
/// Unlike `extract_raw_headers` this does NOT require a blank-line terminator:
/// the server already sent the header block on its own, and whether it appends
/// the trailing CRLFCRLF varies. Same cap applies.
pub fn header_block_to_string(raw: &[u8]) -> Option<String> {
    if raw.len() > RAW_HEADER_CAP {
        log::warn!(
            "header_block_to_string: {} bytes exceeds cap, discarding",
            raw.len()
        );
        return None;
    }
    let text = String::from_utf8_lossy(raw).trim_end().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Byte offset of the end of the header block (exclusive of the blank line).
fn find_header_block_end(raw: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\n' {
            // "\n\n" or "\n\r\n" — a bare LF pair or a CRLF pair.
            if raw.get(i + 1) == Some(&b'\n') {
                return Some(i + 1);
            }
            if raw.get(i + 1) == Some(&b'\r') && raw.get(i + 2) == Some(&b'\n') {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// A parsed email message with all extracted fields.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedMessage {
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub date: Option<String>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub plain_text: Option<String>,
    pub html_body: Option<String>,
    pub sanitized_html: Option<String>,
    pub snippet: Option<String>,
    pub attachments: Vec<AttachmentMeta>,
    pub calendar_events: Vec<ical_util::CalendarEvent>,
    pub size_bytes: u64,
    /// The verbatim RFC 5322 header block (everything before the first empty
    /// line), captured because sync throws the raw bytes away and nothing in
    /// the parsed field set can reconstruct `Authentication-Results`,
    /// `Received`, DKIM signatures or any other unmodelled header. Populated
    /// for free wherever we already hold the full raw message — see
    /// `extract_raw_headers`. `None` when the block is absent or over
    /// `RAW_HEADER_CAP`.
    pub raw_headers: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub email: String,
}

/// Parse raw RFC822 message bytes into a ParsedMessage.
pub fn parse_message(raw: &[u8]) -> ParsedMessage {
    let message = match mail_parser::MessageParser::default().parse(raw) {
        Some(m) => m,
        None => {
            return ParsedMessage {
                subject: None,
                from_name: None,
                from_email: "unknown@unknown".to_string(),
                to_list: vec![],
                cc_list: vec![],
                bcc_list: vec![],
                date: None,
                message_id: None,
                in_reply_to: None,
                references: vec![],
                plain_text: None,
                html_body: None,
                sanitized_html: None,
                snippet: None,
                attachments: vec![],
                calendar_events: vec![],
                size_bytes: raw.len() as u64,
                // Still worth keeping when mail-parser gives up entirely —
                // a message we can't parse is exactly one someone will want to
                // inspect the headers of.
                raw_headers: extract_raw_headers(raw),
            };
        }
    };

    let subject = message.subject().map(normalize_unstructured_header);

    let (from_name, from_email) = message
        .from()
        .and_then(|addr| addr.first())
        .map(|a| {
            (
                a.name().map(|n| n.to_string()),
                a.address().unwrap_or("unknown@unknown").to_string(),
            )
        })
        .unwrap_or((None, "unknown@unknown".to_string()));

    let to_list = message
        .to()
        .map(|addr| extract_addresses(addr))
        .unwrap_or_default();

    let cc_list = message
        .cc()
        .map(|addr| extract_addresses(addr))
        .unwrap_or_default();

    // Bcc is symmetric to .to()/.cc(). It survives in a saved draft's MIME
    // (build_draft_raw writes a Bcc: header) so we carry it through the local
    // draft cache for reopen — see gotcha #25's sibling fix.
    let bcc_list = message
        .bcc()
        .map(|addr| extract_addresses(addr))
        .unwrap_or_default();

    let date = message.date().map(|d| {
        // Normalize to UTC so ORDER BY date DESC sorts chronologically
        let tz_secs = (d.tz_hour as i32 * 3600 + d.tz_minute as i32 * 60)
            * if d.tz_before_gmt { -1 } else { 1 };
        FixedOffset::east_opt(tz_secs)
            .and_then(|offset| {
                offset
                    .with_ymd_and_hms(
                        d.year as i32, d.month as u32, d.day as u32,
                        d.hour as u32, d.minute as u32, d.second as u32,
                    )
                    .single()
            })
            .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
            .unwrap_or_else(|| d.to_rfc3339())
    });

    let message_id = message.message_id().map(|s| s.to_string());
    let in_reply_to = message.in_reply_to().as_text().map(|s| s.to_string());
    let references: Vec<String> = message
        .references()
        .as_text_list()
        .map(|list| list.into_iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let plain_text = message.body_text(0).map(|s| s.to_string());
    let html_body = message.body_html(0).map(|s| s.to_string());

    let snippet = plain_text.as_ref().map(|text| {
        clean_snippet(text)
    }).filter(|s| !s.is_empty());

    // If plain_text produced no usable snippet, try extracting text from HTML
    let snippet = snippet.or_else(|| {
        html_body.as_ref().and_then(|html| {
            let text = html_to_plain_text(html);
            let s = clean_snippet(&text);
            if s.is_empty() { None } else { Some(s) }
        })
    });

    // Sanitize HTML, falling back to stripped version if ammonia/html5ever panics
    let mut sanitized_html = html_body.as_ref().map(|html| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sanitize_html(html))) {
            Ok(clean) => clean,
            Err(_) => {
                log::error!("sanitize_html panicked, falling back to plain text rendering");
                let escaped = html.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                format!("<pre style=\"white-space:pre-wrap\">{}</pre>", escaped)
            }
        }
    });

    // Extract attachment metadata using Message-level methods
    let attachment_count = message.attachment_count();
    log::info!(
        "parse_message: raw={} bytes, parts={}, attachment_count={}, text_body_count={}, html_body_count={}",
        raw.len(),
        message.parts.len(),
        attachment_count,
        message.text_body_count(),
        message.html_body_count(),
    );
    for (i, part) in message.parts.iter().enumerate() {
        let ct = part
            .headers
            .iter()
            .find(|h| h.name == mail_parser::HeaderName::ContentType)
            .and_then(|h| h.value.as_content_type())
            .map(|c| {
                if let Some(sub) = c.subtype() {
                    format!("{}/{}", c.ctype(), sub)
                } else {
                    c.ctype().to_string()
                }
            })
            .unwrap_or_else(|| "(none)".to_string());
        let cd = part
            .headers
            .iter()
            .find(|h| h.name == mail_parser::HeaderName::ContentDisposition)
            .and_then(|h| h.value.as_content_type())
            .map(|c| c.ctype().to_string())
            .unwrap_or_else(|| "(none)".to_string());
        log::info!("parse_message part[{}]: content-type={}, disposition={}, len={}", i, ct, cd, part.len());
    }
    let mut calendar_events = Vec::new();
    let attachments: Vec<AttachmentMeta> = (0..attachment_count)
        .filter_map(|i| {
            let part = message.attachment(i)?;
            // Use the part's len and headers for metadata
            let headers = part.headers();
            let content_type = headers.iter()
                .find(|h| h.name == mail_parser::HeaderName::ContentType)
                .and_then(|h| h.value.as_content_type())
                .map(|ct| {
                    if let Some(sub) = ct.subtype() {
                        format!("{}/{}", ct.ctype(), sub)
                    } else {
                        ct.ctype().to_string()
                    }
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());

            // Check for calendar events in text/calendar parts
            if content_type == "text/calendar" {
                if let Ok(text) = std::str::from_utf8(part.contents()) {
                    calendar_events.extend(ical_util::parse_ics(text));
                }
            }

            let filename = filename_from_headers(headers);
            let content_id = headers.iter()
                .find(|h| h.name == mail_parser::HeaderName::ContentId)
                .and_then(|h| h.value.as_text())
                .map(|s| s.to_string());
            Some(AttachmentMeta {
                filename,
                content_type: content_type.clone(),
                size_bytes: part.len() as u64,
                content_id,
                is_inline: content_type.starts_with("image/"),
            })
        })
        .collect();

    // Also check for inline text/calendar body parts (not attachments)
    for i in 0..message.parts.len() {
        let part = &message.parts[i];
        if let Some(ct) = part.headers.iter()
            .find(|h| h.name == mail_parser::HeaderName::ContentType)
            .and_then(|h| h.value.as_content_type())
        {
            if ct.ctype() == "text" && ct.subtype() == Some("calendar") {
                if let Some(body) = part.text_contents() {
                    if calendar_events.is_empty() {
                        calendar_events.extend(ical_util::parse_ics(body));
                    }
                }
            }
        }
    }

    // Inline cid: image refs as data: URLs. Walk every part (not just
    // attachments — iPhone-style multipart/related inline images aren't
    // surfaced through `message.attachment(i)`). Dedupe by normalized
    // Content-ID.
    if let Some(html) = sanitized_html.as_ref() {
        if html.contains("cid:") {
            let mut cid_parts: HashMap<String, (String, Vec<u8>)> = HashMap::new();
            for part in message.parts.iter() {
                let content_type = part
                    .headers
                    .iter()
                    .find(|h| h.name == mail_parser::HeaderName::ContentType)
                    .and_then(|h| h.value.as_content_type())
                    .map(|ct| {
                        if let Some(sub) = ct.subtype() {
                            format!("{}/{}", ct.ctype(), sub)
                        } else {
                            ct.ctype().to_string()
                        }
                    });
                let Some(content_type) = content_type else { continue };
                if !content_type.starts_with("image/") {
                    continue;
                }
                let raw_cid = part
                    .headers
                    .iter()
                    .find(|h| h.name == mail_parser::HeaderName::ContentId)
                    .and_then(|h| h.value.as_text());
                let Some(raw_cid) = raw_cid else { continue };
                let key = normalize_cid(raw_cid);
                if key.is_empty() {
                    continue;
                }
                cid_parts
                    .entry(key)
                    .or_insert_with(|| (content_type, part.contents().to_vec()));
            }

            if !cid_parts.is_empty() {
                let rewritten = inline_cid_images(html, &cid_parts);
                sanitized_html = Some(rewritten);
            }
        }
    }

    ParsedMessage {
        subject,
        from_name,
        from_email,
        to_list,
        cc_list,
        bcc_list,
        date,
        message_id,
        in_reply_to,
        references,
        plain_text,
        html_body,
        sanitized_html,
        snippet,
        attachments,
        calendar_events,
        size_bytes: raw.len() as u64,
        raw_headers: extract_raw_headers(raw),
    }
}

/// Extract the binary data of an attachment by index from raw message bytes.
pub fn extract_attachment(raw: &[u8], index: usize) -> Option<(String, Vec<u8>)> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    let part = message.attachment(index)?;
    let filename = filename_from_headers(part.headers())
        .unwrap_or_else(|| "attachment".to_string());
    Some((filename, part.contents().to_vec()))
}

/// Extract every attachment from a raw RFC822 message as
/// `(filename, content_type, bytes, is_inline)` tuples.
///
/// Used to reload an existing draft's attachments into the compose window so
/// they are visible to the user and preserved when the draft is re-saved or
/// sent. The `is_inline` flag marks cid: body images that are already embedded
/// as `data:` URIs in the sanitized HTML (see `inline_cid_images`); the caller
/// drops those to avoid duplicating body images, while keeping genuine file
/// attachments (PDFs, docs, and images sent with `Content-Disposition: attachment`).
pub fn extract_all_attachments(raw: &[u8]) -> Vec<(String, String, Vec<u8>, bool)> {
    let Some(message) = mail_parser::MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    let count = message.attachment_count();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let Some(part) = message.attachment(i) else {
            continue;
        };
        let headers = part.headers();
        let content_type = headers
            .iter()
            .find(|h| h.name == mail_parser::HeaderName::ContentType)
            .and_then(|h| h.value.as_content_type())
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let filename =
            filename_from_headers(headers).unwrap_or_else(|| "attachment".to_string());
        let disposition_inline = headers
            .iter()
            .find(|h| h.name == mail_parser::HeaderName::ContentDisposition)
            .and_then(|h| h.value.as_content_type())
            .map(|cd| cd.ctype().eq_ignore_ascii_case("inline"))
            .unwrap_or(false);
        let has_content_id = headers
            .iter()
            .any(|h| h.name == mail_parser::HeaderName::ContentId);
        // Embedded body images (cid:) are already inlined as data: URIs in the
        // sanitized HTML, so don't reload them as separate attachments. A real
        // image attachment uses `Content-Disposition: attachment` with no cid.
        let is_inline =
            disposition_inline || (has_content_id && content_type.starts_with("image/"));
        out.push((filename, content_type, part.contents().to_vec(), is_inline));
    }
    out
}

fn filename_from_headers(headers: &[mail_parser::Header<'_>]) -> Option<String> {
    headers.iter()
        .find(|h| h.name == mail_parser::HeaderName::ContentDisposition)
        .and_then(|h| h.value.as_content_type())
        .and_then(|ct| ct.attribute("filename"))
        .or_else(|| {
            headers.iter()
                .find(|h| h.name == mail_parser::HeaderName::ContentType)
                .and_then(|h| h.value.as_content_type())
                .and_then(|ct| ct.attribute("name"))
        })
        .map(|s| s.to_string())
}

/// Extract addresses from a mail-parser Address enum.
fn extract_addresses(addr: &mail_parser::Address) -> Vec<EmailAddress> {
    addr.iter()
        .map(|a| EmailAddress {
            name: a.name().map(|n| n.to_string()),
            email: a.address().unwrap_or("").to_string(),
        })
        .collect()
}
/// Sanitize HTML email content using ammonia.
pub fn sanitize_html(html: &str) -> String {
    let mut allowed_tags: HashSet<&str> = HashSet::new();
    for tag in &[
        "p", "div", "span", "a", "img", "br", "hr",
        "b", "i", "strong", "em", "u", "s", "strike",
        "h1", "h2", "h3", "h4", "h5", "h6",
        "ul", "ol", "li",
        "table", "thead", "tbody", "tfoot", "tr", "th", "td",
        "blockquote", "pre", "code",
        "sup", "sub", "small",
        "center", "font",
    ] {
        allowed_tags.insert(tag);
    }

    let cleaned = ammonia::Builder::new()
        .tags(allowed_tags)
        .link_rel(Some("noopener noreferrer"))
        .add_generic_attributes(&["style", "class", "id", "dir", "lang"])
        .add_tag_attributes("a", &["href", "title", "target"])
        .add_tag_attributes("img", &["src", "alt", "width", "height"])
        .add_tag_attributes("td", &["colspan", "rowspan", "align", "valign", "width", "height"])
        .add_tag_attributes("th", &["colspan", "rowspan", "align", "valign", "width", "height"])
        .add_tag_attributes("table", &["cellpadding", "cellspacing", "border", "width"])
        .add_tag_attributes("font", &["color", "size", "face"])
        // cid: is required for inline image attachments; data: is required for
        // base64-embedded signature logos. Neither makes a network request, so
        // they don't carry the privacy concerns that drive remote-image blocking.
        // The attribute_filter below restricts cid:/data: to <img src=...> so
        // they can't be used as <a href=...> targets (which could navigate to
        // a data: HTML page inside the iframe).
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid", "data"]))
        .attribute_filter(|element, attribute, value| {
            if (element == "a" || element == "area") && attribute == "href" {
                let lower = value.to_ascii_lowercase();
                if lower.starts_with("data:") || lower.starts_with("cid:") {
                    return None;
                }
            }
            Some(value.into())
        })
        .clean(html)
        .to_string();

    // Add target="_blank" to links
    rewrite_remote_images(&cleaned.replace("<a ", "<a target=\"_blank\" "))
}

fn rewrite_remote_images(html: &str) -> String {
    remote_image_regex().replace_all(html, rewrite_remote_image).into_owned()
}

fn remote_image_regex() -> &'static Regex {
    static REMOTE_IMAGE_RE: OnceLock<Regex> = OnceLock::new();
    REMOTE_IMAGE_RE.get_or_init(|| {
        Regex::new(r#"(?i)<img([^>]*?)\s+src="(https?://[^"]+)"([^>]*)>"#)
            .expect("valid remote image regex")
    })
}

fn rewrite_remote_image(captures: &Captures<'_>) -> String {
    let before = captures.get(1).map_or("", |value| value.as_str());
    let src = captures.get(2).map_or("", |value| value.as_str());
    let after = captures.get(3).map_or("", |value| value.as_str());

    format!(
        r#"<img{} src="" data-original-src="{}"{}>"#,
        before,
        src,
        after,
    )
}

/// Normalize a Content-ID or `cid:` URL identifier for matching. Strips angle
/// brackets, percent-decodes, lowercases. Per RFC 2392 cid: matching is
/// case-insensitive.
fn normalize_cid(raw: &str) -> String {
    let trimmed = raw.trim();
    let no_brackets = trimmed.trim_start_matches('<').trim_end_matches('>');
    percent_decode(no_brackets).to_ascii_lowercase()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cid_image_regex() -> &'static Regex {
    static CID_IMAGE_RE: OnceLock<Regex> = OnceLock::new();
    CID_IMAGE_RE.get_or_init(|| {
        Regex::new(r#"(?i)<img([^>]*?)\s+src="cid:([^"]+)"([^>]*)>"#)
            .expect("valid cid image regex")
    })
}

/// Rewrite `<img src="cid:XXX">` references to `data:<ct>;base64,...` URLs
/// using the matching parts in `parts` (keyed by normalized Content-ID).
/// Unmatched cids are left untouched. Per-image and total size caps prevent
/// SQLite bloat — over the cap the original cid: ref stays in place.
pub fn inline_cid_images(
    html: &str,
    parts: &HashMap<String, (String, Vec<u8>)>,
) -> String {
    if parts.is_empty() {
        return html.to_string();
    }

    let mut total_inlined: usize = 0;

    cid_image_regex()
        .replace_all(html, |captures: &Captures<'_>| {
            let before = captures.get(1).map_or("", |v| v.as_str());
            let cid_raw = captures.get(2).map_or("", |v| v.as_str());
            let after = captures.get(3).map_or("", |v| v.as_str());

            let key = normalize_cid(cid_raw);
            let Some((content_type, bytes)) = parts.get(&key) else {
                return captures.get(0).map_or(String::new(), |m| m.as_str().to_string());
            };

            if bytes.len() > CID_INLINE_PER_IMAGE_CAP {
                log::warn!(
                    "inline_cid_images: skipping cid={} (size={} > per-image cap)",
                    cid_raw,
                    bytes.len()
                );
                return captures.get(0).map_or(String::new(), |m| m.as_str().to_string());
            }
            if total_inlined.saturating_add(bytes.len()) > CID_INLINE_TOTAL_CAP {
                log::warn!(
                    "inline_cid_images: skipping cid={} (would exceed total cap)",
                    cid_raw
                );
                return captures.get(0).map_or(String::new(), |m| m.as_str().to_string());
            }

            total_inlined = total_inlined.saturating_add(bytes.len());
            let encoded = BASE64_STANDARD.encode(bytes);
            format!(
                r#"<img{} src="data:{};base64,{}"{}>"#,
                before, content_type, encoded, after
            )
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        extract_all_attachments, extract_raw_headers, header_block_to_string, html_to_plain_text,
        inline_cid_images, normalize_unstructured_header, parse_message,
        repair_double_encoded_utf8, sanitize_html, RAW_HEADER_CAP,
    };
    use regex::Regex;
    use std::collections::HashMap;

    /// The snippet bug: `out.push(bytes[i] as char)` is a Latin-1 decode, so
    /// each UTF-8 byte of `“` became its own char and was re-encoded — the inbox
    /// list showed `âA writerâand` for `“A writer—and`. Mutation-fails if the
    /// byte accumulator is reverted to a String of `as char` pushes.
    #[test]
    fn html_to_plain_text_keeps_multibyte_characters_intact() {
        let html = "<p>\u{201C}A writer\u{2014}and, I believe, generally all \
                    persons\u{2014}must think\u{201D} \u{2014} caf\u{e9}, na\u{ef}ve, \u{1f600}</p>";
        let text = html_to_plain_text(html);

        assert!(text.contains("\u{201C}A writer\u{2014}and"), "got: {text:?}");
        assert!(text.contains("caf\u{e9}"), "got: {text:?}");
        assert!(text.contains("na\u{ef}ve"), "got: {text:?}");
        assert!(text.contains('\u{1f600}'), "4-byte char lost: {text:?}");
        // The tell-tale mojibake lead byte, and the invisible C1 controls that
        // made the corruption hard to see in the UI.
        assert!(!text.contains('\u{e2}'), "double-encoded: {text:?}");
        assert!(
            !text.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)),
            "C1 control leaked: {text:?}"
        );
    }

    /// A multi-byte character inside a tag must be skipped whole, not half —
    /// the state machine only transitions on ASCII `<`/`>`, so this holds.
    #[test]
    fn html_to_plain_text_skips_non_ascii_inside_tags_cleanly() {
        let text = html_to_plain_text("<a title=\"caf\u{e9} \u{2014} ol\u{e9}\">visible\u{2019}s</a>");
        assert_eq!(text.trim(), "visible\u{2019}s");
    }

    #[test]
    fn repair_double_encoded_utf8_restores_the_original_text() {
        // Exactly what uid 108123 held: C3A2 C280 C29C for a single `“`.
        let broken = "\u{e2}\u{80}\u{9c}A writer\u{e2}\u{80}\u{94}and";
        assert_eq!(
            repair_double_encoded_utf8(broken).as_deref(),
            Some("\u{201C}A writer\u{2014}and")
        );
    }

    /// The invisible-char strip ran AFTER the mangling, so mangled zero-widths
    /// survived into stored snippets. Un-mangling one has to strip it, or the
    /// repaired snippet differs from what the fixed code now produces.
    #[test]
    fn repair_double_encoded_utf8_strips_the_zero_widths_the_bug_smuggled_past() {
        // U+200B (E2 80 8B) decoded as Latin-1.
        let broken = "hi\u{e2}\u{80}\u{8b}there";
        assert_eq!(repair_double_encoded_utf8(broken).as_deref(), Some("hithere"));
    }

    /// The guards, and the whole reason this is safe to run over every stored
    /// snippet: legitimate text is left alone rather than mangled in reverse.
    #[test]
    fn repair_double_encoded_utf8_declines_text_that_is_not_mojibake() {
        for clean in [
            "plain ascii only",
            "Caf\u{e9} au lait",       // E9 alone is not valid UTF-8
            "\u{e2}me et conscience",  // E2 not followed by a continuation byte
            "\u{201C}already fixed\u{201D}", // > U+00FF, fails guard 1
            "R\u{e9}sum\u{e9}",
            "",
        ] {
            assert_eq!(repair_double_encoded_utf8(clean), None, "mangled: {clean:?}");
        }
    }

    /// `clean_snippet` truncates at 200 bytes on a char boundary, which for
    /// mojibake lands mid-logical-sequence and strands a lead byte. One dangling
    /// byte at the tail used to reject the entire 191-char snippet.
    #[test]
    fn repair_double_encoded_utf8_survives_a_truncated_tail() {
        // "…22:11:50 EDT" then a stranded C2 (lead byte of a 2-byte sequence).
        let broken = "22:11:50 EDT\u{c2}";
        assert_eq!(
            repair_double_encoded_utf8(broken).as_deref(),
            Some("22:11:50 EDT")
        );
        // A dangling sequence anywhere but the end means genuinely malformed
        // data, so that run is left alone rather than silently shortened.
        let mid = "a\u{c2}\u{34f}b";
        assert_eq!(repair_double_encoded_utf8(mid), None);
    }

    /// Entities are decoded AFTER the mangling ran, so a real U+034F can sit
    /// beside corrupted bytes. Whole-string guard 1 threw the row away over that
    /// one good char; per-piece keeps it and repairs the rest.
    #[test]
    fn repair_double_encoded_utf8_repairs_around_characters_that_survived() {
        let broken = "you can\u{e2}\u{80}\u{99}t \u{34f}be bothered\u{34f}";
        assert_eq!(
            repair_double_encoded_utf8(broken).as_deref(),
            Some("you can\u{2019}t \u{34f}be bothered\u{34f}")
        );
    }

    /// Idempotent — the migration is safe to re-run, and an older binary
    /// stamping the schema version back down re-runs it (gotchas #26, #37).
    #[test]
    fn repair_double_encoded_utf8_is_idempotent() {
        let broken = "\u{e2}\u{80}\u{9c}quoted\u{e2}\u{80}\u{9d}";
        let once = repair_double_encoded_utf8(broken).expect("repairs once");
        assert_eq!(repair_double_encoded_utf8(&once), None);
    }

    /// The motivating header (T21): a DMARC verdict cannot be recovered from any
    /// parsed field, so the block has to be kept byte-for-byte.
    #[test]
    fn extract_raw_headers_keeps_unmodelled_headers_verbatim() {
        let raw = b"Received: from mail.example.com (mail.example.com [203.0.113.9])\r\n\
                    \tby mx.google.com with ESMTPS id abc123\r\n\
                    Authentication-Results: mx.google.com;\r\n\
                    \tdkim=pass header.i=@example.com;\r\n\
                    \tdmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=example.com\r\n\
                    Subject: hello\r\n\
                    \r\n\
                    Body text that must NOT appear.\r\n";
        let headers = extract_raw_headers(raw).expect("header block");
        assert!(headers.contains("Authentication-Results: mx.google.com;"));
        assert!(
            headers.contains("dmarc=pass (p=REJECT sp=REJECT dis=NONE)"),
            "folded continuation must survive: {headers}"
        );
        assert!(headers.contains("203.0.113.9"), "Received chain must survive");
        assert!(
            !headers.contains("Body text"),
            "body must be excluded: {headers}"
        );
    }

    #[test]
    fn extract_raw_headers_handles_bare_lf_and_missing_terminator() {
        // Locally-appended drafts and mbox imports are LF-terminated, not CRLF.
        let lf = extract_raw_headers(b"Subject: x\nFrom: a@b.c\n\nbody").expect("LF block");
        assert_eq!(lf, "Subject: x\nFrom: a@b.c");
        // No blank line at all => not a message; better to store nothing than
        // to store the entire body under the name "headers".
        assert!(extract_raw_headers(b"Subject: x\r\nFrom: a@b.c\r\n").is_none());
        assert!(extract_raw_headers(b"").is_none());
    }

    #[test]
    fn extract_raw_headers_refuses_an_oversized_block() {
        let mut raw = Vec::new();
        while raw.len() <= RAW_HEADER_CAP {
            raw.extend_from_slice(b"X-Padding: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        raw.extend_from_slice(b"\r\nbody");
        assert!(extract_raw_headers(&raw).is_none());
    }

    /// A `BODY[HEADER]` response arrives without a body, so requiring the blank
    /// line (as `extract_raw_headers` does) would reject the lazy-fetch path's
    /// output on any server that omits the trailing CRLFCRLF.
    #[test]
    fn header_block_to_string_does_not_require_a_terminator() {
        assert_eq!(
            header_block_to_string(b"Subject: x\r\nFrom: a@b.c\r\n").as_deref(),
            Some("Subject: x\r\nFrom: a@b.c")
        );
        assert!(header_block_to_string(b"   \r\n").is_none());
    }

    #[test]
    fn parse_message_populates_raw_headers() {
        let raw = b"From: a@b.c\r\nSubject: hi\r\nX-Custom: kept\r\n\r\nbody\r\n";
        let parsed = parse_message(raw);
        let headers = parsed.raw_headers.expect("raw_headers populated");
        assert!(headers.contains("X-Custom: kept"));
    }

    #[test]
    fn extract_all_attachments_separates_real_files_from_inline_images() {
        // multipart/mixed: html body referencing a cid: image, a real PDF
        // attachment, and the inline png that backs the cid.
        let raw = "From: a@example.com\r\n\
To: b@example.com\r\n\
Subject: Test\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hello <img src=\"cid:img1\"></p>\r\n\
--BOUND\r\n\
Content-Type: application/pdf; name=\"report.pdf\"\r\n\
Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--BOUND\r\n\
Content-Type: image/png\r\n\
Content-Disposition: inline\r\n\
Content-Id: <img1>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--BOUND--\r\n";

        let atts = extract_all_attachments(raw.as_bytes());
        assert_eq!(
            atts.len(),
            2,
            "expected pdf + inline image, got {:?}",
            atts.iter().map(|a| (&a.0, &a.1, a.3)).collect::<Vec<_>>()
        );

        let pdf = atts.iter().find(|a| a.0 == "report.pdf").expect("pdf present");
        assert_eq!(pdf.1, "application/pdf");
        assert!(!pdf.2.is_empty(), "pdf bytes must be decoded");
        assert!(!pdf.3, "a real file attachment must NOT be flagged inline");

        let img = atts
            .iter()
            .find(|a| a.1.starts_with("image/"))
            .expect("inline image present");
        assert!(
            img.3,
            "a cid: inline image must be flagged inline so the caller drops it"
        );
    }

    #[test]
    fn parse_message_extracts_bcc_list() {
        // A saved draft carries a Bcc: header (build_draft_raw writes it). The
        // parser must surface it so the local draft cache can round-trip Bcc on
        // reopen (gotcha #25 sibling fix).
        let raw = "From: me@example.com\r\n\
To: a@example.com\r\n\
Cc: c@example.com\r\n\
Bcc: secret@example.com, hidden@example.com\r\n\
Subject: Test\r\n\
\r\n\
body\r\n";

        let parsed = parse_message(raw.as_bytes());
        let bcc: Vec<&str> = parsed.bcc_list.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(bcc, vec!["secret@example.com", "hidden@example.com"]);
        // Sanity: the other recipient lists are unaffected.
        assert_eq!(
            parsed.to_list.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
            vec!["a@example.com"]
        );
        assert_eq!(
            parsed.cc_list.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
            vec!["c@example.com"]
        );
    }

    #[test]
    fn normalize_unescapes_amazon_style_quoted_subject() {
        assert_eq!(
            normalize_unstructured_header(r#"Shipped: \"Layla 300 Thread Count...\""#),
            r#"Shipped: "Layla 300 Thread Count...""#
        );
    }

    #[test]
    fn normalize_unescapes_double_backslash() {
        assert_eq!(normalize_unstructured_header(r"foo\\bar"), r"foo\bar");
    }

    #[test]
    fn normalize_preserves_unknown_backslash_escapes() {
        assert_eq!(normalize_unstructured_header(r"foo\nbar"), r"foo\nbar");
    }

    #[test]
    fn normalize_preserves_trailing_backslash() {
        assert_eq!(normalize_unstructured_header(r"weird\"), r"weird\");
    }

    #[test]
    fn normalize_is_noop_without_backslashes() {
        assert_eq!(
            normalize_unstructured_header("Hello \"World\""),
            "Hello \"World\""
        );
        assert_eq!(
            normalize_unstructured_header("plain subject"),
            "plain subject"
        );
    }

    #[test]
    fn normalize_is_idempotent() {
        let once = normalize_unstructured_header(r#"\"foo\""#);
        let twice = normalize_unstructured_header(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn sanitize_html_rewrites_remote_image_sources() {
        let html = r#"<p>Hello</p><img src="https://tracker.example/pixel.png" alt="pixel">"#;
        let sanitized = sanitize_html(html);
        let remote_src = Regex::new(r#"<img[^>]*\ssrc="https://tracker\.example/pixel\.png""#)
            .expect("valid image source regex");

        assert!(sanitized.contains(r#"data-original-src="https://tracker.example/pixel.png""#));
        assert!(sanitized.contains(r#"src="""#));
        assert!(!remote_src.is_match(&sanitized));
    }

    #[test]
    fn sanitize_html_preserves_http_links() {
        let html = r#"<a href="https://example.com">Example</a>"#;
        let sanitized = sanitize_html(html);

        assert!(sanitized.contains(r#"href="https://example.com""#));
        assert!(sanitized.contains(r#"target="_blank""#));
    }

    #[test]
    fn sanitize_html_preserves_inline_image_schemes() {
        let html = r#"<img src="cid:logo@signature" alt="logo"><img src="data:image/png;base64,AAAA" alt="pixel">"#;
        let sanitized = sanitize_html(html);

        assert!(sanitized.contains(r#"src="cid:logo@signature""#));
        assert!(sanitized.contains(r#"src="data:image/png;base64,AAAA""#));
    }

    #[test]
    fn sanitize_html_strips_data_and_cid_from_anchor_hrefs() {
        let html = r#"<a href="data:text/html,<script>alert(1)</script>">data</a><a href="cid:trick">cid</a><a href="DATA:text/html,x">upper</a>"#;
        let sanitized = sanitize_html(html);

        // The anchors should remain (as plain text wrappers) but their hrefs should be gone.
        assert!(!sanitized.contains("href=\"data:"));
        assert!(!sanitized.contains("href=\"cid:"));
        assert!(!sanitized.contains("href=\"DATA:"));
    }

    fn cid_parts_with(
        entries: &[(&str, &str, &[u8])],
    ) -> std::collections::HashMap<String, (String, Vec<u8>)> {
        entries
            .iter()
            .map(|(cid, ct, bytes)| (cid.to_string(), (ct.to_string(), bytes.to_vec())))
            .collect()
    }

    #[test]
    fn inline_cid_images_replaces_matching_cid_with_data_url() {
        let html = r#"<p>hi</p><img src="cid:logo@host" alt="logo">"#;
        let parts = cid_parts_with(&[("logo@host", "image/png", &[0xDE, 0xAD, 0xBE, 0xEF])]);

        let out = super::inline_cid_images(html, &parts);

        assert!(out.contains(r#"src="data:image/png;base64,3q2+7w==""#));
        assert!(!out.contains("cid:logo@host"));
    }

    #[test]
    fn inline_cid_images_strips_brackets_and_lowercases_for_match() {
        let html = r#"<img src="cid:Foo@Host" alt="x">"#;
        let parts = cid_parts_with(&[("foo@host", "image/jpeg", &[0x01, 0x02])]);

        let out = super::inline_cid_images(html, &parts);
        assert!(out.contains("data:image/jpeg;base64,"));
        assert!(!out.contains("cid:Foo@Host"));
    }

    #[test]
    fn inline_cid_images_percent_decodes_at_lookup() {
        let html = r#"<img src="cid:foo%40host" alt="x">"#;
        let parts = cid_parts_with(&[("foo@host", "image/gif", &[0x99])]);

        let out = super::inline_cid_images(html, &parts);
        assert!(out.contains("data:image/gif;base64,"));
    }

    #[test]
    fn inline_cid_images_leaves_unmatched_cid_alone() {
        let html = r#"<img src="cid:missing@host">"#;
        let parts = cid_parts_with(&[("known@host", "image/png", &[0xAA])]);

        let out = super::inline_cid_images(html, &parts);
        assert!(out.contains(r#"src="cid:missing@host""#));
        assert!(!out.contains("data:image/png"));
    }

    #[test]
    fn inline_cid_images_replaces_multiple_in_one_html() {
        let html = r#"<img src="cid:a@h"><span>x</span><img src="cid:b@h">"#;
        let parts = cid_parts_with(&[
            ("a@h", "image/png", &[0x01]),
            ("b@h", "image/png", &[0x02]),
        ]);

        let out = super::inline_cid_images(html, &parts);
        assert!(!out.contains("cid:a@h"));
        assert!(!out.contains("cid:b@h"));
        assert_eq!(out.matches("data:image/png;base64,").count(), 2);
    }

    #[test]
    fn inline_cid_images_skips_oversized_image_keeps_cid() {
        let big = vec![0xAB; super::CID_INLINE_PER_IMAGE_CAP + 1];
        let html = r#"<img src="cid:big@h"><img src="cid:small@h">"#;
        let parts = cid_parts_with(&[
            ("big@h", "image/png", big.as_slice()),
            ("small@h", "image/png", &[0x55]),
        ]);

        let out = super::inline_cid_images(html, &parts);
        assert!(out.contains(r#"src="cid:big@h""#));
        assert!(out.contains("data:image/png;base64,"));
        assert!(!out.contains(r#"src="cid:small@h""#));
    }

    #[test]
    fn inline_cid_images_does_not_touch_non_img_cid_refs() {
        let html = r#"<a href="cid:a@h">x</a><img src="cid:a@h">"#;
        let parts = cid_parts_with(&[("a@h", "image/png", &[0xFF])]);

        let out = super::inline_cid_images(html, &parts);
        assert!(out.contains(r#"<a href="cid:a@h""#));
        assert!(out.contains("<img src=\"data:image/png;base64,"));
    }

    #[test]
    fn parse_message_inlines_multipart_related_image() {
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        let raw = format!(
            "From: a@b.com\r\n\
             To: c@d.com\r\n\
             Subject: inline image\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/related; boundary=BOUNDARY\r\n\
             \r\n\
             --BOUNDARY\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: 7bit\r\n\
             \r\n\
             <html><body><p>see image</p><img src=\"cid:pix@iphone\"></body></html>\r\n\
             --BOUNDARY\r\n\
             Content-Type: image/png\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-ID: <pix@iphone>\r\n\
             Content-Disposition: inline\r\n\
             \r\n\
             {}\r\n\
             --BOUNDARY--\r\n",
            png_b64
        );

        let parsed = super::parse_message(raw.as_bytes());
        let html = parsed.sanitized_html.expect("sanitized_html present");
        assert!(
            html.contains("data:image/png;base64,"),
            "expected data: URL in sanitized html, got: {}",
            html
        );
        assert!(
            !html.contains("cid:pix@iphone"),
            "cid: ref should be replaced, got: {}",
            html
        );
    }
}
