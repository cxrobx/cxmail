//! RFC 2822 date normalization.
//!
//! Lives here rather than in `email::imap` because `db::schema`'s v-series
//! migrations re-normalize stored `Date` headers, and a migration reaching into
//! the email crate is half of what made `db` and `email` mutually dependent.
//!
//! `email::imap` re-exports `normalize_date_to_iso8601` and `DATE_SENTINEL`.

/// Sentinel stored when a `Date` header is present but unrecoverable. Sorts to
/// the *bottom* of date-DESC views so garbage never floats above real mail
/// (see bug-bash BUG-01). Matches the convention in `email::import`.
pub const DATE_SENTINEL: &str = "1970-01-01T00:00:00+00:00";

/// Convert RFC 2822 or other date formats to ISO 8601 (RFC 3339) for consistent SQLite sorting.
pub fn normalize_date_to_iso8601(date_str: &str) -> String {
    // Already ISO 8601? Return as-is.
    if date_str.len() > 4
        && date_str[..4].chars().all(|c| c.is_ascii_digit())
        && date_str.as_bytes().get(4) == Some(&b'-')
    {
        return date_str.to_string();
    }
    // Try RFC 2822 (e.g., "Sat, 05 Apr 2026 03:45:00 +0000")
    if let Ok(dt) = chrono::DateTime::<chrono::FixedOffset>::parse_from_rfc2822(date_str) {
        return dt.with_timezone(&chrono::Utc).to_rfc3339();
    }
    // Handle dates without day-of-week (e.g., "14 Oct 2025 23:02:24 -0000")
    let parts: Vec<&str> = date_str.trim().rsplitn(2, ' ').collect();
    if parts.len() == 2 {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(parts[1], "%d %b %Y %H:%M:%S") {
            let tz_secs = parse_tz_offset(parts[0]);
            if let Some(offset) = chrono::FixedOffset::east_opt(tz_secs) {
                if let Some(dt_with_tz) = dt.and_local_timezone(offset).single() {
                    return dt_with_tz.with_timezone(&chrono::Utc).to_rfc3339();
                }
            }
        }
    }
    // Salvage: some senders append junk after the timezone offset, e.g.
    // "Wed, 25 Mar 2026 10:05:36 +0000.1731-577592" or
    // "Thu, 02 Apr 2026 00:30:54 +0200 . 714661948". Recover the leading
    // RFC 2822 date by cutting at the first valid numeric offset.
    if let Some(salvaged) = salvage_rfc2822(date_str.trim()) {
        if let Ok(dt) = chrono::DateTime::<chrono::FixedOffset>::parse_from_rfc2822(&salvaged) {
            return dt.with_timezone(&chrono::Utc).to_rfc3339();
        }
    }
    // Unrecoverable (e.g. "_smtpDate . 714661948"): low sentinel rather than the
    // raw string, so it sorts to the bottom of date-ordered views, not the top.
    DATE_SENTINEL.to_string()
}

fn parse_tz_offset(s: &str) -> i32 {
    let s = s.trim();
    if s.eq_ignore_ascii_case("GMT")
        || s.eq_ignore_ascii_case("UTC")
        || s == "+0000"
        || s == "-0000"
    {
        return 0;
    }
    if s.len() == 5 && (s.starts_with('+') || s.starts_with('-')) {
        let sign = if s.starts_with('-') { -1 } else { 1 };
        if let (Ok(hours), Ok(minutes)) = (s[1..3].parse::<i32>(), s[3..5].parse::<i32>()) {
            return sign * (hours * 3600 + minutes * 60);
        }
    }
    0
}

/// Truncate a date string at the first valid numeric timezone offset
/// (`+HHMM` / `-HHMM`) that follows whitespace, returning the leading
/// candidate RFC 2822 date. Used to recover dates with sender-appended junk
/// after the offset. Returns `None` if no such offset is found.
fn salvage_rfc2822(date_str: &str) -> Option<String> {
    let bytes = date_str.as_bytes();
    for i in 1..bytes.len() {
        let c = bytes[i];
        if (c == b'+' || c == b'-')
            && bytes[i - 1] == b' '
            && bytes
                .get(i + 1..i + 5)
                .map_or(false, |o| o.iter().all(|b| b.is_ascii_digit()))
        {
            // i..i+5 is "+HHMM" / "-HHMM"; end index i+5 is in-bounds per the get() above.
            return Some(date_str[..i + 5].to_string());
        }
    }
    None
}
