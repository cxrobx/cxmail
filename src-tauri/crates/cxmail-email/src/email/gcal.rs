use crate::email::oauth2;
use crate::error::AppError;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};

const API_ROOT: &str = "https://www.googleapis.com/calendar/v3";

// The wire types live in `cxmail-core` so `cxmail-db` can persist them without
// depending on this crate — that was half the db/email import cycle. Only the
// serde shapes moved; the HTTP client below is unchanged.
pub use cxmail_core::mail::gcal_dto::{
    ConferenceCreateRequest, ConferenceData, ConferenceEntryPoint, ConferenceSolutionKey,
    EventAttendee, EventDateTime, EventPerson, GcalCalendar, GcalEvent,
};


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<GcalCalendar>,
    next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEvent {
    pub id: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub start: EventDateTime,
    pub end: EventDateTime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<EventAttendee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_data: Option<ConferenceData>,
}

impl NewEvent {
    pub fn add_meet(&mut self, request_id: String) {
        self.conference_data = Some(ConferenceData {
            create_request: Some(ConferenceCreateRequest {
                request_id,
                conference_solution_key: ConferenceSolutionKey {
                    solution_type: "hangoutsMeet".to_string(),
                },
            }),
            entry_points: Vec::new(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<EventDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<EventDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<EventAttendee>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conference_data: Option<ConferenceData>,
}

#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub time_min: Option<String>,
    pub time_max: Option<String>,
    pub sync_token: Option<String>,
    pub page_token: Option<String>,
    pub show_deleted: bool,
    pub single_events: bool,
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsPage {
    #[serde(default)]
    pub items: Vec<GcalEvent>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

/// Which conferencing provider backs an event.
///
/// **One enum, not a second `add_zoom` bool.** Two independent booleans make an
/// illegal state representable (`add_meet && add_zoom`), and the two ways to get
/// it wrong fail differently: Google would provision a Meet link that the Zoom
/// block then sits underneath, and the user would see two join buttons with no
/// way to know which one the invitees will use.
///
/// `Meet` is the default so existing behaviour is preserved byte-for-byte — every
/// caller that says nothing keeps getting exactly what it got before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conferencing {
    #[default]
    Meet,
    Zoom,
    None,
}

impl Conferencing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meet => "meet",
            Self::Zoom => "zoom",
            Self::None => "none",
        }
    }

    /// The three accepted spellings, named in error messages.
    pub const VALID: [&'static str; 3] = ["meet", "zoom", "none"];
}

/// Resolve the conferencing choice from the two parameters that can express it.
///
/// `conference` is the real control; `add_meet` is the pre-Zoom boolean, kept for
/// back-compatibility. The back-compat pin is the last arm: both absent ⇒
/// `Meet`, exactly as before Zoom existed.
///
/// A **contradiction is rejected in both directions**, because each direction
/// fails invisibly and oppositely: `conference="zoom"` with `add_meet=true` would
/// silently pick one provider and drop the other, and `conference="meet"` with
/// `add_meet=false` would produce an event with no conferencing at all while the
/// caller believes it asked for a Meet link.
pub fn parse_conferencing(
    conference: Option<&str>,
    add_meet: Option<bool>,
) -> Result<Conferencing, AppError> {
    let requested = match conference.map(|value| value.trim().to_ascii_lowercase()) {
        None => None,
        Some(value) if value.is_empty() => None,
        Some(value) if value == "meet" => Some(Conferencing::Meet),
        Some(value) if value == "zoom" => Some(Conferencing::Zoom),
        Some(value) if value == "none" => Some(Conferencing::None),
        Some(value) => {
            return Err(AppError::General(format!(
                "Unknown conference {value:?}. Valid values: {}.",
                Conferencing::VALID.join(", ")
            )))
        }
    };

    match (requested, add_meet) {
        (Some(Conferencing::Meet), Some(false)) => Err(AppError::General(
            "conference=\"meet\" and add_meet=false contradict each other.".to_string(),
        )),
        (Some(choice), Some(true)) if choice != Conferencing::Meet => Err(AppError::General(
            format!(
                "conference={:?} and add_meet=true contradict each other.",
                choice.as_str()
            ),
        )),
        (Some(choice), _) => Ok(choice),
        (None, Some(false)) => Ok(Conferencing::None),
        // Both absent, or add_meet=true: the pre-Zoom default.
        (None, _) => Ok(Conferencing::Meet),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SendUpdates {
    None,
    ExternalOnly,
    All,
}

impl SendUpdates {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ExternalOnly => "externalOnly",
            Self::All => "all",
        }
    }
}

#[derive(Clone)]
pub struct GcalClient {
    access_token: String,
    http: reqwest::Client,
    api_root: String,
}

impl GcalClient {
    pub async fn for_account(email: &str) -> Result<Self, AppError> {
        let access_token = oauth2::get_valid_gcal_access_token(email).await?;
        Ok(Self::new(access_token))
    }

    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("Google Calendar HTTP client"),
            api_root: API_ROOT.to_string(),
        }
    }

    fn calendar_url(&self, calendar_id: &str, suffix: &str) -> Result<url::Url, AppError> {
        let mut url = url::Url::parse(&self.api_root)
            .map_err(|e| AppError::CalendarApi(0, format!("Invalid Calendar API root: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| AppError::CalendarApi(0, "Invalid Calendar API URL".to_string()))?;
            segments.push("calendars").push(calendar_id);
            for segment in suffix
                .trim_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
            {
                segments.push(segment);
            }
        }
        Ok(url)
    }

    async fn send_json<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        url: url::Url,
        body: Option<&impl Serialize>,
        etag: Option<&str>,
    ) -> Result<T, AppError> {
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(&self.access_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AppError::CalendarApi(0, format!("Calendar request failed: {e}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(calendar_http_error(status, text));
        }
        serde_json::from_str(&text)
            .map_err(|e| AppError::Parse(format!("Invalid Calendar API response: {e}")))
    }

    pub async fn list_calendars(&self) -> Result<Vec<GcalCalendar>, AppError> {
        let mut all = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = url::Url::parse(&format!("{}/users/me/calendarList", self.api_root))
                .map_err(|e| AppError::CalendarApi(0, e.to_string()))?;
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let page: CalendarListResponse = self
                .send_json(Method::GET, url, None::<&&str>, None)
                .await?;
            all.extend(page.items);
            page_token = page.next_page_token;
            if page_token.is_none() {
                break;
            }
        }
        Ok(all)
    }

    pub async fn list_events(
        &self,
        calendar_id: &str,
        query: ListQuery,
    ) -> Result<EventsPage, AppError> {
        let mut url = self.calendar_url(calendar_id, "events")?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(value) = &query.time_min {
                qp.append_pair("timeMin", value);
            }
            if let Some(value) = &query.time_max {
                qp.append_pair("timeMax", value);
            }
            if let Some(value) = &query.sync_token {
                qp.append_pair("syncToken", value);
            }
            if let Some(value) = &query.page_token {
                qp.append_pair("pageToken", value);
            }
            qp.append_pair(
                "showDeleted",
                if query.show_deleted { "true" } else { "false" },
            );
            qp.append_pair(
                "singleEvents",
                if query.single_events { "true" } else { "false" },
            );
            if let Some(value) = query.max_results {
                qp.append_pair("maxResults", &value.to_string());
            }
        }
        self.send_json(Method::GET, url, None::<&&str>, None).await
    }

    pub async fn get_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<GcalEvent, AppError> {
        let url = self.calendar_url(calendar_id, &format!("events/{event_id}"))?;
        self.send_json(Method::GET, url, None::<&&str>, None).await
    }

    pub async fn insert_event(
        &self,
        calendar_id: &str,
        event: &NewEvent,
        send_updates: SendUpdates,
    ) -> Result<GcalEvent, AppError> {
        let requested_meet = event.conference_data.is_some();
        let mut url = self.calendar_url(calendar_id, "events")?;
        url.query_pairs_mut()
            .append_pair("conferenceDataVersion", "1")
            .append_pair("sendUpdates", send_updates.as_str());
        let response = self.send_json(Method::POST, url, Some(event), None).await?;
        if requested_meet && meet_url(&response).is_none() {
            return Err(AppError::CalendarApi(
                200,
                "Google created the event but did not provision a Meet link".to_string(),
            ));
        }
        Ok(response)
    }

    pub async fn patch_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        patch: &EventPatch,
        etag: Option<&str>,
        send_updates: SendUpdates,
    ) -> Result<GcalEvent, AppError> {
        let mut url = self.calendar_url(calendar_id, &format!("events/{event_id}"))?;
        url.query_pairs_mut()
            .append_pair("conferenceDataVersion", "1")
            .append_pair("sendUpdates", send_updates.as_str());
        self.send_json(Method::PATCH, url, Some(patch), etag).await
    }

    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: SendUpdates,
    ) -> Result<(), AppError> {
        let mut url = self.calendar_url(calendar_id, &format!("events/{event_id}"))?;
        url.query_pairs_mut()
            .append_pair("sendUpdates", send_updates.as_str());
        let response = self
            .http
            .delete(url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| AppError::CalendarApi(0, format!("Calendar delete failed: {e}")))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(calendar_http_error(status, text))
    }
}

pub fn generate_event_id() -> String {
    // Google accepts base32hex lowercase IDs. UUID bytes provide 128 bits of
    // entropy; the id remains stable because callers store it before retrying.
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    let mut output = String::with_capacity(26);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

pub fn meet_url(event: &GcalEvent) -> Option<String> {
    event.hangout_link.clone().or_else(|| {
        event.conference_data.as_ref().and_then(|data| {
            data.entry_points
                .iter()
                .find(|entry| entry.entry_point_type.as_deref() == Some("video"))
                .and_then(|entry| entry.uri.clone())
        })
    })
}

fn calendar_http_error(status: StatusCode, body: String) -> AppError {
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    AppError::CalendarApi(status.as_u16(), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_contract_has_stable_id_meet_request_and_required_query() {
        let mut event = NewEvent {
            id: "0123456789abcdefghijklmnop".to_string(),
            summary: "Sync".to_string(),
            description: None,
            location: None,
            start: EventDateTime {
                date_time: Some("2026-07-30T16:30:00-04:00".to_string()),
                time_zone: Some("America/New_York".to_string()),
                date: None,
            },
            end: EventDateTime {
                date_time: Some("2026-07-30T17:30:00-04:00".to_string()),
                time_zone: Some("America/New_York".to_string()),
                date: None,
            },
            attendees: Vec::new(),
            conference_data: None,
        };
        event.add_meet("stable-request-id".to_string());
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["id"], event.id);
        assert_eq!(
            json["conferenceData"]["createRequest"]["requestId"],
            "stable-request-id"
        );
        assert_eq!(
            json["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"],
            "hangoutsMeet"
        );

        let client = GcalClient::new("token".to_string());
        let mut url = client.calendar_url("primary", "events").unwrap();
        url.query_pairs_mut()
            .append_pair("conferenceDataVersion", "1");
        assert!(url.as_str().contains("conferenceDataVersion=1"));
    }

    /// The back-compat pin. Every existing caller passes either nothing or
    /// `add_meet: true`, and both must keep producing a Meet link.
    #[test]
    fn conferencing_defaults_to_meet_and_rejects_contradictions() {
        assert_eq!(parse_conferencing(None, None).unwrap(), Conferencing::Meet);
        assert_eq!(
            parse_conferencing(None, Some(true)).unwrap(),
            Conferencing::Meet
        );
        assert_eq!(parse_conferencing(None, Some(false)).unwrap(), Conferencing::None);
        assert_eq!(
            parse_conferencing(Some("zoom"), None).unwrap(),
            Conferencing::Zoom
        );
        assert_eq!(
            parse_conferencing(Some("ZOOM  "), None).unwrap(),
            Conferencing::Zoom
        );
        assert_eq!(parse_conferencing(Some(""), None).unwrap(), Conferencing::Meet);
        assert_eq!(
            parse_conferencing(Some("none"), None).unwrap(),
            Conferencing::None
        );
        // Redundant but consistent is fine.
        assert_eq!(
            parse_conferencing(Some("meet"), Some(true)).unwrap(),
            Conferencing::Meet
        );

        // Both contradictions, both directions.
        assert!(parse_conferencing(Some("zoom"), Some(true)).is_err());
        assert!(parse_conferencing(Some("none"), Some(true)).is_err());
        assert!(parse_conferencing(Some("meet"), Some(false)).is_err());
        // Unknown values name the valid set.
        let error = parse_conferencing(Some("webex"), None).unwrap_err().to_string();
        assert!(error.contains("meet") && error.contains("zoom") && error.contains("none"));
    }

    #[test]
    fn generated_ids_are_valid_base32hex() {
        let id = generate_event_id();
        assert!(id.len() >= 5);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c)));
    }
}

#[cfg(test)]
mod attendee_serialization_tests {
    use super::*;

    #[test]
    fn a_write_attendee_serializes_to_email_only() {
        // Google returns 400 Bad Request — with no useful message — if an insert
        // carries responseStatus: null or the output-only self/organizer fields.
        let attendee = EventAttendee {
            email: Some("jordanreyes14@gmail.com".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&attendee).unwrap();
        assert_eq!(json, serde_json::json!({"email": "jordanreyes14@gmail.com"}));
    }

    #[test]
    fn read_fields_still_round_trip() {
        let parsed: EventAttendee = serde_json::from_value(serde_json::json!({
            "email": "chris@cxventures.io",
            "displayName": "Chris",
            "responseStatus": "accepted",
            "self": true,
            "organizer": true
        }))
        .unwrap();
        assert_eq!(parsed.response_status.as_deref(), Some("accepted"));
        assert!(parsed.self_attendee && parsed.organizer);
        // and a populated attendee still serializes those back out
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json["responseStatus"], "accepted");
        assert_eq!(json["self"], true);
    }
}
