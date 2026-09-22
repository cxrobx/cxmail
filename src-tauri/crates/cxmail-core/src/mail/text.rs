//! Pure text helpers shared by the parser and the database layer.
//!
//! These live in `cxmail-core` rather than `cxmail-email` because `cxmail-db`
//! needs them — `db::messages::backfill_snippets_from_bodies` derives snippets
//! and `db::schema` repairs mojibake during migration. Keeping them here is
//! what makes the dependency edge point one way (`core <- db <- email`) instead
//! of the cycle those two modules used to form.
//!
//! Nothing in here touches mail_parser, ammonia, or the network. `email::parser`
//! re-exports every item, so `email::parser::clean_snippet` still resolves.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentMeta {
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// Un-escape JSON-style backslash sequences that some senders (notably Amazon)
/// emit literally into RFC 5322 unstructured headers like Subject. Conservative:
/// only `\"` and `\\` are unescaped; any other `\X` is left as-is so we don't
/// silently corrupt headers that legitimately contain backslashes.
pub fn normalize_unstructured_header<S: AsRef<str>>(input: S) -> String {
    let s = input.as_ref();
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('"') => {
                    chars.next();
                    out.push('"');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}



/// Check if a line looks like CSS (selector block, property, or at-rule).
fn is_css_line(trimmed: &str) -> bool {
    // Lines with { } that look like CSS selector blocks (e.g., "p{ }", "table{ }", "h1,h2{ }")
    if trimmed.contains('{') {
        // CSS property keywords
        if trimmed.contains("margin") || trimmed.contains("padding")
            || trimmed.contains("font-") || trimmed.contains("color:")
            || trimmed.contains("background") || trimmed.contains("text-")
            || trimmed.contains("border") || trimmed.contains("display:")
            || trimmed.contains("width:") || trimmed.contains("height:") {
            return true;
        }
        // CSS selector prefixes: class, id, at-rule
        if trimmed.starts_with('.') || trimmed.starts_with('#') || trimmed.starts_with('@') {
            return true;
        }
        // Bare element selectors: "p{", "div{", "table{", "h1,h2,h3{", "img,a{", etc.
        // Check if everything before the first '{' looks like CSS selectors
        if let Some(before_brace) = trimmed.split('{').next() {
            let selector = before_brace.trim();
            if !selector.is_empty() && selector.len() < 200 {
                let looks_like_selectors = selector.split(',').all(|part| {
                    let p = part.trim().trim_start_matches('*');
                    // CSS selectors are short identifiers, possibly with pseudo-classes
                    p.is_empty() || (p.len() < 40 && !p.contains(' ')
                        && p.chars().all(|c| c.is_alphanumeric() || "-_.:>+~[]()".contains(c)))
                });
                if looks_like_selectors {
                    return true;
                }
            }
        }
    }
    // CSS property lines: "property: value;"
    if trimmed.ends_with(';') && trimmed.contains(':') && !trimmed.contains("://") {
        return true;
    }
    // Closing brace only
    if trimmed == "}" || trimmed == "};" {
        return true;
    }
    false
}

/// Extract a clean snippet from email body text.

/// Strips CSS, HTML artifacts, excessive whitespace, and common boilerplate.
pub fn clean_snippet(text: &str) -> String {
    let mut result = String::new();
    let mut in_css = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Detect and skip CSS blocks
        if is_css_line(trimmed) {
            if trimmed.contains('{') && !trimmed.contains('}') {
                in_css = true;
            }
            continue;
        }
        if in_css {
            if trimmed.contains('}') {
                in_css = false;
            }
            continue;
        }

        // Skip HTML-like artifacts
        if trimmed.starts_with('<') || trimmed.starts_with("<!") {
            continue;
        }

        // Skip common boilerplate
        if trimmed.starts_with("---") || trimmed.starts_with("___")
            || trimmed.starts_with("***") || trimmed == "--" {
            break; // signature separator, stop here
        }

        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(trimmed);

        if result.len() >= 200 {
            break;
        }
    }

    // Collapse multiple spaces
    let collapsed: String = result.split_whitespace().collect::<Vec<_>>().join(" ");

    // Truncate to ~200 bytes on a word boundary. Use a char-safe slice so we
    // don't panic if byte 200 lands inside a multi-byte UTF-8 sequence (e.g.
    // \u{34f} combining grapheme joiner in lululemon marketing emails).
    let truncated = if collapsed.len() <= 200 {
        collapsed
    } else {
        let mut cutoff = 200;
        while cutoff > 0 && !collapsed.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        let head = &collapsed[..cutoff];
        match head.rfind(' ') {
            Some(pos) => head[..pos].to_string(),
            None => head.to_string(),
        }
    };

    // Decode any HTML entities (e.g. &#x27;, &amp;) that leaked through from
    // senders whose plain-text part was generated by a crude HTML strip.
    html_escape::decode_html_entities(&truncated).into_owned()
}

/// Public re-export of `html_to_plain_text` for use by the DB snippet backfill.
pub fn html_to_plain_text_public(html: &str) -> String {
    html_to_plain_text(html)
}

/// Strip HTML tags to produce plain text for snippet extraction.
pub fn html_to_plain_text(html: &str) -> String {
    // Simple tag stripping — no need for a full HTML parser here.
    //
    // `out` accumulates BYTES, not chars. This used to be a String fed by
    // `out.push(bytes[i] as char)`, which is a Latin-1 decode: the three UTF-8
    // bytes of `—` (E2 80 94) each became their own char and were then
    // re-encoded, so every smart quote and em dash in an HTML-only newsletter
    // reached `messages.snippet` double-encoded (`â` plus two invisible C1
    // controls). Copying the bytes through and decoding once at the end is what
    // keeps a multi-byte character intact — same shape as `percent_decode`
    // below. Splitting a character across the tag state machine is impossible:
    // it only transitions on the ASCII bytes `<` and `>`, while every byte of a
    // multi-byte sequence is >= 0x80, so a sequence is always wholly inside or
    // wholly outside a tag.
    let mut out: Vec<u8> = Vec::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_style = false;
    let mut in_script = false;

    let bytes = html.as_bytes();

    // ASCII-lowercased copy of `html` used only for case-insensitive tag sniffing.
    // `str::to_lowercase` can change byte length for certain Unicode chars, which
    // would misalign with `bytes[i]` and panic on `lower_bytes[i..]`. ASCII-only
    // lowercasing preserves byte length.
    let lower_owned: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
    let lower_bytes: &[u8] = &lower_owned;

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Check for <style or <script
            if lower_bytes[i..].starts_with(b"<style") {
                in_style = true;
            } else if lower_bytes[i..].starts_with(b"<script") {
                in_script = true;
            } else if lower_bytes[i..].starts_with(b"</style") {
                in_style = false;
            } else if lower_bytes[i..].starts_with(b"</script") {
                in_script = false;
            }
            // Add space for block-level tags
            if lower_bytes[i..].starts_with(b"<br") || lower_bytes[i..].starts_with(b"<p")
                || lower_bytes[i..].starts_with(b"<div") || lower_bytes[i..].starts_with(b"<tr")
                || lower_bytes[i..].starts_with(b"<li") {
                out.push(b' ');
            }
            in_tag = true;
        } else if bytes[i] == b'>' {
            in_tag = false;
        } else if !in_tag && !in_style && !in_script {
            out.push(bytes[i]);
        }
        i += 1;
    }

    let out = String::from_utf8_lossy(&out);
    // Decode all HTML entities (named, numeric, hex)
    let decoded = html_escape::decode_html_entities(&out).into_owned();
    strip_invisible_chars(&decoded)
}

/// Strip zero-width characters that clutter snippets.
///
/// Shared with `repair_double_encoded_utf8` on purpose: the Latin-1 bug mangled
/// these into three-char sequences *before* this ran, so they slipped through
/// and are still sitting in already-stored snippets. Un-mangling one restores a
/// real U+200B, which then has to be stripped here or the repaired snippet ends
/// up different from what the fixed code would have produced.
fn strip_invisible_chars(s: &str) -> String {
    s.replace('\u{200C}', "") // &zwnj; zero-width non-joiner
        .replace('\u{200B}', "") // zero-width space
        .replace('\u{200D}', "") // &zwj; zero-width joiner
        .replace('\u{FEFF}', "") // byte order mark
        .replace('\u{00AD}', "") // &shy; soft hyphen
}

/// Reverse a UTF-8-decoded-as-Latin-1 round trip ("mojibake"), or return `None`.
///
/// Repairs snippets written before `html_to_plain_text` stopped doing
/// `bytes[i] as char` — `“` reaching the DB as `Ã¢â‚¬Å“`. Returns `None` unless
/// all three guards hold, because the transform is only safe when the input
/// really is a re-encoded byte string:
///
/// 1. every char is <= U+00FF, so each one *was* a single Latin-1 byte;
/// 2. those bytes are STRICTLY valid UTF-8 (never `from_utf8_lossy` — a lossy
///    decode would happily turn genuine Latin-1 prose into replacement chars);
/// 3. the result actually differs from the input.
///
/// Legitimate text fails these: "Café" is `43 61 66 E9`, and a lone `E9` is not
/// valid UTF-8; "âme" is `E2 6D 65`, and `6D` is not a continuation byte. Pure
/// ASCII decodes to itself and is rejected by guard 3. Already-repaired text
/// holds U+201C, which fails guard 1 — so this is idempotent.
///
/// Applied per PIECE, not to the whole string, because a stored snippet can
/// legitimately mix the two: `clean_snippet` decodes HTML entities *after* the
/// mangling ran, so a `&#847;` in a lululemon email is a correct U+034F sitting
/// beside corrupted bytes. Whole-string guard 1 threw those rows away over a
/// single good character. A piece is a maximal run of chars <= U+00FF and must
/// still decode strictly *in its entirety*, so the guards lose no strength —
/// one genuine accented char anywhere in a run still rejects that whole run.
pub fn repair_double_encoded_utf8(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut buf: Vec<u8> = Vec::new();
    let mut changed = false;

    for c in s.chars() {
        let cp = c as u32;
        if cp <= 0xFF {
            buf.push(cp as u8);
        } else {
            // Never a Latin-1 byte, so it cannot be part of a mangled sequence:
            // it ends the run and passes through untouched.
            flush_latin1_piece(&mut buf, &mut out, &mut changed, false);
            out.push(c);
        }
    }
    flush_latin1_piece(&mut buf, &mut out, &mut changed, true);

    if !changed {
        return None; // guard 3
    }
    let cleaned = strip_invisible_chars(&out);
    if cleaned == s {
        return None;
    }
    Some(cleaned)
}

/// Decode one run of Latin-1 bytes back to UTF-8, or copy it through unchanged.
///
/// `allow_dangling_tail` is only ever true for the run that ends the string.
/// `clean_snippet` truncates to 200 bytes on a *char* boundary — which, when
/// the chars are mojibake, lands mid-logical-sequence and strands a lead byte
/// with no continuation. Dropping up to 3 such bytes rescues the row; anywhere
/// else in the string an incomplete sequence means the data really is malformed,
/// so the run is left alone.
fn flush_latin1_piece(
    buf: &mut Vec<u8>,
    out: &mut String,
    changed: &mut bool,
    allow_dangling_tail: bool,
) {
    if buf.is_empty() {
        return;
    }
    let decoded = match std::str::from_utf8(buf) {
        Ok(text) => Some(text.to_string()),
        // `error_len() == None` is std's "unexpected end of input", i.e. exactly
        // a truncated trailing sequence rather than an invalid byte.
        Err(e)
            if allow_dangling_tail
                && e.error_len().is_none()
                && buf.len() - e.valid_up_to() <= 3 =>
        {
            std::str::from_utf8(&buf[..e.valid_up_to()])
                .ok()
                .map(|t| t.to_string())
        }
        Err(_) => None, // guard 2
    };

    match decoded {
        Some(text) => {
            let original: String = buf.iter().map(|&b| b as char).collect();
            if text != original {
                *changed = true;
            }
            out.push_str(&text);
        }
        None => out.extend(buf.iter().map(|&b| b as char)),
    }
    buf.clear();
}

