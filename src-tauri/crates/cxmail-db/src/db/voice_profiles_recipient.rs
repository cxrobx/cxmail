use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipientProfileRow {
    pub account_id: String,
    pub recipient_email: String,
    pub profile_json: String,
    pub sample_count: i64,
    pub last_extracted_message_date: String,
    pub model_used: Option<String>,
    pub generated_at: String,
    pub learning_version: i64,
}

pub fn get(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<Option<RecipientProfileRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT account_id, recipient_email, profile_json, sample_count,
                    last_extracted_message_date, model_used, generated_at,
                    learning_version
             FROM voice_profiles_recipient
             WHERE account_id = ?1 AND recipient_email = ?2",
            params![account_id, recipient_email.to_lowercase()],
            |row| {
                Ok(RecipientProfileRow {
                    account_id: row.get(0)?,
                    recipient_email: row.get(1)?,
                    profile_json: row.get(2)?,
                    sample_count: row.get(3)?,
                    last_extracted_message_date: row.get(4)?,
                    model_used: row.get(5)?,
                    generated_at: row.get(6)?,
                    learning_version: row.get(7)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn upsert(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    profile_json: &str,
    model_used: &str,
    sample_count: i64,
    last_extracted_message_date: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO voice_profiles_recipient
            (account_id, recipient_email, profile_json, sample_count,
             last_extracted_message_date, model_used, generated_at, learning_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), 1)
         ON CONFLICT(account_id, recipient_email) DO UPDATE SET
             profile_json = excluded.profile_json,
             sample_count = excluded.sample_count,
             last_extracted_message_date = excluded.last_extracted_message_date,
             model_used = excluded.model_used,
             generated_at = excluded.generated_at,
             learning_version = excluded.learning_version",
        params![
            account_id,
            recipient_email.to_lowercase(),
            profile_json,
            sample_count,
            last_extracted_message_date,
            model_used,
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, account_id: &str, recipient_email: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM voice_profiles_recipient
         WHERE account_id = ?1 AND recipient_email = ?2",
        params![account_id, recipient_email.to_lowercase()],
    )?;
    Ok(())
}

pub fn list_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<RecipientProfileRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT account_id, recipient_email, profile_json, sample_count,
                last_extracted_message_date, model_used, generated_at,
                learning_version
         FROM voice_profiles_recipient
         WHERE account_id = ?1
         ORDER BY datetime(generated_at) DESC",
    )?;
    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok(RecipientProfileRow {
                account_id: row.get(0)?,
                recipient_email: row.get(1)?,
                profile_json: row.get(2)?,
                sample_count: row.get(3)?,
                last_extracted_message_date: row.get(4)?,
                model_used: row.get(5)?,
                generated_at: row.get(6)?,
                learning_version: row.get(7)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}
