use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct DetectedEvent {
    pub summary: String,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub location: Option<String>,
    /// Free-form detail surfaced to the UI. For meeting invites this carries the
    /// provider join URL so `extractMeetingLink` can light up the "Join" button
    /// without rendering an ugly raw URL (which `location` would).
    pub description: Option<String>,
    pub confidence: f64,
}

/// Known domains that send event-like emails (flights, hotels, events).
fn is_event_sender_domain(domain: &str) -> bool {
    matches!(
        domain,
        // Airlines
        "united.com"
            | "aa.com"
            | "delta.com"
            | "southwest.com"
            | "jetblue.com"
            | "spirit.com"
            | "alaskaair.com"
            | "frontier.com"
            | "hawaiianairlines.com"
            | "britishairways.com"
            | "aircanada.com"
            | "ryanair.com"
            | "easyjet.com"
            | "lufthansa.com"
            // Hotels & Travel
            | "booking.com"
            | "airbnb.com"
            | "marriott.com"
            | "hilton.com"
            | "ihg.com"
            | "hyatt.com"
            | "expedia.com"
            | "hotels.com"
            | "vrbo.com"
            | "kayak.com"
            // Events & Tickets
            | "eventbrite.com"
            | "ticketmaster.com"
            | "meetup.com"
            | "lu.ma"
            | "zoom.us"
            | "calendly.com"
            | "doodle.com"
    )
}

/// Detect event-like information from email content using heuristic patterns.
/// No AI calls — pure keyword/pattern matching. Sub-millisecond per email.
///
/// **Panic-safe by construction.** This runs on the sync path
/// (`commands::messages::persist_parsed_metadata`, inside a DB transaction) and
/// inside two migrations, so a panic here does not merely lose one event: on sync
/// it poisons a tokio worker, which is how gotchas #9/#10 present — "Syncing…"
/// stuck forever — and in a migration it aborts app startup. Detection is a
/// best-effort nicety, so it must never be able to take anything else down.
///
/// The guard lives HERE rather than at each call site, deliberately. `parse_message`
/// takes the opposite approach and gotcha #9 has to say "all `parse_message` calls
/// need `catch_unwind`" — a rule someone must remember at every new call site. One
/// funnel cannot be forgotten. (This is also why `panic = "abort"` must never be
/// added to the release profile; `catch_unwind` needs unwinding.)
///
/// Not hypothetical: three char-boundary panics were found in here on 2026-08-10,
/// one reachable from any message containing a `──────────` separator. See #21b.
pub fn detect_events(
    subject: Option<&str>,
    plain_text: Option<&str>,
    from_email: &str,
    email_date: &str,
) -> Vec<DetectedEvent> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        detect_events_inner(subject, plain_text, from_email, email_date)
    })) {
        Ok(events) => events,
        Err(_) => {
            // Deliberately logs the sender and date, never the body.
            log::error!(
                "detect_events panicked for a message from {from_email} dated {email_date}; \
                 no events detected for it. This is a bug — see gotcha #21b."
            );
            Vec::new()
        }
    }
}

fn detect_events_inner(
    subject: Option<&str>,
    plain_text: Option<&str>,
    from_email: &str,
    email_date: &str,
) -> Vec<DetectedEvent> {
    let domain = from_email
        .to_lowercase()
        .split('@')
        .nth(1)
        .unwrap_or_default()
        .to_string();

    let is_known_sender = is_event_sender_domain(&domain);
    let base_confidence: f64 = if is_known_sender { 0.85 } else { 0.65 };

    let subject_lower = subject.unwrap_or("").to_lowercase();
    let text_lower = plain_text.unwrap_or("").to_lowercase();

    // Parse the email date as a reference point for relative date resolution
    let reference_date = parse_reference_date(email_date);

    let mut events = Vec::new();

    // Strategy 1: Flight confirmations
    if let Some(event) = detect_flight(&subject_lower, &text_lower, base_confidence, reference_date)
    {
        events.push(event);
    }

    // Strategy 2: Hotel/accommodation bookings
    if let Some(event) = detect_hotel(&subject_lower, &text_lower, base_confidence, reference_date)
    {
        events.push(event);
    }

    // Strategy 3: Event/ticket confirmations
    if let Some(event) =
        detect_event_ticket(&subject_lower, &text_lower, base_confidence, reference_date)
    {
        events.push(event);
    }

    // Strategy 4: Meeting/interview mentions in subject
    if let Some(event) =
        detect_meeting_in_subject(&subject_lower, subject, base_confidence, reference_date)
    {
        events.push(event);
    }

    // Strategy 5: Body-based meeting invitations (Zoom/Meet/Teams/Webex/Jitsi
    // pasted into the body — no `text/calendar` part). Pass ORIGINAL-CASE text:
    // join-URL matching is case-sensitive and the fn lowercases internally where
    // it needs to.
    if let Some(event) = detect_meeting_invite(
        subject,
        plain_text.unwrap_or(""),
        base_confidence,
        reference_date,
    ) {
        events.push(event);
    }

    events
}

fn parse_reference_date(email_date: &str) -> NaiveDate {
    // Try ISO 8601 first
    if let Ok(dt) = NaiveDateTime::parse_from_str(email_date, "%Y-%m-%dT%H:%M:%S%.fZ") {
        return dt.date();
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(email_date, "%Y-%m-%dT%H:%M:%S%.f%:z") {
        return dt.date();
    }
    // `get`, not a length-guarded slice — see `parse_natural_date` for the
    // panic this shape caused. This one runs for every message that syncs.
    if let Some(prefix) = email_date.get(..10) {
        if let Ok(d) = NaiveDate::parse_from_str(prefix, "%Y-%m-%d") {
            return d;
        }
    }
    Utc::now().date_naive()
}

/// Detect flight confirmations
fn detect_flight(
    subject: &str,
    text: &str,
    base_confidence: f64,
    reference_date: NaiveDate,
) -> Option<DetectedEvent> {
    let is_flight_subject = subject.contains("flight")
        || subject.contains("itinerary")
        || subject.contains("booking confirmation")
        || subject.contains("e-ticket")
        || subject.contains("trip confirmation")
        || subject.contains("your trip");

    if !is_flight_subject {
        return None;
    }

    // Look for departure date in body
    let date = find_labeled_date(text, &["departure", "depart", "outbound", "flight date"], reference_date)?;

    Some(DetectedEvent {
        summary: extract_flight_summary(subject, text),
        dtstart: date.format("%Y-%m-%d").to_string(),
        dtend: None,
        location: extract_destination(text),
        description: None,
        confidence: base_confidence.min(0.95),
    })
}

/// Detect hotel/accommodation bookings
fn detect_hotel(
    subject: &str,
    text: &str,
    base_confidence: f64,
    reference_date: NaiveDate,
) -> Option<DetectedEvent> {
    let is_hotel_subject = subject.contains("reservation")
        || subject.contains("hotel")
        || subject.contains("check-in")
        || subject.contains("booking confirmed")
        || subject.contains("your stay");

    if !is_hotel_subject {
        return None;
    }

    let checkin =
        find_labeled_date(text, &["check-in", "check in", "arrival", "checking in"], reference_date)?;
    let checkout = find_labeled_date(text, &["check-out", "check out", "departure", "checking out"], reference_date);

    Some(DetectedEvent {
        summary: capitalize_first(subject),
        dtstart: checkin.format("%Y-%m-%d").to_string(),
        dtend: checkout.map(|d| d.format("%Y-%m-%d").to_string()),
        location: extract_hotel_name(text),
        description: None,
        confidence: base_confidence.min(0.90),
    })
}

/// Detect event/ticket confirmations
fn detect_event_ticket(
    subject: &str,
    text: &str,
    base_confidence: f64,
    reference_date: NaiveDate,
) -> Option<DetectedEvent> {
    let is_event_subject = subject.contains("registration confirmed")
        || subject.contains("your ticket")
        || subject.contains("event confirmation")
        || subject.contains("you're registered")
        || subject.contains("rsvp confirmed")
        || subject.contains("your registration");

    if !is_event_subject {
        return None;
    }

    let date = find_labeled_date(text, &["date", "when", "event date", "starts"], reference_date)?;

    Some(DetectedEvent {
        summary: capitalize_first(subject),
        dtstart: date.format("%Y-%m-%d").to_string(),
        dtend: None,
        location: extract_venue(text),
        description: None,
        confidence: base_confidence.min(0.90),
    })
}

/// Detect meetings/interviews mentioned directly in the subject line
fn detect_meeting_in_subject(
    subject_lower: &str,
    subject_original: Option<&str>,
    base_confidence: f64,
    reference_date: NaiveDate,
) -> Option<DetectedEvent> {
    // Look for patterns like "Interview on April 5th" or "Meeting on March 10"
    let triggers = ["interview on ", "meeting on ", "call on ", "appointment on "];

    for trigger in &triggers {
        if let Some(pos) = subject_lower.find(trigger) {
            let after = &subject_lower[pos + trigger.len()..];
            if let Some(date) = parse_natural_date(after, reference_date) {
                return Some(DetectedEvent {
                    summary: capitalize_first(
                        subject_original.unwrap_or(subject_lower),
                    ),
                    dtstart: date.format("%Y-%m-%d").to_string(),
                    dtend: None,
                    location: None,
                    description: None,
                    confidence: (base_confidence - 0.05).max(0.60),
                });
            }
        }
    }
    None
}

/// Strategy 5: detect a meeting invitation pasted into the email *body* (no
/// `text/calendar` part). Common for forwarded Zoom/Meet/Teams invites and
/// "Sent from my iPhone" relays.
///
/// `plain_text` MUST be original-case — join-URL matching is case-sensitive.
///
/// Two-signal gate to suppress promos: a provider join URL is required, AND
/// at least one of {a signature phrase, a parseable labeled datetime}. If the
/// body looks like a webinar/training promo, the phrase signal is mandatory.
fn detect_meeting_invite(
    subject: Option<&str>,
    plain_text: &str,
    _base_confidence: f64,
    reference_date: NaiveDate,
) -> Option<DetectedEvent> {
    // Join URLs / signatures live near the top; cap the scan to keep quoted
    // reply chains from dominating.
    let scan_region = head(plain_text, 4000);

    // Signal 1 (required): a real provider join URL.
    let join_url = find_join_url(scan_region)?;

    let lower = scan_region.to_lowercase();

    // Signal 2a: an invite signature phrase.
    const PHRASES: &[&str] = &[
        "is inviting you",
        "scheduled zoom meeting",
        "microsoft teams meeting",
        "video call link",
        "join meeting",
    ];
    let has_phrase = PHRASES.iter().any(|p| lower.contains(p));

    // Signal 2b: a parseable labeled date/time. Scan only the leading block so
    // a quoted invite buried in a reply thread doesn't get mis-dated.
    let dt_region = head(plain_text, 1000);
    // Labeled wins, so nothing that worked before changes. The unlabeled scan is
    // strictly a fallback for the shape `find_labeled_datetime` structurally
    // cannot see — a bare date line with the time beneath it.
    let (dtstart, dtend) = match find_labeled_datetime(
        dt_region,
        &["time", "when", "date", "starts", "start"],
        reference_date,
    ) {
        Some(labeled) => (Some(labeled), None),
        None => match find_unlabeled_datetime(dt_region, reference_date) {
            Some((start, end)) => (Some(start), end),
            None => (None, None),
        },
    };
    let has_datetime = dtstart.is_some();

    // Promo guard: "join our webinar / training / workshop / course" blasts
    // often carry a real Zoom link. Require the explicit phrase signal for
    // those, and cap confidence so they stay low-trust.
    let is_promo = ["webinar", "training", "workshop", "course"]
        .iter()
        .any(|w| lower.contains(w));

    if is_promo {
        if !has_phrase {
            return None;
        }
    } else if !has_phrase && !has_datetime {
        return None;
    }

    // Title: Topic: → Event: → email subject.
    let summary = extract_labeled_line_value(scan_region, &["topic:", "event:"], 200)
        .or_else(|| subject.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Meeting".to_string());

    // Confidence tiers: URL=0.7, +datetime=0.85, +phrase+datetime=0.95.
    let mut confidence: f64 = 0.7;
    if has_datetime {
        confidence = 0.85;
        if has_phrase {
            confidence = 0.95;
        }
    }
    if is_promo {
        confidence = confidence.min(0.7);
    }

    // dtstart: parsed datetime if any, else fall back to the day the email
    // arrived as an all-day event (date-only, existing all-day behavior).
    let dtstart =
        dtstart.unwrap_or_else(|| reference_date.format("%Y-%m-%d").to_string());

    Some(DetectedEvent {
        summary: capitalize_first(&summary),
        dtstart,
        // Only ever set when the source line carried an explicit end time, so an
        // event with no stated duration keeps its previous shape rather than
        // acquiring a guessed one.
        dtend,
        location: None,
        description: Some(join_url),
        confidence,
    })
}

/// Provider join-URL patterns, mirrored from `src/lib/meetingLink.ts` (each is
/// host-anchored so a bare "zoom" mention won't match). Scanned in priority
/// order; the first match wins. Compiled once.
fn join_url_regexes() -> &'static [Regex] {
    static REGEXES: OnceLock<Vec<Regex>> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            // zoom.us/j/<id>, /my/<name>, /w/<id> (+ vanity subdomains)
            r#"(?i)https?://(?:[\w-]+\.)?zoom\.us/(?:j|my|w)/[^\s"'<>)]+"#,
            r#"(?i)https?://meet\.google\.com/[^\s"'<>)]+"#,
            r#"(?i)https?://teams\.microsoft\.com/l/meetup-join/[^\s"'<>)]+|https?://teams\.live\.com/[^\s"'<>)]+"#,
            r#"(?i)https?://(?:[\w-]+\.)?webex\.com/[^\s"'<>)]+"#,
            r#"(?i)https?://meet\.jit\.si/[^\s"'<>)]+"#,
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid join-URL regex"))
        .collect()
    })
}

/// Return the first recognized provider join URL in `text`, in provider
/// priority order.
fn find_join_url(text: &str) -> Option<String> {
    for re in join_url_regexes() {
        if let Some(m) = re.find(text) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Named-timezone aliases → IANA zone, longest-first so a longer alias matches
/// before its prefix. Matched on word boundaries (see `contains_word`) so the
/// two-letter codes don't fire inside ordinary words.
const TZ_ALIASES: &[(&str, Tz)] = &[
    ("central time (us and canada)", Tz::America__Chicago),
    ("eastern time (us and canada)", Tz::America__New_York),
    ("pacific time (us and canada)", Tz::America__Los_Angeles),
    ("mountain time (us and canada)", Tz::America__Denver),
    ("central time", Tz::America__Chicago),
    ("eastern time", Tz::America__New_York),
    ("pacific time", Tz::America__Los_Angeles),
    ("mountain time", Tz::America__Denver),
    ("cdt", Tz::America__Chicago),
    ("cst", Tz::America__Chicago),
    ("edt", Tz::America__New_York),
    ("est", Tz::America__New_York),
    ("pdt", Tz::America__Los_Angeles),
    ("pst", Tz::America__Los_Angeles),
    ("mdt", Tz::America__Denver),
    ("mst", Tz::America__Denver),
    ("utc", Tz::UTC),
    ("gmt", Tz::UTC),
    ("ct", Tz::America__Chicago),
    ("et", Tz::America__New_York),
    ("pt", Tz::America__Los_Angeles),
    ("mt", Tz::America__Denver),
];

/// Match a known timezone alias inside `s` (which the caller may pass in any
/// case — we lowercase here). Returns the IANA zone for the first (longest)
/// alias that appears on a word boundary.
fn match_tz(s: &str) -> Option<Tz> {
    let lower = s.to_lowercase();
    for (alias, tz) in TZ_ALIASES {
        if contains_word(&lower, alias) {
            return Some(*tz);
        }
    }
    None
}

/// `needle` (ASCII, lowercase) appears in `haystack` bounded by non-alphanumeric
/// chars (or string ends) on both sides. Prevents "ct"/"et" from matching inside
/// words like "contact" or "meeting".
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        let abs = start + rel;
        let before_ok = abs == 0
            || !haystack[..abs]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
        let after = abs + needle.len();
        let after_ok = after >= haystack.len()
            || !haystack[after..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
        if before_ok && after_ok {
            return true;
        }
        // Advance one byte; all aliases start with ASCII so this stays on a
        // char boundary.
        start = abs + 1;
    }
    false
}

/// Parse a date + optional clock time + optional timezone into a `dtstart`
/// string, per the dtstart contract:
///   - recognized tz   → convert to UTC, `"%Y-%m-%dT%H:%M:%SZ"`
///   - tz unknown/absent (but a time is present) → floating `"%Y-%m-%dT%H:%M:%S"`
///   - no parseable time → `None` (caller keeps a date-only all-day event)
fn parse_datetime_tz(s: &str, reference_date: NaiveDate) -> Option<String> {
    let date = parse_natural_date(s, reference_date)?;
    let (hour, minute) = parse_clock_time(s)?; // no time → None
    let naive = date.and_hms_opt(hour, minute, 0)?;

    match match_tz(s) {
        Some(tz) => {
            // Resolve the wall-clock time in its zone, then convert to UTC.
            // `.single()` is the common case; fall back to `.earliest()` for a
            // DST-ambiguous hour so we still emit something sensible.
            let zoned = tz
                .from_local_datetime(&naive)
                .single()
                .or_else(|| tz.from_local_datetime(&naive).earliest())?;
            let utc = zoned.with_timezone(&Utc);
            Some(utc.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        }
        None => Some(naive.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}

/// Parse the first `HH:MM` (+ optional AM/PM) clock time in `s`. 24h when no
/// meridiem is present. Returns (hour 0-23, minute 0-59).
fn parse_clock_time(s: &str) -> Option<(u32, u32)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b(\d{1,2}):([0-5]\d)\s*([ap]\.?m\.?)?").expect("valid clock regex")
    });
    let caps = re.captures(s)?;
    let mut hour: u32 = caps.get(1)?.as_str().parse().ok()?;
    let minute: u32 = caps.get(2)?.as_str().parse().ok()?;

    match caps.get(3).map(|m| m.as_str().to_ascii_lowercase()) {
        Some(ref m) if m.starts_with('p') => {
            if hour != 12 {
                hour += 12;
            }
        }
        Some(ref m) if m.starts_with('a') => {
            if hour == 12 {
                hour = 0;
            }
        }
        _ => {} // 24h, no meridiem
    }

    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Find a labeled date/time on its own line (`label:` or `label `), returning a
/// `dtstart` string. Prefers a full timed/zoned parse; falls back to date-only.
fn find_labeled_datetime(
    text: &str,
    labels: &[&str],
    reference_date: NaiveDate,
) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        for &label in labels {
            for sep in [":", " "] {
                let prefix = format!("{label}{sep}");
                if !lower.starts_with(&prefix) {
                    continue;
                }
                // `label` + `sep` are ASCII, matched as a prefix, so byte index
                // `prefix.len()` is a valid char boundary in the original line.
                let value = trimmed[prefix.len()..].trim();
                if let Some(dt) = parse_datetime_tz(value, reference_date) {
                    return Some(dt);
                }
                if let Some(d) = parse_natural_date(value, reference_date) {
                    return Some(d.format("%Y-%m-%d").to_string());
                }
            }
        }
    }
    None
}

/// Every clock time on a line, in order — so a `10:30am - 10:55am` range yields
/// both ends rather than only the start.
fn parse_clock_times(s: &str) -> Vec<(u32, u32)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b(\d{1,2}):([0-5]\d)\s*([ap]\.?m\.?)?").expect("valid clock regex")
    });
    // A bare "1:00" after a meridiem-bearing time inherits nothing, so each match
    // is resolved on its own — the same rule `parse_clock_time` applies.
    re.captures_iter(s)
        .filter_map(|caps| {
            let mut hour: u32 = caps.get(1)?.as_str().parse().ok()?;
            let minute: u32 = caps.get(2)?.as_str().parse().ok()?;
            match caps.get(3).map(|m| m.as_str().to_ascii_lowercase()) {
                Some(ref m) if m.starts_with('p') => {
                    if hour != 12 {
                        hour += 12;
                    }
                }
                Some(ref m) if m.starts_with('a') => {
                    if hour == 12 {
                        hour = 0;
                    }
                }
                _ => {}
            }
            (hour <= 23 && minute <= 59).then_some((hour, minute))
        })
        .collect()
}

/// Render a wall-clock date/time per the dtstart contract in gotcha #21: a
/// recognized zone becomes true UTC with a trailing `Z`; an unknown or absent
/// zone stays floating with no `Z`. `tz_source` is the text the zone is read
/// from — deliberately a parameter, because it must be the line the TIME came
/// from and never the whole message. The Anthropic emails carry `⏱️ PST` in the
/// signature, so a document-wide zone scan would silently retime a `12:30pm ET`
/// meeting by three hours.
fn render_dtstart(
    date: NaiveDate,
    hour: u32,
    minute: u32,
    tz_source: &str,
) -> Option<String> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    match match_tz(tz_source) {
        Some(tz) => {
            let zoned = tz
                .from_local_datetime(&naive)
                .single()
                .or_else(|| tz.from_local_datetime(&naive).earliest())?;
            Some(
                zoned
                    .with_timezone(&Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            )
        }
        None => Some(naive.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}

/// Find a date and time that carry no `label:` at all, including the very common
/// shape where the date is its own line and the time sits on the line beneath it:
///
/// ```text
/// *Thursday, August 13, 2026:*
/// 12:30pm - 1:00pm ET - Hiring Manager Screen with Jonathan Cham
/// ```
///
/// `find_labeled_datetime` cannot see this — it only inspects lines that *start
/// with* one of `time`/`when`/`date`/`starts`/`start` — so such an email fell all
/// the way through to the all-day fallback and was filed on the day the message
/// arrived. That is the bug this exists to fix.
///
/// Kept narrow on purpose. A candidate date line must parse as a date from its
/// first token (after decoration and a weekday are stripped), which means prose
/// like "we met in August 2025 about…" cannot match, since `words[0]` would be
/// `we`. Returns `(dtstart, dtend)`; `dtend` is present only when the time line
/// carries a second, later time.
fn find_unlabeled_datetime(
    text: &str,
    reference_date: NaiveDate,
) -> Option<(String, Option<String>)> {
    let lines: Vec<&str> = text.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let Some(date) = parse_natural_date(line, reference_date) else {
            continue;
        };

        // Prefer a time on the date's own line; otherwise look at the next
        // non-empty line. Only those two — reaching further would start pairing a
        // date with a time from an unrelated paragraph.
        let mut candidates: Vec<&str> = vec![line];
        if let Some(next) = lines[i + 1..].iter().find(|l| !l.trim().is_empty()) {
            candidates.push(next);
        }

        for source in candidates {
            let times = parse_clock_times(source);
            let Some(&(hour, minute)) = times.first() else {
                continue;
            };
            // The zone is read from the line the time came from, falling back to
            // the date line — never from the whole message.
            let dtstart = render_dtstart(date, hour, minute, source)
                .or_else(|| render_dtstart(date, hour, minute, line))?;
            let dtend = times
                .get(1)
                .filter(|(h, m)| (*h, *m) > (hour, minute))
                .and_then(|&(h, m)| render_dtstart(date, h, m, source));
            return Some((dtstart, dtend));
        }
    }
    None
}

/// Extract the value after a line-leading label (e.g. `Topic: ...`), preserving
/// original case. Returns the first non-empty match, capped at `max` chars.
fn extract_labeled_line_value(text: &str, labels: &[&str], max: usize) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        for &label in labels {
            if lower.starts_with(label) {
                // `label` is ASCII → byte index `label.len()` is a char boundary.
                let value = trimmed[label.len()..].trim();
                if !value.is_empty() {
                    let truncated: String = value.chars().take(max).collect();
                    return Some(truncated.trim().to_string());
                }
            }
        }
    }
    None
}

/// Byte-boundary-safe prefix of `s` up to `max_bytes`.
fn head(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── Date parsing helpers ──────────────────────────────────────────────

/// Find a date that follows a labeled field in the email body.
/// e.g., "Departure: April 5, 2026" or "Check-in: 2026-04-05"
fn find_labeled_date(text: &str, labels: &[&str], reference_date: NaiveDate) -> Option<NaiveDate> {
    for label in labels {
        // Look for "label:" or "label " patterns
        for separator in &[":", " "] {
            let pattern = format!("{}{}", label, separator);
            if let Some(pos) = text.find(&pattern) {
                let after = &text[pos + pattern.len()..];
                let after = after.trim_start();
                // Take the rest of the line
                let line_end = after.find('\n').unwrap_or(after.len());
                // `line_end` is a real boundary (from `find`), but `.min(60)`
                // was not — a line over 60 bytes could be cut mid-character.
                // `head` walks back to a boundary.
                let date_str = head(&after[..line_end], 60);
                if let Some(date) = parse_natural_date(date_str, reference_date) {
                    return Some(date);
                }
            }
        }
    }
    None
}

/// Parse various natural date formats.
/// Weekday names, which carry no date information but sit in front of the month
/// and so block month-first parsing entirely. Boundary-checked at the call site,
/// so the short forms cannot eat the start of a longer name or a real word.
const WEEKDAYS: &[&str] = &[
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "mon", "tue",
    "tues", "wed", "weds", "thu", "thur", "thurs", "fri", "sat", "sun",
];

/// Strip what an HTML→plain-text conversion leaves wrapped around a date line,
/// plus a leading weekday and a trailing colon.
///
/// Real shape, from two Anthropic recruiting emails:
/// `*Thursday, August 13, 2026:*` — bold markers survive the conversion, and
/// `try_parse_month_day_year` needs `words[0]` to BE the month name, so both the
/// `*` and the `Thursday,` have to go or the date is simply unparseable. That was
/// the first of two reasons the schedule in those emails was missed.
fn strip_date_decoration(s: &str) -> &str {
    let s = s
        .trim()
        .trim_matches(|c: char| matches!(c, '*' | '_' | '#' | '`' | '|' | '~'))
        .trim();
    let s = s.trim_end_matches(':').trim();

    let lower = s.to_ascii_lowercase();
    for weekday in WEEKDAYS {
        let Some(rest) = lower.strip_prefix(weekday) else {
            continue;
        };
        // Require a separator, so "sun" cannot eat "sunday" and "mar" cannot eat
        // a word that merely starts the same way.
        if rest.starts_with(',') || rest.starts_with(' ') {
            // The prefix is ASCII, so this byte index is a char boundary.
            return s[weekday.len()..]
                .trim_start_matches([',', ' '])
                .trim();
        }
    }
    s
}

fn parse_natural_date(s: &str, reference_date: NaiveDate) -> Option<NaiveDate> {
    let s = strip_date_decoration(s);
    // Trim common suffixes
    let s = s
        .trim_end_matches(|c: char| c == '.' || c == ',' || c == ' ')
        .trim();

    // ISO format: 2026-04-05.
    //
    // `s.get(..10)`, never `&s[..10]`. A length check is NOT a boundary check:
    // this panicked on a real message whose body contained a `──────────`
    // separator, because each box-drawing char is 3 bytes and byte 10 lands
    // mid-character. Latent until `find_unlabeled_datetime` began feeding this
    // function every line of the scan region rather than only label-stripped
    // values, and a panic here aborts detection for that message (gotcha #9's
    // shape). Found by running the migration against a copy of a real mailbox.
    if let Some(prefix) = s.get(..10) {
        if let Ok(d) = NaiveDate::parse_from_str(prefix, "%Y-%m-%d") {
            return Some(d);
        }
    }

    // US format: 04/05/2026
    if let Some(d) = try_parse_slash_date(s) {
        return Some(d);
    }

    // "Month Day, Year" — "April 5, 2026" or "April 5th, 2026"
    if let Some(d) = try_parse_month_day_year(s, reference_date) {
        return Some(d);
    }

    // "Day Month Year" — "5 April 2026"
    if let Some(d) = try_parse_day_month_year(s, reference_date) {
        return Some(d);
    }

    None
}

fn try_parse_slash_date(s: &str) -> Option<NaiveDate> {
    // Match MM/DD/YYYY at the start
    let parts: Vec<&str> = s.splitn(4, '/').collect();
    if parts.len() >= 3 {
        let month: u32 = parts[0].trim().parse().ok()?;
        let day: u32 = parts[1].trim().parse().ok()?;
        // Year might have trailing text
        let year_str = parts[2].split_whitespace().next()?;
        let year: i32 = year_str.trim().parse().ok()?;
        if (1..=12).contains(&month) && (1..=31).contains(&day) && year > 2000 {
            return NaiveDate::from_ymd_opt(year, month, day);
        }
    }
    None
}

fn try_parse_month_day_year(s: &str, reference_date: NaiveDate) -> Option<NaiveDate> {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }

    let month = parse_month_name(words[0])?;
    let day = parse_day_number(words[1])?;

    let year = if words.len() >= 3 {
        parse_year(words[2]).unwrap_or(reference_date.year())
    } else {
        reference_date.year()
    };

    NaiveDate::from_ymd_opt(year, month, day)
}

fn try_parse_day_month_year(s: &str, reference_date: NaiveDate) -> Option<NaiveDate> {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }

    let day = parse_day_number(words[0])?;
    let month = parse_month_name(words[1])?;

    let year = if words.len() >= 3 {
        parse_year(words[2]).unwrap_or(reference_date.year())
    } else {
        reference_date.year()
    };

    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_month_name(s: &str) -> Option<u32> {
    match s
        .trim_end_matches(|c: char| !c.is_alphabetic())
        .to_lowercase()
        .as_str()
    {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_day_number(s: &str) -> Option<u32> {
    // Strip ordinal suffixes: "5th" -> "5", "1st" -> "1"
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let day: u32 = digits.parse().ok()?;
    if (1..=31).contains(&day) {
        Some(day)
    } else {
        None
    }
}

fn parse_year(s: &str) -> Option<i32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let year: i32 = digits.parse().ok()?;
    if year > 2000 && year < 2100 {
        Some(year)
    } else {
        None
    }
}

// ── Content extraction helpers ────────────────────────────────────────

fn extract_flight_summary(subject: &str, _text: &str) -> String {
    // Use subject as summary, capitalizing first letter
    capitalize_first(subject)
}

fn extract_destination(text: &str) -> Option<String> {
    // Look for "to [City]" or "destination: [City]" patterns
    for label in &["destination:", "arriving:", "to:"] {
        if let Some(pos) = text.find(label) {
            let after = text[pos + label.len()..].trim_start();
            let end = after.find('\n').unwrap_or(after.len()).min(50);
            let value = after[..end].trim();
            if !value.is_empty() {
                return Some(capitalize_first(value));
            }
        }
    }
    None
}

fn extract_hotel_name(text: &str) -> Option<String> {
    for label in &["hotel:", "property:", "accommodation:"] {
        if let Some(pos) = text.find(label) {
            let after = text[pos + label.len()..].trim_start();
            let end = after.find('\n').unwrap_or(after.len()).min(80);
            let value = after[..end].trim();
            if !value.is_empty() {
                return Some(capitalize_first(value));
            }
        }
    }
    None
}

fn extract_venue(text: &str) -> Option<String> {
    for label in &["venue:", "location:", "where:", "address:"] {
        if let Some(pos) = text.find(label) {
            let after = text[pos + label.len()..].trim_start();
            let end = after.find('\n').unwrap_or(after.len()).min(80);
            let value = after[..end].trim();
            if !value.is_empty() {
                return Some(capitalize_first(value));
            }
        }
    }
    None
}

fn capitalize_first(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
    }

    #[test]
    fn parse_datetime_tz_central_converts_to_utc_with_dst() {
        // May (CDT, UTC-5): 09:00 local → 14:00 UTC.
        assert_eq!(
            parse_datetime_tz("May 29, 2026 09:00 AM Central Time (US and Canada)", ref_date())
                .as_deref(),
            Some("2026-05-29T14:00:00Z")
        );
        // January (CST, UTC-6): 09:00 local → 15:00 UTC. Proves per-date DST.
        assert_eq!(
            parse_datetime_tz("January 29, 2026 09:00 AM Central Time", ref_date()).as_deref(),
            Some("2026-01-29T15:00:00Z")
        );
    }

    #[test]
    fn parse_datetime_tz_unknown_zone_is_floating() {
        // Unknown/absent tz with a time → floating (no trailing Z).
        assert_eq!(
            parse_datetime_tz("May 29, 2026 09:00 AM Narnia Time", ref_date()).as_deref(),
            Some("2026-05-29T09:00:00")
        );
        assert_eq!(
            parse_datetime_tz("May 29, 2026 14:30", ref_date()).as_deref(),
            Some("2026-05-29T14:30:00")
        );
    }

    /// The exact shape of two real recruiting emails, anonymized: an emphasized,
    /// unlabeled date line with the time range on the line beneath it, and a
    /// DIFFERENT zone abbreviation sitting in the signature further down.
    ///
    /// Before the fix this produced an ALL-DAY event on the day the mail arrived
    /// (`reference_date`), because `find_labeled_datetime` only inspects lines
    /// starting with a label and the date line could not be parsed anyway
    /// (`words[0]` was `*Thursday,`).
    const UNLABELED_SCHEDULE_BODY: &str = "Hi Christopher,\n\n\
        Thanks for taking the time and letting us know your availability to chat!\n\
        Your interview is confirmed for the following schedule and you should have\n\
        received a calendar invite.\n\n\
        *Thursday, August 13, 2026:*\n\
        12:30pm - 1:00pm ET - Hiring Manager Screen with A. Interviewer\n\n\
        Join this video call link: https://meet.google.com/abc-defg-hij\n\n\
        Best,\n*A. Recruiter*\nRecruiting @ Example\n\u{23f1}\u{fe0f} PST\n";

    #[test]
    fn an_unlabeled_date_line_with_the_time_beneath_it_is_found() {
        let event = detect_meeting_invite(
            Some("Interview | Christopher Robinson"),
            UNLABELED_SCHEDULE_BODY,
            0.5,
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
        )
        .expect("a Meet link plus a schedule must detect");

        // 12:30pm ET on Aug 13 2026 (EDT, UTC-4) => 16:30Z. NOT an all-day event
        // on Aug 10, which is what the reference-date fallback used to produce.
        assert_eq!(event.dtstart, "2026-08-13T16:30:00Z");
        assert_eq!(event.dtend.as_deref(), Some("2026-08-13T17:00:00Z"));
        assert_eq!(
            event.description.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        // URL + phrase ("video call link") + datetime.
        assert!((event.confidence - 0.95).abs() < f64::EPSILON);
    }

    /// The signature carries `PST` while the meeting is `ET`. Reading the zone
    /// from the whole message instead of from the line the time came from would
    /// silently shift this meeting by three hours — a wrong answer that looks
    /// entirely plausible.
    #[test]
    fn the_zone_comes_from_the_time_line_not_the_signature() {
        let event = detect_meeting_invite(
            Some("Interview"),
            UNLABELED_SCHEDULE_BODY,
            0.5,
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(event.dtstart, "2026-08-13T16:30:00Z", "ET, not PST");
        assert_ne!(
            event.dtstart, "2026-08-13T19:30:00Z",
            "19:30Z would mean the signature's PST won"
        );
    }

    #[test]
    fn date_decoration_and_weekday_prefixes_are_stripped() {
        let r = ref_date();
        let expected = NaiveDate::from_ymd_opt(2026, 8, 13);
        for line in [
            "*Thursday, August 13, 2026:*",
            "Thursday, August 13, 2026",
            "thu, August 13, 2026",
            "**August 13, 2026**",
            "August 13, 2026:",
            "_Thu, August 13, 2026_",
        ] {
            assert_eq!(parse_natural_date(line, r), expected, "failed on {line:?}");
        }
        // A weekday short form must not eat the start of a real word, and prose
        // must still not parse as a date.
        assert_eq!(parse_natural_date("Sunscreen 5, 2026", r), None);
        assert_eq!(parse_natural_date("we met in August 2025 about this", r), None);
    }

    #[test]
    fn a_time_range_yields_both_ends_and_a_lone_time_yields_only_a_start() {
        assert_eq!(
            parse_clock_times("12:30pm - 1:00pm ET - Hiring Manager Screen"),
            vec![(12, 30), (13, 0)]
        );
        assert_eq!(parse_clock_times("10:30am - 10:55am PT"), vec![(10, 30), (10, 55)]);
        assert_eq!(parse_clock_times("at 9:00 AM sharp"), vec![(9, 0)]);
        assert_eq!(parse_clock_times("no times here"), Vec::new());

        // An end time that is not after the start is discarded rather than
        // producing an event that ends before it begins.
        let body = "*August 13, 2026:*\n3:00pm - 1:00pm ET\nJoin meeting https://meet.google.com/x-y-z\n";
        let event =
            detect_meeting_invite(Some("s"), body, 0.5, NaiveDate::from_ymd_opt(2026, 8, 10).unwrap())
                .unwrap();
        assert_eq!(event.dtstart, "2026-08-13T19:00:00Z");
        assert_eq!(event.dtend, None, "a backwards range must not become a dtend");
    }

    /// A labeled datetime must keep winning, so nothing that worked before the
    /// unlabeled fallback existed changes behaviour.
    #[test]
    fn a_labeled_datetime_still_takes_precedence() {
        let body = "Sam is inviting you to a scheduled Zoom meeting.\n\n\
            Time: May 29, 2026 09:00 AM Central Time (US and Canada)\n\n\
            *August 13, 2026:*\n11:00pm - 11:30pm ET\n\n\
            Join Zoom Meeting\nhttps://us02web.zoom.us/j/85123456789\n";
        let event =
            detect_meeting_invite(Some("s"), body, 0.5, ref_date()).unwrap();
        assert_eq!(event.dtstart, "2026-05-29T14:00:00Z", "the labeled Time: wins");
        assert_eq!(event.dtend, None);
    }

    /// The three panics fixed in #21b are gone, but the class is not closed — so
    /// the funnel is pinned instead of the individual sites. Feeds bodies that
    /// previously panicked, plus multibyte content generally, and asserts the
    /// caller gets a value rather than an unwind.
    #[test]
    fn detect_events_never_panics_on_hostile_bodies() {
        let hostile = [
            // The exact shape that panicked: 3-byte chars where byte 10 lands
            // mid-character.
            "──────────",
            "─────",
            "Time: ──────────\n",
            "Date: ──────────────────────────────────────────────────────────────────\n",
            // Multibyte at every offset around the old slice points.
            "é────\nJoin meeting https://us02web.zoom.us/j/1\n",
            "日本語のテキストです、とても長い行です、これはテストのためのものです\n",
            "🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉\n",
            // Truncated / degenerate.
            "",
            "\n\n\n",
            "Time:",
            "*",
            "─",
        ];
        // Asserts ONLY that the call returns. An empty result is a fine answer,
        // and deliberately nothing here depends on detection succeeding — so
        // reintroducing a panic makes THIS test fail, and nothing else.
        for body in hostile {
            let _ = detect_events(Some("Meeting ──────"), Some(body), "a@b.com", body);
            let _ = detect_events(None, Some(body), "", "──────────");
        }
    }

    /// The positive control for the guard above: it must not be masking a
    /// detector that quietly stopped detecting. Kept free of the box-drawing
    /// separator, so a panic regression fails the panic test rather than this one
    /// — two failures with two different causes, not one ambiguous failure.
    #[test]
    fn a_multibyte_body_still_detects_a_real_invite() {
        let mixed = "Hi 🎉\n\n*Thursday, August 13, 2026:*\n\
                     12:30pm - 1:00pm ET — Café ☕ review\n\n\
                     Join this video call link: https://meet.google.com/abc-defg-hij\n";
        let events = detect_events(
            Some("Review"),
            Some(mixed),
            "a@b.com",
            "2026-08-10T12:00:00Z",
        );
        assert!(
            events.iter().any(|e| e.dtstart == "2026-08-13T16:30:00Z"),
            "multibyte prose must not stop detection: {events:?}"
        );
    }

    #[test]
    fn parse_datetime_tz_no_time_returns_none() {
        // Date but no clock time → None (caller keeps date-only all-day).
        assert_eq!(parse_datetime_tz("May 29, 2026", ref_date()), None);
    }

    const ERIC_BODY: &str = "Sam Smith is inviting you to a scheduled Zoom meeting.\n\n\
Topic: Northwind Company Project Discussion\n\
Time: May 29, 2026 09:00 AM Central Time (US and Canada)\n\n\
Join Zoom Meeting\n\
https://us02web.zoom.us/j/87654321098?pwd=abcdEFGH\n\n\
Meeting ID: 876 5432 1098\n\
Sent from my iPhone\n";

    #[test]
    fn detect_meeting_invite_zoom_body() {
        let event = detect_meeting_invite(
            Some("Northwind Company Project Discussion (Forwarded)"),
            ERIC_BODY,
            0.65,
            ref_date(),
        )
        .expect("should detect the pasted Zoom invite");

        assert!((event.confidence - 0.95).abs() < 1e-9, "URL+phrase+datetime → 0.95");
        assert_eq!(
            event.description.as_deref(),
            Some("https://us02web.zoom.us/j/87654321098?pwd=abcdEFGH")
        );
        assert_eq!(event.dtstart, "2026-05-29T14:00:00Z");
        assert_eq!(event.summary, "Northwind Company Project Discussion");
        assert_eq!(event.location, None);
    }

    #[test]
    fn detect_meeting_invite_rejects_webinar_promo() {
        // Real zoom URL + a datetime, but webinar/promo with no signature phrase.
        let promo = "Join our Zoom webinar!\n\
Time: June 1, 2026 02:00 PM Eastern Time\n\
Register now: https://us02web.zoom.us/j/111222333\n";
        assert!(detect_meeting_invite(Some("Free Webinar"), promo, 0.65, ref_date()).is_none());
    }

    #[test]
    fn detect_meeting_invite_rejects_non_provider_url() {
        // Lookalike URL on a non-provider host → no join URL → None, even with a
        // signature phrase present.
        let body = "Acme is inviting you to read more.\n\
See https://example.com/zoom for details.\n";
        assert!(detect_meeting_invite(Some("Newsletter"), body, 0.65, ref_date()).is_none());
    }

    #[test]
    fn detect_events_surfaces_invite_end_to_end() {
        let events = detect_events(
            Some("Northwind Company Project Discussion"),
            Some(ERIC_BODY),
            "sam@harborline.example",
            "2026-05-28T20:00:00Z",
        );
        assert!(
            events
                .iter()
                .any(|e| e.description.as_deref() == Some(
                    "https://us02web.zoom.us/j/87654321098?pwd=abcdEFGH"
                ) && e.dtstart == "2026-05-29T14:00:00Z"),
            "Strategy 5 should produce the timed Zoom event"
        );
    }
}
