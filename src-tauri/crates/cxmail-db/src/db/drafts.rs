//! A draft's identity across its IMAP revisions (v60).
//!
//! IMAP has no mutable message: every save of a draft is an APPEND of a new
//! message with a new UID, and the old UID is expunged. The app's autosave,
//! `commands::compose::edit_draft` and the MCP's `edit_draft` all do this, so a
//! `(folder, uid)` names one *revision*, never the draft — and a caller holding
//! a UID the compose window has since autosaved past is holding nothing
//! (gotcha #57: `UID STORE` on a nonexistent UID is silently ignored, so the
//! stale reference used to fork the draft rather than fail).
//!
//! This table is the logical draft: a `draft_id` minted at first save and
//! written into the MIME as `X-CXMail-Draft-Id`, pointing at whichever UID is
//! the current revision. Two rules keep it honest:
//!
//! - **`current_uid` only moves forward within a UIDVALIDITY generation.**
//!   UIDs are strictly ascending inside one generation, so the highest UID
//!   ever seen for a draft *is* its latest revision, and `link` can be fed by
//!   every writer and by the folder sync in any order — including the window
//!   where both the old and the new revision are on the server. A new
//!   generation (or a new folder) resets the pointer; a UID is meaningless
//!   outside the mailbox generation that issued it.
//! - **A writer claims the row before it touches IMAP.** `claim` is a single
//!   compare-and-set on `current_uid`: the caller names the revision it read,
//!   and loses — before any APPEND — if another writer has already replaced it
//!   or holds an unexpired claim. The claim is a short expiring token, not a
//!   SQLite transaction held across IMAP awaits (gotcha #11).
//!
//! No FK to `messages`: the UID row is evicted on every save, and the draft
//! must outlive every one of its revisions.

use crate::error::AppError;
use chrono::{Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};

/// How long a claim holds without being advanced or released. Long enough for
/// a connect timeout (15 s), an APPEND and the UID lookup's retries; short
/// enough that a crashed writer does not lock the draft for a session.
pub const CLAIM_TTL_SECS: i64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRow {
    pub draft_id: String,
    pub account_id: String,
    pub folder_name: String,
    pub uidvalidity: u32,
    pub current_uid: u32,
}

/// What `claim` decided. Only `Claimed` permits an IMAP write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    /// `current_uid` is no longer the revision the caller read.
    Moved { current_uid: u32 },
    /// Another writer holds an unexpired claim on this revision.
    Busy { expires_at: String },
    /// No draft with that id.
    Missing,
}

/// The current revision's headline fields, for a conflict reply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DraftSnapshot {
    pub subject: Option<String>,
    pub to_list: Option<String>,
}

/// Split out of the migration so tests can build the table without replaying
/// 59 versions of history.
pub(crate) fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS drafts (
            draft_id         TEXT PRIMARY KEY,
            account_id       TEXT NOT NULL,
            folder_name      TEXT NOT NULL,
            uidvalidity      INTEGER NOT NULL,
            current_uid      INTEGER NOT NULL,
            claim_token      TEXT,
            claim_expires_at TEXT,
            created_at       TEXT NOT NULL,
            updated_at       TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_drafts_current
            ON drafts(account_id, folder_name, current_uid);",
    )
}

/// Both timestamps this module writes use one format, so the expiry compare in
/// `claim` is a plain string compare against a value we wrote ourselves —
/// never against SQLite's `datetime('now')` (gotcha #23).
fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn expiry_iso() -> String {
    (Utc::now() + Duration::seconds(CLAIM_TTL_SECS)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn get(conn: &Connection, draft_id: &str) -> Result<Option<DraftRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT draft_id, account_id, folder_name, uidvalidity, current_uid
             FROM drafts WHERE draft_id = ?1",
            params![draft_id],
            row_to_draft,
        )
        .optional()?;
    Ok(row)
}

/// The draft whose *current* revision is `uid` — the bridge for callers that
/// still address drafts by UID.
pub fn find_by_uid(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<DraftRow>, AppError> {
    let row = conn
        .query_row(
            "SELECT draft_id, account_id, folder_name, uidvalidity, current_uid
             FROM drafts WHERE account_id = ?1 AND folder_name = ?2 AND current_uid = ?3",
            params![account_id, folder, uid],
            row_to_draft,
        )
        .optional()?;
    Ok(row)
}

fn row_to_draft(r: &rusqlite::Row<'_>) -> rusqlite::Result<DraftRow> {
    Ok(DraftRow {
        draft_id: r.get(0)?,
        account_id: r.get(1)?,
        folder_name: r.get(2)?,
        uidvalidity: r.get::<_, i64>(3)? as u32,
        current_uid: r.get::<_, i64>(4)? as u32,
    })
}

/// Record that revision `uid` of `draft_id` exists in `folder` under
/// `uidvalidity`. Idempotent and order-independent: within one generation and
/// folder the pointer only ever moves to a HIGHER uid, so the sync can report
/// the old and the new revision in either order without regressing it. A
/// different generation or folder replaces the pointer outright.
pub fn link(
    conn: &Connection,
    draft_id: &str,
    account_id: &str,
    folder: &str,
    uidvalidity: u32,
    uid: u32,
) -> Result<(), AppError> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO drafts (draft_id, account_id, folder_name, uidvalidity, current_uid, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(draft_id) DO UPDATE SET
            current_uid = CASE
                WHEN excluded.uidvalidity <> drafts.uidvalidity
                  OR excluded.folder_name <> drafts.folder_name
                  OR excluded.account_id <> drafts.account_id
                THEN excluded.current_uid
                ELSE MAX(drafts.current_uid, excluded.current_uid)
            END,
            uidvalidity = excluded.uidvalidity,
            folder_name = excluded.folder_name,
            account_id  = excluded.account_id,
            updated_at  = excluded.updated_at",
        params![draft_id, account_id, folder, i64::from(uidvalidity), i64::from(uid), now],
    )?;
    Ok(())
}

/// Compare-and-set: take the write claim on `draft_id` if — and only if — its
/// current revision is still `expected_uid` and nobody else holds an
/// unexpired claim. One UPDATE, so two writers racing for the same revision
/// cannot both win. `token` is the caller's handle for `advance` / `release`.
pub fn claim(
    conn: &Connection,
    draft_id: &str,
    expected_uid: u32,
    token: &str,
) -> Result<ClaimOutcome, AppError> {
    let now = now_iso();
    let expires = expiry_iso();
    let changed = conn.execute(
        "UPDATE drafts
            SET claim_token = ?1, claim_expires_at = ?2, updated_at = ?3
          WHERE draft_id = ?4
            AND current_uid = ?5
            AND (claim_token IS NULL OR claim_expires_at IS NULL OR claim_expires_at < ?3)",
        params![token, expires, now, draft_id, i64::from(expected_uid)],
    )?;
    if changed == 1 {
        return Ok(ClaimOutcome::Claimed);
    }
    let row: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT current_uid, claim_expires_at FROM drafts WHERE draft_id = ?1",
            params![draft_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        None => ClaimOutcome::Missing,
        Some((cur, _)) if cur as u32 != expected_uid => ClaimOutcome::Moved {
            current_uid: cur as u32,
        },
        Some((_, expires_at)) => ClaimOutcome::Busy {
            expires_at: expires_at.unwrap_or_default(),
        },
    })
}

/// Give a claim back without advancing — the write did not happen. Only the
/// holder's token releases it.
pub fn release(conn: &Connection, draft_id: &str, token: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE drafts SET claim_token = NULL, claim_expires_at = NULL, updated_at = ?3
          WHERE draft_id = ?1 AND claim_token = ?2",
        params![draft_id, token, now_iso()],
    )?;
    Ok(())
}

/// The new revision is on the server: move the pointer and drop our claim.
/// Same forward-only rule as `link` — if our claim expired mid-flight and a
/// later writer already appended a higher UID, theirs stays current, and both
/// revisions are on the server for that writer's own old-UID delete to
/// resolve. Only our own claim is cleared; a claim someone else took after our
/// expiry is theirs.
pub fn advance(
    conn: &Connection,
    draft_id: &str,
    token: &str,
    uidvalidity: u32,
    new_uid: u32,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE drafts
            SET current_uid = CASE WHEN uidvalidity <> ?3 THEN ?4 ELSE MAX(current_uid, ?4) END,
                uidvalidity = ?3,
                claim_token = CASE WHEN claim_token = ?2 THEN NULL ELSE claim_token END,
                claim_expires_at = CASE WHEN claim_token = ?2 THEN NULL ELSE claim_expires_at END,
                updated_at = ?5
          WHERE draft_id = ?1",
        params![draft_id, token, i64::from(uidvalidity), i64::from(new_uid), now_iso()],
    )?;
    Ok(())
}

/// Headline fields of the revision at `uid`, for telling a losing writer what
/// it lost to. `None` when the local cache has no row for it (not synced yet).
pub fn snapshot(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<DraftSnapshot>, AppError> {
    let row = conn
        .query_row(
            "SELECT subject, to_list FROM messages
              WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            params![account_id, folder, uid],
            |r| {
                Ok(DraftSnapshot {
                    subject: r.get(0)?,
                    to_list: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        create_table(&c).unwrap();
        create_table(&c).unwrap(); // idempotent
        c
    }

    #[test]
    fn link_is_forward_only_within_a_generation_and_resets_across_one() {
        let c = conn();
        link(&c, "d1", "acct", "Drafts", 7, 10).unwrap();
        link(&c, "d1", "acct", "Drafts", 7, 12).unwrap();
        assert_eq!(get(&c, "d1").unwrap().unwrap().current_uid, 12);
        // The sync reporting the OLD revision after the new one must not regress it.
        link(&c, "d1", "acct", "Drafts", 7, 11).unwrap();
        assert_eq!(get(&c, "d1").unwrap().unwrap().current_uid, 12);
        // A new UIDVALIDITY generation restarts the numbering — a lower uid is newer.
        link(&c, "d1", "acct", "Drafts", 8, 3).unwrap();
        let row = get(&c, "d1").unwrap().unwrap();
        assert_eq!((row.uidvalidity, row.current_uid), (8, 3));
        // So does a different folder.
        link(&c, "d1", "acct", "Drafts/Old", 8, 1).unwrap();
        let row = get(&c, "d1").unwrap().unwrap();
        assert_eq!((row.folder_name.as_str(), row.current_uid), ("Drafts/Old", 1));
    }

    #[test]
    fn find_by_uid_only_matches_the_current_revision() {
        let c = conn();
        link(&c, "d1", "acct", "Drafts", 7, 10).unwrap();
        link(&c, "d1", "acct", "Drafts", 7, 12).unwrap();
        assert!(find_by_uid(&c, "acct", "Drafts", 10).unwrap().is_none());
        assert_eq!(
            find_by_uid(&c, "acct", "Drafts", 12).unwrap().unwrap().draft_id,
            "d1"
        );
    }

    #[test]
    fn claim_is_a_compare_and_set_on_the_current_uid() {
        let c = conn();
        link(&c, "d1", "acct", "Drafts", 7, 10).unwrap();
        // Wrong revision: refused, and told what the current one is.
        assert_eq!(
            claim(&c, "d1", 9, "tok-a").unwrap(),
            ClaimOutcome::Moved { current_uid: 10 }
        );
        assert_eq!(claim(&c, "nope", 10, "tok-a").unwrap(), ClaimOutcome::Missing);
        // First writer wins; the second, naming the same revision, is Busy.
        assert_eq!(claim(&c, "d1", 10, "tok-a").unwrap(), ClaimOutcome::Claimed);
        assert!(matches!(
            claim(&c, "d1", 10, "tok-b").unwrap(),
            ClaimOutcome::Busy { .. }
        ));
        // The winner advances; the loser, re-reading, now sees Moved.
        advance(&c, "d1", "tok-a", 7, 11).unwrap();
        assert_eq!(
            claim(&c, "d1", 10, "tok-b").unwrap(),
            ClaimOutcome::Moved { current_uid: 11 }
        );
        // And with the fresh revision, the claim is free again.
        assert_eq!(claim(&c, "d1", 11, "tok-b").unwrap(), ClaimOutcome::Claimed);
    }

    #[test]
    fn release_frees_only_the_holders_claim() {
        let c = conn();
        link(&c, "d1", "acct", "Drafts", 7, 10).unwrap();
        assert_eq!(claim(&c, "d1", 10, "tok-a").unwrap(), ClaimOutcome::Claimed);
        release(&c, "d1", "tok-b").unwrap(); // not the holder — no effect
        assert!(matches!(claim(&c, "d1", 10, "tok-b").unwrap(), ClaimOutcome::Busy { .. }));
        release(&c, "d1", "tok-a").unwrap();
        assert_eq!(claim(&c, "d1", 10, "tok-b").unwrap(), ClaimOutcome::Claimed);
    }

    #[test]
    fn an_expired_claim_is_reclaimable() {
        let c = conn();
        link(&c, "d1", "acct", "Drafts", 7, 10).unwrap();
        assert_eq!(claim(&c, "d1", 10, "tok-a").unwrap(), ClaimOutcome::Claimed);
        // Age the claim past its TTL the way a crashed writer would leave it.
        c.execute(
            "UPDATE drafts SET claim_expires_at = '2000-01-01T00:00:00.000Z' WHERE draft_id = 'd1'",
            [],
        )
        .unwrap();
        assert_eq!(claim(&c, "d1", 10, "tok-b").unwrap(), ClaimOutcome::Claimed);
        // The late writer's advance keeps the pointer forward-only and leaves
        // the new holder's claim alone.
        advance(&c, "d1", "tok-a", 7, 11).unwrap();
        assert!(matches!(claim(&c, "d1", 11, "tok-c").unwrap(), ClaimOutcome::Busy { .. }));
        assert_eq!(get(&c, "d1").unwrap().unwrap().current_uid, 11);
    }

    #[test]
    fn snapshot_reads_the_messages_row_and_tolerates_absence() {
        let c = conn();
        c.execute_batch(
            "CREATE TABLE messages (account_id TEXT, folder_name TEXT, uid INTEGER, subject TEXT, to_list TEXT);
             INSERT INTO messages VALUES ('acct', 'Drafts', 12, 'Re: the deck', '[\"dana@northwind.example\"]');",
        )
        .unwrap();
        assert!(snapshot(&c, "acct", "Drafts", 11).unwrap().is_none());
        let s = snapshot(&c, "acct", "Drafts", 12).unwrap().unwrap();
        assert_eq!(s.subject.as_deref(), Some("Re: the deck"));
        assert!(s.to_list.unwrap().contains("dana@"));
    }
}
