use crate::error::AppError;
use rusqlite::{params, Connection};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct RecipientVoiceBucketRow {
    pub bucket_id: String,
    pub centroid: Vec<f64>,
    pub sample_count: i64,
    pub first_message_date: String,
    pub last_message_date: String,
}

#[derive(Debug, Clone)]
pub struct BucketedRecipientSampleRow {
    pub bucket_id: String,
    pub bucket_sample_count: i64,
    pub folder_name: String,
    pub uid: u32,
    pub date: String,
    pub body: String,
    pub feature: Vec<f64>,
}

fn encode_feature(feature: &[f64]) -> Result<String, AppError> {
    serde_json::to_string(feature)
        .map_err(|e| AppError::Parse(format!("voice feature serialize: {e}")))
}

fn decode_feature(raw: &str) -> Result<Vec<f64>, AppError> {
    serde_json::from_str(raw).map_err(|e| AppError::Parse(format!("voice feature parse: {e}")))
}

pub fn list_buckets(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<Vec<RecipientVoiceBucketRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT bucket_id, centroid_json, sample_count,
                first_message_date, last_message_date
         FROM recipient_voice_buckets
         WHERE account_id = ?1 AND recipient_email = ?2
         ORDER BY datetime(first_message_date), bucket_id",
    )?;
    let raw = stmt
        .query_map(params![account_id, recipient_email.to_lowercase()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    raw.into_iter()
        .map(
            |(bucket_id, centroid_json, sample_count, first_message_date, last_message_date)| {
                Ok(RecipientVoiceBucketRow {
                    bucket_id,
                    centroid: decode_feature(&centroid_json)?,
                    sample_count,
                    first_message_date,
                    last_message_date,
                })
            },
        )
        .collect()
}

pub fn assigned_message_keys(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<HashSet<(String, u32)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT folder_name, uid
         FROM recipient_voice_sample_buckets
         WHERE account_id = ?1 AND recipient_email = ?2",
    )?;
    let rows = stmt
        .query_map(params![account_id, recipient_email.to_lowercase()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
        })?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

pub fn insert_bucket(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    bucket_id: &str,
    centroid: &[f64],
    message_date: &str,
) -> Result<(), AppError> {
    let centroid_json = encode_feature(centroid)?;
    conn.execute(
        "INSERT OR IGNORE INTO recipient_voice_buckets
            (account_id, recipient_email, bucket_id, centroid_json, sample_count,
             first_message_date, last_message_date, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5, datetime('now'))",
        params![
            account_id,
            recipient_email.to_lowercase(),
            bucket_id,
            centroid_json,
            message_date,
        ],
    )?;
    Ok(())
}

pub fn insert_assignment(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    folder_name: &str,
    uid: u32,
    message_date: &str,
    bucket_id: &str,
    similarity: f64,
    feature: &[f64],
) -> Result<bool, AppError> {
    let feature_json = encode_feature(feature)?;
    let changed = conn.execute(
        "INSERT OR IGNORE INTO recipient_voice_sample_buckets
            (account_id, recipient_email, folder_name, uid, message_date,
             bucket_id, similarity, feature_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            account_id,
            recipient_email.to_lowercase(),
            folder_name,
            uid,
            message_date,
            bucket_id,
            similarity,
            feature_json,
        ],
    )?;
    Ok(changed == 1)
}

pub fn update_bucket_after_assignment(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    bucket_id: &str,
    centroid: &[f64],
    sample_count: i64,
    first_message_date: &str,
    last_message_date: &str,
) -> Result<(), AppError> {
    let centroid_json = encode_feature(centroid)?;
    conn.execute(
        "UPDATE recipient_voice_buckets
         SET centroid_json = ?4,
             sample_count = ?5,
             first_message_date = ?6,
             last_message_date = ?7,
             updated_at = datetime('now')
         WHERE account_id = ?1 AND recipient_email = ?2 AND bucket_id = ?3",
        params![
            account_id,
            recipient_email.to_lowercase(),
            bucket_id,
            centroid_json,
            sample_count,
            first_message_date,
            last_message_date,
        ],
    )?;
    Ok(())
}

pub fn list_bucketed_samples(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<Vec<BucketedRecipientSampleRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT a.bucket_id, b.sample_count, a.folder_name, a.uid,
                m.date, mb.plain_text, a.feature_json
         FROM recipient_voice_sample_buckets a
         JOIN recipient_voice_buckets b
           ON b.account_id = a.account_id
          AND b.recipient_email = a.recipient_email
          AND b.bucket_id = a.bucket_id
         JOIN messages m
           ON m.account_id = a.account_id
          AND m.folder_name = a.folder_name
          AND m.uid = a.uid
         JOIN message_bodies mb
           ON mb.account_id = a.account_id
          AND mb.folder_name = a.folder_name
          AND mb.uid = a.uid
         WHERE a.account_id = ?1
           AND a.recipient_email = ?2
           AND mb.plain_text IS NOT NULL
         ORDER BY datetime(m.date) DESC, a.uid DESC",
    )?;
    let raw = stmt
        .query_map(params![account_id, recipient_email.to_lowercase()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? as u32,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    raw.into_iter()
        .map(
            |(bucket_id, bucket_sample_count, folder_name, uid, date, body, feature_json)| {
                Ok(BucketedRecipientSampleRow {
                    bucket_id,
                    bucket_sample_count,
                    folder_name,
                    uid,
                    date,
                    body,
                    feature: decode_feature(&feature_json)?,
                })
            },
        )
        .collect()
}

pub fn assignment_count(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<i64, AppError> {
    let count = conn.query_row(
        "SELECT COUNT(*)
         FROM recipient_voice_sample_buckets
         WHERE account_id = ?1 AND recipient_email = ?2",
        params![account_id, recipient_email.to_lowercase()],
        |row| row.get(0),
    )?;
    Ok(count)
}
