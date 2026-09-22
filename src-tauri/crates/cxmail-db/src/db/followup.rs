use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FollowupRow {
    pub id: i64,
    pub account_id: String,
    pub sent_message_id: String,
    pub sender_email: String,
    pub to_email: String,
    pub subject: Option<String>,
    pub remind_at: String,
    pub status: String,
    pub created_at: String,
}

pub fn insert(
    conn: &Connection,
    account_id: &str,
    sent_message_id: &str,
    sender_email: &str,
    to_email: &str,
    subject: Option<&str>,
    remind_at: &str,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO followup_reminders
         (account_id, sent_message_id, sender_email, to_email, subject, remind_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![account_id, sent_message_id, sender_email, to_email, subject, remind_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM followup_reminders WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn update_status(conn: &Connection, id: i64, status: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE followup_reminders SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

/// Return followup reminders whose remind_at time has passed and are still pending.
pub fn list_due(conn: &Connection) -> Result<Vec<FollowupRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, sent_message_id, sender_email, to_email, subject,
                remind_at, status, created_at
         FROM followup_reminders
         WHERE datetime(remind_at) <= datetime('now') AND status = 'pending'
         ORDER BY remind_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FollowupRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                sent_message_id: row.get(2)?,
                sender_email: row.get(3)?,
                to_email: row.get(4)?,
                subject: row.get(5)?,
                remind_at: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Return all pending followup reminders (for the Follow-ups view).
pub fn list_pending(conn: &Connection) -> Result<Vec<FollowupRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, sent_message_id, sender_email, to_email, subject,
                remind_at, status, created_at
         FROM followup_reminders
         WHERE status IN ('pending', 'fired')
         ORDER BY remind_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FollowupRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                sent_message_id: row.get(2)?,
                sender_email: row.get(3)?,
                to_email: row.get(4)?,
                subject: row.get(5)?,
                remind_at: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Check if a reply exists for a sent message (excluding self-replies).
pub fn check_reply_exists(
    conn: &Connection,
    sent_message_id: &str,
    sender_email: &str,
) -> Result<bool, AppError> {
    let pattern = format!("%{}%", sent_message_id);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages
         WHERE (in_reply_to = ?1 OR reference_ids LIKE ?2)
           AND from_email != ?3",
        params![sent_message_id, pattern, sender_email],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Auto-cancel any pending reminders that already have replies.
pub fn auto_cancel_replied(conn: &Connection) -> Result<u32, AppError> {
    let affected = conn.execute(
        "UPDATE followup_reminders SET status = 'auto-cancelled'
         WHERE status = 'pending'
         AND EXISTS (
             SELECT 1 FROM messages
             WHERE (messages.in_reply_to = followup_reminders.sent_message_id
                    OR messages.reference_ids LIKE '%' || followup_reminders.sent_message_id || '%')
             AND messages.from_email != followup_reminders.sender_email
         )",
        [],
    )?;
    Ok(affected as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE followup_reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                sent_message_id TEXT NOT NULL,
                sender_email TEXT NOT NULL,
                to_email TEXT NOT NULL,
                subject TEXT,
                remind_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(account_id, sent_message_id));",
        )
        .unwrap();
        conn
    }

    /// Regression for the ISO-vs-`datetime('now')` lexicographic bug — see
    /// `scheduled::tests` for the full explanation. A reminder whose `remind_at`
    /// is today at UTC midnight is always past and must be due.
    #[test]
    fn list_due_returns_iso_row_dated_today_at_utc_midnight() {
        let conn = setup();
        conn.execute(
            "INSERT INTO followup_reminders
                (account_id, sent_message_id, sender_email, to_email, remind_at, status)
             VALUES ('a', 'm1', 's@x.com', 't@x.com', date('now') || 'T00:00:00.001Z', 'pending')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert_eq!(due.len(), 1, "today's UTC-midnight ISO reminder must be due");
    }

    #[test]
    fn list_due_excludes_future_and_non_pending() {
        let conn = setup();
        // Future pending reminder: not due.
        conn.execute(
            "INSERT INTO followup_reminders
                (account_id, sent_message_id, sender_email, to_email, remind_at, status)
             VALUES ('a', 'm2', 's@x.com', 't@x.com', '2999-12-31T23:59:59.000Z', 'pending')",
            [],
        )
        .unwrap();
        // Past reminder already fired: excluded by the status filter.
        conn.execute(
            "INSERT INTO followup_reminders
                (account_id, sent_message_id, sender_email, to_email, remind_at, status)
             VALUES ('a', 'm3', 's@x.com', 't@x.com', date('now') || 'T00:00:00.001Z', 'fired')",
            [],
        )
        .unwrap();

        let due = list_due(&conn).unwrap();
        assert!(due.is_empty(), "future and non-pending reminders must be excluded");
    }
}
