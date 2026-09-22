//! The invitation-delivery ledger: which attendees CXMail actually asked Google
//! to notify, and when.
//!
//! **Google will not tell us.** The Calendar API's attendee object carries
//! exactly five fields — `email`, `displayName`, `organizer`, `self`,
//! `responseStatus` — and not one of them records whether an invitation was ever
//! emailed. Worse, `responseStatus` is stamped `needsAction` the instant an
//! attendee is attached, so on the organizer's own calendar *"invited, hasn't
//! replied yet"* and *"never told, has no idea"* render identically. That is how
//! an event sits on your calendar looking published, with recipients attached,
//! that nobody ever received: adding an attendee is a field write, and
//! notification is a separate side effect governed by `sendUpdates`, whose API
//! default is `none`.
//!
//! Since the fact cannot be read back, it has to be *written down* at the moment
//! we make the call. This table is that record, and it exists only because
//! `clear_invite_approval` deliberately destroys the approval snapshot on every
//! terminal path — correct for its own purpose (single-use approval, so one
//! decision cannot deliver twice) but it means the receipt was being shredded
//! the instant it was earned.
//!
//! **A side table keyed on the remote triple, with no foreign keys** — the same
//! shape and the same reasoning as `zoom_meetings` (see
//! `db::schema::migrate_v52_zoom_meetings` and gotcha #44). A column on
//! `gcal_events` fails twice over: `upsert_remote_event` is driven entirely by
//! Google's payload, so every sync tick would null it, and four paths DELETE
//! from that table, so the record would die with the mirror row. A ledger row
//! whose event has been swept is a tombstone, not garbage — it is the evidence
//! that someone was told.
//!
//! There is **no backfill**, deliberately. The fact is unreconstructible from
//! anything stored (same situation as gotcha #37), so every attendee that
//! predates this table reports `unknown`. An honest `unknown` is the point;
//! painting history as `sent` would rebuild the exact lie this replaces.

use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// What we can honestly say about one attendee.
///
/// Ordered by strength of evidence. Only `RESPONDED` is proof of *receipt* —
/// everything else describes what we did, not what reached anyone.
pub mod state {
    /// The attendee accepted, declined, or marked tentative. Physical proof they
    /// received the invitation: a response cannot exist without one.
    pub const RESPONDED: &str = "responded";
    /// We asked Google to notify this address and Google accepted the request.
    /// **Not** a delivery receipt — says nothing about bounces, spam filing, or
    /// tenant-level quarantine. Label it "sent", never "received".
    pub const SENT: &str = "sent";
    /// This attendee is on the guest list and we have positive reason to believe
    /// they were never announced: either the event has a ledger (so we know what
    /// a send looks like here and they are not in it) or it is flagged as having
    /// un-notified attendees.
    pub const UNSENT: &str = "unsent";
    /// We have no record either way. The overwhelmingly common cause is an event
    /// created or edited in Google Calendar's own web UI, where the Send /
    /// Don't-send choice happens somewhere CXMail cannot observe.
    pub const UNKNOWN: &str = "unknown";
}

/// One attendee's delivery state, resolved for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttendeeDelivery {
    pub email: String,
    pub display_name: Option<String>,
    /// Google's raw `responseStatus` (`needsAction`, `accepted`, `declined`,
    /// `tentative`), passed through untouched so the UI can distinguish an
    /// acceptance from a decline.
    pub response_status: Option<String>,
    /// Whether this attendee is the local user.
    pub is_self: bool,
    /// One of [`state`].
    pub state: String,
}

/// Trim and lowercase, so a ledger write and a later lookup cannot disagree over
/// spelling. Mirrors what `email::gcal_invite::normalize_recipients` does to the
/// approval snapshot; kept independent because `cxmail-db` sits *below*
/// `cxmail-email` in the dependency order and must not reach up into it.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Record that Google accepted a request to notify these addresses.
///
/// Call this **only after a 2xx**. The whole value of the ledger is that a row
/// means "Google took the send", so writing optimistically before the call
/// would reintroduce exactly the ambiguity it exists to remove.
///
/// Idempotent: re-notifying the same attendee refreshes `notified_at` rather
/// than stacking rows, so the ledger answers "when were they last told".
pub fn record_notified(
    conn: &Connection,
    account_id: &str,
    gcal_calendar_id: &str,
    gcal_event_id: &str,
    emails: &[String],
    source: &str,
) -> Result<usize, AppError> {
    let mut written = 0usize;
    for email in emails {
        let normalized = normalize_email(email);
        if normalized.is_empty() {
            continue;
        }
        written += conn.execute(
            "INSERT INTO invite_notifications
                (account_id, gcal_calendar_id, gcal_event_id, attendee_email, source)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id, gcal_calendar_id, gcal_event_id, attendee_email)
             DO UPDATE SET notified_at=datetime('now'), source=excluded.source",
            params![account_id, gcal_calendar_id, gcal_event_id, normalized, source],
        )?;
    }
    Ok(written)
}

/// Every address we have notified for one event.
pub fn notified_emails(
    conn: &Connection,
    account_id: &str,
    gcal_calendar_id: &str,
    gcal_event_id: &str,
) -> Result<HashSet<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT attendee_email FROM invite_notifications
         WHERE account_id=?1 AND gcal_calendar_id=?2 AND gcal_event_id=?3",
    )?;
    let rows = stmt
        .query_map(params![account_id, gcal_calendar_id, gcal_event_id], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

/// The whole ledger for an account (or every account when `None`), keyed by the
/// remote triple.
///
/// One query for a range of events rather than one per event: the calendar view
/// renders a month at a time, and a per-event lookup there is a query storm held
/// under the DB mutex (gotcha #11).
pub type LedgerIndex = HashMap<(String, String, String), HashSet<String>>;

/// Whether the ledger table exists at all.
///
/// The MCP binary runs migrations on a non-fatal path, so a database that never
/// reached v57 is reachable at runtime (the same hazard gotcha #26 guards for
/// `messages_fts`). This read sits on `list_unified_by_range`, so an error here
/// would refuse to render the entire calendar over a missing bookkeeping table.
/// Degrading is also semantically right rather than a papering-over: no ledger
/// means no record, which is precisely what `unknown` says.
fn ledger_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='invite_notifications'",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn notified_by_event(
    conn: &Connection,
    account_id: Option<&str>,
) -> Result<LedgerIndex, AppError> {
    if !ledger_exists(conn) {
        return Ok(LedgerIndex::new());
    }
    let mut stmt = conn.prepare(
        "SELECT account_id, gcal_calendar_id, gcal_event_id, attendee_email
         FROM invite_notifications
         WHERE (?1 IS NULL OR account_id=?1)",
    )?;
    let mut index: LedgerIndex = HashMap::new();
    let rows = stmt.query_map(params![account_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (account, calendar, event, email) = row?;
        index
            .entry((account, calendar, event))
            .or_default()
            .insert(email);
    }
    Ok(index)
}

/// Resolve every attendee on an event to a display state.
///
/// Pure — no DB, no network — so the four-state rule is unit-testable and lives
/// in exactly one place. Deriving it a second time in TypeScript is how the
/// sidebar and the detail panel end up disagreeing (gotcha #36's one-matcher
/// rule), so the frontend renders what this returns and decides nothing.
///
/// `event_has_ledger` is what separates `UNSENT` from `UNKNOWN`: if we have ever
/// notified anyone for this event, we know what a send looks like here, so an
/// attendee absent from the ledger was genuinely added afterwards and left out.
/// With no ledger at all and nothing flagged, we simply do not know.
pub fn derive(
    attendees_json: Option<&str>,
    notified: Option<&HashSet<String>>,
    pending_notify: bool,
) -> Vec<AttendeeDelivery> {
    let Some(raw) = attendees_json else {
        return Vec::new();
    };
    let Ok(attendees) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    let event_has_ledger = notified.is_some_and(|set| !set.is_empty());

    attendees
        .iter()
        .filter_map(|attendee| {
            let email = attendee.get("email")?.as_str()?.trim();
            if email.is_empty() {
                return None;
            }
            let normalized = normalize_email(email);
            let response_status = attendee
                .get("responseStatus")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let is_self = attendee
                .get("self")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);

            let has_responded = response_status
                .as_deref()
                .is_some_and(|status| !status.eq_ignore_ascii_case("needsAction"));

            let resolved = if has_responded {
                state::RESPONDED
            } else if notified.is_some_and(|set| set.contains(&normalized)) {
                state::SENT
            } else if event_has_ledger || pending_notify {
                state::UNSENT
            } else {
                state::UNKNOWN
            };

            Some(AttendeeDelivery {
                email: email.to_string(),
                display_name: attendee
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                response_status,
                is_self,
                state: resolved.to_string(),
            })
        })
        .collect()
}

/// Whether cancelling this event should send cancellation notices.
///
/// Deleting an event used to pass `SendUpdates::None`, so a meeting vanished
/// from the organizer's calendar and stayed on everyone else's — the same defect
/// as the create path, one step worse because the guests who *were* told are
/// exactly the ones left holding a dead slot.
///
/// **Fail safe means notify.** The rule is "notify unless we have positive
/// evidence nobody was ever told": only an event where *every* guest resolves to
/// [`state::UNSENT`] stays silent, because a cancellation there would be the
/// first any of them ever heard of the meeting. `UNKNOWN` deliberately counts as
/// "might know" — most of a pre-v57 calendar is unknown, and the harm of a
/// surprising cancellation notice is trivial beside the harm of someone holding
/// time for a meeting that no longer exists.
///
/// An event with no guests returns false: there is nobody to tell, and passing
/// `All` there would be a claim the code does not mean.
pub fn cancellation_should_notify(delivery: &[AttendeeDelivery]) -> bool {
    let others: Vec<&AttendeeDelivery> = delivery.iter().filter(|a| !a.is_self).collect();
    if others.is_empty() {
        return false;
    }
    !others.iter().all(|a| a.state == state::UNSENT)
}

/// Whether a delete should actually send cancellation notices, given what the
/// caller asked for and what the ledger knows.
///
/// **The asymmetry is the invariant.** A caller can always choose silence, and
/// can never force mail that [`cancellation_should_notify`] would not send. That
/// is what makes it safe to honour a value arriving from the frontend at all
/// (architecture invariant #6): it can only ever *reduce* the blast radius, so a
/// buggy or hostile caller passing `true` on an event nobody was ever told about
/// still sends nothing. Inverting either half is a way to email a client's whole
/// team by accident, so both directions are test-pinned.
pub fn cancellation_send_updates(requested_notify: bool, delivery: &[AttendeeDelivery]) -> bool {
    requested_notify && cancellation_should_notify(delivery)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate_v57_invite_notifications(&conn).unwrap();
        conn
    }

    fn attendees(entries: &[(&str, &str)]) -> String {
        let list = entries
            .iter()
            .map(|(email, status)| {
                serde_json::json!({ "email": email, "responseStatus": status })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&list).unwrap()
    }

    #[test]
    fn recording_is_idempotent_and_normalizes_spelling() {
        let conn = memory_db();
        let emails = vec!["Dana@Northwind.example ".to_string(), "sam@harborline.example".to_string()];
        record_notified(&conn, "acct", "primary", "evt", &emails, "cxmail").unwrap();
        record_notified(&conn, "acct", "primary", "evt", &emails, "cxmail").unwrap();

        let stored = notified_emails(&conn, "acct", "primary", "evt").unwrap();
        assert_eq!(stored.len(), 2, "re-notifying must refresh, not stack rows");
        assert!(
            stored.contains("dana@northwind.example"),
            "a write and a later lookup must agree on spelling"
        );
    }

    #[test]
    fn a_response_outranks_the_ledger_because_it_is_proof_of_receipt() {
        // Nothing recorded, yet the attendee accepted: they demonstrably got it.
        // Reporting `unknown` here would hide the strongest evidence we ever get.
        let resolved = derive(
            Some(&attendees(&[("sam@harborline.example", "accepted")])),
            None,
            false,
        );
        assert_eq!(resolved[0].state, state::RESPONDED);
    }

    #[test]
    fn an_attendee_added_after_a_send_reads_unsent_not_unknown() {
        // The Aug 14 shape: five people invited, four more attached half an hour
        // before the meeting and never announced.
        let mut notified = HashSet::new();
        notified.insert("dana@northwind.example".to_string());

        let resolved = derive(
            Some(&attendees(&[
                ("dana@northwind.example", "needsAction"),
                ("jkim@ironside.example", "needsAction"),
            ])),
            Some(&notified),
            false,
        );

        assert_eq!(resolved[0].state, state::SENT);
        assert_eq!(
            resolved[1].state,
            state::UNSENT,
            "the event has a ledger, so absence from it is evidence, not ignorance"
        );
    }

    #[test]
    fn no_ledger_and_nothing_flagged_is_unknown_never_sent() {
        // An event created in Google Calendar's web UI: the Send / Don't-send
        // choice happened where CXMail could not see it. Claiming either answer
        // would be inventing evidence.
        let resolved = derive(
            Some(&attendees(&[("riley@northwind.example", "needsAction")])),
            None,
            false,
        );
        assert_eq!(resolved[0].state, state::UNKNOWN);
    }

    #[test]
    fn pending_notify_alone_is_enough_to_say_unsent() {
        // Created by CXMail with sendUpdates=none and never approved: no ledger
        // exists, but the flag is positive evidence nobody was told.
        let resolved = derive(
            Some(&attendees(&[("morgan@catalystpartners.example", "needsAction")])),
            None,
            true,
        );
        assert_eq!(resolved[0].state, state::UNSENT);
    }

    #[test]
    fn malformed_attendee_json_yields_no_rows_rather_than_a_wrong_answer() {
        assert!(derive(Some("{not json"), None, true).is_empty());
        assert!(derive(None, None, true).is_empty());
        // An attendee with no address cannot be matched against the ledger, so it
        // must not be rendered with a confident-looking state.
        let anonymous = serde_json::json!([{ "responseStatus": "needsAction" }]).to_string();
        assert!(derive(Some(&anonymous), None, true).is_empty());
    }

    #[test]
    fn a_database_without_the_ledger_table_degrades_instead_of_erroring() {
        // Reachable in the wild: the MCP migrates non-fatally, so a pre-v57
        // database can be live. This read hangs off `list_unified_by_range`, so
        // returning an error here would refuse to draw the whole calendar over a
        // missing bookkeeping table — and "no ledger" already has an honest
        // rendering, which is `unknown`.
        let conn = Connection::open_in_memory().unwrap();
        let index = notified_by_event(&conn, None).unwrap();
        assert!(index.is_empty());

        let resolved = derive(
            Some(&attendees(&[("dana@northwind.example", "needsAction")])),
            index.get(&("a".into(), "b".into(), "c".into())),
            false,
        );
        assert_eq!(resolved[0].state, state::UNKNOWN);
    }

    fn guest(email: &str, state: &str) -> AttendeeDelivery {
        AttendeeDelivery {
            email: email.to_string(),
            display_name: None,
            response_status: None,
            is_self: false,
            state: state.to_string(),
        }
    }

    #[test]
    fn cancelling_notifies_unless_provably_nobody_was_ever_told() {
        // Somebody knows: they must be told it is off.
        assert!(cancellation_should_notify(&[
            guest("a@x.com", state::SENT),
            guest("b@x.com", state::UNSENT),
        ]));
        assert!(cancellation_should_notify(&[guest(
            "a@x.com",
            state::RESPONDED
        )]));

        // `unknown` counts as "might know" — fail safe is to notify. Most of a
        // pre-v57 calendar is unknown, and silence there recreates the bug.
        assert!(cancellation_should_notify(&[guest(
            "a@x.com",
            state::UNKNOWN
        )]));

        // The one silent case: every guest provably never heard of it, so a
        // cancellation would be the first they knew of the meeting.
        assert!(!cancellation_should_notify(&[
            guest("a@x.com", state::UNSENT),
            guest("b@x.com", state::UNSENT),
        ]));

        // Nobody to tell.
        assert!(!cancellation_should_notify(&[]));
    }

    #[test]
    fn a_caller_can_choose_silence_but_can_never_force_mail() {
        let knows = [guest("a@x.com", state::RESPONDED)];
        let told_nobody = [guest("a@x.com", state::UNSENT)];

        // Asking to notify, on an event people know about: mail goes out.
        assert!(cancellation_send_updates(true, &knows));
        // Declining to notify always wins — this is the "delete without
        // notifying" escape hatch, and it must be absolute.
        assert!(!cancellation_send_updates(false, &knows));
        // Asking to notify cannot override the rule. This is the direction that
        // matters: it is what makes trusting a frontend boolean safe at all.
        assert!(!cancellation_send_updates(true, &told_nobody));
        assert!(!cancellation_send_updates(false, &told_nobody));
    }

    #[test]
    fn the_organizers_own_attendee_row_never_decides_a_cancellation() {
        // The local user is on their own event, usually `accepted`. Counting
        // that as "somebody knows" would make every solo event notify, and
        // counting it as unsent could silence a real one.
        let mut me = guest("chris@cxventures.io", state::RESPONDED);
        me.is_self = true;
        assert!(!cancellation_should_notify(&[
            me.clone(),
            guest("b@x.com", state::UNSENT),
        ]));
        assert!(!cancellation_should_notify(&[me]));
    }

    #[test]
    fn the_ledger_index_groups_by_the_remote_triple() {
        let conn = memory_db();
        record_notified(&conn, "acct", "primary", "evt-1", &["a@x.com".into()], "cxmail").unwrap();
        record_notified(&conn, "acct", "primary", "evt-2", &["b@x.com".into()], "cxmail").unwrap();

        let index = notified_by_event(&conn, Some("acct")).unwrap();
        assert_eq!(index.len(), 2);
        assert!(index[&(
            "acct".to_string(),
            "primary".to_string(),
            "evt-1".to_string()
        )]
        .contains("a@x.com"));
    }
}
