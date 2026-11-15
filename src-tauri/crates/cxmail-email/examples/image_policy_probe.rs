//! Read-only compatibility probe: report counts, never mailbox content.
use cxmail_email::email::image_policy::{authenticate, AuthStatus};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("pass the mailbox DB path")?;
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut q = conn.prepare("SELECT a.provider,a.imap_host,m.from_email,h.raw_headers FROM message_headers h JOIN accounts a ON a.id=h.account_id JOIN messages m ON m.account_id=h.account_id AND m.folder_name=h.folder_name AND m.uid=h.uid WHERE a.provider='gmail' ORDER BY m.date DESC LIMIT 200")?;
    let mut counts = [0usize; 3];
    for row in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })? {
        let (provider, host, from, raw) = row?;
        let index = match authenticate(&provider, &host, &from, Some(&raw)).status {
            AuthStatus::Passed => 0,
            AuthStatus::Failed => 1,
            AuthStatus::Unknown => 2,
        };
        counts[index] += 1;
    }
    println!(
        "Cached Google message checks: passed={}, failed={}, unavailable={}",
        counts[0], counts[1], counts[2]
    );
    Ok(())
}
