use crate::db;
use crate::email::gcal::{
    EventAttendee, EventDateTime, EventPatch, GcalClient, ListQuery, NewEvent, SendUpdates,
};
use crate::email::oauth2;
use crate::error::AppError;
use crate::LockExt;
use chrono::{Duration, SecondsFormat, Utc};
use std::sync::Mutex;

/// True when the Calendar API rejected the call because the stored grant is
/// missing a scope, rather than for any transient reason.
fn is_insufficient_scope(error: &AppError) -> bool {
    match error {
        AppError::CalendarApi(403, message) => {
            let m = message.to_ascii_lowercase();
            m.contains("insufficient") && m.contains("scope")
        }
        _ => false,
    }
}

/// A scope 403 is terminal, not transient: the stored grant can never satisfy
/// the call, so the default retry-every-two-minutes behaviour just buries a
/// warning nobody reads (it logged once per account per tick, indefinitely).
/// Invalidating the grant drops the account out of the sync loop and flips the
/// UI back to "Connect", which is the only thing that actually fixes it.
fn handle_sync_error(email: &str, error: &AppError) {
    if is_insufficient_scope(error) {
        log::error!(
            "Google Calendar grant for {email} is missing a required scope, so it has been \
             marked disconnected. Add the scopes under Data Access in the Google Cloud console, \
             then reconnect. ({error})"
        );
        if let Err(e) = oauth2::invalidate_gcal_connection(email) {
            log::warn!("Failed to invalidate Calendar grant for {email}: {e}");
        }
    } else {
        log::warn!("Google Calendar sync failed for {email}: {error}");
    }
}

pub async fn sync_all(db_conn: &Mutex<rusqlite::Connection>) -> Result<(), AppError> {
    let accounts = {
        let conn = db_conn.safe_lock();
        db::accounts::list(&conn)?
    };
    for account in accounts
        .into_iter()
        .filter(|account| account.provider == "gmail" && account.is_active)
    {
        if !oauth2::is_gcal_connected(&account.email)? {
            continue;
        }
        if let Err(error) = sync_account(db_conn, &account.id, &account.email).await {
            handle_sync_error(&account.email, &error);
        }
    }
    Ok(())
}

/// Flush local Calendar mutations without doing a remote pull. The app calls
/// this on every 30-second scheduler tick; the slower two-minute pass uses
/// [`sync_all`] so it still preserves the push-before-pull ordering.
pub async fn push_all(db_conn: &Mutex<rusqlite::Connection>) -> Result<(), AppError> {
    let accounts = {
        let conn = db_conn.safe_lock();
        db::accounts::list(&conn)?
    };
    for account in accounts
        .into_iter()
        .filter(|account| account.provider == "gmail" && account.is_active)
    {
        if !oauth2::is_gcal_connected(&account.email)? {
            continue;
        }
        let client = match GcalClient::for_account(&account.email).await {
            Ok(client) => client,
            Err(error) => {
                log::warn!(
                    "Google Calendar token refresh failed for {}: {}",
                    account.email,
                    error
                );
                continue;
            }
        };
        if let Err(error) = push_account(db_conn, &account.id, &client).await {
            handle_sync_error(&account.email, &error);
        }
    }
    Ok(())
}

pub async fn sync_account(
    db_conn: &Mutex<rusqlite::Connection>,
    account_id: &str,
    email: &str,
) -> Result<(), AppError> {
    let client = GcalClient::for_account(email).await?;
    push_account(db_conn, account_id, &client).await?;
    let calendars = client.list_calendars().await?;
    {
        let conn = db_conn.safe_lock();
        for calendar in &calendars {
            db::gcal::upsert_calendar(&conn, account_id, calendar)?;
        }
    }
    let selected = {
        let conn = db_conn.safe_lock();
        db::gcal::list_calendars(&conn, account_id)?
            .into_iter()
            .filter(|calendar| calendar.selected)
            .collect::<Vec<_>>()
    };
    for calendar in selected {
        if let Err(error) = pull_calendar(db_conn, &client, &calendar).await {
            let conn = db_conn.safe_lock();
            let _ = db::gcal::set_sync_error(&conn, calendar.id, &error.to_string());
            return Err(error);
        }
    }
    {
        let conn = db_conn.safe_lock();
        let _ = db::gcal::purge_old_cancelled(&conn);
    }
    Ok(())
}

async fn pull_calendar(
    db_conn: &Mutex<rusqlite::Connection>,
    client: &GcalClient,
    calendar: &db::gcal::GcalCalendarRow,
) -> Result<(), AppError> {
    let run_started = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut token = calendar.sync_token.clone();
    let mut full = token.is_none();
    let mut retried_after_expiry = false;

    loop {
        let mut page_token = None;
        let mut next_sync_token = None;
        loop {
            let query = if full {
                ListQuery {
                    time_min: Some(
                        (Utc::now() - Duration::days(calendar.sync_window_days as i64))
                            .to_rfc3339_opts(SecondsFormat::Secs, true),
                    ),
                    page_token: page_token.clone(),
                    show_deleted: false,
                    single_events: true,
                    max_results: Some(250),
                    ..Default::default()
                }
            } else {
                ListQuery {
                    sync_token: token.clone(),
                    page_token: page_token.clone(),
                    show_deleted: true,
                    single_events: true,
                    max_results: Some(250),
                    ..Default::default()
                }
            };
            let page = match client.list_events(&calendar.gcal_calendar_id, query).await {
                Ok(page) => page,
                Err(AppError::CalendarApi(410, _)) if !retried_after_expiry => {
                    retried_after_expiry = true;
                    full = true;
                    token = None;
                    let conn = db_conn.safe_lock();
                    db::gcal::set_sync_token(&conn, calendar.id, None)?;
                    break;
                }
                Err(error) => return Err(error),
            };
            {
                let conn = db_conn.safe_lock();
                for event in &page.items {
                    db::gcal::upsert_remote_event(
                        &conn,
                        calendar.id,
                        &calendar.account_id,
                        &calendar.gcal_calendar_id,
                        event,
                        &run_started,
                    )?;
                }
            }
            page_token = page.next_page_token;
            if let Some(value) = page.next_sync_token {
                next_sync_token = Some(value);
            }
            if page_token.is_none() {
                let conn = db_conn.safe_lock();
                if full {
                    db::gcal::sweep_full_sync(&conn, calendar.id, &run_started)?;
                }
                db::gcal::set_sync_token(&conn, calendar.id, next_sync_token.as_deref())?;
                return Ok(());
            }
        }
        if !full || !retried_after_expiry {
            return Ok(());
        }
    }
}

async fn push_account(
    db_conn: &Mutex<rusqlite::Connection>,
    account_id: &str,
    client: &GcalClient,
) -> Result<(), AppError> {
    let dirty = {
        let conn = db_conn.safe_lock();
        db::gcal::list_dirty_for_account(&conn, account_id)?
    };
    for row in dirty {
        let result = match row.sync_state.as_str() {
            "local_new" => {
                let mut event = new_event_from_row(&row)?;
                if row.conference_json.is_some() {
                    event.add_meet(format!("meet-{}", row.gcal_event_id));
                }
                client
                    .insert_event(&row.gcal_calendar_id, &event, SendUpdates::None)
                    .await
            }
            "local_dirty" => {
                let patch = patch_from_row(&row)?;
                client
                    .patch_event(
                        &row.gcal_calendar_id,
                        &row.gcal_event_id,
                        &patch,
                        row.etag.as_deref(),
                        SendUpdates::None,
                    )
                    .await
            }
            "local_deleted" => {
                // Dormant today — nothing writes `local_deleted` (gotcha #44) —
                // but it is left correct rather than as a landmine for whoever
                // resurrects it. Same rule as the interactive delete: notify
                // unless the ledger positively says nobody was ever told.
                let cancel_updates = {
                    let conn = db_conn.safe_lock();
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
                    // Nothing asked for silence here, so the rule alone decides.
                    if db::invite_notifications::cancellation_send_updates(true, &delivery) {
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
                        let conn = db_conn.safe_lock();
                        db::gcal::delete_local(&conn, row.id)?;
                        continue;
                    }
                    Err(error) => Err(error),
                }
            }
            _ => continue,
        };

        match result {
            Ok(remote) => {
                let conn = db_conn.safe_lock();
                db::gcal::prepare_push_success(&conn, row.id)?;
                db::gcal::upsert_remote_event(
                    &conn,
                    row.calendar_row_id,
                    &row.account_id,
                    &row.gcal_calendar_id,
                    &remote,
                    &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                )?;
            }
            Err(AppError::CalendarApi(412, _)) => {
                let remote = client
                    .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
                    .await?;
                if row.sync_state == "local_dirty"
                    && changes_are_disjoint(
                        row.raw_json.as_deref(),
                        &remote,
                        &patch_from_row(&row)?,
                    )
                {
                    let retried = client
                        .patch_event(
                            &row.gcal_calendar_id,
                            &row.gcal_event_id,
                            &patch_from_row(&row)?,
                            remote.etag.as_deref(),
                            SendUpdates::None,
                        )
                        .await?;
                    let conn = db_conn.safe_lock();
                    db::gcal::prepare_push_success(&conn, row.id)?;
                    db::gcal::upsert_remote_event(
                        &conn,
                        row.calendar_row_id,
                        &row.account_id,
                        &row.gcal_calendar_id,
                        &retried,
                        &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                    )?;
                    continue;
                }
                let remote_json = serde_json::to_string(&remote)
                    .map_err(|e| AppError::Parse(format!("Conflict serialization failed: {e}")))?;
                let conn = db_conn.safe_lock();
                db::gcal::record_push_error(&conn, row.id, Some(&remote_json))?;
            }
            Err(AppError::CalendarApi(404 | 410, _)) => {
                let conn = db_conn.safe_lock();
                db::gcal::delete_local(&conn, row.id)?;
            }
            Err(error) => {
                let conn = db_conn.safe_lock();
                db::gcal::record_push_error(&conn, row.id, None)?;
                log::warn!(
                    "Google Calendar push failed for event {}: {}",
                    row.id,
                    error
                );
            }
        }
    }
    Ok(())
}

pub fn changes_are_disjoint(
    base_json: Option<&str>,
    remote: &crate::email::gcal::GcalEvent,
    local_patch: &EventPatch,
) -> bool {
    let Some(base_json) = base_json else {
        return false;
    };
    let Ok(base) = serde_json::from_str::<serde_json::Value>(base_json) else {
        return false;
    };
    let Ok(remote) = serde_json::to_value(remote) else {
        return false;
    };
    let Ok(local) = serde_json::to_value(local_patch) else {
        return false;
    };
    let Some(local_object) = local.as_object() else {
        return false;
    };
    let fields = [
        "summary",
        "description",
        "location",
        "start",
        "end",
        "attendees",
        "conferenceData",
    ];
    let local_changed = fields
        .iter()
        .filter(|field| {
            local_object
                .get(**field)
                .is_some_and(|value| value != &base[**field])
        })
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let remote_changed = fields
        .iter()
        .filter(|field| remote[**field] != base[**field])
        .copied()
        .collect::<std::collections::HashSet<_>>();
    local_changed.is_disjoint(&remote_changed)
}

fn row_datetime(value: &str, timezone: Option<&str>) -> EventDateTime {
    if value.len() == 10 {
        EventDateTime {
            date: Some(value.to_string()),
            date_time: None,
            time_zone: timezone.map(str::to_string),
        }
    } else {
        EventDateTime {
            date_time: Some(value.to_string()),
            date: None,
            time_zone: timezone.map(str::to_string),
        }
    }
}

fn attendees(row: &db::gcal::GcalEventRow) -> Result<Vec<EventAttendee>, AppError> {
    row.attendees_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| AppError::Parse(format!("Invalid attendee data: {e}")))
        .map(|value| value.unwrap_or_default())
}

fn new_event_from_row(row: &db::gcal::GcalEventRow) -> Result<NewEvent, AppError> {
    Ok(NewEvent {
        id: row.gcal_event_id.clone(),
        summary: row.summary.clone().unwrap_or_else(|| "Event".to_string()),
        description: row.description.clone(),
        location: row.location.clone(),
        start: row_datetime(&row.dtstart, row.start_tz.as_deref()),
        end: row_datetime(
            row.dtend.as_deref().unwrap_or(&row.dtstart),
            row.end_tz.as_deref().or(row.start_tz.as_deref()),
        ),
        attendees: attendees(row)?,
        conference_data: None,
    })
}

fn patch_from_row(row: &db::gcal::GcalEventRow) -> Result<EventPatch, AppError> {
    Ok(EventPatch {
        summary: row.summary.clone(),
        description: row.description.clone(),
        location: row.location.clone(),
        start: Some(row_datetime(&row.dtstart, row.start_tz.as_deref())),
        end: row
            .dtend
            .as_deref()
            .map(|value| row_datetime(value, row.end_tz.as_deref())),
        attendees: Some(attendees(row)?),
        conference_data: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_way_change_sets_only_auto_merge_when_disjoint() {
        let base = serde_json::json!({
            "summary": "Old",
            "location": "Room A",
            "start": {"dateTime":"2026-07-30T16:30:00-04:00"}
        });
        let remote = crate::email::gcal::GcalEvent {
            summary: Some("Remote title".to_string()),
            location: Some("Room A".to_string()),
            start: EventDateTime {
                date_time: Some("2026-07-30T16:30:00-04:00".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let local_location = EventPatch {
            location: Some("Room B".to_string()),
            ..Default::default()
        };
        assert!(changes_are_disjoint(
            Some(&base.to_string()),
            &remote,
            &local_location
        ));

        let local_title = EventPatch {
            summary: Some("Local title".to_string()),
            ..Default::default()
        };
        assert!(!changes_are_disjoint(
            Some(&base.to_string()),
            &remote,
            &local_title
        ));
    }
}

#[cfg(test)]
mod scope_failure_tests {
    use super::*;

    #[test]
    fn googles_insufficient_scope_403_is_terminal() {
        let error = AppError::CalendarApi(
            403,
            "Request had insufficient authentication scopes.".to_string(),
        );
        assert!(is_insufficient_scope(&error));
    }

    #[test]
    fn other_403s_are_not_treated_as_scope_failures() {
        // A quota or permission 403 is transient or user-specific; invalidating
        // the grant there would sign the account out for the wrong reason.
        let quota = AppError::CalendarApi(403, "Rate Limit Exceeded".to_string());
        assert!(!is_insufficient_scope(&quota));
    }

    #[test]
    fn non_403_statuses_are_never_scope_failures() {
        assert!(!is_insufficient_scope(&AppError::CalendarApi(
            410,
            "Sync token is no longer valid".to_string()
        )));
        assert!(!is_insufficient_scope(&AppError::CalendarApi(
            412,
            "Precondition Failed".to_string()
        )));
    }
}
