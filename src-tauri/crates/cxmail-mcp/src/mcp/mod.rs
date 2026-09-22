// The socket notifier lives in `cxmail-core`, not here: it has no `crate::`
// imports of its own, and the APP consumes it too (`lib.rs` listens for pokes
// from the standalone MCP process). Left in the mcp module, the app would need
// a dependency on the whole MCP crate for two symbols.
pub use cxmail_core::bridge;

pub mod server;
