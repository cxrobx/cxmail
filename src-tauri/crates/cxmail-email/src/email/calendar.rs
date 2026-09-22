use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CalendarEvent {
    pub event_uid: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub status: Option<String>,
    pub method: Option<String>,
    pub raw_ics: String,
}

/// Parse ICS content and extract calendar events.
pub fn parse_ics(ics_content: &str) -> Vec<CalendarEvent> {
    let mut events = Vec::new();

    // Extract METHOD from the top-level VCALENDAR
    let method = extract_property(ics_content, "METHOD");

    // Split on VEVENT blocks
    let mut remaining = ics_content;
    while let Some(start) = remaining.find("BEGIN:VEVENT") {
        if let Some(end) = remaining[start..].find("END:VEVENT") {
            let event_block = &remaining[start..start + end + "END:VEVENT".len()];

            let dtstart = extract_property(event_block, "DTSTART");
            if let Some(dtstart_val) = dtstart {
                events.push(CalendarEvent {
                    event_uid: extract_property(event_block, "UID"),
                    summary: extract_property(event_block, "SUMMARY"),
                    description: extract_property(event_block, "DESCRIPTION"),
                    location: extract_property(event_block, "LOCATION"),
                    dtstart: normalize_ics_datetime(&dtstart_val),
                    dtend: extract_property(event_block, "DTEND").map(|d| normalize_ics_datetime(&d)),
                    organizer_name: extract_organizer_name(event_block),
                    organizer_email: extract_organizer_email(event_block),
                    status: extract_property(event_block, "STATUS"),
                    method: method.clone(),
                    raw_ics: ics_content.to_string(),
                });
            }

            remaining = &remaining[start + end + "END:VEVENT".len()..];
        } else {
            break;
        }
    }

    events
}

/// Generate an ICS RSVP response (METHOD:REPLY).
pub fn generate_rsvp_ics(
    event_uid: &str,
    dtstart: &str,
    organizer_email: &str,
    attendee_email: &str,
    response: &str, // "ACCEPTED", "DECLINED", "TENTATIVE"
) -> String {
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    format!(
        "BEGIN:VCALENDAR\r\n\
         VERSION:2.0\r\n\
         PRODID:-//CXMail//EN\r\n\
         METHOD:REPLY\r\n\
         BEGIN:VEVENT\r\n\
         UID:{}\r\n\
         DTSTART:{}\r\n\
         DTSTAMP:{}\r\n\
         ORGANIZER:mailto:{}\r\n\
         ATTENDEE;PARTSTAT={}:mailto:{}\r\n\
         END:VEVENT\r\n\
         END:VCALENDAR\r\n",
        event_uid, dtstart, now, organizer_email, response, attendee_email
    )
}

/// Extract a simple ICS property value (handles property parameters like DTSTART;VALUE=DATE:20260401).
fn extract_property(block: &str, name: &str) -> Option<String> {
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(name) {
            let after_name = &trimmed[name.len()..];
            if after_name.starts_with(':') {
                return Some(after_name[1..].to_string());
            } else if after_name.starts_with(';') {
                // Has parameters, find the colon
                if let Some(colon_pos) = after_name.find(':') {
                    return Some(after_name[colon_pos + 1..].to_string());
                }
            }
        }
    }
    None
}

/// Extract organizer email from ORGANIZER property (e.g., "ORGANIZER;CN=Name:mailto:email@example.com").
fn extract_organizer_email(block: &str) -> Option<String> {
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ORGANIZER") {
            if let Some(mailto_pos) = trimmed.to_lowercase().find("mailto:") {
                return Some(trimmed[mailto_pos + 7..].to_string());
            }
        }
    }
    None
}

/// Extract organizer CN (common name) from ORGANIZER property.
fn extract_organizer_name(block: &str) -> Option<String> {
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ORGANIZER") {
            if let Some(cn_pos) = trimmed.find("CN=") {
                let after_cn = &trimmed[cn_pos + 3..];
                let end = after_cn.find(|c: char| c == ':' || c == ';').unwrap_or(after_cn.len());
                let name = after_cn[..end].trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Normalize ICS datetime format (20260401T090000Z) to ISO 8601.
fn normalize_ics_datetime(dt: &str) -> String {
    let dt = dt.trim();
    if dt.len() >= 15 && dt.contains('T') {
        // Format: 20260401T090000 or 20260401T090000Z
        let year = &dt[0..4];
        let month = &dt[4..6];
        let day = &dt[6..8];
        let hour = &dt[9..11];
        let min = &dt[11..13];
        let sec = &dt[13..15];
        let tz = if dt.ends_with('Z') { "Z" } else { "" };
        format!("{}-{}-{}T{}:{}:{}{}", year, month, day, hour, min, sec, tz)
    } else if dt.len() == 8 {
        // All-day event: 20260401
        let year = &dt[0..4];
        let month = &dt[4..6];
        let day = &dt[6..8];
        format!("{}-{}-{}", year, month, day)
    } else {
        dt.to_string()
    }
}
