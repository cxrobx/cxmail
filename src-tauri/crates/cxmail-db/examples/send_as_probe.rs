//! What send-as detection finds on a copy of a real mailbox: per account, the
//! send-as list (configured + found on Sent mail) and the delivery-header
//! suggestions. Point it at a `.backup` of the live DB, never the live DB (#12).
//!
//!     cargo run -p cxmail-db --example send_as_probe -- /tmp/probe.db

fn main() {
    let path = std::env::args().nth(1).expect("usage: send_as_probe <db path>");
    let conn = rusqlite::Connection::open(&path).expect("open");
    cxmail_db::db::schema::initialize(&conn).expect("initialize");
    let mut stmt = conn
        .prepare("SELECT id, email, display_name FROM accounts ORDER BY email")
        .unwrap();
    let accounts: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    for (id, email, name) in accounts {
        let list =
            cxmail_db::db::identities::send_as_addresses(&conn, &id, &email, name.as_deref())
                .unwrap();
        println!("{email}");
        for a in &list[1..] {
            let src = if a.from_sent { "found in Sent" } else { "configured" };
            println!("    send-as  {}  ({src})", a.email);
        }
        for (s, n) in cxmail_db::db::identities::suggest_aliases(&conn, &id, &email, 8).unwrap() {
            println!("    suggest  {s}  ({n})");
        }
    }
}
