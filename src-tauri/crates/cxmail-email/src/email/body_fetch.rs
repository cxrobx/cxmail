//! On-demand body fetches for mail the desktop has not cached yet.
//!
//! The phone opens messages the desktop never displayed, so their bodies are
//! not in SQLite. Every such open used to log in to IMAP from scratch, and
//! more than once: the MCP fetched the body without storing it, then the image
//! policy fetched it again, on every open, forever. Gmail caps simultaneous
//! connections per account and throttles logins from one IP (see the ledger
//! notes in `imap.rs`), so logins are the cost to minimise, not just latency.
//!
//! Three rules here:
//! * **Store what you fetch.** Every fetch goes through
//!   [`body_store::store_fetched_body`], so a message costs one IMAP fetch in
//!   its lifetime and every later open reads SQLite.
//! * **One held session per account** ([`BodyFetcher`]) in the long-running
//!   MCP process, reused while requests keep coming and logged out
//!   [`IDLE_TTL`] after the last one. Ten quick previews are one login, and
//!   the per-account lock makes a racing preview and open share one fetch.
//! * **Every IMAP step has a timeout and every session ends in
//!   [`imap::disconnect`]** (gotchas #10 and #46). A step that times out
//!   leaves the stream mid-response, so that session is closed, never reused.

use super::{body_store, imap, parser};
use crate::{db, error::AppError};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, Once};
use std::time::{Duration, Instant};

/// How long a held session may sit unused before it is logged out.
pub const IDLE_TTL: Duration = Duration::from_secs(45);
/// One SELECT or FETCH. `connect_for_account` carries its own 15 s timeout.
pub const OP_TIMEOUT: Duration = Duration::from_secs(20);
const REAP_EVERY: Duration = Duration::from_secs(15);
/// Newly-arrived INBOX bodies the IDLE sync stores per wake-up, and the time
/// it may spend on them (the same 12 s budget as the snippet prefetch).
pub const IDLE_PREFETCH_LIMIT: usize = 10;
pub const IDLE_PREFETCH_BUDGET: Duration = Duration::from_secs(12);

/// What a fetch produced.
pub enum Fetched {
    /// Fetched now, parsed, and (best effort) stored.
    Parsed(Box<parser::ParsedMessage>),
    /// Another request stored it while this one waited for the account.
    AlreadyCached,
}

struct Held {
    session: Option<imap::ImapSession>,
    email: String,
    folder: Option<String>,
    last_used: Instant,
}

impl Held {
    async fn close(mut self) {
        if let Some(session) = self.session.take() {
            let _ = imap::disconnect(session, &self.email).await;
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Reached only when the process ends with a session held: the socket
        // closes without LOGOUT. Count it, as `SessionGuard` would.
        if self.session.is_some() {
            imap::note_abandoned(&self.email);
        }
    }
}

#[derive(Default)]
struct Slot {
    held: Option<Held>,
}

/// Process-wide pool of one IMAP session per account. Use [`shared`].
pub struct BodyFetcher {
    slots: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<Slot>>>>,
    reaper: Once,
}

impl Default for BodyFetcher {
    fn default() -> Self {
        Self { slots: StdMutex::new(HashMap::new()), reaper: Once::new() }
    }
}

static SHARED: LazyLock<BodyFetcher> = LazyLock::new(BodyFetcher::default);

/// The fetcher every MCP session in this process shares. Per-session
/// fetchers would each hold their own connection, which is the cost this
/// module exists to cut.
pub fn shared() -> &'static BodyFetcher {
    &SHARED
}

impl BodyFetcher {
    fn slot(&self, account_id: &str) -> Arc<tokio::sync::Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.entry(account_id.to_string()).or_default().clone()
    }

    /// Fetch, parse and store one message's body. Must be called inside a
    /// Tokio runtime (the first call starts the idle-session reaper).
    pub async fn fetch(
        &'static self,
        db_path: &Path,
        account: &db::accounts::Account,
        folder: &str,
        uid: u32,
    ) -> Result<Fetched, AppError> {
        self.reaper.call_once(|| {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(REAP_EVERY).await;
                    self.reap(IDLE_TTL).await;
                }
            });
        });
        let slot = self.slot(&account.id);
        let mut slot = slot.lock().await;
        // Single flight: a request that waited on this account's lock finds
        // the body the one ahead of it stored.
        if cached(db_path, &account.id, folder, uid) {
            return Ok(Fetched::AlreadyCached);
        }
        let raw = fetch_raw(&mut slot, account, folder, uid).await?;
        // Still holding the account: a request queued for this same message
        // must find it stored, not fetch it again.
        let parsed = parse_and_store(db_path.to_path_buf(), account.id.clone(), folder.to_string(), uid, raw).await;
        drop(slot);
        Ok(Fetched::Parsed(Box::new(parsed?)))
    }

    /// Log out every session idle longer than `ttl`. A session in use is
    /// skipped (its lock is held), never waited for.
    pub async fn reap(&self, ttl: Duration) {
        let slots: Vec<_> = {
            let slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
            slots.values().cloned().collect()
        };
        for slot in slots {
            let Ok(mut slot) = slot.try_lock() else { continue };
            if slot.held.as_ref().is_some_and(|h| h.last_used.elapsed() >= ttl) {
                if let Some(held) = slot.held.take() {
                    held.close().await;
                }
            }
        }
    }

    /// Sessions currently held, for tests and diagnostics.
    pub fn held_count(&self) -> usize {
        let slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots
            .values()
            .filter(|s| s.try_lock().map(|s| s.held.is_some()).unwrap_or(true))
            .count()
    }
}

fn cached(db_path: &Path, account_id: &str, folder: &str, uid: u32) -> bool {
    body_store::open_for_write(db_path)
        .and_then(|c| body_store::is_fully_cached(&c, account_id, folder, uid))
        .unwrap_or(false)
}

/// Fetch on the held session when it is fresh; otherwise, or when the reused
/// session fails for any reason but a missing message, on a new one.
async fn fetch_raw(
    slot: &mut Slot,
    account: &db::accounts::Account,
    folder: &str,
    uid: u32,
) -> Result<Vec<u8>, AppError> {
    if let Some(mut held) = slot.held.take() {
        if held.email == account.email && held.last_used.elapsed() < IDLE_TTL {
            match fetch_on(&mut held, folder, uid).await {
                Ok(raw) => {
                    held.last_used = Instant::now();
                    slot.held = Some(held);
                    return Ok(raw);
                }
                // The session answered; the message is gone. Keep it.
                Err(e @ AppError::NotFound(_)) => {
                    held.last_used = Instant::now();
                    slot.held = Some(held);
                    return Err(e);
                }
                Err(e) => {
                    log::info!("body_fetch: held session for {} failed ({e}); reconnecting once", account.email);
                    held.close().await;
                }
            }
        } else {
            held.close().await;
        }
    }
    let session = imap::connect_for_account(account).await?;
    let mut held = Held { session: Some(session), email: account.email.clone(), folder: None, last_used: Instant::now() };
    match fetch_on(&mut held, folder, uid).await {
        Ok(raw) => {
            held.last_used = Instant::now();
            slot.held = Some(held);
            Ok(raw)
        }
        Err(e) => {
            held.close().await;
            Err(e)
        }
    }
}

async fn fetch_on(held: &mut Held, folder: &str, uid: u32) -> Result<Vec<u8>, AppError> {
    let session = held.session.as_mut().ok_or_else(|| AppError::Imap("session closed".into()))?;
    if held.folder.as_deref() != Some(folder) {
        held.folder = None;
        step(imap::select_folder(session, folder), "SELECT").await?;
        held.folder = Some(folder.to_string());
    }
    step(imap::fetch_body(session, uid), "FETCH").await
}

async fn step<T>(op: impl std::future::Future<Output = Result<T, AppError>>, what: &str) -> Result<T, AppError> {
    tokio::time::timeout(OP_TIMEOUT, op)
        .await
        .map_err(|_| AppError::Imap(format!("{what} timed out after {}s", OP_TIMEOUT.as_secs())))?
}

/// One fetch on its own session, for one-shot processes (the phone's
/// `cxmail-mcp remote` helpers), where holding a session past exit only
/// leaks it. Logged out on every path.
pub async fn fetch_once(account: &db::accounts::Account, folder: &str, uid: u32) -> Result<Vec<u8>, AppError> {
    let session = imap::connect_for_account(account).await?;
    let mut held = Held { session: Some(session), email: account.email.clone(), folder: None, last_used: Instant::now() };
    let result = fetch_on(&mut held, folder, uid).await;
    held.close().await;
    result
}

/// Parse off the async workers (attachment text extraction can take seconds)
/// and store. A failed store is logged, not returned: the caller still has a
/// good body to show.
pub async fn parse_and_store(
    db_path: std::path::PathBuf,
    account_id: String,
    folder: String,
    uid: u32,
    raw: Vec<u8>,
) -> Result<parser::ParsedMessage, AppError> {
    tokio::task::spawn_blocking(move || {
        let parsed = std::panic::catch_unwind(|| parser::parse_message(&raw))
            .map_err(|_| AppError::Parse("Message could not be parsed".into()))?;
        match body_store::open_for_write(&db_path)
            .and_then(|c| body_store::store_fetched_body(&c, &account_id, &folder, uid, &parsed))
        {
            Ok(_) => {}
            Err(e) => log::warn!("body_fetch: storing {folder}/{uid} failed: {e}"),
        }
        Ok(parsed)
    })
    .await
    .map_err(|e| AppError::Parse(format!("parse task failed: {e}")))?
}

/// Prefetch bodies for newly-arrived mail on a session the caller already
/// holds (the IDLE sync's), within `budget`. Costs no login. `uids` newest
/// first; at most `limit` uncached ones are fetched. Returns `false` when a
/// fetch timed out mid-response, so the caller knows the session is no longer
/// in a known state.
pub async fn prefetch_on_session(
    db: &Arc<StdMutex<rusqlite::Connection>>,
    session: &mut imap::ImapSession,
    account_id: &str,
    folder: &str,
    uids: &[u32],
    limit: usize,
    budget: Duration,
) -> bool {
    use crate::LockExt;
    let wanted: Vec<u32> = {
        let conn = db.safe_lock();
        uids.iter()
            .copied()
            .filter(|uid| !body_store::is_fully_cached(&conn, account_id, folder, *uid).unwrap_or(true))
            .take(limit)
            .collect()
    };
    if wanted.is_empty() {
        return true;
    }
    let started = Instant::now();
    let mut fetched = Vec::new();
    let mut healthy = true;
    for uid in wanted {
        let Some(left) = budget.checked_sub(started.elapsed()).filter(|d| !d.is_zero()) else { break };
        match tokio::time::timeout(left, imap::fetch_body(session, uid)).await {
            Ok(Ok(raw)) => fetched.push((uid, raw)),
            Ok(Err(e)) => log::warn!("IDLE body prefetch: UID {uid} in {folder} failed: {e}"),
            Err(_) => {
                log::warn!("IDLE body prefetch: UID {uid} in {folder} hit the time budget");
                healthy = false;
                break;
            }
        }
    }
    let count = fetched.len();
    let parsed = tokio::task::spawn_blocking(move || {
        fetched
            .into_iter()
            .filter_map(|(uid, raw)| {
                std::panic::catch_unwind(|| parser::parse_message(&raw)).ok().map(|p| (uid, p))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    {
        // Gotcha #11: the DB lock is taken only after every IMAP await.
        let conn = db.safe_lock();
        for (uid, p) in &parsed {
            if let Err(e) = body_store::store_fetched_body(&conn, account_id, folder, *uid, p) {
                log::warn!("IDLE body prefetch: storing UID {uid} failed: {e}");
            }
        }
    }
    log::info!("IDLE body prefetch: stored {} of {count} fetched for {folder}", parsed.len());
    healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::body_store;

    /// A file DB (the fetcher opens its own connections) whose account has a
    /// provider `connect_for_account` rejects before any network or Keychain
    /// access, so a test can tell "fetched" from "never tried".
    fn file_db(name: &str) -> (std::path::PathBuf, db::accounts::Account) {
        let path = std::env::temp_dir().join(format!("cxmail-body-fetch-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = body_store::open_for_write(&path).unwrap();
        db::schema::initialize(&c).unwrap();
        c.execute_batch(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host) VALUES ('a','me@example.net','offline','imap.example.net','smtp.example.net');
             INSERT INTO folders (account_id,name,folder_type) VALUES ('a','INBOX','inbox');
             INSERT INTO messages (account_id,folder_name,uid,from_email,date,message_id) VALUES ('a','INBOX',1,'news@example.org','2026-11-15T12:00:00Z','<one@example.org>');",
        )
        .unwrap();
        let account = db::accounts::get_by_id(&c, "a").unwrap().unwrap();
        (path, account)
    }

    #[tokio::test]
    async fn a_stored_message_is_served_without_touching_the_network() {
        let (path, account) = file_db("cached");
        let parsed = parser::parse_message(body_store::tests::RISKY.as_bytes());
        body_store::store_fetched_body(&body_store::open_for_write(&path).unwrap(), "a", "INBOX", 1, &parsed).unwrap();
        let fetcher: &'static BodyFetcher = Box::leak(Box::default());
        assert!(matches!(fetcher.fetch(&path, &account, "INBOX", 1).await, Ok(Fetched::AlreadyCached)));
        assert_eq!(fetcher.held_count(), 0, "no session was opened");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_body_only_row_is_refetched_and_a_failed_connect_holds_nothing() {
        let (path, account) = file_db("bodyonly");
        body_store::open_for_write(&path)
            .unwrap()
            .execute_batch("INSERT INTO message_bodies (account_id,folder_name,uid,plain_text) VALUES ('a','INBOX',1,'Hello');")
            .unwrap();
        let fetcher: &'static BodyFetcher = Box::leak(Box::default());
        let err = match fetcher.fetch(&path, &account, "INBOX", 1).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a body-only row must not count as cached"),
        };
        assert!(err.contains("Unsupported provider"), "it tried to fetch: {err}");
        assert_eq!(fetcher.held_count(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn reaping_an_empty_pool_is_a_no_op() {
        let fetcher: &'static BodyFetcher = Box::leak(Box::default());
        fetcher.reap(Duration::ZERO).await;
        assert_eq!(fetcher.held_count(), 0);
    }
}
