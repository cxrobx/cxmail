//! Keeps a Zoom meeting in step with the Google Calendar event that owns it.
//!
//! **The reconciler is mirror-driven, not command-driven.** The most common
//! reschedule in practice is dragging the event in Google Calendar's web UI — no
//! local command observes that. `gcal_events` is the one place every reschedule
//! eventually lands, so `plan_for` reads the mirror and runs on the background
//! tick rather than hanging off `update_calendar_event`.
//!
//! `zoom_meetings.pushed_topic` / `pushed_start` / `pushed_duration` record *what
//! Zoom is believed to hold*, advanced only on a 2xx. Four properties fall out of
//! that one choice:
//!
//! * **No PATCH on a mere touch.** An attendee change, an etag bump or a plain
//!   re-pull rewrites the mirror row without changing topic/start/duration, so
//!   the plan is `Hold` and zero HTTP happens.
//! * **Change detection is a string compare with no timezone maths.**
//!   `gcal_events.dtstart` is *always* `YYYY-MM-DDTHH:MM:SSZ`
//!   (`db::gcal::upsert_remote_event` routes every write through
//!   `normalize_event_times` → `to_rfc3339_opts(SecondsFormat::Secs, true)`),
//!   which is byte-identical to what we send Zoom as `start_time`.
//! * **Idempotent.** Running the pass twice is a no-op.
//! * **Self-retrying.** A failed PATCH leaves `pushed_*` untouched, so the next
//!   tick recomputes the same patch.
//!
//! ### The positive-evidence rule
//!
//! The reconciler **never deletes a Zoom meeting because a mirror row is
//! absent.** Absence is ambiguous: a full re-sync sweep, a cancelled-event
//! purge, an account removal and a fresh database all produce it. Deletion
//! requires positive evidence — an explicit local tombstone, a mirror row whose
//! `status` is `cancelled`, or a targeted `get_event` answering 404/410. When the
//! verifying GET cannot run at all, `attempts` climbs and the row is parked
//! `unverified` rather than guessed at.

use crate::db;
use crate::db::gcal::GcalEventRow;
use crate::db::zoom::{state, ZoomMeetingRow, MAX_ATTEMPTS};
use crate::email::gcal::GcalClient;
use crate::email::zoom::{
    self, NewZoomMeeting, ZoomClient, ZoomMeeting, ZoomMeetingPatch,
};
use crate::error::AppError;
use crate::LockExt;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

/// Reasons a plan resolves to "do nothing remote". Named constants rather than
/// ad-hoc strings so the async side can branch on one of them (`retire`) without
/// matching on prose.
pub const HOLD_NO_CHANGE: &str = "Zoom already holds this topic, start and duration";
pub const HOLD_CONFLICT: &str =
    "the Google mirror row is in conflict — resolve that before touching Zoom";
pub const HOLD_ALL_DAY: &str = "the event is all-day, which a Zoom meeting cannot represent";
pub const HOLD_RECURRING: &str =
    "the event is part of a recurring series, which is deliberately out of scope";
pub const HOLD_NOT_RECORDED: &str =
    "no Zoom meeting id was ever recorded for this event, so there is nothing to address";
pub const HOLD_PARKED: &str =
    "the row is parked (retired or unverified) and is never reconciled automatically";
pub const HOLD_PAST_AND_ORPHANED: &str =
    "the event is gone and the meeting is already in the past — retiring rather than deleting, \
     so any cloud recording and attendance report survive";

/// A Zoom meeting must not be created for an event with no end time; 60 minutes
/// is what Google's own UI defaults to.
const DEFAULT_DURATION_MINUTES: i64 = 60;

/// What Zoom *should* hold for an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desired {
    pub topic: String,
    pub start_time: String,
    pub duration: i64,
}

/// What to do about one (event, meeting) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomPlan {
    /// Do nothing remote. Carries the reason so a log line explains itself.
    Hold(&'static str),
    /// Push these fields. `patch` is only the fields that changed; `desired` is
    /// the full triple to record once Zoom accepts it.
    Patch {
        patch: ZoomMeetingPatch,
        desired: Desired,
    },
    /// Positive evidence exists that the meeting should not: delete it.
    Delete,
    /// The mirror row is missing, which is ambiguous. Ask Google directly.
    Verify,
    /// Nothing exists remotely (or ever did), so drop the local row.
    Forget,
}

/// The single place the Zoom `start_time` wire format is decided.
///
/// **Both the create path and the reconcile path must go through this.** They are
/// compared as strings — that is the whole basis of change detection — so seeding
/// the believed-remote state in a different spelling of the same instant produces
/// a `Patch` for an event nobody touched.
///
/// This shipped wrong once, and the trap is worth naming: `to_rfc3339_opts(_,
/// use_z = true)` emits `Z` only when the offset **already is** UTC. A
/// `DateTime<FixedOffset>` at `-04:00` formats as `2026-08-11T15:00:00-04:00`,
/// which is the same instant as `gcal_events.dtstart`'s
/// `2026-08-11T19:00:00Z` and not the same bytes. `with_timezone(&Utc)` first is
/// what makes the flag do anything. Pinned by
/// `tests::a_freshly_created_link_needs_no_immediate_patch`.
pub fn wire_start_time<Tz: chrono::TimeZone>(instant: DateTime<Tz>) -> String {
    instant
        .with_timezone(&Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Derive what Zoom should hold from the mirror row.
///
/// Returns `None` for an all-day event: `dtstart` is then a bare `YYYY-MM-DD`
/// with no instant to give Zoom, and an all-day Zoom meeting is a category error
/// rather than something to approximate.
pub fn desired_from_event(event: &GcalEventRow) -> Option<Desired> {
    if event.is_all_day || event.dtstart.len() == 10 {
        return None;
    }
    let start = DateTime::parse_from_rfc3339(&event.dtstart).ok()?;
    let minutes = event
        .dtend
        .as_deref()
        .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
        .map(|end| (end - start).num_minutes())
        .unwrap_or(DEFAULT_DURATION_MINUTES);
    Some(Desired {
        topic: zoom::topic_from_summary(event.summary.as_deref().unwrap_or("Meeting")),
        // Re-emit through the shared formatter rather than passing `dtstart`
        // through, so the create and reconcile paths cannot disagree.
        start_time: wire_start_time(start),
        duration: zoom::clamp_duration_minutes(minutes),
    })
}

/// Diff the believed-remote state against what the event now says.
fn patch_for(zoom: &ZoomMeetingRow, desired: &Desired) -> ZoomMeetingPatch {
    ZoomMeetingPatch {
        topic: (zoom.pushed_topic.as_deref() != Some(desired.topic.as_str()))
            .then(|| desired.topic.clone()),
        start_time: (zoom.pushed_start.as_deref() != Some(desired.start_time.as_str()))
            .then(|| desired.start_time.clone()),
        duration: (zoom.pushed_duration != Some(desired.duration)).then_some(desired.duration),
    }
}

/// Decide what to do about one pair. Pure — the entire policy lives here, so all
/// of it is testable without a network double.
///
/// `event` is `None` when the Google mirror row is absent, which is **not**
/// evidence the event was deleted (see the positive-evidence rule above).
pub fn plan_for(
    event: Option<&GcalEventRow>,
    zoom: &ZoomMeetingRow,
    now: DateTime<Utc>,
) -> ZoomPlan {
    match zoom.state.as_str() {
        state::DELETE_PENDING => {
            if zoom.zoom_meeting_id.is_none() {
                // The record predates the object and the object never arrived.
                // There is nothing to delete, so the tombstone is just litter.
                ZoomPlan::Forget
            } else {
                ZoomPlan::Delete
            }
        }
        state::RETIRED | state::UNVERIFIED => ZoomPlan::Hold(HOLD_PARKED),
        state::CREATING if zoom.zoom_meeting_id.is_none() => ZoomPlan::Hold(HOLD_NOT_RECORDED),
        _ => plan_for_live_meeting(event, zoom, now),
    }
}

fn plan_for_live_meeting(
    event: Option<&GcalEventRow>,
    zoom: &ZoomMeetingRow,
    now: DateTime<Utc>,
) -> ZoomPlan {
    let Some(event) = event else {
        // A meeting whose start has already passed and whose event has vanished
        // is a completed call, not a mistake. Deleting it can destroy its cloud
        // recording, so it is retired instead — and there is nothing to verify.
        let already_happened = zoom
            .pushed_start
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_some_and(|start| start.with_timezone(&Utc) < now);
        return if already_happened {
            ZoomPlan::Hold(HOLD_PAST_AND_ORPHANED)
        } else {
            ZoomPlan::Verify
        };
    };

    // A conflicted mirror row means CXMail and Google disagree about the event
    // itself. Pushing either side's version to Zoom would pick a winner the user
    // has not picked yet.
    if event.sync_state == "conflict" {
        return ZoomPlan::Hold(HOLD_CONFLICT);
    }
    if event.status == "cancelled" || event.sync_state == "local_deleted" {
        return ZoomPlan::Delete;
    }
    if event.recurring_event_id.is_some() {
        return ZoomPlan::Hold(HOLD_RECURRING);
    }
    let Some(desired) = desired_from_event(event) else {
        return ZoomPlan::Hold(HOLD_ALL_DAY);
    };
    let patch = patch_for(zoom, &desired);
    if patch.is_empty() {
        return ZoomPlan::Hold(HOLD_NO_CHANGE);
    }
    ZoomPlan::Patch { patch, desired }
}

/// Fold a push result back into the believed-remote state.
///
/// `Some(desired)` means record it. `None` means leave the row byte-for-byte as
/// it was, so the next tick recomputes exactly the same patch — which is the
/// whole retry mechanism.
pub fn apply_outcome<'a>(desired: &'a Desired, pushed_ok: bool) -> Option<&'a Desired> {
    pushed_ok.then_some(desired)
}

// ─── The description block ─────────────────────────────────────────────

/// Fence markers. Fenced rather than pattern-matched so the block can be found,
/// replaced and removed exactly, and so a description that merely *mentions* a
/// zoom.us URL in prose is never mistaken for one we wrote.
pub const BLOCK_OPEN: &str = "[cxmail:zoom]";
pub const BLOCK_CLOSE: &str = "[/cxmail:zoom]";

/// Group a Zoom meeting id the way Zoom's own UI does, so it can be read aloud.
fn format_meeting_id(id: &str) -> String {
    if !id.chars().all(|c| c.is_ascii_digit()) {
        return id.to_string();
    }
    match id.len() {
        11 => format!("{} {} {}", &id[..3], &id[3..7], &id[7..]),
        10 => format!("{} {} {}", &id[..3], &id[3..6], &id[6..]),
        _ => id.to_string(),
    }
}

/// The join block appended to an event's **description**.
///
/// Description, never `location` — verified against `src/lib/meetingLink.ts`,
/// which scans `location → description → raw_ics` with a host-anchored
/// `zoom.us/(j|my|w)/` regex. `location` is *rendered* as a raw-URL MapPin line
/// (gotcha #21); `description` is scanned but not displayed, and is what lights
/// CXMail's "Join meeting" button. So this needs no frontend change at all.
pub fn zoom_block(join_url: &str, meeting_id: &str) -> String {
    format!(
        "{BLOCK_OPEN}\nJoin Zoom Meeting\n{join_url}\n\nMeeting ID: {}\n{BLOCK_CLOSE}",
        format_meeting_id(meeting_id)
    )
}

/// Byte range of the fenced block, markers included.
fn find_block(text: &str) -> Option<(usize, usize)> {
    let start = text.find(BLOCK_OPEN)?;
    let close = text[start..].find(BLOCK_CLOSE)? + start;
    Some((start, close + BLOCK_CLOSE.len()))
}

pub fn has_zoom_block(description: Option<&str>) -> bool {
    description.and_then(find_block).is_some()
}

/// Remove our block and restore the surrounding prose byte-identically.
/// `None` when there is no block, which lets callers distinguish "nothing to do"
/// from "removed".
pub fn detach_zoom_block(description: Option<&str>) -> Option<String> {
    let text = description?;
    let (start, end) = find_block(text)?;
    let mut prefix = &text[..start];
    let mut suffix = &text[end..];
    // Strip exactly the separator `upsert_zoom_block` inserts, from whichever
    // side it is on, so a round trip is lossless.
    if let Some(trimmed) = prefix.strip_suffix("\n\n") {
        prefix = trimmed;
    } else if prefix.is_empty() {
        suffix = suffix.strip_prefix("\n\n").unwrap_or(suffix);
    }
    Some(format!("{prefix}{suffix}"))
}

/// Apply the block to a description, replacing any block already there.
///
/// Idempotent by construction: it detaches first, so applying twice yields one
/// block and the user's prose survives untouched.
pub fn upsert_zoom_block(description: Option<&str>, block: &str) -> String {
    let base = detach_zoom_block(description)
        .unwrap_or_else(|| description.unwrap_or_default().to_string());
    if base.is_empty() {
        block.to_string()
    } else {
        format!("{base}\n\n{block}")
    }
}

/// True when replacing `existing` with `incoming` would silently destroy the
/// join link. `update_calendar_event` sets `description` wholesale, so
/// "reschedule and update the description" would otherwise wipe the block and
/// report success.
pub fn zoom_block_would_be_dropped(existing: Option<&str>, incoming: &str) -> bool {
    has_zoom_block(existing) && !has_zoom_block(Some(incoming))
}

/// Refuse to attach Zoom to an event whose shape a single meeting cannot model.
///
/// Refused at the boundary rather than half-modelled: a partially-handled
/// recurrence is worse than a clean "no", and an all-day Zoom meeting is a
/// category error.
pub fn validate_attachable(event: &GcalEventRow) -> Result<(), AppError> {
    if event.is_all_day || event.dtstart.len() == 10 {
        return Err(AppError::ZoomApi(
            0,
            "An all-day event cannot carry a Zoom meeting — a Zoom meeting needs a start instant \
             and a duration. Give the event a start and end time."
                .to_string(),
        ));
    }
    if event.recurring_event_id.is_some() {
        return Err(AppError::ZoomApi(
            0,
            "A recurring event cannot carry a Zoom meeting yet. Create a single event instead."
                .to_string(),
        ));
    }
    Ok(())
}

// ─── Async: create ─────────────────────────────────────────────────────

/// A Zoom meeting created for an event that does not exist in Google yet.
pub struct ZoomAttachment {
    /// `zoom_meetings.id` — needed to unwind if the Google insert then fails.
    pub row_id: i64,
    pub meeting_id: String,
    pub join_url: String,
    /// The event description with the join block applied.
    pub description: String,
}

/// Create the Zoom meeting for an event about to be inserted into Google.
///
/// **Ordering: the record predates the object.** `begin_create` writes a durable
/// `creating` row *before* Zoom is asked for anything, so a crash between the two
/// leaves a visible row rather than a silent orphan.
#[allow(clippy::too_many_arguments)]
pub async fn attach_to_new_event(
    db_conn: &Mutex<rusqlite::Connection>,
    account_id: &str,
    calendar_id: &str,
    gcal_event_id: &str,
    summary: &str,
    start_time: &str,
    duration_minutes: i64,
    description: Option<&str>,
) -> Result<ZoomAttachment, AppError> {
    let topic = zoom::topic_from_summary(summary);
    let duration = zoom::clamp_duration_minutes(duration_minutes);

    // Fail before writing anything if Zoom is not set up.
    let client = ZoomClient::connect().await?;

    let row_id = {
        let conn = db_conn.safe_lock();
        db::zoom::begin_create(&conn, account_id, calendar_id, gcal_event_id)?
    };

    let meeting: ZoomMeeting = match client
        .create_meeting(&NewZoomMeeting {
            topic: topic.clone(),
            meeting_type: 2,
            start_time: start_time.to_string(),
            duration,
            // Correlation handle for a future Zoom-side sweep. Nothing reads it
            // today; it costs one field now and is unrecoverable later.
            agenda: format!("CXMail:{gcal_event_id}"),
        })
        .await
    {
        Ok(meeting) => meeting,
        Err(error) => {
            // Zoom refused, so no remote object exists and the claim is litter.
            let conn = db_conn.safe_lock();
            let _ = db::zoom::forget(&conn, row_id);
            return Err(error);
        }
    };

    {
        let conn = db_conn.safe_lock();
        db::zoom::record_created(
            &conn,
            row_id,
            &meeting.id,
            &meeting.join_url,
            &topic,
            start_time,
            duration,
        )?;
    }

    let block = zoom_block(&meeting.join_url, &meeting.id);
    Ok(ZoomAttachment {
        row_id,
        meeting_id: meeting.id,
        join_url: meeting.join_url,
        description: upsert_zoom_block(description, &block),
    })
}

/// Compensate for a Google `events.insert` that failed *after* the Zoom meeting
/// was created.
///
/// `insert_status` is the HTTP status the insert reported — `0` is the transport
/// sentinel, meaning we never got an answer and therefore do not know whether
/// Google created the event.
///
/// Three outcomes, and the asymmetry is deliberate:
/// * a real status ⇒ the event definitely does not exist ⇒ delete the meeting;
///   if *that* fails, keep the row as a tombstone for the reaper rather than
///   dropping it;
/// * transport sentinel ⇒ probe Google; a 404/410 means no event, so delete;
/// * probe also failed ⇒ **leave everything alone.** Deleting a Zoom meeting
///   that may be attached to a live event is worse than an orphan, and the
///   orphan is visible in settings.
pub async fn unwind_after_failed_insert(
    db_conn: &Mutex<rusqlite::Connection>,
    attachment: &ZoomAttachment,
    gcal_client: &GcalClient,
    calendar_id: &str,
    gcal_event_id: &str,
    insert_status: u16,
) {
    if insert_status == 0 {
        match gcal_client.get_event(calendar_id, gcal_event_id).await {
            Ok(_) => {
                log::warn!(
                    "Zoom meeting {} kept: the Google insert reported no status but the event \
                     exists, so the meeting is attached to it",
                    attachment.meeting_id
                );
                return;
            }
            Err(AppError::CalendarApi(404 | 410, _)) => {}
            Err(error) => {
                log::warn!(
                    "Zoom meeting {} left in place: could not determine whether the Google event \
                     was created ({error}). An orphan is safer than deleting a live meeting; it \
                     is listed under Zoom settings.",
                    attachment.meeting_id
                );
                return;
            }
        }
    }

    let conn_marked = {
        let conn = db_conn.safe_lock();
        db::zoom::mark_delete_pending(&conn, attachment.row_id).is_ok()
    };
    let deleted = match ZoomClient::connect().await {
        Ok(client) => client.delete_meeting(&attachment.meeting_id).await,
        Err(error) => Err(error),
    };
    match deleted {
        Ok(()) => {
            let conn = db_conn.safe_lock();
            let _ = db::zoom::forget(&conn, attachment.row_id);
        }
        Err(error) => {
            // Do NOT drop the row — it is the only thing that will make the
            // reaper try again.
            let conn = db_conn.safe_lock();
            let _ = db::zoom::record_error(&conn, attachment.row_id, &error.to_string());
            log::warn!(
                "Could not delete orphaned Zoom meeting {} ({error}); left as delete_pending \
                 (tombstone written: {conn_marked})",
                attachment.meeting_id
            );
        }
    }
}

// ─── Async: reconcile ──────────────────────────────────────────────────

/// Reconcile the one event a local command just touched.
///
/// Called non-fatally after an update: a Zoom failure must not fail the calendar
/// edit the user actually asked for, and the background pass will retry.
pub async fn reconcile_event(
    db_conn: &Mutex<rusqlite::Connection>,
    gcal_event_row_id: i64,
) -> Result<(), AppError> {
    let (event, link) = {
        let conn = db_conn.safe_lock();
        (
            db::gcal::get_event(&conn, gcal_event_row_id)?,
            db::zoom::get_by_local_event(&conn, gcal_event_row_id)?,
        )
    };
    let Some(link) = link else { return Ok(()) };
    let plan = plan_for(event.as_ref(), &link, Utc::now());
    if matches!(plan, ZoomPlan::Hold(_)) {
        log_hold(&plan, &link);
        return Ok(());
    }
    // `get_by_local_event` joined against the mirror, so the event is present
    // and `Verify` is unreachable here — no GcalClient needed.
    execute_plan(db_conn, event.as_ref(), &link, plan, None).await
}

/// Reconcile every live link for one account.
///
/// `email` is `None` when the account row is gone. Verification then cannot run,
/// so a row needing it climbs `attempts` and parks `unverified` — visible, and
/// never guessed at.
pub async fn reconcile_account(
    db_conn: &Mutex<rusqlite::Connection>,
    account_id: &str,
    email: Option<&str>,
) -> Result<(), AppError> {
    let links = {
        let conn = db_conn.safe_lock();
        db::zoom::list_reconcilable_for_account(&conn, account_id)?
    };
    if links.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let mut gcal_client: Option<GcalClient> = None;

    for link in links {
        let event = {
            let conn = db_conn.safe_lock();
            db::gcal::get_by_remote_id(
                &conn,
                &link.account_id,
                &link.gcal_calendar_id,
                &link.gcal_event_id,
            )?
        };
        let plan = plan_for(event.as_ref(), &link, now);

        if let ZoomPlan::Hold(reason) = plan {
            if reason == HOLD_PAST_AND_ORPHANED {
                let conn = db_conn.safe_lock();
                let _ = db::zoom::retire(&conn, link.id);
                log::info!(
                    "Zoom meeting {} retired: {reason}",
                    link.zoom_meeting_id.as_deref().unwrap_or("?")
                );
            } else {
                log_hold(&plan, &link);
            }
            continue;
        }

        if matches!(plan, ZoomPlan::Verify) && gcal_client.is_none() {
            if let Some(email) = email {
                match GcalClient::for_account(email).await {
                    Ok(client) => gcal_client = Some(client),
                    Err(error) => {
                        park_unverifiable(db_conn, &link, &error.to_string());
                        continue;
                    }
                }
            } else {
                park_unverifiable(
                    db_conn,
                    &link,
                    "the Gmail account that owned this event no longer exists in CXMail",
                );
                continue;
            }
        }

        if let Err(error) =
            execute_plan(db_conn, event.as_ref(), &link, plan, gcal_client.as_ref()).await
        {
            let conn = db_conn.safe_lock();
            let _ = db::zoom::record_error(&conn, link.id, &error.to_string());
            log::warn!("Zoom reconcile failed for event {}: {error}", link.gcal_event_id);
        }
    }
    Ok(())
}

/// Reconcile every account with a live link. Skips entirely when Zoom is not
/// configured, so a user who never enabled it pays nothing.
pub async fn reconcile_all(db_conn: &Mutex<rusqlite::Connection>) -> Result<(), AppError> {
    if !zoom::is_configured() {
        return Ok(());
    }
    let account_ids = {
        let conn = db_conn.safe_lock();
        db::zoom::accounts_with_links(&conn)?
    };
    for account_id in account_ids {
        let email = {
            let conn = db_conn.safe_lock();
            db::accounts::get_by_id(&conn, &account_id)?.map(|account| account.email)
        };
        if let Err(error) = reconcile_account(db_conn, &account_id, email.as_deref()).await {
            log::warn!("Zoom reconcile failed for account {account_id}: {error}");
        }
    }
    Ok(())
}

/// Reap local tombstones. One indexed SELECT, usually zero rows, so it is cheap
/// enough to run on every tick.
pub async fn flush_pending_deletes(
    db_conn: &Mutex<rusqlite::Connection>,
) -> Result<(), AppError> {
    let rows = {
        let conn = db_conn.safe_lock();
        db::zoom::list_pending_deletes(&conn)?
    };
    if rows.is_empty() {
        return Ok(());
    }
    let mut client: Option<ZoomClient> = None;
    for row in rows {
        let Some(meeting_id) = row.zoom_meeting_id.clone() else {
            // Nothing was ever created, so there is nothing to delete.
            let conn = db_conn.safe_lock();
            let _ = db::zoom::forget(&conn, row.id);
            continue;
        };
        if client.is_none() {
            match ZoomClient::connect().await {
                Ok(built) => client = Some(built),
                Err(error) => {
                    // Credentials revoked or unreachable. The tombstones stay,
                    // which is the point — they are reaped once Zoom is back.
                    log::warn!("Zoom delete reaper idle: {error}");
                    return Ok(());
                }
            }
        }
        let client = client.as_ref().expect("built above");
        match client.delete_meeting(&meeting_id).await {
            Ok(()) => {
                let conn = db_conn.safe_lock();
                db::zoom::forget(&conn, row.id)?;
                log::info!("Deleted Zoom meeting {meeting_id}");
            }
            Err(error) => {
                let conn = db_conn.safe_lock();
                let _ = db::zoom::record_error(&conn, row.id, &error.to_string());
                log::warn!("Could not delete Zoom meeting {meeting_id}: {error}");
            }
        }
    }
    Ok(())
}

fn log_hold(plan: &ZoomPlan, link: &ZoomMeetingRow) {
    if let ZoomPlan::Hold(reason) = plan {
        if *reason == HOLD_NO_CHANGE {
            return; // the common case; logging it would drown the log
        }
        log::debug!(
            "Zoom meeting {} held: {reason}",
            link.zoom_meeting_id.as_deref().unwrap_or("?")
        );
    }
}

fn park_unverifiable(
    db_conn: &Mutex<rusqlite::Connection>,
    link: &ZoomMeetingRow,
    reason: &str,
) {
    let conn = db_conn.safe_lock();
    match db::zoom::record_error(&conn, link.id, reason) {
        Ok(attempts) if attempts > MAX_ATTEMPTS => {
            let _ = db::zoom::mark_unverified(&conn, link.id, reason);
            log::warn!(
                "Zoom meeting {} parked as unverified after {attempts} attempts: {reason}. It is \
                 listed under Zoom settings and will never be deleted automatically.",
                link.zoom_meeting_id.as_deref().unwrap_or("?")
            );
        }
        _ => log::debug!(
            "Zoom meeting {} could not be verified this pass: {reason}",
            link.zoom_meeting_id.as_deref().unwrap_or("?")
        ),
    }
}

/// Carry out a decided plan. `Hold` never reaches here.
async fn execute_plan(
    db_conn: &Mutex<rusqlite::Connection>,
    _event: Option<&GcalEventRow>,
    link: &ZoomMeetingRow,
    plan: ZoomPlan,
    gcal_client: Option<&GcalClient>,
) -> Result<(), AppError> {
    match plan {
        ZoomPlan::Hold(_) => Ok(()),
        ZoomPlan::Forget => {
            let conn = db_conn.safe_lock();
            db::zoom::forget(&conn, link.id)
        }
        ZoomPlan::Delete => {
            // Tombstone FIRST, then let the reaper own the HTTP — so a crash
            // between the two is recoverable and there is one delete path.
            {
                let conn = db_conn.safe_lock();
                db::zoom::mark_delete_pending(&conn, link.id)?;
            }
            flush_pending_deletes(db_conn).await
        }
        ZoomPlan::Patch { patch, desired } => {
            let Some(meeting_id) = link.zoom_meeting_id.as_deref() else {
                return Ok(());
            };
            let client = ZoomClient::connect().await?;
            let result = client.update_meeting(meeting_id, &patch).await;
            // The believed-remote state advances only on success, which is what
            // makes a failed push retry the identical patch next tick.
            match apply_outcome(&desired, result.is_ok()) {
                Some(desired) => {
                    let conn = db_conn.safe_lock();
                    db::zoom::record_pushed(
                        &conn,
                        link.id,
                        &desired.topic,
                        &desired.start_time,
                        desired.duration,
                    )?;
                    log::info!("Zoom meeting {meeting_id} updated to match its calendar event");
                    Ok(())
                }
                None => result,
            }
        }
        ZoomPlan::Verify => {
            let Some(client) = gcal_client else {
                park_unverifiable(db_conn, link, "no Google Calendar client was available");
                return Ok(());
            };
            match client
                .get_event(&link.gcal_calendar_id, &link.gcal_event_id)
                .await
            {
                Ok(event) if event.status.as_deref() == Some("cancelled") => {
                    let conn = db_conn.safe_lock();
                    db::zoom::mark_delete_pending(&conn, link.id)
                }
                Ok(_) => {
                    // The event is alive; only our cached mirror row is missing
                    // (a full re-sync sweep does this routinely). The next pull
                    // restores it. Deleting anything here is the exact mistake
                    // the positive-evidence rule exists to prevent.
                    log::debug!(
                        "Zoom meeting {} kept: the Google event still exists, only the local \
                         mirror row is missing",
                        link.zoom_meeting_id.as_deref().unwrap_or("?")
                    );
                    Ok(())
                }
                Err(AppError::CalendarApi(404 | 410, _)) => {
                    let conn = db_conn.safe_lock();
                    db::zoom::mark_delete_pending(&conn, link.id)
                }
                Err(error) => {
                    park_unverifiable(db_conn, link, &error.to_string());
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(dtstart: &str, dtend: Option<&str>, summary: &str) -> GcalEventRow {
        GcalEventRow {
            id: 1,
            calendar_row_id: 1,
            account_id: "acct".to_string(),
            gcal_calendar_id: "primary".to_string(),
            gcal_event_id: "evt-1".to_string(),
            ical_uid: None,
            recurring_event_id: None,
            original_start_time: None,
            etag: Some("etag".to_string()),
            sequence: 0,
            updated: None,
            summary: Some(summary.to_string()),
            description: None,
            location: None,
            dtstart: dtstart.to_string(),
            dtend: dtend.map(str::to_string),
            is_all_day: dtstart.len() == 10,
            start_tz: Some("America/New_York".to_string()),
            end_tz: None,
            organizer_name: None,
            organizer_email: None,
            organizer_self: true,
            attendees_json: None,
            self_response_status: None,
            status: "confirmed".to_string(),
            transparency: None,
            html_link: None,
            hangout_link: None,
            conference_json: None,
            sync_state: "synced".to_string(),
            pending_notify: false,
            push_attempts: 0,
            local_updated_at: None,
            raw_json: None,
            remote_json: None,
        }
    }

    fn link(topic: &str, start: &str, duration: i64) -> ZoomMeetingRow {
        ZoomMeetingRow {
            id: 7,
            account_id: "acct".to_string(),
            gcal_calendar_id: "primary".to_string(),
            gcal_event_id: "evt-1".to_string(),
            zoom_meeting_id: Some("86903742305".to_string()),
            join_url: Some("https://us02web.zoom.us/j/86903742305".to_string()),
            state: state::ACTIVE.to_string(),
            pushed_topic: Some(topic.to_string()),
            pushed_start: Some(start.to_string()),
            pushed_duration: Some(duration),
            attempts: 0,
            last_error: None,
            created_at: "2026-08-10T00:00:00Z".to_string(),
            updated_at: "2026-08-10T00:00:00Z".to_string(),
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn plan_holds_when_nothing_the_meeting_cares_about_changed() {
        let event = event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Design review");
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);
        assert_eq!(
            plan_for(Some(&event), &link, now()),
            ZoomPlan::Hold(HOLD_NO_CHANGE)
        );

        // An attendee change / etag bump rewrites the mirror row without moving
        // topic, start or duration — so still zero HTTP.
        let mut touched = event.clone();
        touched.etag = Some("etag-2".to_string());
        touched.attendees_json = Some(r#"[{"email":"a@example.com"}]"#.to_string());
        touched.pending_notify = true;
        assert_eq!(
            plan_for(Some(&touched), &link, now()),
            ZoomPlan::Hold(HOLD_NO_CHANGE)
        );
    }

    #[test]
    fn plan_patches_the_start_when_the_event_moves() {
        let moved = event("2026-08-12T17:00:00Z", Some("2026-08-12T17:45:00Z"), "Design review");
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);
        match plan_for(Some(&moved), &link, now()) {
            ZoomPlan::Patch { patch, desired } => {
                assert_eq!(patch.start_time.as_deref(), Some("2026-08-12T17:00:00Z"));
                assert!(patch.topic.is_none(), "the title did not change");
                assert!(patch.duration.is_none(), "the length did not change");
                assert_eq!(desired.duration, 45);
            }
            other => panic!("expected Patch, got {other:?}"),
        }
    }

    /// A rename must NOT carry `start_time`. Sending it would be harmless on
    /// Zoom but would make it impossible to tell a rename from a reschedule on
    /// the wire, which is step 7 of the live verification.
    #[test]
    fn plan_patches_only_the_topic_on_a_rename() {
        let renamed = event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Renamed");
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);
        match plan_for(Some(&renamed), &link, now()) {
            ZoomPlan::Patch { patch, .. } => {
                assert_eq!(patch.topic.as_deref(), Some("Renamed"));
                assert!(patch.start_time.is_none());
                assert!(patch.duration.is_none());
            }
            other => panic!("expected Patch, got {other:?}"),
        }
    }

    #[test]
    fn plan_patches_the_duration_when_only_the_end_moves() {
        let longer = event("2026-08-12T15:00:00Z", Some("2026-08-12T16:30:00Z"), "Design review");
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);
        match plan_for(Some(&longer), &link, now()) {
            ZoomPlan::Patch { patch, .. } => {
                assert_eq!(patch.duration, Some(90));
                assert!(patch.start_time.is_none());
                assert!(patch.topic.is_none());
            }
            other => panic!("expected Patch, got {other:?}"),
        }
    }

    #[test]
    fn plan_deletes_on_positive_evidence_only() {
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);

        let mut cancelled =
            event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Design review");
        cancelled.status = "cancelled".to_string();
        assert_eq!(plan_for(Some(&cancelled), &link, now()), ZoomPlan::Delete);

        let mut locally_deleted =
            event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Design review");
        locally_deleted.sync_state = "local_deleted".to_string();
        assert_eq!(plan_for(Some(&locally_deleted), &link, now()), ZoomPlan::Delete);

        let mut tombstoned = link.clone();
        tombstoned.state = state::DELETE_PENDING.to_string();
        assert_eq!(plan_for(None, &tombstoned, now()), ZoomPlan::Delete);
    }

    /// The most dangerous regression the design guards against: a full re-sync
    /// sweeps mirror rows, and that must never delete a Zoom meeting.
    #[test]
    fn a_missing_mirror_row_verifies_and_never_deletes() {
        let future = link("Design review", "2026-08-12T15:00:00Z", 45);
        assert_eq!(plan_for(None, &future, now()), ZoomPlan::Verify);

        // A completed meeting whose event is gone is retired, not deleted —
        // deleting it can destroy its recording and attendance report.
        let past = link("Design review", "2026-08-01T15:00:00Z", 45);
        assert_eq!(
            plan_for(None, &past, now()),
            ZoomPlan::Hold(HOLD_PAST_AND_ORPHANED)
        );

        // Absence of a parseable believed-start is still not evidence.
        let mut unknown = future.clone();
        unknown.pushed_start = None;
        assert_eq!(plan_for(None, &unknown, now()), ZoomPlan::Verify);
    }

    #[test]
    fn plan_holds_on_conflict_all_day_recurring_and_parked_rows() {
        let link = link("Design review", "2026-08-12T15:00:00Z", 45);

        let mut conflicted =
            event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Design review");
        conflicted.sync_state = "conflict".to_string();
        assert_eq!(
            plan_for(Some(&conflicted), &link, now()),
            ZoomPlan::Hold(HOLD_CONFLICT)
        );

        let all_day = event("2026-08-12", None, "Offsite");
        assert_eq!(
            plan_for(Some(&all_day), &link, now()),
            ZoomPlan::Hold(HOLD_ALL_DAY)
        );

        let mut recurring =
            event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Standup");
        recurring.recurring_event_id = Some("series-1".to_string());
        assert_eq!(
            plan_for(Some(&recurring), &link, now()),
            ZoomPlan::Hold(HOLD_RECURRING)
        );

        let happy = event("2026-08-12T17:00:00Z", Some("2026-08-12T17:45:00Z"), "Moved");
        for parked in [state::RETIRED, state::UNVERIFIED] {
            let mut row = link.clone();
            row.state = parked.to_string();
            assert_eq!(
                plan_for(Some(&happy), &row, now()),
                ZoomPlan::Hold(HOLD_PARKED),
                "{parked} rows are never reconciled automatically"
            );
        }
    }

    /// The unrecoverable window — Zoom accepted the create, the process died
    /// before the id landed — must stay visible and must never be acted on.
    #[test]
    fn a_creating_row_with_no_meeting_id_is_held_not_guessed_at() {
        let mut row = link("Design review", "2026-08-12T15:00:00Z", 45);
        row.state = state::CREATING.to_string();
        row.zoom_meeting_id = None;
        row.pushed_start = None;
        assert_eq!(plan_for(None, &row, now()), ZoomPlan::Hold(HOLD_NOT_RECORDED));

        // A tombstone with nothing behind it is litter, not a delete.
        row.state = state::DELETE_PENDING.to_string();
        assert_eq!(plan_for(None, &row, now()), ZoomPlan::Forget);
    }

    /// **The bug this file shipped with, caught live on the first real create.**
    ///
    /// The create path seeded `pushed_start` from a `DateTime<FixedOffset>` via
    /// `to_rfc3339_opts(_, use_z = true)`, which emits `Z` only when the offset
    /// already IS UTC — so a `-04:00` instant was stored as
    /// `2026-08-11T15:00:00-04:00` while `gcal_events.dtstart` held
    /// `2026-08-11T19:00:00Z`. Same instant, different bytes, and change
    /// detection is a byte compare: the next tick patched an untouched event.
    ///
    /// Self-healing after one wasted PATCH, so nothing broke visibly — which is
    /// exactly why it needs a test rather than an eyeball.
    #[test]
    fn a_freshly_created_link_needs_no_immediate_patch() {
        // What `event_datetime` hands the create path: a zoned local instant.
        let local = DateTime::parse_from_rfc3339("2026-08-11T15:00:00-04:00").unwrap();
        // What Google returns and `normalize_event_times` stores: the same
        // instant as UTC with a trailing Z.
        let mirror = event("2026-08-11T19:00:00Z", Some("2026-08-11T19:45:00Z"), "CXMail Zoom test");

        let mut link = link("CXMail Zoom test", &wire_start_time(local), 45);
        assert_eq!(
            link.pushed_start.as_deref(),
            Some("2026-08-11T19:00:00Z"),
            "the shared formatter must normalize to UTC, not preserve the offset"
        );
        assert_eq!(
            plan_for(Some(&mirror), &link, now()),
            ZoomPlan::Hold(HOLD_NO_CHANGE),
            "a just-created meeting must need no patch at all"
        );

        // Negative control — the exact broken spelling, proving this test can
        // actually see the bug rather than passing vacuously.
        link.pushed_start = Some(local.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        assert_eq!(link.pushed_start.as_deref(), Some("2026-08-11T15:00:00-04:00"));
        assert!(
            matches!(plan_for(Some(&mirror), &link, now()), ZoomPlan::Patch { .. }),
            "the un-normalized form is what caused the spurious patch"
        );
    }

    #[test]
    fn wire_start_time_normalizes_every_offset_to_the_same_bytes() {
        // Three spellings of one instant must produce one string.
        for spelling in [
            "2026-08-11T19:00:00Z",
            "2026-08-11T15:00:00-04:00",
            "2026-08-11T21:00:00+02:00",
        ] {
            let parsed = DateTime::parse_from_rfc3339(spelling).unwrap();
            assert_eq!(
                wire_start_time(parsed),
                "2026-08-11T19:00:00Z",
                "{spelling} must normalize to the dtstart byte contract"
            );
        }
    }

    #[test]
    fn a_failed_push_does_not_advance_the_recorded_state() {
        let moved = event("2026-08-12T17:00:00Z", Some("2026-08-12T17:45:00Z"), "Design review");
        let row = link("Design review", "2026-08-12T15:00:00Z", 45);
        let ZoomPlan::Patch { patch, desired } = plan_for(Some(&moved), &row, now()) else {
            panic!("expected Patch");
        };

        // Failure records nothing…
        assert!(apply_outcome(&desired, false).is_none());
        // …so the row is unchanged and the next tick computes the same patch.
        let retried = plan_for(Some(&moved), &row, now());
        assert_eq!(retried, ZoomPlan::Patch { patch: patch.clone(), desired: desired.clone() });

        // Success records the triple, and then there is nothing left to do.
        let recorded = apply_outcome(&desired, true).expect("success records");
        let mut advanced = row.clone();
        advanced.pushed_topic = Some(recorded.topic.clone());
        advanced.pushed_start = Some(recorded.start_time.clone());
        advanced.pushed_duration = Some(recorded.duration);
        assert_eq!(
            plan_for(Some(&moved), &advanced, now()),
            ZoomPlan::Hold(HOLD_NO_CHANGE)
        );
    }

    #[test]
    fn desired_defaults_a_missing_end_to_one_hour_and_clamps_absurd_ones() {
        let no_end = event("2026-08-12T15:00:00Z", None, "Design review");
        assert_eq!(desired_from_event(&no_end).unwrap().duration, 60);

        let marathon = event("2026-08-12T15:00:00Z", Some("2026-08-20T15:00:00Z"), "Retreat");
        assert_eq!(desired_from_event(&marathon).unwrap().duration, 1440);

        assert!(desired_from_event(&event("2026-08-12", None, "Offsite")).is_none());
    }

    #[test]
    fn upsert_zoom_block_is_idempotent_and_preserves_prose() {
        let block = zoom_block("https://us02web.zoom.us/j/86903742305", "86903742305");
        // The exact literal, byte for byte. `src/lib/__tests__/zoomBlockLink.test.ts`
        // holds a copy and asserts that `extractMeetingLink` finds the join URL
        // inside it — drift here silently stops the "Join meeting" button
        // appearing, which is indistinguishable from the meeting never existing.
        assert_eq!(
            block,
            "[cxmail:zoom]\nJoin Zoom Meeting\nhttps://us02web.zoom.us/j/86903742305\n\n\
             Meeting ID: 869 0374 2305\n[/cxmail:zoom]"
        );

        let prose = "Agenda:\n- roadmap\n- hiring";
        let once = upsert_zoom_block(Some(prose), &block);
        let twice = upsert_zoom_block(Some(&once), &block);
        assert_eq!(once, twice, "applying twice yields one block");
        assert_eq!(once.matches(BLOCK_OPEN).count(), 1);
        assert!(once.starts_with(prose));

        // Detaching restores the original prose byte-identically.
        assert_eq!(detach_zoom_block(Some(&twice)).as_deref(), Some(prose));

        // Applied to an empty description, and back again.
        let bare = upsert_zoom_block(None, &block);
        assert_eq!(bare, block);
        assert_eq!(detach_zoom_block(Some(&bare)).as_deref(), Some(""));

        // Replacing an old link with a new one leaves exactly one block.
        let fresh = zoom_block("https://us02web.zoom.us/j/999", "999");
        let replaced = upsert_zoom_block(Some(&once), &fresh);
        assert_eq!(replaced.matches(BLOCK_OPEN).count(), 1);
        assert!(replaced.contains("/j/999"));
        assert!(!replaced.contains("86903742305"));
        assert!(replaced.starts_with(prose));

        // A description that merely MENTIONS zoom.us is not ours and survives.
        let passing = "Dana said he'd send a https://zoom.us/j/12345 link later";
        assert!(!has_zoom_block(Some(passing)));
        assert_eq!(detach_zoom_block(Some(passing)), None);
        assert!(upsert_zoom_block(Some(passing), &block).starts_with(passing));
    }

    #[test]
    fn a_wholesale_description_update_that_would_drop_the_block_is_detectable() {
        let block = zoom_block("https://us02web.zoom.us/j/1", "1");
        let with_block = upsert_zoom_block(Some("Agenda"), &block);

        assert!(zoom_block_would_be_dropped(Some(&with_block), "New agenda"));
        assert!(!zoom_block_would_be_dropped(
            Some(&with_block),
            &upsert_zoom_block(Some("New agenda"), &block)
        ));
        // Nothing to lose when there was no block to begin with.
        assert!(!zoom_block_would_be_dropped(Some("Agenda"), "New agenda"));
        assert!(!zoom_block_would_be_dropped(None, "New agenda"));
    }

    #[test]
    fn validate_attachable_refuses_shapes_a_single_meeting_cannot_model() {
        let timed = event("2026-08-12T15:00:00Z", Some("2026-08-12T15:45:00Z"), "Design review");
        assert!(validate_attachable(&timed).is_ok());

        assert!(validate_attachable(&event("2026-08-12", None, "Offsite")).is_err());

        let mut recurring = timed.clone();
        recurring.recurring_event_id = Some("series-1".to_string());
        assert!(validate_attachable(&recurring).is_err());
    }

    #[test]
    fn meeting_ids_are_grouped_the_way_zoom_prints_them() {
        assert_eq!(format_meeting_id("86903742305"), "869 0374 2305");
        assert_eq!(format_meeting_id("8690374230"), "869 037 4230");
        assert_eq!(format_meeting_id("12345"), "12345");
        assert_eq!(format_meeting_id("abc-def"), "abc-def");
    }
}
