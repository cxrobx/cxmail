//! The dependency-light half of the mail domain.
//!
//! Everything here is a pure function or a serde shape: no IMAP, no SMTP, no
//! network, no `mail_parser`, no `ammonia`. It lives below `cxmail-db` because
//! the database layer genuinely needs it — threading keys, snippet cleanup,
//! event detection, calendar DTOs — and before the split that need was an
//! import cycle between `db` and `email`.
//!
//! `cxmail-email` re-exports each of these under its historical path, so
//! `email::message_id`, `email::needs_you`, `email::parser::clean_snippet` and
//! friends all still resolve.

pub mod dashes;
pub mod date;
pub mod detect_events;
pub mod gcal_dto;
pub mod message_id;
pub mod needs_you;
pub mod text;
pub mod triage;
pub mod weekdays;
