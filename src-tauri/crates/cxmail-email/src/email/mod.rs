pub mod ai;
pub mod attachment_file;
pub mod autoconfig;
pub mod calendar;
pub mod categorize;
pub mod draft_local;
pub mod event_input;
pub mod external_writer;
pub mod gcal;
pub mod gcal_invite;
pub mod gcal_sync;
pub mod imap;
pub mod import;
pub mod inference;
pub mod inline_styles;
pub mod jmap;
pub mod loose_text;
pub mod oauth2;
pub mod parser;
pub mod providers;
pub mod search_parse;
pub mod server_search;
pub mod smtp;
pub mod spam;
pub mod summarize;
pub mod sync;
pub mod tracking;
pub mod triage;
pub mod triage_gate;
pub mod voice;
pub mod zoom;
pub mod zoom_sync;

// These four moved into `cxmail-core` so `cxmail-db` could stop importing this
// crate — they are pure functions with no IMAP, network, or Tauri surface, and
// the DB layer legitimately needs them. Re-exported here because they are still
// email concepts: `email::message_id::compute_thread_root_id` reads the same as
// it always did, and every call site is unchanged.
pub use cxmail_core::mail::{dashes, detect_events, message_id, needs_you};
