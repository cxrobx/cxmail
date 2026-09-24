use crate::db;
use crate::email::{draft_local, imap, parser, smtp, tracking};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;

// Moved into `cxmail-email` so the MCP send path can reach it without
// depending on the app crate. Re-exported at the old name.
pub(crate) use crate::email::attachment_file::read_attachment_from_path;
use base64::Engine;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Emitter, Manager, State};

#[derive(Debug, Clone, Serialize)]
pub struct SendResult {
    pub send_id: Option<String>,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedDraftRef {
    pub folder: String,
    pub uid: u32,
    /// The draft's stable identity (`db::drafts`, v60) — names the draft
    /// itself, where `uid` names one revision of it. Set for every draft saved
    /// since v60.
    pub draft_id: Option<String>,
}

#[tauri::command]
pub async fn send_email(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    account_id: String,
    email: smtp::OutgoingEmail,
    delay_seconds: Option<u32>,
) -> Result<SendResult, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let mut email = email;
    // Validate the From before anything reaches SMTP. Unlike the draft path,
    // `smtp::send_email` has always honoured `from_email` verbatim — so
    // without this an arbitrary address from the frontend would go out on the
    // wire. Resolving also normalizes the spelling and picks up the alias's
    // own display name.
    let from = resolve_from(&state, &account, &email)?;
    email.from_email = from.email.clone();
    if let Some(name) = from.display_name.clone() {
        email.from_name = Some(name);
    }

    maybe_inject_tracking_pixel(&state.db, &account, &mut email).await;

    let delay = delay_seconds.unwrap_or(0);

    // Pre-generate a message_id so we can return it immediately (for follow-up reminders)
    let message_id = format!("<{}@cxmail.app>", uuid::Uuid::new_v4());

    if delay > 0 {
        // Undo send: delay before actually sending
        let send_id = uuid::Uuid::new_v4().to_string();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        {
            let mut pending = state.pending_sends.safe_lock();
            pending.insert(send_id.clone(), cancel_tx);
        }

        let send_id_clone = send_id.clone();
        let delayed_message_id = message_id.clone();
        let app_handle = app.clone();
        tokio::spawn(async move {
            let sleep = tokio::time::sleep(std::time::Duration::from_secs(delay as u64));

            tokio::select! {
                _ = sleep => {
                    // Timer expired — send the email
                    match smtp::send_email_with_message_id(&account, &email, Some(&delayed_message_id)).await {
                        Ok(_msg_id) => {
                            log::info!("Delayed send {} completed", send_id_clone);
                            if let Err(e) = app_handle.emit("send-completed", &send_id_clone) {
                                log::error!("Failed to emit send-completed event: {e}");
                            }
                        }
                        Err(e) => {
                            log::error!("Delayed send {} failed: {}", send_id_clone, e);
                            if let Err(emit_err) = app_handle.emit("send-failed", serde_json::json!({
                                "sendId": send_id_clone,
                                "error": e.to_string(),
                            })) {
                                log::error!("Failed to emit send-failed event: {emit_err}");
                            }
                        }
                    }
                }
                _ = cancel_rx => {
                    // Cancelled by user
                    log::info!("Send {} cancelled by user", send_id_clone);
                    if let Err(e) = app_handle.emit("send-cancelled", &send_id_clone) {
                        log::error!("Failed to emit send-cancelled event: {e}");
                    }
                }
            }

            // Clean up pending_sends
            if let Some(state) = app_handle.try_state::<AppState>() {
                let mut pending = state.pending_sends.safe_lock();
                pending.remove(&send_id_clone);
            }
        });

        Ok(SendResult {
            send_id: Some(send_id),
            message_id,
        })
    } else {
        // Immediate send (no delay)
        let msg_id = smtp::send_email(&account, &email).await?;
        Ok(SendResult {
            send_id: None,
            message_id: msg_id,
        })
    }
}

/// Inject an open-tracking pixel when the per-message flag AND the account's
/// track_opens_enabled flag are set AND a tracker service_url is configured.
/// Infallible by design: tracking failures are logged, never abort a send.
/// Shared by the interactive send path and the scheduled-send worker.
pub(crate) async fn maybe_inject_tracking_pixel(
    db: &std::sync::Mutex<rusqlite::Connection>,
    account: &db::accounts::Account,
    email: &mut smtp::OutgoingEmail,
) {
    if !email.track_opens.unwrap_or(false) || !account.track_opens_enabled {
        return;
    }

    let (config, tracking_api_key) = {
        let conn = db.safe_lock();
        match (db::tracking::get_config(&conn), db::tracking::get_api_key(&conn)) {
            (Ok(config), Ok(api_key)) => (config, api_key),
            (Err(e), _) | (_, Err(e)) => {
                log::error!("Failed to load tracking config, sending without pixel: {e}");
                return;
            }
        }
    };
    let Some(service_url) = config.service_url.as_deref() else {
        return;
    };

    let pixel_code = tracking::generate_pixel_code();
    email.html_body = tracking::inject_tracking_pixel(&email.html_body, service_url, &pixel_code);

    // Register pixel with remote tracker service
    if let Some(api_key) = &tracking_api_key {
        let client = reqwest::Client::new();
        let register_url = format!("{}/api/pixels", service_url.trim_end_matches('/'));
        if let Err(e) = client
            .post(&register_url)
            .bearer_auth(api_key)
            .json(&serde_json::json!({ "pixel_code": pixel_code }))
            .send()
            .await
        {
            log::error!("Failed to register tracking pixel with remote service: {e}");
        }
    }

    // Store pixel locally
    let to_email = email.to.first().map(|r| r.email.clone()).unwrap_or_default();
    let pixel = db::tracking::TrackingPixel {
        id: None,
        pixel_code,
        account_id: account.id.clone(),
        message_id: None,
        to_email,
        subject: Some(email.subject.clone()),
        open_count: 0,
        first_open_at: None,
        last_open_at: None,
        created_at: String::new(),
    };
    let conn = db.safe_lock();
    if let Err(e) = db::tracking::insert_pixel(&conn, &pixel) {
        log::error!("Failed to store tracking pixel locally: {e}");
    }
}

#[tauri::command]
pub async fn cancel_send(
    state: State<'_, AppState>,
    send_id: String,
) -> Result<(), AppError> {
    let cancel_tx = {
        let mut pending = state.pending_sends.safe_lock();
        pending.remove(&send_id)
    };
    if let Some(tx) = cancel_tx {
        let _ = tx.send(());
        Ok(())
    } else {
        Err(AppError::NotFound("No pending send with that ID".to_string()))
    }
}

/// Build the draft's raw MIME.
///
/// `from` is the RESOLVED send-as address (`db::identities::resolve_send_as`),
/// not the raw `email.from_email`: a draft's `From:` used to be hardcoded to
/// the account's own address, so a draft saved while an alias was selected
/// came back addressed from the primary — the send path honoured
/// `from_email` and the draft path did not, and they disagreed silently.
/// Resolving above this call is also what keeps a `From` the user never
/// configured from reaching IMAP.
fn build_draft_raw(
    from: &db::identities::SendAsAddress,
    email: &smtp::OutgoingEmail,
    message_id_bare: &str,
    draft_id: &str,
) -> Result<String, AppError> {
    let mut message = mail_builder::MessageBuilder::new();
    // mail-builder wraps the value in `<` `>` itself — pass the bare ID.
    message = message.message_id(message_id_bare);
    // The draft's stable identity rides in the MIME (v60) so the folder sync
    // can re-link a revision this process never saw — see `db::drafts`.
    message = message.header(
        draft_local::DRAFT_ID_HEADER,
        mail_builder::headers::raw::Raw::new(draft_id),
    );
    // The display name follows the address: an alias may carry its own, and
    // falling back to the caller's keeps the account's name when it does not.
    match from.display_name.as_deref().or(email.from_name.as_deref()) {
        Some(name) => message = message.from((name, from.email.as_str())),
        None => message = message.from(from.email.as_str()),
    }
    // One `.to()` / `.cc()` / `.bcc()` call per field — see `smtp::to_address_list`.
    if !email.to.is_empty() {
        message = message.to(smtp::to_address_list(&email.to));
    }
    if !email.cc.is_empty() {
        message = message.cc(smtp::to_address_list(&email.cc));
    }
    if !email.bcc.is_empty() {
        message = message.bcc(smtp::to_address_list(&email.bcc));
    }
    message = message.subject(&email.subject);
    if let Some(reply_to) = &email.in_reply_to {
        // Pass the bare ID — mail-builder re-wraps in `<` `>` (matches message_id above).
        message = message.in_reply_to(smtp::strip_message_id_brackets(reply_to));
    }
    if let Some(refs) = &email.references {
        message = message.header(
            "References",
            mail_builder::headers::raw::Raw::new(refs.as_str()),
        );
    }
    let styled_html = crate::email::inline_styles::apply_inline_font_styles(&email.html_body);
    message = message.html_body(&styled_html);

    for att in &email.attachments {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&att.data_base64)
            .map_err(|e| AppError::General(format!("Invalid base64 attachment: {}", e)))?;
        message = message.attachment(att.content_type.as_str(), att.filename.as_str(), decoded);
    }

    message
        .write_to_string()
        .map_err(|e| AppError::General(format!("Failed to build draft: {}", e)))
}

/// Which address this outgoing message may be sent / saved FROM.
///
/// `OutgoingEmail.from_email` arrives from the frontend, so it is validated
/// here rather than trusted (architecture invariant #6). Blank means "the
/// account's own address", which is what every caller sent before send-as
/// existed; anything the account is not configured to send as is refused with
/// a message naming the remedy. One matcher for the whole app
/// (`db::identities::resolve_for_account`) — the MCP draft handlers call the
/// same one.
pub(crate) fn resolve_from(
    state: &AppState,
    account: &db::accounts::Account,
    email: &smtp::OutgoingEmail,
) -> Result<db::identities::SendAsAddress, AppError> {
    let conn = state.db.safe_lock();
    db::identities::resolve_for_account(
        &conn,
        &account.id,
        &account.email,
        account.display_name.as_deref(),
        Some(email.from_email.as_str()),
    )
}

fn resolve_drafts_folder(state: &AppState, account_id: &str, provider: &str) -> String {
    let conn = state.db.safe_lock();
    db::folders::folder_for_account(&conn, account_id, provider, "drafts")
}

#[tauri::command]
pub async fn save_draft(
    state: State<'_, AppState>,
    account_id: String,
    email: smtp::OutgoingEmail,
) -> Result<SavedDraftRef, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    // Resolve the From before any IMAP work (#30's ordering): an address the
    // account is not configured to send as is a caller error, and a caller
    // error must not leave a draft on the server.
    let from = resolve_from(&state, &account, &email)?;

    let drafts_folder = resolve_drafts_folder(&state, &account_id, &account.provider);
    let message_id_bare = format!("{}@cxmail.app", uuid::Uuid::new_v4());
    let message_id_header = format!("<{}>", message_id_bare);
    // First save mints the draft's identity (v60).
    let draft_id = uuid::Uuid::new_v4().to_string();
    let raw = build_draft_raw(&from, &email, &message_id_bare, &draft_id)?;

    let mut session = imap::connect_for_account(&account).await?;
    // The drafts row records the UIDVALIDITY the UID was issued under — a UID
    // means nothing outside its mailbox generation.
    let (uidvalidity, _) = imap::select_folder(&mut session, &drafts_folder).await?;
    // \Seen so the post-save folder sync doesn't treat the draft as new unread mail
    // (would fire desktop notifications + inflate the tray badge per autosave).
    // \Draft is the RFC 3501 marker every IMAP client sets for drafts.
    session
        .append(&drafts_folder, Some("(\\Seen \\Draft)"), None, raw.as_bytes())
        .await
        .map_err(|e| AppError::Imap(format!("APPEND to Drafts failed: {}", e)))?;

    let uid = imap::find_uid_by_message_id(&mut session, &drafts_folder, &message_id_header).await?;
    let _ = imap::disconnect(session, &account.email).await;

    // Persist the freshly-APPENDed draft into the local cache so it shows up
    // in search immediately (the FTS triggers index the insert). IMAP is
    // still the source of truth — if this fails the next Drafts sync
    // reconciles state.
    persist_draft_to_local(&state, &account_id, &drafts_folder, uid, raw.as_bytes());
    {
        let conn = state.db.safe_lock();
        if let Err(e) =
            db::drafts::link(&conn, &draft_id, &account_id, &drafts_folder, uidvalidity, uid)
        {
            log::warn!("save_draft: drafts row for {} not written: {}", draft_id, e);
        }
    }

    log::info!(
        "Draft saved to {} (UID {}, draft {}) for {}",
        drafts_folder, uid, draft_id, account.email
    );
    Ok(SavedDraftRef {
        folder: drafts_folder,
        uid,
        draft_id: Some(draft_id),
    })
}

/// Shared between save_draft and edit_draft: parse the raw MIME we just
/// APPENDed and write it to messages/message_bodies (tightly scoped DB lock,
/// Gotcha #11). The v40 FTS triggers index the write — no search-side call.
fn persist_draft_to_local(
    state: &AppState,
    account_id: &str,
    folder: &str,
    uid: u32,
    raw: &[u8],
) {
    let conn = state.db.safe_lock();
    if let Err(e) = draft_local::persist_local_draft(&conn, account_id, folder, uid, raw) {
        log::warn!(
            "save/edit_draft: local DB write failed for UID {} in {}: {}",
            uid, folder, e
        );
    }
}

/// Replace an existing draft: APPEND the updated version, then delete the old UID.
/// APPEND runs first so a failed delete leaves a duplicate draft rather than losing content.
/// The delete is a targeted `UID EXPUNGE` — a mailbox-wide EXPUNGE would also purge
/// whatever another client had marked `\Deleted` in the folder (gotcha #57).
///
/// v60: `uid` is resolved to the logical draft (`db::drafts`) and the row is
/// CLAIMED — a compare-and-set on `current_uid` — before any IMAP work. If the
/// MCP (or another instance) has already replaced this revision, or holds the
/// claim, this returns a `Draft conflict:` error and writes nothing. The
/// composer keeps its dirty content on a failed save, adopts the live UID from
/// the bridge's `draft-updated` event, and its next autosave lands on that
/// revision. A draft with no row (saved before v60) proceeds unclaimed and
/// gets its row on this save.
#[tauri::command]
pub async fn edit_draft(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    email: smtp::OutgoingEmail,
) -> Result<SavedDraftRef, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    // Above the claim and every IMAP call (#30/#57's ordering): a From the
    // account cannot send as must not cost the draft its claim, let alone
    // reach the server.
    let from = resolve_from(&state, &account, &email)?;

    let drafts_folder = resolve_drafts_folder(&state, &account_id, &account.provider);
    // The folder the user saw is usually drafts_folder, but trust the caller's
    // value in case of non-standard mailbox layouts.
    let old_folder = if folder.is_empty() {
        drafts_folder.clone()
    } else {
        folder.clone()
    };

    let existing = {
        let conn = state.db.safe_lock();
        db::drafts::find_by_uid(&conn, &account_id, &old_folder, uid)?
    };
    let draft_id = existing
        .as_ref()
        .map(|r| r.draft_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let claim_token = uuid::Uuid::new_v4().to_string();
    let claimed = if existing.is_some() {
        let outcome = {
            let conn = state.db.safe_lock();
            db::drafts::claim(&conn, &draft_id, uid, &claim_token)?
        };
        match outcome {
            db::drafts::ClaimOutcome::Claimed => true,
            db::drafts::ClaimOutcome::Moved { current_uid } => {
                return Err(AppError::General(format!(
                    "Draft conflict: UID {uid} is no longer the current revision of draft {draft_id} (now UID {current_uid}); nothing was saved"
                )));
            }
            db::drafts::ClaimOutcome::Busy { expires_at } => {
                return Err(AppError::General(format!(
                    "Draft conflict: draft {draft_id} is being saved by another writer (claim held until {expires_at}); nothing was saved"
                )));
            }
            // The row vanished between lookup and claim — proceed as a
            // pre-v60 draft would and re-create it on success.
            db::drafts::ClaimOutcome::Missing => false,
        }
    } else {
        false
    };

    let result = replace_draft_revision(
        &state,
        &account,
        &account_id,
        &old_folder,
        uid,
        &email,
        &from,
        &drafts_folder,
        &draft_id,
        &claim_token,
        claimed,
    )
    .await;
    if result.is_err() && claimed {
        let conn = state.db.safe_lock();
        if let Err(e) = db::drafts::release(&conn, &draft_id, &claim_token) {
            log::warn!("edit_draft: releasing claim on {} failed: {}", draft_id, e);
        }
    }
    result
}

/// The IMAP half of `edit_draft`, split out so the caller can release the
/// claim on any failure. Returns only after the new revision is on the server
/// and the drafts row points at it.
#[allow(clippy::too_many_arguments)]
async fn replace_draft_revision(
    state: &AppState,
    account: &db::accounts::Account,
    account_id: &str,
    old_folder: &str,
    uid: u32,
    email: &smtp::OutgoingEmail,
    from: &db::identities::SendAsAddress,
    drafts_folder: &str,
    draft_id: &str,
    claim_token: &str,
    claimed: bool,
) -> Result<SavedDraftRef, AppError> {
    let message_id_bare = format!("{}@cxmail.app", uuid::Uuid::new_v4());
    let message_id_header = format!("<{}>", message_id_bare);
    let raw = build_draft_raw(from, email, &message_id_bare, draft_id)?;

    let mut session = imap::connect_for_account(account).await?;
    let (uidvalidity, _) = imap::select_folder(&mut session, drafts_folder).await?;

    // Same flag rationale as save_draft: \Seen suppresses new-mail notification +
    // unread badge from the post-save sync; \Draft is the RFC 3501 marker.
    session
        .append(drafts_folder, Some("(\\Seen \\Draft)"), None, raw.as_bytes())
        .await
        .map_err(|e| AppError::Imap(format!("APPEND to Drafts failed: {}", e)))?;

    let new_uid =
        imap::find_uid_by_message_id(&mut session, drafts_folder, &message_id_header).await?;

    // The new revision is on the server: move the pointer NOW, before the
    // old-UID delete. From here on the old revision is the duplicate.
    {
        let conn = state.db.safe_lock();
        let moved = if claimed {
            db::drafts::advance(&conn, draft_id, claim_token, uidvalidity, new_uid)
        } else {
            db::drafts::link(&conn, draft_id, account_id, drafts_folder, uidvalidity, new_uid)
        };
        if let Err(e) = moved {
            log::warn!(
                "edit_draft: drafts row for {} not advanced to UID {}: {}",
                draft_id, new_uid, e
            );
        }
    }

    // Now delete the old draft.
    match imap::select_folder(&mut session, old_folder).await {
        Ok(_) => {
            if let Err(e) = imap::store_flags(&mut session, uid, true, "\\Deleted").await {
                log::warn!(
                    "edit_draft: failed to mark old draft UID {} deleted: {}",
                    uid, e
                );
            } else if let Err(e) = imap::uid_expunge(&mut session, uid).await {
                log::warn!(
                    "edit_draft: UID EXPUNGE {} failed after marking old draft deleted: {}",
                    uid, e
                );
            }
        }
        Err(e) => {
            log::warn!(
                "edit_draft: SELECT {} failed while removing old draft UID {}: {}",
                old_folder, uid, e
            );
        }
    }

    let _ = imap::disconnect(session, &account.email).await;

    // Evict the old UID from the local cache (the FTS delete trigger evicts
    // the search row in the same transaction), then persist the new one.
    {
        let conn = state.db.safe_lock();
        if let Err(e) = db::messages::delete_uids(&conn, account_id, old_folder, &[uid]) {
            log::warn!(
                "edit_draft: local DB delete of old UID {} in {} failed: {}",
                uid, old_folder, e
            );
        }
    }

    persist_draft_to_local(state, account_id, drafts_folder, new_uid, raw.as_bytes());

    log::info!(
        "Draft UID {} replaced with UID {} in {} (draft {}) for {}",
        uid, new_uid, drafts_folder, draft_id, account.email
    );
    Ok(SavedDraftRef {
        folder: drafts_folder.to_string(),
        uid: new_uid,
        draft_id: Some(draft_id.to_string()),
    })
}

/// Read a file from disk and return it as base64 with metadata for compose attachments.
#[tauri::command]
pub async fn read_file_as_base64(
    path: String,
) -> Result<(String, String, String), AppError> {
    let (filename, content_type, data) = read_attachment_from_path(&path)?;
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &data,
    );
    Ok((filename, content_type, b64))
}


/// Download an attachment from a message. Fetches the raw message, extracts the attachment,
/// and saves it to the specified path.
#[tauri::command]
pub async fn download_attachment(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    attachment_index: usize,
    save_path: String,
) -> Result<(), AppError> {
    log::info!(
        "download_attachment: enter account={} folder={} uid={} index={} save_path={:?}",
        account_id, folder, uid, attachment_index, save_path
    );

    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    log::info!("download_attachment: IMAP connected for uid={}", uid);
    let _ = imap::select_folder(&mut session, &folder).await?;
    log::info!("download_attachment: folder selected for uid={}", uid);
    let raw_body = imap::fetch_body(&mut session, uid).await?;
    log::info!(
        "download_attachment: fetched body uid={} bytes={}",
        uid,
        raw_body.len()
    );
    let _ = imap::disconnect(session, &account.email).await;

    let (filename, data) = parser::extract_attachment(&raw_body, attachment_index)
        .ok_or_else(|| AppError::NotFound("Attachment not found".to_string()))?;
    log::info!(
        "download_attachment: extracted filename={:?} bytes={}",
        filename,
        data.len()
    );

    let path = if save_path.is_empty() {
        // Default to Downloads
        dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(&filename)
    } else {
        PathBuf::from(&save_path)
    };

    std::fs::write(&path, &data).map_err(|e| {
        log::error!(
            "download_attachment: fs::write failed path={:?} err={}",
            path, e
        );
        AppError::Io(e)
    })?;

    log::info!("Attachment saved to {:?}", path);
    Ok(())
}

/// Fetch a message's real (non-inline) attachments as `OutgoingAttachment`s
/// (filename, content_type, base64 bytes) so they can be reloaded into the
/// compose window when editing an existing draft. Without this, opening a draft
/// shows no attachments AND re-saving rebuilds the draft without them — silently
/// dropping the files. Inline cid: body images are excluded (already embedded as
/// data: URIs in the HTML body). Mirrors `download_attachment`'s IMAP fetch path.
#[tauri::command]
pub async fn fetch_outgoing_attachments(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<Vec<smtp::OutgoingAttachment>, AppError> {
    let to_outgoing = |b: db::draft_blobs::DraftBlob| smtp::OutgoingAttachment {
        filename: b.filename,
        content_type: b.content_type,
        data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &b.data),
    };

    // Local first. A draft this software saved has its parts cached
    // (`persist_local_draft`), so the common reopen never touches IMAP —
    // which on 2026-08-25 was a fresh connection, an 8.8 MB FETCH and a 5 s
    // LOGOUT timeout, per open, on an account that could barely connect.
    let cached = {
        let conn = state.db.safe_lock();
        db::draft_blobs::get(&conn, &account_id, &folder, uid)?
    };
    if !cached.is_empty() {
        log::info!(
            "fetch_outgoing_attachments: account={} folder={} uid={} -> {} attachment(s) (cached)",
            account_id,
            folder,
            uid,
            cached.len()
        );
        return Ok(cached.into_iter().map(to_outgoing).collect());
    }

    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    let _ = imap::select_folder(&mut session, &folder).await?;
    let raw_body = imap::fetch_body(&mut session, uid).await?;
    // The bytes are in hand; the LOGOUT is hygiene, not part of the answer.
    // Awaiting it here put its 5 s timeout on the critical path of every open.
    let email = account.email.clone();
    tokio::spawn(async move {
        let _ = imap::disconnect(session, &email).await;
    });

    let blobs = draft_local::draft_blobs_from_raw(&raw_body);
    // Back-fill the cache so the NEXT open of this draft is local — this is
    // the path drafts saved before v59 (or by another client) take once.
    {
        let conn = state.db.safe_lock();
        if let Err(e) = db::draft_blobs::replace(&conn, &account_id, &folder, uid, &blobs) {
            log::warn!(
                "fetch_outgoing_attachments: could not cache attachment bytes for uid {uid}: {e}"
            );
        }
    }
    log::info!(
        "fetch_outgoing_attachments: account={} folder={} uid={} -> {} attachment(s) (fetched from IMAP)",
        account_id,
        folder,
        uid,
        blobs.len()
    );
    Ok(blobs.into_iter().map(to_outgoing).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The draft path used to hardcode the account's own address, so an alias
    /// draft came back From the primary. The resolved send-as is what must
    /// reach the header, whatever the frontend's `from_email` said.
    #[test]
    fn a_draft_is_built_from_the_resolved_send_as_address() {
        let email: smtp::OutgoingEmail = serde_json::from_value(serde_json::json!({
            "from_email": "sidalias@icloud.com",
            "from_name": "Chris",
            "to": [{ "name": null, "email": "friend@example.com" }],
            "cc": [],
            "bcc": [],
            "subject": "Re: Hello",
            "html_body": "<p>hi</p>",
            "text_body": "hi",
            "attachments": []
        }))
        .expect("OutgoingEmail fixture");
        let alias = db::identities::SendAsAddress {
            account_id: "acct".into(),
            email: "rileyprime@icloud.com".into(),
            display_name: None,
            signature_html: None,
            is_primary: false,
            identity_id: Some(2),
            from_sent: false,
        };
        let raw = build_draft_raw(&alias, &email, "x@cxmail.app", "draft-1").unwrap();
        let text = raw.as_str();
        let from_line = text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("from:"))
            .expect("a From header");
        assert!(from_line.contains("rileyprime@icloud.com"), "{from_line}");
        assert!(!from_line.contains("sidalias@"), "{from_line}");
        assert!(from_line.contains("Chris"), "the caller's name survives: {from_line}");
    }
}
