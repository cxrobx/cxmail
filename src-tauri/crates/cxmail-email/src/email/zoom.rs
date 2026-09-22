//! Zoom Server-to-Server OAuth client — the minimum needed to own a meeting's
//! lifecycle (create, retime, delete) on behalf of one account.
//!
//! Why this exists at all: Google Meet screen-share on macOS cannot carry
//! computer audio, because a browser gets no system-audio tap. Zoom sidesteps
//! that with its own audio driver, but Google cannot mint a Zoom link — only
//! Zoom can — so attaching one to a calendar event needs a real API client.
//!
//! Shape borrowed from two existing files on purpose: the client struct and the
//! single `send` funnel mirror [`crate::email::gcal::GcalClient`]; the non-OAuth
//! HTTP hygiene (compile-time endpoint override, HTTPS guard, per-call timeout,
//! typed response) mirrors `commands::license::validate_remote`.

use crate::error::AppError;
use crate::keychain;
use chrono::Utc;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};

/// Credentials are account-global, not per-Gmail-account: one Zoom
/// Server-to-Server app owns every meeting CXMail creates. Deliberately NOT part
/// of `commands::accounts::credential_keys_for` — removing a Gmail account must
/// not delete the Zoom app registration (pinned by a test there).
pub const ACCOUNT_ID_KEY: &str = "zoom:cxmail:account_id";
pub const CLIENT_ID_KEY: &str = "zoom:cxmail:client_id";
pub const CLIENT_SECRET_KEY: &str = "zoom:cxmail:client_secret";
pub const ACCESS_KEY: &str = "zoom:cxmail:access";
pub const EXPIRES_KEY: &str = "zoom:cxmail:expires";

/// Every key this integration owns, in the order a partial write should be
/// unwound. Used by `commands::zoom` so "save failed halfway" cannot leave a
/// client id paired with someone else's secret.
pub const ALL_CREDENTIAL_KEYS: [&str; 5] = [
    ACCOUNT_ID_KEY,
    CLIENT_ID_KEY,
    CLIENT_SECRET_KEY,
    ACCESS_KEY,
    EXPIRES_KEY,
];

const DEFAULT_API_ROOT: &str = "https://api.zoom.us/v2";
const DEFAULT_OAUTH_URL: &str = "https://zoom.us/oauth/token";

/// Refresh this many seconds before the token actually expires. Same 300s skew
/// `oauth2::get_valid_gcal_access_token` uses, for the same reason: a token that
/// expires mid-flight fails the call, not the next one.
const EXPIRY_SKEW_SECONDS: i64 = 300;

/// Zoom rejects a topic over 200 characters. The calendar summary stays
/// authoritative, so truncating here is lossless where it matters.
pub const MAX_TOPIC_CHARS: usize = 200;

/// A scheduled meeting, as Zoom describes it back to us.
///
/// **There is deliberately no `start_url` field.** Zoom returns one on create: a
/// URL that starts the meeting *as the host, already authenticated*. serde drops
/// unknown fields, so omitting the field means the value dies at the parse
/// boundary and cannot reach a log line, a DB column, an error message, or a
/// tool result — a stronger guarantee than a review convention, and one that
/// survives refactors. Pinned by
/// `tests::a_create_response_cannot_capture_start_url`.
#[derive(Debug, Clone, Deserialize)]
pub struct ZoomMeeting {
    /// Zoom sends this as a JSON number; we store it as TEXT everywhere because
    /// it is an identifier, not a quantity.
    #[serde(deserialize_with = "meeting_id_as_string")]
    pub id: String,
    pub join_url: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
}

/// Zoom's meeting id is numeric in JSON but an identifier in meaning. Accept
/// either spelling so a future API change to a string id does not become a parse
/// failure on the create path.
fn meeting_id_as_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(i64),
        Text(String),
    }
    match Raw::deserialize(deserializer)? {
        Raw::Number(value) => Ok(value.to_string()),
        Raw::Text(value) => Ok(value),
    }
}

/// Create body for `POST /users/me/meetings`.
///
/// `start_time` is sent as an instant (`…Z`) and `timezone` is deliberately
/// **omitted** — Zoom ignores the zone when the instant is explicit, and sending
/// both invites a disagreement between two representations of one moment.
#[derive(Debug, Clone, Serialize)]
pub struct NewZoomMeeting {
    pub topic: String,
    /// 2 = scheduled meeting. Recurring types are unreachable by design.
    #[serde(rename = "type")]
    pub meeting_type: u8,
    pub start_time: String,
    pub duration: i64,
    /// Carries `CXMail:<gcal_event_id>` so a future Zoom-side sweep can
    /// correlate an orphaned meeting back to the event that made it.
    pub agenda: String,
}

/// Partial update for `PATCH /meetings/{id}`. Every field is skipped when
/// absent, so a rename sends `topic` and nothing else — which is exactly what
/// makes the reconciler's change detection observable on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ZoomMeetingPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
}

impl ZoomMeetingPatch {
    pub fn is_empty(&self) -> bool {
        self.topic.is_none() && self.start_time.is_none() && self.duration.is_none()
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

/// The three values a Zoom Server-to-Server OAuth app is identified by.
#[derive(Debug, Clone)]
pub struct ZoomCredentials {
    pub account_id: String,
    pub client_id: String,
    pub client_secret: String,
}

/// Read the stored credential triple. `Ok(None)` means "not configured", which
/// is a normal state and not an error — Zoom is opt-in.
pub fn load_credentials() -> Result<Option<ZoomCredentials>, AppError> {
    let account_id = keychain::get_credential(ACCOUNT_ID_KEY)?;
    let client_id = keychain::get_credential(CLIENT_ID_KEY)?;
    let client_secret = keychain::get_credential(CLIENT_SECRET_KEY)?;
    match (account_id, client_id, client_secret) {
        (Some(account_id), Some(client_id), Some(client_secret))
            if !account_id.trim().is_empty()
                && !client_id.trim().is_empty()
                && !client_secret.trim().is_empty() =>
        {
            Ok(Some(ZoomCredentials {
                account_id,
                client_id,
                client_secret,
            }))
        }
        _ => Ok(None),
    }
}

pub fn is_configured() -> bool {
    matches!(load_credentials(), Ok(Some(_)))
}

/// Pure predicate: may the cached access token still be used?
///
/// Extracted so the reuse rule is testable without a network double. A
/// regression here re-authenticates on every call, which rate-limits the Zoom
/// account rather than failing visibly.
pub fn token_is_fresh(expires_at: i64, now: i64) -> bool {
    now < expires_at - EXPIRY_SKEW_SECONDS
}

fn api_root() -> String {
    option_env!("CXMAIL_ZOOM_API_ROOT")
        .unwrap_or(DEFAULT_API_ROOT)
        .trim_end_matches('/')
        .to_string()
}

fn oauth_url() -> String {
    option_env!("CXMAIL_ZOOM_OAUTH_URL")
        .unwrap_or(DEFAULT_OAUTH_URL)
        .to_string()
}

/// Refuse a plaintext endpoint. A loopback override is tolerated in debug builds
/// only, so a test harness can stand in for Zoom without weakening a shipped
/// build. Same rule as `commands::license::validate_remote`.
fn require_secure(url: &str) -> Result<(), AppError> {
    if url.starts_with("https://")
        || (cfg!(debug_assertions) && url.starts_with("http://127.0.0.1:"))
    {
        return Ok(());
    }
    Err(AppError::ZoomApi(
        0,
        format!("Zoom endpoint is not secure: {url}"),
    ))
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::ZoomApi(0, format!("Zoom HTTP client failed: {e}")))
}

/// Exchange the credential triple for an access token.
///
/// Auth is HTTP Basic over `client_id:client_secret` — `reqwest`'s `basic_auth`
/// handles the encoding, so no `base64` call is needed here even though the
/// crate is in the tree.
pub async fn fetch_token(credentials: &ZoomCredentials) -> Result<(String, i64, String), AppError> {
    let url = oauth_url();
    require_secure(&url)?;
    let response = http_client()?
        .post(&url)
        .basic_auth(&credentials.client_id, Some(&credentials.client_secret))
        .form(&[
            ("grant_type", "account_credentials"),
            ("account_id", credentials.account_id.as_str()),
        ])
        .send()
        .await
        .map_err(|e| AppError::ZoomApi(0, format!("Zoom token request failed: {e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(zoom_http_error(status, text));
    }
    let token: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| AppError::Parse(format!("Invalid Zoom token response: {e}")))?;
    let expires_at = Utc::now().timestamp() + token.expires_in.unwrap_or(3600);
    Ok((
        token.access_token,
        expires_at,
        token.scope.unwrap_or_default(),
    ))
}

/// Return a usable access token, reusing the cached one until it nears expiry.
async fn valid_access_token(credentials: &ZoomCredentials) -> Result<String, AppError> {
    let cached = keychain::get_credential(ACCESS_KEY)?;
    let expires_at = keychain::get_credential(EXPIRES_KEY)?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if let Some(token) = cached.filter(|t| !t.trim().is_empty()) {
        if token_is_fresh(expires_at, Utc::now().timestamp()) {
            return Ok(token);
        }
    }
    let (token, expires_at, _scope) = fetch_token(credentials).await?;
    keychain::store_credential(ACCESS_KEY, &token)?;
    keychain::store_credential(EXPIRES_KEY, &expires_at.to_string())?;
    Ok(token)
}

#[derive(Clone)]
pub struct ZoomClient {
    access_token: String,
    http: reqwest::Client,
    api_root: String,
}

impl ZoomClient {
    /// Build a client from the stored credentials, minting or reusing a token.
    ///
    /// Returns `AppError::ZoomApi(0, …)` when Zoom is not configured, so the
    /// caller can distinguish "the user never set this up" from "Zoom said no".
    pub async fn connect() -> Result<Self, AppError> {
        let credentials = load_credentials()?.ok_or_else(|| {
            AppError::ZoomApi(
                0,
                "Zoom is not configured. Add the Server-to-Server OAuth account id, client id \
                 and client secret in CXMail's Zoom settings."
                    .to_string(),
            )
        })?;
        let access_token = valid_access_token(&credentials).await?;
        Ok(Self::with_token(access_token))
    }

    pub fn with_token(access_token: String) -> Self {
        Self {
            access_token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("Zoom HTTP client"),
            api_root: api_root(),
        }
    }

    fn url(&self, suffix: &str) -> Result<url::Url, AppError> {
        let joined = format!("{}/{}", self.api_root, suffix.trim_start_matches('/'));
        require_secure(&joined)?;
        url::Url::parse(&joined).map_err(|e| AppError::ZoomApi(0, format!("Invalid Zoom URL: {e}")))
    }

    /// The single request funnel. Every Zoom call goes through here so the
    /// status/transport split and the error shape are decided in one place.
    ///
    /// `Ok(None)` is a successful call with no body — Zoom answers PATCH and
    /// DELETE with `204 No Content`, and treating that as a parse failure would
    /// report every successful reschedule as an error.
    async fn send(
        &self,
        method: Method,
        url: url::Url,
        body: Option<&impl Serialize>,
    ) -> Result<Option<String>, AppError> {
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(&self.access_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AppError::ZoomApi(0, format!("Zoom request failed: {e}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(zoom_http_error(status, text));
        }
        if text.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(text))
    }

    pub async fn create_meeting(&self, body: &NewZoomMeeting) -> Result<ZoomMeeting, AppError> {
        let url = self.url("users/me/meetings")?;
        let text = self
            .send(Method::POST, url, Some(body))
            .await?
            .ok_or_else(|| {
                AppError::ZoomApi(0, "Zoom created the meeting but returned no body".to_string())
            })?;
        serde_json::from_str(&text)
            .map_err(|e| AppError::Parse(format!("Invalid Zoom meeting response: {e}")))
    }

    pub async fn update_meeting(
        &self,
        meeting_id: &str,
        patch: &ZoomMeetingPatch,
    ) -> Result<(), AppError> {
        if patch.is_empty() {
            return Ok(());
        }
        let url = self.url(&format!("meetings/{meeting_id}"))?;
        self.send(Method::PATCH, url, Some(patch)).await?;
        Ok(())
    }

    /// Delete a meeting. A 404 is success: the object we were asked to remove is
    /// already gone, and treating that as a failure would make the delete
    /// reaper retry forever against nothing.
    pub async fn delete_meeting(&self, meeting_id: &str) -> Result<(), AppError> {
        let url = self.url(&format!("meetings/{meeting_id}"))?;
        match self.send(Method::DELETE, url, None::<&&str>).await {
            Ok(_) => Ok(()),
            Err(AppError::ZoomApi(404, _)) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Zoom errors carry `{"code": …, "message": "…"}`. Fall back to a truncated
/// body so an HTML error page cannot flood a log line.
pub fn zoom_http_error(status: StatusCode, body: String) -> AppError {
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| {
            json.get("message")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    AppError::ZoomApi(status.as_u16(), message)
}

/// Truncate a calendar summary to something Zoom will accept as a topic.
pub fn topic_from_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    let topic = if trimmed.is_empty() { "Meeting" } else { trimmed };
    if topic.chars().count() <= MAX_TOPIC_CHARS {
        return topic.to_string();
    }
    topic.chars().take(MAX_TOPIC_CHARS).collect()
}

/// Clamp a computed duration into the range Zoom accepts. Defensive rather than
/// documented: a negative duration is refused earlier (`end > start`), and a
/// multi-day span is not a meeting.
pub fn clamp_duration_minutes(minutes: i64) -> i64 {
    minutes.clamp(1, 1440)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host-authenticated `start_url` must be structurally unreachable, not
    /// merely unlogged. Deserialize a realistic response that includes it, then
    /// prove the value cannot come back out of anything we hold.
    #[test]
    fn a_create_response_cannot_capture_start_url() {
        let raw = serde_json::json!({
            "uuid": "abcd1234==",
            "id": 86903742305i64,
            "host_id": "hostid",
            "topic": "Design review",
            "type": 2,
            "status": "waiting",
            "start_time": "2026-08-12T15:00:00Z",
            "duration": 45,
            "timezone": "America/New_York",
            "join_url": "https://us02web.zoom.us/j/86903742305",
            "start_url": "https://us02web.zoom.us/s/86903742305?zak=SUPER_SECRET_HOST_TOKEN",
            "settings": {"host_video": true}
        })
        .to_string();

        let meeting: ZoomMeeting = serde_json::from_str(&raw).unwrap();
        assert_eq!(meeting.id, "86903742305");
        assert_eq!(meeting.join_url, "https://us02web.zoom.us/j/86903742305");
        assert_eq!(meeting.duration, Some(45));

        // Nothing we can print, store or return carries it.
        let debug = format!("{meeting:?}");
        assert!(!debug.contains("start_url"));
        assert!(!debug.contains("SUPER_SECRET_HOST_TOKEN"));
        assert!(!debug.contains("/s/"));
    }

    #[test]
    fn a_string_meeting_id_is_accepted_too() {
        let meeting: ZoomMeeting = serde_json::from_value(serde_json::json!({
            "id": "86903742305",
            "join_url": "https://us02web.zoom.us/j/86903742305"
        }))
        .unwrap();
        assert_eq!(meeting.id, "86903742305");
    }

    /// A re-auth on every call would silently rate-limit the account rather
    /// than fail, so the reuse window is pinned.
    #[test]
    fn token_is_reused_until_it_nears_expiry() {
        let expires_at = 10_000i64;
        assert!(token_is_fresh(expires_at, 5_000), "fresh token is reused");
        assert!(
            token_is_fresh(expires_at, expires_at - EXPIRY_SKEW_SECONDS - 1),
            "still fresh one second before the skew window opens"
        );
        assert!(
            !token_is_fresh(expires_at, expires_at - EXPIRY_SKEW_SECONDS),
            "refreshes once inside the 300s skew, not at the hard expiry"
        );
        assert!(!token_is_fresh(expires_at, expires_at + 1));
        // An absent cache reads as expires_at = 0, which must never look fresh.
        assert!(!token_is_fresh(0, 1_700_000_000));
    }

    #[test]
    fn a_create_body_sends_an_instant_and_omits_the_timezone() {
        let body = NewZoomMeeting {
            topic: "Design review".to_string(),
            meeting_type: 2,
            start_time: "2026-08-12T15:00:00Z".to_string(),
            duration: 45,
            agenda: "CXMail:abc123".to_string(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["start_time"], "2026-08-12T15:00:00Z");
        assert_eq!(json["type"], 2);
        assert_eq!(json["agenda"], "CXMail:abc123");
        assert!(
            json.get("timezone").is_none(),
            "the instant is explicit; a timezone alongside it is a second, \
             disagreeing representation of one moment"
        );
    }

    #[test]
    fn a_rename_patch_carries_only_the_topic() {
        let patch = ZoomMeetingPatch {
            topic: Some("Renamed".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&patch).unwrap();
        assert_eq!(json["topic"], "Renamed");
        assert!(json.get("start_time").is_none());
        assert!(json.get("duration").is_none());
        assert!(!patch.is_empty());
        assert!(ZoomMeetingPatch::default().is_empty());
    }

    #[test]
    fn topics_are_truncated_and_durations_clamped() {
        assert_eq!(topic_from_summary("  Sync  "), "Sync");
        assert_eq!(topic_from_summary("   "), "Meeting");
        let long = "x".repeat(MAX_TOPIC_CHARS + 50);
        assert_eq!(topic_from_summary(&long).chars().count(), MAX_TOPIC_CHARS);

        assert_eq!(clamp_duration_minutes(45), 45);
        assert_eq!(clamp_duration_minutes(0), 1);
        assert_eq!(clamp_duration_minutes(-30), 1);
        assert_eq!(clamp_duration_minutes(100_000), 1440);
    }

    #[test]
    fn zoom_errors_prefer_the_api_message_and_stay_bounded() {
        let structured = zoom_http_error(
            StatusCode::NOT_FOUND,
            r#"{"code":3001,"message":"Meeting does not exist: 123."}"#.to_string(),
        );
        match structured {
            AppError::ZoomApi(404, message) => {
                assert_eq!(message, "Meeting does not exist: 123.")
            }
            other => panic!("expected ZoomApi(404), got {other:?}"),
        }

        let html = zoom_http_error(StatusCode::BAD_GATEWAY, "<html>".repeat(500));
        match html {
            AppError::ZoomApi(502, message) => assert_eq!(message.chars().count(), 500),
            other => panic!("expected ZoomApi(502), got {other:?}"),
        }
    }

    /// The whole reason `ZoomApi` exists as its own variant: a Zoom 404 must not
    /// be matchable by the arms that delete the Google mirror row.
    #[test]
    fn a_zoom_404_is_not_a_calendar_404() {
        let error = AppError::ZoomApi(404, "Meeting does not exist".to_string());
        assert!(
            !matches!(error, AppError::CalendarApi(404 | 410, _)),
            "a Zoom 404 reaching gcal_sync's delete arm would delete the Google mirror row"
        );
        assert!(error.to_string().starts_with("Zoom API error (404)"));
    }
}
