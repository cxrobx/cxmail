//! Live draft updates across machines (T666).
//!
//! An agent edit made through `cxmail-mcp` on the SAME Mac reaches an open
//! compose window through the bridge (`mcp-activity`, kind `draft-updated`).
//! An edit made on ANOTHER Mac never did: each install has its own cache, and
//! nothing told this one that the server's Drafts folder had moved on.
//!
//! This module watches the Drafts folder while a compose window here holds a
//! saved draft. One IMAP connection per account, logged in once and shared by
//! every compose window of that account (leases, refcounted), closed when the
//! last one goes. It asks the server for news with a NOOP every 3 s on that
//! held session. Not a fresh connection per check: that would be ~900 logins
//! an hour per window, which is what Gmail throttles, and a throttled account
//! loses INBOX sync and send with it. Not IDLE either: measured on Gmail
//! (2026-11-13), IDLE reported another session's APPEND to Drafts ~30 s late,
//! every time, where a NOOP on the held session saw it in ~1 s.
//!
//! When the server shows a newer revision of a draft than this Mac last knew,
//! the watcher syncs Drafts into the cache and emits the same `mcp-activity`
//! `draft-updated` envelope the bridge does, with `old_uid` = the revision this
//! Mac knew. The compose window's existing listener does the rest: adopt the
//! new UID, then reload if it has no unsaved edits or show the "updated
//! elsewhere" banner if it does. Revisions this Mac wrote itself are not
//! announced (`note_own_revision`, plus the write claim held while a save is
//! in flight).
//!
//! The write paths do not depend on this: `edit_draft` asks the server before
//! every APPEND (`email::draft_revisions`), so a stale window that never heard
//! about an edit still cannot autosave over it.

use crate::db;
use crate::email::{draft_revisions, imap};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use cxmail_core::bridge::Envelope;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

/// How often the held session asks the server for news (one NOOP).
const POLL_EVERY: Duration = Duration::from_secs(3);
/// Bound freshness by an explicit server read, not unsolicited notices.
/// The real two-Mac T666 check saw NOOP notices lag 9–13 seconds on Gmail.
/// Draft folders are small; read their identity headers on every poll even
/// when the server's unsolicited queue is silent.
const RESCAN_EVERY: Duration = POLL_EVERY;
/// Any one command on the held session must answer quickly or the socket is dead.
const IDLE_OP_TIMEOUT: Duration = Duration::from_secs(15);
/// A session that stayed up this long resets the failure count.
const SESSION_HEALTHY_AFTER: Duration = Duration::from_secs(4 * 60);
/// The `tool` on the envelopes this module emits, so the toast can say where
/// the edit came from.
pub const TOOL: &str = "another-device";

struct StopSignal {
    stopped: AtomicBool,
    notify: Notify,
}

impl StopSignal {
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        // `notify_one` stores a permit, so a stop that lands between two
        // waits is not lost.
        self.notify.notify_one();
    }
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

struct Watch {
    leases: HashSet<u64>,
    stop: Arc<StopSignal>,
}

static WATCHES: LazyLock<Mutex<HashMap<String, Watch>>> = LazyLock::new(Default::default);
static LEASE_ACCOUNT: LazyLock<Mutex<HashMap<u64, String>>> = LazyLock::new(Default::default);
static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);

/// Revisions this process APPENDed: `(account_id, folder, uid)`. Bounded; it
/// only has to outlive the gap between an APPEND and the watcher's next scan.
static OWN: LazyLock<Mutex<HashSet<(String, String, u32)>>> = LazyLock::new(Default::default);
const OWN_CAP: usize = 512;

/// Record a revision this Mac just wrote, so the watcher does not announce it
/// as an edit from elsewhere. Call right after the APPEND's UID is known.
pub fn note_own_revision(account_id: &str, folder: &str, uid: u32) {
    let mut own = OWN.safe_lock();
    if own.len() >= OWN_CAP {
        own.clear();
    }
    own.insert((account_id.to_string(), folder.to_string(), uid));
}

fn is_own(account_id: &str, folder: &str, uid: u32) -> bool {
    OWN.safe_lock()
        .contains(&(account_id.to_string(), folder.to_string(), uid))
}

/// One draft whose newest server revision moved past what this Mac knew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub draft_id: String,
    pub old_uid: u32,
    pub new_uid: u32,
}

/// What a scan means, given what this watcher last saw (`baseline`, updated in
/// place) and this Mac's belief for drafts it has not seen yet (`known`, the
/// `drafts` row's current UID). A newer revision is announced unless this Mac
/// wrote it (`own`) or is mid-write on that draft (`claimed`): the latter is
/// left out of the baseline so the next scan looks again, by which time the
/// writer has recorded its UID as its own.
pub(crate) fn diff(
    baseline: &mut HashMap<String, u32>,
    scan: &draft_revisions::DraftIndex,
    known: impl Fn(&str) -> Option<u32>,
    own: impl Fn(u32) -> bool,
    claimed: impl Fn(&str) -> bool,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for (draft_id, uids) in scan {
        let Some(&newest) = uids.last() else { continue };
        let prev = baseline.get(draft_id).copied().or_else(|| known(draft_id));
        match prev {
            Some(prev) if prev < newest => {
                if own(newest) {
                    baseline.insert(draft_id.clone(), newest);
                } else if claimed(draft_id) {
                    // Decide on the next scan.
                } else {
                    changes.push(Change {
                        draft_id: draft_id.clone(),
                        old_uid: prev,
                        new_uid: newest,
                    });
                    baseline.insert(draft_id.clone(), newest);
                }
            }
            // Nothing known yet (a draft another machine started), or already
            // current: just remember it. No window here can hold it stale.
            _ => {
                baseline.insert(draft_id.clone(), newest);
            }
        }
    }
    changes
}

/// Start (or join) the Drafts watcher for `account_id`. Returns a lease the
/// compose window hands back to `draft_watch_stop` when it closes.
#[tauri::command]
pub async fn draft_watch_start(app: AppHandle, account_id: String) -> Result<u64, AppError> {
    let account = {
        let state = app.state::<AppState>();
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };
    if !account.is_active {
        return Err(AppError::General("Account is not active".to_string()));
    }
    let lease = NEXT_LEASE.fetch_add(1, Ordering::SeqCst);
    LEASE_ACCOUNT.safe_lock().insert(lease, account_id.clone());
    let spawn = {
        let mut watches = WATCHES.safe_lock();
        match watches.get_mut(&account_id) {
            Some(w) if !w.stop.is_stopped() => {
                w.leases.insert(lease);
                None
            }
            _ => {
                let stop = Arc::new(StopSignal {
                    stopped: AtomicBool::new(false),
                    notify: Notify::new(),
                });
                watches.insert(
                    account_id.clone(),
                    Watch {
                        leases: HashSet::from([lease]),
                        stop: stop.clone(),
                    },
                );
                Some(stop)
            }
        }
    };
    if let Some(stop) = spawn {
        log::info!("draft watch: starting for {}", account.email);
        tauri::async_runtime::spawn(watch_loop(app, account, stop));
    }
    Ok(lease)
}

/// Give a lease back. The watcher stops when its account has none left.
/// Unknown leases are ignored (a window closing twice, a reload).
#[tauri::command]
pub async fn draft_watch_stop(lease: u64) -> Result<(), AppError> {
    let Some(account_id) = LEASE_ACCOUNT.safe_lock().remove(&lease) else {
        return Ok(());
    };
    let mut watches = WATCHES.safe_lock();
    if let Some(w) = watches.get_mut(&account_id) {
        w.leases.remove(&lease);
        if w.leases.is_empty() {
            w.stop.stop();
            watches.remove(&account_id);
        }
    }
    Ok(())
}

async fn watch_loop(app: AppHandle, account: db::accounts::Account, stop: Arc<StopSignal>) {
    let mut consecutive_failures: u32 = 0;
    let mut baseline: HashMap<String, u32> = HashMap::new();
    let mut generation: Option<u32> = None;
    while !stop.is_stopped() {
        let started = std::time::Instant::now();
        match run_session(&app, &account, &stop, &mut baseline, &mut generation).await {
            Ok(()) => break,
            Err(e) => {
                let ran_for = started.elapsed();
                consecutive_failures = if ran_for >= SESSION_HEALTHY_AFTER {
                    1
                } else {
                    consecutive_failures.saturating_add(1)
                };
                // The INBOX watcher's schedule, so both back off a refused
                // account together.
                let delay = imap::connect_backoff(consecutive_failures);
                log::warn!(
                    "draft watch: {} lost after {}s: {e}. Retrying in {}s (failure #{consecutive_failures})",
                    account.email,
                    ran_for.as_secs(),
                    delay.as_secs()
                );
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = stop.notify.notified() => {}
                }
            }
        }
    }
    log::info!("draft watch: stopped for {}", account.email);
}

async fn run_session(
    app: &AppHandle,
    account: &db::accounts::Account,
    stop: &StopSignal,
    baseline: &mut HashMap<String, u32>,
    generation: &mut Option<u32>,
) -> Result<(), AppError> {
    let email = account.email.as_str();
    let folder = {
        let state = app.state::<AppState>();
        let conn = state.db.safe_lock();
        db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts")
    };
    let mut session = imap::connect_for_account(account).await?;
    let mut accounting = imap::SessionGuard::new(email);

    loop {
        if stop.is_stopped() {
            accounting.disarm();
            let _ = imap::disconnect(session, email).await;
            return Ok(());
        }

        // Scan, then decide, then sync, then announce. The order matters: the
        // Drafts sync re-links `drafts` rows as it goes, so the comparison has
        // to read them first; and the window reloads from the cache, so the
        // new revision has to be in it before the event goes out.
        let scan = draft_revisions::scan(&mut session, &folder).await?;
        if *generation != Some(scan.uidvalidity) {
            baseline.clear();
            *generation = Some(scan.uidvalidity);
        }
        let changes = {
            let state = app.state::<AppState>();
            let conn = state.db.safe_lock();
            diff(
                baseline,
                &scan.drafts,
                |id| {
                    db::drafts::get(&conn, id)
                        .ok()
                        .flatten()
                        .filter(|r| r.folder_name == folder && r.uidvalidity == scan.uidvalidity)
                        .map(|r| r.current_uid)
                },
                |uid| is_own(&account.id, &folder, uid),
                |id| db::drafts::claim_active(&conn, id).unwrap_or(false),
            )
        };
        if !changes.is_empty() {
            let state = app.state::<AppState>();
            crate::idle::mini_sync(&state, app, &mut session, &account.id, email, &folder).await;
            {
                let conn = state.db.safe_lock();
                draft_revisions::link_scan(&conn, &account.id, &folder, &scan);
            }
            for c in &changes {
                log::info!(
                    "draft watch: draft {} moved {} -> {} on the server ({})",
                    c.draft_id, c.old_uid, c.new_uid, email
                );
                let env = Envelope::new("draft-updated", &account.id, &folder, c.new_uid)
                    .with_old_uid(c.old_uid)
                    .with_tool(TOOL);
                if let Err(e) = app.emit("mcp-activity", &env) {
                    log::warn!("draft watch: emit failed: {e}");
                }
            }
            // mini_sync leaves Drafts selected; the NOOP below relies on that.
        }

        // Wait for a change. A NOOP on this held session, not IDLE: measured
        // on Gmail 2026-11-13, IDLE delivered another session's APPEND to
        // Drafts ~30 s late every time (batched), while a NOOP saw it ~1 s
        // after the APPEND. Same single login either way, so nothing here
        // adds connections for Gmail to throttle.
        // The scan's own SELECT (and the sync's) queue EXISTS notices; they
        // are not news, and leaving them would rescan every cycle.
        let _ = drain_changed(&mut session);
        let mut since_scan = Duration::ZERO;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(POLL_EVERY) => {}
                _ = stop.notify.notified() => {}
            }
            if stop.is_stopped() {
                break;
            }
            since_scan += POLL_EVERY;
            tokio::time::timeout(IDLE_OP_TIMEOUT, session.noop())
                .await
                .map_err(|_| AppError::Imap("Drafts NOOP timed out".to_string()))?
                .map_err(|e| AppError::Imap(format!("Drafts NOOP failed: {e}")))?;
            if drain_changed(&mut session) || since_scan >= RESCAN_EVERY {
                break;
            }
        }
    }
}

/// Did the server report the selected folder changing (a revision appended or
/// expunged) since the last call? Drains the unsolicited queue either way.
fn drain_changed(session: &mut imap::ImapSession) -> bool {
    use async_imap::types::UnsolicitedResponse as U;
    let mut changed = false;
    while let Ok(r) = session.unsolicited_responses.try_recv() {
        if matches!(r, U::Exists(_) | U::Expunge(_) | U::Recent(_)) {
            changed = true;
        }
    }
    changed
}


#[cfg(test)]
mod tests {
    use super::*;

    fn index(entries: &[(&str, &[u32])]) -> draft_revisions::DraftIndex {
        entries
            .iter()
            .map(|(id, uids)| (id.to_string(), uids.to_vec()))
            .collect()
    }

    #[test]
    fn a_revision_from_elsewhere_is_announced_once() {
        // The 2026-11-13 case seen from Mac B: it knew 1127, Mac A
        // wrote 1129.
        let mut base = HashMap::new();
        let scan = index(&[("d", &[1129])]);
        let known = |_: &str| Some(1127);
        let ch = diff(&mut base, &scan, known, |_| false, |_| false);
        assert_eq!(
            ch,
            vec![Change { draft_id: "d".into(), old_uid: 1127, new_uid: 1129 }]
        );
        // The next scan of the same folder says nothing new.
        assert!(diff(&mut base, &scan, known, |_| false, |_| false).is_empty());
        // A further edit is announced from the revision last announced.
        let scan = index(&[("d", &[1131])]);
        let ch = diff(&mut base, &scan, known, |_| false, |_| false);
        assert_eq!(ch[0].old_uid, 1129);
        assert_eq!(ch[0].new_uid, 1131);
    }

    #[test]
    fn this_macs_own_save_is_not_announced() {
        let mut base = HashMap::new();
        let scan = index(&[("d", &[11])]);
        assert!(diff(&mut base, &scan, |_| Some(10), |u| u == 11, |_| false).is_empty());
        assert_eq!(base.get("d"), Some(&11));
    }

    #[test]
    fn a_draft_mid_save_here_is_decided_on_the_next_scan() {
        let mut base = HashMap::new();
        let scan = index(&[("d", &[11])]);
        // Claim held and the UID not yet recorded: say nothing, remember nothing.
        assert!(diff(&mut base, &scan, |_| Some(10), |_| false, |_| true).is_empty());
        assert!(!base.contains_key("d"));
        // Next scan: the writer has recorded it as its own.
        assert!(diff(&mut base, &scan, |_| Some(10), |u| u == 11, |_| false).is_empty());
    }

    #[test]
    fn unknown_or_current_drafts_are_only_remembered() {
        let mut base = HashMap::new();
        let scan = index(&[("new", &[5]), ("same", &[7])]);
        let known = |id: &str| (id == "same").then_some(7);
        assert!(diff(&mut base, &scan, known, |_| false, |_| false).is_empty());
        assert_eq!(base.get("new"), Some(&5));
        assert_eq!(base.get("same"), Some(&7));
    }

    #[test]
    fn leases_share_one_watch_and_the_last_one_stops_it() {
        // Registry bookkeeping without IMAP: simulate two leases on one account.
        let stop = Arc::new(StopSignal { stopped: AtomicBool::new(false), notify: Notify::new() });
        WATCHES.safe_lock().insert(
            "acct-test".into(),
            Watch { leases: HashSet::from([900_001, 900_002]), stop: stop.clone() },
        );
        LEASE_ACCOUNT.safe_lock().insert(900_001, "acct-test".into());
        LEASE_ACCOUNT.safe_lock().insert(900_002, "acct-test".into());
        tauri::async_runtime::block_on(draft_watch_stop(900_001)).unwrap();
        assert!(!stop.is_stopped(), "one lease left keeps it running");
        tauri::async_runtime::block_on(draft_watch_stop(900_002)).unwrap();
        assert!(stop.is_stopped(), "the last lease stops it");
        assert!(!WATCHES.safe_lock().contains_key("acct-test"));
        // A stale lease is a no-op.
        tauri::async_runtime::block_on(draft_watch_stop(900_002)).unwrap();
    }
}
