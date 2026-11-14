//! A draft's revisions as the SERVER sees them (T666).
//!
//! `db::drafts` is each Mac's own belief about which UID is a draft's current
//! revision. It is fed by that Mac's writers and its folder sync, so it lags
//! whatever another machine wrote since. Two CXMail installs (or the app on
//! one Mac and `cxmail-mcp` on another) editing one draft used to trust that
//! belief: one Mac's compose window autosaved its stale text as a fresh
//! revision over an agent's edit from another Mac, and that agent's next
//! `edit_draft` by `draft_id` failed on a UID the window had already retired.
//!
//! The fix is to ask the server before every write. Every revision CXMail
//! writes carries `X-CXMail-Draft-Id` (`draft_local::DRAFT_ID_HEADER`), and
//! inside one UIDVALIDITY generation UIDs only ascend, so the highest UID in
//! the Drafts folder carrying a draft's id IS its newest revision, whoever
//! wrote it. This module is that one question (`scan`) plus the two pure rules
//! every writer applies to the answer (`resolve`, `superseded`).
//!
//! The scan fetches only the header field, for the whole folder, and matches
//! client-side. It deliberately does not use `UID SEARCH HEADER`: Gmail's
//! IMAP search runs on its own index, and a custom `X-` header is not
//! something to bet a draft on. A Drafts folder is small, so the fetch is too.

use crate::email::draft_local::DRAFT_ID_HEADER;
use crate::email::imap::ImapSession;
use crate::error::AppError;
use futures::StreamExt;
use std::collections::BTreeMap;

/// Every CXMail draft revision in the currently relevant folder, grouped by
/// draft id: `draft_id -> UIDs ascending`. Revisions without the header
/// (another client's drafts, or a draft Gmail's web composer rebuilt) are not
/// in it: they degrade to "unknown", never to a wrong match.
pub type DraftIndex = BTreeMap<String, Vec<u32>>;

/// What the folder held when it was scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftScan {
    pub uidvalidity: u32,
    pub uidnext: u32,
    pub drafts: DraftIndex,
}

impl DraftScan {
    /// UIDs of `draft_id`, ascending. Empty when the server has none.
    pub fn revisions(&self, draft_id: &str) -> &[u32] {
        self.drafts.get(draft_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The draft id that revision `uid` carries, if any.
    pub fn draft_id_of(&self, uid: u32) -> Option<&str> {
        self.drafts
            .iter()
            .find(|(_, uids)| uids.contains(&uid))
            .map(|(id, _)| id.as_str())
    }
}

/// SELECT `folder` and read every message's `X-CXMail-Draft-Id`. Leaves
/// `folder` selected, so a caller can APPEND / STORE straight after.
pub async fn scan(session: &mut ImapSession, folder: &str) -> Result<DraftScan, AppError> {
    let mailbox = session
        .select(folder)
        .await
        .map_err(|e| AppError::Imap(format!("SELECT {folder} failed: {e}")))?;
    let uidvalidity = mailbox
        .uid_validity
        .ok_or_else(|| AppError::Imap("No UIDVALIDITY in SELECT response".to_string()))?;
    let uidnext = mailbox.uid_next.unwrap_or(0);
    let mut drafts = DraftIndex::new();
    // `1:*` on an empty mailbox is a BAD on some servers; nothing to read anyway.
    if mailbox.exists == 0 {
        return Ok(DraftScan { uidvalidity, uidnext, drafts });
    }
    let field = DRAFT_ID_HEADER.to_ascii_uppercase();
    let mut stream = session
        .uid_fetch("1:*", format!("(UID BODY.PEEK[HEADER.FIELDS ({field})])"))
        .await
        .map_err(|e| AppError::Imap(format!("FETCH draft ids in {folder} failed: {e}")))?;
    while let Some(item) = stream.next().await {
        let fetch = item.map_err(|e| AppError::Imap(format!("FETCH draft ids item: {e}")))?;
        let Some(uid) = fetch.uid else { continue };
        let text = fetch
            .header()
            .and_then(|h| std::str::from_utf8(h).ok())
            .unwrap_or("");
        if let Some(id) = parse_draft_id(text) {
            drafts.entry(id).or_default().push(uid);
        }
    }
    for uids in drafts.values_mut() {
        uids.sort_unstable();
        uids.dedup();
    }
    Ok(DraftScan { uidvalidity, uidnext, drafts })
}

/// The draft id in a `HEADER.FIELDS (X-CXMAIL-DRAFT-ID)` response, if present.
/// Header names are case-insensitive (RFC 5322 §1.2.2).
pub fn parse_draft_id(header_text: &str) -> Option<String> {
    let name = DRAFT_ID_HEADER.to_ascii_lowercase();
    header_text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim().to_ascii_lowercase() == name)
            .then(|| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

/// Which revision a writer should replace, given what the server holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Write over this UID. Equal to `expected` unless the caller asked to
    /// follow the newest revision and the server had moved on.
    Write(u32),
    /// The server's newest revision is not the one the caller named, and the
    /// caller asked for exactly that one: refuse, before any APPEND.
    Moved { current: u32 },
    /// No revision of this draft carries the header. The caller falls back to
    /// its own existence check on `expected`.
    Unknown,
}

/// `revisions` ascending (from `DraftScan::revisions`). `follow_newest` is
/// true for a writer that addressed the logical draft (`draft_id`, no
/// `expected_uid`) and false for one holding a specific revision: the
/// compose window's autosave, or an agent that named a UID.
pub fn resolve(revisions: &[u32], expected: u32, follow_newest: bool) -> Resolution {
    let Some(&newest) = revisions.last() else {
        return Resolution::Unknown;
    };
    if newest == expected || follow_newest {
        Resolution::Write(newest)
    } else {
        Resolution::Moved { current: newest }
    }
}

/// The revisions to retire once `new_uid` is on the server: the one replaced
/// plus every older revision of the same draft a lost race left behind, and
/// never anything at or above `new_uid` (a revision another writer appended
/// after ours is newer than ours, and is theirs to retire).
pub fn superseded(revisions: &[u32], replaced: u32, new_uid: u32) -> Vec<u32> {
    let mut out: Vec<u32> = revisions
        .iter()
        .copied()
        .chain(std::iter::once(replaced))
        .filter(|&u| u < new_uid)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Point this Mac's `drafts` rows at the newest revision `scan` saw of every
/// draft in it. `db::drafts::link` is forward-only within a generation, so
/// this never regresses a row, and it creates the row for a draft another
/// machine started that this one has not synced yet.
pub fn link_scan(
    conn: &rusqlite::Connection,
    account_id: &str,
    folder: &str,
    scan: &DraftScan,
) {
    for (id, uids) in &scan.drafts {
        if let Some(&newest) = uids.last() {
            if let Err(e) =
                crate::db::drafts::link(conn, id, account_id, folder, scan.uidvalidity, newest)
            {
                log::warn!("draft scan: linking {id} to UID {newest} in {folder} failed: {e}");
            }
        }
    }
}

/// Retire `uids` from `folder`, one targeted `UID EXPUNGE` each (gotcha #57:
/// a mailbox-wide EXPUNGE would also purge what another client marked
/// `\Deleted`). Returns the UIDs actually retired, so the caller evicts only
/// those from its cache. Failures are logged, never fatal: a revision left
/// behind is a duplicate, and the next save's `superseded` retires it.
pub async fn retire(session: &mut ImapSession, folder: &str, uids: &[u32]) -> Vec<u32> {
    if uids.is_empty() {
        return Vec::new();
    }
    if let Err(e) = crate::email::imap::select_folder(session, folder).await {
        log::warn!("draft retire: SELECT {folder} failed: {e}");
        return Vec::new();
    }
    let mut retired = Vec::with_capacity(uids.len());
    for &uid in uids {
        let r = match crate::email::imap::store_flags(session, uid, true, "\\Deleted").await {
            Ok(()) => crate::email::imap::uid_expunge(session, uid).await,
            Err(e) => Err(e),
        };
        match r {
            Ok(()) => retired.push(uid),
            Err(e) => log::warn!("draft retire: UID {uid} in {folder} not retired: {e}"),
        }
    }
    retired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_header_case_insensitively_and_ignores_others() {
        let text = "x-cxmail-draft-id:  7f3e2a10-0001 \r\n\r\n";
        assert_eq!(parse_draft_id(text).as_deref(), Some("7f3e2a10-0001"));
        assert_eq!(
            parse_draft_id("X-CXMail-Draft-Id: d1\r\n").as_deref(),
            Some("d1")
        );
        assert_eq!(parse_draft_id("Subject: hi\r\n\r\n"), None);
        assert_eq!(parse_draft_id("X-CXMail-Draft-Id:   \r\n"), None);
        assert_eq!(parse_draft_id(""), None);
    }

    #[test]
    fn the_newest_revision_is_current_and_a_stale_writer_is_refused() {
        // The 2026-11-13 failure: Mac B held 1128, Mac A wrote 1129.
        assert_eq!(resolve(&[1128, 1129], 1129, false), Resolution::Write(1129));
        assert_eq!(
            resolve(&[1128, 1129], 1128, false),
            Resolution::Moved { current: 1129 }
        );
        // Our revision is already gone and only a newer one is left.
        assert_eq!(resolve(&[1130], 1127, false), Resolution::Moved { current: 1130 });
    }

    #[test]
    fn a_writer_addressing_the_logical_draft_follows_the_newest() {
        // Mac A's edit_draft by draft_id, with its cache still on 1129.
        assert_eq!(resolve(&[1130], 1129, true), Resolution::Write(1130));
        assert_eq!(resolve(&[1129], 1129, true), Resolution::Write(1129));
    }

    #[test]
    fn no_tagged_revision_is_unknown_not_a_match() {
        assert_eq!(resolve(&[], 10, true), Resolution::Unknown);
        assert_eq!(resolve(&[], 10, false), Resolution::Unknown);
    }

    #[test]
    fn superseded_takes_everything_older_and_nothing_newer() {
        // Leftover duplicates from the race are retired with the replaced one.
        assert_eq!(superseded(&[1128, 1130], 1130, 1131), vec![1128, 1130]);
        // The replaced UID is retired even when it carries no header.
        assert_eq!(superseded(&[], 42, 43), vec![42]);
        // A revision appended after ours is newer and stays.
        assert_eq!(superseded(&[10, 12, 14], 12, 13), vec![10, 12]);
        // Never the new revision itself.
        assert_eq!(superseded(&[10, 11], 10, 11), vec![10]);
    }

    #[test]
    fn scan_lookups() {
        let mut drafts = DraftIndex::new();
        drafts.insert("a".into(), vec![3, 7]);
        drafts.insert("b".into(), vec![5]);
        let s = DraftScan { uidvalidity: 1, uidnext: 8, drafts };
        assert_eq!(s.revisions("a"), &[3, 7]);
        assert_eq!(s.revisions("zzz"), &[] as &[u32]);
        assert_eq!(s.draft_id_of(5), Some("b"));
        assert_eq!(s.draft_id_of(4), None);
    }
}
