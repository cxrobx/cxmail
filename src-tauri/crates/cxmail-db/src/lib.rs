//! cxmail's database layer.
//!
//! Deliberately Tauri-free, and provably so: nothing under `db/` has ever named
//! a `tauri::` item, which is why this crate could be lifted out whole. It sits
//! directly above `cxmail-core` and below `cxmail-email` — `email` still calls
//! into `db`, but `db` no longer calls back.

pub mod db;

// Re-exported at their historical paths so the 29 files under `db/` keep saying
// `crate::error::AppError` and `crate::keychain::get_credential` verbatim. The
// move stays a move; none of them needed editing for the extraction itself.
pub use cxmail_core::{error, keychain, LockExt};
