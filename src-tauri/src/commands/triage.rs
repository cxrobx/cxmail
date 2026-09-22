//! The background AI triage pass.
//!
//! Lives in the app package because it needs `AppState`'s DB handle and the
//! same small-lock-scope discipline the reclassify path uses (gotcha #11): a
//! held mutex across an HTTP await stalls `fetchBody`, which the user sees as
//! "Loading message…" that never resolves.
//!
//! Shape: read a batch under the lock, DROP the lock, await the provider, take
//! the lock again to write. Never one scope around the whole thing.

use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use crate::db;
use cxmail_email::email::inference::InferenceClient;
use cxmail_email::email::triage::{self, PROMPT_VERSION};
use cxmail_email::email::triage_gate::{self, Gate, MessageForTriage, TriageMode};
use serde::Serialize;
use tauri::State;

/// Messages classified per pass. Small on purpose: the tick is every 30s, so
/// this is ~10/minute, and a backlog of a few hundred clears in half an hour
/// without ever looking like a burst to the provider.
const BATCH_PER_PASS: usize = 5;

/// Hard ceiling on verdicts per rolling day. A runaway loop is the only way
/// this becomes expensive, so the bound is in code rather than in a setting.
const DAILY_CAP: i64 = 400;

#[derive(Debug, Default, Clone, Serialize)]
pub struct TriagePassStats {
    pub considered: u32,
    pub classified: u32,
    pub withheld: u32,
    pub failed: u32,
    /// Set when the daily cap stopped the pass, so the UI can say so rather
    /// than showing a silent zero.
    pub capped: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriageStatus {
    pub mode: String,
    pub model: String,
    pub effort: String,
    /// Tokens actually billed across every stored verdict — read, not estimated.
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Messages the gate refused to send. Not verdicts.
    pub withheld: i64,
    pub verdicts: i64,
    pub today: i64,
    pub daily_cap: i64,
    pub pending: usize,
    pub provider_ready: bool,
}

/// One pass. Returns immediately in `off`.
pub async fn run_pass(state: &AppState) -> Result<TriagePassStats, AppError> {
    let mut stats = TriagePassStats::default();

    let mode = triage_gate::effective_mode();
    if !mode.may_send() {
        return Ok(stats);
    }

    // Cap check and batch read share one lock scope, then it is dropped for
    // the whole network section.
    let batch = {
        let conn = state.db.safe_lock();
        if db::triage::count_since(&conn, "datetime('now','-1 day')")? >= DAILY_CAP {
            stats.capped = true;
            return Ok(stats);
        }
        db::triage::list_pending(&conn, PROMPT_VERSION, BATCH_PER_PASS)?
    };
    if batch.is_empty() {
        return Ok(stats);
    }

    // All three read the credential store, so they are read ONCE per pass, not
    // once per message: on the signed app each read is a Keychain hit
    // (gotcha #31), and a 5-message pass would otherwise take 15.
    let model = triage::effective_model();
    let effort = triage::effective_effort();
    let client = match InferenceClient::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("triage: inference provider not configured, pass skipped: {e}");
            return Ok(stats);
        }
    };

    for msg in batch {
        stats.considered += 1;

        let to: Vec<String> = parse_addrs(msg.to_list.as_deref());
        let cc: Vec<String> = parse_addrs(msg.cc_list.as_deref());
        let gated = triage_gate::gate(
            mode,
            &MessageForTriage {
                from_email: &msg.from_email,
                from_name: msg.from_name.as_deref(),
                to: &to,
                cc: &cc,
                date: &msg.date,
                subject: msg.subject.as_deref(),
                body: Some(&msg.body),
            },
        );

        let safe = match gated {
            Gate::Send(safe) => safe,
            Gate::Withheld(reason) => {
                // Recorded, but NOT as a verdict. `mark_withheld` writes a row
                // whose only job is to take this message out of `list_pending`;
                // `get` filters it, so it reaches Needs You as nothing at all
                // rather than as "no reply needed".
                //
                // Leaving it merely pending — which is what this did first —
                // livelocked the pass: `list_pending` is newest-first with a
                // limit of five, so the same withheld messages were re-picked
                // every two minutes and nothing behind them was ever reached.
                let conn = state.db.safe_lock();
                if let Err(e) = db::triage::mark_withheld(
                    &conn,
                    &msg.account_id,
                    &msg.folder_name,
                    msg.uid,
                    &reason.reason(),
                    PROMPT_VERSION,
                ) {
                    log::warn!("triage: recording withheld failed for {}: {e}", msg.uid);
                }
                log::debug!(
                    "triage: withheld {}/{} ({})",
                    msg.folder_name,
                    msg.uid,
                    reason.reason()
                );
                stats.withheld += 1;
                continue;
            }
        };

        match triage::classify(&client, &safe, &model, &effort).await {
            Ok(c) => {
                let conn = state.db.safe_lock();
                match db::triage::upsert(
                    &conn,
                    &msg.account_id,
                    &msg.folder_name,
                    msg.uid,
                    &c.verdict,
                    PROMPT_VERSION,
                    c.input_tokens,
                    c.output_tokens,
                ) {
                    Ok(()) => stats.classified += 1,
                    Err(e) => {
                        log::warn!("triage: storing verdict failed for {}: {e}", msg.uid);
                        stats.failed += 1;
                    }
                }
            }
            Err(e) => {
                // Nothing stored — the message stays pending and is retried on
                // a later pass. A parse failure must never become a verdict.
                log::warn!("triage: classify failed for {}: {e}", msg.uid);
                stats.failed += 1;
            }
        }
    }

    if stats.classified > 0 || stats.withheld > 0 {
        log::info!(
            "triage pass [{}]: {} classified, {} withheld, {} failed",
            mode.as_str(),
            stats.classified,
            stats.withheld,
            stats.failed
        );
    }
    Ok(stats)
}

fn parse_addrs(json: Option<&str>) -> Vec<String> {
    let Some(raw) = json else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .ok()
        .map(|v| {
            v.iter()
                .filter_map(|e| e.get("email").and_then(|x| x.as_str()))
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// --- commands -------------------------------------------------------------

#[tauri::command]
pub async fn get_triage_status(state: State<'_, AppState>) -> Result<TriageStatus, AppError> {
    let mode = triage_gate::effective_mode();
    let model = triage::effective_model();
    let effort = triage::effective_effort();
    let conn = state.db.safe_lock();
    let (input_tokens, output_tokens) = db::triage::token_totals(&conn)?;
    let withheld = db::triage::withheld_count(&conn)?;
    Ok(TriageStatus {
        mode: mode.as_str().to_string(),
        model,
        effort,
        input_tokens,
        output_tokens,
        withheld,
        verdicts: db::triage::count(&conn)?,
        today: db::triage::count_since(&conn, "datetime('now','-1 day')")?,
        daily_cap: DAILY_CAP,
        pending: db::triage::list_pending(&conn, PROMPT_VERSION, 10_000)?.len(),
        provider_ready: InferenceClient::load().is_ok(),
    })
}

#[tauri::command]
pub async fn set_triage_mode(mode: String) -> Result<String, AppError> {
    // Anything unrecognized resolves to Off rather than erroring, matching the
    // read path — the setting can never be left in a state that sends mail by
    // accident.
    let resolved = triage_gate::effective_mode_from(Some(&mode));
    triage_gate::save_mode(resolved)?;
    Ok(resolved.as_str().to_string())
}

#[tauri::command]
pub async fn set_account_triage_enabled(
    state: State<'_, AppState>,
    account_id: String,
    enabled: bool,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::accounts::set_triage_enabled(&conn, &account_id, enabled)?;
    Ok(())
}

#[tauri::command]
pub async fn set_triage_model(model: String) -> Result<String, AppError> {
    triage::save_model(&model)?;
    Ok(triage::effective_model())
}

#[tauri::command]
pub async fn set_triage_effort(effort: String) -> Result<String, AppError> {
    triage::save_effort(&effort)?;
    Ok(triage::effective_effort())
}

/// Run a pass now, for the settings panel's "Run once" button.
#[tauri::command]
pub async fn run_triage_pass_now(
    state: State<'_, AppState>,
) -> Result<TriagePassStats, AppError> {
    run_pass(&state).await
}
