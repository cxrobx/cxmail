//! Does a real thread still read correctly once folder copies are collapsed
//! and unsent drafts stop counting as messages?
//!
//! `cargo run -p cxmail-db --example thread_membership_probe -- <copy.db> [subject substring]`
//!
//! Point it at a `.backup` copy, never the live database (gotcha #12).
//!
//! With no filter it reports the mailbox-wide shape: how many threads carry an
//! unsent draft, and how much of every thread was duplicate folder copies.
//! With a subject substring it prints one thread twice — the raw rows the old
//! query returned, and the members `get_thread` returns now — so the collapse
//! can be read off rather than inferred. Watch for a thread that loses a row
//! it should have kept: the dedupe must only ever merge copies of the SAME
//! message_id, so every disappearance should pair with a survivor above it.

use cxmail_db::db::messages;
use rusqlite::Connection;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: thread_membership_probe <copy.db> [subject]");
    let filter = args.next();
    assert!(
        !path.ends_with("cxmail.db") || path.contains("backup") || path.contains("probe"),
        "point this at a .backup copy, not the live database (gotcha #12)"
    );
    let conn = Connection::open(&path).expect("open db");
    cxmail_db::db::schema::initialize(&conn).expect("migrate");

    match filter {
        Some(subject) => one_thread(&conn, &subject),
        None => overview(&conn),
    }
}

fn overview(conn: &Connection) {
    let q = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };

    println!("== mailbox-wide ==");
    println!(
        "threads carrying an unsent draft : {}",
        q("SELECT COUNT(*) FROM (
             SELECT m.account_id, m.thread_root_id FROM messages m
              JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
             WHERE f.folder_type = 'drafts' AND m.thread_root_id IS NOT NULL
             GROUP BY 1, 2)")
    );
    println!(
        "rows in those threads            : {}",
        q("SELECT COUNT(*) FROM messages m WHERE m.thread_root_id IN (
             SELECT m2.thread_root_id FROM messages m2
              JOIN folders f ON f.account_id = m2.account_id AND f.name = m2.folder_name
             WHERE f.folder_type = 'drafts')")
    );
    println!(
        "…of which are duplicate copies   : {}",
        q("SELECT COUNT(*) - COUNT(DISTINCT COALESCE(NULLIF(m.message_id,''),
                 'uid:' || m.folder_name || ':' || m.uid) || '|' || m.account_id)
             FROM messages m WHERE m.thread_root_id IN (
               SELECT m2.thread_root_id FROM messages m2
                JOIN folders f ON f.account_id = m2.account_id AND f.name = m2.folder_name
               WHERE f.folder_type = 'drafts')")
    );
    println!("\nRe-run with a subject substring to see one thread collapse.");
}

fn one_thread(conn: &Connection, subject: &str) {
    let (account_id, root): (String, String) = conn
        .query_row(
            "SELECT account_id, thread_root_id FROM messages
              WHERE subject LIKE '%' || ?1 || '%' AND thread_root_id IS NOT NULL
              ORDER BY datetime(date) DESC LIMIT 1",
            [subject],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("no thread matched that subject");

    println!("account {account_id}  thread {root}\n");

    println!("== every row (what the thread used to render) ==");
    let mut stmt = conn
        .prepare(
            "SELECT m.folder_name, m.uid, substr(m.date,1,16), COALESCE(f.folder_type,'-'),
                    COALESCE(m.message_id,'(none)')
               FROM messages m
               LEFT JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
              WHERE m.account_id = ?1 AND m.thread_root_id = ?2
              ORDER BY datetime(m.date) ASC, m.folder_name",
        )
        .unwrap();
    let raw: Vec<(String, u32, String, String, String)> = stmt
        .query_map(rusqlite::params![account_id, root], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (folder, uid, date, kind, mid) in &raw {
        println!("  {date}  {folder:<20} uid {uid:<6} [{kind}]  {mid}");
    }

    println!("\n== members get_thread returns now ==");
    let rows = messages::get_thread(conn, &account_id, &root).unwrap();
    for r in &rows {
        let draft: i64 = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM folders WHERE account_id=?1 AND name=?2 AND folder_type='drafts')",
                rusqlite::params![account_id, r.folder_name.clone().unwrap_or_default()],
                |x| x.get(0),
            )
            .unwrap();
        println!(
            "  {}  {:<20} uid {:<6} {}",
            &r.date.chars().take(16).collect::<String>(),
            r.folder_name.clone().unwrap_or_default(),
            r.uid,
            if draft == 1 { "← DRAFT (shown, marked, not counted)" } else { "" }
        );
    }
    println!("\n{} rows → {} members", raw.len(), rows.len());

    // And what the list row's badge will say for this thread.
    if let Ok((listed, _)) = messages::list_by_folder(conn, &account_id, "INBOX", 0, 500, None, false) {
        if let Some(row) = listed.iter().find(|r| r.thread_root_id.as_deref() == Some(root.as_str())) {
            println!(
                "INBOX list row: badge {} ({} + 1 exchanged), {} unsent draft(s)",
                row.thread_count + 1,
                row.thread_count,
                row.thread_draft_count
            );
        }
    }
}
