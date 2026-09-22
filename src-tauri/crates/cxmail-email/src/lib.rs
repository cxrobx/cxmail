//! cxmail's mail engine.
//!
//! IMAP, SMTP, OAuth2, parsing, sync, calendar and the AI stack. Depends on
//! `cxmail-db` — that direction is real and stays — but no longer the other way
//! round, and **not on tauri**. The two modules that once took a
//! `tauri::AppHandle` (`oauth2`, `gcal_invite`) take a
//! [`cxmail_core::AppCtx`] instead, which carries the only two things they used
//! it for: an event sink and the shared connection.
//!
//! `email::idle` is the exception that stayed behind in the app crate — its six
//! `crate::notify` calls reach macOS notification APIs that are Tauri-bound,
//! and `lib.rs` was its only caller.

pub mod email;

// Re-exported at their historical paths so the 32 files under `email/` keep
// saying `crate::db::messages`, `crate::error::AppError`, `crate::keychain` and
// `crate::LockExt` unedited.
pub use cxmail_core::{error, keychain, secrets, LockExt};
pub use cxmail_db::db;
