//! Persist locally-generated drafts (CXMail compose modal or MCP) into the
//! same SQLite tables the IMAP sync would populate, so search finds them
//! without waiting for the next folder sync.
//!
//! The IMAP `APPEND` is the source of truth; this helper writes the same
//! information the upcoming sync would write, derived from the raw MIME we
//! just built. If the write fails the IMAP draft is unaffected and the next
//! sync cycle reconciles state.

use crate::db;
use crate::email::parser::{self, ParsedMessage};
use crate::error::AppError;
use rusqlite::Connection;
use std::panic::AssertUnwindSafe;

/// The header that carries a draft's stable identity through IMAP (v60,
/// `db::drafts`). Every writer of a draft's MIME sets it; the folder sync
/// reads it back (`imap::ImapMessageHeader::draft_id`). The value is the bare
/// `draft_id`, no brackets. Gmail's web composer rebuilds the MIME and drops
/// unknown headers, so a draft edited there loses its identity — which must
/// degrade to "not found", never to a wrong match.
pub const DRAFT_ID_HEADER: &str = "X-CXMail-Draft-Id";

/// Parse `raw` and persist a header row + body + address lists + attachments
/// for a newly-created draft. The v40 FTS triggers index the insert
/// automatically — no separate search write is needed (or possible; the
/// standalone MCP binary shares this path).
///
/// `raw` is the RFC822 bytes that were just APPENDed to the IMAP server.
/// `uid` is the UID the server assigned (obtained via UID SEARCH).
pub fn persist_local_draft(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    raw: &[u8],
) -> Result<ParsedMessage, AppError> {
    // Parser can panic on hostile MIME (see Gotcha #9). Drafts we just built
    // shouldn't trigger it, but the wrap keeps a malformed signature/HTML from
    // taking down the Tauri command.
    let parsed = std::panic::catch_unwind(AssertUnwindSafe(|| parser::parse_message(raw)))
        .map_err(|_| AppError::General("parse_message panicked on local draft".to_string()))?;

    // DO NOT wrap these writes in an outer `conn.unchecked_transaction()`.
    // `insert_batch` opens its OWN transaction (db::messages.rs), so an outer one
    // makes its `BEGIN` fail with "cannot start a transaction within a transaction"
    // — which rolled back EVERY local draft write from May–Jul 2026 (the failure was
    // masked because the follow-up IMAP Drafts sync re-persisted the row minutes
    // later, and it silently broke the live compose-modal reload). Each helper below
    // owns its transaction (insert_batch) or is a single autocommitting statement
    // (insert_body/update_snippet/store_address_lists/insert_attachments), and the
    // body/address/attachment calls already tolerate individual failure. Regression:
    // test `persist_local_draft_is_searchable`.

    // Headers row. Mirrors `headers_to_insert_batch` in sync.rs — flags column
    // is the legacy JSON column and is left empty; is_read/is_flagged hold the
    // truth. `\Seen` is set because we APPEND with `\Seen \Draft`.
    // mail-parser strips the angle brackets, so writing its output straight
    // through mints a BARE `id@host` into a column that is 99.9% bracketed —
    // which every exact-match lookup then misses (gotcha #30). Re-bracket at
    // ingress, mirroring `email/import.rs`, so drafts are repliable-to and the
    // reference chain stays RFC-valid.
    use crate::email::message_id::{normalize_message_id, normalize_reference_chain};
    let references_joined = normalize_reference_chain(&parsed.references.join(" "));
    let message_id_norm = parsed.message_id.as_deref().and_then(normalize_message_id);
    let in_reply_to_norm = parsed.in_reply_to.as_deref().and_then(normalize_message_id);
    let to_json = serde_json::to_string(&parsed.to_list).unwrap_or_else(|_| "[]".to_string());
    let cc_json = serde_json::to_string(&parsed.cc_list).unwrap_or_else(|_| "[]".to_string());
    // Bcc rides along in the local cache so a same-session draft reopen recovers
    // it without an IMAP round-trip (gotcha #25 sibling fix).
    let bcc_json = serde_json::to_string(&parsed.bcc_list).unwrap_or_else(|_| "[]".to_string());
    let date_str = parsed.date.clone().unwrap_or_default();
    let snippet_str = parsed.snippet.clone();
    let has_attachments = !parsed.attachments.is_empty();

    {
        let row = (
            uid,
            parsed.subject.as_deref(),
            parsed.from_name.as_deref(),
            Some(parsed.from_email.as_str()),
            date_str.as_str(),
            snippet_str.as_deref(),
            "[]",
            message_id_norm.as_deref(),
            in_reply_to_norm.as_deref(),
            if references_joined.is_empty() {
                None
            } else {
                Some(references_joined.as_str())
            },
            true,  // is_read — drafts open with \Seen
            false, // is_flagged
            has_attachments,
            parsed.size_bytes as i64,
            None::<&str>, // list_unsubscribe
            None::<&str>, // list_unsubscribe_post
            to_json.as_str(),
            cc_json.as_str(),
        );
        db::messages::insert_batch(conn, account_id, folder, &[row])?;
    }

    if let Err(e) = db::messages::insert_body(
        conn,
        account_id,
        folder,
        uid,
        parsed.plain_text.as_deref(),
        parsed.html_body.as_deref(),
        parsed.sanitized_html.as_deref(),
        true,
    ) {
        log::warn!("Failed to store body for local draft UID {uid}: {e}");
    }

    if let Some(ref snippet) = parsed.snippet {
        if !snippet.is_empty() {
            let _ = db::messages::update_snippet(conn, account_id, folder, uid, snippet);
        }
    }

    if let Err(e) = db::messages::store_address_lists(
        conn, account_id, folder, uid, &to_json, &cc_json, &bcc_json,
    ) {
        log::warn!("Failed to store address lists for local draft UID {uid}: {e}");
    }

    if let Err(e) =
        db::messages::insert_attachments(conn, account_id, folder, uid, &parsed.attachments)
    {
        log::warn!("Failed to insert attachments for local draft UID {uid}: {e}");
    }

    // Keep the attachment BYTES too. The compose window reloads them on every
    // reopen (gotcha #25) and used to go to IMAP for the whole message each
    // time; we have the raw MIME right here. Inline (cid:) images are already
    // data: URIs in the sanitized HTML, so only the real parts are kept — the
    // same filter `fetch_outgoing_attachments` applies on the IMAP path.
    if parsed.attachments.iter().any(|a| !a.is_inline) {
        let blobs = draft_blobs_from_raw(raw);
        if let Err(e) = db::draft_blobs::replace(conn, account_id, folder, uid, &blobs) {
            log::warn!("Failed to cache attachment bytes for local draft UID {uid}: {e}");
        }
    }

    Ok(parsed)
}

/// The non-inline attachment parts of `raw`, decoded — what a reopen needs.
pub fn draft_blobs_from_raw(raw: &[u8]) -> Vec<db::draft_blobs::DraftBlob> {
    parser::extract_all_attachments(raw)
        .into_iter()
        .filter(|(_, _, _, is_inline)| !is_inline)
        .map(|(filename, content_type, data, _)| db::draft_blobs::DraftBlob {
            filename,
            content_type,
            data,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::search::{self, SearchFilters};
    use rusqlite::Connection;

    /// Full production schema (v1→v40) on an in-memory DB, so `insert_batch`'s
    /// columns, the accounts FK, and the v40 FTS triggers all exist exactly as
    /// they do in production.
    fn schema_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();
        conn
    }

    const RAW_DRAFT: &[u8] = b"From: Chris <chris@cxventures.io>\r\n\
To: dana@northwind.example\r\n\
Subject: A quick review of Brightloom's June blog posts\r\n\
Message-ID: <draft-test-1@cxmail.app>\r\n\
Date: Sat, 05 Jul 2026 23:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Both blogs are ready to publish after a few small fixes.\r\n";

    /// A multipart draft: prose, one inline cid: image, one real attachment.
    const RAW_DRAFT_WITH_ATTACHMENT: &[u8] = b"From: Chris <chris@cxventures.io>\r\n\
To: dana@northwind.example\r\n\
Subject: The product, start to finish, in three minutes\r\n\
Message-ID: <draft-test-2@cxmail.app>\r\n\
Date: Tue, 25 Aug 2026 04:43:25 +0000\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hi Dana, the attached video shows the whole loop.\r\n\
--b1\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@cxmail>\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--b1\r\n\
Content-Type: video/mp4\r\n\
Content-Disposition: attachment; filename=\"Northwind-Product-Walkthrough.mp4\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
AAAAHGZ0eXA=\r\n\
--b1--\r\n";

    /// Reopening a draft reloads its real attachments (gotcha #25). Those bytes
    /// were in the raw MIME we just saved — persisting must keep them, and only
    /// them: the inline logo is already a data: URI in the sanitized HTML.
    #[test]
    fn persist_local_draft_caches_real_attachment_bytes_but_not_inline_images() {
        let conn = schema_conn();
        persist_local_draft(&conn, "acct", "[Gmail]/Drafts", 739, RAW_DRAFT_WITH_ATTACHMENT)
            .expect("persist must succeed");

        let blobs = db::draft_blobs::get(&conn, "acct", "[Gmail]/Drafts", 739).unwrap();
        assert_eq!(blobs.len(), 1, "exactly the real attachment, got {blobs:?}");
        assert_eq!(blobs[0].filename, "Northwind-Product-Walkthrough.mp4");
        assert_eq!(blobs[0].content_type, "video/mp4");
        assert_eq!(blobs[0].data, vec![0x00, 0x00, 0x00, 0x1c, 0x66, 0x74, 0x79, 0x70]);

        // A prose-only draft writes nothing here.
        persist_local_draft(&conn, "acct", "[Gmail]/Drafts", 740, RAW_DRAFT).unwrap();
        assert!(db::draft_blobs::get(&conn, "acct", "[Gmail]/Drafts", 740).unwrap().is_empty());

        // And the autosave UID churn evicts: edit_draft deletes the old row.
        db::messages::delete_uids(&conn, "acct", "[Gmail]/Drafts", &[739]).unwrap();
        assert!(db::draft_blobs::get(&conn, "acct", "[Gmail]/Drafts", 739).unwrap().is_empty());
    }

    /// The regression: pre-fix, `persist_local_draft` opened an outer transaction
    /// that made `insert_batch`'s own `BEGIN` fail with "cannot start a transaction
    /// within a transaction", so nothing was written and the draft was invisible
    /// locally (and never live-reloaded in the compose modal) until the next IMAP
    /// sync. This asserts the header row, the body, AND the FTS index all land.
    #[test]
    fn persist_local_draft_is_searchable() {
        let conn = schema_conn();

        let parsed = persist_local_draft(&conn, "acct", "[Gmail]/Drafts", 500, RAW_DRAFT)
            .expect("persist must succeed — a nested transaction here is the bug");
        assert_eq!(
            parsed.subject.as_deref(),
            Some("A quick review of Brightloom's June blog posts")
        );

        // Header row landed.
        let subject: String = conn
            .query_row(
                "SELECT subject FROM messages WHERE account_id='acct' AND folder_name='[Gmail]/Drafts' AND uid=500",
                [],
                |r| r.get(0),
            )
            .expect("messages row must exist after persist");
        assert!(subject.contains("Brightloom"));

        // Body landed.
        let body_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_bodies WHERE account_id='acct' AND folder_name='[Gmail]/Drafts' AND uid=500",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body_count, 1, "body must be persisted exactly once");

        // FTS triggers indexed it — a body-word query returns the draft.
        let hits = search::search(
            &conn,
            &SearchFilters {
                keywords: Some("Brightloom".into()),
                ..Default::default()
            },
            10,
            0,
            false,
        )
        .unwrap();
        assert!(
            hits.iter().any(|h| h.uid == 500),
            "the freshly-persisted draft must be FTS-searchable, got {hits:?}"
        );
    }
}
