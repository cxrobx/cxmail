use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledRow {
    pub id: i64,
    pub account_id: String,
    pub email_json: String,
    pub send_at: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
}

pub fn insert(
    conn: &Connection,
    account_id: &str,
    email_json: &str,
    send_at: &str,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO scheduled_emails (account_id, email_json, send_at)
         VALUES (?1, ?2, ?3)",
        params![account_id, email_json, send_at],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Return scheduled emails whose send_at time has passed and are still pending.
pub fn list_due(conn: &Connection) -> Result<Vec<ScheduledRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email_json, send_at, status, error_message, created_at
         FROM scheduled_emails
         WHERE datetime(send_at) <= datetime('now') AND status = 'pending'
         ORDER BY send_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ScheduledRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                email_json: row.get(2)?,
                send_at: row.get(3)?,
                status: row.get(4)?,
                error_message: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Return all pending scheduled emails (for the Scheduled view).
pub fn list_pending(conn: &Connection) -> Result<Vec<ScheduledRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email_json, send_at, status, error_message, created_at
         FROM scheduled_emails
         WHERE status = 'pending'
         ORDER BY send_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ScheduledRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                email_json: row.get(2)?,
                send_at: row.get(3)?,
                status: row.get(4)?,
                error_message: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn update_status(
    conn: &Connection,
    id: i64,
    status: &str,
    error_message: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE scheduled_emails SET status = ?1, error_message = ?2 WHERE id = ?3",
        params![status, error_message, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM scheduled_emails WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<ScheduledRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email_json, send_at, status, error_message, created_at
         FROM scheduled_emails WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(ScheduledRow {
            id: row.get(0)?,
            account_id: row.get(1)?,
            email_json: row.get(2)?,
            send_at: row.get(3)?,
            status: row.get(4)?,
            error_message: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    match rows.next() {
        Some(Ok(row)) => Ok(Some(row)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

pub fn update_email(
    conn: &Connection,
    id: i64,
    email_json: &str,
    send_at: &str,
) -> Result<(), AppError> {
    let affected = conn.execute(
        "UPDATE scheduled_emails SET email_json = ?1, send_at = ?2
         WHERE id = ?3 AND status = 'pending'",
        params![email_json, send_at, id],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound("Scheduled email not found or already processed".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE scheduled_emails (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                email_json TEXT NOT NULL,
                send_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        conn
    }

    /// Regression for the ISO-vs-`datetime('now')` lexicographic bug. A row dated
    /// today at the very start of the UTC day is always in the past, yet shares
    /// today's UTC date. With a raw string compare
    /// (`'2026-06-03T00:00:00.001Z' <= '2026-06-03 12:00:00'`) the `T` (0x54) vs
    /// space (0x20) at index 10 wrongly excludes it; wrapping both operands in
    /// `datetime()` fixes it. A naive 2020/2999 fixture would NOT catch this.
    #[test]
    fn list_due_returns_iso_row_dated_today_at_utc_midnight() {
        let conn = setup();
        conn.execute(
            "INSERT INTO scheduled_emails (account_id, email_json, send_at, status)
             VALUES ('a', '{}', date('now') || 'T00:00:00.001Z', 'pending')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert_eq!(due.len(), 1, "today's UTC-midnight ISO row must be due");
    }

    #[test]
    fn list_due_excludes_future_and_non_pending() {
        let conn = setup();
        // Future pending row: not due.
        conn.execute(
            "INSERT INTO scheduled_emails (account_id, email_json, send_at, status)
             VALUES ('a', '{}', '2999-12-31T23:59:59.000Z', 'pending')",
            [],
        )
        .unwrap();
        // Past row, already sent: excluded by the status filter.
        conn.execute(
            "INSERT INTO scheduled_emails (account_id, email_json, send_at, status)
             VALUES ('a', '{}', date('now') || 'T00:00:00.001Z', 'sent')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert!(due.is_empty(), "future and non-pending rows must be excluded");
    }
}
