//! Local opt-ins and reply suggestions. Suggestions never write mail.
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountOptions {
    pub account_id: String,
    pub auto_draft: bool,
    pub semantic_search: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplySuggestion {
    pub body: String,
    pub created_at: String,
    pub model: String,
}

pub(crate) fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS intelligence_accounts (
        account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        auto_draft INTEGER NOT NULL DEFAULT 0,
        semantic_search INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS reply_suggestions (
        message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
        snapshot TEXT NOT NULL, body TEXT NOT NULL, model TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'ready',
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS intelligence_usage (
        day TEXT NOT NULL, kind TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(day, kind)
    );",
    )
}

pub fn options(conn: &Connection) -> Result<Vec<AccountOptions>, AppError> {
    Ok(conn
        .prepare(
            "SELECT a.id, COALESCE(i.auto_draft,0), COALESCE(i.semantic_search,0)
        FROM accounts a LEFT JOIN intelligence_accounts i ON i.account_id=a.id ORDER BY a.email",
        )?
        .query_map([], |r| {
            Ok(AccountOptions {
                account_id: r.get(0)?,
                auto_draft: r.get(1)?,
                semantic_search: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn save_options(conn: &Connection, options: &AccountOptions) -> Result<(), AppError> {
    if !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",
        [&options.account_id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(AppError::NotFound("Account not found".into()));
    }
    conn.execute("INSERT INTO intelligence_accounts(account_id,auto_draft,semantic_search) VALUES (?1,?2,?3)
        ON CONFLICT(account_id) DO UPDATE SET auto_draft=excluded.auto_draft,semantic_search=excluded.semantic_search",
        params![options.account_id,options.auto_draft,options.semantic_search])?;
    Ok(())
}

/// The thread members a suggestion was written against: the message, its
/// thread, and direct replies to it. Three lookups joined by UNION, each on an
/// index; the OR + `trim(x.in_reply_to)` form scanned the whole account for
/// every Needs You item (gotcha #30), and the unary `+` keeps SQLite on the
/// in_reply_to index when there are no ANALYZE statistics. Same members, same
/// order, so the fingerprint and every stored suggestion are unchanged.
const SNAPSHOT_SQL: &str = "WITH m AS (SELECT id, account_id, thread_root_id, message_id FROM messages WHERE id=?1),
    members AS (
        SELECT id FROM m
        UNION SELECT x.id FROM messages x, m
              WHERE m.thread_root_id IS NOT NULL AND x.account_id=m.account_id AND x.thread_root_id=m.thread_root_id
        UNION SELECT x.id FROM messages x, m
              WHERE m.message_id IS NOT NULL
                AND x.in_reply_to IN ('<'||trim(m.message_id,'<>')||'>', trim(m.message_id,'<>'))
                AND +x.account_id=m.account_id)
    SELECT x.id, x.date, COALESCE(b.plain_text,'') FROM members JOIN messages x ON x.id=members.id
    JOIN folders f ON f.account_id=x.account_id AND f.name=x.folder_name
    LEFT JOIN message_bodies b ON b.account_id=x.account_id AND b.folder_name=x.folder_name AND b.uid=x.uid
    WHERE f.folder_type NOT IN ('drafts','trash','junk') ORDER BY x.id";

/// A fingerprint includes every non-draft member, so a new response or a body
/// arriving after header sync invalidates an old suggestion.
pub fn snapshot(conn: &Connection, message_id: i64) -> Result<String, AppError> {
    let mut stmt = conn.prepare(SNAPSHOT_SQL)?;
    let rows = stmt.query_map([message_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut hash = 0xcbf29ce484222325u64;
    for row in rows {
        let (id, date, body) = row?;
        for byte in format!("{id}\0{date}\0{body}\0").bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

pub fn suggestion(conn: &Connection, id: i64) -> Result<Option<ReplySuggestion>, AppError> {
    let snapshot = snapshot(conn, id)?;
    Ok(conn.query_row("SELECT s.body,s.created_at,s.model FROM reply_suggestions s
        JOIN messages m ON m.id=s.message_id JOIN intelligence_accounts i ON i.account_id=m.account_id
        WHERE s.message_id=?1 AND s.snapshot=?2 AND s.status='ready' AND i.auto_draft=1",
        params![id,snapshot],|r|Ok(ReplySuggestion{body:r.get(0)?,created_at:r.get(1)?,model:r.get(2)?})).optional()?)
}

pub fn already_generated(conn: &Connection, id: i64, snapshot: &str) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM reply_suggestions WHERE message_id=?1 AND snapshot=?2)",
        params![id, snapshot],
        |r| r.get(0),
    )?)
}

/// Compare the snapshot again after inference: late replies must never publish
/// a suggestion about a thread the provider did not see.
pub fn save_suggestion(
    conn: &Connection,
    id: i64,
    expected: &str,
    body: &str,
    model: &str,
) -> Result<bool, AppError> {
    if body.trim().is_empty() || body.len() > 30_000 {
        return Err(AppError::General("Invalid reply suggestion".into()));
    }
    if snapshot(conn, id)? != expected {
        return Ok(false);
    }
    conn.execute("INSERT INTO reply_suggestions(message_id,snapshot,body,model) VALUES(?1,?2,?3,?4)
        ON CONFLICT(message_id) DO UPDATE SET snapshot=excluded.snapshot,body=excluded.body,model=excluded.model,status='ready',created_at=datetime('now')",
        params![id,expected,body,model])?;
    Ok(true)
}

pub fn dismiss_suggestion(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE reply_suggestions SET status='dismissed' WHERE message_id=?1",
        [id],
    )?;
    Ok(())
}

/// Reserve before sending a request. Failures also count, so a broken provider
/// cannot turn a two-minute tick into unlimited paid retries.
pub fn reserve_usage(
    conn: &Connection,
    kind: &str,
    count: i64,
    cap: i64,
) -> Result<bool, AppError> {
    conn.execute(
        "INSERT OR IGNORE INTO intelligence_usage(day,kind,count) VALUES(date('now'),?1,0)",
        [kind],
    )?;
    Ok(conn.execute("UPDATE intelligence_usage SET count=count+?2 WHERE day=date('now') AND kind=?1 AND count+?2<=?3",params![kind,count,cap])?==1)
}

pub fn message_id(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT id FROM messages WHERE account_id=?1 AND folder_name=?2 AND uid=?3",
        params![account, folder, uid],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound("Message not found".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_never_scans_the_account() {
        let c = Connection::open_in_memory().unwrap();
        super::super::schema::initialize(&c).unwrap();
        let plan: Vec<String> = c
            .prepare(&format!("EXPLAIN QUERY PLAN {SNAPSHOT_SQL}"))
            .unwrap()
            .query_map([1], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let lookups: Vec<_> = plan.iter().filter(|l| l.contains(" x ")).collect();
        assert!(
            lookups.iter().all(|l| !l.starts_with("SCAN")),
            "a member lookup scans messages: {plan:?}"
        );
        for key in ["thread_root_id=?", "in_reply_to=?"] {
            assert!(
                lookups.iter().any(|l| l.contains(key)),
                "no lookup uses {key}: {plan:?}"
            );
        }
    }
    #[test]
    fn cap_counts_failed_attempts_and_never_overruns() {
        let c = Connection::open_in_memory().unwrap();
        super::super::schema::initialize(&c).unwrap();
        assert!(reserve_usage(&c, "draft", 2, 3).unwrap());
        assert!(!reserve_usage(&c, "draft", 2, 3).unwrap());
        assert!(reserve_usage(&c, "draft", 1, 3).unwrap());
        assert!(!reserve_usage(&c, "draft", 1, 3).unwrap());
    }
}
