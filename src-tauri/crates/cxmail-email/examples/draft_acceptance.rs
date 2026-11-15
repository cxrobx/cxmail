//! Read-only server assertion for a disposable cross-Mac draft (T666).
//! Args: <database> <account-email> <draft-id> <expected-body-substring>
//! Run in Aqua with the app's signing identity; never prints credentials or mail.
use cxmail_email::email::{draft_revisions, imap};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: draft_acceptance <database> <account-email> <draft-id> <expected-body-substring>".into());
    }
    let conn = rusqlite::Connection::open_with_flags(&args[0], rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let account = cxmail_db::db::accounts::list(&conn)?
        .into_iter().find(|a| a.email == args[1]).ok_or("account not found")?;
    let folder = cxmail_db::db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts");
    let mut session = imap::connect_for_account(&account).await?;
    let result = async {
        let scan = draft_revisions::scan(&mut session, &folder).await?;
        let revisions = scan.revisions(&args[2]);
        if revisions.len() != 1 {
            return Err(format!("expected exactly one server revision, got {revisions:?}").into());
        }
        let body = imap::fetch_body(&mut session, revisions[0]).await?;
        let parsed = mail_parser::MessageParser::default().parse(&body).ok_or("invalid MIME")?;
        let text = parsed.body_text(0).unwrap_or_default();
        let html = parsed.body_html(0).unwrap_or_default();
        if !text.contains(&args[3]) && !html.contains(&args[3]) {
            return Err("server body does not contain the expected test marker".into());
        }
        println!("PASS server_revision_count=1 uid={} expected_marker_present=true", revisions[0]);
        Ok::<(), Box<dyn std::error::Error>>(())
    }.await;
    let _ = imap::disconnect(session, &account.email).await;
    result
}
