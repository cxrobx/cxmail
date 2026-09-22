//! Which repo would "Open in Claude" land in, for real mail?
//!
//! `cargo run -p cxmail-db --example resolve_repo_probe -- <copy.db> [limit]`
//!
//! Point it at a `.backup` copy, never the live database — `initialize` is not
//! run here, but the app and the helper both hold that file (gotcha #12).
//! Answers the question no unit test can: does the mapping table, the group
//! rules and the real corpus together produce sensible landings, or does one
//! loose contact row swallow the whole inbox?

use cxmail_db::db::claude_repos;
use rusqlite::Connection;
use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: resolve_repo_probe <db> [limit]");
    let limit: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);

    let conn = Connection::open(&path).expect("open db");
    let mut stmt = conn
        .prepare(
            "SELECT account_id, folder_name, uid, from_email, substr(subject, 1, 44)
               FROM messages
              WHERE folder_name = 'INBOX'
              ORDER BY date DESC
              LIMIT ?1",
        )
        .expect("prepare");
    let rows: Vec<(String, String, u32, Option<String>, Option<String>)> = stmt
        .query_map([limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");

    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (account_id, folder, uid, from, subject) in &rows {
        let resolved = claude_repos::resolve_for_message(&conn, account_id, folder, *uid)
            .expect("resolve");
        let key = match &resolved {
            Some(r) => format!(
                "{:<8} {:<26} {}",
                r.scope.as_str(),
                r.source,
                r.repo_path
            ),
            None => "(none — scratch dir)".to_string(),
        };
        *tally.entry(key.clone()).or_default() += 1;
        let bucket = samples.entry(key).or_default();
        if bucket.len() < 3 {
            bucket.push(format!(
                "{} — {}",
                from.as_deref().unwrap_or("?"),
                subject.as_deref().unwrap_or("(no subject)")
            ));
        }
    }

    println!("{} recent INBOX messages\n", rows.len());
    let mut sorted: Vec<_> = tally.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    for (key, count) in sorted {
        println!("{count:>4}  {key}");
        for sample in samples.get(&key).into_iter().flatten() {
            println!("        {sample}");
        }
    }
}
