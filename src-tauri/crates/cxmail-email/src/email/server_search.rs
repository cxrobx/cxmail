//! Server-side search query builders. Local sync only covers INBOX (recent)
//! + Drafts + browsed folders; these translate `SearchFilters` into
//! provider-native IMAP SEARCH queries so a search can fall back to the whole
//! server mailbox (Gmail: `X-GM-RAW` over `[Gmail]/All Mail` — literally
//! Gmail's own engine; iCloud/Outlook: standard RFC 3501 SEARCH keys).
//! Pure string builders — no network — so they unit-test directly.

use crate::db::search::SearchFilters;
use chrono::NaiveDate;

/// Escape a value for embedding inside an IMAP quoted string.
fn imap_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Take the date part of a bound that may carry a time suffix
/// ("2026-06-24T23:59:59" → 2026-06-24).
fn date_part(value: &str) -> Option<NaiveDate> {
    let d = value.split('T').next().unwrap_or(value);
    NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()
}

/// Gmail search syntax (used verbatim inside `X-GM-RAW "..."`).
/// Full-fidelity: Gmail's engine even searches attachment content.
pub fn build_gmail_raw_query(f: &SearchFilters) -> String {
    let mut parts: Vec<String> = Vec::new();
    let quote_if_spaced = |v: &str| {
        if v.contains(' ') {
            format!("\"{}\"", v.replace('"', ""))
        } else {
            v.to_string()
        }
    };
    if let Some(ref from) = f.from {
        parts.push(format!("from:{}", quote_if_spaced(from)));
    }
    if let Some(ref to) = f.to {
        parts.push(format!("to:{}", quote_if_spaced(to)));
    }
    if let Some(ref cc) = f.cc {
        parts.push(format!("cc:{}", quote_if_spaced(cc)));
    }
    if let Some(ref subject) = f.subject_contains {
        parts.push(format!("subject:{}", quote_if_spaced(subject)));
    }
    if let Some(ref filename) = f.filename {
        parts.push(format!("filename:{}", quote_if_spaced(filename)));
    }
    if let Some(d) = f.date_after.as_deref().and_then(date_part) {
        parts.push(format!("after:{}", d.format("%Y/%m/%d")));
    }
    if let Some(d) = f.date_before.as_deref().and_then(date_part) {
        // Gmail's before: is exclusive; our date_before bound is inclusive
        // end-of-day, so push the boundary one day out.
        let next = d + chrono::Duration::days(1);
        parts.push(format!("before:{}", next.format("%Y/%m/%d")));
    }
    if f.has_attachments == Some(true) {
        parts.push("has:attachment".to_string());
    }
    if f.is_starred == Some(true) {
        parts.push("is:starred".to_string());
    }
    if f.is_unread == Some(true) {
        parts.push("is:unread".to_string());
    }
    if let Some(larger) = f.larger_bytes {
        parts.push(format!("larger:{larger}"));
    }
    if let Some(smaller) = f.smaller_bytes {
        parts.push(format!("smaller:{smaller}"));
    }
    if let Some(ref kw) = f.keywords {
        let kw = kw.trim();
        if !kw.is_empty() {
            parts.push(kw.to_string());
        }
    }
    parts.join(" ")
}

/// The full IMAP command argument for Gmail: `X-GM-RAW "<query>"`.
pub fn build_gmail_uid_search(f: &SearchFilters) -> String {
    format!("X-GM-RAW {}", imap_quote(&build_gmail_raw_query(f)))
}

/// Standard RFC 3501 SEARCH keys (iCloud / Outlook). Keys are implicitly
/// AND-ed. `filename` has no standard key and is skipped (TEXT may still
/// catch it via body/attachment text on some servers).
pub fn build_imap_search_query(f: &SearchFilters) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ref from) = f.from {
        parts.push(format!("FROM {}", imap_quote(from)));
    }
    if let Some(ref to) = f.to {
        parts.push(format!("TO {}", imap_quote(to)));
    }
    if let Some(ref cc) = f.cc {
        parts.push(format!("CC {}", imap_quote(cc)));
    }
    if let Some(ref subject) = f.subject_contains {
        parts.push(format!("SUBJECT {}", imap_quote(subject)));
    }
    if let Some(d) = f.date_after.as_deref().and_then(date_part) {
        parts.push(format!("SINCE {}", d.format("%d-%b-%Y")));
    }
    if let Some(d) = f.date_before.as_deref().and_then(date_part) {
        // BEFORE is exclusive of the named day; our bound is inclusive.
        let next = d + chrono::Duration::days(1);
        parts.push(format!("BEFORE {}", next.format("%d-%b-%Y")));
    }
    if f.is_starred == Some(true) {
        parts.push("FLAGGED".to_string());
    }
    if f.is_unread == Some(true) {
        parts.push("UNSEEN".to_string());
    }
    if let Some(larger) = f.larger_bytes {
        parts.push(format!("LARGER {larger}"));
    }
    if let Some(smaller) = f.smaller_bytes {
        parts.push(format!("SMALLER {smaller}"));
    }
    if let Some(ref kw) = f.keywords {
        // One TEXT key per word/quoted phrase — implicit AND.
        let chars: Vec<char> = kw.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let term: String = if chars[i] == '"' {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                let t: String = chars[start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                }
                t
            } else {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[start..i].iter().collect()
            };
            let term = term.trim();
            // Skip negations — standard SEARCH NOT TEXT is possible but
            // fragile across servers; local re-ranking handles precision.
            if !term.is_empty() && !term.starts_with('-') {
                parts.push(format!("TEXT {}", imap_quote(term)));
            }
            let _ = &term;
        }
    }
    if parts.is_empty() {
        "ALL".to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters() -> SearchFilters {
        SearchFilters {
            keywords: Some("budget \"q3 numbers\"".to_string()),
            from: Some("sarah".to_string()),
            to: Some("bob@x.com".to_string()),
            subject_contains: Some("review".to_string()),
            date_after: Some("2026-06-01".to_string()),
            date_before: Some("2026-06-24T23:59:59".to_string()),
            has_attachments: Some(true),
            is_starred: Some(true),
            is_unread: Some(true),
            larger_bytes: Some(2_097_152),
            filename: Some("report.pdf".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn gmail_raw_query_translates_all_fields() {
        let q = build_gmail_raw_query(&filters());
        assert!(q.contains("from:sarah"));
        assert!(q.contains("to:bob@x.com"));
        assert!(q.contains("subject:review"));
        assert!(q.contains("filename:report.pdf"));
        assert!(q.contains("after:2026/06/01"));
        // Inclusive end-of-day 06-24 → exclusive Gmail before: 06-25.
        assert!(q.contains("before:2026/06/25"));
        assert!(q.contains("has:attachment"));
        assert!(q.contains("is:starred"));
        assert!(q.contains("is:unread"));
        assert!(q.contains("larger:2097152"));
        assert!(q.contains("budget"));
        assert!(q.contains("\"q3 numbers\""));
    }

    #[test]
    fn gmail_uid_search_wraps_and_escapes() {
        let f = SearchFilters {
            keywords: Some("say \"hi\"".to_string()),
            ..Default::default()
        };
        let cmd = build_gmail_uid_search(&f);
        assert!(cmd.starts_with("X-GM-RAW \""));
        assert!(cmd.contains("\\\"hi\\\""));
    }

    #[test]
    fn standard_imap_query_translates_fields() {
        let q = build_imap_search_query(&filters());
        assert!(q.contains("FROM \"sarah\""));
        assert!(q.contains("TO \"bob@x.com\""));
        assert!(q.contains("SUBJECT \"review\""));
        assert!(q.contains("SINCE 01-Jun-2026"));
        assert!(q.contains("BEFORE 25-Jun-2026"));
        assert!(q.contains("FLAGGED"));
        assert!(q.contains("UNSEEN"));
        assert!(q.contains("LARGER 2097152"));
        assert!(q.contains("TEXT \"budget\""));
        assert!(q.contains("TEXT \"q3 numbers\""));
    }

    #[test]
    fn empty_filters_yield_all() {
        assert_eq!(build_imap_search_query(&SearchFilters::default()), "ALL");
    }

    #[test]
    fn negated_keywords_are_skipped_in_standard_search() {
        let f = SearchFilters {
            keywords: Some("budget -promo".to_string()),
            ..Default::default()
        };
        let q = build_imap_search_query(&f);
        assert!(q.contains("TEXT \"budget\""));
        assert!(!q.contains("promo"));
    }
}
