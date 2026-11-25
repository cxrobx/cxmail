//! One held IMAP session per account for the phone's requests: body fetches
//! for mail the desktop has not cached yet, and the small actions (read,
//! star, archive, move, trash).
//!
//! The phone opens messages the desktop never displayed, so their bodies are
//! not in SQLite, and every open used to log in to IMAP from scratch, more than
//! once. Every action logged in too, and the phone marks a message read each
//! time you leave it. Gmail caps simultaneous connections per account and
//! throttles logins from one IP (see the ledger notes in `imap.rs`), so logins
//! are the cost to minimise, not just latency.
//!
//! The rules here:
//! * **Store what you fetch.** Every fetch goes through
//!   [`body_store::store_fetched_body`], so a message costs one IMAP fetch in
//!   its lifetime and every later open reads SQLite.
//! * **One held session per account** ([`BodyFetcher`]) in the long-running
//!   MCP process, reused while requests keep coming and logged out
//!   [`IDLE_TTL`] after the last one, or [`MAX_AGE`] after its login. The
//!   per-account lock makes a racing preview and open share one fetch.
//! * **Every IMAP step has a timeout.** A step that times out or loses the
//!   connection leaves the stream mid-response, so that session is dropped,
//!   never reused (gotchas #10 and #46). A server refusal (NO/BAD) is an
//!   answer: the session stays.
//! * **Retry only what is safe to repeat.** A reused session that broke gets
//!   one retry on a fresh login, except after a COPY went out (the copy may
//!   have landed). STORE is idempotent, and a repeated UID MOVE of a message
//!   that already moved finds nothing to move.
//! * **Take the session out of its slot for the length of an operation.** If
//!   the request is cancelled mid-command, the session is dropped (closed and
//!   counted by `Held`'s `Drop`), never put back half-read.

use super::{body_store, imap, parser};
use crate::{db, error::AppError};
use async_imap::error::Error as ImapError;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, Once};
use std::time::{Duration, Instant};

/// How long a held session may sit unused before it is logged out.
pub const IDLE_TTL: Duration = Duration::from_secs(45);
/// How long a session may live however busy it is, so continuous use still
/// picks up a refreshed token or a revoked password within minutes.
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// One IMAP step. `connect_for_account` carries its own 15 s timeout.
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

/// A change to one message, run on the account's held session.
#[derive(Debug, Clone, Copy)]
pub enum MailAction<'a> {
    /// `UID STORE +FLAGS`/`-FLAGS`, e.g. `\Seen` or `\Flagged`.
    Flag { add: bool, flag: &'a str },
    /// `UID MOVE` to another folder (archive, trash and move all are this).
    Move { to: &'a str },
}

#[derive(Debug, Clone, Copy)]
enum Op<'a> {
    Fetch,
    Act(MailAction<'a>),
}

enum Out {
    Raw(Vec<u8>),
    Done,
}

/// Why an operation failed, which decides what happens to the session.
enum Failure {
    /// The server answered (NO/BAD, or no such message). The session is fine.
    Refused(AppError),
    /// The connection broke or timed out before anything unrepeatable went out.
    /// Drop the session; a reused one may retry once on a fresh login.
    Broken(AppError),
    /// Broke after a step that must not be repeated. Drop it, never retry.
    Final(AppError),
}

/// Everything `connect_for_account` reads off the account row, so editing a
/// generic IMAP account's host or login stops the old session being reused.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Fingerprint {
    provider: String,
    email: String,
    host: String,
    port: i32,
    security: String,
    username: Option<String>,
}

impl Fingerprint {
    fn of(a: &db::accounts::Account) -> Self {
        Self {
            provider: a.provider.clone(),
            email: a.email.clone(),
            host: a.imap_host.clone(),
            port: a.imap_port,
            security: a.imap_security.clone(),
            username: a.imap_username.clone(),
        }
    }
}

struct Held {
    session: Option<imap::ImapSession>,
    key: Fingerprint,
    folder: Option<String>,
    opened: Instant,
    last_used: Instant,
}

impl Held {
    fn new(session: imap::ImapSession, account: &db::accounts::Account) -> Self {
        let now = Instant::now();
        Self { session: Some(session), key: Fingerprint::of(account), folder: None, opened: now, last_used: now }
    }

    fn reusable_for(&self, account: &db::accounts::Account) -> bool {
        self.key == Fingerprint::of(account) && self.last_used.elapsed() < IDLE_TTL && self.opened.elapsed() < MAX_AGE
    }

    async fn close(mut self) {
        if let Some(session) = self.session.take() {
            let _ = imap::disconnect(session, &self.key.email).await;
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Reached when a broken session is dropped, a request is cancelled
        // mid-command, or the process ends with a session held: the socket
        // closes without LOGOUT. Count it, as `SessionGuard` would.
        if self.session.is_some() {
            imap::note_abandoned(&self.key.email);
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

    /// Start the idle-session reaper once. Every entry point that can open a
    /// session calls this, so a session opened by an action alone is reaped too.
    fn ensure_reaper(&'static self) {
        self.reaper.call_once(|| {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(REAP_EVERY).await;
                    self.reap(IDLE_TTL).await;
                }
            });
        });
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
        self.ensure_reaper();
        let slot = self.slot(&account.id);
        let mut slot = slot.lock().await;
        // Single flight: a request that waited on this account's lock finds
        // the body the one ahead of it stored.
        if cached(db_path, &account.id, folder, uid) {
            return Ok(Fetched::AlreadyCached);
        }
        let raw = match run_in_slot(&mut slot, account, folder, uid, Op::Fetch).await? {
            Out::Raw(raw) => raw,
            Out::Done => return Err(AppError::Imap("FETCH returned nothing".into())),
        };
        // Still holding the account: a request queued for this same message
        // must find it stored, not fetch it again.
        let parsed = parse_and_store(db_path.to_path_buf(), account.id.clone(), folder.to_string(), uid, raw).await;
        drop(slot);
        Ok(Fetched::Parsed(Box::new(parsed?)))
    }

    /// Run one action on the account's held session. Must be called inside a
    /// Tokio runtime.
    pub async fn act(
        &'static self,
        account: &db::accounts::Account,
        folder: &str,
        uid: u32,
        action: MailAction<'_>,
    ) -> Result<(), AppError> {
        self.ensure_reaper();
        let slot = self.slot(&account.id);
        let mut slot = slot.lock().await;
        run_in_slot(&mut slot, account, folder, uid, Op::Act(action)).await.map(|_| ())
    }

    /// Log out every session idle longer than `ttl` or older than
    /// [`MAX_AGE`]. A session in use is skipped (its lock is held), never
    /// waited for, and a slot's lock is released before its LOGOUT, which can
    /// take seconds on a stalled server.
    pub async fn reap(&self, ttl: Duration) {
        let slots: Vec<_> = {
            let slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
            slots.values().cloned().collect()
        };
        for slot in slots {
            let expired = {
                let Ok(mut slot) = slot.try_lock() else { continue };
                if slot.held.as_ref().is_some_and(|h| h.last_used.elapsed() >= ttl || h.opened.elapsed() >= MAX_AGE) {
                    slot.held.take()
                } else {
                    None
                }
            };
            if let Some(held) = expired {
                held.close().await;
            }
        }
    }

    /// Log out every idle session now. Call on a graceful exit: a `static`
    /// is never dropped, so without this they would close without LOGOUT.
    pub async fn close_all(&self) {
        self.reap(Duration::ZERO).await;
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

/// Run `op` on the held session when it is still good for this account;
/// otherwise, or when the reused session broke, on a new one.
async fn run_in_slot(
    slot: &mut Slot,
    account: &db::accounts::Account,
    folder: &str,
    uid: u32,
    op: Op<'_>,
) -> Result<Out, AppError> {
    if let Some(mut held) = slot.held.take() {
        if held.reusable_for(account) {
            match run_held(&mut held, folder, uid, op).await {
                Ok(out) => {
                    held.last_used = Instant::now();
                    slot.held = Some(held);
                    return Ok(out);
                }
                Err(Failure::Refused(e)) => {
                    held.last_used = Instant::now();
                    slot.held = Some(held);
                    return Err(e);
                }
                Err(Failure::Final(e)) => return Err(e),
                Err(Failure::Broken(e)) => {
                    // Dropped, not logged out: LOGOUT on a broken stream only
                    // adds a wait before the retry.
                    log::info!("body_fetch: held session for {} broke ({e}); reconnecting once", account.email);
                }
            }
        } else {
            tokio::spawn(held.close());
        }
    }
    let session = imap::connect_for_account(account).await?;
    let mut held = Held::new(session, account);
    match run_held(&mut held, folder, uid, op).await {
        Ok(out) => {
            held.last_used = Instant::now();
            slot.held = Some(held);
            Ok(out)
        }
        Err(Failure::Refused(e)) => {
            held.last_used = Instant::now();
            slot.held = Some(held);
            Err(e)
        }
        Err(Failure::Broken(e) | Failure::Final(e)) => Err(e),
    }
}

async fn run_held(held: &mut Held, folder: &str, uid: u32, op: Op<'_>) -> Result<Out, Failure> {
    let Some(session) = held.session.as_mut() else {
        return Err(Failure::Broken(AppError::Imap("session closed".into())));
    };
    run_on(session, &mut held.folder, folder, uid, op).await
}

/// One operation on a session whose selected mailbox is `selected`.
async fn run_on<S: imap::ImapIo>(
    session: &mut async_imap::Session<S>,
    selected: &mut Option<String>,
    folder: &str,
    uid: u32,
    op: Op<'_>,
) -> Result<Out, Failure> {
    let result = run_op(session, selected, folder, uid, op).await;
    // Untagged responses the library's helpers forward (EXPUNGE, EXISTS,
    // FLAGS) land in a bounded, lossy channel nobody reads; empty it so it
    // never sits full. A `* BYE` is caught by `run_to_completion` instead.
    while session.unsolicited_responses.try_recv().is_ok() {}
    result
}

async fn run_op<S: imap::ImapIo>(
    session: &mut async_imap::Session<S>,
    selected: &mut Option<String>,
    folder: &str,
    uid: u32,
    op: Op<'_>,
) -> Result<Out, Failure> {
    if selected.as_deref() != Some(folder) {
        *selected = None;
        // Read through the tagged completion: the library's `select` reports
        // success on a hang-up (gotcha #69).
        match timed(imap::select_checked(session, folder), "SELECT").await? {
            Ok(()) => *selected = Some(folder.to_string()),
            Err(e) => return Err(classify(e, || format!("SELECT {folder}"))),
        }
    }
    let out = match op {
        Op::Fetch => match timed(imap::fetch_body_checked(session, uid), "FETCH").await? {
            Ok(Some(raw)) => Out::Raw(raw),
            // The server answered OK with no body: the message is gone.
            Ok(None) => return Err(Failure::Refused(AppError::NotFound(format!("Message UID {uid} not found")))),
            Err(e) => return Err(classify(e, || format!("UID FETCH {uid}"))),
        },
        Op::Act(MailAction::Flag { add, flag }) => {
            let command = format!("UID STORE {} {}FLAGS ({})", uid, if add { "+" } else { "-" }, flag);
            match timed(imap::run_to_completion(session, &command, |_| {}), "STORE").await? {
                Ok(()) => Out::Done,
                Err(e) => return Err(classify(e, || command.clone())),
            }
        }
        Op::Act(MailAction::Move { to }) => match timed(move_checked(session, uid, to), "MOVE").await? {
            Ok(()) => Out::Done,
            // BAD = no MOVE on this server (see `imap::move_message`). From
            // the COPY on, nothing may be retried.
            Err(ImapError::Bad(resp)) => {
                log::warn!("UID MOVE answered BAD ({}); copying and removing instead", resp.trim());
                match tokio::time::timeout(OP_TIMEOUT, imap::copy_and_remove(session, uid, to)).await {
                    Ok(Ok(())) => Out::Done,
                    Ok(Err(e)) => return Err(Failure::Final(e)),
                    Err(_) => return Err(Failure::Final(AppError::Imap("COPY fallback timed out".into()))),
                }
            }
            Err(e) => return Err(classify(e, || format!("UID MOVE {uid} to {to}"))),
        },
    };
    Ok(out)
}

async fn move_checked<S: imap::ImapIo>(session: &mut async_imap::Session<S>, uid: u32, to: &str) -> Result<(), ImapError> {
    let command = format!("UID MOVE {uid} {}", imap::quote_mailbox(to)?);
    imap::run_to_completion(session, &command, |_| {}).await
}

/// A server's NO/BAD is an answer, and a name refused before sending
/// (`Validate`) never reached the wire; anything else means the stream is in
/// an unknown state.
fn classify(e: ImapError, what: impl Fn() -> String) -> Failure {
    match e {
        ImapError::No(_) | ImapError::Bad(_) | ImapError::Validate(_) => {
            Failure::Refused(AppError::Imap(format!("{} refused: {e}", what())))
        }
        _ => Failure::Broken(AppError::Imap(format!("{} failed: {e}", what()))),
    }
}

/// One IMAP step under [`OP_TIMEOUT`]; a timeout is a broken session.
async fn timed<T>(op: impl std::future::Future<Output = T>, what: &str) -> Result<T, Failure> {
    tokio::time::timeout(OP_TIMEOUT, op)
        .await
        .map_err(|_| Failure::Broken(AppError::Imap(format!("{what} timed out after {}s", OP_TIMEOUT.as_secs()))))
}

/// One fetch on its own session, for one-shot processes (the phone's
/// `cxmail-mcp remote` helpers), where holding a session past exit only
/// leaks it. Logged out on every path it can be.
pub async fn fetch_once(account: &db::accounts::Account, folder: &str, uid: u32) -> Result<Vec<u8>, AppError> {
    let session = imap::connect_for_account(account).await?;
    let mut held = Held::new(session, account);
    let result = run_held(&mut held, folder, uid, Op::Fetch).await;
    match result {
        Ok(Out::Raw(raw)) => {
            held.close().await;
            Ok(raw)
        }
        Ok(Out::Done) => {
            held.close().await;
            Err(AppError::Imap("FETCH returned nothing".into()))
        }
        Err(Failure::Refused(e)) => {
            held.close().await;
            Err(e)
        }
        // Broken mid-response: drop it (counted as abandoned), no LOGOUT wait.
        Err(Failure::Broken(e) | Failure::Final(e)) => Err(e),
    }
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

    fn held_for(account: &db::accounts::Account) -> Held {
        let now = Instant::now();
        Held { session: None, key: Fingerprint::of(account), folder: None, opened: now, last_used: now }
    }

    #[test]
    fn a_held_session_is_reused_only_for_the_same_login_while_young_and_recent() {
        let (path, account) = file_db("fingerprint");
        let mut held = held_for(&account);
        assert!(held.reusable_for(&account));

        let mut moved = account.clone();
        moved.imap_host = "imap.elsewhere.example".into();
        assert!(!held.reusable_for(&moved), "an edited host must not reuse the old login");
        let mut renamed = account.clone();
        renamed.imap_username = Some("someone-else".into());
        assert!(!held.reusable_for(&renamed), "an edited login name must not reuse it");

        if let Some(t) = Instant::now().checked_sub(IDLE_TTL) {
            held.last_used = t;
            assert!(!held.reusable_for(&account), "idle past the TTL");
            held.last_used = Instant::now();
        }
        if let Some(t) = Instant::now().checked_sub(MAX_AGE) {
            held.opened = t;
            assert!(!held.reusable_for(&account), "busy, but past the max age");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_refusal_keeps_the_session_and_anything_else_drops_it() {
        assert!(matches!(classify(ImapError::No("[TRYCREATE]".into()), || "MOVE".into()), Failure::Refused(_)));
        assert!(matches!(classify(ImapError::Bad("unknown command".into()), || "MOVE".into()), Failure::Refused(_)));
        assert!(matches!(classify(ImapError::ConnectionLost, || "MOVE".into()), Failure::Broken(_)));
        let io = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone");
        assert!(matches!(classify(ImapError::Io(io), || "STORE".into()), Failure::Broken(_)));
    }

    #[tokio::test]
    async fn an_action_alone_starts_the_reaper_and_a_failed_login_holds_nothing() {
        let (path, account) = file_db("act");
        let fetcher: &'static BodyFetcher = Box::leak(Box::default());
        let err = fetcher
            .act(&account, "INBOX", 1, MailAction::Flag { add: true, flag: "\\Seen" })
            .await
            .expect_err("the offline provider cannot log in");
        assert!(err.to_string().contains("Unsupported provider"), "{err}");
        assert!(fetcher.reaper.is_completed(), "a session opened by an action must be reaped too");
        assert_eq!(fetcher.held_count(), 0);
        let _ = std::fs::remove_file(&path);
    }

    /// A scripted IMAP server over an in-memory pipe. `reply` gets each
    /// command after its tag and returns the lines to send, `{tag}` standing
    /// for the command's tag; a reply of `["CLOSE"]` hangs up. Every command
    /// is logged for the assertions.
    async fn wire(
        reply: impl Fn(&str) -> Vec<String> + Send + 'static,
    ) -> (async_imap::Session<tokio_util::compat::Compat<tokio::io::DuplexStream>>, Arc<StdMutex<Vec<String>>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        use tokio_util::compat::TokioAsyncReadCompatExt;
        let (client, server) = tokio::io::duplex(1 << 16);
        let log = Arc::new(StdMutex::new(Vec::new()));
        let seen = log.clone();
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(server);
            let mut lines = tokio::io::BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let (tag, cmd) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                seen.lock().unwrap().push(cmd.to_string());
                let out = if cmd.starts_with("LOGIN") { vec![format!("{tag} OK logged in")] } else { reply(cmd) };
                if out == ["CLOSE"] {
                    break;
                }
                for l in out {
                    w.write_all(l.replace("{tag}", tag).as_bytes()).await.unwrap();
                    w.write_all(b"\r\n").await.unwrap();
                }
            }
        });
        let session = async_imap::Client::new(client.compat()).login("u", "p").await.map_err(|(e, _)| e).unwrap();
        (session, log)
    }

    fn selected(cmd: &str) -> Option<Vec<String>> {
        cmd.starts_with("SELECT").then(|| {
            vec!["* 3 EXISTS".into(), "* OK [UIDVALIDITY 1] ok".into(), "{tag} OK [READ-WRITE] SELECT done".into()]
        })
    }

    fn sent(log: &Arc<StdMutex<Vec<String>>>, prefix: &str) -> usize {
        log.lock().unwrap().iter().filter(|c| c.starts_with(prefix)).count()
    }

    const RAW: &str = "Subject: hi\r\n\r\nhello";

    #[tokio::test]
    async fn a_fetch_reads_past_another_messages_flag_update_and_leaves_the_session_clean() {
        let (mut session, log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| {
                if cmd.starts_with("UID FETCH 42") {
                    // Another client marked message 2 read: the server says so
                    // first, and again about message 3 after the body.
                    vec![
                        "* 2 FETCH (FLAGS (\\Seen))".into(),
                        format!("* 1 FETCH (UID 42 BODY[] {{{}}}\r\n{RAW})", RAW.len()),
                        "* 3 FETCH (FLAGS ())".into(),
                        "{tag} OK FETCH done".into(),
                    ]
                } else if cmd.starts_with("UID STORE 42") {
                    vec!["{tag} OK STORE done".into()]
                } else {
                    vec!["{tag} BAD unexpected".into()]
                }
            })
        })
        .await;
        let mut folder = None;
        let raw = match run_on(&mut session, &mut folder, "INBOX", 42, Op::Fetch).await {
            Ok(Out::Raw(raw)) => raw,
            Ok(Out::Done) => panic!("no body"),
            Err(Failure::Refused(e) | Failure::Broken(e) | Failure::Final(e)) => panic!("fetch failed: {e}"),
        };
        assert_eq!(raw, RAW.as_bytes());
        let flagged = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Flag { add: true, flag: "\\Seen" })).await;
        assert!(matches!(flagged, Ok(Out::Done)), "the next command on the same session must succeed");
        assert_eq!(sent(&log, "SELECT"), 1, "the held folder is not re-selected");
    }

    #[tokio::test]
    async fn a_fetch_or_select_on_a_dead_connection_is_broken_never_not_found() {
        let (mut session, _log) = wire(|cmd| selected(cmd).unwrap_or_else(|| vec!["CLOSE".into()])).await;
        let mut folder = None;
        let fetched = run_on(&mut session, &mut folder, "INBOX", 42, Op::Fetch).await;
        assert!(matches!(fetched, Err(Failure::Broken(_))), "a hang-up mid-FETCH must drop the session and retry");

        let (mut session, _log) = wire(|_| vec!["CLOSE".into()]).await;
        let mut folder = None;
        let fetched = run_on(&mut session, &mut folder, "INBOX", 42, Op::Fetch).await;
        assert!(matches!(fetched, Err(Failure::Broken(_))), "a hang-up on SELECT is not a selected folder");
        assert_eq!(folder, None);

        let (mut session, _log) =
            wire(|cmd| selected(cmd).unwrap_or_else(|| vec!["* BYE going away".into(), "{tag} OK done".into()])).await;
        let mut folder = None;
        let flagged = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Flag { add: true, flag: "\\Seen" })).await;
        assert!(matches!(flagged, Err(Failure::Broken(_))), "after * BYE the session is finished");
    }

    #[tokio::test]
    async fn a_refused_fetch_is_a_refusal_not_a_missing_message() {
        let (mut session, _log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| vec!["{tag} NO [UNAVAILABLE] try later".into()])
        })
        .await;
        let mut folder = None;
        match run_on(&mut session, &mut folder, "INBOX", 42, Op::Fetch).await {
            Err(Failure::Refused(e)) => assert!(!matches!(e, AppError::NotFound(_)), "{e}"),
            _ => panic!("a NO is a refusal"),
        }
        let (mut session, _log) =
            wire(|cmd| selected(cmd).unwrap_or_else(|| vec!["{tag} OK FETCH done".into()])).await;
        let mut folder = None;
        match run_on(&mut session, &mut folder, "INBOX", 42, Op::Fetch).await {
            Err(Failure::Refused(AppError::NotFound(_))) => {}
            _ => panic!("OK with no body means the message is gone"),
        }
    }

    #[tokio::test]
    async fn an_untagged_no_during_select_is_read_past_to_the_tagged_answer() {
        let (mut session, log) = wire(|cmd| {
            if cmd.starts_with("SELECT") {
                vec!["* NO [ALERT] maintenance tonight".into(), "* 3 EXISTS".into(), "{tag} OK [READ-WRITE] done".into()]
            } else if cmd.starts_with("UID STORE") {
                vec!["{tag} OK".into()]
            } else {
                vec!["{tag} BAD unexpected".into()]
            }
        })
        .await;
        let mut folder = None;
        let flagged = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Flag { add: true, flag: "\\Seen" })).await;
        assert!(matches!(flagged, Ok(Out::Done)));
        assert_eq!(folder.as_deref(), Some("INBOX"));
        assert_eq!(sent(&log, "SELECT \"INBOX\""), 1);
    }

    #[tokio::test]
    async fn the_copy_fallback_quotes_a_folder_name_with_a_space() {
        let (mut session, log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| {
                if cmd.starts_with("UID MOVE") { vec!["{tag} BAD unknown command".into()] } else { vec!["{tag} OK".into()] }
            })
        })
        .await;
        let mut folder = None;
        let moved =
            run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Move { to: "Deleted Messages" })).await;
        assert!(matches!(moved, Ok(Out::Done)));
        assert_eq!(sent(&log, "UID COPY 42 \"Deleted Messages\""), 1, "{:?}", log.lock().unwrap());
        assert_eq!(imap::quote_mailbox("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
        assert!(imap::quote_mailbox("evil\r\nA1 LOGOUT").is_err());
    }

    #[tokio::test]
    async fn a_refused_move_is_an_answer_and_never_falls_through_to_a_copy() {
        let (mut session, log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| {
                if cmd.starts_with("UID MOVE") {
                    vec!["{tag} NO [TRYCREATE] no such mailbox".into()]
                } else {
                    vec!["{tag} OK".into()]
                }
            })
        })
        .await;
        let mut folder = None;
        let moved = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Move { to: "Archive" })).await;
        assert!(matches!(moved, Err(Failure::Refused(_))), "a NO keeps the session and is not retried");
        assert!(imap::move_message(&mut session, 42, "Archive").await.is_err());
        assert_eq!(sent(&log, "UID COPY"), 0, "a refusal must never become COPY + delete");
    }

    #[tokio::test]
    async fn a_server_without_move_copies_then_expunges_only_that_message() {
        let (mut session, log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| {
                if cmd.starts_with("UID MOVE") {
                    vec!["{tag} BAD unknown command".into()]
                } else {
                    vec!["{tag} OK".into()]
                }
            })
        })
        .await;
        let mut folder = None;
        let moved = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Move { to: "Archive" })).await;
        assert!(matches!(moved, Ok(Out::Done)));
        assert_eq!(sent(&log, "UID COPY 42"), 1);
        assert_eq!(sent(&log, "UID STORE 42 +FLAGS (\\Deleted)"), 1);
        assert_eq!(sent(&log, "UID EXPUNGE 42"), 1);
        assert_eq!(sent(&log, "EXPUNGE"), 0, "never the mailbox-wide form, which purges other clients' deletions");
    }

    #[tokio::test]
    async fn a_failure_after_the_copy_is_final_so_the_copy_is_never_repeated() {
        let (mut session, _log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| {
                if cmd.starts_with("UID MOVE") {
                    vec!["{tag} BAD unknown command".into()]
                } else if cmd.starts_with("UID COPY") {
                    vec!["{tag} OK copied".into()]
                } else {
                    vec!["CLOSE".into()]
                }
            })
        })
        .await;
        let mut folder = None;
        let moved = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Move { to: "Archive" })).await;
        assert!(matches!(moved, Err(Failure::Final(_))), "after a COPY went out, no retry");
    }

    #[tokio::test]
    async fn a_dropped_connection_is_broken_and_a_refused_expunge_is_an_error() {
        let (mut session, _log) = wire(|cmd| {
            selected(cmd).unwrap_or_else(|| if cmd.starts_with("UID STORE") { vec!["CLOSE".into()] } else { vec![] })
        })
        .await;
        let mut folder = None;
        let flagged = run_on(&mut session, &mut folder, "INBOX", 42, Op::Act(MailAction::Flag { add: true, flag: "\\Seen" })).await;
        assert!(matches!(flagged, Err(Failure::Broken(_))), "a hang-up drops the session");

        let (mut session, _log) = wire(|_| vec!["{tag} NO [CANNOT] expunge refused".into()]).await;
        assert!(imap::expunge(&mut session).await.is_err(), "a refused EXPUNGE must not read as success");
    }

    #[tokio::test]
    async fn reaping_an_empty_pool_is_a_no_op() {
        let fetcher: &'static BodyFetcher = Box::leak(Box::default());
        fetcher.reap(Duration::ZERO).await;
        assert_eq!(fetcher.held_count(), 0);
    }
}
