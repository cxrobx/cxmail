//! Resolve every attendee on a real calendar to its delivery state.
//!
//! Unit tests pin the four-state rule against fixtures; this answers the
//! question fixtures cannot — what the guest panel will actually SAY about mail
//! that really exists. Point it at a `.backup` copy, never the live DB
//! (gotcha #12: two writers on one SQLite file).
//!
//!     cargo run -p cxmail-db --example invite_delivery_probe -- /tmp/probe.db
//!     cargo run -p cxmail-db --example invite_delivery_probe -- /tmp/probe.db Northwind

use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: invite_delivery_probe <db path> [summary filter]");
    let filter = args.next();

    let conn = rusqlite::Connection::open(&path).expect("open");
    let events = cxmail_db::db::gcal::list_unified_by_range(
        &conn,
        None,
        "2026-06-01T00:00:00Z",
        "2027-01-01T00:00:00Z",
    )
    .expect("list events");

    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut with_guests = 0usize;

    for event in &events {
        if event.attendee_delivery.is_empty() {
            continue;
        }
        with_guests += 1;
        for attendee in &event.attendee_delivery {
            *tally.entry(attendee.state.clone()).or_default() += 1;
        }

        let matches = filter
            .as_ref()
            .is_none_or(|needle| {
                event
                    .summary
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle.to_lowercase())
            });
        if filter.is_some() && matches {
            println!(
                "\n{}  ({})",
                event.summary.as_deref().unwrap_or("(no title)"),
                &event.dtstart
            );
            for attendee in &event.attendee_delivery {
                println!(
                    "   {:<10} {:<44} response={}",
                    attendee.state,
                    attendee.email,
                    attendee.response_status.as_deref().unwrap_or("-")
                );
            }
        }
    }

    println!("\n── {with_guests} events with guests, of {} in range", events.len());
    for (state, count) in &tally {
        println!("   {state:<10} {count}");
    }
}
