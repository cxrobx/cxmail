//! Google Calendar wire types.
//!
//! Serde shapes only — `GcalClient` and everything reqwest-flavoured stays in
//! `email::gcal`. They live here because `db::gcal` and `db::zoom` persist
//! these structs, and a DB layer that had to reach into the email crate for a
//! DTO is what made `db` and `email` mutually dependent.
//!
//! `email::gcal` re-exports all of them, so `email::gcal::GcalEvent` still
//! resolves.

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GcalCalendar {
    pub id: String,
    pub summary: Option<String>,
    pub time_zone: Option<String>,
    pub access_role: Option<String>,
    pub background_color: Option<String>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventPerson {
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(default, rename = "self")]
    pub self_attendee: bool,
    #[serde(default)]
    pub organizer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
/// One attendee. Every field is skipped when empty because this struct is used
/// for BOTH directions: Google populates `displayName` / `responseStatus` /
/// `self` / `organizer` on read, but rejects them on write. Serializing a
/// `None` as an explicit `null` makes `events.insert` fail with a bare
/// `400 Bad Request` — `responseStatus: null` is not a valid enum value, and
/// `self` / `organizer` are output-only.
pub struct EventAttendee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
    #[serde(default, rename = "self", skip_serializing_if = "is_false")]
    pub self_attendee: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub organizer: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceSolutionKey {
    #[serde(rename = "type")]
    pub solution_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceCreateRequest {
    pub request_id: String,
    pub conference_solution_key: ConferenceSolutionKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceEntryPoint {
    pub entry_point_type: Option<String>,
    pub uri: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_request: Option<ConferenceCreateRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry_points: Vec<ConferenceEntryPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GcalEvent {
    pub id: Option<String>,
    /// Explicit rename: `rename_all = "camelCase"` renders this field as
    /// `iCalUid`, but Google's field is `iCalUID`. Without this the value never
    /// deserialized, `gcal_events.ical_uid` stayed NULL for every remote row,
    /// its index could never match, and `list_unified_by_range` could not
    /// collapse a mailed .ics invite against the same event on the calendar —
    /// so both showed up.
    #[serde(rename = "iCalUID")]
    pub i_cal_uid: Option<String>,
    pub etag: Option<String>,
    pub status: Option<String>,
    pub html_link: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub organizer: Option<EventPerson>,
    pub attendees: Option<Vec<EventAttendee>>,
    #[serde(default)]
    pub start: EventDateTime,
    pub end: Option<EventDateTime>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<EventDateTime>,
    pub transparency: Option<String>,
    pub sequence: Option<i64>,
    pub hangout_link: Option<String>,
    pub conference_data: Option<ConferenceData>,
}


#[cfg(test)]
mod tests {
    use super::*;

    /// These types crossed a crate boundary in the workspace split, and their
    /// serde attributes are the whole contract with Google. `EventAttendee`
    /// documents the failure mode: an output-only field serialized as an
    /// explicit `null` makes `events.insert` fail with a bare `400 Bad
    /// Request`. Pin the wire shape so a lost attribute is a test failure
    /// rather than a calendar invite that silently stops sending.
    #[test]
    fn attendee_omits_every_field_google_rejects_on_write() {
        let attendee = EventAttendee {
            email: Some("chris@cxventures.io".to_string()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&attendee).unwrap(),
            r#"{"email":"chris@cxventures.io"}"#,
            "responseStatus/self/organizer are output-only; emitting them fails the write"
        );
    }

    #[test]
    fn event_datetime_is_camel_case_and_skips_empties() {
        let timed = EventDateTime {
            date_time: Some("2026-08-19T14:00:00-04:00".to_string()),
            date: None,
            time_zone: Some("America/New_York".to_string()),
        };
        assert_eq!(
            serde_json::to_string(&timed).unwrap(),
            r#"{"dateTime":"2026-08-19T14:00:00-04:00","timeZone":"America/New_York"}"#
        );

        let all_day = EventDateTime {
            date: Some("2026-08-19".to_string()),
            date_time: None,
            time_zone: None,
        };
        assert_eq!(serde_json::to_string(&all_day).unwrap(), r#"{"date":"2026-08-19"}"#);
    }

    #[test]
    fn calendar_and_event_read_the_camel_case_google_sends() {
        let event: GcalEvent = serde_json::from_str(
            r#"{"id":"e1","iCalUID":"u1","summary":"Sync",
                "start":{"dateTime":"2026-08-19T14:00:00Z"},
                "organizer":{"email":"a@b.com","self":true},
                "hangoutLink":"https://meet.google.com/abc"}"#,
        )
        .expect("deserializes Google's camelCase");
        assert_eq!(event.hangout_link.as_deref(), Some("https://meet.google.com/abc"));
        assert!(event.organizer.expect("organizer").self_attendee);

        // The field Google spells `iCalUID`, which `rename_all = "camelCase"`
        // alone renders as `iCalUid` and therefore never matched. This is the
        // regression guard: without the explicit rename, every remote row's
        // ical_uid is NULL and a mailed .ics invite shows up alongside the same
        // event from the calendar instead of collapsing into it.
        assert_eq!(event.i_cal_uid.as_deref(), Some("u1"));

        let cal: GcalCalendar =
            serde_json::from_str(r#"{"id":"c1","accessRole":"owner","primary":true}"#).unwrap();
        assert_eq!(cal.access_role.as_deref(), Some("owner"));
        assert!(cal.primary);
    }
}
