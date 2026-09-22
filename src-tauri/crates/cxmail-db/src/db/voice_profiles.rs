use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceProfileRow {
    pub account_id: String,
    pub voice_profile_json: String,
    pub voice_examples_json: String,
    pub model_used: Option<String>,
    pub sample_count: i64,
    pub generated_at: String,
}

pub fn get_by_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<VoiceProfileRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT account_id, voice_profile_json, voice_examples_json, model_used, sample_count, generated_at
             FROM voice_profiles WHERE account_id = ?1",
            params![account_id],
            |row| {
                Ok(VoiceProfileRow {
                    account_id: row.get(0)?,
                    voice_profile_json: row.get(1)?,
                    voice_examples_json: row.get(2)?,
                    model_used: row.get(3)?,
                    sample_count: row.get(4)?,
                    generated_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn upsert(
    conn: &Connection,
    account_id: &str,
    voice_profile_json: &str,
    voice_examples_json: &str,
    model_used: &str,
    sample_count: i64,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO voice_profiles (account_id, voice_profile_json, voice_examples_json, model_used, sample_count, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(account_id) DO UPDATE SET
             voice_profile_json = excluded.voice_profile_json,
             voice_examples_json = excluded.voice_examples_json,
             model_used = excluded.model_used,
             sample_count = excluded.sample_count,
             generated_at = excluded.generated_at",
        params![
            account_id,
            voice_profile_json,
            voice_examples_json,
            model_used,
            sample_count
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, account_id: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM voice_profiles WHERE account_id = ?1",
        params![account_id],
    )?;
    Ok(())
}
