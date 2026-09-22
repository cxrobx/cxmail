//! The `zoom_meetings` side table: which Zoom meeting backs which Google
//! Calendar event, and what Zoom is believed to currently hold.
//!
//! Pure SQL only — every decision about *what should happen* lives in
//! `email::zoom_sync`, so it can be unit-tested without a network double. See
//! `db::schema::migrate_v52_zoom_meetings` for why this is a side table keyed on
//! the remote triple rather than a column on `gcal_events`.

use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// The local record's lifecycle. Written before the remote object exists and
/// removed after it is confirmed gone, so an orphan window is one HTTP call wide.
pub mod state {
    /// The row exists, the Zoom meeting may or may not. Never auto-deleted: this
    /// is the one unrecoverable window (Zoom accepted the create, the process
    /// died before the id was persisted) and it must stay *visible*.
    pub const CREATING: &str = "creating";
    /// A real meeting, id and join URL recorded.
    pub const ACTIVE: &str = "active";
    /// A local tombstone. Written *before* the delete call, so a crash mid-delete
    /// leaves something the reaper can retry.
    pub const DELETE_PENDING: &str = "delete_pending";
    /// The verifying GET failed too many times. Surfaced in settings and never
    /// auto-deleted — we do not know whether the event still exists.
    pub const UNVERIFIED: &str = "unverified";
    /// A past meeting whose event is gone. Deliberately not deleted: deleting a
    /// completed Zoom meeting can destroy its cloud recording and attendance
    /// report, so a technical orphan is the better of two bad outcomes.
    pub const RETIRED: &str = "retired";
}

/// How many verification failures before a row stops being reconciled.
pub const MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZoomMeetingRow {
    pub id: i64,
    pub account_id: String,
    pub gcal_calendar_id: String,
    pub gcal_event_id: String,
    pub zoom_meeting_id: Option<String>,
    pub join_url: Option<String>,
    pub state: String,
    pub pushed_topic: Option<String>,
    pub pushed_start: Option<String>,
    pub pushed_duration: Option<i64>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const COLUMNS: &str = "id, account_id, gcal_calendar_id, gcal_event_id, zoom_meeting_id,
     join_url, state, pushed_topic, pushed_start, pushed_duration, attempts,
     last_error, created_at, updated_at";

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ZoomMeetingRow> {
    Ok(ZoomMeetingRow {
        id: r.get(0)?,
        account_id: r.get(1)?,
        gcal_calendar_id: r.get(2)?,
        gcal_event_id: r.get(3)?,
        zoom_meeting_id: r.get(4)?,
        join_url: r.get(5)?,
        state: r.get(6)?,
        pushed_topic: r.get(7)?,
        pushed_start: r.get(8)?,
        pushed_duration: r.get(9)?,
        attempts: r.get(10)?,
        last_error: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    })
}

/// Claim the link BEFORE the Zoom meeting exists.
///
/// Ordering is the point: the durable local record predates the remote object,
/// so a crash between this and `record_created` leaves a visible `creating` row
/// rather than a silent orphan. Re-running for the same triple resets the row
/// (a retried create is a fresh attempt, not a second meeting).
pub fn begin_create(
    conn: &Connection,
    account_id: &str,
    gcal_calendar_id: &str,
    gcal_event_id: &str,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO zoom_meetings
            (account_id, gcal_calendar_id, gcal_event_id, state, attempts, last_error)
         VALUES (?1, ?2, ?3, 'creating', 0, NULL)
         ON CONFLICT(account_id, gcal_calendar_id, gcal_event_id) DO UPDATE SET
            state='creating', attempts=0, last_error=NULL, updated_at=datetime('now')",
        params![account_id, gcal_calendar_id, gcal_event_id],
    )?;
    conn.query_row(
        "SELECT id FROM zoom_meetings
         WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3",
        params![account_id, gcal_calendar_id, gcal_event_id],
        |r| r.get(0),
    )
    .map_err(AppError::Database)
}

/// Record a meeting Zoom has confirmed, seeding the believed-remote state.
pub fn record_created(
    conn: &Connection,
    id: i64,
    zoom_meeting_id: &str,
    join_url: &str,
    topic: &str,
    start_time: &str,
    duration: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET
            zoom_meeting_id=?1, join_url=?2, state='active',
            pushed_topic=?3, pushed_start=?4, pushed_duration=?5,
            attempts=0, last_error=NULL, updated_at=datetime('now')
         WHERE id=?6",
        params![zoom_meeting_id, join_url, topic, start_time, duration, id],
    )?;
    Ok(())
}

/// Advance the believed-remote state after a 2xx PATCH — and only after one.
///
/// A failed push must leave these columns untouched so the next reconciler tick
/// recomputes the same patch. Pinned by
/// `email::zoom_sync::tests::a_failed_push_does_not_advance_the_recorded_state`.
pub fn record_pushed(
    conn: &Connection,
    id: i64,
    topic: &str,
    start_time: &str,
    duration: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET
            pushed_topic=?1, pushed_start=?2, pushed_duration=?3,
            attempts=0, last_error=NULL, updated_at=datetime('now')
         WHERE id=?4",
        params![topic, start_time, duration, id],
    )?;
    Ok(())
}

pub fn get_by_event(
    conn: &Connection,
    account_id: &str,
    gcal_calendar_id: &str,
    gcal_event_id: &str,
) -> Result<Option<ZoomMeetingRow>, AppError> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM zoom_meetings
             WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3"
        ),
        params![account_id, gcal_calendar_id, gcal_event_id],
        map_row,
    )
    .optional()
    .map_err(AppError::Database)
}

/// Resolve the Zoom link from a local `gcal_events.id`, which is the handle the
/// app and the MCP both already hold.
pub fn get_by_local_event(
    conn: &Connection,
    gcal_event_row_id: i64,
) -> Result<Option<ZoomMeetingRow>, AppError> {
    conn.query_row(
        &format!(
            "SELECT {}
             FROM zoom_meetings z
             JOIN gcal_events g
               ON g.account_id = z.account_id
              AND g.gcal_calendar_id = z.gcal_calendar_id
              AND g.gcal_event_id = z.gcal_event_id
             WHERE g.id = ?1",
            COLUMNS
                .split(',')
                .map(|c| format!("z.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        params![gcal_event_row_id],
        map_row,
    )
    .optional()
    .map_err(AppError::Database)
}

/// Write the tombstone *before* calling Zoom, so an interrupted delete is
/// reapable rather than forgotten.
pub fn mark_delete_pending(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET state='delete_pending', updated_at=datetime('now')
         WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

/// Undo a tombstone that turned out to be premature.
///
/// The delete paths write the tombstone *before* calling Google, so a Google
/// delete that then fails (or degrades to "decline as an attendee") must put the
/// row back — otherwise the reaper deletes a meeting still attached to a live
/// event. Scoped to `delete_pending` so it can never resurrect a retired row.
pub fn clear_delete_pending(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings
         SET state = CASE WHEN zoom_meeting_id IS NULL THEN 'creating' ELSE 'active' END,
             updated_at = datetime('now')
         WHERE id=?1 AND state='delete_pending'",
        params![id],
    )?;
    Ok(())
}

pub fn record_error(conn: &Connection, id: i64, message: &str) -> Result<i64, AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET attempts=attempts+1, last_error=?1,
                updated_at=datetime('now')
         WHERE id=?2",
        params![message.chars().take(500).collect::<String>(), id],
    )?;
    conn.query_row(
        "SELECT attempts FROM zoom_meetings WHERE id=?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(AppError::Database)
}

/// Park a row whose existence could not be verified. Never auto-deleted from
/// here — absence of evidence is not evidence the event is gone.
pub fn mark_unverified(conn: &Connection, id: i64, message: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET state='unverified', last_error=?1,
                updated_at=datetime('now')
         WHERE id=?2",
        params![message.chars().take(500).collect::<String>(), id],
    )?;
    Ok(())
}

pub fn retire(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE zoom_meetings SET state='retired', updated_at=datetime('now') WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

/// Drop the local row. Only ever called once the remote object is confirmed
/// gone (or was never created).
pub fn forget(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM zoom_meetings WHERE id=?1", params![id])?;
    Ok(())
}

pub fn list_pending_deletes(conn: &Connection) -> Result<Vec<ZoomMeetingRow>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM zoom_meetings WHERE state='delete_pending' ORDER BY id"
    ))?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)?;
    Ok(rows)
}

/// Rows the reconciler should look at for one account.
pub fn list_reconcilable_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<ZoomMeetingRow>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM zoom_meetings
         WHERE account_id=?1 AND state IN ('creating','active')
         ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![account_id], map_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)?;
    Ok(rows)
}

/// Distinct accounts with at least one live link, so a reconcile pass can skip
/// accounts that have never used Zoom without touching their tokens.
pub fn accounts_with_links(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT account_id FROM zoom_meetings
         WHERE state IN ('creating','active') ORDER BY account_id",
    )?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)?;
    Ok(rows)
}

/// Links whose Google mirror row no longer exists.
///
/// This is what a column could never give us: mirror deletion produces a work
/// item instead of silence. It is NOT a delete list — a missing mirror row is
/// ambiguous (a full re-sync sweep, a cancelled-event purge, an account removed,
/// a fresh database), so the reconciler verifies against Google before acting.
pub fn list_orphans(conn: &Connection) -> Result<Vec<ZoomMeetingRow>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {}
         FROM zoom_meetings z
         LEFT JOIN gcal_events g
           ON g.account_id = z.account_id
          AND g.gcal_calendar_id = z.gcal_calendar_id
          AND g.gcal_event_id = z.gcal_event_id
         WHERE g.id IS NULL AND z.state IN ('creating','active')
         ORDER BY z.id",
        COLUMNS
            .split(',')
            .map(|c| format!("z.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    ))?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)?;
    Ok(rows)
}

/// Counts for the settings panel, so a stuck row is visible without a log dive.
pub fn health_counts(conn: &Connection) -> Result<(i64, i64), AppError> {
    let orphans = list_orphans(conn)?.len() as i64;
    let unverified: i64 = conn.query_row(
        "SELECT COUNT(*) FROM zoom_meetings WHERE state IN ('unverified','creating')",
        [],
        |r| r.get(0),
    )?;
    Ok((orphans, unverified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cxmail_core::mail::gcal_dto::{EventDateTime, GcalCalendar, GcalEvent};

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
             INSERT INTO accounts(id) VALUES ('acct');",
        )
        .unwrap();
        crate::db::schema::migrate_v43_gcal(&conn).unwrap();
        crate::db::schema::migrate_v45_invite_approval_snapshot(&conn).unwrap();
        crate::db::schema::migrate_v52_zoom_meetings(&conn).unwrap();
        conn
    }

    fn remote_event(id: &str, start: &str, summary: &str) -> GcalEvent {
        GcalEvent {
            id: Some(id.to_string()),
            summary: Some(summary.to_string()),
            etag: Some(format!("etag-{summary}")),
            start: EventDateTime {
                date_time: Some(start.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn seed_calendar(conn: &Connection) -> i64 {
        crate::db::gcal::upsert_calendar(
            conn,
            "acct",
            &GcalCalendar {
                id: "primary@example.com".to_string(),
                primary: true,
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// **The central regression test.** A Google sync knows nothing about Zoom,
    /// and `upsert_remote_event` rewrites `gcal_events` wholesale from the remote
    /// payload. If the link ever moves back onto that table as a column, this
    /// fails.
    #[test]
    fn a_google_sync_cannot_null_the_zoom_link() {
        let conn = setup();
        let calendar_row = seed_calendar(&conn);
        crate::db::gcal::upsert_remote_event(
            &conn,
            calendar_row,
            "acct",
            "primary@example.com",
            &remote_event("evt-1", "2026-08-12T11:00:00-04:00", "Design review"),
            "2026-08-10T12:00:00Z",
        )
        .unwrap();

        let link = begin_create(&conn, "acct", "primary@example.com", "evt-1").unwrap();
        record_created(
            &conn,
            link,
            "86903742305",
            "https://us02web.zoom.us/j/86903742305",
            "Design review",
            "2026-08-12T15:00:00Z",
            45,
        )
        .unwrap();

        // A fresh remote payload for the SAME triple — a retitled, retimed event
        // and a new etag, exactly what a real sync tick delivers.
        crate::db::gcal::upsert_remote_event(
            &conn,
            calendar_row,
            "acct",
            "primary@example.com",
            &remote_event("evt-1", "2026-08-12T12:30:00-04:00", "Design review v2"),
            "2026-08-10T12:05:00Z",
        )
        .unwrap();

        let row = get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .expect("the Zoom link must survive a Google sync");
        assert_eq!(row.zoom_meeting_id.as_deref(), Some("86903742305"));
        assert_eq!(
            row.join_url.as_deref(),
            Some("https://us02web.zoom.us/j/86903742305")
        );
        assert_eq!(row.state, state::ACTIVE);
        // The believed-remote state is also untouched — the sync changed Google,
        // not Zoom, so the reconciler must still see a pending change.
        assert_eq!(row.pushed_start.as_deref(), Some("2026-08-12T15:00:00Z"));
        assert_eq!(row.pushed_topic.as_deref(), Some("Design review"));
    }

    /// Every way a mirror row can vanish must leave a reapable tombstone. With a
    /// column the id would die with the row and the meeting would be orphaned
    /// invisibly.
    #[test]
    fn deleting_the_mirror_leaves_a_reapable_tombstone() {
        #[allow(clippy::type_complexity)]
        let deleters: Vec<(&str, Box<dyn Fn(&Connection, i64, i64)>)> = vec![
            (
                "sweep_full_sync",
                Box::new(|conn: &Connection, calendar_row: i64, _event_row: i64| {
                    // A full re-sync deletes anything not seen in this run — the
                    // routine consequence of an expired sync token.
                    crate::db::gcal::sweep_full_sync(conn, calendar_row, "2999-01-01T00:00:00Z")
                        .unwrap();
                }),
            ),
            (
                "purge_old_cancelled",
                Box::new(|conn: &Connection, _calendar_row: i64, event_row: i64| {
                    conn.execute(
                        "UPDATE gcal_events SET status='cancelled',
                                dtstart=datetime('now','-30 days') WHERE id=?1",
                        params![event_row],
                    )
                    .unwrap();
                    crate::db::gcal::purge_old_cancelled(conn).unwrap();
                }),
            ),
            (
                "delete_local",
                Box::new(|conn: &Connection, _calendar_row: i64, event_row: i64| {
                    crate::db::gcal::delete_local(conn, event_row).unwrap();
                }),
            ),
            (
                "accounts CASCADE",
                Box::new(|conn: &Connection, _calendar_row: i64, _event_row: i64| {
                    conn.execute("DELETE FROM accounts WHERE id='acct'", []).unwrap();
                }),
            ),
        ];

        for (name, delete) in deleters {
            let conn = setup();
            let calendar_row = seed_calendar(&conn);
            let event_row = crate::db::gcal::upsert_remote_event(
                &conn,
                calendar_row,
                "acct",
                "primary@example.com",
                &remote_event("evt-1", "2026-08-12T11:00:00-04:00", "Design review"),
                "2026-08-10T12:00:00Z",
            )
            .unwrap();
            let link = begin_create(&conn, "acct", "primary@example.com", "evt-1").unwrap();
            record_created(
                &conn,
                link,
                "86903742305",
                "https://us02web.zoom.us/j/86903742305",
                "Design review",
                "2026-08-12T15:00:00Z",
                45,
            )
            .unwrap();
            assert!(
                list_orphans(&conn).unwrap().is_empty(),
                "{name}: not an orphan while the mirror row exists"
            );

            delete(&conn, calendar_row, event_row);

            let orphans = list_orphans(&conn).unwrap();
            assert_eq!(orphans.len(), 1, "{name}: expected exactly one tombstone");
            assert_eq!(
                orphans[0].zoom_meeting_id.as_deref(),
                Some("86903742305"),
                "{name}: the tombstone must still name the meeting"
            );
            let (orphan_count, _) = health_counts(&conn).unwrap();
            assert_eq!(orphan_count, 1, "{name}: surfaced in the settings counts");
        }
    }

    #[test]
    fn the_lifecycle_transitions_round_trip() {
        let conn = setup();
        let calendar_row = seed_calendar(&conn);
        let event_row = crate::db::gcal::upsert_remote_event(
            &conn,
            calendar_row,
            "acct",
            "primary@example.com",
            &remote_event("evt-1", "2026-08-12T11:00:00-04:00", "Design review"),
            "2026-08-10T12:00:00Z",
        )
        .unwrap();

        let link = begin_create(&conn, "acct", "primary@example.com", "evt-1").unwrap();
        let row = get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .unwrap();
        assert_eq!(row.state, state::CREATING);
        assert!(row.zoom_meeting_id.is_none());

        record_created(
            &conn,
            link,
            "111",
            "https://us02web.zoom.us/j/111",
            "Design review",
            "2026-08-12T15:00:00Z",
            45,
        )
        .unwrap();

        // Resolvable from the local mirror id, which is the handle callers hold.
        let by_local = get_by_local_event(&conn, event_row).unwrap().unwrap();
        assert_eq!(by_local.id, link);
        assert_eq!(by_local.zoom_meeting_id.as_deref(), Some("111"));

        record_pushed(&conn, link, "Renamed", "2026-08-12T15:00:00Z", 45).unwrap();
        let row = get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .unwrap();
        assert_eq!(row.pushed_topic.as_deref(), Some("Renamed"));

        assert_eq!(record_error(&conn, link, "boom").unwrap(), 1);
        assert_eq!(record_error(&conn, link, "boom").unwrap(), 2);
        let row = get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .unwrap();
        assert_eq!(row.last_error.as_deref(), Some("boom"));

        mark_delete_pending(&conn, link).unwrap();
        assert_eq!(list_pending_deletes(&conn).unwrap().len(), 1);
        assert!(list_reconcilable_for_account(&conn, "acct").unwrap().is_empty());

        forget(&conn, link).unwrap();
        assert!(get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_retried_create_reuses_the_row_rather_than_adding_a_second() {
        let conn = setup();
        let first = begin_create(&conn, "acct", "primary@example.com", "evt-1").unwrap();
        record_error(&conn, first, "zoom refused").unwrap();
        let second = begin_create(&conn, "acct", "primary@example.com", "evt-1").unwrap();
        assert_eq!(first, second);
        let row = get_by_event(&conn, "acct", "primary@example.com", "evt-1")
            .unwrap()
            .unwrap();
        assert_eq!(row.attempts, 0, "a retry is a fresh attempt");
        assert!(row.last_error.is_none());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM zoom_meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn retired_and_unverified_rows_leave_the_reconcile_and_delete_lanes() {
        let conn = setup();
        let a = begin_create(&conn, "acct", "primary@example.com", "evt-a").unwrap();
        let b = begin_create(&conn, "acct", "primary@example.com", "evt-b").unwrap();
        record_created(&conn, a, "1", "https://z/1", "A", "2026-01-01T00:00:00Z", 30).unwrap();
        record_created(&conn, b, "2", "https://z/2", "B", "2026-01-01T00:00:00Z", 30).unwrap();

        retire(&conn, a).unwrap();
        mark_unverified(&conn, b, "could not verify").unwrap();

        assert!(list_reconcilable_for_account(&conn, "acct").unwrap().is_empty());
        assert!(list_pending_deletes(&conn).unwrap().is_empty());
        // A retired row is intentionally NOT surfaced as a problem; an
        // unverified one is.
        let (orphans, unverified) = health_counts(&conn).unwrap();
        assert_eq!(orphans, 0, "neither state is reconcilable, so neither is an orphan");
        assert_eq!(unverified, 1);
        assert!(accounts_with_links(&conn).unwrap().is_empty());
    }
}
