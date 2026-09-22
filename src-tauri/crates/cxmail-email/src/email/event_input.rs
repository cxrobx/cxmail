//! Validation for user-supplied calendar event input.
//!
//! Lives in `cxmail-email` rather than `commands/` because both the Tauri
//! calendar path and the MCP `create_calendar_event` tool need it, and the MCP
//! is not allowed to depend on the app crate. Neither function touches Tauri or
//! the database — they turn strings into the wire types and reject what Google
//! would reject anyway, with a message a person can act on.

use crate::email::gcal::{EventAttendee, EventDateTime};
use crate::error::AppError;
use chrono::{LocalResult, NaiveDateTime, TimeZone};

pub fn validate_attendees(items: &[String]) -> Result<Vec<EventAttendee>, AppError> {
    let mut result = Vec::new();
    for value in items {
        let email = value.trim().to_ascii_lowercase();
        if email.is_empty() {
            continue;
        }
        if !email.contains('@') || email.contains(char::is_whitespace) {
            return Err(AppError::General(format!(
                "Invalid attendee email: {value}"
            )));
        }
        if !result
            .iter()
            .any(|attendee: &EventAttendee| attendee.email.as_deref() == Some(&email))
        {
            result.push(EventAttendee {
                email: Some(email),
                ..Default::default()
            });
        }
    }
    Ok(result)
}

pub fn event_datetime(value: &str, zone_name: &str) -> Result<EventDateTime, AppError> {
    if value.len() == 10 {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|e| AppError::General(format!("Invalid all-day date: {e}")))?;
        return Ok(EventDateTime {
            date: Some(value.to_string()),
            date_time: None,
            time_zone: Some(zone_name.to_string()),
        });
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(EventDateTime {
            date_time: Some(parsed.to_rfc3339()),
            date: None,
            time_zone: Some(zone_name.to_string()),
        });
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|e| AppError::General(format!("Invalid event date/time: {e}")))?;
    let zone: chrono_tz::Tz = zone_name
        .parse()
        .map_err(|_| AppError::General(format!("Invalid IANA time zone: {zone_name}")))?;
    let local = match zone.from_local_datetime(&naive) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(_, _) => {
            return Err(AppError::General(
                "The selected time is ambiguous because of daylight saving time".to_string(),
            ))
        }
        LocalResult::None => {
            return Err(AppError::General(
                "The selected time does not exist because of daylight saving time".to_string(),
            ))
        }
    };
    Ok(EventDateTime {
        date_time: Some(local.to_rfc3339()),
        date: None,
        time_zone: Some(zone_name.to_string()),
    })
}
