//! Stored per-message risk reports (v66), written on an agent's first contact
//! with a message through the MCP and read back on every later call, so the
//! check runs once per email.
//!
//! **Keyed by account + Message-ID, not by the folder/uid triple.** Moving a
//! message changes its UID, and the point of storing is that a moved message
//! is not re-assessed (or re-sent to a model in the second half of the check).
//! The key is the bare, lowercased Message-ID from
//! `message_id_match_variants` (gotcha #30: the column holds three spellings);
//! a message without one falls back to `uid:<folder>:<uid>`. Folder and uid
//! are kept as the last place it was seen, for the approval hook's lookup.
//!
//! **No foreign key to `messages`.** A report must survive the message
//! moving folders, which deletes and re-inserts its `messages` row. It
//! cascades with the account only.
//!
//! The deterministic half (`signals_json`, `rules_version`) is rewritten when
//! `email::risk::RULES_VERSION` moves. The model half (Phase C columns) is
//! written separately and only ever by its own pass. Sender standing is never
//! stored: it changes as mail flows, so callers compute it live.

use cxmail_core::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};

pub fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_risk (
             account_id        TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             message_key       TEXT    NOT NULL,
             folder_name       TEXT,
             uid               INTEGER,
             rules_version     INTEGER NOT NULL,
             signals_json      TEXT    NOT NULL,
             model             TEXT,
             model_version     TEXT,
             p_phishing        REAL,
             p_ai_instructions REAL,
             p_impersonation   REAL,
             model_withheld    TEXT,
             model_at          TEXT,
             created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             updated_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             PRIMARY KEY (account_id, message_key)
         );
         CREATE INDEX IF NOT EXISTS idx_message_risk_location
             ON message_risk(account_id, folder_name, uid);",
    )
}

/// The storage key for a message. `message_id` is whatever spelling the
/// caller holds.
pub fn message_key(message_id: Option<&str>, folder_name: &str, uid: u32) -> String {
    message_id
        .and_then(cxmail_core::mail::message_id::message_id_match_variants)
        .map(|(_, bare)| bare.to_lowercase())
        .unwrap_or_else(|| format!("uid:{folder_name}:{uid}"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredRisk {
    pub rules_version: u32,
    pub signals_json: String,
    pub model: Option<String>,
    pub model_version: Option<String>,
    pub p_phishing: Option<f64>,
    pub p_ai_instructions: Option<f64>,
    pub p_impersonation: Option<f64>,
    pub model_withheld: Option<String>,
}

pub fn get(conn: &Connection, account_id: &str, key: &str) -> Result<Option<StoredRisk>, AppError> {
    Ok(conn
        .query_row(
            "SELECT rules_version, signals_json, model, model_version, p_phishing,
                    p_ai_instructions, p_impersonation, model_withheld
               FROM message_risk WHERE account_id = ?1 AND message_key = ?2",
            params![account_id, key],
            |r| {
                Ok(StoredRisk {
                    rules_version: r.get::<_, i64>(0)? as u32,
                    signals_json: r.get(1)?,
                    model: r.get(2)?,
                    model_version: r.get(3)?,
                    p_phishing: r.get(4)?,
                    p_ai_instructions: r.get(5)?,
                    p_impersonation: r.get(6)?,
                    model_withheld: r.get(7)?,
                })
            },
        )
        .optional()?)
}

/// Write the deterministic half and the last-seen location. Leaves the model
/// columns untouched on an existing row.
pub fn upsert_signals(
    conn: &Connection,
    account_id: &str,
    key: &str,
    folder_name: &str,
    uid: u32,
    rules_version: u32,
    signals_json: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO message_risk (account_id, message_key, folder_name, uid, rules_version, signals_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id, message_key) DO UPDATE SET
             folder_name = excluded.folder_name,
             uid = excluded.uid,
             rules_version = excluded.rules_version,
             signals_json = excluded.signals_json,
             updated_at = datetime('now')",
        params![account_id, key, folder_name, uid, rules_version as i64, signals_json],
    )?;
    Ok(())
}

/// Record where the message is now, without touching the report.
pub fn touch_location(conn: &Connection, account_id: &str, key: &str, folder_name: &str, uid: u32) -> Result<(), AppError> {
    conn.execute(
        "UPDATE message_risk SET folder_name = ?3, uid = ?4
          WHERE account_id = ?1 AND message_key = ?2 AND (folder_name IS NOT ?3 OR uid IS NOT ?4)",
        params![account_id, key, folder_name, uid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host) VALUES ('a', 'a@x.example', 'gmail', 'i', 's')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn key_is_the_same_for_every_message_id_spelling() {
        let k1 = message_key(Some("<ABC@mail.example>"), "INBOX", 1);
        let k2 = message_key(Some("abc@mail.example"), "Archive", 9);
        assert_eq!(k1, "abc@mail.example");
        assert_eq!(k1, k2);
        assert_eq!(message_key(None, "INBOX", 7), "uid:INBOX:7");
    }

    #[test]
    fn a_moved_message_keeps_its_report_and_model_half() {
        let conn = db();
        let k = message_key(Some("<m1@x.example>"), "INBOX", 5);
        upsert_signals(&conn, "a", &k, "INBOX", 5, 1, "[]").unwrap();
        conn.execute(
            "UPDATE message_risk SET model = 'decisions:gpt-6-luna', p_phishing = 0.9 WHERE message_key = ?1",
            [&k],
        )
        .unwrap();
        touch_location(&conn, "a", &k, "Archive", 77).unwrap();
        upsert_signals(&conn, "a", &k, "Archive", 77, 2, r#"[{"code":"x"}]"#).unwrap();
        let r = get(&conn, "a", &k).unwrap().unwrap();
        assert_eq!(r.rules_version, 2);
        assert_eq!(r.p_phishing, Some(0.9), "re-assessing the rules must not drop the model verdict");
        let (f, u): (String, i64) = conn
            .query_row("SELECT folder_name, uid FROM message_risk WHERE message_key = ?1", [&k], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((f.as_str(), u), ("Archive", 77));
    }

    #[test]
    fn reports_die_with_the_account_only() {
        let conn = db();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        upsert_signals(&conn, "a", "k", "INBOX", 1, 1, "[]").unwrap();
        conn.execute("DELETE FROM accounts WHERE id = 'a'", []).unwrap();
        assert!(get(&conn, "a", "k").unwrap().is_none());
    }
}
