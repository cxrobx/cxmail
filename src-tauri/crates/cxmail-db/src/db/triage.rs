//! Storage for the AI triage pass: which messages still need a verdict, and
//! the verdicts themselves.
//!
//! `triage_verdicts` is a side table on the message triple with `ON DELETE
//! CASCADE` — the `claude_repos` shape, not the `zoom_meetings` one (gotcha
//! #44), because a verdict on a message that no longer exists means nothing
//! and must not survive as a tombstone. One row per message: a re-run under a
//! new `prompt_version` REPLACES the old verdict rather than sitting beside it,
//! so the table never mixes two prompts' opinions of the same mail.
//!
//! The pass runs in the app only. The MCP and the helper get the table through
//! `initialize` like every other migration and never write to it.

use crate::error::AppError;
use cxmail_core::mail::triage::TriageVerdict;
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS triage_verdicts (
            account_id     TEXT    NOT NULL,
            folder_name    TEXT    NOT NULL,
            uid            INTEGER NOT NULL,
            needs_response REAL    NOT NULL,
            needs_action   REAL    NOT NULL,
            category       TEXT    NOT NULL,
            urgency        INTEGER NOT NULL,
            model          TEXT    NOT NULL,
            input_tokens   INTEGER NOT NULL DEFAULT 0,
            output_tokens  INTEGER NOT NULL DEFAULT 0,
            prompt_version TEXT    NOT NULL,
            -- Non-NULL means the gate refused to send this message. The row
            -- exists ONLY so the pass stops re-picking it; it is not a verdict
            -- and `get` never returns one for it.
            withheld_reason TEXT,
            created_at     TEXT    NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, folder_name, uid),
            FOREIGN KEY (account_id, folder_name, uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );",
    )
}

/// A message awaiting a verdict, with everything the gate needs to build a
/// payload. Body is the cached plain text; the query guarantees it is present.
#[derive(Debug, Clone)]
pub struct PendingMessage {
    pub account_id: String,
    pub folder_name: String,
    pub uid: u32,
    pub from_email: String,
    pub from_name: Option<String>,
    pub subject: Option<String>,
    pub to_list: Option<String>,
    pub cc_list: Option<String>,
    pub date: String,
    pub body: String,
}

/// Inbox messages on triage-enabled accounts that have a cached body and no
/// verdict under `prompt_version`, newest first.
///
/// Newest first is deliberate: the queue is about what needs the user NOW, and
/// a spend cap that cuts a backfill short should leave the recent mail done
/// and the old mail waiting, never the reverse.
pub fn list_pending(
    conn: &Connection,
    prompt_version: &str,
    limit: usize,
) -> Result<Vec<PendingMessage>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT m.account_id, m.folder_name, m.uid, m.from_email, m.from_name, m.subject,
                m.to_list, m.cc_list, m.date, b.plain_text
           FROM messages m
           JOIN accounts a ON a.id = m.account_id AND a.triage_enabled = 1
           JOIN folders f  ON f.account_id = m.account_id AND f.name = m.folder_name
                          AND f.folder_type = 'inbox'
           JOIN message_bodies b ON b.account_id = m.account_id
                                AND b.folder_name = m.folder_name AND b.uid = m.uid
          WHERE b.plain_text IS NOT NULL AND trim(b.plain_text) <> ''
            AND NOT EXISTS (SELECT 1 FROM triage_verdicts v
                             WHERE v.account_id = m.account_id
                               AND v.folder_name = m.folder_name
                               AND v.uid = m.uid
                               AND v.prompt_version = ?1)
          ORDER BY m.date DESC
          LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![prompt_version, limit as i64], |r| {
        Ok(PendingMessage {
            account_id: r.get(0)?,
            folder_name: r.get(1)?,
            uid: r.get(2)?,
            from_email: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            from_name: r.get(4)?,
            subject: r.get(5)?,
            to_list: r.get(6)?,
            cc_list: r.get(7)?,
            date: r.get(8)?,
            body: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Record that the gate refused to send this message.
///
/// Without this the pass livelocks: withheld messages stayed pending, and
/// `list_pending` is newest-first with a small limit, so the same five were
/// re-picked every two minutes forever and nothing behind them was ever
/// reached. Observed live — `0 classified, 4 withheld, 1 failed`, identical on
/// every pass.
///
/// It is deliberately NOT a verdict. `get` filters these out, so a withheld
/// message reaches Needs You as nothing at all rather than as "no reply
/// needed" — the distinction the whole feature rests on.
pub fn mark_withheld(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    reason: &str,
    prompt_version: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO triage_verdicts
            (account_id, folder_name, uid, needs_response, needs_action, category,
             urgency, model, prompt_version, withheld_reason)
         VALUES (?1, ?2, ?3, 0, 0, 'unknown', 0, '', ?4, ?5)",
        params![account_id, folder_name, uid, prompt_version, reason],
    )?;
    Ok(())
}

/// Store (or replace) a verdict.
#[allow(clippy::too_many_arguments)]
pub fn upsert(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    v: &TriageVerdict,
    prompt_version: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO triage_verdicts
            (account_id, folder_name, uid, needs_response, needs_action, category,
             urgency, model, prompt_version, input_tokens, output_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            account_id,
            folder_name,
            uid,
            v.needs_response,
            v.needs_action,
            v.category,
            v.urgency as i64,
            v.model,
            prompt_version,
            input_tokens,
            output_tokens
        ],
    )?;
    Ok(())
}

pub fn get(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<TriageVerdict>, AppError> {
    let v = conn
        .query_row(
            "SELECT needs_response, needs_action, category, urgency, model
               FROM triage_verdicts
              WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3
                AND withheld_reason IS NULL",
            params![account_id, folder_name, uid],
            |r| {
                Ok(TriageVerdict {
                    needs_response: r.get::<_, f64>(0)? as f32,
                    needs_action: r.get::<_, f64>(1)? as f32,
                    category: r.get(2)?,
                    urgency: r.get::<_, i64>(3)?.clamp(0, 4) as u8,
                    model: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(v)
}

/// Verdicts written since `datetime('now', ...)` expression `since`.
///
/// The daily spend cap reads this rather than keeping a counter: the table IS
/// the record of what was spent, so the cap cannot drift out of sync with
/// reality or reset itself by being forgotten.
pub fn count_since(conn: &Connection, since_sql: &str) -> Result<i64, AppError> {
    Ok(conn.query_row(
        &format!("SELECT COUNT(*) FROM triage_verdicts WHERE created_at >= {since_sql}"),
        [],
        |r| r.get(0),
    )?)
}

/// Tokens billed across every stored verdict, so spend is READ rather than
/// estimated from prompt lengths (which is what the first run had to do).
pub fn token_totals(conn: &Connection) -> Result<(i64, i64), AppError> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0) FROM triage_verdicts",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Verdicts stored so far, for the settings panel and for shadow-mode review.
/// Withheld rows are not verdicts and are not counted.
pub fn count(conn: &Connection) -> Result<i64, AppError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM triage_verdicts WHERE withheld_reason IS NULL",
        [],
        |r| r.get(0),
    )?)
}

/// How many messages the gate refused, by reason — the number that says
/// whether the blocklist is doing something or nothing.
pub fn withheld_count(conn: &Connection) -> Result<i64, AppError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM triage_verdicts WHERE withheld_reason IS NOT NULL",
        [],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, triage_enabled)
             VALUES ('on', 'a@x.io', 'gmail', 'i', 's', 1),
                    ('off', 'b@x.io', 'gmail', 'i', 's', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id, name, folder_type) VALUES ('on','INBOX','inbox'), ('off','INBOX','inbox')",
            [],
        )
        .unwrap();
        conn
    }

    fn msg(conn: &Connection, acct: &str, uid: u32, body: Option<&str>) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, from_email, subject, date)
             VALUES (?1, 'INBOX', ?2, 'p@x.io', 's', '2026-09-18 10:00:00')",
            params![acct, uid],
        )
        .unwrap();
        if let Some(b) = body {
            conn.execute(
                "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text)
                 VALUES (?1, 'INBOX', ?2, ?3)",
                params![acct, uid, b],
            )
            .unwrap();
        }
    }

    fn verdict() -> TriageVerdict {
        TriageVerdict {
            needs_response: 0.9,
            needs_action: 0.1,
            category: "primary".into(),
            urgency: 2,
            model: "test".into(),
        }
    }

    #[test]
    fn only_enabled_accounts_with_a_cached_body_are_pending() {
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        msg(&conn, "on", 2, None); // no body — the gate would withhold, so don't even ask
        msg(&conn, "off", 3, Some("hello")); // account not opted in
        let pending = list_pending(&conn, "v1", 100).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].uid, 1);
    }

    #[test]
    fn a_stored_verdict_removes_the_message_from_pending_but_a_new_prompt_brings_it_back() {
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        upsert(&conn, "on", "INBOX", 1, &verdict(), "v1", 900, 40).unwrap();
        assert!(list_pending(&conn, "v1", 100).unwrap().is_empty());
        // A prompt change invalidates the verdict: the message is asked again.
        assert_eq!(list_pending(&conn, "v2", 100).unwrap().len(), 1);
        // ...and the re-run REPLACES rather than accumulating.
        upsert(&conn, "on", "INBOX", 1, &verdict(), "v2", 900, 40).unwrap();
        assert_eq!(count(&conn).unwrap(), 1);
    }

    #[test]
    fn token_totals_come_from_the_table_not_an_estimate() {
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        msg(&conn, "on", 2, Some("hello"));
        upsert(&conn, "on", "INBOX", 1, &verdict(), "v1", 1000, 40).unwrap();
        upsert(&conn, "on", "INBOX", 2, &verdict(), "v1", 1200, 60).unwrap();
        assert_eq!(token_totals(&conn).unwrap(), (2200, 100));
    }

    #[test]
    fn a_withheld_message_leaves_the_queue_without_becoming_a_verdict() {
        // Both halves matter. Staying pending livelocked the pass; becoming a
        // verdict would make "we never looked" read as "no reply needed".
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        assert_eq!(list_pending(&conn, "v1", 10).unwrap().len(), 1);

        mark_withheld(&conn, "on", "INBOX", 1, "financial sender (chase.com)", "v1").unwrap();

        assert!(list_pending(&conn, "v1", 10).unwrap().is_empty(), "still re-picked");
        assert_eq!(get(&conn, "on", "INBOX", 1).unwrap(), None, "surfaced as a verdict");
        assert_eq!(count(&conn).unwrap(), 0, "counted as a verdict");
        assert_eq!(withheld_count(&conn).unwrap(), 1);
    }

    #[test]
    fn a_withheld_message_is_reconsidered_under_a_new_prompt() {
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        mark_withheld(&conn, "on", "INBOX", 1, "no cached body", "v1").unwrap();
        assert_eq!(list_pending(&conn, "v2", 10).unwrap().len(), 1);
    }

    #[test]
    fn verdict_round_trips_and_dies_with_its_message() {
        let conn = db();
        msg(&conn, "on", 1, Some("hello"));
        upsert(&conn, "on", "INBOX", 1, &verdict(), "v1", 900, 40).unwrap();
        assert_eq!(get(&conn, "on", "INBOX", 1).unwrap(), Some(verdict()));
        conn.execute("DELETE FROM messages WHERE uid = 1", []).unwrap();
        // ON DELETE CASCADE: a verdict on a message that no longer exists
        // means nothing (contrast zoom_meetings, gotcha #44).
        assert_eq!(count(&conn).unwrap(), 0);
    }

    #[test]
    fn newest_first_so_a_spend_cap_leaves_old_mail_waiting_not_new() {
        let conn = db();
        for (uid, date) in [(1, "2026-09-01"), (2, "2026-09-18"), (3, "2026-09-10")] {
            conn.execute(
                "INSERT INTO messages (account_id, folder_name, uid, from_email, subject, date)
                 VALUES ('on','INBOX',?1,'p@x.io','s',?2)",
                params![uid, date],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text) VALUES ('on','INBOX',?1,'b')",
                params![uid],
            )
            .unwrap();
        }
        let first = list_pending(&conn, "v1", 1).unwrap();
        assert_eq!(first[0].uid, 2);
    }
}
