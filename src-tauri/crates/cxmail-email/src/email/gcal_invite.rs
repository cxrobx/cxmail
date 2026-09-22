//! App-side delivery of an approved calendar invitation.
//!
//! `send_calendar_invites` (the MCP tool) does not send anything. It captures a
//! recipient snapshot, records an approval request, pings the app, and returns
//! immediately — so the tool call cannot expire while the user is looking away.
//! Approving in the app is what actually notifies attendees, and this module is
//! that step.
//!
//! Two properties from the old blocking implementation are load-bearing and are
//! preserved here verbatim:
//!
//! * **Drift check.** The live attendee list is re-read from Google immediately
//!   before the PATCH and compared against the set the user approved. Anyone
//!   added in the meantime aborts delivery rather than being silently notified.
//!   Since approval is now open-ended, that window is hours rather than seconds,
//!   which makes the check more important, not less.
//! * **Single-use.** The approval is cleared on every terminal path — success,
//!   drift, or API failure — so one decision can never deliver twice.
//!
//! Every DB lock scope closes before the network calls (gotcha #11): holding the
//! connection mutex across an await blocks the UI's own queries for the duration
//! of a Google round trip.

use cxmail_core::AppCtx;

use crate::db;
use crate::email::gcal::{EventPatch, GcalClient, SendUpdates};
use crate::error::AppError;
use crate::LockExt;

/// What happened to an approved invitation, for logging and the UI event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Attendees were notified. Carries the recipient list actually notified.
    Sent(Vec<String>),
    /// The approval id had no approved event behind it — already delivered,
    /// cleared by the staleness sweep, or denied. Not an error.
    NothingToDo,
}

/// Normalize an attendee list into the comparable form the approval snapshot
/// is stored in. Kept in one place so the request side and the delivery side
/// cannot drift apart in how they spell an address.
pub fn normalize_recipients<'a>(emails: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out = emails
        .into_iter()
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

/// Render a stored `gcal_events.dtstart` for human eyes on the approval card.
///
/// The column holds either a UTC instant (`…Z`, normalized by
/// `normalize_event_times`) or a bare `YYYY-MM-DD` for all-day events. Showing
/// the raw value put `2026-08-03T20:00:00Z` in front of the user — not just
/// unreadable, but a *different number* from the 4:00 PM the meeting actually
/// happens at. On a card whose entire job is "confirm this before notifying
/// people", the one field that must not require mental arithmetic is the time.
pub fn format_event_start(dtstart: &str) -> String {
    format_event_start_in(dtstart, &chrono::Local)
}

fn format_event_start_in<Tz>(dtstart: &str, tz: &Tz) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(dtstart) {
        return parsed
            .with_timezone(tz)
            .format("%a, %b %-d · %-I:%M %p")
            .to_string();
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(dtstart, "%Y-%m-%d") {
        return format!("{} · all day", date.format("%a, %b %-d"));
    }
    // Unrecognized shape: show it verbatim rather than inventing a time.
    dtstart.to_string()
}

/// Deliver the invitation behind `approval_id`, if it is approved and undelivered.
///
/// Returns `NothingToDo` rather than an error when there is no approved event
/// for the id: a double-click on Approve, or a decision the sweep already
/// cleared, is benign and must not surface as a failure.
pub async fn deliver_approved_invite(
    ctx: &AppCtx,
    approval_id: &str,
) -> Result<DeliveryOutcome, AppError> {

    // 1. Resolve the approval to an event + the recipients the user saw.
    let Some(approved) = ({
        let conn = ctx.db().safe_lock();
        db::gcal::approved_invite(&conn, approval_id)?
    }) else {
        return Ok(DeliveryOutcome::NothingToDo);
    };

    if approved.recipients.is_empty() {
        // Either a pre-v45 approval with no snapshot, or an event whose
        // attendees vanished between request and approval. Fail closed.
        let conn = ctx.db().safe_lock();
        db::gcal::clear_invite_approval(&conn, approved.event_id)?;
        return Err(AppError::General(
            "The approved recipient list is empty; nothing was sent. Ask Claude to request approval again."
                .to_string(),
        ));
    }

    // 2. Load the event row and its account.
    let (row, account_email) = {
        let conn = ctx.db().safe_lock();
        let row = db::gcal::get_event(&conn, approved.event_id)?
            .ok_or_else(|| AppError::General("Calendar event not found".to_string()))?;
        let account = db::accounts::get_by_id(&conn, &row.account_id)?
            .ok_or_else(|| AppError::General(format!("Account {} not found", row.account_id)))?;
        (row, account.email)
    };

    // 3. Re-read the live event. No lock is held across any of this.
    let client = GcalClient::for_account(&account_email).await?;
    let fresh = client
        .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
        .await?;
    let attendees = fresh.attendees.clone().unwrap_or_default();
    let current_recipients = normalize_recipients(
        attendees
            .iter()
            .filter_map(|attendee| attendee.email.as_deref()),
    );

    // 4. Drift check — the approval authorized one specific recipient set.
    if current_recipients != approved.recipients {
        let conn = ctx.db().safe_lock();
        db::gcal::clear_invite_approval(&conn, approved.event_id)?;
        return Err(AppError::General(
            "The attendee list changed since you approved; no invitation was sent. Review the updated recipients and approve again."
                .to_string(),
        ));
    }

    // 5. Notify.
    let remote = match client
        .patch_event(
            &row.gcal_calendar_id,
            &row.gcal_event_id,
            &EventPatch {
                summary: row.summary.clone(),
                attendees: Some(attendees.clone()),
                ..Default::default()
            },
            fresh.etag.as_deref(),
            SendUpdates::All,
        )
        .await
    {
        Ok(remote) => remote,
        Err(error) => {
            // Clear the approval so the event is not wedged behind a pending
            // decision that already resolved. The user re-approves to retry.
            let conn = ctx.db().safe_lock();
            db::gcal::clear_invite_approval(&conn, approved.event_id)?;
            return Err(error);
        }
    };

    // 6. Record the result.
    {
        let conn = ctx.db().safe_lock();
        let id = db::gcal::upsert_remote_event(
            &conn,
            row.calendar_row_id,
            &row.account_id,
            &row.gcal_calendar_id,
            &remote,
            &chrono::Utc::now().to_rfc3339(),
        )?;
        db::gcal::set_pending_notify(&conn, id, false)?;
        db::gcal::clear_invite_approval(&conn, id)?;
        // Keep the receipt. `clear_invite_approval` above destroys the approval
        // snapshot — correct, since an approval is single-use — but that is also
        // the only trace that anyone was ever told, and Google records no such
        // fact on the event itself. Written strictly after the 2xx, so a row
        // means "Google accepted the send" and nothing weaker. Keyed on the
        // remote triple, never on `id`: that id is AUTOINCREMENT and a routine
        // sweep-then-repull would reissue it (gotcha #44).
        db::invite_notifications::record_notified(
            &conn,
            &row.account_id,
            &row.gcal_calendar_id,
            &row.gcal_event_id,
            &current_recipients,
            "approval",
        )?;
    }

    ctx.emit(
        "calendar-invites-sent",
        serde_json::json!({
            "event_id": approved.event_id,
            "account_id": row.account_id,
            "recipients": current_recipients,
        }),
    );

    Ok(DeliveryOutcome::Sent(current_recipients))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot and the live list are compared as normalized strings, so
    /// casing, padding, and ordering differences must not read as drift — and
    /// a genuinely new recipient must.
    #[test]
    fn normalize_recipients_is_order_and_case_insensitive() {
        let approved = normalize_recipients(["Dev@Example.com", " chris@cxventures.io "]);
        let live = normalize_recipients(["chris@cxventures.io", "dev@example.com"]);
        assert_eq!(approved, live);

        let with_extra = normalize_recipients([
            "chris@cxventures.io",
            "dev@example.com",
            "someone-new@example.com",
        ]);
        assert_ne!(approved, with_extra);
    }

    /// The exact case from the Dev Sync card: a UTC instant must render as the
    /// local wall-clock time, not the stored Z value. Pinned to a fixed offset
    /// so the test doesn't depend on the machine's timezone.
    #[test]
    fn format_event_start_renders_local_12_hour_time() {
        let eastern = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(
            format_event_start_in("2026-08-03T20:00:00Z", &eastern),
            "Mon, Aug 3 · 4:00 PM"
        );
        // Morning, and a zone east of UTC that rolls to the next day.
        assert_eq!(
            format_event_start_in("2026-08-03T13:05:00Z", &eastern),
            "Mon, Aug 3 · 9:05 AM"
        );
        let tokyo = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            format_event_start_in("2026-08-03T20:00:00Z", &tokyo),
            "Tue, Aug 4 · 5:00 AM"
        );
    }

    #[test]
    fn format_event_start_handles_all_day_and_unknown_shapes() {
        let utc = chrono::Utc;
        assert_eq!(
            format_event_start_in("2026-08-03", &utc),
            "Mon, Aug 3 · all day"
        );
        // Never invent a time for something we can't parse.
        assert_eq!(format_event_start_in("whenever", &utc), "whenever");
    }

    #[test]
    fn normalize_recipients_drops_blanks_and_duplicates() {
        assert_eq!(
            normalize_recipients(["a@x.com", "   ", "A@X.com", ""]),
            vec!["a@x.com".to_string()]
        );
    }
}
