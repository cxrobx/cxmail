//! Deterministic, zero-network parser for Gmail-style search operators and
//! relative dates in free text. Runs before (and independently of) the LLM
//! query parser — explicit operators mean the user already speaks the query
//! language, so the LLM is skipped entirely.
//!
//! Supported operators: `from:` `to:` `cc:` `in:` `has:attachment`
//! `is:starred|flagged|unread|read` `filename:` `larger:` `smaller:`
//! `before:` `after:`, plus `-negated` terms and `"quoted phrases"` (both
//! left in the keywords for `build_match_expr`). Relative dates ("yesterday",
//! "last week", "this week", "last month", "in march") are extracted from the
//! free text and turned into date bounds.

use crate::db::search::SearchFilters;
use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug)]
pub struct ParsedQuery {
    /// Filters with `keywords` set to the remaining free text (None if empty).
    pub filters: SearchFilters,
    /// True when at least one explicit `op:` operator was consumed. Relative
    /// dates do NOT set this — the LLM can still fill non-date gaps.
    pub found_operators: bool,
}

/// Parse `larger:2M` / `smaller:500K` style sizes into bytes.
/// Accepts bare bytes, K/KB, M/MB, G/GB (case-insensitive).
fn parse_size(value: &str) -> Option<i64> {
    let v = value.trim().to_ascii_uppercase();
    let (digits, multiplier) = if let Some(d) = v.strip_suffix("GB").or_else(|| v.strip_suffix('G').map(|s| s)) {
        (d, 1_073_741_824i64)
    } else if let Some(d) = v.strip_suffix("MB").or_else(|| v.strip_suffix('M').map(|s| s)) {
        (d, 1_048_576i64)
    } else if let Some(d) = v.strip_suffix("KB").or_else(|| v.strip_suffix('K').map(|s| s)) {
        (d, 1_024i64)
    } else {
        (v.as_str(), 1i64)
    };
    let n: i64 = digits.trim().parse().ok()?;
    Some(n.saturating_mul(multiplier))
}

/// Normalize `before:`/`after:` values: accepts YYYY-MM-DD or Gmail's
/// YYYY/MM/DD. Returns canonical YYYY-MM-DD or None if unparseable.
fn parse_date_value(value: &str) -> Option<String> {
    let v = value.trim().replace('/', "-");
    NaiveDate::parse_from_str(&v, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

fn month_number(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
}

fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)
}

fn fmt_after(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Inclusive end-of-day bound — see the `SearchFilters::date_before` doc:
/// a bare date normalizes to midnight (exclusive), so inclusive callers
/// append the end-of-day time.
fn fmt_before_inclusive(d: NaiveDate) -> String {
    format!("{}T23:59:59", d.format("%Y-%m-%d"))
}

/// Extract relative-date phrases from the token stream. Returns the date
/// bounds and the number of tokens consumed starting at `i` (0 = no match).
/// `today` is injected so tests can freeze time.
fn match_relative_date(
    words: &[String],
    i: usize,
    today: NaiveDate,
) -> (Option<String>, Option<String>, usize) {
    let w = |k: usize| words.get(i + k).map(|s| s.to_ascii_lowercase());
    let first = match w(0) {
        Some(f) => f,
        None => return (None, None, 0),
    };
    match first.as_str() {
        "today" => (
            Some(fmt_after(today)),
            Some(fmt_before_inclusive(today)),
            1,
        ),
        "yesterday" => {
            let d = today - Duration::days(1);
            (Some(fmt_after(d)), Some(fmt_before_inclusive(d)), 1)
        }
        "last" => match w(1).as_deref() {
            Some("week") => (
                Some(fmt_after(today - Duration::days(7))),
                Some(fmt_before_inclusive(today)),
                2,
            ),
            Some("month") => (
                Some(fmt_after(today - Duration::days(30))),
                Some(fmt_before_inclusive(today)),
                2,
            ),
            Some("year") => (
                Some(fmt_after(today - Duration::days(365))),
                Some(fmt_before_inclusive(today)),
                2,
            ),
            _ => (None, None, 0),
        },
        "this" => match w(1).as_deref() {
            Some("week") => {
                let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
                (
                    Some(fmt_after(monday)),
                    Some(fmt_before_inclusive(today)),
                    2,
                )
            }
            Some("month") => {
                let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
                (
                    Some(fmt_after(first)),
                    Some(fmt_before_inclusive(today)),
                    2,
                )
            }
            _ => (None, None, 0),
        },
        "in" => {
            // "in march" / "in march 2026" → that month's range; a month later
            // than the current one refers to the previous year.
            let Some(month_word) = w(1) else {
                return (None, None, 0);
            };
            let Some(month) = month_number(&month_word) else {
                return (None, None, 0);
            };
            let (year, consumed) = match w(2).and_then(|y| y.parse::<i32>().ok()) {
                Some(y) if (1990..=2100).contains(&y) => (y, 3),
                _ => {
                    let y = if month > today.month() {
                        today.year() - 1
                    } else {
                        today.year()
                    };
                    (y, 2)
                }
            };
            let start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
            (
                Some(fmt_after(start)),
                Some(fmt_before_inclusive(last_day_of_month(year, month))),
                consumed,
            )
        }
        _ => (None, None, 0),
    }
}

/// Parse a raw query. Colon-operators are consumed into filters; everything
/// else (including quoted phrases and `-negations`, which `build_match_expr`
/// understands) stays in the keywords.
pub fn parse(input: &str, today: NaiveDate) -> ParsedQuery {
    let mut filters = SearchFilters::default();
    let mut found_operators = false;
    let mut keywords: Vec<String> = Vec::new();

    // Tokenize, keeping quoted phrases (with their quotes) as single tokens
    // and honoring op:"quoted value".
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut raw_tokens: Vec<String> = Vec::new();
    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        let mut in_quotes = false;
        while i < n && (in_quotes || !chars[i].is_whitespace()) {
            if chars[i] == '"' {
                in_quotes = !in_quotes;
            }
            i += 1;
        }
        raw_tokens.push(chars[start..i].iter().collect());
    }

    // Pass 1: consume operators; collect the rest as candidate keywords.
    let mut words: Vec<String> = Vec::new();
    for tok in raw_tokens {
        let lower = tok.to_ascii_lowercase();
        let Some(colon) = tok.find(':') else {
            words.push(tok);
            continue;
        };
        let op = &lower[..colon];
        let value = tok[colon + 1..].trim_matches('"').to_string();
        if value.is_empty() {
            // "from:" with no value — treat as plain text (lenient).
            words.push(tok);
            continue;
        }
        match op {
            "from" => {
                filters.from = Some(value);
                found_operators = true;
            }
            "to" => {
                filters.to = Some(value);
                found_operators = true;
            }
            "cc" => {
                filters.cc = Some(value);
                found_operators = true;
            }
            "in" | "folder" => {
                filters.folder = Some(match value.to_ascii_lowercase().as_str() {
                    "sent" => "sent".to_string(),
                    "inbox" => "INBOX".to_string(),
                    "drafts" => "drafts".to_string(),
                    "trash" => "trash".to_string(),
                    "archive" => "archive".to_string(),
                    "spam" | "junk" => "spam".to_string(),
                    other => other.to_string(),
                });
                found_operators = true;
            }
            "has" => {
                if value.eq_ignore_ascii_case("attachment")
                    || value.eq_ignore_ascii_case("attachments")
                {
                    filters.has_attachments = Some(true);
                    found_operators = true;
                } else {
                    words.push(tok);
                }
            }
            "is" => match value.to_ascii_lowercase().as_str() {
                "starred" | "flagged" => {
                    filters.is_starred = Some(true);
                    found_operators = true;
                }
                "unread" => {
                    filters.is_unread = Some(true);
                    found_operators = true;
                }
                "read" => {
                    filters.is_unread = Some(false);
                    found_operators = true;
                }
                _ => words.push(tok),
            },
            "filename" | "attachment" => {
                filters.filename = Some(value);
                found_operators = true;
            }
            "larger" | "bigger" => {
                if let Some(bytes) = parse_size(&value) {
                    filters.larger_bytes = Some(bytes);
                    found_operators = true;
                } else {
                    words.push(tok);
                }
            }
            "smaller" => {
                if let Some(bytes) = parse_size(&value) {
                    filters.smaller_bytes = Some(bytes);
                    found_operators = true;
                } else {
                    words.push(tok);
                }
            }
            "after" | "since" => {
                if let Some(d) = parse_date_value(&value) {
                    filters.date_after = Some(d);
                    found_operators = true;
                } else {
                    words.push(tok);
                }
            }
            "before" | "until" => {
                if let Some(d) = parse_date_value(&value) {
                    // Inclusive per Gmail semantics ("before:" in Gmail is
                    // exclusive, but our chips read "before: <date>" as a
                    // range end) — end-of-day keeps the named day included.
                    filters.date_before = Some(format!("{d}T23:59:59"));
                    found_operators = true;
                } else {
                    words.push(tok);
                }
            }
            "subject" => {
                filters.subject_contains = Some(value);
                found_operators = true;
            }
            _ => words.push(tok),
        }
    }

    // Pass 2: relative dates in the remaining free text (only when no
    // explicit date operator already set the bounds).
    let mut i = 0;
    while i < words.len() {
        if filters.date_after.is_none() && filters.date_before.is_none() {
            let (after, before, consumed) = match_relative_date(&words, i, today);
            if consumed > 0 {
                filters.date_after = after;
                filters.date_before = before;
                words.drain(i..i + consumed);
                continue;
            }
        }
        keywords.push(words[i].clone());
        i += 1;
    }

    let joined = keywords.join(" ");
    filters.keywords = if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    };

    ParsedQuery {
        filters,
        found_operators,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        // Frozen: Wednesday 2026-07-01.
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
    }

    #[test]
    fn parses_from_to_cc_and_keeps_keywords() {
        let p = parse("from:sarah to:bob@x.com cc:carol budget report", today());
        assert!(p.found_operators);
        assert_eq!(p.filters.from.as_deref(), Some("sarah"));
        assert_eq!(p.filters.to.as_deref(), Some("bob@x.com"));
        assert_eq!(p.filters.cc.as_deref(), Some("carol"));
        assert_eq!(p.filters.keywords.as_deref(), Some("budget report"));
    }

    #[test]
    fn quoted_operator_values_and_phrases() {
        let p = parse("from:\"Sarah Lee\" \"quarterly numbers\"", today());
        assert_eq!(p.filters.from.as_deref(), Some("Sarah Lee"));
        // The phrase stays intact for build_match_expr.
        assert_eq!(p.filters.keywords.as_deref(), Some("\"quarterly numbers\""));
    }

    #[test]
    fn folder_flags_and_filename() {
        let p = parse("in:sent has:attachment is:starred filename:report.pdf", today());
        assert!(p.found_operators);
        assert_eq!(p.filters.folder.as_deref(), Some("sent"));
        assert_eq!(p.filters.has_attachments, Some(true));
        assert_eq!(p.filters.is_starred, Some(true));
        assert_eq!(p.filters.filename.as_deref(), Some("report.pdf"));
        assert!(p.filters.keywords.is_none());
    }

    #[test]
    fn is_unread_and_is_read() {
        assert_eq!(parse("is:unread", today()).filters.is_unread, Some(true));
        assert_eq!(parse("is:read", today()).filters.is_unread, Some(false));
    }

    #[test]
    fn size_suffixes() {
        let p = parse("larger:2M smaller:500K", today());
        assert_eq!(p.filters.larger_bytes, Some(2 * 1_048_576));
        assert_eq!(p.filters.smaller_bytes, Some(500 * 1_024));
        assert_eq!(parse("larger:1G", today()).filters.larger_bytes, Some(1_073_741_824));
        assert_eq!(parse("larger:512", today()).filters.larger_bytes, Some(512));
        // Garbage size value falls back to plain text.
        let bad = parse("larger:huge", today());
        assert!(bad.filters.larger_bytes.is_none());
        assert_eq!(bad.filters.keywords.as_deref(), Some("larger:huge"));
    }

    #[test]
    fn absolute_dates_iso_and_gmail_style() {
        let p = parse("after:2026-06-01 before:2026/06/24 invoice", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-06-01"));
        assert_eq!(p.filters.date_before.as_deref(), Some("2026-06-24T23:59:59"));
        assert_eq!(p.filters.keywords.as_deref(), Some("invoice"));
    }

    #[test]
    fn relative_dates_removed_from_keywords() {
        let p = parse("budget yesterday", today());
        assert!(!p.found_operators, "relative dates alone must not gate the LLM off");
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-06-30"));
        assert_eq!(p.filters.date_before.as_deref(), Some("2026-06-30T23:59:59"));
        assert_eq!(p.filters.keywords.as_deref(), Some("budget"));

        let p = parse("emails about the offsite last week", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-06-24"));
        assert_eq!(p.filters.keywords.as_deref(), Some("emails about the offsite"));
    }

    #[test]
    fn this_week_starts_monday() {
        // 2026-07-01 is a Wednesday → Monday is 2026-06-29.
        let p = parse("standup notes this week", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-06-29"));
    }

    #[test]
    fn in_month_resolves_most_recent_past() {
        // March < July → this year's March.
        let p = parse("taxes in march", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-03-01"));
        assert_eq!(p.filters.date_before.as_deref(), Some("2026-03-31T23:59:59"));
        // October > July → last year's October.
        let p = parse("conference in october", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2025-10-01"));
        // Explicit year wins.
        let p = parse("conference in october 2024", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2024-10-01"));
        assert_eq!(p.filters.date_before.as_deref(), Some("2024-10-31T23:59:59"));
        // "in" followed by a non-month is left alone.
        let p = parse("stuck in traffic", today());
        assert!(p.filters.date_after.is_none());
        assert_eq!(p.filters.keywords.as_deref(), Some("stuck in traffic"));
    }

    #[test]
    fn negation_and_unknown_operators_stay_in_keywords() {
        let p = parse("budget -promo weird:thing", today());
        assert!(!p.found_operators);
        assert_eq!(p.filters.keywords.as_deref(), Some("budget -promo weird:thing"));
    }

    #[test]
    fn bare_operator_with_no_value_is_plain_text() {
        let p = parse("from:", today());
        assert!(!p.found_operators);
        assert_eq!(p.filters.keywords.as_deref(), Some("from:"));
    }

    #[test]
    fn explicit_date_operator_suppresses_relative_date() {
        let p = parse("after:2026-01-01 budget yesterday", today());
        assert_eq!(p.filters.date_after.as_deref(), Some("2026-01-01"));
        // "yesterday" stays as a keyword since bounds were explicit.
        assert_eq!(p.filters.keywords.as_deref(), Some("budget yesterday"));
    }
}
