use cxmail_core::mail::gcal_dto::{GcalCalendar, GcalEvent};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
pub struct GcalCalendarRow {
    pub id: i64,
    pub account_id: String,
    pub gcal_calendar_id: String,
    pub summary: Option<String>,
    pub time_zone: Option<String>,
    pub access_role: Option<String>,
    pub bg_color: Option<String>,
    pub is_primary: bool,
    pub selected: bool,
    pub sync_token: Option<String>,
    pub sync_window_days: i32,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcalEventRow {
    pub id: i64,
    pub calendar_row_id: i64,
    pub account_id: String,
    pub gcal_calendar_id: String,
    pub gcal_event_id: String,
    pub ical_uid: Option<String>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<String>,
    pub etag: Option<String>,
    pub sequence: i64,
    pub updated: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub is_all_day: bool,
    pub start_tz: Option<String>,
    pub end_tz: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub organizer_self: bool,
    pub attendees_json: Option<String>,
    pub self_response_status: Option<String>,
    pub status: String,
    pub transparency: Option<String>,
    pub html_link: Option<String>,
    pub hangout_link: Option<String>,
    pub conference_json: Option<String>,
    pub sync_state: String,
    pub pending_notify: bool,
    pub push_attempts: i64,
    pub local_updated_at: Option<String>,
    pub raw_json: Option<String>,
    pub remote_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedCalendarEvent {
    pub id: i64,
    pub kind: String,
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
    pub gcal_calendar_id: Option<String>,
    pub gcal_event_id: Option<String>,
    pub recurring_event_id: Option<String>,
    pub attendees_json: Option<String>,
    pub html_link: Option<String>,
    pub hangout_link: Option<String>,
    pub is_all_day: bool,
    pub start_tz: Option<String>,
    pub sync_state: Option<String>,
    pub pending_notify: bool,
    /// Per-attendee delivery state, resolved by
    /// `db::invite_notifications::derive`. Computed in Rust rather than in the
    /// component so the four-state rule has exactly one implementation
    /// (gotcha #36's one-matcher rule) — the UI renders this and decides nothing.
    pub attendee_delivery: Vec<crate::db::invite_notifications::AttendeeDelivery>,
    /// Whether deleting this event would actually email anyone, per
    /// `invite_notifications::cancellation_should_notify`. Carried on the event
    /// so the confirmation gate can name real consequences without the frontend
    /// re-deriving the rule — a dialog that says "this will email 4 guests" when
    /// no mail is sent is its own kind of lie.
    pub cancellation_notifies: bool,
}

pub fn upsert_calendar(
    conn: &Connection,
    account_id: &str,
    calendar: &GcalCalendar,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO gcal_calendars
         (account_id, gcal_calendar_id, summary, time_zone, access_role, bg_color, is_primary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(account_id, gcal_calendar_id) DO UPDATE SET
            summary=excluded.summary, time_zone=excluded.time_zone,
            access_role=excluded.access_role, bg_color=excluded.bg_color,
            is_primary=excluded.is_primary",
        params![
            account_id,
            calendar.id,
            calendar.summary,
            calendar.time_zone,
            calendar.access_role,
            calendar.background_color,
            calendar.primary as i32,
        ],
    )?;
    conn.query_row(
        "SELECT id FROM gcal_calendars WHERE account_id=?1 AND gcal_calendar_id=?2",
        params![account_id, calendar.id],
        |r| r.get(0),
    )
    .map_err(AppError::Database)
}

pub fn ensure_primary_calendar(conn: &Connection, account_id: &str) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO gcal_calendars
         (account_id, gcal_calendar_id, summary, is_primary)
         VALUES (?1, 'primary', 'Primary', 1)
         ON CONFLICT(account_id, gcal_calendar_id) DO NOTHING",
        params![account_id],
    )?;
    conn.query_row(
        "SELECT id FROM gcal_calendars WHERE account_id=?1 AND gcal_calendar_id='primary'",
        params![account_id],
        |r| r.get(0),
    )
    .map_err(AppError::Database)
}

pub fn list_calendars(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<GcalCalendarRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, gcal_calendar_id, summary, time_zone, access_role,
                bg_color, is_primary, selected, sync_token, sync_window_days,
                last_synced_at, last_error
         FROM gcal_calendars WHERE account_id=?1 ORDER BY is_primary DESC, summary",
    )?;
    let rows = stmt
        .query_map(params![account_id], |r| {
            Ok(GcalCalendarRow {
                id: r.get(0)?,
                account_id: r.get(1)?,
                gcal_calendar_id: r.get(2)?,
                summary: r.get(3)?,
                time_zone: r.get(4)?,
                access_role: r.get(5)?,
                bg_color: r.get(6)?,
                is_primary: r.get::<_, i32>(7)? != 0,
                selected: r.get::<_, i32>(8)? != 0,
                sync_token: r.get(9)?,
                sync_window_days: r.get(10)?,
                last_synced_at: r.get(11)?,
                last_error: r.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_sync_token(
    conn: &Connection,
    calendar_row_id: i64,
    token: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_calendars SET sync_token=?1, last_synced_at=datetime('now'),
                last_error=NULL, last_error_at=NULL WHERE id=?2",
        params![token, calendar_row_id],
    )?;
    Ok(())
}

pub fn set_sync_error(
    conn: &Connection,
    calendar_row_id: i64,
    error: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_calendars SET last_error=?1, last_error_at=datetime('now') WHERE id=?2",
        params![error, calendar_row_id],
    )?;
    Ok(())
}

pub fn normalize_event_times(
    event: &GcalEvent,
) -> Result<(String, Option<String>, bool, Option<String>, Option<String>), AppError> {
    fn normalize(value: &cxmail_core::mail::gcal_dto::EventDateTime) -> Result<(String, bool), AppError> {
        if let Some(date) = &value.date {
            return Ok((date.clone(), true));
        }
        let raw = value
            .date_time
            .as_deref()
            .ok_or_else(|| AppError::Parse("Google event has no start/end value".to_string()))?;
        let parsed = DateTime::parse_from_rfc3339(raw)
            .map_err(|e| AppError::Parse(format!("Invalid Google event datetime: {e}")))?;
        Ok((
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            false,
        ))
    }

    let (start, all_day) = normalize(&event.start)?;
    let end = event.end.as_ref().map(normalize).transpose()?.map(|v| v.0);
    Ok((
        start,
        end,
        all_day,
        event.start.time_zone.clone(),
        event.end.as_ref().and_then(|v| v.time_zone.clone()),
    ))
}

pub fn upsert_remote_event(
    conn: &Connection,
    calendar_row_id: i64,
    account_id: &str,
    calendar_id: &str,
    event: &GcalEvent,
    run_started_at: &str,
) -> Result<i64, AppError> {
    let event_id = event
        .id
        .as_deref()
        .ok_or_else(|| AppError::Parse("Google event response omitted id".to_string()))?;
    let raw = serde_json::to_string(event)
        .map_err(|e| AppError::Parse(format!("Failed to serialize Google event: {e}")))?;
    let existing: Option<(i64, String, Option<String>)> = conn
        .query_row(
            "SELECT id, sync_state, local_updated_at FROM gcal_events
             WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3",
            params![account_id, calendar_id, event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    if let Some((id, sync_state, local_updated_at)) = existing {
        if sync_state != "synced" {
            let remote_is_newer = match (event.updated.as_deref(), local_updated_at.as_deref()) {
                (Some(remote), Some(local)) => remote > local,
                (Some(_), None) => true,
                _ => false,
            };
            if remote_is_newer {
                conn.execute(
                    "UPDATE gcal_events SET sync_state='conflict', remote_json=?1,
                            last_seen_at=?2 WHERE id=?3",
                    params![raw, run_started_at, id],
                )?;
            } else {
                conn.execute(
                    "UPDATE gcal_events SET last_seen_at=?1 WHERE id=?2",
                    params![run_started_at, id],
                )?;
            }
            return Ok(id);
        }
    }

    let (dtstart, dtend, is_all_day, start_tz, end_tz) = normalize_event_times(event)?;
    let attendees_json = event
        .attendees
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Parse(format!("Failed to serialize attendees: {e}")))?;
    let conference_json = event
        .conference_data
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Parse(format!("Failed to serialize conference: {e}")))?;
    let self_response = event.attendees.as_ref().and_then(|items| {
        items
            .iter()
            .find(|a| a.self_attendee)
            .and_then(|a| a.response_status.clone())
    });

    conn.execute(
        "INSERT INTO gcal_events (
            calendar_row_id, account_id, gcal_calendar_id, gcal_event_id, ical_uid,
            recurring_event_id, original_start_time, etag, sequence, updated, summary,
            description, location, dtstart, dtend, is_all_day, start_tz, end_tz,
            organizer_name, organizer_email, organizer_self, attendees_json,
            self_response_status, status, transparency, html_link, hangout_link,
            conference_json, sync_state, pending_notify, push_attempts, last_seen_at,
            raw_json, remote_json
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
            ?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,'synced',
            COALESCE((SELECT pending_notify FROM gcal_events
              WHERE account_id=?2 AND gcal_calendar_id=?3 AND gcal_event_id=?4),0),
            0,?29,?30,NULL
         )
         ON CONFLICT(account_id, gcal_calendar_id, gcal_event_id) DO UPDATE SET
            calendar_row_id=excluded.calendar_row_id, ical_uid=excluded.ical_uid,
            recurring_event_id=excluded.recurring_event_id,
            original_start_time=excluded.original_start_time, etag=excluded.etag,
            sequence=excluded.sequence, updated=excluded.updated, summary=excluded.summary,
            description=excluded.description, location=excluded.location,
            dtstart=excluded.dtstart, dtend=excluded.dtend,
            is_all_day=excluded.is_all_day, start_tz=excluded.start_tz, end_tz=excluded.end_tz,
            organizer_name=excluded.organizer_name, organizer_email=excluded.organizer_email,
            organizer_self=excluded.organizer_self, attendees_json=excluded.attendees_json,
            self_response_status=excluded.self_response_status, status=excluded.status,
            transparency=excluded.transparency, html_link=excluded.html_link,
            hangout_link=excluded.hangout_link, conference_json=excluded.conference_json,
            sync_state='synced', push_attempts=0, last_seen_at=excluded.last_seen_at,
            raw_json=excluded.raw_json, remote_json=NULL",
        params![
            calendar_row_id,
            account_id,
            calendar_id,
            event_id,
            event.i_cal_uid,
            event.recurring_event_id,
            event
                .original_start_time
                .as_ref()
                .and_then(|v| v.date_time.as_ref().or(v.date.as_ref())),
            event.etag,
            event.sequence.unwrap_or(0),
            event.updated,
            event.summary,
            event.description,
            event.location,
            dtstart,
            dtend,
            is_all_day as i32,
            start_tz,
            end_tz,
            event
                .organizer
                .as_ref()
                .and_then(|o| o.display_name.as_ref()),
            event.organizer.as_ref().and_then(|o| o.email.as_ref()),
            event.organizer.as_ref().is_some_and(|o| o.self_attendee) as i32,
            attendees_json,
            self_response,
            event.status.as_deref().unwrap_or("confirmed"),
            event.transparency,
            event.html_link,
            event.hangout_link,
            conference_json,
            run_started_at,
            raw,
        ],
    )?;
    conn.query_row(
        "SELECT id FROM gcal_events WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3",
        params![account_id, calendar_id, event_id],
        |r| r.get(0),
    )
    .map_err(AppError::Database)
}

pub fn get_event(conn: &Connection, id: i64) -> Result<Option<GcalEventRow>, AppError> {
    conn.query_row(
        "SELECT id, calendar_row_id, account_id, gcal_calendar_id, gcal_event_id,
                ical_uid, recurring_event_id, original_start_time, etag, sequence,
                updated, summary, description, location, dtstart, dtend, is_all_day,
                start_tz, end_tz, organizer_name, organizer_email, organizer_self,
                attendees_json, self_response_status, status, transparency, html_link,
                hangout_link, conference_json, sync_state, pending_notify, push_attempts,
                local_updated_at, raw_json, remote_json
         FROM gcal_events WHERE id=?1",
        params![id],
        map_gcal_row,
    )
    .optional()
    .map_err(AppError::Database)
}

/// Look up a mirror row by its **remote** address rather than its local id.
///
/// This is the key `zoom_meetings` is keyed on, and the reason is that
/// `gcal_events.id` is AUTOINCREMENT and never reused: a sweep-then-repull
/// (routine — an expired sync token forces a full re-sync) gives the same event a
/// brand-new local id, so anything holding the old one is silently orphaned. The
/// triple survives that, and is exactly what `upsert_remote_event`'s
/// `ON CONFLICT` clause matches on.
pub fn get_by_remote_id(
    conn: &Connection,
    account_id: &str,
    calendar_id: &str,
    event_id: &str,
) -> Result<Option<GcalEventRow>, AppError> {
    conn.query_row(
        "SELECT id, calendar_row_id, account_id, gcal_calendar_id, gcal_event_id,
                ical_uid, recurring_event_id, original_start_time, etag, sequence,
                updated, summary, description, location, dtstart, dtend, is_all_day,
                start_tz, end_tz, organizer_name, organizer_email, organizer_self,
                attendees_json, self_response_status, status, transparency, html_link,
                hangout_link, conference_json, sync_state, pending_notify, push_attempts,
                local_updated_at, raw_json, remote_json
         FROM gcal_events
         WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3",
        params![account_id, calendar_id, event_id],
        map_gcal_row,
    )
    .optional()
    .map_err(AppError::Database)
}

fn map_gcal_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GcalEventRow> {
    Ok(GcalEventRow {
        id: r.get(0)?,
        calendar_row_id: r.get(1)?,
        account_id: r.get(2)?,
        gcal_calendar_id: r.get(3)?,
        gcal_event_id: r.get(4)?,
        ical_uid: r.get(5)?,
        recurring_event_id: r.get(6)?,
        original_start_time: r.get(7)?,
        etag: r.get(8)?,
        sequence: r.get(9)?,
        updated: r.get(10)?,
        summary: r.get(11)?,
        description: r.get(12)?,
        location: r.get(13)?,
        dtstart: r.get(14)?,
        dtend: r.get(15)?,
        is_all_day: r.get::<_, i32>(16)? != 0,
        start_tz: r.get(17)?,
        end_tz: r.get(18)?,
        organizer_name: r.get(19)?,
        organizer_email: r.get(20)?,
        organizer_self: r.get::<_, i32>(21)? != 0,
        attendees_json: r.get(22)?,
        self_response_status: r.get(23)?,
        status: r.get(24)?,
        transparency: r.get(25)?,
        html_link: r.get(26)?,
        hangout_link: r.get(27)?,
        conference_json: r.get(28)?,
        sync_state: r.get(29)?,
        pending_notify: r.get::<_, i32>(30)? != 0,
        push_attempts: r.get(31)?,
        local_updated_at: r.get(32)?,
        raw_json: r.get(33)?,
        remote_json: r.get(34)?,
    })
}

pub fn sweep_full_sync(
    conn: &Connection,
    calendar_row_id: i64,
    run_started_at: &str,
) -> Result<usize, AppError> {
    conn.execute(
        "DELETE FROM gcal_events
         WHERE calendar_row_id=?1 AND sync_state='synced'
           AND (last_seen_at IS NULL OR last_seen_at < ?2)",
        params![calendar_row_id, run_started_at],
    )
    .map_err(AppError::Database)
}

pub fn purge_old_cancelled(conn: &Connection) -> Result<usize, AppError> {
    conn.execute(
        "DELETE FROM gcal_events
         WHERE status='cancelled'
           AND recurring_event_id IS NULL
           AND datetime(dtstart) < datetime('now','-7 days')",
        [],
    )
    .map_err(AppError::Database)
}

pub fn set_pending_notify(conn: &Connection, id: i64, pending: bool) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_events SET pending_notify=?1 WHERE id=?2",
        params![pending as i32, id],
    )?;
    Ok(())
}

/// The recipient set an approval was granted for, and the event it belongs to.
///
/// Returned by [`approved_invite`] so the app can deliver an invitation the MCP
/// only requested. `recipients` is the normalized (trimmed, lowercased, sorted)
/// snapshot captured at request time — compare the live attendee list against it
/// before notifying anyone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedInvite {
    pub event_id: i64,
    pub recipients: Vec<String>,
}

pub fn request_invite_approval(
    conn: &Connection,
    id: i64,
    approval_id: &str,
    recipients: &[String],
) -> Result<(), AppError> {
    let recipients_json = serde_json::to_string(recipients)
        .map_err(|e| AppError::General(format!("Failed to encode approved recipients: {e}")))?;
    let changed = conn.execute(
        "UPDATE gcal_events
         SET invite_approval_id=?1, invite_approval_status='pending',
             invite_approval_recipients=?2, invite_approval_requested_at=datetime('now')
         WHERE id=?3 AND pending_notify=1",
        params![approval_id, recipients_json, id],
    )?;
    if changed == 0 {
        return Err(AppError::General(
            "The event is not awaiting attendee notification".to_string(),
        ));
    }
    Ok(())
}

/// Look up the event and approved recipient snapshot behind an approval id,
/// but only once the decision is actually `approved`.
///
/// The approval id is the only thing the UI hands back, and resolving it here —
/// rather than trusting an event id supplied by the frontend — keeps invariant
/// #6 intact: a decision can only ever deliver the event it was raised for.
pub fn approved_invite(
    conn: &Connection,
    approval_id: &str,
) -> Result<Option<ApprovedInvite>, AppError> {
    let row = conn
        .query_row(
            "SELECT id, invite_approval_recipients FROM gcal_events
             WHERE invite_approval_id=?1 AND invite_approval_status='approved'",
            params![approval_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(AppError::Database)?;

    let Some((event_id, recipients_json)) = row else {
        return Ok(None);
    };
    // A NULL snapshot means the approval predates v45. Treat it as an empty
    // set so the drift check in the caller fails closed rather than notifying
    // an unreviewed recipient list.
    let recipients = match recipients_json.as_deref() {
        Some(json) => serde_json::from_str::<Vec<String>>(json).unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(Some(ApprovedInvite {
        event_id,
        recipients,
    }))
}

/// Clear approvals left pending longer than `max_age_hours`.
///
/// Nothing auto-denies any more, so an approval the user never answers would
/// otherwise sit `pending` forever and block [`request_invite_approval`] from
/// ever raising another one for that event. Returns the number cleared.
pub fn sweep_stale_invite_approvals(
    conn: &Connection,
    max_age_hours: i64,
) -> Result<usize, AppError> {
    conn.execute(
        "UPDATE gcal_events
         SET invite_approval_id=NULL, invite_approval_status=NULL,
             invite_approval_recipients=NULL, invite_approval_requested_at=NULL
         WHERE invite_approval_status='pending'
           AND invite_approval_requested_at IS NOT NULL
           AND datetime(invite_approval_requested_at) < datetime('now', ?1)",
        params![format!("-{max_age_hours} hours")],
    )
    .map_err(AppError::Database)
}

pub fn respond_to_invite_approval(
    conn: &Connection,
    approval_id: &str,
    approved: bool,
) -> Result<bool, AppError> {
    let changed = conn.execute(
        "UPDATE gcal_events SET invite_approval_status=?1
         WHERE invite_approval_id=?2 AND invite_approval_status='pending'",
        params![if approved { "approved" } else { "denied" }, approval_id],
    )?;
    Ok(changed > 0)
}

pub fn invite_approval_status(
    conn: &Connection,
    id: i64,
    approval_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT invite_approval_status FROM gcal_events
         WHERE id=?1 AND invite_approval_id=?2",
        params![id, approval_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(AppError::Database)
}

pub fn clear_invite_approval(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_events
         SET invite_approval_id=NULL, invite_approval_status=NULL,
             invite_approval_recipients=NULL, invite_approval_requested_at=NULL
         WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

pub fn list_dirty_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<GcalEventRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, calendar_row_id, account_id, gcal_calendar_id, gcal_event_id,
                ical_uid, recurring_event_id, original_start_time, etag, sequence,
                updated, summary, description, location, dtstart, dtend, is_all_day,
                start_tz, end_tz, organizer_name, organizer_email, organizer_self,
                attendees_json, self_response_status, status, transparency, html_link,
                hangout_link, conference_json, sync_state, pending_notify, push_attempts,
                local_updated_at, raw_json, remote_json
         FROM gcal_events
         WHERE account_id=?1 AND sync_state IN ('local_new','local_dirty','local_deleted')
           AND push_attempts <= 5
         ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![account_id], map_gcal_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)?;
    Ok(rows)
}

pub fn prepare_push_success(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_events SET sync_state='synced', remote_json=NULL WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

pub fn record_push_error(
    conn: &Connection,
    id: i64,
    conflict_json: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_events SET
            push_attempts=push_attempts+1,
            sync_state=CASE WHEN push_attempts+1 > 5 OR ?1 IS NOT NULL
                       THEN 'conflict' ELSE sync_state END,
            remote_json=COALESCE(?1, remote_json)
         WHERE id=?2",
        params![conflict_json, id],
    )?;
    Ok(())
}

pub fn delete_local(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM gcal_events WHERE id=?1", params![id])?;
    Ok(())
}

pub fn resolve_conflict_use_theirs(conn: &Connection, id: i64) -> Result<(), AppError> {
    let row = get_event(conn, id)?
        .ok_or_else(|| AppError::NotFound("Google Calendar event not found".to_string()))?;
    let remote = row
        .remote_json
        .ok_or_else(|| AppError::General("No remote conflict state exists".to_string()))?;
    let event: GcalEvent = serde_json::from_str(&remote)
        .map_err(|e| AppError::Parse(format!("Invalid remote event state: {e}")))?;
    upsert_remote_event(
        conn,
        row.calendar_row_id,
        &row.account_id,
        &row.gcal_calendar_id,
        &event,
        &Utc::now().to_rfc3339(),
    )?;
    Ok(())
}

pub fn mark_keep_mine(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE gcal_events SET sync_state='local_dirty', remote_json=NULL,
                push_attempts=0, local_updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

pub fn list_unified_by_range(
    conn: &Connection,
    account_id: Option<&str>,
    range_start: &str,
    range_end: &str,
) -> Result<Vec<UnifiedCalendarEvent>, AppError> {
    let mut gcal_stmt = conn.prepare(
        "SELECT id, account_id, gcal_calendar_id, gcal_event_id, ical_uid, summary,
                description, location, dtstart, dtend, organizer_name, organizer_email,
                status, self_response_status, attendees_json, html_link, hangout_link,
                is_all_day, start_tz, sync_state, pending_notify, created_at,
                recurring_event_id
         FROM gcal_events
         WHERE (?1 IS NULL OR account_id=?1)
           AND status != 'cancelled'
           AND sync_state != 'local_deleted'
           AND datetime(dtstart) >= datetime(?2)
           AND datetime(dtstart) < datetime(?3)
         ORDER BY datetime(dtstart)",
    )?;
    let gcal = gcal_stmt
        .query_map(params![account_id, range_start, range_end], |r| {
            Ok(UnifiedCalendarEvent {
                id: r.get(0)?,
                kind: "gcal".to_string(),
                account_id: r.get(1)?,
                folder_name: String::new(),
                message_uid: 0,
                event_uid: r.get(4)?,
                summary: r.get(5)?,
                description: r.get(6)?,
                location: r.get(7)?,
                dtstart: r.get(8)?,
                dtend: r.get(9)?,
                organizer_name: r.get(10)?,
                organizer_email: r.get(11)?,
                status: r.get(12)?,
                method: None,
                rsvp_status: r
                    .get::<_, Option<String>>(13)?
                    .unwrap_or_else(|| "needs-action".to_string()),
                raw_ics: None,
                source: "gcal".to_string(),
                confidence: None,
                dismissed: false,
                created_at: r.get(21)?,
                gcal_calendar_id: r.get(2)?,
                gcal_event_id: r.get(3)?,
                recurring_event_id: r.get(22)?,
                attendees_json: r.get(14)?,
                html_link: r.get(15)?,
                hangout_link: r.get(16)?,
                is_all_day: r.get::<_, i32>(17)? != 0,
                start_tz: r.get(18)?,
                sync_state: r.get(19)?,
                pending_notify: r.get::<_, i32>(20)? != 0,
                // Filled in below from one bulk ledger read rather than a query
                // per event: this renders a month at a time, and a per-row
                // lookup is a query storm under the DB mutex (gotcha #11).
                attendee_delivery: Vec::new(),
                cancellation_notifies: false,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let ledger = crate::db::invite_notifications::notified_by_event(conn, account_id)?;
    let mut gcal = gcal;
    for event in &mut gcal {
        let (Some(calendar_id), Some(event_id)) =
            (event.gcal_calendar_id.as_ref(), event.gcal_event_id.as_ref())
        else {
            continue;
        };
        let notified = ledger.get(&(
            event.account_id.clone(),
            calendar_id.clone(),
            event_id.clone(),
        ));
        event.attendee_delivery = crate::db::invite_notifications::derive(
            event.attendees_json.as_deref(),
            notified,
            event.pending_notify,
        );
        event.cancellation_notifies =
            crate::db::invite_notifications::cancellation_should_notify(&event.attendee_delivery);
    }

    let gcal_uids: HashSet<String> = gcal.iter().filter_map(|e| e.event_uid.clone()).collect();
    let mut mail_stmt = conn.prepare(
        "SELECT id, account_id, folder_name, message_uid, event_uid, summary,
                description, location, dtstart, dtend, organizer_name, organizer_email,
                status, method, rsvp_status, raw_ics, source, confidence, dismissed, created_at
         FROM calendar_events
         WHERE (?1 IS NULL OR account_id=?1) AND dismissed=0
           AND datetime(dtstart) >= datetime(?2)
           AND datetime(dtstart) < datetime(?3)
         ORDER BY datetime(dtstart)",
    )?;
    let mail = mail_stmt
        .query_map(params![account_id, range_start, range_end], |r| {
            Ok(UnifiedCalendarEvent {
                id: r.get(0)?,
                kind: "mail".to_string(),
                account_id: r.get(1)?,
                folder_name: r.get(2)?,
                message_uid: r.get(3)?,
                event_uid: r.get(4)?,
                summary: r.get(5)?,
                description: r.get(6)?,
                location: r.get(7)?,
                dtstart: r.get(8)?,
                dtend: r.get(9)?,
                organizer_name: r.get(10)?,
                organizer_email: r.get(11)?,
                status: r.get(12)?,
                method: r.get(13)?,
                rsvp_status: r.get(14)?,
                raw_ics: r.get(15)?,
                source: r.get(16)?,
                confidence: r.get(17)?,
                dismissed: r.get::<_, i32>(18)? != 0,
                created_at: r.get(19)?,
                gcal_calendar_id: None,
                gcal_event_id: None,
                recurring_event_id: None,
                attendees_json: None,
                html_link: None,
                hangout_link: None,
                is_all_day: false,
                start_tz: None,
                sync_state: None,
                pending_notify: false,
                // Mail-sourced events carry no Google guest list, so there is
                // nothing whose delivery we could have a record of.
                attendee_delivery: Vec::new(),
                cancellation_notifies: false,
            })
        })?
        .filter_map(|row| match row {
            Ok(event)
                if event
                    .event_uid
                    .as_ref()
                    .is_some_and(|uid| gcal_uids.contains(uid)) =>
            {
                None
            }
            other => Some(other),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = gcal;
    result.extend(mail);
    result.sort_by(|a, b| a.dtstart.cmp(&b.dtstart));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
             CREATE TABLE calendar_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, message_uid INTEGER NOT NULL, event_uid TEXT,
                summary TEXT, description TEXT, location TEXT, dtstart TEXT NOT NULL, dtend TEXT,
                organizer_name TEXT, organizer_email TEXT, status TEXT, method TEXT,
                rsvp_status TEXT DEFAULT 'needs-action', raw_ics TEXT,
                source TEXT NOT NULL DEFAULT 'ics', confidence REAL,
                dismissed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO accounts(id) VALUES ('acct');",
        )
        .unwrap();
        crate::db::schema::migrate_v43_gcal(&conn).unwrap();
        // The delivery ledger is read by `list_unified_by_range`; without
        // it the fixture would silently exercise the missing-table
        // fallback instead of the real join.
        crate::db::schema::migrate_v57_invite_notifications(&conn).unwrap();
        // v43 creates gcal_events; v45 adds the invite-approval snapshot
        // columns by ALTER. Run both so the fixture matches the production
        // migration order rather than a schema that never existed.
        crate::db::schema::migrate_v45_invite_approval_snapshot(&conn).unwrap();
        conn
    }

    #[test]
    fn normalizes_timed_events_to_utc_z_and_preserves_all_day_dates() {
        let timed = GcalEvent {
            start: cxmail_core::mail::gcal_dto::EventDateTime {
                date_time: Some("2026-07-30T16:30:00-04:00".into()),
                date: None,
                time_zone: Some("America/New_York".into()),
            },
            end: None,
            ..Default::default()
        };
        let normalized = normalize_event_times(&timed).unwrap();
        assert_eq!(normalized.0, "2026-07-30T20:30:00Z");
        assert!(!normalized.2);

        let all_day = GcalEvent {
            start: cxmail_core::mail::gcal_dto::EventDateTime {
                date: Some("2026-07-30".into()),
                date_time: None,
                time_zone: Some("America/New_York".into()),
            },
            end: None,
            ..Default::default()
        };
        let normalized = normalize_event_times(&all_day).unwrap();
        assert_eq!(normalized.0, "2026-07-30");
        assert!(normalized.2);
    }

    #[test]
    fn remote_upsert_and_unified_query_dedupe_matching_ics_uid() {
        let conn = setup();
        let calendar = GcalCalendar {
            id: "primary@example.com".to_string(),
            primary: true,
            ..Default::default()
        };
        let calendar_row = upsert_calendar(&conn, "acct", &calendar).unwrap();
        conn.execute(
            "INSERT INTO calendar_events
             (account_id, folder_name, message_uid, event_uid, summary, dtstart)
             VALUES ('acct','INBOX',7,'ical-uid','Email invite','2026-07-30T20:30:00Z')",
            [],
        )
        .unwrap();
        let remote = GcalEvent {
            id: Some("google-event".to_string()),
            i_cal_uid: Some("ical-uid".to_string()),
            summary: Some("Live event".to_string()),
            start: cxmail_core::mail::gcal_dto::EventDateTime {
                date_time: Some("2026-07-30T16:30:00-04:00".to_string()),
                time_zone: Some("America/New_York".to_string()),
                ..Default::default()
            },
            end: Some(cxmail_core::mail::gcal_dto::EventDateTime {
                date_time: Some("2026-07-30T17:30:00-04:00".to_string()),
                time_zone: Some("America/New_York".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        upsert_remote_event(
            &conn,
            calendar_row,
            "acct",
            "primary@example.com",
            &remote,
            "2026-07-28T12:00:00Z",
        )
        .unwrap();
        let rows = list_unified_by_range(
            &conn,
            Some("acct"),
            "2026-07-01T00:00:00Z",
            "2026-08-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "gcal");
        assert_eq!(rows[0].dtstart, "2026-07-30T20:30:00Z");
    }

    #[test]
    fn invite_approval_is_single_event_scoped_and_default_pending() {
        let conn = setup();
        let calendar_row = upsert_calendar(
            &conn,
            "acct",
            &GcalCalendar {
                id: "primary@example.com".to_string(),
                primary: true,
                ..Default::default()
            },
        )
        .unwrap();
        let event_id = upsert_remote_event(
            &conn,
            calendar_row,
            "acct",
            "primary@example.com",
            &GcalEvent {
                id: Some("approval-event".to_string()),
                start: cxmail_core::mail::gcal_dto::EventDateTime {
                    date_time: Some("2026-07-30T16:30:00-04:00".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            "2026-07-28T12:00:00Z",
        )
        .unwrap();

        let approved = vec!["dev@example.com".to_string()];
        assert!(request_invite_approval(&conn, event_id, "request-1", &approved).is_err());
        set_pending_notify(&conn, event_id, true).unwrap();
        request_invite_approval(&conn, event_id, "request-1", &approved).unwrap();
        assert_eq!(
            invite_approval_status(&conn, event_id, "request-1").unwrap(),
            Some("pending".to_string())
        );
        assert!(respond_to_invite_approval(&conn, "request-1", true).unwrap());
        assert_eq!(
            invite_approval_status(&conn, event_id, "request-1").unwrap(),
            Some("approved".to_string())
        );
        assert!(!respond_to_invite_approval(&conn, "request-1", false).unwrap());
        clear_invite_approval(&conn, event_id).unwrap();
        assert_eq!(
            invite_approval_status(&conn, event_id, "request-1").unwrap(),
            None
        );
    }

    /// Seed two events that are both awaiting attendee notification, so an
    /// approval test can assert that a decision touches exactly one of them.
    fn seed_two_pending_events(conn: &Connection) -> (i64, i64) {
        let calendar_row = upsert_calendar(
            conn,
            "acct",
            &GcalCalendar {
                id: "primary@example.com".to_string(),
                primary: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut ids = Vec::new();
        for slug in ["event-one", "event-two"] {
            let id = upsert_remote_event(
                conn,
                calendar_row,
                "acct",
                "primary@example.com",
                &GcalEvent {
                    id: Some(slug.to_string()),
                    start: cxmail_core::mail::gcal_dto::EventDateTime {
                        date_time: Some("2026-07-30T16:30:00-04:00".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                "2026-07-28T12:00:00Z",
            )
            .unwrap();
            set_pending_notify(conn, id, true).unwrap();
            ids.push(id);
        }
        (ids[0], ids[1])
    }

    /// Delivery now happens in a different process than the request, so the
    /// approved recipient set has to survive the round trip through the DB —
    /// and must only be reachable once the user has actually approved.
    #[test]
    fn approved_invite_returns_snapshot_only_after_approval() {
        let conn = setup();
        let (event_id, _) = seed_two_pending_events(&conn);
        let approved = vec!["a@example.com".to_string(), "b@example.com".to_string()];
        request_invite_approval(&conn, event_id, "request-1", &approved).unwrap();

        // Pending is not approved — nothing may be delivered yet.
        assert_eq!(approved_invite(&conn, "request-1").unwrap(), None);

        respond_to_invite_approval(&conn, "request-1", true).unwrap();
        assert_eq!(
            approved_invite(&conn, "request-1").unwrap(),
            Some(ApprovedInvite {
                event_id,
                recipients: approved,
            })
        );

        // An unknown id resolves to nothing rather than to some other event.
        assert_eq!(approved_invite(&conn, "request-does-not-exist").unwrap(), None);

        clear_invite_approval(&conn, event_id).unwrap();
        assert_eq!(approved_invite(&conn, "request-1").unwrap(), None);
    }

    /// A denied decision must not leave a deliverable approval behind.
    #[test]
    fn approved_invite_ignores_denied_decision() {
        let conn = setup();
        let (event_id, _) = seed_two_pending_events(&conn);
        request_invite_approval(&conn, event_id, "request-1", &["a@example.com".to_string()])
            .unwrap();
        respond_to_invite_approval(&conn, "request-1", false).unwrap();
        assert_eq!(approved_invite(&conn, "request-1").unwrap(), None);
    }

    /// Nothing auto-denies any more, so the sweep is the only thing keeping an
    /// unanswered approval from wedging the event permanently. The fixture must
    /// share today's date but sit in the past — a naive 2020 timestamp would
    /// also pass a broken lexicographic comparison (gotcha #23).
    #[test]
    fn sweep_clears_only_stale_pending_approvals() {
        let conn = setup();
        let (fresh_id, stale_id) = seed_two_pending_events(&conn);
        request_invite_approval(&conn, fresh_id, "fresh", &["a@example.com".to_string()]).unwrap();
        request_invite_approval(&conn, stale_id, "stale", &["b@example.com".to_string()]).unwrap();
        conn.execute(
            "UPDATE gcal_events SET invite_approval_requested_at = datetime('now','-30 hours')
             WHERE id=?1",
            params![stale_id],
        )
        .unwrap();

        assert_eq!(sweep_stale_invite_approvals(&conn, 24).unwrap(), 1);
        assert_eq!(
            invite_approval_status(&conn, fresh_id, "fresh").unwrap(),
            Some("pending".to_string())
        );
        assert_eq!(invite_approval_status(&conn, stale_id, "stale").unwrap(), None);

        // An approved-but-undelivered decision is not stale — the user said yes
        // and delivery may simply not have run yet.
        respond_to_invite_approval(&conn, "fresh", true).unwrap();
        conn.execute(
            "UPDATE gcal_events SET invite_approval_requested_at = datetime('now','-30 hours')
             WHERE id=?1",
            params![fresh_id],
        )
        .unwrap();
        assert_eq!(sweep_stale_invite_approvals(&conn, 24).unwrap(), 0);
    }
}
