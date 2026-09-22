//! What does hiding an account from aggregates actually remove, on real mail?
//!
//! `cargo run -p cxmail-db --example hidden_scope_probe -- <copy.db> <account email>`
//!
//! Point it at a `.backup` copy, never the live database (gotcha #12). It runs
//! `initialize` (so a pre-v58 copy is migrated first), then prints every
//! aggregate before and after flipping the flag for the named account — and
//! that account's own inbox, which must not move. Every drop should equal that
//! account's own share and nothing else; a drop anywhere else means a
//! predicate landed on the wrong side of a join.

use cxmail_db::db::{self, accounts, categories, inbox_groups, messages, needs_you, nudges, search};
use rusqlite::Connection;

fn snapshot(conn: &Connection, account_id: &str, phrase: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    out.push((
        "count_total_inbox_unread (dock badge)".into(),
        messages::count_total_inbox_unread(conn).unwrap().to_string(),
    ));
    let (_, all_total) = messages::list_all_inboxes(conn, 0, 1, None, None, false).unwrap();
    out.push(("list_all_inboxes total (threads)".into(), all_total.to_string()));

    let mut cats: Vec<(String, u32)> = categories::count_unread_by_category_unified(conn)
        .unwrap()
        .into_iter()
        .collect();
    cats.sort();
    out.push((
        "count_unread_by_category_unified".into(),
        cats.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "),
    ));

    let groups = inbox_groups::list_groups(conn).unwrap();
    out.push((
        "list_groups unread".into(),
        groups
            .iter()
            .map(|g| format!("{}={}", g.name, g.unread_count))
            .collect::<Vec<_>>()
            .join(" "),
    ));
    for g in &groups {
        let (_, total) = inbox_groups::list_messages_for_group(conn, g.id, 0, 1, false).unwrap();
        out.push((format!("list_messages_for_group[{}] total", g.name), total.to_string()));
    }

    out.push((
        "needs_you::list().len()".into(),
        needs_you::list(conn, 500, false).unwrap().len().to_string(),
    ));
    out.push((
        "nudges::list().len()".into(),
        nudges::list(conn, &nudges::NudgeOptions::default()).unwrap().len().to_string(),
    ));

    let f = search::SearchFilters { keywords: Some(phrase.to_string()), ..Default::default() };
    let hits = search::search(conn, &f, 500, 0, false).unwrap();
    let own = hits.iter().filter(|r| r.account_id == account_id).count();
    out.push((
        format!("search(\"{phrase}\") unscoped"),
        format!("{} hits ({own} from the account)", hits.len()),
    ));
    let f = search::SearchFilters {
        keywords: Some(phrase.to_string()),
        account_ids: Some(vec![account_id.to_string()]),
        ..Default::default()
    };
    out.push((
        format!("search(\"{phrase}\") scoped to the account"),
        search::search(conn, &f, 500, 0, false).unwrap().len().to_string(),
    ));

    // The negative control: the account's own inbox.
    let (_, own_total) = messages::list_by_folder(conn, account_id, "INBOX", 0, 1, None, false).unwrap();
    out.push(("list_by_folder(account, INBOX) total — MUST NOT MOVE".into(), own_total.to_string()));
    let own_unread: u32 = categories::count_unread_by_category(conn, account_id, "INBOX")
        .unwrap()
        .values()
        .sum();
    out.push(("count_unread_by_category(account, INBOX) — MUST NOT MOVE".into(), own_unread.to_string()));
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: hidden_scope_probe <db> <account email> [phrase]");
    let email = args.next().expect("usage: hidden_scope_probe <db> <account email> [phrase]");
    let phrase = args.next().unwrap_or_else(|| "unsubscribe".to_string());

    let conn = Connection::open(&path).expect("open db");
    db::schema::initialize(&conn).expect("initialize (migrates a pre-v58 copy)");

    let account = accounts::list(&conn)
        .expect("list accounts")
        .into_iter()
        .find(|a| a.email.eq_ignore_ascii_case(&email))
        .unwrap_or_else(|| panic!("no account with email {email}"));
    println!(
        "account {} ({}) hidden_from_aggregates={}\n",
        account.email, account.id, account.hidden_from_aggregates
    );

    accounts::set_hidden_from_aggregates(&conn, &account.id, false).unwrap();
    let before = snapshot(&conn, &account.id, &phrase);
    accounts::set_hidden_from_aggregates(&conn, &account.id, true).unwrap();
    let after = snapshot(&conn, &account.id, &phrase);
    // Leave the copy as we found it.
    accounts::set_hidden_from_aggregates(&conn, &account.id, account.hidden_from_aggregates).unwrap();

    let width = before.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    println!("{:<width$}  {:>14}  {:>14}", "aggregate", "visible", "hidden");
    for ((k, b), (_, a)) in before.iter().zip(after.iter()) {
        let flag = if b == a { "" } else { "  <- changed" };
        println!("{k:<width$}  {b:>14}  {a:>14}{flag}");
    }
}
