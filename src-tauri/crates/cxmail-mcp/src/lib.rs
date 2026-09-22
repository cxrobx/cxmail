//! cxmail's MCP server — the tool surface an AI agent drives the mailbox
//! through.
//!
//! Lib-only. The `cxmail-mcp` *binary* stays in the app package (see this
//! crate's Cargo.toml for why), and `bin/mcp.rs` reaches this code through
//! `cxmail_lib::mcp::server`, which the app re-exports.

pub mod mcp;

// Re-exported at their historical paths so `server.rs` keeps saying
// `crate::db::messages`, `crate::email::smtp` and `crate::error::AppError`.
pub use cxmail_core::{error, keychain, secrets, LockExt};
pub use cxmail_db::db;
pub use cxmail_email::email;
