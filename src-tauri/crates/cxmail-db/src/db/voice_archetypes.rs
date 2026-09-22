use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeRow {
    pub account_id: String,
    pub archetype_id: String,
    pub name: String,
    pub description: String,
    pub profile_json: String,
    pub sample_count: i64,
    pub model_used: Option<String>,
    pub generated_at: String,
}

pub fn get(
    conn: &Connection,
    account_id: &str,
    archetype_id: &str,
) -> Result<Option<ArchetypeRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT account_id, archetype_id, name, description, profile_json,
                    sample_count, model_used, generated_at
             FROM voice_archetypes
             WHERE account_id = ?1 AND archetype_id = ?2",
            params![account_id, archetype_id],
            row_to_archetype,
        )
        .optional()?;
    Ok(row)
}

pub fn list_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<ArchetypeRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT account_id, archetype_id, name, description, profile_json,
                sample_count, model_used, generated_at
         FROM voice_archetypes
         WHERE account_id = ?1
         ORDER BY sample_count DESC, name ASC",
    )?;
    let rows = stmt
        .query_map(params![account_id], row_to_archetype)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn insert(
    conn: &Connection,
    account_id: &str,
    archetype_id: &str,
    name: &str,
    description: &str,
    profile_json: &str,
    model_used: &str,
    sample_count: i64,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO voice_archetypes
            (account_id, archetype_id, name, description, profile_json,
             sample_count, model_used, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
        params![
            account_id,
            archetype_id,
            name,
            description,
            profile_json,
            sample_count,
            model_used
        ],
    )?;
    Ok(())
}

pub fn rename(
    conn: &Connection,
    account_id: &str,
    archetype_id: &str,
    new_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE voice_archetypes SET name = ?3
         WHERE account_id = ?1 AND archetype_id = ?2",
        params![account_id, archetype_id, new_name],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, account_id: &str, archetype_id: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM voice_archetypes
         WHERE account_id = ?1 AND archetype_id = ?2",
        params![account_id, archetype_id],
    )?;
    Ok(())
}

pub fn delete_all(conn: &Connection, account_id: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM voice_archetypes WHERE account_id = ?1",
        params![account_id],
    )?;
    Ok(())
}

/// Insert (or replace) an archetype assignment for a single sent message.
pub fn assign_message(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    archetype_id: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO message_archetype
            (account_id, folder_name, uid, archetype_id, assigned_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))
         ON CONFLICT(account_id, folder_name, uid) DO UPDATE SET
             archetype_id = excluded.archetype_id,
             assigned_at = excluded.assigned_at",
        params![account_id, folder_name, uid, archetype_id],
    )?;
    Ok(())
}

/// Pick the dominant archetype for a recipient: the archetype most often
/// assigned to sent messages addressed to that recipient.
pub fn archetype_for_recipient(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<Option<String>, AppError> {
    let needle = recipient_email.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(None);
    }
    // Match against `messages.to_list` (envelope-derived as of v34) with a
    // fallback to `mb.to_json` for old prefetched messages.
    let row = conn
        .query_row(
            "SELECT ma.archetype_id
             FROM message_archetype ma
             JOIN messages m
               ON m.account_id = ma.account_id
              AND m.folder_name = ma.folder_name
              AND m.uid = ma.uid
             LEFT JOIN message_bodies mb
               ON mb.account_id = ma.account_id
              AND mb.folder_name = ma.folder_name
              AND mb.uid = ma.uid
             WHERE ma.account_id = ?1
               AND (
                 EXISTS (
                   SELECT 1 FROM json_each(COALESCE(NULLIF(m.to_list, ''), '[]')) je
                   WHERE LOWER(json_extract(je.value, '$.email')) = ?2
                 )
                 OR (
                   mb.to_json IS NOT NULL AND EXISTS (
                     SELECT 1 FROM json_each(mb.to_json) je
                     WHERE LOWER(json_extract(je.value, '$.email')) = ?2
                   )
                 )
               )
             GROUP BY ma.archetype_id
             ORDER BY COUNT(*) DESC
             LIMIT 1",
            params![account_id, needle],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(row)
}

fn row_to_archetype(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchetypeRow> {
    Ok(ArchetypeRow {
        account_id: row.get(0)?,
        archetype_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        profile_json: row.get(4)?,
        sample_count: row.get(5)?,
        model_used: row.get(6)?,
        generated_at: row.get(7)?,
    })
}
