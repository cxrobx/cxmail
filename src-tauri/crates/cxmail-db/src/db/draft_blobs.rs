//! Attachment BYTES for drafts this software authored, so reopening a draft
//! does not re-download from IMAP what we had in hand when we saved it.
//!
//! Since 2026-06-18 (gotcha #25) opening a draft reloads its real attachments,
//! because the compose window rebuilds the MIME on every save and an attachment
//! it does not hold the bytes for is silently dropped. That reload went to IMAP:
//! a fresh connection, a FETCH of the *entire* message, a LOGOUT — for every
//! open. Invisible on a 40 KB PDF; a 25-second wait on an 8.8 MB draft sitting
//! on an account whose Gmail connects were timing out (2026-08-25).
//!
//! The bytes were never far away. `persist_local_draft` receives the full raw
//! MIME at save time and kept only the body and the attachment *metadata*. This
//! table keeps the parts too, keyed on the draft's triple and cascade-evicted
//! with the `messages` row exactly like `attachments` — a draft's UID moves on
//! every autosave (`edit_draft` expunges and re-appends), and `delete_uids`
//! takes the old blobs with it.
//!
//! **Cache, not truth.** IMAP holds the draft; an empty read here means "not
//! cached" (drafts saved before this table, or by another client) and the reader
//! falls back to IMAP. The reader is only ever called for a draft the metadata
//! says has real attachments, so absence is never ambiguous in practice.

use crate::error::AppError;
use rusqlite::{params, Connection};

/// One non-inline attachment part of a draft, bytes decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftBlob {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Above this many bytes per draft we do not cache and let the reader fall
/// back to IMAP. Gmail caps a message at 25 MB, so this only bites on generic
/// IMAP hosts with a huge draft — where writing it into SQLite on every
/// autosave is the wrong trade.
pub const MAX_CACHED_BYTES_PER_DRAFT: usize = 50 * 1024 * 1024;

/// Split out of the migration so tests can build the table without replaying
/// 58 versions of history. The FK cascade is the eviction strategy — do not
/// drop it.
pub(crate) fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS draft_attachment_blobs (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id   TEXT NOT NULL,
            folder_name  TEXT NOT NULL,
            message_uid  INTEGER NOT NULL,
            filename     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            data         BLOB NOT NULL,
            FOREIGN KEY (account_id, folder_name, message_uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_draft_attachment_blobs_msg
            ON draft_attachment_blobs(account_id, folder_name, message_uid);",
    )
}

/// Replace the cached parts for one draft. Idempotent: prior rows for the
/// triple are deleted first, so a re-persist (sync raced the save, or a later
/// IMAP-fallback open) never accumulates duplicates. Over the size cap the
/// prior rows are still cleared and nothing is written.
pub fn replace(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    blobs: &[DraftBlob],
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM draft_attachment_blobs
         WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3",
        params![account_id, folder_name, uid],
    )?;
    let total: usize = blobs.iter().map(|b| b.data.len()).sum();
    if total <= MAX_CACHED_BYTES_PER_DRAFT {
        let mut stmt = tx.prepare(
            "INSERT INTO draft_attachment_blobs
                (account_id, folder_name, message_uid, filename, content_type, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for b in blobs {
            stmt.execute(params![
                account_id,
                folder_name,
                uid,
                b.filename,
                b.content_type,
                b.data
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// The cached parts for one draft, in the order they appear in the MIME.
/// Empty means not cached — never "no attachments"; the caller decides that
/// from the metadata.
pub fn get(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<DraftBlob>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT filename, content_type, data FROM draft_attachment_blobs
         WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![account_id, folder_name, uid], |r| {
        Ok(DraftBlob {
            filename: r.get(0)?,
            content_type: r.get(1)?,
            data: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal `messages` with the composite key the FK targets, foreign keys
    /// ON as in production (`schema::initialize` sets the pragma).
    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE messages (
                 account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                 UNIQUE(account_id, folder_name, uid)
             );
             INSERT INTO messages VALUES ('acct', '[Gmail]/Drafts', 739);
             INSERT INTO messages VALUES ('acct', '[Gmail]/Drafts', 740);",
        )
        .unwrap();
        create_table(&c).unwrap();
        c
    }

    fn video() -> DraftBlob {
        DraftBlob {
            filename: "Northwind-Product-Walkthrough.mp4".into(),
            content_type: "video/mp4".into(),
            data: vec![0x00, 0x00, 0x00, 0x1c, 0x66, 0x74, 0x79, 0x70],
        }
    }

    #[test]
    fn round_trips_bytes_in_mime_order() {
        let c = conn();
        let pdf = DraftBlob {
            filename: "deck.pdf".into(),
            content_type: "application/pdf".into(),
            data: b"%PDF-1.7".to_vec(),
        };
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[video(), pdf.clone()]).unwrap();
        assert_eq!(get(&c, "acct", "[Gmail]/Drafts", 739).unwrap(), vec![video(), pdf]);
        assert!(get(&c, "acct", "[Gmail]/Drafts", 740).unwrap().is_empty());
    }

    #[test]
    fn replace_is_idempotent_not_additive() {
        let c = conn();
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[video()]).unwrap();
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[video()]).unwrap();
        assert_eq!(get(&c, "acct", "[Gmail]/Drafts", 739).unwrap().len(), 1);
    }

    /// THE eviction contract: the draft's UID moves on every autosave and
    /// `edit_draft` deletes the old `messages` row — the blobs must go with it,
    /// or every autosave of a video draft leaks another copy into the DB.
    #[test]
    fn deleting_the_message_row_evicts_its_blobs() {
        let c = conn();
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[video()]).unwrap();
        c.execute(
            "DELETE FROM messages WHERE account_id='acct' AND folder_name='[Gmail]/Drafts' AND uid=739",
            [],
        )
        .unwrap();
        let orphans: i64 = c
            .query_row("SELECT COUNT(*) FROM draft_attachment_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0, "blobs must cascade with the messages row");
    }

    #[test]
    fn oversize_drafts_are_not_cached_and_prior_rows_are_cleared() {
        let c = conn();
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[video()]).unwrap();
        let huge = DraftBlob {
            filename: "raw.mov".into(),
            content_type: "video/quicktime".into(),
            data: vec![0u8; MAX_CACHED_BYTES_PER_DRAFT + 1],
        };
        replace(&c, "acct", "[Gmail]/Drafts", 739, &[huge]).unwrap();
        assert!(get(&c, "acct", "[Gmail]/Drafts", 739).unwrap().is_empty());
    }

    #[test]
    fn a_blob_needs_its_message_row() {
        let c = conn();
        let err = replace(&c, "acct", "[Gmail]/Drafts", 999, &[video()]);
        assert!(err.is_err(), "no messages row → FK must refuse, not orphan");
    }
}
