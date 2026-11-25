//! The one way a fetched message body reaches SQLite.
//!
//! Every path that fetches a full RFC822 message (the desktop's on-demand
//! open, sync prefetch, the IDLE prefetch, the MCP and the phone's remote
//! helpers) writes the same tables through [`persist_parsed_metadata`]: body,
//! raw headers, snippet, address lists, attachments (which sets
//! `attachment_metadata_checked`), knowledge index and calendar events. A path
//! that wrote only some of them left rows the image policy then read as "no
//! attachments", so the write lives here, below every caller.

use super::{detect_events, parser};
use crate::{db, error::AppError};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

/// Open the mail DB for a write from a process other than the desktop app.
/// `busy_timeout` because the app, its helper and the MCP write the same file
/// (the 0 ms default fails at once with SQLITE_BUSY during a sync); foreign
/// keys because the schema's cascades depend on them per connection.
pub fn open_for_write(db_path: &Path) -> Result<Connection, AppError> {
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

/// True when this message's body AND its attachment list are stored. A body
/// row without `attachment_metadata_checked` is not a full cache entry: the
/// image policy's attachment risk signals would read it as attachment-free.
pub fn is_fully_cached(conn: &Connection, account_id: &str, folder: &str, uid: u32) -> Result<bool, AppError> {
    Ok(conn
        .query_row(
            "SELECT attachment_metadata_checked FROM message_bodies
             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            params![account_id, folder, uid],
            |r| r.get::<_, i32>(0),
        )
        .optional()?
        .is_some_and(|checked| checked != 0))
}

/// Store a body fetched on demand, in one transaction.
///
/// No row yet: the full [`persist_parsed_metadata`] write. A row that already
/// exists (another process stored it meanwhile, or an older path wrote the
/// body alone) keeps its body, summary and address lists (gotcha #14) and only
/// gains what it lacks: raw headers, and the attachment list when it was never
/// checked. Returns whether a new body row was written.
pub fn store_fetched_body(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    parsed: &parser::ParsedMessage,
) -> Result<bool, AppError> {
    let tx = conn.unchecked_transaction()?;
    let existing: Option<i32> = tx
        .query_row(
            "SELECT attachment_metadata_checked FROM message_bodies
             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            params![account_id, folder, uid],
            |r| r.get(0),
        )
        .optional()?;
    let wrote = match existing {
        None => {
            persist_parsed_metadata(&tx, account_id, folder, uid, parsed);
            true
        }
        Some(checked) => {
            if let Some(ref headers) = parsed.raw_headers {
                db::messages::store_raw_headers(&tx, account_id, folder, uid, headers)?;
            }
            if checked == 0 {
                db::messages::insert_attachments(&tx, account_id, folder, uid, &parsed.attachments)?;
            }
            false
        }
    };
    tx.commit()?;
    Ok(wrote)
}

/// Write the full set of parsed metadata (body, snippet, address lists,
/// attachments, calendar events) for a single message inside an open
/// transaction. Mirrors the cache-miss branch of `fetch_message_body` so that
/// prefetch paths populate the same tables the on-demand read expects.
pub fn persist_parsed_metadata(
    tx: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    parsed: &parser::ParsedMessage,
) {
    if let Err(e) = db::messages::insert_body(
        tx,
        account_id,
        folder,
        uid,
        parsed.plain_text.as_deref(),
        parsed.html_body.as_deref(),
        parsed.sanitized_html.as_deref(),
        true,
    ) {
        log::warn!("Failed to store prefetched body for UID {uid}: {e}");
    }

    // Free ride: this path always ran on a full raw RFC822 message, so the
    // header block is already in hand and costs no extra IMAP. Everything that
    // reaches here — sync body prefetch, the on-demand cache miss, mbox import
    // — populates `message_headers` from now on. Best-effort: a failure here
    // must never lose the body write (gotcha: headers are an audit nicety, the
    // body is the product).
    if let Some(ref headers) = parsed.raw_headers {
        if let Err(e) = db::messages::store_raw_headers(tx, account_id, folder, uid, headers) {
            log::warn!("Failed to store raw headers for UID {uid}: {e}");
        }
    }

    if let Some(ref snippet) = parsed.snippet {
        if !snippet.is_empty() {
            let _ = db::messages::update_snippet(tx, account_id, folder, uid, snippet);
        }
    }

    if let Ok(to_json) = serde_json::to_string(&parsed.to_list) {
        let cc_json = serde_json::to_string(&parsed.cc_list).unwrap_or_default();
        let bcc_json = serde_json::to_string(&parsed.bcc_list).unwrap_or_default();
        if let Err(e) = db::messages::store_address_lists(tx, account_id, folder, uid, &to_json, &cc_json, &bcc_json) {
            log::error!("Failed to store address lists for message {uid}: {e}");
        }
    }

    if let Err(e) = db::messages::insert_attachments(tx, account_id, folder, uid, &parsed.attachments) {
        log::error!("Failed to insert attachments for message {uid}: {e}");
    }

    if let Ok(id) = db::intelligence::message_id(tx, account_id, folder, uid) {
        if let Err(e) =
            db::knowledge::index_source(tx, id, "Email", parsed.plain_text.as_deref().unwrap_or(""))
        {
            log::warn!("Body indexing failed: {e}");
        }
        for (source, text) in &parsed.attachment_text {
            if let Err(e) = db::knowledge::index_source(tx, id, source, text) {
                log::warn!("Attachment indexing failed: {e}");
            }
        }
    }

    for event in &parsed.calendar_events {
        if let Err(e) = db::calendar::insert(
            tx,
            account_id,
            folder,
            uid,
            event.event_uid.as_deref(),
            event.summary.as_deref(),
            event.description.as_deref(),
            event.location.as_deref(),
            &event.dtstart,
            event.dtend.as_deref(),
            event.organizer_name.as_deref(),
            event.organizer_email.as_deref(),
            event.status.as_deref(),
            event.method.as_deref(),
            Some(&event.raw_ics),
        ) {
            log::error!("Failed to insert calendar event for message {uid}: {e}");
        }
    }

    if parsed.calendar_events.is_empty() {
        if let Ok(msg_row) = db::messages::get_by_uid(tx, account_id, folder, uid) {
            let detected = detect_events::detect_events(
                parsed.subject.as_deref(),
                parsed.plain_text.as_deref(),
                &parsed.from_email,
                msg_row.as_ref().map(|m| m.date.as_str()).unwrap_or(""),
            );
            for event in &detected {
                if event.confidence >= 0.6 {
                    if let Err(e) = db::calendar::insert_detected(
                        tx,
                        account_id,
                        folder,
                        uid,
                        &event.summary,
                        &event.dtstart,
                        event.dtend.as_deref(),
                        event.location.as_deref(),
                        event.description.as_deref(),
                        event.confidence,
                    ) {
                        log::error!("Failed to insert detected event for message {uid}: {e}");
                    }
                }
            }
        }
    }
}


#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fully authenticated newsletter with a remote image and an
    /// executable attachment. Fiction-only addresses.
    pub(crate) const RISKY: &str = "Received: from mail.example.org by mx.google.com with ESMTPS; today\r\n\
Authentication-Results: mx.google.com; dmarc=pass header.from=example.org\r\n\
From: news@example.org\r\n\
To: me@example.net\r\n\
Subject: Invoice\r\n\
Message-ID: <one@example.org>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hello</p><img src=\"https://example.org/image.png\">\r\n\
--b\r\n\
Content-Type: application/octet-stream; name=\"invoice.exe\"\r\n\
Content-Disposition: attachment; filename=\"invoice.exe\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
TVqQAAMAAAAEAAAA\r\n\
--b--\r\n";

    pub(crate) fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        db::schema::initialize(&c).unwrap();
        c.execute_batch(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host) VALUES ('a','me@example.net','gmail','imap.gmail.com','smtp.gmail.com');
             INSERT INTO folders (account_id,name,folder_type) VALUES ('a','INBOX','inbox');
             INSERT INTO messages (account_id,folder_name,uid,from_email,date,message_id) VALUES ('a','INBOX',1,'news@example.org','2026-11-15T12:00:00Z','<one@example.org>');",
        )
        .unwrap();
        c
    }

    fn attachment_names(c: &Connection) -> Vec<String> {
        db::messages::get_attachments(c, "a", "INBOX", 1)
            .unwrap()
            .into_iter()
            .filter_map(|a| a.filename)
            .collect()
    }

    #[test]
    fn a_new_body_is_stored_whole_with_its_attachment_list_and_headers() {
        let c = db();
        assert!(!is_fully_cached(&c, "a", "INBOX", 1).unwrap());
        let parsed = parser::parse_message(RISKY.as_bytes());
        assert!(store_fetched_body(&c, "a", "INBOX", 1, &parsed).unwrap());
        assert!(is_fully_cached(&c, "a", "INBOX", 1).unwrap());
        let body = db::messages::get_body(&c, "a", "INBOX", 1).unwrap().unwrap();
        assert!(body.sanitized_html.unwrap().contains("Hello"));
        assert!(body.attachment_metadata_checked);
        assert_eq!(attachment_names(&c), vec!["invoice.exe".to_string()]);
        assert!(db::messages::get_raw_headers(&c, "a", "INBOX", 1).unwrap().unwrap().contains("dmarc=pass"));
    }

    #[test]
    fn a_body_only_row_keeps_its_body_and_summary_and_gains_the_attachment_list() {
        // What MCP quoted-history write-back leaves: a body, no attachment check.
        let c = db();
        c.execute_batch(
            "INSERT INTO message_bodies (account_id,folder_name,uid,plain_text,sanitized_html,summary)
             VALUES ('a','INBOX',1,'kept text','<p>kept</p>','kept summary');",
        )
        .unwrap();
        assert!(!is_fully_cached(&c, "a", "INBOX", 1).unwrap(), "body-only is not a full cache entry");
        let parsed = parser::parse_message(RISKY.as_bytes());
        assert!(!store_fetched_body(&c, "a", "INBOX", 1, &parsed).unwrap());
        let (text, summary): (String, String) = c
            .query_row("SELECT plain_text, summary FROM message_bodies WHERE uid=1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((text.as_str(), summary.as_str()), ("kept text", "kept summary"), "gotcha #14: never clobber");
        assert!(is_fully_cached(&c, "a", "INBOX", 1).unwrap());
        assert_eq!(attachment_names(&c), vec!["invoice.exe".to_string()]);
        assert!(db::messages::get_raw_headers(&c, "a", "INBOX", 1).unwrap().is_some());
    }

    #[test]
    fn a_fully_cached_row_is_left_alone() {
        let c = db();
        let parsed = parser::parse_message(RISKY.as_bytes());
        store_fetched_body(&c, "a", "INBOX", 1, &parsed).unwrap();
        c.execute_batch("UPDATE attachments SET filename='renamed.exe'; UPDATE message_bodies SET summary='s';").unwrap();
        assert!(!store_fetched_body(&c, "a", "INBOX", 1, &parsed).unwrap());
        assert_eq!(attachment_names(&c), vec!["renamed.exe".to_string()]);
        let summary: String = c.query_row("SELECT summary FROM message_bodies WHERE uid=1", [], |r| r.get(0)).unwrap();
        assert_eq!(summary, "s");
    }
}
