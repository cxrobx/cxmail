//! Does the meaning-search chunking backfill ever finish, and what does it cost?
//!
//! `cargo run -p cxmail-db --example chunk_backfill_probe -- <copy.db> [--wipe]`
//!
//! Point it at a `.backup` copy, never the live database (gotcha #12).
//!
//! `intelligence::run_pass` opens with `loop { backfill(200); if chunked < 200
//! { break } }`. That terminates only if every selected row leaves a chunk
//! behind. A body that SQLite calls non-empty but `knowledge::chunks()` throws
//! away (newline-only text: SQLite `trim()` strips spaces only) is re-selected
//! for ever, so with >= 200 of them the loop spins for its whole 100 s budget
//! every two minutes. This prints the stuck population and then runs the real
//! loop, so a non-terminating one shows up as a repeating batch count instead
//! of a theory. `--wipe` clears `knowledge_chunks` first to time a cold
//! catch-up over the whole mailbox.
use cxmail_db::db::knowledge;
use rusqlite::Connection;
use std::time::Instant;

const BATCH: usize = 200;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .expect("usage: chunk_backfill_probe <copy.db> [--wipe] [--batch N]");
    let wipe = args.iter().any(|a| a == "--wipe");
    // The loop breaks on `n < BATCH`, so the batch size decides how many stuck
    // rows it takes to never break. Lowering it below the stuck population
    // reproduces the spin on real data without inventing rows.
    let batch = args
        .iter()
        .position(|a| a == "--batch")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(BATCH);
    assert!(
        !path.ends_with("cxmail.db") || path.contains("backup") || path.contains("probe"),
        "point this at a .backup copy, not the live database (gotcha #12)"
    );
    let conn = Connection::open(&path).expect("open db");
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA temp_store=MEMORY;").ok();
    cxmail_db::db::schema::initialize(&conn).expect("migrate");

    if wipe {
        conn.execute_batch("DELETE FROM knowledge_chunks;").expect("wipe");
        println!("wiped knowledge_chunks");
    }

    let selectable: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages m JOIN message_bodies b
               ON b.account_id=m.account_id AND b.folder_name=m.folder_name AND b.uid=m.uid
             WHERE length(trim(COALESCE(b.plain_text,'')))>0 AND NOT EXISTS
               (SELECT 1 FROM knowledge_chunks k WHERE k.message_id=m.id AND k.source='Email')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // The same rows, judged the way Rust judges them.
    let unchunkable: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages m JOIN message_bodies b
               ON b.account_id=m.account_id AND b.folder_name=m.folder_name AND b.uid=m.uid
             WHERE length(trim(COALESCE(b.plain_text,'')))>0
               AND length(trim(COALESCE(b.plain_text,''),' '||char(9)||char(10)||char(13)))=0
               AND NOT EXISTS
               (SELECT 1 FROM knowledge_chunks k WHERE k.message_id=m.id AND k.source='Email')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let opted_in: i64 = conn
        .query_row("SELECT count(*) FROM intelligence_accounts WHERE semantic_search=1", [], |r| r.get(0))
        .unwrap();
    println!("rows the PRE-T257 SELECT returned: {selectable}");
    println!("  of those, chunk to NOTHING     : {unchunkable}  <- the pre-fix loop re-selected these for ever");
    println!("accounts opted into meaning search: {opted_in}");
    println!("batch size                        : {batch}");

    // The real loop from commands/intelligence.rs::run_pass, with a hard stop so
    // a non-terminating one is reported instead of hanging the probe.
    let start = Instant::now();
    let mut iterations = 0u32;
    let mut rows_seen = 0usize;
    loop {
        let n = knowledge::backfill(&conn, batch).expect("backfill");
        iterations += 1;
        rows_seen += n;
        if iterations <= 5 || iterations % 50 == 0 {
            println!("  iter {iterations:>4}: {n} rows  ({:.1}s elapsed)", start.elapsed().as_secs_f64());
        }
        if n < batch {
            println!("loop TERMINATED after {iterations} iterations, {rows_seen} rows, {:.1}s", start.elapsed().as_secs_f64());
            break;
        }
        if start.elapsed().as_secs_f64() >= 100.0 {
            println!(
                "loop STILL RUNNING at the 100 s budget: {iterations} iterations, {rows_seen} rows \
                 — this is the hot loop, {:.0} full-table SELECTs/s",
                iterations as f64 / start.elapsed().as_secs_f64()
            );
            break;
        }
    }
    let left: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages m JOIN message_bodies b
               ON b.account_id=m.account_id AND b.folder_name=m.folder_name AND b.uid=m.uid
             WHERE length(trim(COALESCE(b.plain_text,'')))>0 AND NOT EXISTS
               (SELECT 1 FROM knowledge_chunks k WHERE k.message_id=m.id AND k.source='Email')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("pre-fix-selectable rows left   : {left}  (the fixed SELECT skips them)");
}
