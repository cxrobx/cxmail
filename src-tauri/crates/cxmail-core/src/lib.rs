//! Foundations shared by every cxmail crate.
//!
//! The rule for this crate: everything in it is a leaf. It may depend on
//! third-party crates, but never on `cxmail-db`, `cxmail-email`, `cxmail-mcp`
//! or the app — which is what lets all four depend on it. Chief among them is
//! [`error::AppError`], referenced from roughly two thirds of the backend; had
//! it stayed in the app crate, every other crate would have re-coupled to Tauri
//! through it.

pub mod bridge;
pub mod error;
pub mod events;
pub mod keychain;
pub mod mail;
pub mod secrets;

pub use error::AppError;
pub use events::{AppCtx, EventSink, NoopSink};

use std::sync::{Mutex, MutexGuard};

/// Extension trait for safe mutex locking that tolerates poisoned mutexes.
/// In a desktop app, a poisoned mutex (from a panicked thread) should not
/// cascade-crash the entire application.
pub trait LockExt<T> {
    fn safe_lock(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn safe_lock(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}
