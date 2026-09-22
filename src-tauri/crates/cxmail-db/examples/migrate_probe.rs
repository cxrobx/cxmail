//! Run the migration chain against a copy of a real database.
//!
//! Six of the v-series migrations call helpers that now live in `cxmail-core`
//! rather than beside them, so "it compiles" is not the same as "it still
//! migrates". Point this at a `.backup` of the live DB, never the live DB.
//!
//!     cargo run -p cxmail-db --example migrate_probe -- /tmp/probe.db

fn main() {
    let path = std::env::args().nth(1).expect("usage: migrate_probe <db path>");
    let conn = rusqlite::Connection::open(&path).expect("open");
    let before: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(-1);
    cxmail_db::db::schema::initialize(&conn).expect("initialize");
    let after: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(-1);
    let messages: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap_or(-1);
    println!("user_version {before} -> {after}; messages readable: {messages}");
}
