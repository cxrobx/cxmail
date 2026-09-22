use crate::db;
use crate::email::smtp;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Serialize;
use tauri::State;

/// Outcome of scheduling a send: the new row id plus how many earlier *pending*
/// sends to the same recipients+subject it replaced. Lets the UI tell the user
/// "the latest schedule won" rather than silently double-queueing.
#[derive(Debug, Serialize)]
pub struct ScheduleResult {
    pub id: i64,
    pub superseded: usize,
}

/// Lowercased, trimmed, sorted, de-duplicated `to:` addresses — the recipient
/// half of a scheduled-send identity.
fn recipient_key(email: &smtp::OutgoingEmail) -> Vec<String> {
    let mut addrs: Vec<String> = email
        .to
        .iter()
        .map(|r| r.email.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs
}

/// Subject with surrounding/internal whitespace collapsed and lowercased, so two
/// schedules of the same reply whose subjects differ only by stray tabs/spaces
/// (observed in the wild — `\t` vs `\t\t`) still compare equal.
fn subject_key(subject: &str) -> String {
    subject.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[tauri::command]
pub async fn schedule_send(
    state: State<'_, AppState>,
    account_id: String,
    email: smtp::OutgoingEmail,
    send_at: String,
) -> Result<ScheduleResult, AppError> {
    let email_json = serde_json::to_string(&email)
        .map_err(|e| AppError::General(format!("Failed to serialize email: {}", e)))?;

    let new_to = recipient_key(&email);
    let new_subject = subject_key(&email.subject);

    let conn = state.db.safe_lock();

    // Rescheduling the same email must REPLACE its prior schedule, not queue a
    // second copy (that double-sends to the recipient). Supersede any earlier
    // pending send on this account whose recipients + normalized subject match.
    let mut superseded = 0usize;
    for row in db::scheduled::list_pending(&conn)? {
        if row.account_id != account_id {
            continue;
        }
        let existing: smtp::OutgoingEmail = match serde_json::from_str(&row.email_json) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if recipient_key(&existing) == new_to && subject_key(&existing.subject) == new_subject {
            db::scheduled::delete(&conn, row.id)?;
            superseded += 1;
        }
    }

    let id = db::scheduled::insert(&conn, &account_id, &email_json, &send_at)?;
    log::info!(
        "Scheduled email {} for {} (account {}); superseded {} earlier pending",
        id, send_at, account_id, superseded
    );
    Ok(ScheduleResult { id, superseded })
}

#[tauri::command]
pub async fn list_scheduled(
    state: State<'_, AppState>,
) -> Result<Vec<db::scheduled::ScheduledRow>, AppError> {
    let conn = state.db.safe_lock();
    db::scheduled::list_pending(&conn)
}

#[tauri::command]
pub async fn cancel_scheduled(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::scheduled::delete(&conn, id)?;
    log::info!("Cancelled scheduled email {}", id);
    Ok(())
}

#[tauri::command]
pub async fn edit_scheduled(
    state: State<'_, AppState>,
    id: i64,
    email: smtp::OutgoingEmail,
    send_at: String,
) -> Result<(), AppError> {
    let email_json = serde_json::to_string(&email)
        .map_err(|e| AppError::General(format!("Failed to serialize email: {}", e)))?;

    let conn = state.db.safe_lock();
    db::scheduled::update_email(&conn, id, &email_json, &send_at)?;
    log::info!("Updated scheduled email {} to send at {}", id, send_at);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::smtp::{EmailRecipient, OutgoingEmail};

    fn email(to: &[&str], subject: &str) -> OutgoingEmail {
        OutgoingEmail {
            from_email: "me@example.com".into(),
            from_name: None,
            to: to
                .iter()
                .map(|e| EmailRecipient { name: None, email: (*e).into() })
                .collect(),
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            html_body: String::new(),
            plain_body: None,
            in_reply_to: None,
            references: None,
            track_opens: None,
            attachments: vec![],
        }
    }

    #[test]
    fn subject_key_collapses_stray_whitespace() {
        // The real duplicate differed only by `\t` vs `\t\t` in the subject;
        // both must normalize to the same key so the second supersedes the first.
        let a = "Re: Meeting - Wednesday,\t June 17th @ 9 am cst";
        let b = "Re: Meeting - Wednesday,\t\t June 17th @ 9 am cst";
        assert_eq!(subject_key(a), subject_key(b));
    }

    #[test]
    fn recipient_key_is_case_and_order_insensitive() {
        assert_eq!(
            recipient_key(&email(&["Dana@Northwind.example", "a@b.com"], "x")),
            recipient_key(&email(&["a@b.com", "dana@northwind.example"], "x")),
        );
    }

    #[test]
    fn different_recipient_or_subject_does_not_match() {
        assert_ne!(
            recipient_key(&email(&["dana@northwind.example"], "x")),
            recipient_key(&email(&["other@northwind.example"], "x")),
        );
        assert_ne!(subject_key("Re: deal terms"), subject_key("Re: invoice"));
    }
}
