use crate::db;
use crate::email;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Serialize;
use tauri::State;

fn get_client() -> Result<email::inference::InferenceClient, AppError> {
    email::inference::InferenceClient::load()
}

#[derive(Serialize)]
pub struct SmartReplies {
    pub replies: Vec<String>,
}

#[tauri::command]
pub async fn ai_generate_reply(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    context: String,
) -> Result<String, AppError> {
    let (plain_text, sender_name, voice_ctx) = {
        let conn = state.db.safe_lock();
        let body = db::messages::get_body(&conn, &account_id, &folder, uid)?
            .and_then(|b| b.plain_text)
            .ok_or_else(|| AppError::NotFound("Message body not found".to_string()))?;

        let (sender, sender_email): (String, String) = conn
            .query_row(
                "SELECT COALESCE(from_name, from_email, 'sender'), COALESCE(from_email, '') FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                rusqlite::params![account_id, folder, uid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap_or_else(|_| ("sender".to_string(), String::new()));

        let voice_ctx = email::ai::build_voice_ctx_for_recipient(&conn, &account_id, &sender_email);

        (body, sender, voice_ctx)
    };

    let client = get_client()?;
    email::ai::generate_reply(
        &client,
        &plain_text,
        &sender_name,
        &context,
        voice_ctx.as_deref(),
    )
    .await
}


#[tauri::command]
pub async fn ai_rewrite_text(text: String, instruction: String) -> Result<String, AppError> {
    let client = get_client()?;
    email::ai::rewrite_text(&client, &text, &instruction).await
}

#[tauri::command]
pub async fn ai_adjust_tone(text: String, tone: String) -> Result<String, AppError> {
    let client = get_client()?;
    email::ai::adjust_tone(&client, &text, &tone).await
}

#[tauri::command]
pub async fn ai_proofread(text: String) -> Result<String, AppError> {
    let client = get_client()?;
    email::ai::proofread(&client, &text).await
}

/// The contextual half of Validate — the checks that can't be decided by
/// pattern matching.
///
/// The mechanical half (recipients, placeholders, dead links, missing
/// attachment) runs in the frontend against the live editor and never reaches
/// this command; see `src/lib/draftValidation.ts`.
///
/// Everything the model needs is gathered under ONE short lock scope and the
/// connection is dropped before the network call. A DB mutex held across
/// inference would stall `fetchBody` for the length of the request — gotcha
/// #11's exact shape, and this request is measured in seconds.
#[tauri::command]
pub async fn ai_validate_draft(
    state: State<'_, AppState>,
    account_id: String,
    recipient_email: Option<String>,
    subject: String,
    body_text: String,
    reply_folder: Option<String>,
    reply_uid: Option<u32>,
) -> Result<Vec<email::ai::DraftFinding>, AppError> {
    let recipient = recipient_email
        .map(|r| r.trim().to_lowercase())
        .filter(|r| !r.is_empty());

    let (pinned, original) = {
        let conn = state.db.safe_lock();

        // Account-scoped rules apply to every message; recipient rules stack
        // on top. `list_effective` already returns them in account-then-
        // recipient order, so the specific rule reads last.
        let pinned: Vec<String> = match recipient.as_deref() {
            Some(r) => db::voice_pinned_rules::list_effective(&conn, &account_id, r),
            None => db::voice_pinned_rules::list_account_scoped(&conn, &account_id),
        }
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.rule)
        .collect();

        // Best-effort. A reply whose parent body was never cached still gets
        // reviewed — it just can't be checked for unanswered questions, which
        // is strictly better than refusing to review at all.
        let original = match (reply_folder.as_deref(), reply_uid) {
            (Some(folder), Some(uid)) => db::messages::get_body(&conn, &account_id, folder, uid)
                .ok()
                .flatten()
                .and_then(|b| b.plain_text),
            _ => None,
        };

        (pinned, original)
    };

    let client = get_client()?;
    email::ai::validate_draft(
        &client,
        recipient.as_deref(),
        &subject,
        &body_text,
        original.as_deref(),
        &pinned,
    )
    .await
}

#[tauri::command]
pub async fn ai_smart_replies(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<SmartReplies, AppError> {
    let (plain_text, sender_name) = {
        let conn = state.db.safe_lock();
        let body = db::messages::get_body(&conn, &account_id, &folder, uid)?
            .and_then(|b| b.plain_text)
            .ok_or_else(|| AppError::NotFound("Message body not found".to_string()))?;

        let sender: String = conn
            .query_row(
                "SELECT COALESCE(from_name, from_email, 'sender') FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                rusqlite::params![account_id, folder, uid],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "sender".to_string());

        (body, sender)
    };

    let client = get_client()?;
    let replies = email::ai::suggest_replies(&client, &plain_text, &sender_name).await?;
    Ok(SmartReplies { replies })
}

#[tauri::command]
pub async fn ai_suggest_subject(text: String) -> Result<String, AppError> {
    let client = get_client()?;
    email::ai::suggest_subject(&client, &text).await
}

#[derive(Serialize)]
pub struct VoiceProfileStatus {
    pub exists: bool,
    pub sample_count: i64,
    pub model_used: Option<String>,
    pub generated_at: Option<String>,
}

#[tauri::command]
pub async fn ai_get_voice_profile_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<VoiceProfileStatus, AppError> {
    let conn = state.db.safe_lock();
    let row = db::voice_profiles::get_by_account(&conn, &account_id)?;
    Ok(match row {
        Some(r) => VoiceProfileStatus {
            exists: true,
            sample_count: r.sample_count,
            model_used: r.model_used,
            generated_at: Some(r.generated_at),
        },
        None => VoiceProfileStatus {
            exists: false,
            sample_count: 0,
            model_used: None,
            generated_at: None,
        },
    })
}

#[derive(Serialize)]
pub struct InsightStatus {
    pub pending_edits: i64,
    pub active_insights: i64,
    pub total_insights: i64,
}

#[derive(Serialize)]
pub struct LearnResult {
    pub extracted: i64,
    pub activated: i64,
}

#[tauri::command]
pub async fn ai_get_insight_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<InsightStatus, AppError> {
    let conn = state.db.safe_lock();
    let pending_edits = db::voice_edits::count_pending_edits(&conn, &account_id)?;
    let all = db::voice_edits::list_all_insights(&conn, &account_id)?;
    let active = all.iter().filter(|i| i.is_active).count() as i64;
    Ok(InsightStatus {
        pending_edits,
        active_insights: active,
        total_insights: all.len() as i64,
    })
}

#[tauri::command]
pub async fn ai_learn_from_edits(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<LearnResult, AppError> {
    let edits = {
        let conn = state.db.safe_lock();
        db::voice_edits::list_recent_edits(&conn, &account_id, 50)?
    };
    if edits.len() < 3 {
        return Err(AppError::AiService(format!(
            "Need at least 3 edited drafts to learn from. Found {}.",
            edits.len()
        )));
    }

    let client = get_client()?;
    let insights = email::voice::extract_learning_insights(&client, &edits).await?;

    let (extracted, activated) = {
        let conn = state.db.safe_lock();
        let mut extracted = 0i64;
        let mut activated = 0i64;
        for ins in &insights {
            let dedupe_key: String = ins
                .prompt_addition
                .chars()
                .take(80)
                .collect::<String>()
                .to_lowercase();
            let will_activate = ins.source_edit_count >= 5 && ins.confidence >= 0.8;
            db::voice_edits::upsert_insight(
                &conn,
                &account_id,
                &ins.insight,
                &ins.prompt_addition,
                ins.source_edit_count,
                ins.confidence,
                &dedupe_key,
            )?;
            extracted += 1;
            if will_activate {
                activated += 1;
            }
        }
        (extracted, activated)
    };
    Ok(LearnResult {
        extracted,
        activated,
    })
}

#[tauri::command]
pub async fn ai_log_reply_edit(
    state: State<'_, AppState>,
    account_id: String,
    ai_draft: String,
    sent_body: String,
    recipient_email: Option<String>,
) -> Result<(), AppError> {
    let ai_draft = ai_draft.trim();
    let sent_body = sent_body.trim();
    if ai_draft.is_empty() || sent_body.is_empty() {
        return Ok(());
    }
    let similarity = db::voice_edits::jaccard_similarity(ai_draft, sent_body);
    let recipient_norm = recipient_email
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let conn = state.db.safe_lock();
    db::voice_edits::insert_draft_edit(
        &conn,
        &account_id,
        ai_draft,
        sent_body,
        similarity,
        recipient_norm.as_deref(),
    )?;
    Ok(())
}

#[derive(Serialize)]
pub struct RecipientProfileStatus {
    pub status: String, // "fresh" | "stale" | "missing" | "insufficient_samples"
    pub profile: Option<String>,
    pub sample_count: i64,
    pub generated_at: Option<String>,
}

impl RecipientProfileStatus {
    fn missing() -> Self {
        Self {
            status: "missing".into(),
            profile: None,
            sample_count: 0,
            generated_at: None,
        }
    }
    fn insufficient(count: usize) -> Self {
        Self {
            status: "insufficient_samples".into(),
            profile: None,
            sample_count: count as i64,
            generated_at: None,
        }
    }
    fn from_cached(row: db::voice_profiles_recipient::RecipientProfileRow, status: &str) -> Self {
        Self {
            status: status.to_string(),
            profile: Some(row.profile_json),
            sample_count: row.sample_count,
            generated_at: Some(row.generated_at),
        }
    }
}

const RECIPIENT_EXTRACT_TIMEOUT_SECS: u64 = 30;

/// Return the cached recipient voice profile, or lazily extract one if missing/stale.
/// Soft-fails: if extraction errors or times out, returns a cached/missing status
/// rather than propagating — the UI falls back to the account-level profile.
#[tauri::command]
pub async fn ai_get_recipient_profile(
    state: State<'_, AppState>,
    account_id: String,
    recipient_email: String,
) -> Result<RecipientProfileStatus, AppError> {
    let recipient = recipient_email.trim().to_lowercase();
    if recipient.is_empty() {
        return Ok(RecipientProfileStatus::missing());
    }

    // 1. Cache + staleness check (short DB scope)
    let (cached, needs_refresh) = {
        let conn = state.db.safe_lock();
        let cached = db::voice_profiles_recipient::get(&conn, &account_id, &recipient)?;
        let needs_refresh = email::voice::should_refresh_recipient(&conn, &account_id, &recipient)?;
        (cached, needs_refresh)
    };

    if !needs_refresh {
        if let Some(row) = cached {
            return Ok(RecipientProfileStatus::from_cached(row, "fresh"));
        }
    }

    // 2. Collect samples (short DB scope)
    let learning_set = {
        let conn = state.db.safe_lock();
        email::voice::build_recipient_learning_set(&conn, &account_id, &recipient)?
    };

    if learning_set.samples.len() < email::voice::MIN_SAMPLES_REQUIRED {
        return Ok(match cached {
            Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
            None => RecipientProfileStatus::insufficient(learning_set.samples.len()),
        });
    }

    // 3. Network call — no DB lock held, with timeout per plan
    let client = get_client()?;
    let previous_profile = cached.as_ref().map(|row| row.profile_json.as_str());
    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(RECIPIENT_EXTRACT_TIMEOUT_SECS),
        email::voice::extract_recipient_profile_from_learning_set(
            &client,
            &learning_set,
            previous_profile,
        ),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(error)) => {
            log::warn!(
                "recipient voice extraction failed for {}: {}",
                recipient,
                error
            );
            return Ok(match cached {
                Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
                None => RecipientProfileStatus::missing(),
            });
        }
        Err(_) => {
            log::warn!(
                "recipient voice extraction timed out for {} after {}s",
                recipient,
                RECIPIENT_EXTRACT_TIMEOUT_SECS
            );
            // Timeout or extraction error — surface cached if any, else missing
            return Ok(match cached {
                Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
                None => RecipientProfileStatus::missing(),
            });
        }
    };

    // 4. Persist + return fresh status
    {
        let conn = state.db.safe_lock();
        db::voice_profiles_recipient::upsert(
            &conn,
            &account_id,
            &recipient,
            &outcome.profile_json,
            &outcome.model_used,
            outcome.sample_count,
            &outcome.last_extracted_message_date,
        )?;
    }
    let row = {
        let conn = state.db.safe_lock();
        db::voice_profiles_recipient::get(&conn, &account_id, &recipient)?
    };
    Ok(match row {
        Some(r) => RecipientProfileStatus::from_cached(r, "fresh"),
        None => RecipientProfileStatus::missing(),
    })
}

/// Force-refresh a recipient profile: re-extracts and overwrites the cached
/// row. Unlike `ai_get_recipient_profile`, this skips the staleness check —
/// but it does NOT pre-delete the cache. If the new extraction fails (timeout
/// or insufficient samples now), the previous cached row stays put so the
/// user doesn't lose what they had.
#[tauri::command]
pub async fn ai_refresh_recipient_profile(
    state: State<'_, AppState>,
    account_id: String,
    recipient_email: String,
) -> Result<RecipientProfileStatus, AppError> {
    let recipient = recipient_email.trim().to_lowercase();
    if recipient.is_empty() {
        return Ok(RecipientProfileStatus::missing());
    }

    let (cached, learning_set) = {
        let conn = state.db.safe_lock();
        let cached = db::voice_profiles_recipient::get(&conn, &account_id, &recipient)?;
        let learning_set =
            email::voice::build_recipient_learning_set(&conn, &account_id, &recipient)?;
        (cached, learning_set)
    };

    if learning_set.samples.len() < email::voice::MIN_SAMPLES_REQUIRED {
        // Don't blow away an existing profile just because samples shrank.
        return Ok(match cached {
            Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
            None => RecipientProfileStatus::insufficient(learning_set.samples.len()),
        });
    }

    let client = get_client()?;
    let previous_profile = cached.as_ref().map(|row| row.profile_json.as_str());
    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(RECIPIENT_EXTRACT_TIMEOUT_SECS),
        email::voice::extract_recipient_profile_from_learning_set(
            &client,
            &learning_set,
            previous_profile,
        ),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(error)) => {
            log::warn!(
                "forced recipient voice extraction failed for {}: {}",
                recipient,
                error
            );
            return Ok(match cached {
                Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
                None => RecipientProfileStatus::missing(),
            });
        }
        Err(_) => {
            log::warn!(
                "forced recipient voice extraction timed out for {} after {}s",
                recipient,
                RECIPIENT_EXTRACT_TIMEOUT_SECS
            );
            return Ok(match cached {
                Some(row) => RecipientProfileStatus::from_cached(row, "stale"),
                None => RecipientProfileStatus::missing(),
            });
        }
    };

    {
        let conn = state.db.safe_lock();
        db::voice_profiles_recipient::upsert(
            &conn,
            &account_id,
            &recipient,
            &outcome.profile_json,
            &outcome.model_used,
            outcome.sample_count,
            &outcome.last_extracted_message_date,
        )?;
    }
    let row = {
        let conn = state.db.safe_lock();
        db::voice_profiles_recipient::get(&conn, &account_id, &recipient)?
    };
    Ok(match row {
        Some(r) => RecipientProfileStatus::from_cached(r, "fresh"),
        None => RecipientProfileStatus::missing(),
    })
}

/// Draft a fresh email to a recipient (no inbound message). Conditioned on the
/// recipient's voice profile if one exists, falling back through archetype →
/// account → generic.
#[tauri::command]
pub async fn ai_generate_compose(
    state: State<'_, AppState>,
    account_id: String,
    recipient_email: String,
    subject: Option<String>,
    instruction: Option<String>,
) -> Result<String, AppError> {
    let recipient = recipient_email.trim().to_lowercase();

    let voice_ctx = {
        let conn = state.db.safe_lock();
        email::ai::build_voice_ctx_for_recipient(&conn, &account_id, &recipient)
    };

    let client = get_client()?;
    email::ai::generate_compose(
        &client,
        &recipient,
        subject.as_deref(),
        instruction.as_deref(),
        voice_ctx.as_deref(),
    )
    .await
}

#[derive(Serialize)]
pub struct ArchetypeView {
    pub archetype_id: String,
    pub name: String,
    pub description: String,
    pub profile_json: String,
    pub sample_count: i64,
    pub generated_at: String,
}

impl From<db::voice_archetypes::ArchetypeRow> for ArchetypeView {
    fn from(row: db::voice_archetypes::ArchetypeRow) -> Self {
        Self {
            archetype_id: row.archetype_id,
            name: row.name,
            description: row.description,
            profile_json: row.profile_json,
            sample_count: row.sample_count,
            generated_at: row.generated_at,
        }
    }
}

#[tauri::command]
pub async fn ai_list_archetypes(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ArchetypeView>, AppError> {
    let conn = state.db.safe_lock();
    let rows = db::voice_archetypes::list_for_account(&conn, &account_id)?;
    Ok(rows.into_iter().map(ArchetypeView::from).collect())
}

/// Idempotent "ensure archetypes exist" operation. If archetypes already exist
/// for this account, returns them without re-extracting (the LLM call is
/// expensive). Callers that want a fresh build must use
/// `ai_recluster_archetypes`. The Sidebar UI flips between the two based on
/// whether `list_for_account` returned anything.
#[tauri::command]
pub async fn ai_extract_archetypes(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ArchetypeView>, AppError> {
    {
        let conn = state.db.safe_lock();
        let existing = db::voice_archetypes::list_for_account(&conn, &account_id)?;
        if !existing.is_empty() {
            return Ok(existing.into_iter().map(ArchetypeView::from).collect());
        }
    }
    cluster_and_persist_archetypes(state, account_id, false).await
}

/// Wipe all archetypes for the account, then re-run clustering.
#[tauri::command]
pub async fn ai_recluster_archetypes(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ArchetypeView>, AppError> {
    cluster_and_persist_archetypes(state, account_id, true).await
}

async fn cluster_and_persist_archetypes(
    state: State<'_, AppState>,
    account_id: String,
    replace_existing: bool,
) -> Result<Vec<ArchetypeView>, AppError> {
    // 1. Collect samples (short DB scope)
    let samples = {
        let conn = state.db.safe_lock();
        email::voice::collect_sent_samples_for_clustering(
            &conn,
            &account_id,
            email::voice::ARCHETYPE_MAX_SAMPLES,
        )?
    };
    if samples.len() < email::voice::MIN_SAMPLES_REQUIRED {
        return Err(AppError::AiService(format!(
            "Need at least {} sent messages to cluster archetypes. Found {}.",
            email::voice::MIN_SAMPLES_REQUIRED,
            samples.len()
        )));
    }

    // 2. LLM call (no DB lock)
    let client = get_client()?;
    let clustered = email::voice::cluster_archetypes(&client, &samples).await?;

    // 3. Persist atomically. For re-cluster, only delete existing archetypes
    // after the model has succeeded, and commit the delete+insert as one unit.
    let model_used = client.model();
    {
        let conn = state.db.safe_lock();
        let tx = conn.unchecked_transaction()?;
        if replace_existing {
            db::voice_archetypes::delete_all(&tx, &account_id)?;
        }
        for arch in &clustered {
            db::voice_archetypes::insert(
                &tx,
                &account_id,
                &arch.archetype_id,
                &arch.name,
                &arch.description,
                &arch.profile_json,
                model_used,
                arch.assignments.len() as i64,
            )?;
            for (folder_name, uid) in &arch.assignments {
                db::voice_archetypes::assign_message(
                    &tx,
                    &account_id,
                    folder_name,
                    *uid,
                    &arch.archetype_id,
                )?;
            }
        }
        tx.commit()?;
    }

    let conn = state.db.safe_lock();
    let rows = db::voice_archetypes::list_for_account(&conn, &account_id)?;
    Ok(rows.into_iter().map(ArchetypeView::from).collect())
}

#[tauri::command]
pub async fn ai_rename_archetype(
    state: State<'_, AppState>,
    account_id: String,
    archetype_id: String,
    new_name: String,
) -> Result<(), AppError> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(AppError::AiService("Archetype name cannot be empty".into()));
    }
    let conn = state.db.safe_lock();
    db::voice_archetypes::rename(&conn, &account_id, &archetype_id, trimmed)?;
    Ok(())
}

#[tauri::command]
pub async fn ai_delete_archetype(
    state: State<'_, AppState>,
    account_id: String,
    archetype_id: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::voice_archetypes::delete(&conn, &account_id, &archetype_id)?;
    Ok(())
}

#[tauri::command]
pub async fn ai_extract_voice_profile(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<VoiceProfileStatus, AppError> {
    // 1. Collect samples (short DB scope)
    let samples = {
        let conn = state.db.safe_lock();
        email::voice::collect_sent_samples(&conn, &account_id)?
    };
    if samples.len() < email::voice::MIN_SAMPLES_REQUIRED {
        return Err(AppError::AiService(format!(
            "Need at least {} clean sent messages to build a voice profile. Found {}.",
            email::voice::MIN_SAMPLES_REQUIRED,
            samples.len()
        )));
    }

    // 2. Run extraction (no DB lock held during network call)
    let client = get_client()?;
    let result = email::voice::extract_profile(&client, &samples).await?;

    // 3. Persist + return fresh status
    let conn = state.db.safe_lock();
    db::voice_profiles::upsert(
        &conn,
        &account_id,
        &result.voice_profile_json,
        &result.voice_examples_json,
        &result.model_used,
        samples.len() as i64,
    )?;
    let row = db::voice_profiles::get_by_account(&conn, &account_id)?
        .ok_or_else(|| AppError::AiService("voice profile upsert not readable".into()))?;
    Ok(VoiceProfileStatus {
        exists: true,
        sample_count: row.sample_count,
        model_used: row.model_used,
        generated_at: Some(row.generated_at),
    })
}
