//! Calendar check for authored draft prose. Run before any IMAP write.

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use regex::Regex;
use std::sync::LazyLock;

static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:mon(?:day)?|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:rs(?:day)?)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)\b").unwrap()
});
static PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?P<weekday>mon(?:day)?|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:rs(?:day)?)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)",
        r"\b[\s,]+(?:",
        r"(?P<month>jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\.?\s+(?P<day>\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(?P<year>\d{4}))?",
        r"|(?P<nmonth>\d{1,2})/(?P<nday>\d{1,2})(?:/(?P<nyear>\d{4}))?",
        r")\b"
    )).unwrap()
});
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+").unwrap());

/// An error is a refusal; a successful result may carry an advisory for bare
/// weekdays. `today` is injected so year rollover has deterministic tests.
pub fn check_body(body: &str, today: NaiveDate) -> Result<Option<String>, String> {
    let visible = visible_authored_text(body);
    let visible = URL.replace_all(&visible, " ");
    let mut paired_weekdays = Vec::new();
    for caps in PAIR.captures_iter(&visible) {
        let mentioned = caps.name("weekday").unwrap();
        paired_weekdays.push(mentioned.start());
        let month = caps
            .name("nmonth")
            .or_else(|| caps.name("month"))
            .and_then(|m| month_number(m.as_str()))
            .ok_or_else(|| {
                "Draft has an unrecognized month next to a weekday; nothing was saved.".to_string()
            })?;
        let day = caps
            .name("nday")
            .or_else(|| caps.name("day"))
            .and_then(|d| d.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        let year = caps
            .name("nyear")
            .or_else(|| caps.name("year"))
            .and_then(|y| y.as_str().parse::<i32>().ok());
        let date = match year {
            Some(y) => NaiveDate::from_ymd_opt(y, month, day),
            None => (today.year()..=today.year() + 8)
                .filter_map(|y| NaiveDate::from_ymd_opt(y, month, day))
                .find(|d| *d >= today),
        }
        .ok_or_else(|| {
            format!(
                "Invalid date beside {}: nothing was saved.",
                mentioned.as_str()
            )
        })?;
        let actual = date.weekday();
        if weekday_number(mentioned.as_str()) != Some(actual) {
            return Err(format!(
                "{} is a {}, not {}. Nothing was saved.",
                date.format("%B %-d, %Y"),
                date.format("%A"),
                mentioned.as_str()
            ));
        }
    }

    if WEEKDAY
        .find_iter(&visible)
        .any(|m| !paired_weekdays.contains(&m.start()))
    {
        let calendar = (0..7)
            .map(|offset| {
                let date = today + Duration::days(offset);
                format!("{} {}", date.format("%A"), date.format("%Y-%m-%d"))
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Ok(Some(format!(
            "\n⚠ Check the bare weekday against this calendar before sending: {calendar}."
        )));
    }
    Ok(None)
}

pub fn check_body_today(body: &str) -> Result<Option<String>, String> {
    check_body(body, Local::now().date_naive())
}

fn weekday_number(s: &str) -> Option<Weekday> {
    match s.to_ascii_lowercase().as_str() {
        s if s.starts_with("mon") => Some(Weekday::Mon),
        s if s.starts_with("tue") => Some(Weekday::Tue),
        s if s.starts_with("wed") => Some(Weekday::Wed),
        s if s.starts_with("thu") => Some(Weekday::Thu),
        s if s.starts_with("fri") => Some(Weekday::Fri),
        s if s.starts_with("sat") => Some(Weekday::Sat),
        s if s.starts_with("sun") => Some(Weekday::Sun),
        _ => None,
    }
}

fn month_number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return (1..=12).contains(&n).then_some(n);
    }
    match s.to_ascii_lowercase().get(..3)? {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

/// Keep only text nodes from authored HTML. Tag attributes, style/script
/// contents, and any quoted blockquote are excluded. A plain-text reply's
/// `> ` history is excluded as well.
fn visible_authored_text(body: &str) -> String {
    let html = body
        .as_bytes()
        .windows(2)
        .any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/'));
    if !html {
        return body
            .lines()
            .filter(|line| !line.trim_start().starts_with('>'))
            .collect::<Vec<_>>()
            .join("\n");
    }
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut quote_depth = 0usize;
    let mut skip: Option<String> = None;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            if quote_depth == 0 && skip.is_none() {
                out.push(bytes[i]);
            }
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        let mut quoted = None;
        while end < bytes.len() {
            match bytes[end] {
                b'\'' | b'"' if quoted.is_none() => quoted = Some(bytes[end]),
                c if quoted == Some(c) => quoted = None,
                b'>' if quoted.is_none() => break,
                _ => {}
            }
            end += 1;
        }
        if end == bytes.len() {
            break;
        }
        let tag = body[start..end].trim();
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name == "blockquote" {
            if closing {
                quote_depth = quote_depth.saturating_sub(1);
            } else {
                quote_depth += 1;
            }
        } else if name == "script" || name == "style" {
            if closing && skip.as_deref() == Some(&name) {
                skip = None;
            } else if !closing && skip.is_none() {
                skip = Some(name);
            }
        }
        if quote_depth == 0 && skip.is_none() {
            out.push(b' ');
        }
        i = end + 1;
    }
    let text = String::from_utf8_lossy(&out);
    let decoded = html_escape::decode_html_entities(&text);
    decoded
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn luvrite_wrong_weekday_is_refused() {
        let err = check_body("I'll be out Thursday, September 25", day(2026, 9, 23)).unwrap_err();
        assert!(
            err.contains("September 25, 2026 is a Friday, not Thursday"),
            "{err}"
        );
    }

    #[test]
    fn correct_pairs_pass() {
        assert_eq!(
            check_body(
                "Out Friday, September 25; Fri Sep 25; Friday 9/25",
                day(2026, 9, 23)
            ),
            Ok(None)
        );
    }

    #[test]
    fn bare_weekday_warns_with_real_dates() {
        let note = check_body("Thursday is close; due Friday", day(2026, 9, 23))
            .unwrap()
            .unwrap();
        assert!(note.contains("Wednesday 2026-09-23"));
        assert!(note.contains("Tuesday 2026-09-29"));
    }

    #[test]
    fn quoted_history_and_nonvisible_html_are_skipped() {
        let html = r#"<a href="https://example.test/Thursday/9/25" title="Thursday 9/25">See details</a><style>.x:after{content:'Thursday 9/25'}</style><blockquote class="cx-quote">out Thursday, September 25</blockquote><p>Out Friday, September 25</p>"#;
        assert_eq!(check_body(html, day(2026, 9, 23)), Ok(None));
        assert_eq!(
            check_body(
                "Out Friday, September 25\n> out Thursday, September 25",
                day(2026, 9, 23)
            ),
            Ok(None)
        );
    }

    #[test]
    fn rich_html_checks_text_nodes_but_not_urls_or_attributes() {
        let rich = r#"<table><tr><td><a href="https://example.test/Thursday/9/25" title="Thursday 9/25">Out <strong>Thursday</strong>, September 25</a></td></tr></table>"#;
        let err = check_body(rich, day(2026, 9, 23)).unwrap_err();
        assert!(err.contains("September 25, 2026 is a Friday, not Thursday"));
        assert_eq!(
            check_body(
                "See https://example.test/Thursday/9/25 for details.",
                day(2026, 9, 23)
            ),
            Ok(None)
        );
    }

    #[test]
    fn explicit_year_and_december_rollover() {
        assert_eq!(check_body("Friday, January 2", day(2025, 12, 30)), Ok(None));
        let err = check_body("Thursday, January 2, 2026", day(2026, 12, 30)).unwrap_err();
        assert!(err.contains("January 2, 2026 is a Friday, not Thursday"));
    }
}
