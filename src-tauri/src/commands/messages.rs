use crate::db;
use crate::email::{categorize, detect_events, imap, parser, sync};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Serialize;
use serde_json;
use std::collections::HashSet;
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct ContactResult {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Serialize)]
pub struct MessagePage {
    pub messages: Vec<db::messages::MessageRow>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
}

#[derive(Serialize)]
pub struct MessageDetail {
    pub uid: u32,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to_list: Vec<parser::EmailAddress>,
    pub cc_list: Vec<parser::EmailAddress>,
    pub bcc_list: Vec<parser::EmailAddress>,
    pub date: Option<String>,
    pub plain_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub attachments: Vec<parser::AttachmentMeta>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub message_id: Option<String>,
    pub references: Option<String>,
    /// The message's own `In-Reply-To`. Surfaced so reopening a draft can
    /// round-trip its threading headers instead of rebuilding the MIME without
    /// them — see gotcha #39.
    pub in_reply_to: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct AccountSyncStatus {
    pub account_id: String,
    pub email: String,
    pub success: bool,
    pub error: Option<String>,
    pub new_count: u32,
    /// True when `error` means the account's credentials are actually dead
    /// (Google returned invalid_grant/invalid_client) rather than a
    /// transient sync failure — the frontend uses this to offer a
    /// "Reconnect" action instead of just displaying the error.
    pub needs_reauth: bool,
}

#[derive(Serialize)]
pub struct SyncResult {
    pub new_count: u32,
    pub updated_count: u32,
    pub account_statuses: Vec<AccountSyncStatus>,
}

struct FolderSyncExecution {
    parsed_bodies: Vec<(u32, parser::ParsedMessage)>,
    new_count: u32,
    updated_count: u32,
}

/// Connect, sync one folder, disconnect.
///
/// For callers that touch a single folder. Anything syncing several folders on
/// one account should open the session itself and call
/// `run_folder_sync_on_session` per folder — see the note there.
async fn run_folder_sync_execution(
    state: &AppState,
    app: &tauri::AppHandle,
    account: &db::accounts::Account,
    folder: &str,
    mode: sync::SyncMode,
    body_prefetch_limit: usize,
) -> Result<FolderSyncExecution, AppError> {
    let mut session = imap::connect_for_account(account).await?;
    let result = run_folder_sync_on_session(
        state,
        app,
        account,
        folder,
        mode,
        body_prefetch_limit,
        &mut session,
    )
    .await;
    let _ = imap::disconnect(session, &account.email).await;
    result
}

/// Sync one folder over an ALREADY-OPEN session.
///
/// Split out of `run_folder_sync_execution` so a caller syncing several folders
/// on one account pays for ONE connection instead of one per folder. Before the
/// split, `sync_all_inboxes` opened three full TCP + TLS + XOAUTH2 handshakes
/// per account per tick (INBOX, Drafts, Sent), which measured at ~48
/// connects/hour/account — roughly 430/hour to Gmail from one machine across
/// nine accounts, where ~145 would do. `sync_account_folders` already had the
/// one-session-many-folders shape; this makes it reusable rather than a
/// second implementation.
///
/// The session is left OPEN and SELECTed on `folder`. The caller owns closing
/// it, and must route it through `imap::disconnect` (never a bare drop) or the
/// ledger counts it `abandoned` — see gotcha #46.
async fn run_folder_sync_on_session(
    state: &AppState,
    app: &tauri::AppHandle,
    account: &db::accounts::Account,
    folder: &str,
    mode: sync::SyncMode,
    body_prefetch_limit: usize,
    session: &mut imap::ImapSession,
) -> Result<FolderSyncExecution, AppError> {
    let mut prepared = {
        let conn = state.db.safe_lock();
        sync::prepare_folder_sync(&conn, &account.id, folder)?
    };

    let sync_result = async {
        let plan = sync::select_and_plan(&mut *session, folder, &prepared.sync_state).await?;
        log::info!(
            "Synced remote state for {}/{}: uidvalidity={}, uidnext={}, batches={}",
            account.email,
            folder,
            plan.uidvalidity,
            plan.uidnext,
            plan.batches.len()
        );

        let was_reset = {
            let conn = state.db.safe_lock();
            sync::apply_reset_if_needed(&conn, &account.id, folder, &plan)?
        };
        if was_reset {
            // Keep in-memory snapshot in sync with the now-empty cache so
            // `new_count` doesn't compare incoming UIDs against pre-reset rows.
            prepared.existing_uids.clear();
        }

        let existing_uids = prepared.existing_uids;
        let mut all_headers: Vec<imap::ImapMessageHeader> = Vec::new();
        let mut new_count_running = 0u32;
        let mut final_last_uid = plan.initial_last_uid;
        // Per-message notifications fire only on single-batch syncs (real-time
        // arrivals via IDLE / one-tick incremental). Multi-batch = backfill;
        // surfacing 3,800 desktop popups for an old-mail recovery is worse
        // than no notification at all. DB checkpointing and classification
        // still happen per batch — only the popup side effects are gated.
        let notify_per_batch = plan.batches.len() <= 1;

        for &(start_uid, end_uid) in &plan.batches {
            let batch_headers = imap::fetch_headers(&mut *session, start_uid, end_uid).await?;
            {
                let conn = state.db.safe_lock();
                sync::apply_header_batch(
                    &conn,
                    &account.id,
                    folder,
                    &batch_headers,
                    end_uid,
                    plan.uidvalidity,
                )?;
            }
            if notify_per_batch {
                notify_new_mail(
                    app,
                    state,
                    &account.id,
                    folder,
                    &batch_headers,
                    &existing_uids,
                    was_reset,
                );
                emit_unsubscribed_sender_alerts(app, state, &account.id, &batch_headers, was_reset);
            }
            // Multi-batch = backfill of a large folder: let the frontend
            // repaint as rows land instead of waiting for the whole crawl.
            // Single-batch incremental ticks stay silent — their callers
            // already reload once at the end.
            if plan.batches.len() > 1 {
                let _ = app.emit(
                    "folder-sync-progress",
                    serde_json::json!({ "account_id": account.id, "folder": folder }),
                );
            }
            new_count_running += batch_headers
                .iter()
                .filter(|h| !existing_uids.contains(&h.uid))
                .count() as u32;
            log::info!(
                "Sync batch {}..{} done: {} headers, last_uid={}",
                start_uid,
                end_uid,
                batch_headers.len(),
                end_uid
            );
            all_headers.extend(batch_headers);
            final_last_uid = end_uid;
        }

        {
            let conn = state.db.safe_lock();
            sync::finalize_header_sync(
                &conn,
                &account.id,
                folder,
                final_last_uid,
                plan.uidvalidity,
                plan.uidnext,
            )?;
            update_thread_root_ids_with_conn(&conn, &account.id, folder);
        }

        let reconciliation_plan = {
            let conn = state.db.safe_lock();
            sync::prepare_reconciliation(&conn, &account.id, folder, mode)?
        };
        let reconciliation =
            sync::fetch_reconciliation(&mut *session, &reconciliation_plan).await?;
        let (updated_count, deleted_count) = {
            let conn = state.db.safe_lock();
            sync::apply_reconciliation(&conn, &account.id, folder, &reconciliation_plan, reconciliation)?
        };

        let parsed_bodies = prefetch_and_parse_bodies(
            state,
            &mut *session,
            &account.id,
            folder,
            &all_headers,
            body_prefetch_limit,
        )
        .await;

        Ok::<FolderSyncExecution, AppError>(FolderSyncExecution {
            parsed_bodies,
            new_count: new_count_running,
            updated_count: updated_count + deleted_count,
        })
    }
    .await;

    sync_result
}

async fn prefetch_and_parse_bodies(
    state: &AppState,
    session: &mut imap::ImapSession,
    account_id: &str,
    folder: &str,
    headers: &[imap::ImapMessageHeader],
    limit: usize,
) -> Vec<(u32, parser::ParsedMessage)> {
    if headers.is_empty() || limit == 0 {
        return Vec::new();
    }

    let cached_uids = {
        let conn = state.db.safe_lock();
        db::messages::get_cached_body_uids(&conn, account_id, folder).unwrap_or_default()
    };

    let to_prefetch: Vec<u32> = headers
        .iter()
        .rev()
        .filter(|header| !cached_uids.contains(&header.uid))
        .take(limit)
        .map(|header| header.uid)
        .collect();

    let mut prefetched = Vec::new();
    for uid in to_prefetch {
        match imap::fetch_body(session, uid).await {
            Ok(raw) => prefetched.push((uid, raw)),
            Err(e) => {
                log::warn!("Prefetch body failed for UID {uid}: {e}");
                break;
            }
        }
    }

    prefetched
        .into_iter()
        .filter_map(|(uid, raw)| {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser::parse_message(&raw))) {
                Ok(parsed) => Some((uid, parsed)),
                Err(_) => {
                    log::error!("parse_message panicked for UID {}, skipping", uid);
                    None
                }
            }
        })
        .collect()
}

async fn fetch_raw_message(
    state: &AppState,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<Vec<u8>, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        log::info!("fetch_raw_message: connecting to IMAP for {} ({})", account.email, account.provider);
        let mut session = imap::connect_for_account(&account).await?;
        log::info!("fetch_raw_message: connected, selecting folder {}", folder);
        let _ = imap::select_folder(&mut session, folder).await?;
        log::info!("fetch_raw_message: fetching body for uid={}", uid);
        let body = imap::fetch_body(&mut session, uid).await?;
        log::info!("fetch_raw_message: got body ({} bytes), disconnecting", body.len());
        let _ = imap::disconnect(session, &account.email).await;
        Ok::<Vec<u8>, AppError>(body)
    })
    .await
    .map_err(|_| AppError::Imap("Timed out fetching message — tap Retry".to_string()))?
}

/// Coalesced repair for the `fetch_message_body` cache HIT branch. One IMAP
/// fetch + one parse covers both attachment-metadata backfill and `cid:`
/// inlining. Either flag can be requested independently. The cid: write uses
/// a targeted UPDATE so we don't clobber `summary` / `summary_model`.
///
/// Returns `(new_attachments_if_repaired, new_sanitized_html_if_repaired)`.
async fn repair_cached_body(
    state: &AppState,
    account_id: &str,
    folder: &str,
    uid: u32,
    needs_attachment_repair: bool,
    needs_cid_repair: bool,
) -> Result<(Option<Vec<parser::AttachmentMeta>>, Option<String>), AppError> {
    let raw_body = fetch_raw_message(state, account_id, folder, uid).await?;
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parser::parse_message(&raw_body)
    }))
    .map_err(|_| AppError::Imap("Failed to parse message (internal error)".to_string()))?;

    {
        let conn = state.db.safe_lock();
        let tx = conn.unchecked_transaction()?;
        if needs_attachment_repair {
            db::messages::insert_attachments(&tx, account_id, folder, uid, &parsed.attachments)?;
        }
        if needs_cid_repair {
            // Only write when the re-parse actually produced HTML. Writing NULL
            // here would clobber the cached body AND set cid_resolved=1, leaving
            // a permanently-blank message (BUG-03). If None, skip — the flag
            // stays 0 so a later open retries.
            if let Some(html) = parsed.sanitized_html.as_deref() {
                db::messages::update_sanitized_html_after_cid_repair(
                    &tx, account_id, folder, uid, html,
                )?;
            } else {
                log::warn!(
                    "repair_cached_body: re-parse yielded no HTML for uid={uid}; \
                     skipping cid update to avoid clobbering cached body"
                );
            }
        }
        tx.commit()?;
    }

    Ok((
        needs_attachment_repair.then_some(parsed.attachments),
        // None (not Some("")) when there's no HTML, so the caller keeps the
        // existing cached body rather than rendering blank for this fetch.
        if needs_cid_repair { parsed.sanitized_html } else { None },
    ))
}

/// Write the full set of parsed metadata (body, snippet, address lists,
/// attachments, calendar events) for a single message inside an open
/// transaction. Mirrors the cache-miss branch of `fetch_message_body` so that
/// prefetch paths populate the same tables the on-demand read expects.
fn persist_parsed_metadata(
    tx: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    parsed: &parser::ParsedMessage,
) {
    if let Err(e) = db::messages::insert_body(
        tx,
        account_id,
        folder,
        uid,
        parsed.plain_text.as_deref(),
        parsed.html_body.as_deref(),
        parsed.sanitized_html.as_deref(),
        true,
    ) {
        log::warn!("Failed to store prefetched body for UID {uid}: {e}");
    }

    // Free ride: this path always ran on a full raw RFC822 message, so the
    // header block is already in hand and costs no extra IMAP. Everything that
    // reaches here — sync body prefetch, the on-demand cache miss, mbox import
    // — populates `message_headers` from now on. Best-effort: a failure here
    // must never lose the body write (gotcha: headers are an audit nicety, the
    // body is the product).
    if let Some(ref headers) = parsed.raw_headers {
        if let Err(e) = db::messages::store_raw_headers(tx, account_id, folder, uid, headers) {
            log::warn!("Failed to store raw headers for UID {uid}: {e}");
        }
    }

    if let Some(ref snippet) = parsed.snippet {
        if !snippet.is_empty() {
            let _ = db::messages::update_snippet(tx, account_id, folder, uid, snippet);
        }
    }

    if let Ok(to_json) = serde_json::to_string(&parsed.to_list) {
        let cc_json = serde_json::to_string(&parsed.cc_list).unwrap_or_default();
        let bcc_json = serde_json::to_string(&parsed.bcc_list).unwrap_or_default();
        if let Err(e) = db::messages::store_address_lists(tx, account_id, folder, uid, &to_json, &cc_json, &bcc_json) {
            log::error!("Failed to store address lists for message {uid}: {e}");
        }
    }

    if let Err(e) = db::messages::insert_attachments(tx, account_id, folder, uid, &parsed.attachments) {
        log::error!("Failed to insert attachments for message {uid}: {e}");
    }

    for event in &parsed.calendar_events {
        if let Err(e) = db::calendar::insert(
            tx,
            account_id,
            folder,
            uid,
            event.event_uid.as_deref(),
            event.summary.as_deref(),
            event.description.as_deref(),
            event.location.as_deref(),
            &event.dtstart,
            event.dtend.as_deref(),
            event.organizer_name.as_deref(),
            event.organizer_email.as_deref(),
            event.status.as_deref(),
            event.method.as_deref(),
            Some(&event.raw_ics),
        ) {
            log::error!("Failed to insert calendar event for message {uid}: {e}");
        }
    }

    if parsed.calendar_events.is_empty() {
        if let Ok(msg_row) = db::messages::get_by_uid(tx, account_id, folder, uid) {
            let detected = detect_events::detect_events(
                parsed.subject.as_deref(),
                parsed.plain_text.as_deref(),
                &parsed.from_email,
                msg_row.as_ref().map(|m| m.date.as_str()).unwrap_or(""),
            );
            for event in &detected {
                if event.confidence >= 0.6 {
                    if let Err(e) = db::calendar::insert_detected(
                        tx,
                        account_id,
                        folder,
                        uid,
                        &event.summary,
                        &event.dtstart,
                        event.dtend.as_deref(),
                        event.location.as_deref(),
                        event.description.as_deref(),
                        event.confidence,
                    ) {
                        log::error!("Failed to insert detected event for message {uid}: {e}");
                    }
                }
            }
        }
    }
}

/// Canonical body-write path: persist parsed metadata to SQLite in one
/// transaction. The v40 FTS triggers index the body inside the same
/// transaction — search can never drift from `message_bodies`.
fn persist_body(
    state: &AppState,
    account_id: &str,
    folder: &str,
    uid: u32,
    parsed: &parser::ParsedMessage,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    let tx = conn.unchecked_transaction()?;
    persist_parsed_metadata(&tx, account_id, folder, uid, parsed);
    tx.commit()?;
    Ok(())
}

/// Batch variant of `persist_body` for prefetch paths that hold N parsed
/// messages. One DB transaction for the whole batch.
fn persist_bodies_batch(
    state: &AppState,
    account_id: &str,
    folder: &str,
    parsed_bodies: &[(u32, parser::ParsedMessage)],
) -> Result<(), AppError> {
    if parsed_bodies.is_empty() {
        return Ok(());
    }
    let conn = state.db.safe_lock();
    let tx = conn.unchecked_transaction()?;
    for (uid, parsed) in parsed_bodies {
        persist_parsed_metadata(&tx, account_id, folder, *uid, parsed);
    }
    tx.commit()?;
    Ok(())
}

fn store_prefetched_bodies(
    state: &AppState,
    account_id: &str,
    folder: &str,
    parsed_bodies: &[(u32, parser::ParsedMessage)],
) {
    if let Err(e) = persist_bodies_batch(state, account_id, folder, parsed_bodies) {
        log::error!("Failed to persist prefetched bodies: {e}");
    }
}

fn notify_new_mail(
    app: &tauri::AppHandle,
    state: &AppState,
    account_id: &str,
    folder: &str,
    headers: &[imap::ImapMessageHeader],
    existing_uids: &HashSet<u32>,
    suppress: bool,
) {
    if suppress || headers.is_empty() {
        return;
    }

    let new_uids: Vec<u32> = headers
        .iter()
        .filter(|header| !existing_uids.contains(&header.uid) && !header.is_read)
        .map(|header| header.uid)
        .collect();

    if new_uids.is_empty() {
        return;
    }

    // Scope the mutex lock so it's released before calling on_new_mail.
    // on_new_mail → update_badge → get_total_unread also acquires state.db,
    // and std::sync::Mutex is not re-entrant — previously self-deadlocked.
    let new_mail: Vec<crate::notify::NewMailInfo> = {
        let conn = state.db.safe_lock();
        let categories =
            db::categories::get_message_categories(&conn, account_id, folder, &new_uids)
                .unwrap_or_default();
        headers
            .iter()
            .filter(|header| {
                !existing_uids.contains(&header.uid)
                    && !header.is_read
                    && categories
                        .get(&header.uid)
                        .map_or(true, |category| category == "primary")
            })
            .map(|header| crate::notify::NewMailInfo {
                account_id: account_id.to_string(),
                folder: folder.to_string(),
                uid: header.uid,
                from_name: header.from_name.clone(),
                from_email: header.from_email.clone(),
                subject: header.subject.clone(),
            })
            .collect()
    };

    if !new_mail.is_empty() {
        if crate::notify::account_notify_enabled(app, account_id) {
            crate::notify::on_new_mail(app, &new_mail);
        } else {
            // Account is muted: skip popups but keep tray badge accurate.
            crate::notify::update_badge(app);
        }
    }
}

fn emit_unsubscribed_sender_alerts(
    app: &tauri::AppHandle,
    state: &AppState,
    account_id: &str,
    headers: &[imap::ImapMessageHeader],
    suppress: bool,
) {
    if suppress || headers.is_empty() {
        return;
    }

    let from_emails: Vec<&str> = headers
        .iter()
        .filter_map(|header| header.from_email.as_deref())
        .collect();
    if from_emails.is_empty() {
        return;
    }

    let conn = state.db.safe_lock();
    if let Ok(matched) = db::unsubscribed::check_senders(&conn, account_id, &from_emails) {
        if !matched.is_empty() {
            let _ = app.emit("mail-from-unsubscribed-sender", &matched);
        }
    }
}

fn update_thread_root_ids_with_conn(
    conn: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
) {
    if let Err(e) = db::messages::update_thread_root_ids(conn, account_id, folder) {
        log::error!("Failed to update thread_root_id for {account_id}/{folder}: {e}");
    }
    // Then stitch anything that arrived without threading headers onto the
    // conversation it belongs to. Deliberately a SHORT window: this runs after
    // every folder sync, and re-scanning a year of mail each time would be
    // wasted work — the v48 migration already did the historical pass, and only
    // newly-arrived mail can be newly orphaned.
    if let Err(e) = db::messages::stitch_orphan_threads(conn, 30, false) {
        log::warn!("Failed to stitch orphan threads for {account_id}/{folder}: {e}");
    }
}

async fn prefetch_missing_snippets(
    state: &AppState,
    account: &db::accounts::Account,
    folder: &str,
) {
    let uids_to_fetch = {
        let conn = state.db.safe_lock();
        db::messages::uids_without_snippets(&conn, &account.id, folder, 20).unwrap_or_default()
    };

    if uids_to_fetch.is_empty() {
        return;
    }

    log::info!("Prefetching snippets for {} messages in {}", uids_to_fetch.len(), folder);
    let prefetch_result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        async {
            let mut session = imap::connect_for_account(account).await?;
            // Phase 1: Fetch + parse bodies without holding the DB lock
            let mut parsed_results: Vec<(u32, parser::ParsedMessage)> = Vec::new();
            if imap::select_folder(&mut session, folder).await.is_ok() {
                for &uid in &uids_to_fetch {
                    match imap::fetch_body(&mut session, uid).await {
                        Ok(raw_body) => {
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser::parse_message(&raw_body))) {
                                Ok(parsed) => parsed_results.push((uid, parsed)),
                                Err(_) => {
                                    log::error!("parse_message panicked for UID {}, skipping", uid);
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to prefetch body for UID {uid}: {e}");
                        }
                    }
                }
            }
            let _ = imap::disconnect(session, &account.email).await;

            // Phase 2: batch-write SQLite metadata (FTS triggers index it).
            if let Err(e) = persist_bodies_batch(state, &account.id, folder, &parsed_results) {
                log::error!("Failed to persist snippet prefetch batch: {e}");
            }
            Ok::<(), AppError>(())
        },
    )
    .await;

    match prefetch_result {
        Ok(Err(e)) => log::warn!("Snippet prefetch failed for {folder}: {e}"),
        Err(_) => log::warn!("Snippet prefetch timed out after 15s for {folder}"),
        _ => {}
    }
}

#[tauri::command]
pub async fn fetch_messages(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    page: u32,
    page_size: u32,
    category: Option<String>,
    unread_only: Option<bool>,
) -> Result<MessagePage, AppError> {
    let conn = state.open_read_conn()?;
    let (messages, total) = db::messages::list_by_folder(
        &conn,
        &account_id,
        &folder,
        page,
        page_size,
        category.as_deref(),
        unread_only.unwrap_or(false),
    )?;
    let has_more = (page + 1) * page_size < total;
    Ok(MessagePage {
        messages,
        total,
        page,
        page_size,
        has_more,
    })
}

#[tauri::command]
pub async fn fetch_message_body(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<MessageDetail, AppError> {
    log::info!("fetch_message_body: account={}, folder={}, uid={}", account_id, folder, uid);
    let cached = {
        let conn = state.open_read_conn()?;
        db::messages::get_body(&conn, &account_id, &folder, uid)?
    };

    if let Some(body) = cached {
        log::info!("fetch_message_body: cache HIT for uid={}", uid);
        let (
            msg,
            list_unsub,
            list_unsub_post,
            to_list,
            cc_list,
            bcc_list,
            mut attachments,
            message_id,
            references,
            in_reply_to,
        ) = {
            let conn = state.open_read_conn()?;
            let msg = db::messages::get_by_uid(&conn, &account_id, &folder, uid)?;
            let (list_unsub, list_unsub_post) = db::messages::get_unsubscribe_headers(&conn, &account_id, &folder, uid)?;
            let (to_json, cc_json, bcc_json) = db::messages::get_address_lists(&conn, &account_id, &folder, uid)?;
            let to_list: Vec<parser::EmailAddress> = to_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let cc_list: Vec<parser::EmailAddress> = cc_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let bcc_list: Vec<parser::EmailAddress> = bcc_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let attachments = db::messages::get_attachments(&conn, &account_id, &folder, uid)?;
            let (message_id, references, in_reply_to) =
                db::messages::get_message_headers(&conn, &account_id, &folder, uid)?;
            (
                msg,
                list_unsub,
                list_unsub_post,
                to_list,
                cc_list,
                bcc_list,
                attachments,
                message_id,
                references,
                in_reply_to,
            )
        };

        let needs_attachment_repair = !body.attachment_metadata_checked;
        let needs_cid_repair = !body.cid_resolved
            && body
                .sanitized_html
                .as_deref()
                .map_or(false, |h| h.contains("cid:"));

        let mut sanitized_html = body.sanitized_html;
        if needs_attachment_repair || needs_cid_repair {
            log::info!(
                "fetch_message_body: repairing cached uid={} (attachments={}, cid={})",
                uid,
                needs_attachment_repair,
                needs_cid_repair
            );
            match repair_cached_body(
                state.inner(),
                &account_id,
                &folder,
                uid,
                needs_attachment_repair,
                needs_cid_repair,
            )
            .await
            {
                Ok((new_attachments, new_html)) => {
                    if let Some(att) = new_attachments {
                        attachments = att;
                    }
                    if let Some(html) = new_html {
                        sanitized_html = Some(html);
                    }
                }
                Err(e) => log::warn!("Failed to repair cached body for uid={uid}: {e}"),
            }
        }

        return Ok(MessageDetail {
            uid,
            subject: msg.as_ref().and_then(|m| m.subject.clone()),
            from_name: msg.as_ref().and_then(|m| m.from_name.clone()),
            from_email: msg.as_ref().and_then(|m| m.from_email.clone()).unwrap_or_default(),
            to_list,
            cc_list,
            bcc_list,
            date: msg.as_ref().map(|m| m.date.clone()),
            plain_text: body.plain_text,
            sanitized_html,
            attachments,
            is_read: msg.as_ref().map(|m| m.is_read).unwrap_or(false),
            is_flagged: msg.as_ref().map(|m| m.is_flagged).unwrap_or(false),
            list_unsubscribe: list_unsub,
            list_unsubscribe_post: list_unsub_post,
            message_id,
            references,
            in_reply_to,
        });
    }

    log::info!("fetch_message_body: cache MISS for uid={}, fetching from IMAP...", uid);

    let raw_body = fetch_raw_message(state.inner(), &account_id, &folder, uid).await?;

    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parser::parse_message(&raw_body)
    }))
    .map_err(|_| AppError::Imap("Failed to parse message (internal error)".to_string()))?;

    persist_body(state.inner(), &account_id, &folder, uid, &parsed)?;

    // Read unsubscribe headers and message headers from DB
    let (list_unsub, list_unsub_post, message_id, references, in_reply_to) = {
        let conn = state.db.safe_lock();
        let (unsub, unsub_post) = db::messages::get_unsubscribe_headers(&conn, &account_id, &folder, uid).unwrap_or((None, None));
        let (msg_id, refs, irt) = db::messages::get_message_headers(&conn, &account_id, &folder, uid)
            .unwrap_or((None, None, None));
        (unsub, unsub_post, msg_id, refs, irt)
    };

    Ok(MessageDetail {
        uid,
        subject: parsed.subject,
        from_name: parsed.from_name,
        from_email: parsed.from_email,
        to_list: parsed.to_list,
        cc_list: parsed.cc_list,
        bcc_list: parsed.bcc_list,
        date: parsed.date,
        plain_text: parsed.plain_text,
        sanitized_html: parsed.sanitized_html,
        attachments: parsed.attachments,
        is_read: true,
        is_flagged: false,
        message_id,
        references,
        in_reply_to,
        list_unsubscribe: list_unsub,
        list_unsubscribe_post: list_unsub_post,
    })
}

/// Read-only fetch of a cached message body for passive UI surfaces
/// (e.g. hover tooltips). Never touches IMAP, never writes to the DB, never
/// runs repair. Returns `Ok(None)` on cache miss so the caller can fall back
/// to whatever it had (typically a snippet) without triggering network or
/// write amplification.
#[tauri::command]
pub async fn get_cached_message_body(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<Option<MessageDetail>, AppError> {
    let cached = {
        let conn = state.open_read_conn()?;
        db::messages::get_body(&conn, &account_id, &folder, uid)?
    };

    let Some(body) = cached else {
        return Ok(None);
    };

    let (
        msg,
        list_unsub,
        list_unsub_post,
        to_list,
        cc_list,
        bcc_list,
        attachments,
        message_id,
        references,
        in_reply_to,
    ) = {
        let conn = state.open_read_conn()?;
        let msg = db::messages::get_by_uid(&conn, &account_id, &folder, uid)?;
        let (list_unsub, list_unsub_post) =
            db::messages::get_unsubscribe_headers(&conn, &account_id, &folder, uid)?;
        let (to_json, cc_json, bcc_json) = db::messages::get_address_lists(&conn, &account_id, &folder, uid)?;
        let to_list: Vec<parser::EmailAddress> = to_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let cc_list: Vec<parser::EmailAddress> = cc_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let bcc_list: Vec<parser::EmailAddress> = bcc_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let attachments = db::messages::get_attachments(&conn, &account_id, &folder, uid)?;
        let (message_id, references, in_reply_to) =
            db::messages::get_message_headers(&conn, &account_id, &folder, uid)?;
        (
            msg,
            list_unsub,
            list_unsub_post,
            to_list,
            cc_list,
            bcc_list,
            attachments,
            message_id,
            references,
            in_reply_to,
        )
    };

    Ok(Some(MessageDetail {
        uid,
        subject: msg.as_ref().and_then(|m| m.subject.clone()),
        from_name: msg.as_ref().and_then(|m| m.from_name.clone()),
        from_email: msg
            .as_ref()
            .and_then(|m| m.from_email.clone())
            .unwrap_or_default(),
        to_list,
        cc_list,
        bcc_list,
        date: msg.as_ref().map(|m| m.date.clone()),
        plain_text: body.plain_text,
        sanitized_html: body.sanitized_html,
        attachments,
        is_read: msg.as_ref().map(|m| m.is_read).unwrap_or(false),
        is_flagged: msg.as_ref().map(|m| m.is_flagged).unwrap_or(false),
        list_unsubscribe: list_unsub,
        list_unsubscribe_post: list_unsub_post,
        message_id,
        references,
        in_reply_to,
    }))
}

#[tauri::command]
pub async fn sync_folder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    account_id: String,
    folder: String,
) -> Result<SyncResult, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let execution = run_folder_sync_execution(
        state.inner(),
        &app,
        &account,
        &folder,
        sync::SyncMode::Full,
        3,
    )
    .await?;

    store_prefetched_bodies(state.inner(), &account.id, &folder, &execution.parsed_bodies);

    // Backfill snippets AFTER store_prefetched_bodies so newly inserted bodies
    // get clean_snippet applied in the same sync cycle.
    {
        let conn = state.db.safe_lock();
        if let Err(e) = db::messages::backfill_snippets_from_bodies(&conn) {
            log::error!("Failed to backfill snippets: {e}");
        }
    }

    prefetch_missing_snippets(state.inner(), &account, &folder).await;

    crate::notify::update_badge(&app);

    Ok(SyncResult {
        new_count: execution.new_count,
        updated_count: execution.updated_count,
        account_statuses: vec![AccountSyncStatus {
            account_id: account.id.clone(),
            email: account.email.clone(),
            success: true,
            error: None,
            new_count: execution.new_count,
            needs_reauth: false,
        }],
    })
}

#[tauri::command]
pub async fn mark_as_read(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
) -> Result<(), AppError> {
    {
        let conn = state.db.safe_lock();
        for uid in &uids {
            db::messages::update_flags(&conn, &account_id, &folder, *uid, true)?;
        }
    }

    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let folder_clone = folder.clone();
    let uids_clone = uids.clone();
    // Fire and forget - don't block UI on IMAP flag update
    tokio::spawn(async move {
        if let Ok(mut session) = imap::connect_for_account(&account).await {
            if let Err(e) = imap::select_folder(&mut session, &folder_clone).await {
                log::error!("Failed to select folder {folder_clone} for mark-as-read: {e}");
            }
            for uid in &uids_clone {
                if let Err(e) = imap::store_flags(&mut session, *uid, true, "\\Seen").await {
                    log::error!("Failed to set \\Seen flag on message {uid}: {e}");
                }
            }
            let _ = imap::disconnect(session, &account.email).await;
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn mark_as_unread(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
) -> Result<(), AppError> {
    {
        let conn = state.db.safe_lock();
        for uid in &uids {
            db::messages::update_flags(&conn, &account_id, &folder, *uid, false)?;
        }
    }

    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let folder_clone = folder.clone();
    let uids_clone = uids.clone();
    tokio::spawn(async move {
        if let Ok(mut session) = imap::connect_for_account(&account).await {
            if let Err(e) = imap::select_folder(&mut session, &folder_clone).await {
                log::error!("Failed to select folder {folder_clone} for mark-as-unread: {e}");
            }
            for uid in &uids_clone {
                if let Err(e) = imap::store_flags(&mut session, *uid, false, "\\Seen").await {
                    log::error!("Failed to remove \\Seen flag on message {uid}: {e}");
                }
            }
            let _ = imap::disconnect(session, &account.email).await;
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn fetch_unified_inbox(
    state: State<'_, AppState>,
    page: u32,
    page_size: u32,
    category: Option<String>,
    account_ids: Option<Vec<String>>,
    unread_only: Option<bool>,
) -> Result<MessagePage, AppError> {
    let conn = state.open_read_conn()?;
    let (messages, total) = db::messages::list_all_inboxes(
        &conn,
        page,
        page_size,
        category.as_deref(),
        account_ids.as_deref(),
        unread_only.unwrap_or(false),
    )?;
    let has_more = (page + 1) * page_size < total;
    Ok(MessagePage {
        messages,
        total,
        page,
        page_size,
        has_more,
    })
}

/// RAII guard that clears `state.sync_in_flight` on drop. Ensures the flag
/// is reset even if `sync_all_inboxes_inner` returns early or panics.
struct SyncInFlightGuard<'a> {
    state: &'a AppState,
}
impl<'a> Drop for SyncInFlightGuard<'a> {
    fn drop(&mut self) {
        self.state
            .sync_in_flight
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

#[tauri::command]
pub async fn sync_all_inboxes(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<SyncResult, AppError> {
    sync_all_inboxes_inner(state.inner(), &app).await
}

/// Inner sync used by both the IPC command and the backend periodic safety-net
/// sync in `lib.rs`. Holds an atomic in-flight flag so the two callers can't
/// race each other.
pub async fn sync_all_inboxes_inner(
    state: &AppState,
    app: &tauri::AppHandle,
) -> Result<SyncResult, AppError> {
    use std::sync::atomic::Ordering;
    if state
        .sync_in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        log::info!("sync_all_inboxes: another sync already in flight, skipping");
        return Ok(SyncResult {
            new_count: 0,
            updated_count: 0,
            account_statuses: Vec::new(),
        });
    }
    let _guard = SyncInFlightGuard { state };

    log::info!("sync_all_inboxes: starting");
    let accounts = {
        let conn = state.db.safe_lock();
        db::accounts::list(&conn)?
    };
    log::info!("sync_all_inboxes: got {} accounts", accounts.len());

    let mut total_new = 0u32;
    let mut total_updated = 0u32;
    let mut account_statuses = Vec::new();

    for (i, account) in accounts.iter().enumerate() {
        // Skip an account whose connects are being refused, instead of burning
        // a 15s timeout on it every ~3.75 minutes. This is the half of the
        // backoff that the IDLE loop cannot provide: with only that loop backing
        // off, a refused account still took ~16 attempts an hour from here, on a
        // flat cadence that never decays — enough to keep renewing a
        // server-side throttle. The breaker is advisory and background-only;
        // nothing a user does is gated by it.
        if let Some(remaining) = imap::background_cooldown_remaining(&account.email) {
            log::info!(
                "sync_all_inboxes: [{}/{}] skipping {} — cooling down for {}s after {} consecutive connect failures",
                i + 1,
                accounts.len(),
                account.email,
                remaining.as_secs(),
                imap::connect_failure_streak(&account.email),
            );
            // Deliberately NOT pushed to `account_statuses`: a skip is not a
            // sync failure, and reporting it as one would fire the "Sync failed
            // for X" toast on every tick for an account we are choosing not to
            // contact. The last real error is already recorded in sync_state.
            continue;
        }
        log::info!("sync_all_inboxes: [{}/{}] starting {}", i + 1, accounts.len(), account.email);
        let has_folders = {
            let conn = state.db.safe_lock();
            let folders = db::folders::list_by_account(&conn, &account.id)?;
            !folders.is_empty()
        };

        // Per-account timeout: if one account's IMAP hangs, skip it and continue.
        // Budget is shared by three folder syncs (INBOX, Drafts, Sent), so it
        // was raised from 60s to 90s — INBOX runs first and must not be starved
        // by the two that follow it. They now share ONE connection, so the
        // budget also covers one connect instead of three; if it ever needs
        // revisiting, that made it roomier, not tighter.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(90),
            async {
                // Auto-sync folders for accounts that don't have any yet
                if !has_folders {
                    log::info!("Account {} has no folders, syncing folder list...", account.email);
                    let mut session = imap::connect_for_account(account).await?;
                    match imap::list_folders(&mut session, &account.provider).await {
                        Ok(imap_folders) => {
                            let conn = state.db.safe_lock();
                            for folder in &imap_folders {
                                if let Err(e) = db::folders::upsert(&conn, &account.id, &folder.name, None, &folder.folder_type, folder.delimiter.as_deref(), folder.special_use) {
                                    log::warn!("Failed to upsert folder {}: {e}", folder.name);
                                }
                            }
                            log::info!("Synced {} folders for {}", imap_folders.len(), account.email);
                        }
                        Err(e) => log::warn!("Failed to list folders for {}: {e}", account.email),
                    }
                    let _ = imap::disconnect(session, &account.email).await;
                }

                // ONE session for all three folders. Opening a connection per
                // folder tripled this account's connect rate against Gmail for
                // no benefit — IMAP is happy to SELECT a second folder on the
                // same session, which is what `sync_account_folders` has always
                // done. Closed via `imap::disconnect` at the end of this block,
                // including on the error paths, so the ledger never sees an
                // `abandoned` session from here (gotcha #46).
                let mut session = imap::connect_for_account(account).await?;

                let inbox_result = run_folder_sync_on_session(
                    state,
                    app,
                    account,
                    "INBOX",
                    sync::SyncMode::Limited,
                    10,
                    &mut session,
                )
                .await;

                // INBOX is the contractual sync target: if it failed, close the
                // session and report the failure rather than pressing on to the
                // two best-effort folders on a session that just misbehaved.
                let execution = match inbox_result {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = imap::disconnect(session, &account.email).await;
                        return Err(e);
                    }
                };

                store_prefetched_bodies(state, &account.id, "INBOX", &execution.parsed_bodies);

                // Drafts sync: pick up drafts created in Gmail web, mail.app, or
                // another CXMail session so search reflects server truth. Failure
                // here is logged but doesn't fail the account-level result —
                // INBOX is the contractual sync target.
                let drafts_folder = {
                    let conn = state.db.safe_lock();
                    db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts")
                };
                match run_folder_sync_on_session(
                    state,
                    app,
                    account,
                    &drafts_folder,
                    sync::SyncMode::Limited,
                    10,
                    &mut session,
                )
                .await
                {
                    Ok(drafts_execution) => {
                        store_prefetched_bodies(
                            state,
                            &account.id,
                            &drafts_folder,
                            &drafts_execution.parsed_bodies,
                        );
                    }
                    Err(e) => log::warn!(
                        "sync_all_inboxes: Drafts sync for {} ({}) failed: {}",
                        account.email, drafts_folder, e
                    ),
                }

                // Sent sync: follow-up nudges are computed from "my last message
                // in this thread", so a stale Sent folder makes them blind over
                // exactly the window they cover — before this ran, the newest
                // local Sent message was 5 days behind the mailbox. Like Drafts,
                // failure is logged but doesn't fail the account: INBOX is the
                // contractual sync target.
                //
                // The body prefetch is deliberately small. Nudges need only
                // metadata; the budget belongs to INBOX, where the user is
                // actually reading. A few warm bodies keep recent sends
                // quotable without paying for the whole folder.
                let sent_folder = {
                    let conn = state.db.safe_lock();
                    db::folders::folder_for_account(&conn, &account.id, &account.provider, "sent")
                };
                match run_folder_sync_on_session(
                    state,
                    app,
                    account,
                    &sent_folder,
                    sync::SyncMode::Limited,
                    5,
                    &mut session,
                )
                .await
                {
                    Ok(sent_execution) => {
                        store_prefetched_bodies(
                            state,
                            &account.id,
                            &sent_folder,
                            &sent_execution.parsed_bodies,
                        );
                    }
                    Err(e) => log::warn!(
                        "sync_all_inboxes: Sent sync for {} ({}) failed: {}",
                        account.email, sent_folder, e
                    ),
                }

                let _ = imap::disconnect(session, &account.email).await;

                Ok::<(u32, u32), AppError>((execution.new_count, execution.updated_count))
            }
        )
        .await;

        match result {
            Ok(Ok((new_count, updated_count))) => {
                log::info!(
                    "sync_all_inboxes: [{}/{}] {} done, {} new messages, {} updates",
                    i + 1,
                    accounts.len(),
                    account.email,
                    new_count,
                    updated_count
                );
                total_new += new_count;
                total_updated += updated_count;
                account_statuses.push(AccountSyncStatus {
                    account_id: account.id.clone(),
                    email: account.email.clone(),
                    success: true,
                    error: None,
                    new_count,
                    needs_reauth: false,
                });
            }
            Ok(Err(e)) => {
                let needs_reauth = matches!(e, AppError::ReauthRequired(_));
                let error_msg = format!("{}", e);
                log::error!("sync_all_inboxes: [{}/{}] {} failed: {}", i + 1, accounts.len(), account.email, error_msg);
                let conn = state.db.safe_lock();
                let _ = db::sync_state::record_error(&conn, &account.id, "INBOX", &error_msg);
                account_statuses.push(AccountSyncStatus {
                    account_id: account.id.clone(),
                    email: account.email.clone(),
                    success: false,
                    error: Some(error_msg),
                    new_count: 0,
                    needs_reauth,
                });
            }
            Err(_) => {
                let error_msg = "Timed out after 60s".to_string();
                log::error!("sync_all_inboxes: [{}/{}] {} timed out after 60s, skipping", i + 1, accounts.len(), account.email);
                let conn = state.db.safe_lock();
                let _ = db::sync_state::record_error(&conn, &account.id, "INBOX", &error_msg);
                account_statuses.push(AccountSyncStatus {
                    account_id: account.id.clone(),
                    email: account.email.clone(),
                    success: false,
                    error: Some(error_msg),
                    new_count: 0,
                    needs_reauth: false,
                });
            }
        }

        // Emit per-account event so the frontend can refresh incrementally
        let _ = app.emit("sync-account-done", account_statuses.last().unwrap());
    }

    log::info!("sync_all_inboxes: all accounts done, {} total new messages", total_new);

    // Backfill snippets from any cached message bodies
    {
        let conn = state.db.safe_lock();
        if let Err(e) = db::messages::backfill_snippets_from_bodies(&conn) {
            log::error!("Failed to backfill snippets: {e}");
        }
    }

    // Update dock badge and tray tooltip with final unread count
    crate::notify::update_badge(app);

    Ok(SyncResult {
        new_count: total_new,
        updated_count: total_updated,
        account_statuses,
    })
}

#[tauri::command]
pub async fn force_full_sync(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<SyncResult, AppError> {
    log::info!("force_full_sync: resetting checkpoints for {:?}", account_id);

    {
        let conn = state.db.safe_lock();
        let accounts = match &account_id {
            Some(id) => vec![id.clone()],
            None => db::accounts::list(&conn)?
                .into_iter()
                .map(|a| a.id)
                .collect(),
        };
        for aid in &accounts {
            db::sync_state::reset_for_uidvalidity_change(&conn, aid, "INBOX")?;
        }
    }

    sync_all_inboxes(state, app).await
}

#[tauri::command]
pub async fn move_messages(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
    destination: String,
) -> Result<(), AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    let _ = imap::select_folder(&mut session, &folder).await?;
    for uid in &uids {
        imap::move_message(&mut session, *uid, &destination).await?;
    }
    let _ = imap::disconnect(session, &account.email).await;

    // Remove from local cache
    {
        let conn = state.db.safe_lock();
        for uid in &uids {
            conn.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                rusqlite::params![account_id, folder, uid],
            )?;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn archive_messages(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
) -> Result<(), AppError> {
    // Ask the synced folder list first — it is the only thing that knows where
    // a generic IMAP server keeps its archive. The hardcoded match stays as the
    // fallback: it covers the fresh-account window before the first folder sync
    // (and a folder sync that failed), where the DB has nothing to say.
    let archive_folder = {
        let conn = state.db.safe_lock();
        let account = db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
        db::folders::folder_for_account(&conn, &account_id, &account.provider, "archive")
    };

    move_messages(state, account_id, folder, uids, archive_folder).await
}

#[tauri::command]
pub async fn delete_messages(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
) -> Result<(), AppError> {
    // DB first, hardcoded match as the pre-folder-sync fallback. See the note
    // in `archive_messages` — and gotcha #41 for why the DB lookup is ordered.
    let trash_folder = {
        let conn = state.db.safe_lock();
        let account = db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
        db::folders::folder_for_account(&conn, &account_id, &account.provider, "trash")
    };

    move_messages(state, account_id, folder, uids, trash_folder).await
}

#[tauri::command]
pub async fn toggle_star(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    starred: bool,
) -> Result<(), AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    // Update local DB
    {
        let conn = state.db.safe_lock();
        conn.execute(
            "UPDATE messages SET is_flagged = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
            rusqlite::params![starred as i32, account_id, folder, uid],
        )?;
    }

    // Update IMAP in background
    let folder_clone = folder.clone();
    tokio::spawn(async move {
        if let Ok(mut session) = imap::connect_for_account(&account).await {
            if let Err(e) = imap::select_folder(&mut session, &folder_clone).await {
                log::error!("Failed to select folder {folder_clone} for toggle-star: {e}");
            }
            if let Err(e) = imap::store_flags(&mut session, uid, starred, "\\Flagged").await {
                log::error!("Failed to update \\Flagged flag on message {uid}: {e}");
            }
            let _ = imap::disconnect(session, &account.email).await;
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn toggle_mute(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    muted: bool,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::messages::update_muted(&conn, &account_id, &folder, uid, muted)?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_pin(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    pinned: bool,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::messages::update_pinned(&conn, &account_id, &folder, uid, pinned)?;
    Ok(())
}

/// Plain FTS search — the SearchBar dropdown path. Local, cheap, no LLM.
#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    account_ids: Option<Vec<String>>,
    prefix: Option<bool>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<db::search::SearchResult>, AppError> {
    let filters = db::search::SearchFilters {
        keywords: Some(query),
        account_ids,
        ..Default::default()
    };
    // Read-only connection: searches never contend with the sync DB mutex.
    let conn = state.open_read_conn()?;
    db::search::search(
        &conn,
        &filters,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        prefix.unwrap_or(false),
    )
}

/// One chip of the applied-filter row shown above search results.
#[derive(Serialize)]
pub struct AppliedFilter {
    pub kind: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct AiSearchResponse {
    pub results: Vec<db::search::SearchResult>,
    pub applied_filters: Vec<AppliedFilter>,
    /// True when the LLM parse contributed to the filters.
    pub used_ai: bool,
    /// True when the LLM path was attempted but failed (timeout/parse error).
    pub ai_failed: bool,
}

fn applied_filters_from(f: &db::search::SearchFilters) -> Vec<AppliedFilter> {
    let mut chips = Vec::new();
    let mut push = |kind: &str, value: String| {
        chips.push(AppliedFilter {
            kind: kind.to_string(),
            value,
        })
    };
    if let Some(ref v) = f.from {
        push("from", v.clone());
    }
    if let Some(ref v) = f.to {
        push("to", v.clone());
    }
    if let Some(ref v) = f.cc {
        push("cc", v.clone());
    }
    if let Some(ref v) = f.subject_contains {
        push("subject", v.clone());
    }
    if let Some(ref v) = f.filename {
        push("filename", v.clone());
    }
    if let Some(ref v) = f.date_after {
        push("after", v.split('T').next().unwrap_or(v).to_string());
    }
    if let Some(ref v) = f.date_before {
        push("before", v.split('T').next().unwrap_or(v).to_string());
    }
    if let Some(ref v) = f.folder {
        push("in", v.clone());
    }
    if f.has_attachments == Some(true) {
        push("has", "attachment".to_string());
    }
    if f.is_starred == Some(true) {
        push("is", "starred".to_string());
    }
    match f.is_unread {
        Some(true) => push("is", "unread".to_string()),
        Some(false) => push("is", "read".to_string()),
        None => {}
    }
    if let Some(v) = f.larger_bytes {
        push("larger", v.to_string());
    }
    if let Some(v) = f.smaller_bytes {
        push("smaller", v.to_string());
    }
    if let Some(ref v) = f.keywords {
        push("keywords", v.clone());
    }
    chips
}

/// Merge the LLM parse into deterministically-parsed filters. Deterministic
/// operator fields win on conflict; the LLM fills gaps. Keywords are the one
/// exception: when the LLM ran, its keywords replace the raw free text (it
/// strips filler like "emails from sarah about the…" down to "budget").
fn merge_llm_filters(
    det: &mut db::search::SearchFilters,
    llm: crate::email::ai::ParsedSearchQuery,
) {
    if det.from.is_none() {
        det.from = llm.from.filter(|s| !s.trim().is_empty());
    }
    if det.to.is_none() {
        det.to = llm.to.filter(|s| !s.trim().is_empty());
    }
    if det.subject_contains.is_none() {
        det.subject_contains = llm.subject_contains.filter(|s| !s.trim().is_empty());
    }
    if det.date_after.is_none() {
        det.date_after = llm.date_after.filter(|s| !s.trim().is_empty());
    }
    if det.date_before.is_none() {
        // LLM date_before is an inclusive YYYY-MM-DD — extend to end of day
        // (a bare date normalizes to midnight, i.e. exclusive).
        det.date_before = llm
            .date_before
            .filter(|s| !s.trim().is_empty())
            .map(|d| if d.len() == 10 { format!("{d}T23:59:59") } else { d });
    }
    if det.folder.is_none() {
        det.folder = llm.folder.filter(|s| !s.trim().is_empty());
    }
    if det.has_attachments.is_none() {
        det.has_attachments = llm.has_attachments;
    }
    if det.is_starred.is_none() {
        det.is_starred = llm.is_starred;
    }
    if det.is_unread.is_none() {
        det.is_unread = llm.is_unread;
    }
    det.keywords = llm
        .keywords
        .filter(|s| !s.trim().is_empty())
        .or_else(|| det.keywords.take());
}

/// Natural-language search: deterministic operator/date parse first, then an
/// optional LLM pass for free-form queries. Everything ANDs through the
/// single FTS engine — no more union of keyword and metadata hits.
#[tauri::command]
pub async fn ai_search_messages(
    state: State<'_, AppState>,
    query: String,
    account_ids: Option<Vec<String>>,
) -> Result<AiSearchResponse, AppError> {
    use crate::email;

    let today = chrono::Local::now().date_naive();
    let parsed = email::search_parse::parse(&query, today);
    let mut filters = parsed.filters;

    // The LLM adds value only for multi-word free text with no explicit
    // operators (an operator query means the user already speaks the syntax).
    let word_count = query.split_whitespace().count();
    let mut used_ai = false;
    let mut ai_failed = false;
    if !parsed.found_operators && word_count >= 3 {
        if let Ok(client) = email::inference::InferenceClient::load() {
                match email::ai::parse_search_query(&client, &query).await {
                    Ok(llm) => {
                        merge_llm_filters(&mut filters, llm);
                        used_ai = true;
                    }
                    Err(e) => {
                        log::warn!("ai_search: LLM parse failed, deterministic only: {e}");
                        ai_failed = true;
                    }
                }
        }
    }

    if filters.account_ids.is_none() {
        filters.account_ids = account_ids;
    }

    let results = {
        let conn = state.open_read_conn()?;
        db::search::search(&conn, &filters, 50, 0, false)?
    };
    Ok(AiSearchResponse {
        applied_filters: applied_filters_from(&filters),
        results,
        used_ai,
        ai_failed,
    })
}

/// Re-run a structured search after the user edits filter chips — never
/// re-invokes the LLM.
#[tauri::command]
pub async fn search_with_filters(
    state: State<'_, AppState>,
    filters: db::search::SearchFilters,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<db::search::SearchResult>, AppError> {
    let conn = state.open_read_conn()?;
    db::search::search(
        &conn,
        &filters,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        false,
    )
}

/// Per-account cap on server-search hits ingested per query. The newest N
/// UIDs are kept; anything older is dropped (logged, not silent).
const SERVER_SEARCH_MAX_HITS: usize = 50;

/// IMAP server-side search fallback — closes the coverage gap between the
/// local cache (INBOX recent + Drafts + browsed folders) and the full server
/// mailbox. Gmail searches `[Gmail]/All Mail` with `X-GM-RAW` (Gmail's own
/// engine); iCloud/Outlook iterate synced folders with standard SEARCH keys.
/// Hits not in the local DB are ingested as header rows (the FTS triggers
/// index them → permanently searchable locally), then the local search
/// re-runs and returns merged results.
#[tauri::command]
pub async fn server_search(
    state: State<'_, AppState>,
    mut filters: db::search::SearchFilters,
    account_ids: Option<Vec<String>>,
) -> Result<Vec<db::search::SearchResult>, AppError> {
    // The scope arrives as the separate `account_ids` argument (the frontend
    // never sets `filters.account_ids`), so fold it into the filters HERE —
    // otherwise the final local re-run below is unscoped and merges rows from
    // every account into a search the user scoped to a few.
    filters.account_ids = filters.account_ids.take().or(account_ids);
    let scope = filters.account_ids.clone();

    // Unscoped = every VISIBLE account, the same rule `db::search::search`
    // applies to `None`; naming an account (the search bar inside it) is the
    // one way a hidden account's server is searched.
    let accounts: Vec<db::accounts::Account> = {
        let conn = state.db.safe_lock();
        db::accounts::list(&conn)?
            .into_iter()
            .filter(|a| match &scope {
                Some(ids) => ids.contains(&a.id),
                None => !a.hidden_from_aggregates,
            })
            .collect()
    };

    // Per-account, concurrently, each under its own 25s budget so one dead
    // server can't stall the rest (gotcha #10 posture).
    let mut tasks = Vec::new();
    for account in accounts {
        let filters = filters.clone();
        let state_ref: &AppState = state.inner();
        tasks.push(async move {
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                server_search_one_account(state_ref, &account, &filters),
            )
            .await;
            match outcome {
                Ok(Ok(ingested)) => {
                    log::info!(
                        "server_search: {} done, {} new message(s) ingested",
                        account.email,
                        ingested
                    );
                }
                Ok(Err(e)) => log::warn!("server_search: {} failed: {e}", account.email),
                Err(_) => log::warn!("server_search: {} timed out after 25s", account.email),
            }
        });
    }
    futures::future::join_all(tasks).await;

    // Re-run locally — server hits are now indexed rows.
    let conn = state.open_read_conn()?;
    db::search::search(&conn, &filters, 50, 0, false)
}

/// Search one account's server mailbox and ingest unknown hits as header
/// rows under their real folder. Returns the number of ingested messages.
async fn server_search_one_account(
    state: &AppState,
    account: &db::accounts::Account,
    filters: &db::search::SearchFilters,
) -> Result<u32, AppError> {
    use crate::email::server_search as ss;

    let mut session = imap::connect_for_account(account).await?;
    // (folder, query) targets per provider.
    let targets: Vec<String> = if account.provider == "gmail" {
        vec!["[Gmail]/All Mail".to_string()]
    } else {
        let conn = state.db.safe_lock();
        db::folders::list_by_account(&conn, &account.id)?
            .into_iter()
            .filter(|f| {
                !matches!(
                    f.folder_type.as_deref(),
                    Some("trash") | Some("spam")
                )
            })
            .map(|f| f.name)
            .collect()
    };
    let query = if account.provider == "gmail" {
        ss::build_gmail_uid_search(filters)
    } else {
        ss::build_imap_search_query(filters)
    };

    let mut total_ingested = 0u32;
    for folder in &targets {
        if imap::select_folder(&mut session, folder).await.is_err() {
            continue;
        }
        let mut hits = match imap::search_folder(&mut session, &query).await {
            Ok(h) => h,
            Err(e) => {
                log::warn!(
                    "server_search: SEARCH failed in {}/{}: {e}",
                    account.email,
                    folder
                );
                continue;
            }
        };
        if hits.len() > SERVER_SEARCH_MAX_HITS {
            log::info!(
                "server_search: {}/{} returned {} hits, keeping newest {}",
                account.email,
                folder,
                hits.len(),
                SERVER_SEARCH_MAX_HITS
            );
            let start = hits.len() - SERVER_SEARCH_MAX_HITS;
            hits = hits.split_off(start);
        }
        if hits.is_empty() {
            continue;
        }

        // Only fetch headers for UIDs the local DB doesn't know yet.
        let unknown: Vec<u32> = {
            let conn = state.db.safe_lock();
            let known = db::messages::get_uids_for_folder(&conn, &account.id, folder)
                .unwrap_or_default()
                .into_iter()
                .collect::<std::collections::HashSet<u32>>();
            hits.into_iter().filter(|u| !known.contains(u)).collect()
        };
        if unknown.is_empty() {
            continue;
        }

        let headers = imap::fetch_headers_for_uids(&mut session, &unknown).await?;
        let rows: Vec<_> = headers
            .iter()
            .map(|h| {
                (
                    h.uid,
                    h.subject.as_deref(),
                    h.from_name.as_deref(),
                    h.from_email.as_deref(),
                    h.date.as_deref().unwrap_or(""),
                    None::<&str>,
                    "[]",
                    h.message_id.as_deref(),
                    h.in_reply_to.as_deref(),
                    h.references.as_deref(),
                    h.is_read,
                    h.is_flagged,
                    false,
                    h.size as i64,
                    h.list_unsubscribe.as_deref(),
                    h.list_unsubscribe_post.as_deref(),
                    h.to_list.as_str(),
                    h.cc_list.as_str(),
                )
            })
            .collect();
        {
            let conn = state.db.safe_lock();
            db::messages::insert_batch(&conn, &account.id, folder, &rows)?;
            sync::record_delivered_to(&conn, &account.id, folder, &headers);
        }
        total_ingested += headers.len() as u32;
    }
    let _ = imap::disconnect(session, &account.email).await;
    Ok(total_ingested)
}

#[tauri::command]
pub async fn get_thread(
    state: State<'_, AppState>,
    account_id: String,
    message_id: String,
) -> Result<Vec<db::messages::MessageRow>, AppError> {
    let conn = state.open_read_conn()?;
    db::messages::get_thread(&conn, &account_id, &message_id)
}

#[tauri::command]
pub async fn get_thread_uids_in_folder(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    thread_root_id: String,
) -> Result<Vec<u32>, AppError> {
    let conn = state.open_read_conn()?;
    db::messages::get_thread_uids_in_folder(&conn, &account_id, &folder, &thread_root_id)
}

#[tauri::command]
pub async fn search_contacts(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<ContactResult>, AppError> {
    let conn = state.open_read_conn()?;
    let contacts = db::messages::search_contacts(&conn, &query, 10)?;
    Ok(contacts
        .into_iter()
        .map(|(name, email)| ContactResult { name, email })
        .collect())
}

/// Result of an unsubscribe attempt, returned as tagged JSON to the frontend.
#[derive(serde::Serialize)]
#[serde(tag = "type")]
pub enum UnsubscribeResult {
    #[serde(rename = "one-click")]
    OneClick { confirmed: bool },
    #[serde(rename = "browser")]
    Browser { url: String },
    #[serde(rename = "mailto")]
    Mailto {
        email: String,
        subject: Option<String>,
        body: Option<String>,
    },
}

/// Simple percent-decode for mailto query params.
///
/// Accumulates BYTES and decodes once at the end. `u8 as char` is a Latin-1
/// decode, so pushing decoded bytes straight into a String split every non-ASCII
/// character into its individual UTF-8 bytes and re-encoded each one — an
/// unsubscribe `mailto:` carrying a percent-encoded subject came out mojibake'd.
/// Same defect that corrupted snippets in `email::parser::html_to_plain_text`.
fn percent_decode(s: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut chars = s.as_bytes().iter();
    while let Some(&b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().copied().unwrap_or(0);
            let lo = chars.next().copied().unwrap_or(0);
            if let (Some(h), Some(l)) = (
                (hi as char).to_digit(16),
                (lo as char).to_digit(16),
            ) {
                out.push((h * 16 + l) as u8);
            } else {
                out.push(b'%');
                out.push(hi);
                out.push(lo);
            }
        } else if b == b'+' {
            out.push(b' ');
        } else {
            out.push(b);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a mailto: URL into (email, subject, body).
fn parse_mailto(raw: &str) -> (String, Option<String>, Option<String>) {
    let without_scheme = raw.strip_prefix("mailto:").unwrap_or(raw);
    let (email, query) = match without_scheme.split_once('?') {
        Some((e, q)) => (e.to_string(), Some(q)),
        None => (without_scheme.to_string(), None),
    };
    let mut subject = None;
    let mut body = None;
    if let Some(q) = query {
        for param in q.split('&') {
            if let Some((key, val)) = param.split_once('=') {
                match key.to_lowercase().as_str() {
                    "subject" => subject = Some(percent_decode(val)),
                    "body" => body = Some(percent_decode(val)),
                    _ => {}
                }
            }
        }
    }
    (percent_decode(&email), subject, body)
}

/// Scan email HTML body for unsubscribe links when no List-Unsubscribe header exists.
fn find_unsubscribe_link_in_body(
    conn: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Option<String> {
    let body = db::messages::get_body(conn, account_id, folder, uid).ok()??;
    let html = body.sanitized_html?;

    let re = regex::Regex::new(
        r#"(?i)<a\s[^>]*href\s*=\s*["']([^"']+)["'][^>]*>[^<]*(?:unsub|opt[\s\-]?out|manage\s+(?:email|subscription)|email\s+preferences)[^<]*</a>"#
    ).ok()?;

    let mut https_link: Option<String> = None;
    let mut http_link: Option<String> = None;

    for cap in re.captures_iter(&html) {
        let url = cap.get(1)?.as_str();
        if url.starts_with("https://") {
            https_link = Some(url.to_string());
            break;
        } else if url.starts_with("http://") && http_link.is_none() {
            http_link = Some(url.to_string());
        }
    }

    https_link.or(http_link)
}

/// Attempt to unsubscribe from a mailing list.
#[tauri::command]
pub async fn unsubscribe(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<UnsubscribeResult, AppError> {
    let (list_unsub, list_unsub_post) = {
        let conn = state.db.safe_lock();
        db::messages::get_unsubscribe_headers(&conn, &account_id, &folder, uid)?
    };

    // If no List-Unsubscribe header, try body link fallback
    let header = match list_unsub {
        Some(h) => h,
        None => {
            let conn = state.db.safe_lock();
            if let Some(url) = find_unsubscribe_link_in_body(&conn, &account_id, &folder, uid) {
                return Ok(UnsubscribeResult::Browser { url });
            }
            return Err(AppError::NotFound("No unsubscribe option found".to_string()));
        }
    };

    // Parse URLs from angle brackets: <https://...>, <mailto:...>
    let urls: Vec<&str> = header
        .split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.starts_with('<') && trimmed.ends_with('>') {
                Some(&trimmed[1..trimmed.len() - 1])
            } else {
                None
            }
        })
        .collect();

    let https_url = urls.iter().find(|u| u.starts_with("https://")).copied();
    let mailto_url = urls.iter().find(|u| u.starts_with("mailto:")).copied();

    // RFC 8058: one-click unsubscribe via POST with retry
    let has_one_click = list_unsub_post
        .as_ref()
        .map(|p| p.contains("List-Unsubscribe=One-Click"))
        .unwrap_or(false);

    if has_one_click {
        if let Some(url) = https_url {
            let client = reqwest::Client::new();

            for attempt in 0..3u8 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }

                let result = client
                    .post(url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body("List-Unsubscribe=One-Click")
                    .send()
                    .await;

                match result {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            // Auto-record sender as unsubscribed
                            let conn = state.db.safe_lock();
                            if let Ok(Some(msg)) = db::messages::get_by_uid(&conn, &account_id, &folder, uid) {
                                if let Some(ref email) = msg.from_email {
                                    let _ = db::unsubscribed::record(&conn, &account_id, email, "one-click");
                                }
                            }
                            return Ok(UnsubscribeResult::OneClick { confirmed: true });
                        } else if status.is_redirection() {
                            let conn = state.db.safe_lock();
                            if let Ok(Some(msg)) = db::messages::get_by_uid(&conn, &account_id, &folder, uid) {
                                if let Some(ref email) = msg.from_email {
                                    let _ = db::unsubscribed::record(&conn, &account_id, email, "one-click");
                                }
                            }
                            return Ok(UnsubscribeResult::OneClick { confirmed: false });
                        } else if status.is_client_error() {
                            break; // 4xx = permanent fail
                        }
                        // 5xx = retry
                    }
                    Err(e) => {
                        if attempt == 2 {
                            log::warn!("One-click unsubscribe failed after retries: {}", e);
                        }
                        // Network error = retry
                    }
                }
            }
            // Fall through to browser/mailto
        }
    }

    if let Some(url) = https_url {
        return Ok(UnsubscribeResult::Browser { url: url.to_string() });
    }

    if let Some(url) = mailto_url {
        let (email, subject, body) = parse_mailto(url);
        return Ok(UnsubscribeResult::Mailto { email, subject, body });
    }

    // Last resort: try body link fallback
    {
        let conn = state.db.safe_lock();
        if let Some(url) = find_unsubscribe_link_in_body(&conn, &account_id, &folder, uid) {
            return Ok(UnsubscribeResult::Browser { url });
        }
    }

    Err(AppError::NotFound("No usable unsubscribe URL found".to_string()))
}

/// Unsubscribe from a sender by finding a suitable message and processing it.
#[tauri::command]
pub async fn unsubscribe_sender(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
) -> Result<UnsubscribeResult, AppError> {
    // 1. Find a message with List-Unsubscribe header from this sender
    let target = {
        let conn = state.db.safe_lock();
        db::messages::find_unsubscribable_by_sender(&conn, &account_id, &sender_email)?
    };

    if let Some((folder, uid)) = target {
        return Box::pin(unsubscribe(
            state.clone(),
            account_id,
            folder,
            uid,
        ))
        .await;
    }

    // 2. Fallback: find any message with cached body for link scanning
    let target = {
        let conn = state.db.safe_lock();
        db::messages::find_any_by_sender_with_body(&conn, &account_id, &sender_email)?
    };

    if let Some((folder, uid)) = target {
        let conn = state.db.safe_lock();
        if let Some(url) = find_unsubscribe_link_in_body(&conn, &account_id, &folder, uid) {
            return Ok(UnsubscribeResult::Browser { url });
        }
    }

    Err(AppError::NotFound(format!(
        "No unsubscribe option found for {}",
        sender_email
    )))
}

#[tauri::command]
pub async fn list_unsubscribed_senders(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<String>, AppError> {
    let conn = state.open_read_conn()?;
    db::unsubscribed::list_emails(&conn, account_id.as_deref())
}

#[tauri::command]
pub async fn record_unsubscribed_sender(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
    method: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::unsubscribed::record(&conn, &account_id, &sender_email, &method)
}

#[tauri::command]
pub async fn remove_unsubscribed_sender(
    state: State<'_, AppState>,
    account_id: String,
    sender_email: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::unsubscribed::remove(&conn, &account_id, &sender_email)
}

#[derive(Serialize)]
pub struct ReclassifyResult {
    pub total: u32,
    pub changed: u32,
    pub junk: u32,
    pub skipped_user_override: u32,
}

/// Re-run the heuristic classifier and user mail_rules against every
/// existing message in inbox-type folders. Used to backfill category
/// changes (e.g. the new Junk detector) over the existing DB without
/// requiring a full IMAP re-sync.
///
/// - Respects `category_source = 'user'` — manual overrides are left alone.
/// - Applies `mark_read` / `mark_flagged` side-effects from rules.
/// - Chunks DB writes into batches under a single transaction per batch
///   so the UI isn't blocked for the full duration of a large backfill.
#[tauri::command]
pub async fn reclassify_inbox_messages(
    state: State<'_, AppState>,
) -> Result<ReclassifyResult, AppError> {
    reclassify_impl(&state, false).await
}

/// The body of the above, reusable so the startup backfill classifies through
/// exactly the same path the button does rather than growing a second
/// implementation (gotcha #36).
pub(crate) async fn reclassify_impl(
    state: &AppState,
    only_unclassified: bool,
) -> Result<ReclassifyResult, AppError> {
    // 1. Load everything we need in small lock scopes.
    let rows = {
        let conn = state.db.safe_lock();
        db::messages::list_for_reclassify(&conn, only_unclassified)?
    };

    let rules: Vec<db::rules::MailRule> = {
        let conn = state.db.safe_lock();
        db::rules::list(&conn)?
            .into_iter()
            .filter(|r| r.is_active)
            .collect()
    };

    // 2. Compute new categories in memory (no lock held).
    struct Update {
        account_id: String,
        folder_name: String,
        uid: u32,
        category: String,
        source: &'static str,
        mark_read: bool,
        mark_flagged: bool,
    }

    let total = rows.len() as u32;
    let mut skipped_user_override = 0u32;
    let mut updates: Vec<Update> = Vec::new();
    let mut junk = 0u32;

    for row in rows {
        if row.category_source.as_deref() == Some("user") {
            skipped_user_override += 1;
            continue;
        }

        let from_email = row.from_email.as_deref().unwrap_or("");
        let input = categorize::ClassificationInput {
            from_email,
            from_name: row.from_name.as_deref(),
            subject: row.subject.as_deref(),
            // Stored, and load-bearing: half the unclassified inbox carries it.
            // This used to pass `false` behind a comment saying it was not
            // stored, which quietly filed every bulk sender under Primary.
            has_list_unsubscribe: row.has_list_unsubscribe,
            // Genuinely not stored — there is no `precedence` column.
            precedence: None,
        };
        let (cat, _) = categorize::classify(&input);
        let mut category = cat.as_str().to_string();
        let mut source: &'static str = "heuristic";
        let mut mark_read = false;
        let mut mark_flagged = false;

        if !rules.is_empty() {
            // Rules are filtered per-account here since list() returned all.
            let applicable: Vec<&db::rules::MailRule> = rules
                .iter()
                .filter(|r| {
                    r.account_id
                        .as_deref()
                        .map(|a| a == row.account_id)
                        .unwrap_or(true)
                })
                .collect();
            if !applicable.is_empty() {
                let applicable_owned: Vec<db::rules::MailRule> =
                    applicable.into_iter().cloned().collect();
                let subject = row.subject.as_deref().unwrap_or("");
                // `body` stays empty: bodies aren't loaded for a backfill pass,
                // so a `body` condition cannot match here (the MCP rejects
                // them for this reason). `to_list` IS available and is passed.
                let actions = db::rules::evaluate(
                    &applicable_owned,
                    from_email,
                    &row.to_list,
                    subject,
                    "",
                    &row.account_id,
                );
                for action in &actions {
                    match action.action_type.as_str() {
                        "set_category" => {
                            if let Some(ref v) = action.value {
                                category = v.clone();
                                source = "rule";
                            }
                        }
                        "mark_read" => mark_read = true,
                        "mark_flagged" => mark_flagged = true,
                        _ => {}
                    }
                }
            }
        }

        if category == "junk" {
            junk += 1;
        }

        updates.push(Update {
            account_id: row.account_id,
            folder_name: row.folder_name,
            uid: row.uid,
            category,
            source,
            mark_read,
            mark_flagged,
        });
    }

    // 3. Apply updates in chunks so other readers can get a turn.
    const CHUNK: usize = 200;
    let mut changed = 0u32;
    for batch in updates.chunks(CHUNK) {
        let conn = state.db.safe_lock();
        let tx = conn.unchecked_transaction()?;
        for u in batch {
            // Only write if the category actually changed, to keep the
            // changed-count honest and avoid pointless writes.
            let current: Option<String> = tx
                .query_row(
                    "SELECT category FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                    rusqlite::params![u.account_id, u.folder_name, u.uid],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            // NULL is not "primary" — it is "never classified", and the two
            // must not compare equal here. `unwrap_or("primary")` made a row
            // the classifier judged primary look unchanged, so the backfill
            // wrote nothing and the column stayed NULL: 3,533 of 11,951 on the
            // first live run. Harmless on screen, since COALESCE renders NULL
            // as primary anyway, but the pass never terminates — it rescans
            // them at every launch — and `category_source` never records that
            // they were classified at all.
            let changed_here = match current.as_deref() {
                None => true,
                Some(existing) => existing != u.category.as_str(),
            };
            if changed_here {
                tx.execute(
                    "UPDATE messages SET category = ?1, category_source = ?2
                     WHERE account_id = ?3 AND folder_name = ?4 AND uid = ?5",
                    rusqlite::params![u.category, u.source, u.account_id, u.folder_name, u.uid],
                )?;
                changed += 1;
            }
            if u.mark_read {
                tx.execute(
                    "UPDATE messages SET is_read = 1 WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                    rusqlite::params![u.account_id, u.folder_name, u.uid],
                )?;
            }
            if u.mark_flagged {
                tx.execute(
                    "UPDATE messages SET is_flagged = 1 WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                    rusqlite::params![u.account_id, u.folder_name, u.uid],
                )?;
            }
        }
        tx.commit()?;
        // Drop lock between batches so the UI's read connections aren't
        // starved during a long backfill.
    }

    log::info!(
        "reclassify_inbox_messages: total={} changed={} junk={} skipped_user={}",
        total,
        changed,
        junk,
        skipped_user_override
    );

    Ok(ReclassifyResult {
        total,
        changed,
        junk,
        skipped_user_override,
    })
}

