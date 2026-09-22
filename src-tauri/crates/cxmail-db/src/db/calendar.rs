use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CalendarEventRow {
    pub id: i64,
    pub account_id: String,
    pub folder_name: String,
    pub message_uid: u32,
    pub event_uid: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub status: Option<String>,
    pub method: Option<String>,
    pub rsvp_status: String,
    pub raw_ics: Option<String>,
    pub source: String,
    pub confidence: Option<f64>,
    pub dismissed: bool,
    pub created_at: String,
}

pub fn insert(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    message_uid: u32,
    event_uid: Option<&str>,
    summary: Option<&str>,
    description: Option<&str>,
    location: Option<&str>,
    dtstart: &str,
    dtend: Option<&str>,
    organizer_name: Option<&str>,
    organizer_email: Option<&str>,
    status: Option<&str>,
    method: Option<&str>,
    raw_ics: Option<&str>,
) -> Result<i64, AppError> {
    // Skip if an ICS row already exists for this (message, event_uid) so the
    // user's rsvp_status / dismissed state survives a re-parse. When event_uid
    // is absent we fall back to "one ICS row per message" to avoid duplicates.
    let existing_id: Option<i64> = if let Some(euid) = event_uid {
        conn.query_row(
            "SELECT id FROM calendar_events
             WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
               AND source = 'ics' AND event_uid = ?4
             LIMIT 1",
            params![account_id, folder_name, message_uid, euid],
            |r| r.get(0),
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT id FROM calendar_events
             WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
               AND source = 'ics' AND event_uid IS NULL
             LIMIT 1",
            params![account_id, folder_name, message_uid],
            |r| r.get(0),
        )
        .optional()?
    };
    if let Some(id) = existing_id {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO calendar_events
         (account_id, folder_name, message_uid, event_uid, summary, description, location, dtstart, dtend, organizer_name, organizer_email, status, method, raw_ics)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            account_id, folder_name, message_uid, event_uid, summary, description, location,
            dtstart, dtend, organizer_name, organizer_email, status, method, raw_ics,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_by_message(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<CalendarEventRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                source, confidence, dismissed, created_at
         FROM calendar_events
         WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
         ORDER BY dtstart ASC",
    )?;
    let rows = stmt
        .query_map(params![account_id, folder_name, uid], map_calendar_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn update_rsvp(conn: &Connection, id: i64, status: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE calendar_events SET rsvp_status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn list_upcoming(
    conn: &Connection,
    account_id: Option<&str>,
    limit: u32,
) -> Result<Vec<CalendarEventRow>, AppError> {
    let (sql, use_account) = if let Some(_) = account_id {
        (
            "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                    dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                    source, confidence, dismissed, created_at
             FROM calendar_events
             WHERE account_id = ?1 AND dismissed = 0 AND datetime(dtstart) >= datetime('now')
             ORDER BY dtstart ASC LIMIT ?2",
            true,
        )
    } else {
        (
            "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                    dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                    source, confidence, dismissed, created_at
             FROM calendar_events
             WHERE dismissed = 0 AND datetime(dtstart) >= datetime('now')
             ORDER BY dtstart ASC LIMIT ?1",
            false,
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = if use_account {
        stmt.query_map(params![account_id, limit], map_calendar_row)?
    } else {
        stmt.query_map(params![limit], map_calendar_row)?
    };
    let result = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(result)
}

fn map_calendar_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarEventRow> {
    Ok(CalendarEventRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_name: row.get(2)?,
        message_uid: row.get(3)?,
        event_uid: row.get(4)?,
        summary: row.get(5)?,
        description: row.get(6)?,
        location: row.get(7)?,
        dtstart: row.get(8)?,
        dtend: row.get(9)?,
        organizer_name: row.get(10)?,
        organizer_email: row.get(11)?,
        status: row.get(12)?,
        method: row.get(13)?,
        rsvp_status: row.get(14)?,
        raw_ics: row.get(15)?,
        source: row.get(16)?,
        confidence: row.get(17)?,
        dismissed: row.get::<_, i32>(18)? != 0,
        created_at: row.get(19)?,
    })
}

pub fn list_by_date_range(
    conn: &Connection,
    account_id: Option<&str>,
    range_start: &str,
    range_end: &str,
) -> Result<Vec<CalendarEventRow>, AppError> {
    let (sql, use_account) = if account_id.is_some() {
        (
            "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                    dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                    source, confidence, dismissed, created_at
             FROM calendar_events
             WHERE account_id = ?1 AND dismissed = 0 AND datetime(dtstart) >= datetime(?2) AND datetime(dtstart) < datetime(?3)
             ORDER BY dtstart ASC",
            true,
        )
    } else {
        (
            "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                    dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                    source, confidence, dismissed, created_at
             FROM calendar_events
             WHERE dismissed = 0 AND datetime(dtstart) >= datetime(?1) AND datetime(dtstart) < datetime(?2)
             ORDER BY dtstart ASC",
            false,
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = if use_account {
        stmt.query_map(params![account_id, range_start, range_end], map_calendar_row)?
    } else {
        stmt.query_map(params![range_start, range_end], map_calendar_row)?
    };
    let result = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(result)
}

pub fn insert_detected(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    message_uid: u32,
    summary: &str,
    dtstart: &str,
    dtend: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
    confidence: f64,
) -> Result<i64, AppError> {
    // One heuristic-detected row per message, preserving any user-set
    // dismissed flag on re-parse.
    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM calendar_events
             WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
               AND source = 'detected'
             LIMIT 1",
            params![account_id, folder_name, message_uid],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing_id {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO calendar_events
         (account_id, folder_name, message_uid, summary, dtstart, dtend, location, description, source, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'detected', ?9)",
        params![account_id, folder_name, message_uid, summary, dtstart, dtend, location, description, confidence],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Correct an existing heuristic-detected row in place.
///
/// [`insert_detected`] deliberately early-returns an existing row's id and never
/// touches its fields, which is right for the sync path — re-parsing the same
/// message must not churn rows. But it also means a detector *bug fix* is
/// invisible for mail already in the DB: the wrong row simply survives. This is
/// the one caller that may overwrite, and it exists for migrations.
///
/// Two things are deliberately preserved:
/// * **`dismissed`** — the user's decision to hide the event. Re-detecting is not
///   permission to un-hide it, and a delete-then-reinsert would silently do so.
/// * **the row id** — so anything holding it keeps resolving.
///
/// Scoped to `source = 'detected'`, so an ICS-sourced or user-created event can
/// never be rewritten by a heuristic. Returns true when a row was updated.
pub fn refresh_detected(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    message_uid: u32,
    summary: &str,
    dtstart: &str,
    dtend: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
    confidence: f64,
) -> Result<bool, AppError> {
    let changed = conn.execute(
        "UPDATE calendar_events
         SET summary = ?4, dtstart = ?5, dtend = ?6, location = ?7,
             description = ?8, confidence = ?9
         WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
           AND source = 'detected'",
        params![
            account_id,
            folder_name,
            message_uid,
            summary,
            dtstart,
            dtend,
            location,
            description,
            confidence
        ],
    )?;
    Ok(changed > 0)
}

pub fn dismiss_event(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE calendar_events SET dismissed = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE calendar_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, message_uid INTEGER NOT NULL, event_uid TEXT,
                summary TEXT, description TEXT, location TEXT, dtstart TEXT NOT NULL, dtend TEXT,
                organizer_name TEXT, organizer_email TEXT, status TEXT, method TEXT,
                rsvp_status TEXT DEFAULT 'needs-action', raw_ics TEXT, source TEXT NOT NULL DEFAULT 'ics',
                confidence REAL, dismissed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        conn
    }

    /// `insert_detected` is intentionally a no-op on an existing row, which is
    /// right for the sync path and is exactly why a detector *fix* cannot reach
    /// mail already stored. `refresh_detected` is the migration-only escape
    /// hatch, and these are the properties that make it safe.
    #[test]
    fn refresh_detected_corrects_a_wrong_row_without_resurrecting_a_dismissal() {
        let conn = setup();

        // What the buggy detector stored: an all-day event on the day the mail
        // arrived, instead of the real time from the body.
        let id = insert_detected(
            &conn, "acct", "INBOX", 42, "Interview", "2026-08-10", None, None,
            Some("https://meet.google.com/abc-defg-hij"), 0.7,
        )
        .unwrap();

        // Re-running insert changes nothing — this is the gap being closed.
        let same = insert_detected(
            &conn, "acct", "INBOX", 42, "Interview", "2026-08-13T16:30:00Z", None, None, None, 0.95,
        )
        .unwrap();
        assert_eq!(same, id);
        let dtstart: String = conn
            .query_row("SELECT dtstart FROM calendar_events WHERE id=?1", params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(dtstart, "2026-08-10", "insert_detected must not overwrite");

        // The user hid it. That decision has to survive a re-detection.
        dismiss_event(&conn, id).unwrap();

        assert!(refresh_detected(
            &conn, "acct", "INBOX", 42, "Interview", "2026-08-13T16:30:00Z",
            Some("2026-08-13T17:00:00Z"), None, Some("https://meet.google.com/abc-defg-hij"), 0.95,
        )
        .unwrap());

        let (row_id, dtstart, dtend, confidence, dismissed): (i64, String, Option<String>, f64, i64) = conn
            .query_row(
                "SELECT id, dtstart, dtend, confidence, dismissed FROM calendar_events WHERE message_uid=42",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(row_id, id, "the row id must be stable, not delete-and-reinsert");
        assert_eq!(dtstart, "2026-08-13T16:30:00Z");
        assert_eq!(dtend.as_deref(), Some("2026-08-13T17:00:00Z"));
        assert!((confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(dismissed, 1, "re-detecting is not permission to un-hide it");
    }

    /// A heuristic must never be able to rewrite a real ICS invite or an event
    /// the user created themselves.
    #[test]
    fn refresh_detected_cannot_touch_a_non_detected_row() {
        let conn = setup();
        conn.execute(
            "INSERT INTO calendar_events
             (account_id, folder_name, message_uid, summary, dtstart, source)
             VALUES ('acct','INBOX',7,'Real invite','2026-08-13T16:30:00Z','ics')",
            [],
        )
        .unwrap();

        assert!(!refresh_detected(
            &conn, "acct", "INBOX", 7, "Guessed", "2099-01-01T00:00:00Z", None, None, None, 0.95,
        )
        .unwrap());

        let (summary, dtstart): (String, String) = conn
            .query_row(
                "SELECT summary, dtstart FROM calendar_events WHERE message_uid=7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(summary, "Real invite");
        assert_eq!(dtstart, "2026-08-13T16:30:00Z");
    }

    /// Regression for the ISO-vs-`datetime('now')` lexicographic bug — see
    /// `scheduled::tests` for the full explanation. An event earlier *today*
    /// (UTC midnight) is in the past and must NOT linger as "upcoming"; raw
    /// string compare wrongly kept it until the UTC clock rolled past midnight.
    #[test]
    fn list_upcoming_excludes_event_earlier_today_includes_future() {
        let conn = setup();
        conn.execute(
            "INSERT INTO calendar_events (account_id, folder_name, message_uid, summary, dtstart)
             VALUES ('a', 'INBOX', 1, 'past', date('now') || 'T00:00:00.001Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_events (account_id, folder_name, message_uid, summary, dtstart)
             VALUES ('a', 'INBOX', 2, 'future', '2999-12-31T23:59:59.000Z')",
            [],
        )
        .unwrap();

        // All-account path.
        let up = list_upcoming(&conn, None, 50).unwrap();
        assert_eq!(up.len(), 1, "only the future event is upcoming");
        assert_eq!(up[0].summary.as_deref(), Some("future"));

        // Account-scoped path takes the same fix.
        let up_acct = list_upcoming(&conn, Some("a"), 50).unwrap();
        assert_eq!(up_acct.len(), 1);
        assert_eq!(up_acct[0].summary.as_deref(), Some("future"));
    }

    /// All three `dtstart` storage shapes from gotcha #21 must survive
    /// `datetime()` — a NULL return would silently drop the row.
    #[test]
    fn datetime_parses_all_three_dtstart_formats() {
        let conn = setup();
        conn.execute(
            "INSERT INTO calendar_events (account_id, folder_name, message_uid, summary, dtstart) VALUES
                ('a', 'INBOX', 1, 'z',        '2999-12-31T23:59:59.000Z'),
                ('a', 'INBOX', 2, 'floating', '2999-12-31T23:59:59'),
                ('a', 'INBOX', 3, 'dateonly', '2999-12-31')",
            [],
        )
        .unwrap();

        let up = list_upcoming(&conn, None, 50).unwrap();
        assert_eq!(
            up.len(),
            3,
            "no dtstart format may be silently dropped by datetime()"
        );
    }

    /// `list_by_date_range` receives date-only `YYYY-MM-DD` keys (CalendarView's
    /// `toDateKey`); wrapping both sides in `datetime()` keeps the half-open
    /// `[start, end)` window precise against a timed ISO `dtstart`.
    #[test]
    fn list_by_date_range_matches_date_only_keys() {
        let conn = setup();
        conn.execute(
            "INSERT INTO calendar_events (account_id, folder_name, message_uid, summary, dtstart)
             VALUES ('a', 'INBOX', 1, 'jun3', '2026-06-03T13:00:00.000Z')",
            [],
        )
        .unwrap();

        let in_range = list_by_date_range(&conn, None, "2026-06-03", "2026-06-04").unwrap();
        assert_eq!(in_range.len(), 1, "event must fall inside its own day window");

        let out_range = list_by_date_range(&conn, None, "2026-06-04", "2026-06-05").unwrap();
        assert!(out_range.is_empty(), "next day's window excludes it");
    }
}
