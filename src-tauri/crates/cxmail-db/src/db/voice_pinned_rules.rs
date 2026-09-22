//! Pinned, human-authored writing rules — absolute directives that survive
//! voice-profile rebuilds.
//!
//! The distinction this module exists to keep is:
//!
//! | | `voice_profiles_recipient.profile_json` | `voice_pinned_rules` |
//! |---|---|---|
//! | author | the LLM | the user |
//! | nature | statistical description ("often opens 'Hey Bro. Ellis,'") | absolute rule ("ALWAYS address as 'Bro. Ellis'") |
//! | lifetime | overwritten by `extract_recipient_profile(force=true)` | permanent until deleted |
//!
//! A soft description leaves a drafting agent free to write "Hi Sam," and be
//! faithful to the corpus. A pinned rule does not. See the v51 migration in
//! `db::schema` for why they cannot share storage.

use crate::error::AppError;
use rusqlite::{params, Connection};

/// Longest single rule. A pinned rule is injected into every draft's prompt
/// context, so an essay pasted in here quietly crowds out the rest of the
/// voice context on every message.
pub const MAX_RULE_CHARS: usize = 500;

/// Most rules one scope may hold. Same reasoning as `MAX_RULE_CHARS`, plus:
/// past a couple of dozen absolutes nothing is absolute any more.
pub const MAX_RULES_PER_SCOPE: usize = 20;

/// Which messages a rule governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every message sent from this account.
    Account,
    /// Only mail addressed to one recipient.
    Recipient,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Account => "account",
            Scope::Recipient => "recipient",
        }
    }

    fn from_str(s: &str) -> Scope {
        match s {
            "account" => Scope::Account,
            _ => Scope::Recipient,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PinnedRule {
    pub id: i64,
    pub account_id: String,
    pub scope: Scope,
    /// Empty string for account-scoped rules.
    pub recipient_email: String,
    pub rule: String,
    pub created_at: String,
}

/// Normalize an address the same way `voice_profiles_recipient` does, so a rule
/// pinned for `Sam@Harborline.example` is found when drafting to `sam@harborline.example`.
pub fn normalize_recipient(recipient_email: &str) -> String {
    recipient_email.trim().to_lowercase()
}

fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<PinnedRule> {
    let scope: String = row.get(2)?;
    Ok(PinnedRule {
        id: row.get(0)?,
        account_id: row.get(1)?,
        scope: Scope::from_str(&scope),
        recipient_email: row.get(3)?,
        rule: row.get(4)?,
        created_at: row.get(5)?,
    })
}

const SELECT_COLUMNS: &str = "id, account_id, scope, recipient_email, rule, created_at";

/// Insert a rule. Returns `(id, created)` — re-pinning identical text for the
/// same scope is a no-op that returns the existing row's id rather than an
/// error, since the caller's intent ("this rule should exist") is already true.
pub fn add(
    conn: &Connection,
    account_id: &str,
    scope: Scope,
    recipient_email: &str,
    rule: &str,
) -> Result<(i64, bool), AppError> {
    let recipient = match scope {
        Scope::Account => String::new(),
        Scope::Recipient => normalize_recipient(recipient_email),
    };
    let rule = rule.trim();

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM voice_pinned_rules
             WHERE account_id = ?1 AND scope = ?2 AND recipient_email = ?3 AND rule = ?4",
            params![account_id, scope.as_str(), recipient, rule],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok((id, false));
    }

    conn.execute(
        "INSERT INTO voice_pinned_rules (account_id, scope, recipient_email, rule)
         VALUES (?1, ?2, ?3, ?4)",
        params![account_id, scope.as_str(), recipient, rule],
    )?;
    Ok((conn.last_insert_rowid(), true))
}

/// Count the rules already pinned for one scope — the ceiling check callers run
/// before `add`.
pub fn count_for_scope(
    conn: &Connection,
    account_id: &str,
    scope: Scope,
    recipient_email: &str,
) -> Result<usize, AppError> {
    let recipient = match scope {
        Scope::Account => String::new(),
        Scope::Recipient => normalize_recipient(recipient_email),
    };
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM voice_pinned_rules
         WHERE account_id = ?1 AND scope = ?2 AND recipient_email = ?3",
        params![account_id, scope.as_str(), recipient],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}

/// Every rule that applies when writing to `recipient_email` from this account:
/// the account-wide rules first, then the ones pinned for that address.
///
/// **Order is the precedence contract** — account rules are the standing policy,
/// recipient rules are the specific override, and both the prompt block and the
/// MCP payload present them in this order so "more specific wins" is legible to
/// whoever reads them. Do not sort this by id or date.
pub fn list_effective(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<Vec<PinnedRule>, AppError> {
    let recipient = normalize_recipient(recipient_email);
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM voice_pinned_rules
         WHERE account_id = ?1
           AND (scope = 'account' OR (scope = 'recipient' AND recipient_email = ?2))
         ORDER BY (scope = 'recipient') ASC, id ASC"
    ))?;
    let rows = stmt
        .query_map(params![account_id, recipient], |row| row_to_rule(row))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Account-scoped rules only. Used when there is no recipient in play (drafting
/// context for an archetype or the account-level profile).
pub fn list_account_scoped(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<PinnedRule>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM voice_pinned_rules
         WHERE account_id = ?1 AND scope = 'account'
         ORDER BY id ASC"
    ))?;
    let rows = stmt
        .query_map(params![account_id], |row| row_to_rule(row))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Every rule on the account, both scopes — the browse/audit view.
pub fn list_all(conn: &Connection, account_id: &str) -> Result<Vec<PinnedRule>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM voice_pinned_rules
         WHERE account_id = ?1
         ORDER BY (scope = 'recipient') ASC, recipient_email ASC, id ASC"
    ))?;
    let rows = stmt
        .query_map(params![account_id], |row| row_to_rule(row))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<PinnedRule>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM voice_pinned_rules WHERE id = ?1"
    ))?;
    let mut rows = stmt.query_map(params![id], |row| row_to_rule(row))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

/// Delete by id. Returns whether a row was actually removed.
pub fn delete(conn: &Connection, id: i64) -> Result<bool, AppError> {
    let n = conn.execute("DELETE FROM voice_pinned_rules WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
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

    fn texts(rules: &[PinnedRule]) -> Vec<&str> {
        rules.iter().map(|r| r.rule.as_str()).collect()
    }

    /// THE acceptance test for this feature: a forced profile rebuild must not
    /// touch a pinned rule. `extract_recipient_profile(force=true)` makes an LLM
    /// call and then performs exactly one write — the upsert below — so driving
    /// the real `voice_profiles_recipient::upsert` is what actually proves the
    /// storage split, where a mocked "rebuild" would prove nothing.
    #[test]
    fn a_forced_profile_rebuild_leaves_pinned_rules_intact() {
        let conn = db();
        crate::db::voice_profiles_recipient::upsert(
            &conn,
            "acct",
            "sam@harborline.example",
            r#"{"typical_greeting":"Hi [Name], or Hey [Name]"}"#,
            "claude-opus-5",
            8,
            "2026-07-25T10:00:00Z",
        )
        .unwrap();
        add(
            &conn,
            "acct",
            Scope::Recipient,
            "sam@harborline.example",
            "Always address as \"Bro. Ellis\", never \"Sam\".",
        )
        .unwrap();

        // Forced rebuild: same call the MCP tool makes, wholly new profile JSON.
        crate::db::voice_profiles_recipient::upsert(
            &conn,
            "acct",
            "sam@harborline.example",
            r#"{"typical_greeting":"Hi [Name]"}"#,
            "claude-opus-5",
            9,
            "2026-08-10T10:00:00Z",
        )
        .unwrap();

        let after = list_effective(&conn, "acct", "sam@harborline.example").unwrap();
        assert_eq!(
            texts(&after),
            vec!["Always address as \"Bro. Ellis\", never \"Sam\"."],
            "a forced rebuild must not disturb pinned rules"
        );
    }

    /// Evicting the cached profile entirely is the other destructive path, and
    /// it must not take the user's instructions with it — hence no FK to
    /// `voice_profiles_recipient`.
    #[test]
    fn deleting_the_cached_profile_leaves_pinned_rules_intact() {
        let conn = db();
        crate::db::voice_profiles_recipient::upsert(
            &conn,
            "acct",
            "sam@harborline.example",
            "{}",
            "m",
            3,
            "2026-07-25T10:00:00Z",
        )
        .unwrap();
        add(&conn, "acct", Scope::Recipient, "sam@harborline.example", "R").unwrap();

        crate::db::voice_profiles_recipient::delete(&conn, "acct", "sam@harborline.example").unwrap();

        assert_eq!(
            list_effective(&conn, "acct", "sam@harborline.example")
                .unwrap()
                .len(),
            1
        );
    }

    /// A rule can be pinned for someone with no derived profile at all — that is
    /// the common case for a new contact, and the read paths must not require
    /// one.
    #[test]
    fn a_rule_needs_no_cached_profile() {
        let conn = db();
        add(&conn, "acct", Scope::Recipient, "new@example.com", "R").unwrap();
        assert!(
            crate::db::voice_profiles_recipient::get(&conn, "acct", "new@example.com")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            list_effective(&conn, "acct", "new@example.com")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn account_rules_apply_to_every_recipient_and_come_first() {
        let conn = db();
        add(
            &conn,
            "acct",
            Scope::Recipient,
            "sam@harborline.example",
            "Address as Bro. Ellis",
        )
        .unwrap();
        add(
            &conn,
            "acct",
            Scope::Account,
            "",
            "Never open with \"Hope you're well\"",
        )
        .unwrap();

        // Account rule leads, recipient rule follows — the precedence contract.
        assert_eq!(
            texts(&list_effective(&conn, "acct", "sam@harborline.example").unwrap()),
            vec!["Never open with \"Hope you're well\"", "Address as Bro. Ellis"]
        );
        // A different recipient gets the account rule and nothing else.
        assert_eq!(
            texts(&list_effective(&conn, "acct", "someone@else.com").unwrap()),
            vec!["Never open with \"Hope you're well\""]
        );
        assert_eq!(list_account_scoped(&conn, "acct").unwrap().len(), 1);
    }

    #[test]
    fn recipient_matching_is_case_and_whitespace_insensitive() {
        let conn = db();
        add(
            &conn,
            "acct",
            Scope::Recipient,
            "  Sam@Harborline.example ",
            "Bro. Ellis",
        )
        .unwrap();
        assert_eq!(
            list_effective(&conn, "acct", "sam@harborline.example")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn re_pinning_the_same_rule_is_a_no_op_not_a_duplicate() {
        let conn = db();
        let (first, created) = add(&conn, "acct", Scope::Recipient, "e@x.com", "R").unwrap();
        assert!(created);
        let (second, created_again) = add(&conn, "acct", Scope::Recipient, "e@x.com", "R").unwrap();
        assert!(!created_again);
        assert_eq!(first, second);
        assert_eq!(list_effective(&conn, "acct", "e@x.com").unwrap().len(), 1);
    }

    /// The same text at both scopes is two distinct rules, not a duplicate —
    /// the UNIQUE index keys on scope as well.
    #[test]
    fn the_same_text_at_two_scopes_is_two_rules() {
        let conn = db();
        assert!(add(&conn, "acct", Scope::Recipient, "e@x.com", "R").unwrap().1);
        assert!(add(&conn, "acct", Scope::Account, "", "R").unwrap().1);
        assert_eq!(list_effective(&conn, "acct", "e@x.com").unwrap().len(), 2);
    }

    #[test]
    fn delete_removes_one_rule_and_reports_a_miss() {
        let conn = db();
        let (id, _) = add(&conn, "acct", Scope::Recipient, "e@x.com", "R").unwrap();
        add(&conn, "acct", Scope::Recipient, "e@x.com", "S").unwrap();

        assert!(delete(&conn, id).unwrap());
        assert_eq!(texts(&list_effective(&conn, "acct", "e@x.com").unwrap()), vec!["S"]);
        assert!(!delete(&conn, id).unwrap(), "second delete finds nothing");
    }

    #[test]
    fn removing_the_account_removes_its_rules() {
        let conn = db();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        add(&conn, "acct", Scope::Account, "", "R").unwrap();
        conn.execute("DELETE FROM accounts WHERE id = 'acct'", [])
            .unwrap();
        assert!(list_all(&conn, "acct").unwrap().is_empty());
    }

    #[test]
    fn count_for_scope_is_scoped() {
        let conn = db();
        add(&conn, "acct", Scope::Recipient, "e@x.com", "R").unwrap();
        add(&conn, "acct", Scope::Recipient, "e@x.com", "S").unwrap();
        add(&conn, "acct", Scope::Account, "", "T").unwrap();
        assert_eq!(count_for_scope(&conn, "acct", Scope::Recipient, "e@x.com").unwrap(), 2);
        assert_eq!(count_for_scope(&conn, "acct", Scope::Account, "").unwrap(), 1);
        assert_eq!(count_for_scope(&conn, "acct", Scope::Recipient, "other@x.com").unwrap(), 0);
    }
}
