use crate::db;
use crate::email::{
    calendar as ical_util,
    gcal::{EventPatch, GcalClient, NewEvent, SendUpdates},
    gcal_sync, smtp,
};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::{Deserialize, Serialize};
use tauri::State;

// Moved into `cxmail-email` so the MCP can reach them without depending on
// the app crate. Re-exported so this module's own call sites are unchanged.
pub(crate) use crate::email::event_input::{event_datetime, validate_attendees};

#[tauri::command]
pub async fn get_calendar_events(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<Vec<db::calendar::CalendarEventRow>, AppError> {
    let conn = state.db.safe_lock();
    db::calendar::get_by_message(&conn, &account_id, &folder, uid)
}

#[tauri::command]
pub async fn rsvp_event(
    state: State<'_, AppState>,
    event_id: i64,
    account_id: String,
    response: String,
) -> Result<(), AppError> {
    let partstat = match response.as_str() {
        "accepted" => "ACCEPTED",
        "declined" => "DECLINED",
        "tentative" => "TENTATIVE",
        _ => return Err(AppError::General("Invalid RSVP response".to_string())),
    };

    let (event, account) = {
        let conn = state.db.safe_lock();
        let events = db::calendar::get_by_message(&conn, &account_id, "", 0);
        // Get the specific event by id
        let mut stmt = conn.prepare(
            "SELECT id, account_id, folder_name, message_uid, event_uid, summary, description, location,
                    dtstart, dtend, organizer_name, organizer_email, status, method, rsvp_status, raw_ics,
                    source, confidence, dismissed, created_at
             FROM calendar_events WHERE id = ?1",
        )?;
        let event = stmt
            .query_row(rusqlite::params![event_id], |row| {
                Ok(db::calendar::CalendarEventRow {
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
            })
            .map_err(|_| AppError::NotFound("Calendar event not found".to_string()))?;

        drop(events);

        let account = db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
        (event, account)
    };
    let account_email = account.email.clone();

    let organizer_email = event
        .organizer_email
        .as_deref()
        .ok_or_else(|| AppError::General("No organizer email for RSVP".to_string()))?;

    let event_uid = event.event_uid.as_deref().unwrap_or("unknown");

    // Generate RSVP ICS
    let rsvp_ics = ical_util::generate_rsvp_ics(
        event_uid,
        &event.dtstart,
        organizer_email,
        &account_email,
        partstat,
    );

    // Send RSVP email with text/calendar attachment
    let display = match partstat {
        "ACCEPTED" => "Accepted",
        "DECLINED" => "Declined",
        "TENTATIVE" => "Tentative",
        _ => partstat,
    };
    let subject = format!(
        "{}: {}",
        display,
        event.summary.as_deref().unwrap_or("Event")
    );

    let email = smtp::OutgoingEmail {
        from_email: account_email.clone(),
        from_name: None,
        to: vec![smtp::EmailRecipient {
            name: event.organizer_name.clone(),
            email: organizer_email.to_string(),
        }],
        cc: vec![],
        bcc: vec![],
        subject,
        html_body: format!(
            "<p>{} has {} the invitation: {}</p>",
            account_email,
            partstat.to_lowercase(),
            event.summary.as_deref().unwrap_or("Event"),
        ),
        plain_body: None,
        in_reply_to: None,
        references: None,
        track_opens: None,
        attachments: vec![smtp::OutgoingAttachment {
            filename: "response.ics".to_string(),
            content_type: "text/calendar; method=REPLY".to_string(),
            data_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                rsvp_ics.as_bytes(),
            ),
        }],
    };

    smtp::send_email(&account, &email).await?;

    // Update RSVP status in DB
    {
        let conn = state.db.safe_lock();
        db::calendar::update_rsvp(&conn, event_id, &response)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn list_upcoming_events(
    state: State<'_, AppState>,
    account_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<db::calendar::CalendarEventRow>, AppError> {
    let conn = state.db.safe_lock();
    db::calendar::list_upcoming(&conn, account_id.as_deref(), limit.unwrap_or(10))
}

#[tauri::command]
pub async fn list_calendar_events_in_range(
    state: State<'_, AppState>,
    account_id: Option<String>,
    range_start: String,
    range_end: String,
) -> Result<Vec<db::gcal::UnifiedCalendarEvent>, AppError> {
    let conn = state.db.safe_lock();
    db::gcal::list_unified_by_range(&conn, account_id.as_deref(), &range_start, &range_end)
}

#[tauri::command]
pub async fn dismiss_calendar_event(
    state: State<'_, AppState>,
    event_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::calendar::dismiss_event(&conn, event_id)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalendarEventInput {
    pub account_id: String,
    pub calendar_id: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: String,
    pub time_zone: String,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default)]
    pub add_meet: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalendarEventUpdate {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub time_zone: Option<String>,
    pub attendees: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalendarConnection {
    pub account_id: String,
    pub connected: bool,
    pub calendars: Vec<db::gcal::GcalCalendarRow>,
}



async fn account_and_client(
    state: &AppState,
    account_id: &str,
) -> Result<(db::accounts::Account, GcalClient), AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };
    if account.provider != "gmail" {
        return Err(AppError::General(
            "Google Calendar requires a Gmail account".to_string(),
        ));
    }
    let client = GcalClient::for_account(&account.email).await?;
    Ok((account, client))
}

async fn refresh_calendar_list(
    state: &AppState,
    account_id: &str,
    client: &GcalClient,
) -> Result<Vec<db::gcal::GcalCalendarRow>, AppError> {
    let calendars = client.list_calendars().await?;
    let conn = state.db.safe_lock();
    for calendar in &calendars {
        db::gcal::upsert_calendar(&conn, account_id, calendar)?;
    }
    db::gcal::list_calendars(&conn, account_id)
}

#[tauri::command]
pub async fn list_google_calendars(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<CalendarConnection, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };
    let connected =
        account.provider == "gmail" && crate::email::oauth2::is_gcal_connected(&account.email)?;
    let calendars = if connected {
        let client = GcalClient::for_account(&account.email).await?;
        refresh_calendar_list(state.inner(), &account_id, &client).await?
    } else {
        Vec::new()
    };
    Ok(CalendarConnection {
        account_id,
        connected,
        calendars,
    })
}

#[tauri::command]
pub async fn create_google_calendar_event(
    state: State<'_, AppState>,
    input: CalendarEventInput,
) -> Result<db::gcal::GcalEventRow, AppError> {
    let summary = input.summary.trim();
    if summary.is_empty() || summary.chars().count() > 500 {
        return Err(AppError::General(
            "Event title must be between 1 and 500 characters".to_string(),
        ));
    }
    let (_account, client) = account_and_client(state.inner(), &input.account_id).await?;
    let calendars = refresh_calendar_list(state.inner(), &input.account_id, &client).await?;
    let calendar = if let Some(id) = input.calendar_id.as_deref() {
        calendars
            .iter()
            .find(|calendar| calendar.gcal_calendar_id == id)
    } else {
        calendars
            .iter()
            .find(|calendar| calendar.is_primary)
            .or_else(|| calendars.first())
    }
    .ok_or_else(|| AppError::NotFound("No writable Google calendar found".to_string()))?;

    let id = crate::email::gcal::generate_event_id();
    let mut event = NewEvent {
        id: id.clone(),
        summary: summary.to_string(),
        description: input.description.filter(|value| !value.trim().is_empty()),
        location: input.location.filter(|value| !value.trim().is_empty()),
        start: event_datetime(&input.start, &input.time_zone)?,
        end: event_datetime(&input.end, &input.time_zone)?,
        attendees: validate_attendees(&input.attendees)?,
        conference_data: None,
    };
    if event.start.date_time.is_some() && event.end.date_time.is_some() {
        let start = chrono::DateTime::parse_from_rfc3339(event.start.date_time.as_deref().unwrap())
            .map_err(|e| AppError::General(e.to_string()))?;
        let end = chrono::DateTime::parse_from_rfc3339(event.end.date_time.as_deref().unwrap())
            .map_err(|e| AppError::General(e.to_string()))?;
        if end <= start {
            return Err(AppError::General(
                "Event end must be after its start".to_string(),
            ));
        }
    }
    if input.add_meet {
        event.add_meet(format!("meet-{id}"));
    }
    let remote = match client
        .insert_event(&calendar.gcal_calendar_id, &event, SendUpdates::None)
        .await
    {
        Ok(remote) => remote,
        Err(AppError::CalendarApi(409, _)) => {
            client.get_event(&calendar.gcal_calendar_id, &id).await?
        }
        Err(error) => return Err(error),
    };
    let row_id = {
        let conn = state.db.safe_lock();
        let row_id = db::gcal::upsert_remote_event(
            &conn,
            calendar.id,
            &input.account_id,
            &calendar.gcal_calendar_id,
            &remote,
            &chrono::Utc::now().to_rfc3339(),
        )?;
        db::gcal::set_pending_notify(&conn, row_id, !event.attendees.is_empty())?;
        row_id
    };
    let conn = state.db.safe_lock();
    db::gcal::get_event(&conn, row_id)?
        .ok_or_else(|| AppError::NotFound("Created calendar event was not cached".to_string()))
}

#[tauri::command]
pub async fn update_google_calendar_event(
    state: State<'_, AppState>,
    event_id: i64,
    update: CalendarEventUpdate,
) -> Result<db::gcal::GcalEventRow, AppError> {
    let row = {
        let conn = state.db.safe_lock();
        db::gcal::get_event(&conn, event_id)?
            .ok_or_else(|| AppError::NotFound("Google Calendar event not found".to_string()))?
    };
    let (_account, client) = account_and_client(state.inner(), &row.account_id).await?;
    let zone = update
        .time_zone
        .as_deref()
        .or(row.start_tz.as_deref())
        .unwrap_or("UTC");
    let patch = EventPatch {
        summary: update.summary.map(|value| value.trim().to_string()),
        description: update.description,
        location: update.location,
        start: update
            .start
            .as_deref()
            .map(|value| event_datetime(value, zone))
            .transpose()?,
        end: update
            .end
            .as_deref()
            .map(|value| event_datetime(value, zone))
            .transpose()?,
        attendees: update
            .attendees
            .as_ref()
            .map(|items| validate_attendees(items))
            .transpose()?,
        conference_data: None,
    };
    let attendees_changed = patch.attendees.is_some();
    let remote = match client
        .patch_event(
            &row.gcal_calendar_id,
            &row.gcal_event_id,
            &patch,
            row.etag.as_deref(),
            SendUpdates::None,
        )
        .await
    {
        Ok(remote) => remote,
        Err(AppError::CalendarApi(412, _)) => {
            let fresh = client
                .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
                .await?;
            if gcal_sync::changes_are_disjoint(row.raw_json.as_deref(), &fresh, &patch) {
                client
                    .patch_event(
                        &row.gcal_calendar_id,
                        &row.gcal_event_id,
                        &patch,
                        fresh.etag.as_deref(),
                        SendUpdates::None,
                    )
                    .await?
            } else {
                let remote_json = serde_json::to_string(&fresh)
                    .map_err(|e| AppError::Parse(format!("Conflict serialization failed: {e}")))?;
                let conn = state.db.safe_lock();
                db::gcal::record_push_error(&conn, row.id, Some(&remote_json))?;
                return Err(AppError::General(
                    "This event changed in Google Calendar. Resolve the conflict before editing."
                        .to_string(),
                ));
            }
        }
        Err(error) => return Err(error),
    };
    let refreshed_id = {
        let conn = state.db.safe_lock();
        let id = db::gcal::upsert_remote_event(
            &conn,
            row.calendar_row_id,
            &row.account_id,
            &row.gcal_calendar_id,
            &remote,
            &chrono::Utc::now().to_rfc3339(),
        )?;
        if attendees_changed {
            db::gcal::set_pending_notify(&conn, id, true)?;
        }
        id
    };
    // Push the change to any Zoom meeting backing this event. Non-fatal by
    // design: the Google edit the user asked for has already landed, and the
    // background reconciler retries — failing the whole command here would make
    // a Zoom outage look like a broken calendar.
    //
    // Awaited BEFORE the read-back guard is taken; holding a `MutexGuard`
    // across an `.await` does not compile (it is `!Send`) and, worse, would
    // block every other DB reader for the duration of an HTTP call (gotcha #11).
    if let Err(error) = gcal_sync_zoom(state.inner(), refreshed_id).await {
        log::warn!("Zoom reconcile after calendar update failed: {error}");
    }
    let conn = state.db.safe_lock();
    db::gcal::get_event(&conn, refreshed_id)?
        .ok_or_else(|| AppError::NotFound("Updated calendar event was not cached".to_string()))
}

/// Thin wrapper so the Zoom hook reads the same at both call sites and the
/// non-fatal contract is stated in one place.
async fn gcal_sync_zoom(state: &AppState, gcal_event_row_id: i64) -> Result<(), AppError> {
    crate::email::zoom_sync::reconcile_event(&state.db, gcal_event_row_id).await
}

/// Delete a Google Calendar event, optionally telling its guests.
///
/// `notify` is **explicit and required** because this is one of the few actions
/// that puts real mail in other people's inboxes, and every such path in this
/// app is gated on a human decision (compose has undo-send, invitations have the
/// approval card). The caller has to have asked; there is no implicit default
/// that quietly emails a client's whole team.
#[tauri::command]
pub async fn delete_google_calendar_event(
    state: State<'_, AppState>,
    event_id: i64,
    notify: bool,
) -> Result<(), AppError> {
    let row = {
        let conn = state.db.safe_lock();
        db::gcal::get_event(&conn, event_id)?
            .ok_or_else(|| AppError::NotFound("Google Calendar event not found".to_string()))?
    };
    let (account, client) = account_and_client(state.inner(), &row.account_id).await?;
    // Tombstone the Zoom link BEFORE the Google delete. The mirror row is about
    // to be deleted, and with it the only join between the event and its
    // meeting — so a crash between the two calls would otherwise leave a real
    // Zoom meeting running with nothing at all pointing at it. Written first, the
    // tombstone survives and the reaper finishes the job.
    let zoom_link_id = {
        let conn = state.db.safe_lock();
        let link_id = db::zoom::get_by_local_event(&conn, row.id)
            .ok()
            .flatten()
            .map(|link| link.id);
        if let Some(link_id) = link_id {
            if let Err(error) = db::zoom::mark_delete_pending(&conn, link_id) {
                log::warn!("Could not tombstone the Zoom link for event {}: {error}", row.id);
            }
        }
        link_id
    };
    // Cancelling used to pass `SendUpdates::None`, so the meeting disappeared
    // from this calendar and stayed on everyone else's.
    //
    // Two independent conditions must BOTH hold before a cancellation notice
    // goes out, and the asymmetry is deliberate: the caller can always choose
    // silence, and can never force mail the rule would not send. Invariant #6 —
    // a frontend value that can only ever *reduce* the blast radius is safe to
    // honour; one that could widen it is not. So a compromised or buggy caller
    // passing `notify: true` on an event nobody was ever told about still sends
    // nothing.
    let cancel_updates = {
        let conn = state.db.safe_lock();
        let notified = db::invite_notifications::notified_emails(
            &conn,
            &row.account_id,
            &row.gcal_calendar_id,
            &row.gcal_event_id,
        )
        .unwrap_or_default();
        let delivery = db::invite_notifications::derive(
            row.attendees_json.as_deref(),
            Some(&notified),
            row.pending_notify,
        );
        if db::invite_notifications::cancellation_send_updates(notify, &delivery) {
            SendUpdates::All
        } else {
            SendUpdates::None
        }
    };
    match client
        .delete_event(&row.gcal_calendar_id, &row.gcal_event_id, cancel_updates)
        .await
    {
        Ok(()) | Err(AppError::CalendarApi(404 | 410, _)) => {
            {
                let conn = state.db.safe_lock();
                db::gcal::delete_local(&conn, row.id)?;
            }
            // Reap now so the meeting disappears from zoom.us with the event
            // rather than on the next tick. A failure leaves the tombstone in
            // place — it is retried every 30 seconds until Zoom accepts it.
            if let Err(error) = crate::email::zoom_sync::flush_pending_deletes(&state.db).await {
                log::warn!("Zoom delete reaper failed after calendar delete: {error}");
            }
            Ok(())
        }
        // 403 means someone else organizes this event, so "delete" degrades to
        // "decline as an attendee" — the event goes on existing and so must its
        // conferencing. In practice CXMail never attaches a Zoom meeting to an
        // event it does not organize, so there should be nothing to undo; the
        // revert is here so that guarantee is structural rather than argued.
        Err(AppError::CalendarApi(403, _)) => {
            if let Some(link_id) = zoom_link_id {
                let conn = state.db.safe_lock();
                if let Err(error) = db::zoom::clear_delete_pending(&conn, link_id) {
                    log::error!(
                        "Declined event {} but could not revert its Zoom tombstone ({error}) — the \
                         meeting may be deleted by the reaper while the event still exists",
                        row.id
                    );
                }
            }
            let mut fresh = client
                .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
                .await?;
            let attendees = fresh.attendees.get_or_insert_with(Vec::new);
            let Some(self_attendee) = attendees.iter_mut().find(|attendee| {
                attendee.self_attendee
                    || attendee
                        .email
                        .as_deref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(&account.email))
            }) else {
                return Err(AppError::General(
                    "Only the organizer can delete this event, and CXMail could not identify your attendee record to decline it".to_string(),
                ));
            };
            self_attendee.response_status = Some("declined".to_string());
            let remote = client
                .patch_event(
                    &row.gcal_calendar_id,
                    &row.gcal_event_id,
                    &EventPatch {
                        attendees: Some(attendees.clone()),
                        ..Default::default()
                    },
                    fresh.etag.as_deref(),
                    // A decline that notifies nobody is not a decline: here we
                    // are an attendee, not the organizer, so the ledger rule
                    // does not apply — the organizer demonstrably knows about
                    // their own event, and telling them we are not coming is the
                    // entire content of the action. It still honours an explicit
                    // `notify: false`, because overriding a deliberate "do not
                    // email anyone" is not ours to do; that is the same choice
                    // Google's own UI offers.
                    if notify {
                        SendUpdates::All
                    } else {
                        SendUpdates::None
                    },
                )
                .await?;
            let conn = state.db.safe_lock();
            db::gcal::upsert_remote_event(
                &conn,
                row.calendar_row_id,
                &row.account_id,
                &row.gcal_calendar_id,
                &remote,
                &chrono::Utc::now().to_rfc3339(),
            )?;
            Ok(())
        }
        // The Google delete failed for some other reason, so the event still
        // exists. Revert the tombstone or the reaper would delete a meeting that
        // is still attached to a live event — the exact mistake the
        // positive-evidence rule exists to prevent.
        Err(error) => {
            if let Some(link_id) = zoom_link_id {
                let conn = state.db.safe_lock();
                if let Err(revert) = db::zoom::clear_delete_pending(&conn, link_id) {
                    log::error!(
                        "Calendar delete for event {} failed ({error}) and its Zoom tombstone could \
                         not be reverted ({revert}) — the meeting may be deleted while the event \
                         still exists",
                        row.id
                    );
                }
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn send_google_calendar_invites(
    state: State<'_, AppState>,
    event_id: i64,
) -> Result<db::gcal::GcalEventRow, AppError> {
    let row = {
        let conn = state.db.safe_lock();
        db::gcal::get_event(&conn, event_id)?
            .ok_or_else(|| AppError::NotFound("Google Calendar event not found".to_string()))?
    };
    let (_account, client) = account_and_client(state.inner(), &row.account_id).await?;
    // Invitation delivery and RSVP-style attendee writes must use the server's
    // current complete attendee array. Sending a stale cached array can silently
    // remove co-attendees.
    let fresh = client
        .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
        .await?;
    let attendees = fresh.attendees.clone().unwrap_or_default();
    if attendees.is_empty() {
        return Err(AppError::General(
            "This event has no attendees to notify".to_string(),
        ));
    }
    // Capture who we are about to have Google notify, before the list is moved
    // into the patch. This is the only moment the fact exists anywhere: Google
    // records nothing about invitation delivery on the event, so if it is not
    // written down here it is unrecoverable (see `db::invite_notifications`).
    let notified: Vec<String> = attendees
        .iter()
        .filter_map(|attendee| attendee.email.clone())
        .collect();
    let remote = client
        .patch_event(
            &row.gcal_calendar_id,
            &row.gcal_event_id,
            &EventPatch {
                summary: row.summary.clone(),
                attendees: Some(attendees),
                ..Default::default()
            },
            fresh.etag.as_deref(),
            SendUpdates::All,
        )
        .await?;
    let refreshed_id = {
        let conn = state.db.safe_lock();
        let id = db::gcal::upsert_remote_event(
            &conn,
            row.calendar_row_id,
            &row.account_id,
            &row.gcal_calendar_id,
            &remote,
            &chrono::Utc::now().to_rfc3339(),
        )?;
        db::gcal::set_pending_notify(&conn, id, false)?;
        // Strictly after the 2xx above — a ledger row must mean "Google took the
        // send", never "we intended to send".
        db::invite_notifications::record_notified(
            &conn,
            &row.account_id,
            &row.gcal_calendar_id,
            &row.gcal_event_id,
            &notified,
            "manual",
        )?;
        id
    };
    let conn = state.db.safe_lock();
    db::gcal::get_event(&conn, refreshed_id)?
        .ok_or_else(|| AppError::NotFound("Notified calendar event was not cached".to_string()))
}

#[tauri::command]
pub async fn sync_google_calendar_now(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };
    gcal_sync::sync_account(&state.db, &account.id, &account.email).await
}

#[tauri::command]
pub async fn resolve_google_calendar_conflict(
    state: State<'_, AppState>,
    event_id: i64,
    resolution: String,
) -> Result<(), AppError> {
    {
        let conn = state.db.safe_lock();
        match resolution.as_str() {
            "mine" => db::gcal::mark_keep_mine(&conn, event_id)?,
            "theirs" => return db::gcal::resolve_conflict_use_theirs(&conn, event_id),
            _ => {
                return Err(AppError::General(
                    "Conflict resolution must be 'mine' or 'theirs'".to_string(),
                ))
            }
        }
    }
    let row = {
        let conn = state.db.safe_lock();
        db::gcal::get_event(&conn, event_id)?
            .ok_or_else(|| AppError::NotFound("Google Calendar event not found".to_string()))?
    };
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &row.account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?
    };
    gcal_sync::sync_account(&state.db, &account.id, &account.email).await
}
