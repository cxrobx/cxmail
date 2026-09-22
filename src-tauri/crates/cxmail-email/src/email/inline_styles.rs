use regex::Regex;
use std::sync::LazyLock;

/// Base font style applied to all block elements — matches Gmail's native compose defaults.
/// `text-size-adjust:100%` disables Chromium/WebKit's mobile font-boosting, which would
/// otherwise scale wrapped paragraphs while leaving short single-line blocks at the
/// declared size — producing visibly mismatched greeting/body/sign-off in Gmail mobile.
const BASE_STYLE: &str =
    "font-family:Arial,Helvetica,sans-serif;font-size:13px;color:#222222;-webkit-text-size-adjust:100%;text-size-adjust:100%";

/// Returns the inline style string for a given block-level tag.
fn style_for_tag(tag: &str) -> &'static str {
    match tag.to_lowercase().as_str() {
        "h1" => "font-family:Arial,Helvetica,sans-serif;font-size:x-large;color:#222222;font-weight:bold;margin:16px 0 8px;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "h2" => "font-family:Arial,Helvetica,sans-serif;font-size:large;color:#222222;font-weight:bold;margin:14px 0 6px;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "h3" => "font-family:Arial,Helvetica,sans-serif;font-size:medium;color:#222222;font-weight:bold;margin:12px 0 4px;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "h4" | "h5" | "h6" => "font-family:Arial,Helvetica,sans-serif;font-size:13px;color:#222222;font-weight:bold;margin:10px 0 4px;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "blockquote" => "font-family:Arial,Helvetica,sans-serif;font-size:13px;color:#222222;border-left:2px solid #ccc;padding-left:12px;margin:8px 0;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "ul" | "ol" => "font-family:Arial,Helvetica,sans-serif;font-size:13px;color:#222222;padding-left:24px;margin:4px 0;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        "pre" => "font-family:monospace;font-size:12px;color:#222222;background:#f5f5f5;padding:8px;border-radius:4px;-webkit-text-size-adjust:100%;text-size-adjust:100%",
        _ => BASE_STYLE, // p, div, li, etc.
    }
}

/// Regex matching block-level opening tags we want to style.
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<(p|h[1-6]|div|ul|ol|li|blockquote|pre)(\s[^>]*)?>").unwrap()
});

/// Regex for extracting an existing style attribute value.
static STYLE_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)style\s*=\s*"([^"]*)""#).unwrap()
});

/// Anchored regex matching a single CSS declaration (no surrounding whitespace) that
/// `apply_inline_font_styles` itself injects. Used to make the merge step idempotent:
/// when a re-run sees an already-styled block, we strip our own previously-injected
/// declarations from the existing style before prepending the new base. Without this,
/// CSS last-wins lets a legacy `font-size:small` override the new `font-size:13px`.
///
/// The patterns are intentionally narrow — they only match property+value pairs this
/// file injects. User-supplied declarations (e.g. `font-size:18px`, `color:#0a84ff`,
/// `font-weight:bold` on a signature `<p>`) do not match and pass through unchanged.
static OUR_DECL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?ix)
        ^ (?:
              font-family\s*:\s*(?:Arial,Helvetica,sans-serif|monospace)
            | font-size\s*:\s*(?:small|x-small|medium|large|x-large|13px|12px)
            | color\s*:\s*\#222222
            | -webkit-text-size-adjust\s*:\s*100%
            | text-size-adjust\s*:\s*100%
            | border-left\s*:\s*2px\s+solid\s+\#ccc
            | padding\s*:\s*8px
            | padding-left\s*:\s*(?:12|24)px
            | margin\s*:\s*(?:16px\s+0\s+8px|14px\s+0\s+6px|12px\s+0\s+4px|10px\s+0\s+4px|8px\s+0|4px\s+0)
            | background\s*:\s*\#f5f5f5
            | border-radius\s*:\s*4px
        ) $
    "#).unwrap()
});

/// Applies Gmail-style inline font styles to all block-level elements in outgoing HTML.
///
/// - Tags without a `style` attribute get one injected.
/// - Tags with an existing `style` attribute get base styles prepended (existing properties
///   override via CSS last-wins rule).
pub fn apply_inline_font_styles(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }

    TAG_RE
        .replace_all(html, |caps: &regex::Captures| {
            let full_match = caps.get(0).unwrap().as_str();
            let tag_name = caps.get(1).unwrap().as_str();
            let attrs = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let tag_style = style_for_tag(tag_name);

            if let Some(style_caps) = STYLE_ATTR_RE.captures(full_match) {
                // Existing style — strip any of OUR previously-injected declarations
                // (idempotency: a draft saved with old code carries `font-size:small`
                // baked into the style attr; we must remove it before prepending the
                // new base, or CSS last-wins keeps the bug). Anything not matching
                // OUR_DECL_RE is user content and passes through.
                let existing = style_caps.get(1).unwrap().as_str();
                let kept: Vec<&str> = existing
                    .split(';')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty() && !OUR_DECL_RE.is_match(s))
                    .collect();
                let merged = if kept.is_empty() {
                    tag_style.to_string()
                } else {
                    format!("{};{}", tag_style, kept.join(";"))
                };
                STYLE_ATTR_RE
                    .replace(full_match, format!(r#"style="{}""#, merged))
                    .to_string()
            } else {
                // No style attribute — inject one
                format!(
                    "<{} style=\"{}\"{}>{end}",
                    tag_name,
                    tag_style,
                    attrs,
                    end = if full_match.ends_with("/>") {
                        "/>"
                    } else {
                        ""
                    }
                )
            }
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert_eq!(apply_inline_font_styles(""), "");
    }

    /// Asserts that `decl` appears as an independent CSS declaration in `haystack`,
    /// not as a substring of a longer declaration. Anchored on `;`, whitespace, or
    /// quote so that e.g. `text-size-adjust:100%` is NOT satisfied by the longer
    /// `-webkit-text-size-adjust:100%`.
    fn has_decl(haystack: &str, decl: &str) -> bool {
        let pattern = format!(r#"(?:^|[;"\s]){}(?:[;"\s]|$)"#, regex::escape(decl));
        Regex::new(&pattern).unwrap().is_match(haystack)
    }

    #[test]
    fn test_plain_paragraph() {
        let input = "<p>Hello world</p>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("font-family:Arial,Helvetica,sans-serif"));
        assert!(has_decl(&result, "font-size:13px"));
        assert!(result.starts_with("<p style=\""));
    }

    #[test]
    fn test_text_size_adjust_both_present_on_paragraph() {
        let result = apply_inline_font_styles("<p>Hi</p>");
        assert!(has_decl(&result, "-webkit-text-size-adjust:100%"));
        assert!(has_decl(&result, "text-size-adjust:100%"));
    }

    #[test]
    fn test_text_size_adjust_present_on_all_branches() {
        for tag in ["h1", "h2", "h3", "h4", "blockquote", "ul", "ol", "li", "pre", "div"] {
            let html = format!("<{tag}>x</{tag}>");
            let result = apply_inline_font_styles(&html);
            assert!(has_decl(&result, "-webkit-text-size-adjust:100%"), "missing -webkit on <{tag}>: {result}");
            assert!(has_decl(&result, "text-size-adjust:100%"), "missing unprefixed on <{tag}>: {result}");
        }
    }

    #[test]
    fn test_idempotent_on_already_styled_legacy_html() {
        // Simulates a draft saved with the OLD code: pre-injected `font-size:small`.
        let legacy = r#"<p style="font-family:Arial,Helvetica,sans-serif;font-size:small;color:#222222">Hello</p>"#;
        let result = apply_inline_font_styles(legacy);
        assert!(has_decl(&result, "font-size:13px"), "13px missing: {result}");
        assert!(!result.contains("font-size:small"), "legacy small not stripped: {result}");
    }

    #[test]
    fn test_idempotent_on_double_application() {
        let once = apply_inline_font_styles("<p>Hi</p>");
        let twice = apply_inline_font_styles(&once);
        assert_eq!(once, twice, "f(f(x)) != f(x):\n  once:  {once}\n  twice: {twice}");
    }

    #[test]
    fn test_idempotent_on_double_application_pre() {
        let once = apply_inline_font_styles("<pre>code</pre>");
        let twice = apply_inline_font_styles(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn test_user_signature_font_size_preserved() {
        // Non-matching declarations (user content) must pass through unchanged.
        let sig = r#"<p style="font-size:18px;color:#0a84ff;font-weight:bold">Christopher Robinson</p>"#;
        let result = apply_inline_font_styles(sig);
        assert!(has_decl(&result, "font-size:18px"), "user 18px stripped: {result}");
        assert!(has_decl(&result, "color:#0a84ff"), "user color stripped: {result}");
        assert!(has_decl(&result, "font-weight:bold"), "user bold stripped: {result}");
    }

    #[test]
    fn test_heading_sizes() {
        let input = "<h1>Title</h1><h2>Subtitle</h2><h3>Section</h3>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("font-size:x-large"));
        assert!(result.contains("font-size:large"));
        assert!(result.contains("font-size:medium"));
    }

    #[test]
    fn test_existing_style_merged() {
        let input = r#"<div style="margin-top:16px;">Content</div>"#;
        let result = apply_inline_font_styles(input);
        // Base styles prepended, existing margin preserved at end
        assert!(result.contains("font-family:Arial,Helvetica,sans-serif"));
        assert!(result.contains("margin-top:16px"));
    }

    #[test]
    fn test_signature_div_preserved() {
        let input = r#"<div class="email-signature" style="margin-top:16px;">Sig</div>"#;
        let result = apply_inline_font_styles(input);
        assert!(result.contains("email-signature"));
        assert!(result.contains("margin-top:16px"));
        assert!(result.contains("font-family:Arial,Helvetica,sans-serif"));
    }

    #[test]
    fn test_list_elements() {
        let input = "<ul><li>Item 1</li><li>Item 2</li></ul>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("<ul style=\""));
        assert!(result.contains("<li style=\""));
        assert!(result.contains("padding-left:24px"));
    }

    #[test]
    fn test_blockquote() {
        let input = "<blockquote>Quoted text</blockquote>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("border-left:2px solid #ccc"));
    }

    #[test]
    fn test_pre_monospace() {
        let input = "<pre>code here</pre>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("font-family:monospace"));
        assert!(!result.contains("font-family:Arial"));
    }

    #[test]
    fn test_inline_tags_untouched() {
        let input = "<p>Hello <strong>bold</strong> and <em>italic</em></p>";
        let result = apply_inline_font_styles(input);
        // strong and em should not get style attributes
        assert!(!result.contains("<strong style="));
        assert!(!result.contains("<em style="));
    }

    #[test]
    fn test_tracking_pixel_untouched() {
        let input = r#"<img src="http://example.com/t/abc.png" width="1" height="1" style="display:none" alt="" />"#;
        let result = apply_inline_font_styles(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_no_block_elements() {
        let input = "Just plain text with <strong>bold</strong>";
        let result = apply_inline_font_styles(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_nested_elements() {
        let input = "<div><p>Nested paragraph</p></div>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("<div style=\""));
        assert!(result.contains("<p style=\""));
    }

    #[test]
    fn test_case_insensitive() {
        let input = "<P>Upper case tag</P>";
        let result = apply_inline_font_styles(input);
        assert!(result.contains("font-family:Arial,Helvetica,sans-serif"));
    }
}
