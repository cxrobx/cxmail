use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("IMAP error: {0}")]
    Imap(String),

    #[error("OAuth2 error: {0}")]
    OAuth2(String),

    #[error("Keychain error: {0}")]
    Keychain(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Reconnect required for {0}")]
    ReauthRequired(String),

    #[error("Google Calendar API error ({0}): {1}")]
    CalendarApi(u16, String),

    /// A Zoom REST failure. Deliberately its OWN variant rather than a reuse of
    /// [`AppError::CalendarApi`], and that separation is load-bearing: three
    /// places match `Err(AppError::CalendarApi(404 | 410, _))` and respond by
    /// deleting the local Google mirror row (`gcal_sync::push_account`,
    /// `commands::calendar::delete_google_calendar_event`). A Zoom 404 — a
    /// meeting the user deleted at zoom.us — reaching one of those arms would
    /// delete the *Google* event's cached row as collateral.
    ///
    /// Same conventions as the Calendar variant: `0` is the transport sentinel
    /// ("we never got a status, so we do not know what happened"), and callers
    /// branch on the number.
    #[error("Zoom API error ({0}): {1}")]
    ZoomApi(u16, String),

    #[error("AI service error: {0}")]
    AiService(String),

    #[error("{0}")]
    General(String),
}

// Serialize for Tauri IPC — only expose the message, not internals
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
