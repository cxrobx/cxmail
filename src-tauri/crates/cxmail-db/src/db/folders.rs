use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FolderRow {
    pub id: i64,
    pub account_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub folder_type: Option<String>,
    pub delimiter: Option<String>,
    pub total_count: i32,
    pub unread_count: i32,
    pub uidvalidity: Option<u32>,
    pub uidnext: Option<u32>,
}

pub fn upsert(
    conn: &Connection,
    account_id: &str,
    name: &str,
    display_name: Option<&str>,
    folder_type: &str,
    delimiter: Option<&str>,
    special_use: bool,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO folders (account_id, name, display_name, folder_type, delimiter, special_use)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id, name) DO UPDATE SET
            display_name = excluded.display_name,
            folder_type = excluded.folder_type,
            delimiter = excluded.delimiter,
            special_use = excluded.special_use",
        params![
            account_id,
            name,
            display_name,
            folder_type,
            delimiter,
            special_use as i32
        ],
    )?;
    Ok(())
}

/// The account's folder of a given type, chosen **deterministically**.
///
/// The `ORDER BY` is safety-critical, not polish. Both accessors this replaced
/// were `LIMIT 1` with no ordering at all, so with two candidate rows SQLite
/// could hand back either one — and duplicates are reachable today:
/// `classify_outlook_folder` accepts *both* "Deleted Items" and "Trash" as
/// trash, so an Outlook user who made their own `Trash` folder has two
/// `folder_type='trash'` rows. Routing `delete_messages` through the DB
/// without an ordering would then pick a destination at random.
///
/// `special_use DESC` first makes a server's RFC 6154 declaration outrank a
/// name guess — the correct precedence for exactly the provider this exists
/// for. `LENGTH(name)` then prefers the shallower mailbox (`Sent` over
/// `Sent/2019`), and `name` breaks any remaining tie so the answer is stable
/// across calls and across machines.
fn folder_of_type(conn: &Connection, account_id: &str, folder_type: &str) -> Option<String> {
    conn.query_row(
        "SELECT name FROM folders
          WHERE account_id = ?1 AND folder_type = ?2
          ORDER BY special_use DESC, LENGTH(name), name
          LIMIT 1",
        params![account_id, folder_type],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

pub fn drafts_folder_for_account(conn: &Connection, account_id: &str) -> Option<String> {
    folder_of_type(conn, account_id, "drafts")
}

/// The account's Sent folder, if the folder list has been synced.
///
/// Sent is not just an archive: follow-up nudges are computed from "my last
/// message in this thread", which is unknowable without it. Before this folder
/// joined the sync loop the newest local Sent message could sit days behind the
/// mailbox, which is exactly the window a nudge covers.
///
/// Because `db::nudges` depends on this answer, a change to how it resolves can
/// empty the nudge lane with no error anywhere. Verify against the real mailbox,
/// not only against unit tests.
pub fn sent_folder_for_account(conn: &Connection, account_id: &str) -> Option<String> {
    folder_of_type(conn, account_id, "sent")
}

/// The account's Archive folder, if the folder list has been synced.
pub fn archive_folder_for_account(conn: &Connection, account_id: &str) -> Option<String> {
    folder_of_type(conn, account_id, "archive")
}

/// The account's Trash folder, if the folder list has been synced.
pub fn trash_folder_for_account(conn: &Connection, account_id: &str) -> Option<String> {
    folder_of_type(conn, account_id, "trash")
}

/// The one predicate behind "this message is an unsent draft" — a SQL
/// fragment that is true when the folder named by `folder_expr` is a drafts
/// folder of the account named by `account_id_expr`. Every query that has to
/// tell a draft from a sent message embeds this (gotcha #36: one matcher),
/// with whatever expressions name the message's account and folder there
/// (`m.account_id`/`m.folder_name`, `messages.account_id`/…).
///
/// **`EXISTS`, so absence means "not a draft".** A message whose folder row
/// has not been synced yet counts as ordinary mail — the failure direction
/// that shows too much rather than hiding correspondence. It also means a
/// fixture with no `folders` table rows behaves exactly as it did before this
/// existed.
///
/// It matches on `folder_type`, never on the folder's NAME: a generic IMAP
/// account can call its drafts mailbox anything, and the classifier has
/// already resolved that at sync time. It is also deliberately not
/// `drafts_folder_for_account` — that accessor picks ONE folder, and an
/// account with two `folder_type='drafts'` rows (reachable; see
/// [`folder_of_type`]) would have the other one silently treated as sent mail.
///
/// The alias is `df` rather than `f` because callers already use `folders f`.
pub fn is_draft_sql(account_id_expr: &str, folder_expr: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM folders df WHERE df.account_id = {account_id_expr} AND df.name = {folder_expr} AND df.folder_type = 'drafts')"
    )
}

/// How much a folder is preferred to hold the canonical copy of a message,
/// for the ONE job of deduplicating a thread whose members appear in several
/// folders at once. Lower wins.
///
/// | rank | folders | why |
/// |---|---|---|
/// | 0 | INBOX, Sent, labels | where correspondence actually lives |
/// | 1 | drafts | a draft is real, but a Sent copy of the same message_id means it went out and this row is a stale un-expunged draft |
/// | 2 | archive | on Gmail, `[Gmail]/All Mail` MIRRORS everything, so it is a duplicate of some other row far more often than it is the only copy |
///
/// **This orders; it never filters.** A message that lives *only* in Archive
/// — the normal case on iCloud and Outlook, where Archive is a real
/// destination rather than Gmail's catch-all — is the sole row in its
/// partition and is kept untouched. Filtering on rank instead would delete
/// every archived message from its own thread.
///
/// The alias is `rf`, for the same reason `is_draft_sql` uses `df`.
pub fn folder_rank_sql(account_id_expr: &str, folder_expr: &str) -> String {
    format!(
        "(SELECT COALESCE(MAX(CASE rf.folder_type WHEN 'archive' THEN 2 WHEN 'drafts' THEN 1 ELSE 0 END), 0)          FROM folders rf WHERE rf.account_id = {account_id_expr} AND rf.name = {folder_expr})"
    )
}

/// The name to use for a special folder when the folder list has not been
/// synced yet — a brand-new account, or an account whose folder sync failed.
///
/// This is the historical hardcoded table, verbatim, in ONE place. It used to
/// be spelled out at ten call sites across `commands::messages`,
/// `commands::compose` and `mcp::server`, which is why `sent` disagreed
/// between them (only the sync loop knew iCloud calls it "Sent Messages").
///
/// It is a fallback, never the primary answer: it cannot know where a generic
/// IMAP server keeps anything. Always try [`folder_for_account`] first.
pub fn fallback_folder_name(provider: &str, folder_type: &str) -> String {
    match (folder_type, provider) {
        ("drafts", "gmail") => "[Gmail]/Drafts",
        ("drafts", _) => "Drafts",
        ("sent", "gmail") => "[Gmail]/Sent Mail",
        ("sent", "icloud") => "Sent Messages",
        ("sent", _) => "Sent",
        ("archive", "gmail") => "[Gmail]/All Mail",
        ("archive", _) => "Archive",
        ("trash", "gmail") => "[Gmail]/Trash",
        ("trash", "outlook") => "Deleted Items",
        ("trash", _) => "Trash",
        ("spam", "gmail") => "[Gmail]/Spam",
        ("spam", _) => "Junk",
        (_, _) => "INBOX",
    }
    .to_string()
}

/// The account's folder of the given type: the synced folder list first,
/// [`fallback_folder_name`] second. This is what the ten call sites should use.
pub fn folder_for_account(
    conn: &Connection,
    account_id: &str,
    provider: &str,
    folder_type: &str,
) -> String {
    folder_of_type(conn, account_id, folder_type)
        .unwrap_or_else(|| fallback_folder_name(provider, folder_type))
}

pub fn list_by_account(conn: &Connection, account_id: &str) -> Result<Vec<FolderRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, name, display_name, folder_type, delimiter, total_count, unread_count, uidvalidity, uidnext
         FROM folders WHERE account_id = ?1 ORDER BY folder_type, name",
    )?;
    let folders = stmt
        .query_map(params![account_id], |row| {
            Ok(FolderRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                display_name: row.get(3)?,
                folder_type: row.get(4)?,
                delimiter: row.get(5)?,
                total_count: row.get(6)?,
                unread_count: row.get(7)?,
                uidvalidity: row.get::<_, Option<u32>>(8)?,
                uidnext: row.get::<_, Option<u32>>(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(folders)
}

pub fn update_counts(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    unread_count: i32,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE folders SET unread_count = ?1 WHERE account_id = ?2 AND name = ?3",
        params![unread_count, account_id, folder_name],
    )?;
    Ok(())
}

pub fn delete(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, folder_name],
    )?;
    Ok(())
}

pub fn update_sync_info(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uidvalidity: u32,
    uidnext: u32,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE folders SET uidvalidity = ?1, uidnext = ?2, last_synced = datetime('now') WHERE account_id = ?3 AND name = ?4",
        params![uidvalidity, uidnext, account_id, folder_name],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE folders (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id TEXT NOT NULL,
                 name TEXT NOT NULL,
                 display_name TEXT,
                 folder_type TEXT,
                 delimiter TEXT,
                 special_use INTEGER NOT NULL DEFAULT 0,
                 UNIQUE(account_id, name)
             );",
        )
        .unwrap();
        conn
    }

    /// `classify_outlook_folder` accepts BOTH "Deleted Items" and "Trash", so
    /// an Outlook user with a self-made `Trash` folder really does get two
    /// `folder_type='trash'` rows. The accessors this replaced were `LIMIT 1`
    /// with no ORDER BY, which makes the delete destination a coin flip.
    #[test]
    fn two_candidates_resolve_the_same_way_every_time() {
        let conn = setup();
        upsert(&conn, "acct", "Deleted Items", None, "trash", None, false).unwrap();
        upsert(&conn, "acct", "Trash", None, "trash", None, false).unwrap();

        let first = trash_folder_for_account(&conn, "acct").unwrap();
        for _ in 0..25 {
            assert_eq!(trash_folder_for_account(&conn, "acct").unwrap(), first);
        }
        // Shortest name wins the tie — "Trash" (5) over "Deleted Items" (13).
        assert_eq!(first, "Trash");
    }

    /// A server that DECLARED the folder outranks one we merely guessed at,
    /// regardless of name length. This is the precedence that matters for the
    /// provider the feature exists for.
    #[test]
    fn a_declared_special_use_outranks_a_name_guess() {
        let conn = setup();
        // Shorter name, but only guessed.
        upsert(&conn, "acct", "Sent", None, "sent", None, false).unwrap();
        // Longer name, but the server said so.
        upsert(&conn, "acct", "INBOX.Sent Items", None, "sent", Some("."), true).unwrap();

        assert_eq!(
            sent_folder_for_account(&conn, "acct").unwrap(),
            "INBOX.Sent Items"
        );
    }

    #[test]
    fn ties_break_on_name_so_equal_length_is_still_stable() {
        let conn = setup();
        upsert(&conn, "acct", "Binn", None, "trash", None, false).unwrap();
        upsert(&conn, "acct", "Aash", None, "trash", None, false).unwrap();
        assert_eq!(trash_folder_for_account(&conn, "acct").unwrap(), "Aash");
    }

    #[test]
    fn an_unsynced_account_has_no_answer() {
        let conn = setup();
        assert_eq!(sent_folder_for_account(&conn, "acct"), None);
        assert_eq!(trash_folder_for_account(&conn, "acct"), None);
        assert_eq!(archive_folder_for_account(&conn, "acct"), None);
        assert_eq!(drafts_folder_for_account(&conn, "acct"), None);
    }

    /// The fallback table must reproduce what the ten call sites hardcoded
    /// before they were consolidated — a difference here is a silently changed
    /// destination for archive/delete/draft-save on an existing account.
    #[test]
    fn the_fallback_table_matches_what_the_call_sites_hardcoded() {
        assert_eq!(fallback_folder_name("gmail", "drafts"), "[Gmail]/Drafts");
        assert_eq!(fallback_folder_name("icloud", "drafts"), "Drafts");
        assert_eq!(fallback_folder_name("outlook", "drafts"), "Drafts");
        assert_eq!(fallback_folder_name("imap", "drafts"), "Drafts");

        assert_eq!(fallback_folder_name("gmail", "sent"), "[Gmail]/Sent Mail");
        assert_eq!(fallback_folder_name("icloud", "sent"), "Sent Messages");
        assert_eq!(fallback_folder_name("outlook", "sent"), "Sent");

        assert_eq!(fallback_folder_name("gmail", "archive"), "[Gmail]/All Mail");
        assert_eq!(fallback_folder_name("icloud", "archive"), "Archive");

        assert_eq!(fallback_folder_name("gmail", "trash"), "[Gmail]/Trash");
        assert_eq!(fallback_folder_name("outlook", "trash"), "Deleted Items");
        assert_eq!(fallback_folder_name("icloud", "trash"), "Trash");
        assert_eq!(fallback_folder_name("imap", "trash"), "Trash");
    }

    #[test]
    fn the_db_answer_beats_the_fallback_but_the_fallback_covers_the_gap() {
        let conn = setup();
        // Before any folder sync: the historical name.
        assert_eq!(
            folder_for_account(&conn, "acct", "gmail", "archive"),
            "[Gmail]/All Mail"
        );
        // After: whatever the server actually has. This is the whole point for
        // a generic host, whose archive we cannot possibly have hardcoded.
        upsert(&conn, "acct", "Archives", None, "archive", None, true).unwrap();
        assert_eq!(folder_for_account(&conn, "acct", "imap", "archive"), "Archives");
    }

    #[test]
    fn upsert_round_trips_special_use_and_updates_it_on_conflict() {
        let conn = setup();
        upsert(&conn, "acct", "Sent", None, "sent", None, false).unwrap();
        let flag: i64 = conn
            .query_row("SELECT special_use FROM folders WHERE name='Sent'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(flag, 0);

        // A later sync where the server volunteered \Sent must upgrade the row,
        // not leave it looking like a guess forever.
        upsert(&conn, "acct", "Sent", None, "sent", None, true).unwrap();
        let flag: i64 = conn
            .query_row("SELECT special_use FROM folders WHERE name='Sent'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(flag, 1);
    }

    /// One matcher (gotcha #36). rusqlite is built without `trace`, so the
    /// only way to prove no query hand-rolls its own "is this a draft?" is to
    /// read the source: `folder_type` must be compared to `'drafts'` in this
    /// module and nowhere else, and the modules that need the answer must
    /// still be asking for it. A second spelling drifts the moment one of
    /// them changes — and the two directions fail oppositely and silently: a
    /// draft counted as a message puts a wrong number in the list badge, a
    /// sent message mistaken for a draft hides real correspondence behind a
    /// "Draft" label. (Test modules may name it freely; fixture DDL does.)
    #[test]
    fn the_draft_rule_is_spelled_in_exactly_one_place() {
        let modules: [(&str, &str); 2] = [
            ("messages", include_str!("messages.rs")),
            ("inbox_groups", include_str!("inbox_groups.rs")),
        ];
        for (name, src) in modules {
            let production = src
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first piece");
            assert!(
                !production.contains("'drafts'"),
                "db/{name}.rs spells the draft folder_type itself — use folders::is_draft_sql()"
            );
        }
        let messages = include_str!("messages.rs");
        let production = messages.split("#[cfg(test)]").next().unwrap();
        assert!(
            production.contains("is_draft_sql("),
            "db/messages.rs no longer excludes unsent drafts from the thread count"
        );
        assert!(
            production.contains("folder_rank_sql("),
            "db/messages.rs no longer deduplicates a thread's folder copies"
        );
    }

    /// The rank is what decides which copy of a message survives dedupe, and
    /// each tier is load-bearing in a different direction — see
    /// [`folder_rank_sql`].
    #[test]
    fn folder_rank_orders_ordinary_then_drafts_then_archive() {
        let conn = setup();
        upsert(&conn, "acct", "INBOX", None, "inbox", None, false).unwrap();
        upsert(&conn, "acct", "[Gmail]/Sent Mail", None, "sent", None, false).unwrap();
        upsert(&conn, "acct", "[Gmail]/Drafts", None, "drafts", None, false).unwrap();
        upsert(&conn, "acct", "[Gmail]/All Mail", None, "archive", None, false).unwrap();

        let rank = |folder: &str| -> i64 {
            conn.query_row(
                &format!("SELECT {}", folder_rank_sql("'acct'", "?1")),
                params![folder],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(rank("INBOX"), 0);
        assert_eq!(rank("[Gmail]/Sent Mail"), 0);
        assert_eq!(rank("[Gmail]/Drafts"), 1);
        assert_eq!(rank("[Gmail]/All Mail"), 2);
        // A folder the sync has never seen ranks with ordinary mail rather
        // than falling to NULL, which would sort unpredictably.
        assert_eq!(rank("Some/Unsynced/Label"), 0);
    }

    /// `is_draft_sql` is an EXISTS, so an unsynced folder list means "not a
    /// draft" — the failure direction that shows mail rather than hiding it.
    #[test]
    fn an_unknown_folder_is_not_a_draft() {
        let conn = setup();
        upsert(&conn, "acct", "[Gmail]/Drafts", None, "drafts", None, false).unwrap();
        let is_draft = |folder: &str| -> i64 {
            conn.query_row(
                &format!("SELECT {}", is_draft_sql("'acct'", "?1")),
                params![folder],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(is_draft("[Gmail]/Drafts"), 1);
        assert_eq!(is_draft("INBOX"), 0);
        assert_eq!(is_draft("Never/Synced"), 0);
        // Another account's drafts folder is not this account's.
        assert_eq!(
            conn.query_row(
                &format!("SELECT {}", is_draft_sql("'other'", "'[Gmail]/Drafts'")),
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
