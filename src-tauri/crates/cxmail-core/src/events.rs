//! Frontend event emission, decoupled from Tauri.
//!
//! `email::oauth2` and `email::gcal_invite` announce progress to the UI while
//! doing work that has nothing else to do with Tauri — an OAuth exchange, an
//! invite send. Taking an `&AppHandle` for that alone would drag the whole
//! Tauri stack into `cxmail-email`, and with it every binary that links the
//! email crate.
//!
//! So they take an `&dyn EventSink` instead. The app crate implements it over
//! `tauri::Emitter`; `cxmail-helper` and the MCP, which have no window to talk
//! to, pass [`NoopSink`].

/// A place to send a fire-and-forget notification to the UI.
///
/// Deliberately one method taking `serde_json::Value`: every existing call site
/// emitted either a `json!` literal or something `Serialize`, and none of them
/// checked the result. Implementations must not block or panic — callers drop
/// the outcome on the floor by design.
pub trait EventSink: Send + Sync + 'static {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

/// Discards every event. For binaries with no frontend, and for tests.
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit_json(&self, _event: &str, _payload: serde_json::Value) {}
}

/// Convenience for the several call sites that emit a `Serialize` struct rather
/// than a `json!` literal. A payload that cannot be serialized becomes `null`
/// rather than a panic — this channel is advisory, not load-bearing.
pub fn to_payload<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

/// The two capabilities `email::oauth2` and `email::gcal_invite` ever actually
/// wanted from a `tauri::AppHandle`: somewhere to announce progress, and the
/// shared database connection.
///
/// Both modules run detached — an OAuth flow lives on its own thread waiting
/// for a loopback callback, an invite delivery on the async runtime — so this
/// is cloneable and `'static` the way an `AppHandle` was. Holding the two
/// capabilities explicitly is what let `cxmail-email` drop its Tauri
/// dependency: the app crate builds one of these from an `AppHandle`, while
/// `cxmail-helper` and the MCP build one with a [`NoopSink`].
#[derive(Clone)]
pub struct AppCtx {
    events: std::sync::Arc<dyn EventSink>,
    db: std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
}

impl AppCtx {
    pub fn new(
        events: std::sync::Arc<dyn EventSink>,
        db: std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    ) -> Self {
        Self { events, db }
    }

    /// The shared connection. Callers lock it with `LockExt::safe_lock` exactly
    /// as they did through `AppState`.
    pub fn db(&self) -> &std::sync::Mutex<rusqlite::Connection> {
        &self.db
    }

    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.emit_json(event, payload);
    }

    /// Emit a `Serialize` payload. Unserializable values become `null` rather
    /// than a panic — this channel is advisory.
    pub fn emit_value<T: serde::Serialize>(&self, event: &str, payload: &T) {
        self.events.emit_json(event, to_payload(payload));
    }
}
