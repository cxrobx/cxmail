use crate::db;
use crate::email::imap;
use crate::error::AppError;
use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::Connection;
use std::collections::HashSet;
use std::time::Duration;

const FETCH_BATCH_SIZE: u32 = 250;
const LIMITED_FLAG_WINDOW: u32 = 250;
const FULL_EXISTENCE_WINDOW: u32 = 500;
const LIMITED_FLAG_RECONCILE_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    Full,
    Limited,
}

#[derive(Debug)]
pub struct PreparedFolderSync {
    pub sync_state: db::sync_state::SyncStateRow,
    pub existing_uids: HashSet<u32>,
}

#[derive(Debug, Clone)]
pub struct SelectedPlan {
    pub uidvalidity: u32,
    pub uidnext: u32,
    pub initial_last_uid: u32,
    pub reset_required: bool,
    pub batches: Vec<(u32, u32)>,
}

#[derive(Debug)]
pub struct ReconciliationPlan {
    reconcile_flags: bool,
    flag_uids: Vec<u32>,
    reconcile_existence: bool,
    existence_uids: Vec<u32>,
}

#[derive(Debug)]
pub struct FetchedReconciliation {
    unseen_uids: Option<HashSet<u32>>,
    flags: Vec<imap::ImapMessageFlags>,
    existing_on_server: HashSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncPlan {
    reset_required: bool,
    initial_last_uid: u32,
    batches: Vec<(u32, u32)>,
}

pub fn prepare_folder_sync(
    conn: &Connection,
    account_id: &str,
    folder: &str,
) -> Result<PreparedFolderSync, AppError> {
    let sync_state = db::sync_state::seed_if_missing_from_local_max(conn, account_id, folder)?;
    let existing_uids = db::messages::get_uids_for_folder(conn, account_id, folder)?
        .into_iter()
        .collect();

    Ok(PreparedFolderSync {
        sync_state,
        existing_uids,
    })
}

/// Select the folder on the IMAP server and compute the fetch plan from the
/// current `sync_state`. Pure IMAP + planning — does not touch SQLite.
pub async fn select_and_plan(
    session: &mut imap::ImapSession,
    folder: &str,
    sync_state: &db::sync_state::SyncStateRow,
) -> Result<SelectedPlan, AppError> {
    let (uidvalidity, uidnext) = imap::select_folder(session, folder).await?;
    let plan = build_sync_plan(
        sync_state.uidvalidity,
        sync_state.last_uid,
        uidvalidity,
        uidnext,
    );
    Ok(SelectedPlan {
        uidvalidity,
        uidnext,
        initial_last_uid: plan.initial_last_uid,
        reset_required: plan.reset_required,
        batches: plan.batches,
    })
}

/// If `plan.reset_required`, wipe the folder cache and reset sync_state.
/// Returns `true` when a reset occurred so callers can also clear their
/// in-memory `existing_uids` snapshot. Note: in-memory clearing is the
/// caller's responsibility — a future caller that forgets it will compute
/// `new_count` against pre-reset UIDs and report 0 new for re-inserted
/// messages.
pub fn apply_reset_if_needed(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    plan: &SelectedPlan,
) -> Result<bool, AppError> {
    if plan.reset_required {
        db::messages::delete_folder_cache(conn, account_id, folder)?;
        db::sync_state::reset_for_uidvalidity_change(conn, account_id, folder)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Insert one batch of fetched headers (INSERT OR IGNORE — never clobbers an
/// existing row) and advance `sync_state.last_uid` to `batch_end_uid`. We use
/// the batch's end UID rather than `headers.last().uid` so that empty/sparse
/// batches still move the floor — otherwise a dead range (e.g. all UIDs
/// expunged) would force every subsequent sync to re-scan it.
///
/// `upsert_success` also clears `last_error` / `last_error_at` and bumps
/// `last_synced`, so each successful batch advertises forward progress even
/// when an outer per-account timeout later cancels the loop.
/// Classify new message headers and update their category in the database.
///
/// Called by `apply_header_batch`, which is the ONLY way a message row is
/// created, so a message cannot exist without having been classified. It used
/// to live in `commands/messages.rs` and be called by hand after each insert —
/// and only the app's full-sync path remembered. `idle.rs` and `bin/helper.rs`
/// insert through the same batch function and did not, so every message that
/// arrived by IDLE — which is most of them, since the helper is a KeepAlive
/// LaunchAgent running whether the app is open or not — was stored with
/// `category = NULL`. `COALESCE(category, 'primary')` then showed all of it in
/// Primary: 11,943 inbox messages on the live mailbox, 58.6% of the last 45
/// days. Folding it into the insert is what stops that returning (gotcha #36:
/// one matcher, not three call sites that must remember).
///
/// Pipeline per header:
///   1. Sender override (user-configured per-sender category)
///   2. Heuristic classifier (`email::categorize`)
///   3. User mail_rules — can override category via `set_category` action
///      or apply `mark_read` / `mark_flagged` side-effects.
pub fn classify_headers(
    conn: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
    headers: &[imap::ImapMessageHeader],
) {
    // Load rules once per sync batch. Filter to active rules that either
    // match this account or are global (account_id = None).
    let rules: Vec<db::rules::MailRule> = match db::rules::list(conn) {
        Ok(all) => all
            .into_iter()
            .filter(|r| r.is_active)
            .filter(|r| {
                r.account_id
                    .as_deref()
                    .map(|a| a == account_id)
                    .unwrap_or(true)
            })
            .collect(),
        Err(e) => {
            log::warn!("Failed to load mail_rules, continuing without rule eval: {e}");
            Vec::new()
        }
    };

    for h in headers {
        let from_email = h.from_email.as_deref().unwrap_or("");

        // 1. Sender override, 2. heuristic classifier.
        let (mut category, mut source) =
            match db::categories::get_sender_category(conn, from_email) {
                Ok(Some(cat)) => (cat, "user".to_string()),
                _ => {
                    let input = crate::email::categorize::ClassificationInput {
                        from_email,
                        from_name: h.from_name.as_deref(),
                        subject: h.subject.as_deref(),
                        has_list_unsubscribe: h.list_unsubscribe.is_some(),
                        precedence: h.precedence.as_deref(),
                    };
                    let (cat, _confidence) = crate::email::categorize::classify(&input);
                    (cat.as_str().to_string(), "heuristic".to_string())
                }
            };

        // 3. Evaluate user rules. Rules see from / to / subject. The body is
        // NOT available at classify time (headers only), so a `body` condition
        // can never match — the MCP rejects them at creation for that reason,
        // and `list_mail_rules` warns about any legacy rule that has one.
        if !rules.is_empty() {
            let subject = h.subject.as_deref().unwrap_or("");
            let actions =
                db::rules::evaluate(&rules, from_email, &h.to_list, subject, "", account_id);
            let mut rule_marked_read = false;
            let mut rule_marked_flagged = false;
            for action in &actions {
                match action.action_type.as_str() {
                    "set_category" => {
                        if let Some(ref cat) = action.value {
                            category = cat.clone();
                            source = "rule".to_string();
                        }
                    }
                    "mark_read" => rule_marked_read = true,
                    "mark_flagged" => rule_marked_flagged = true,
                    // "move" and "delete" are intentionally not auto-applied
                    // during sync — they're destructive/IMAP side-effects
                    // that should be triggered from the UI or a dedicated
                    // rules-runner command, not the hot sync path.
                    _ => {}
                }
            }
            if rule_marked_read {
                if let Err(e) = db::messages::update_flags(conn, account_id, folder, h.uid, true) {
                    log::warn!("rule mark_read failed for uid {}: {e}", h.uid);
                }
            }
            if rule_marked_flagged {
                if let Err(e) = conn.execute(
                    "UPDATE messages SET is_flagged = 1 WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                    rusqlite::params![account_id, folder, h.uid],
                ) {
                    log::warn!("rule mark_flagged failed for uid {}: {e}", h.uid);
                }
            }
        }

        if let Err(e) = db::categories::update_message_category(
            conn, account_id, folder, h.uid, &category, &source,
        ) {
            log::error!("Failed to update category for message {}: {e}", h.uid);
        }
    }
}

/// Store each header's delivery-header addresses (v64). Best-effort: they feed
/// send-as suggestions and the reply default, so a failure is logged, never
/// allowed to fail the sync that carried them.
pub fn record_delivered_to(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    headers: &[imap::ImapMessageHeader],
) {
    let rows: Vec<(u32, &str)> = headers
        .iter()
        .filter_map(|h| h.delivered_to.as_deref().map(|d| (h.uid, d)))
        .collect();
    if rows.is_empty() {
        return;
    }
    if let Err(e) = db::messages::set_delivered_to_batch(conn, account_id, folder, &rows) {
        log::warn!("sync: recording delivered-to for {folder} failed: {e}");
    }
}

pub fn apply_header_batch(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    headers: &[imap::ImapMessageHeader],
    batch_end_uid: u32,
    uidvalidity: u32,
) -> Result<(), AppError> {
    if !headers.is_empty() {
        let batch = headers_to_insert_batch(headers);
        db::messages::insert_batch(conn, account_id, folder, &batch)?;
        // Inside the insert, deliberately: see `classify_headers`. Every caller
        // gets it, and a new caller cannot forget it.
        classify_headers(conn, account_id, folder, headers);
        record_delivered_to(conn, account_id, folder, headers);
        // Re-link drafts this software saved (v60): the compose window's
        // autosave and any other CXMail instance mint a new UID per save, and
        // the header is how the logical draft follows it through a sync.
        // `link` is forward-only within this uidvalidity, so seeing the old
        // and the new revision in either order cannot regress the pointer.
        for h in headers {
            if let Some(id) = h.draft_id.as_deref() {
                if let Err(e) = db::drafts::link(conn, id, account_id, folder, uidvalidity, h.uid) {
                    log::warn!("sync: linking draft {id} to UID {} in {folder} failed: {e}", h.uid);
                }
            }
        }
    }
    db::sync_state::upsert_success(conn, account_id, folder, batch_end_uid, uidvalidity)?;
    Ok(())
}

/// Final write at the end of a header sync. Always re-runs `upsert_success`
/// (so a zero-batch sync still bumps `last_synced` and clears any stale
/// `last_error`) and updates the folders-table metadata. Caller passes
/// `final_last_uid` = batches.last().end_uid (or `plan.initial_last_uid` if
/// no batches ran).
pub fn finalize_header_sync(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    final_last_uid: u32,
    uidvalidity: u32,
    uidnext: u32,
) -> Result<(), AppError> {
    db::sync_state::upsert_success(conn, account_id, folder, final_last_uid, uidvalidity)?;
    db::folders::update_sync_info(conn, account_id, folder, uidvalidity, uidnext)?;
    Ok(())
}

pub fn prepare_reconciliation(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    mode: SyncMode,
) -> Result<ReconciliationPlan, AppError> {
    let sync_state = db::sync_state::get(conn, account_id, folder)?.ok_or_else(|| {
        AppError::General("sync_state row missing during reconciliation".to_string())
    })?;

    let reconcile_flags = should_reconcile_flags(&sync_state, mode);
    let flag_uids = if reconcile_flags {
        match mode {
            SyncMode::Full => db::messages::get_uids_for_folder(conn, account_id, folder)?,
            SyncMode::Limited => db::messages::get_recent_uids_for_folder(
                conn,
                account_id,
                folder,
                LIMITED_FLAG_WINDOW,
            )?,
        }
    } else {
        Vec::new()
    };

    let reconcile_existence = matches!(mode, SyncMode::Full);
    let existence_uids = if reconcile_existence {
        db::messages::get_recent_uids_for_folder(conn, account_id, folder, FULL_EXISTENCE_WINDOW)?
    } else {
        Vec::new()
    };

    Ok(ReconciliationPlan {
        reconcile_flags,
        flag_uids,
        reconcile_existence,
        existence_uids,
    })
}

pub async fn fetch_reconciliation(
    session: &mut imap::ImapSession,
    plan: &ReconciliationPlan,
) -> Result<FetchedReconciliation, AppError> {
    let unseen_uids = match imap::search_unseen(session).await {
        Ok(uids) => Some(uids),
        Err(e) => {
            log::warn!("SEARCH UNSEEN failed (unread count will be stale): {e}");
            None
        }
    };

    let flags = if plan.reconcile_flags {
        if !plan.flag_uids.is_empty() {
            imap::fetch_flags(session, &plan.flag_uids).await?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let existing_on_server = if plan.reconcile_existence {
        if !plan.existence_uids.is_empty() {
            imap::uid_search_range(session, &plan.existence_uids).await?
        } else {
            HashSet::new()
        }
    } else {
        HashSet::new()
    };

    Ok(FetchedReconciliation {
        unseen_uids,
        flags,
        existing_on_server,
    })
}

pub fn apply_reconciliation(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    plan: &ReconciliationPlan,
    fetched: FetchedReconciliation,
) -> Result<(u32, u32), AppError> {
    if let Some(ref unseen_uids) = fetched.unseen_uids {
        db::folders::update_counts(conn, account_id, folder, unseen_uids.len() as i32)?;
    }

    let mut updated_count = 0u32;
    if plan.reconcile_flags {
        updated_count = db::messages::apply_server_flags(
            conn,
            account_id,
            folder,
            &fetched
                .flags
                .iter()
                .map(|item| (item.uid, item.is_read, item.is_flagged))
                .collect::<Vec<_>>(),
        )?;
        db::sync_state::mark_flags_reconciled(conn, account_id, folder)?;
    }

    let mut deleted_count = 0u32;
    if plan.reconcile_existence {
        let missing_uids: Vec<u32> = plan
            .existence_uids
            .iter()
            .copied()
            .filter(|uid| !fetched.existing_on_server.contains(uid))
            .collect();
        if !missing_uids.is_empty() {
            deleted_count = db::messages::delete_uids(conn, account_id, folder, &missing_uids)?;
        }
        db::sync_state::mark_existence_reconciled(conn, account_id, folder)?;
    }

    Ok((updated_count, deleted_count))
}

fn headers_to_insert_batch(
    headers: &[imap::ImapMessageHeader],
) -> Vec<(
    u32,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    &str,
    Option<&str>,
    &str,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    bool,
    bool,
    bool,
    i64,
    Option<&str>,
    Option<&str>,
    &str,
    &str,
)> {
    headers
        .iter()
        .map(|header| {
            (
                header.uid,
                header.subject.as_deref(),
                header.from_name.as_deref(),
                header.from_email.as_deref(),
                header.date.as_deref().unwrap_or(""),
                None::<&str>,
                "[]",
                header.message_id.as_deref(),
                header.in_reply_to.as_deref(),
                header.references.as_deref(),
                header.is_read,
                header.is_flagged,
                false,
                header.size as i64,
                header.list_unsubscribe.as_deref(),
                header.list_unsubscribe_post.as_deref(),
                header.to_list.as_str(),
                header.cc_list.as_str(),
            )
        })
        .collect()
}

fn should_reconcile_flags(state: &db::sync_state::SyncStateRow, mode: SyncMode) -> bool {
    match mode {
        SyncMode::Full => true,
        SyncMode::Limited => state
            .last_flags_reconciled_at
            .as_deref()
            .and_then(parse_sqlite_datetime)
            .map(|timestamp| {
                Utc::now() - timestamp
                    >= chrono::Duration::from_std(LIMITED_FLAG_RECONCILE_INTERVAL).unwrap()
            })
            .unwrap_or(true),
    }
}

fn parse_sqlite_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc))
        })
}

fn build_sync_plan(
    stored_uidvalidity: Option<u32>,
    stored_last_uid: u32,
    server_uidvalidity: u32,
    uidnext: u32,
) -> SyncPlan {
    let reset_required = match stored_uidvalidity {
        Some(stored) => stored != server_uidvalidity,
        None => stored_last_uid > 0, // Have cached msgs but no uidvalidity recorded → reset
    };

    let initial_last_uid = if reset_required {
        0
    } else {
        stored_last_uid.min(uidnext.saturating_sub(1))
    };

    let start_uid = initial_last_uid.saturating_add(1);
    let end_uid = uidnext.saturating_sub(1);
    let batches = build_fetch_batches(start_uid, end_uid, FETCH_BATCH_SIZE);

    SyncPlan {
        reset_required,
        initial_last_uid,
        batches,
    }
}

fn build_fetch_batches(start_uid: u32, end_uid: u32, batch_size: u32) -> Vec<(u32, u32)> {
    if end_uid < start_uid || batch_size == 0 {
        return Vec::new();
    }

    let mut batches = Vec::new();
    let mut current_start = start_uid;
    while current_start <= end_uid {
        let current_end = current_start
            .saturating_add(batch_size.saturating_sub(1))
            .min(end_uid);
        batches.push((current_start, current_end));
        if current_end == u32::MAX {
            break;
        }
        current_start = current_end.saturating_add(1);
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sync_after_seed_with_existing_messages_triggers_reset() {
        // When uidvalidity is unknown (None) but messages exist locally (last_uid > 0),
        // we must reset to re-validate against the server's uidvalidity
        let plan = build_sync_plan(None, 50, 1, 56);

        assert!(plan.reset_required);
        assert_eq!(plan.initial_last_uid, 0);
        assert_eq!(plan.batches, vec![(1, 55)]);
    }

    #[test]
    fn first_sync_fresh_folder_no_reset() {
        // When uidvalidity is unknown and no local messages exist, no reset needed
        let plan = build_sync_plan(None, 0, 1, 56);

        assert!(!plan.reset_required);
        assert_eq!(plan.initial_last_uid, 0);
        assert_eq!(plan.batches, vec![(1, 55)]);
    }

    #[test]
    fn unchanged_uidnext_fetches_nothing() {
        let plan = build_sync_plan(Some(1), 55, 1, 56);

        assert!(plan.batches.is_empty());
    }

    #[test]
    fn large_gap_is_fetched_in_full_batches() {
        let plan = build_sync_plan(Some(1), 100, 1, 356);

        assert_eq!(plan.batches, vec![(101, 350), (351, 355)]);
    }

    #[test]
    fn uidvalidity_mismatch_forces_reset_and_rebuild() {
        let plan = build_sync_plan(Some(9), 50, 10, 26);

        assert!(plan.reset_required);
        assert_eq!(plan.initial_last_uid, 0);
        assert_eq!(plan.batches, vec![(1, 25)]);
    }

    // -------- per-batch checkpointing tests --------

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                message_id TEXT,
                in_reply_to TEXT,
                reference_ids TEXT,
                subject TEXT,
                from_name TEXT,
                from_email TEXT,
                to_list TEXT,
                cc_list TEXT,
                date TEXT NOT NULL,
                snippet TEXT,
                flags TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_flagged INTEGER NOT NULL DEFAULT 0,
                has_attachments INTEGER NOT NULL DEFAULT 0,
                size_bytes INTEGER DEFAULT 0,
                list_unsubscribe TEXT,
                list_unsubscribe_post TEXT,
                thread_root_id TEXT,
                delivered_to TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(account_id, folder_name, uid)
            );

            CREATE TABLE sync_state (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                last_uid INTEGER DEFAULT 0,
                uidvalidity INTEGER,
                last_synced TEXT,
                last_flags_reconciled_at TEXT,
                last_existence_reconciled_at TEXT,
                last_error TEXT,
                last_error_at TEXT,
                UNIQUE(account_id, folder_name)
            );

            CREATE TABLE folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                name TEXT NOT NULL,
                display_name TEXT,
                folder_type TEXT,
                delimiter TEXT,
                flags TEXT,
                total_count INTEGER DEFAULT 0,
                unread_count INTEGER DEFAULT 0,
                uidvalidity INTEGER,
                uidnext INTEGER,
                last_synced TEXT,
                UNIQUE(account_id, name)
            );
            ",
        )
        .expect("schema");

        conn.execute(
            "INSERT INTO folders (account_id, name) VALUES ('acc-1', 'INBOX')",
            [],
        )
        .unwrap();

        conn
    }

    /// v60: a synced header carrying `X-CXMail-Draft-Id` links the drafts
    /// row, and the pointer only ever moves forward within the generation —
    /// the sync may report the old and the new revision in either order.
    #[test]
    /// The invariant that stops the NULL-category leak coming back.
    ///
    /// Classification used to be a separate call the caller made after
    /// `apply_header_batch`, and two of the three callers (`idle.rs`,
    /// `bin/helper.rs`) never made it — 11,943 inbox messages on the live
    /// mailbox ended up with `category = NULL`, which `COALESCE(category,
    /// 'primary')` shows in Primary. If someone hoists classification back out
    /// of the insert, this fails.
    #[test]
    fn a_message_cannot_be_inserted_without_being_classified() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();

        let mut promo = header(1, "50% off everything this weekend");
        promo.from_email = Some("newsletter@e.lululemon.com".to_string());
        promo.list_unsubscribe = Some("<https://example.com/u>".to_string());
        let plain = header(2, "Re: the contract");

        apply_header_batch(&conn, "acct", "INBOX", &[promo, plain], 2, 7).unwrap();

        let categories: Vec<Option<String>> = conn
            .prepare("SELECT category FROM messages WHERE account_id='acct' ORDER BY uid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(categories.len(), 2);
        for (uid, c) in categories.iter().enumerate() {
            assert!(c.is_some(), "uid {} was stored unclassified", uid + 1);
        }
        // And the List-Unsubscribe signal actually reached the classifier —
        // half the unclassified inbox carries it, and without it every bulk
        // sender lands in Primary.
        assert_eq!(categories[0].as_deref(), Some("promotions"));
    }

    fn apply_header_batch_links_drafts_forward_only() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();
        let mut newer = header(12, "v2");
        newer.draft_id = Some("d1".into());
        let mut older = header(10, "v1");
        older.draft_id = Some("d1".into());
        let mut plain = header(11, "not a draft");
        plain.draft_id = None;

        apply_header_batch(&conn, "acct", "Drafts", &[newer], 12, 7).unwrap();
        // The OLD revision arriving afterwards must not regress the pointer.
        apply_header_batch(&conn, "acct", "Drafts", &[older, plain], 12, 7).unwrap();
        let row = crate::db::drafts::get(&conn, "d1").unwrap().unwrap();
        assert_eq!((row.current_uid, row.uidvalidity, row.folder_name.as_str()), (12, 7, "Drafts"));
        assert!(crate::db::drafts::find_by_uid(&conn, "acct", "Drafts", 11).unwrap().is_none());
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM drafts", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "a header without the id links nothing");
    }

    fn header(uid: u32, subject: &str) -> imap::ImapMessageHeader {
        imap::ImapMessageHeader {
            uid,
            subject: Some(subject.to_string()),
            from_name: None,
            from_email: Some("sender@example.com".to_string()),
            to_list: "[]".to_string(),
            cc_list: "[]".to_string(),
            date: Some("2026-03-31 12:00:00".to_string()),
            message_id: None,
            in_reply_to: None,
            references: None,
            flags: Vec::new(),
            size: 100,
            is_read: false,
            is_flagged: false,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            precedence: None,
            draft_id: None,
            delivered_to: None,
        }
    }

    fn read_sync_state(
        conn: &Connection,
    ) -> (u32, Option<u32>, Option<String>, Option<String>) {
        conn.query_row(
            "SELECT last_uid, uidvalidity, last_error, last_synced FROM sync_state
             WHERE account_id = 'acc-1' AND folder_name = 'INBOX'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u32,
                    row.get::<_, Option<u32>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .unwrap()
    }

    #[test]
    fn apply_header_batch_with_headers_advances_to_batch_end() {
        // Sparse batch: only UID 10 and 20 returned, but batch covered 1..=250.
        // Floor must move to 250 so the next sync skips the dead range.
        let conn = setup_db();
        let headers = vec![header(10, "ten"), header(20, "twenty")];

        apply_header_batch(&conn, "acc-1", "INBOX", &headers, 250, 7).unwrap();

        let (last_uid, uidvalidity, _, _) = read_sync_state(&conn);
        assert_eq!(last_uid, 250);
        assert_eq!(uidvalidity, Some(7));
    }

    #[test]
    fn apply_header_batch_empty_advances_to_batch_end() {
        let conn = setup_db();

        apply_header_batch(&conn, "acc-1", "INBOX", &[], 50, 7).unwrap();

        let (last_uid, uidvalidity, _, _) = read_sync_state(&conn);
        assert_eq!(last_uid, 50);
        assert_eq!(uidvalidity, Some(7));
    }

    #[test]
    fn apply_header_batch_inserts_or_ignores() {
        let conn = setup_db();
        // Pre-insert UID=10 with subject "orig"
        let preexisting = vec![header(10, "orig")];
        apply_header_batch(&conn, "acc-1", "INBOX", &preexisting, 10, 7).unwrap();

        // Re-insert UID=10 with subject "new" — should NOT clobber
        let updated = vec![header(10, "new")];
        apply_header_batch(&conn, "acc-1", "INBOX", &updated, 11, 7).unwrap();

        let stored: String = conn
            .query_row(
                "SELECT subject FROM messages WHERE account_id = 'acc-1'
                 AND folder_name = 'INBOX' AND uid = 10",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "orig");
    }

    #[test]
    fn apply_header_batch_clears_last_error() {
        let conn = setup_db();
        // Pre-set last_error
        db::sync_state::record_error(&conn, "acc-1", "INBOX", "Timed out").unwrap();
        let (_, _, before, _) = read_sync_state(&conn);
        assert_eq!(before.as_deref(), Some("Timed out"));

        apply_header_batch(&conn, "acc-1", "INBOX", &[header(1, "x")], 100, 7).unwrap();

        let (_, _, after, _) = read_sync_state(&conn);
        assert_eq!(after, None);
    }

    #[test]
    fn apply_reset_if_needed_wipes_when_reset_true() {
        let conn = setup_db();
        // Pre-populate messages + sync_state
        apply_header_batch(&conn, "acc-1", "INBOX", &[header(1, "x")], 100, 7).unwrap();
        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 1);

        let plan = SelectedPlan {
            uidvalidity: 8,
            uidnext: 200,
            initial_last_uid: 0,
            reset_required: true,
            batches: vec![(1, 199)],
        };
        let was_reset = apply_reset_if_needed(&conn, "acc-1", "INBOX", &plan).unwrap();
        assert!(was_reset);

        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_after, 0);

        let (last_uid, uidvalidity, _, _) = read_sync_state(&conn);
        assert_eq!(last_uid, 0);
        assert_eq!(uidvalidity, None);
    }

    #[test]
    fn apply_reset_if_needed_no_op_when_reset_false() {
        let conn = setup_db();
        apply_header_batch(&conn, "acc-1", "INBOX", &[header(1, "x")], 100, 7).unwrap();

        let plan = SelectedPlan {
            uidvalidity: 7,
            uidnext: 200,
            initial_last_uid: 100,
            reset_required: false,
            batches: vec![(101, 199)],
        };
        let was_reset = apply_reset_if_needed(&conn, "acc-1", "INBOX", &plan).unwrap();
        assert!(!was_reset);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (last_uid, uidvalidity, _, _) = read_sync_state(&conn);
        assert_eq!(last_uid, 100);
        assert_eq!(uidvalidity, Some(7));
    }

    #[test]
    fn finalize_header_sync_clears_last_error_with_zero_batches() {
        let conn = setup_db();
        db::sync_state::record_error(&conn, "acc-1", "INBOX", "Timed out").unwrap();

        finalize_header_sync(&conn, "acc-1", "INBOX", 42, 7, 43).unwrap();

        let (last_uid, uidvalidity, last_error, last_synced) = read_sync_state(&conn);
        assert_eq!(last_uid, 42);
        assert_eq!(uidvalidity, Some(7));
        assert_eq!(last_error, None);
        assert!(last_synced.is_some());

        // folders.uidvalidity / uidnext also updated
        let (fv, fn_): (Option<u32>, Option<u32>) = conn
            .query_row(
                "SELECT uidvalidity, uidnext FROM folders WHERE account_id = 'acc-1' AND name = 'INBOX'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(fv, Some(7));
        assert_eq!(fn_, Some(43));
    }
}
