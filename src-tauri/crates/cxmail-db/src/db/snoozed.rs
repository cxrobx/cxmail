use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SnoozedRow {
    pub id: i64,
    pub account_id: String,
    pub folder_name: String,
    pub uid: u32,
    pub wake_at: String,
}

pub fn insert(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    wake_at: &str,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO snoozed_messages (account_id, folder_name, uid, wake_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![account_id, folder_name, uid, wake_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM snoozed_messages WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn delete_by_message(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM snoozed_messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
        params![account_id, folder_name, uid],
    )?;
    Ok(())
}

/// Return snoozed messages whose wake_at time has passed.
pub fn list_due(conn: &Connection) -> Result<Vec<SnoozedRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, folder_name, uid, wake_at
         FROM snoozed_messages WHERE datetime(wake_at) <= datetime('now')
         ORDER BY wake_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SnoozedRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                folder_name: row.get(2)?,
                uid: row.get(3)?,
                wake_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Return all snoozed messages (for the Snoozed view).
pub fn list_all(conn: &Connection) -> Result<Vec<SnoozedRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, folder_name, uid, wake_at
         FROM snoozed_messages ORDER BY wake_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SnoozedRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                folder_name: row.get(2)?,
                uid: row.get(3)?,
                wake_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snoozed_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                wake_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(account_id, folder_name, uid));",
        )
        .unwrap();
        conn
    }

    /// Regression for the ISO-vs-`datetime('now')` lexicographic bug — see
    /// `scheduled::tests` for the full explanation. A snooze whose `wake_at` is
    /// today at UTC midnight is always past and must wake immediately.
    #[test]
    fn list_due_wakes_iso_row_dated_today_at_utc_midnight() {
        let conn = setup();
        conn.execute(
            "INSERT INTO snoozed_messages (account_id, folder_name, uid, wake_at)
             VALUES ('a', 'INBOX', 1, date('now') || 'T00:00:00.001Z')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert_eq!(due.len(), 1, "today's UTC-midnight ISO snooze must wake");
    }

    #[test]
    fn list_due_excludes_future() {
        let conn = setup();
        conn.execute(
            "INSERT INTO snoozed_messages (account_id, folder_name, uid, wake_at)
             VALUES ('a', 'INBOX', 2, '2999-12-31T23:59:59.000Z')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert!(due.is_empty(), "future snooze must not wake");
    }
}
