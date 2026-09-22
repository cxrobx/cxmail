//! Cross-process notification bridge.
//!
//! The standalone `cxmail-mcp` process and the running CXMail GUI share only the
//! SQLite DB — when an MCP tool writes a draft (IMAP `APPEND` + local cache row),
//! the GUI has no way to know. This bridge carries one-shot JSON notifications
//! over a Unix-domain socket so the app can live-refresh without polling.
//!
//! Wire protocol: connect → write one JSON [`Envelope`] line → close. The app's
//! accept loop (see `lib.rs::run()` `.setup()`) reads the line, parses it, and
//! re-emits it to the frontend as the `mcp-activity` Tauri event.
//!
//! Every failure on the sender side is swallowed (debug-logged only): if the app
//! isn't running, the draft is already committed to SQLite and shows on the next
//! open/sync, so a missing live update must NEVER turn into a tool error.
//!
//! Security: the socket lives in the user's app-data dir, so same-user/same-machine
//! filesystem perms are the access control (no port, no token). The payload carries
//! only identifiers — no credentials or message bodies cross the socket (consistent
//! with the no-secrets-in-transit invariant) — and the app treats it strictly as a
//! *hint to re-fetch from the DB*, never as trusted content.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// Fixed socket location inside the shared app-data dir. Both processes resolve
/// it identically: `dirs::data_dir()/com.cxmail.app/ipc.sock` — on macOS that is
/// `~/Library/Application Support/com.cxmail.app/ipc.sock`, the same directory
/// that holds `cxmail.db` (matching `bin/mcp.rs`'s db-path convention).
pub fn socket_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app")
        .join("ipc.sock")
}

/// One notification. A *hint to re-fetch from the DB*, never trusted content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Schema version. Bump when the shape changes so readers can branch on it.
    pub v: u32,
    /// `"draft-created" | "draft-updated" | "email-mutated" | "calendar" |
    /// "calendar-approval-request"`.
    pub kind: String,
    pub account_id: String,
    pub folder: String,
    /// The new / affected UID.
    pub uid: u32,
    /// `draft-updated` only: the UID the edit replaced (expunged). Lets an open
    /// compose modal match its current UID and then adopt `uid` (the fresh one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_uid: Option<u32>,
    /// Originating tool name, for debugging and toast text. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Correlates an approve-tier request with the in-app modal response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
}

impl Envelope {
    /// Construct a v1 envelope with no `old_uid` / `tool`.
    pub fn new(kind: &str, account_id: &str, folder: &str, uid: u32) -> Self {
        Self {
            v: 1,
            kind: kind.to_string(),
            account_id: account_id.to_string(),
            folder: folder.to_string(),
            uid,
            old_uid: None,
            tool: None,
            approval_id: None,
        }
    }

    /// Attach the replaced UID (for `draft-updated`).
    pub fn with_old_uid(mut self, old_uid: u32) -> Self {
        self.old_uid = Some(old_uid);
        self
    }

    /// Attach the originating tool name (for debugging / toast text).
    pub fn with_tool(mut self, tool: &str) -> Self {
        self.tool = Some(tool.to_string());
        self
    }

    pub fn with_approval_id(mut self, approval_id: &str) -> Self {
        self.approval_id = Some(approval_id.to_string());
        self
    }
}

/// Connect to the app's socket and write one [`Envelope`] line, newline-terminated.
///
/// Best-effort: every failure path — app not running, slow/no accept, serialize
/// error — is debug-logged and swallowed so the caller's tool result is untouched.
/// The 500 ms timeout bounds the pathological case where the socket file exists but
/// nothing is accepting (e.g. the app crashed without cleanup); a stale path usually
/// fails fast with ECONNREFUSED, and the timeout is the backstop.
pub async fn notify(env: &Envelope) {
    let path = socket_path();

    let mut payload = match serde_json::to_vec(env) {
        Ok(v) => v,
        Err(e) => {
            log::debug!("mcp bridge: serialize failed: {e}");
            return;
        }
    };
    payload.push(b'\n');

    let mut stream = match tokio::time::timeout(
        Duration::from_millis(500),
        UnixStream::connect(&path),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            log::debug!(
                "mcp bridge: connect {} failed (app not running?): {e}",
                path.display()
            );
            return;
        }
        Err(_) => {
            log::debug!("mcp bridge: connect {} timed out", path.display());
            return;
        }
    };

    if let Err(e) = stream.write_all(&payload).await {
        // The app accepted the connection but the handoff broke — worth surfacing
        // (unlike the benign "app not running" connect failure above, which stays debug).
        log::warn!("mcp bridge: write to app failed after connect: {e}");
        return;
    }
    if let Err(e) = stream.flush().await {
        log::warn!("mcp bridge: flush to app failed after connect: {e}");
        return;
    }
    // Dropping `stream` closes the write half; the app's reader sees EOF.
    // INFO (not debug) so the positive delivery signal is visible in the MCP
    // stderr logs at the default `cxmail=info` filter — this is the line that
    // confirms an edit_draft notification actually reached a running app.
    log::info!(
        "mcp bridge: delivered {} (tool={:?}, uid={}, old_uid={:?}) to app",
        env.kind, env.tool, env.uid, env.old_uid
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip_draft_updated() {
        let env = Envelope::new("draft-updated", "acct-1", "Drafts", 123)
            .with_old_uid(100)
            .with_tool("edit_draft");
        let bytes = serde_json::to_vec(&env).unwrap();
        let parsed: Envelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(env, parsed);
        assert_eq!(parsed.old_uid, Some(100));
        assert_eq!(parsed.tool.as_deref(), Some("edit_draft"));
    }

    #[test]
    fn envelope_minimal_skips_optional_fields() {
        let env = Envelope::new("email-mutated", "acct-2", "INBOX", 7);
        let json = serde_json::to_string(&env).unwrap();
        // Optional fields are omitted from the wire form when None.
        assert!(!json.contains("old_uid"), "old_uid should be skipped: {json}");
        assert!(!json.contains("\"tool\""), "tool should be skipped: {json}");
        let parsed: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.old_uid, None);
        assert_eq!(parsed.tool, None);
        assert_eq!(parsed.v, 1);
    }

    #[test]
    fn envelope_roundtrip_calendar_approval() {
        let env = Envelope::new("calendar-approval-request", "acct-1", "primary", 42)
            .with_tool("send_calendar_invites")
            .with_approval_id("approval-123");
        let bytes = serde_json::to_vec(&env).unwrap();
        let parsed: Envelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, env);
        assert_eq!(parsed.approval_id.as_deref(), Some("approval-123"));
    }

    #[test]
    fn envelope_tolerates_trailing_newline() {
        let env = Envelope::new("draft-created", "a", "Drafts", 5);
        let mut bytes = serde_json::to_vec(&env).unwrap();
        bytes.push(b'\n');
        // serde_json treats the trailing newline as ignorable whitespace.
        let parsed: Envelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn envelope_ignores_unknown_fields_forward_compat() {
        // A future sender adding fields must not break an older reader.
        let json = r#"{"v":2,"kind":"draft-created","account_id":"a","folder":"Drafts","uid":5,"future_field":true}"#;
        let parsed: Envelope = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.uid, 5);
        assert_eq!(parsed.v, 2);
    }

    #[test]
    fn socket_path_ends_with_expected_segments() {
        let p = socket_path();
        assert!(p.ends_with("com.cxmail.app/ipc.sock"), "got {:?}", p);
    }
}
