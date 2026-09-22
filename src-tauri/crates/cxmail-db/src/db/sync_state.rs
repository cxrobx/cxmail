use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct SyncStateRow {
    pub account_id: String,
    pub folder_name: String,
    pub last_uid: u32,
    pub uidvalidity: Option<u32>,
    pub last_synced: Option<String>,
    pub last_flags_reconciled_at: Option<String>,
    pub last_existence_reconciled_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
}

pub fn get(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<Option<SyncStateRow>, AppError> {
    conn.query_row(
        "SELECT account_id, folder_name, last_uid, uidvalidity, last_synced,
                last_flags_reconciled_at, last_existence_reconciled_at,
                last_error, last_error_at
         FROM sync_state
         WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
        |row| {
            Ok(SyncStateRow {
                account_id: row.get(0)?,
                folder_name: row.get(1)?,
                last_uid: row.get::<_, i64>(2)? as u32,
                uidvalidity: row.get(3)?,
                last_synced: row.get(4)?,
                last_flags_reconciled_at: row.get(5)?,
                last_existence_reconciled_at: row.get(6)?,
                last_error: row.get(7)?,
                last_error_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(AppError::Database)
}

pub fn seed_if_missing_from_local_max(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<SyncStateRow, AppError> {
    if let Some(state) = get(conn, account_id, folder_name)? {
        return Ok(state);
    }

    let last_uid: u32 = conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
        |row| row.get::<_, i64>(0).map(|value| value as u32),
    )?;

    conn.execute(
        "INSERT INTO sync_state (account_id, folder_name, last_uid)
         VALUES (?1, ?2, ?3)",
        params![account_id, folder_name, last_uid],
    )?;

    get(conn, account_id, folder_name)?
        .ok_or_else(|| AppError::General("Failed to seed sync_state row".to_string()))
}

pub fn upsert_success(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    last_uid: u32,
    uidvalidity: u32,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO sync_state (
            account_id, folder_name, last_uid, uidvalidity, last_synced, last_error, last_error_at
         ) VALUES (?1, ?2, ?3, ?4, datetime('now'), NULL, NULL)
         ON CONFLICT(account_id, folder_name) DO UPDATE SET
            last_uid = excluded.last_uid,
            uidvalidity = excluded.uidvalidity,
            last_synced = excluded.last_synced,
            last_error = NULL,
            last_error_at = NULL",
        params![account_id, folder_name, last_uid, uidvalidity],
    )?;
    Ok(())
}

pub fn record_error(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    error: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO sync_state (
            account_id, folder_name, last_error, last_error_at
         ) VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(account_id, folder_name) DO UPDATE SET
            last_error = excluded.last_error,
            last_error_at = excluded.last_error_at",
        params![account_id, folder_name, error],
    )?;
    Ok(())
}

pub fn reset_for_uidvalidity_change(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO sync_state (
            account_id, folder_name, last_uid, uidvalidity,
            last_flags_reconciled_at, last_existence_reconciled_at
         ) VALUES (?1, ?2, 0, NULL, NULL, NULL)
         ON CONFLICT(account_id, folder_name) DO UPDATE SET
            last_uid = 0,
            uidvalidity = NULL,
            last_flags_reconciled_at = NULL,
            last_existence_reconciled_at = NULL",
        params![account_id, folder_name],
    )?;
    Ok(())
}

pub fn mark_flags_reconciled(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE sync_state
         SET last_flags_reconciled_at = datetime('now')
         WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
    )?;
    Ok(())
}

pub fn mark_existence_reconciled(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE sync_state
         SET last_existence_reconciled_at = datetime('now')
         WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "
            CREATE TABLE messages (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL
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
            ",
        )
        .expect("schema");
        conn
    }

    #[test]
    fn seed_uses_max_local_uid_when_messages_exist() {
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid) VALUES (?1, ?2, ?3)",
            params!["acc-1", "INBOX", 41],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid) VALUES (?1, ?2, ?3)",
            params!["acc-1", "INBOX", 99],
        )
        .unwrap();

        let state = seed_if_missing_from_local_max(&conn, "acc-1", "INBOX").unwrap();

        assert_eq!(state.last_uid, 99);
        assert_eq!(state.uidvalidity, None);
    }

    #[test]
    fn seed_uses_zero_when_folder_is_empty() {
        let conn = setup_conn();

        let state = seed_if_missing_from_local_max(&conn, "acc-1", "INBOX").unwrap();

        assert_eq!(state.last_uid, 0);
    }

    #[test]
    fn upsert_success_advances_checkpoint() {
        let conn = setup_conn();
        seed_if_missing_from_local_max(&conn, "acc-1", "INBOX").unwrap();

        upsert_success(&conn, "acc-1", "INBOX", 123, 456).unwrap();
        let state = get(&conn, "acc-1", "INBOX").unwrap().unwrap();

        assert_eq!(state.last_uid, 123);
        assert_eq!(state.uidvalidity, Some(456));
        assert!(state.last_synced.is_some());
    }

    #[test]
    fn reset_clears_checkpoint_after_uidvalidity_change() {
        let conn = setup_conn();
        upsert_success(&conn, "acc-1", "INBOX", 123, 456).unwrap();
        mark_flags_reconciled(&conn, "acc-1", "INBOX").unwrap();
        mark_existence_reconciled(&conn, "acc-1", "INBOX").unwrap();

        reset_for_uidvalidity_change(&conn, "acc-1", "INBOX").unwrap();
        let state = get(&conn, "acc-1", "INBOX").unwrap().unwrap();

        assert_eq!(state.last_uid, 0);
        assert_eq!(state.uidvalidity, None);
        assert!(state.last_flags_reconciled_at.is_none());
        assert!(state.last_existence_reconciled_at.is_none());
    }
}
