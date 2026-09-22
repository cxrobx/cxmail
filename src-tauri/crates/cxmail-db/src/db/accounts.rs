use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: i32,
    pub smtp_host: String,
    pub smtp_port: i32,
    /// `"implicit"` or `"starttls"`. Stored rather than inferred from the port:
    /// the same port number is STARTTLS on one server and implicit TLS on
    /// another, so a port heuristic would silently pick the wrong handshake.
    pub imap_security: String,
    pub smtp_security: String,
    /// Login name, when it differs from `email`. `None` = authenticate as
    /// `email`, which is what every provider CXMail shipped with does.
    pub imap_username: Option<String>,
    pub smtp_username: Option<String>,
    pub color: Option<String>,
    pub is_active: bool,
    pub sort_order: i32,
    pub group_name: Option<String>,
    pub notify_enabled: bool,
    pub track_opens_enabled: bool,
    /// Keep this account out of every cross-account surface — All Inboxes,
    /// the account folders, rule-based inbox groups, Needs You, nudges,
    /// unscoped search, the category counts and the dock badge. Its mail is
    /// still synced and is shown when the account itself is clicked. See
    /// [`visible_in_aggregates_sql`] for how the queries enforce it.
    pub hidden_from_aggregates: bool,
    /// Let the background AI triage pass read this account's inbox. Default
    /// OFF; a per-account opt-in because the value of triage is concentrated
    /// where the sensitivity is (client correspondence), and that trade is the
    /// user's to make one account at a time.
    pub triage_enabled: bool,
}

/// Insert a new account, or — if `account.email` already belongs to an
/// existing row — update that row's connection details in place and return
/// its (unchanged) id. Re-authenticating an already-connected email must
/// never mint a second row: `messages.account_id`, `folders`, `sync_state`
/// etc. all reference `accounts.id`, so preserving it is what keeps existing
/// mail attached instead of orphaning it. Only connection fields are updated
/// on conflict — user settings (display_name, color, sort_order, group_name,
/// notify_enabled, track_opens_enabled, hidden_from_aggregates) are
/// deliberately left untouched.
pub fn upsert(conn: &Connection, account: &Account) -> Result<String, AppError> {
    let max_order: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM accounts",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);

    conn.execute(
        "INSERT INTO accounts (id, email, display_name, provider, imap_host, imap_port, smtp_host, smtp_port, imap_security, smtp_security, imap_username, smtp_username, color, is_active, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(email) DO UPDATE SET
             provider = excluded.provider,
             imap_host = excluded.imap_host,
             imap_port = excluded.imap_port,
             smtp_host = excluded.smtp_host,
             smtp_port = excluded.smtp_port,
             imap_security = excluded.imap_security,
             smtp_security = excluded.smtp_security,
             imap_username = excluded.imap_username,
             smtp_username = excluded.smtp_username,
             is_active = excluded.is_active,
             updated_at = datetime('now')",
        params![
            account.id,
            account.email,
            account.display_name,
            account.provider,
            account.imap_host,
            account.imap_port,
            account.smtp_host,
            account.smtp_port,
            account.imap_security,
            account.smtp_security,
            account.imap_username,
            account.smtp_username,
            account.color,
            account.is_active as i32,
            max_order + 1,
        ],
    )?;

    conn.query_row(
        "SELECT id FROM accounts WHERE email = ?1",
        params![account.email],
        |row| row.get(0),
    )
    .map_err(AppError::Database)
}

pub fn list(conn: &Connection) -> Result<Vec<Account>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, email, display_name, provider, imap_host, imap_port, smtp_host, smtp_port, imap_security, smtp_security, imap_username, smtp_username, color, is_active, sort_order, group_name, notify_enabled, track_opens_enabled, hidden_from_aggregates, triage_enabled FROM accounts ORDER BY sort_order, created_at",
    )?;
    let accounts = stmt
        .query_map([], |row| {
            Ok(Account {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                provider: row.get(3)?,
                imap_host: row.get(4)?,
                imap_port: row.get(5)?,
                smtp_host: row.get(6)?,
                smtp_port: row.get(7)?,
                imap_security: row.get(8)?,
                smtp_security: row.get(9)?,
                imap_username: row.get(10)?,
                smtp_username: row.get(11)?,
                color: row.get(12)?,
                is_active: row.get::<_, i32>(13)? != 0,
                sort_order: row.get(14)?,
                group_name: row.get(15)?,
                notify_enabled: row.get::<_, i32>(16)? != 0,
                track_opens_enabled: row.get::<_, i32>(17)? != 0,
                hidden_from_aggregates: row.get::<_, i32>(18)? != 0,
                triage_enabled: row.get::<_, i32>(19)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(accounts)
}

pub fn get_by_id(conn: &Connection, id: &str) -> Result<Option<Account>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, email, display_name, provider, imap_host, imap_port, smtp_host, smtp_port, imap_security, smtp_security, imap_username, smtp_username, color, is_active, sort_order, group_name, notify_enabled, track_opens_enabled, hidden_from_aggregates, triage_enabled FROM accounts WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(Account {
            id: row.get(0)?,
            email: row.get(1)?,
            display_name: row.get(2)?,
            provider: row.get(3)?,
            imap_host: row.get(4)?,
            imap_port: row.get(5)?,
            smtp_host: row.get(6)?,
            smtp_port: row.get(7)?,
            imap_security: row.get(8)?,
            smtp_security: row.get(9)?,
            imap_username: row.get(10)?,
            smtp_username: row.get(11)?,
            color: row.get(12)?,
            is_active: row.get::<_, i32>(13)? != 0,
            sort_order: row.get(14)?,
            group_name: row.get(15)?,
            notify_enabled: row.get::<_, i32>(16)? != 0,
            track_opens_enabled: row.get::<_, i32>(17)? != 0,
            hidden_from_aggregates: row.get::<_, i32>(18)? != 0,
            triage_enabled: row.get::<_, i32>(19)? != 0,
        })
    })?;
    match rows.next() {
        Some(Ok(account)) => Ok(Some(account)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

pub fn update_order(conn: &Connection, account_ids: &[String]) -> Result<(), AppError> {
    let mut stmt = conn.prepare("UPDATE accounts SET sort_order = ?1 WHERE id = ?2")?;
    for (i, id) in account_ids.iter().enumerate() {
        stmt.execute(params![i as i32, id])?;
    }
    Ok(())
}

pub fn set_group(conn: &Connection, id: &str, group_name: Option<&str>) -> Result<(), AppError> {
    conn.execute(
        "UPDATE accounts SET group_name = ?1 WHERE id = ?2",
        params![group_name, id],
    )?;
    Ok(())
}

pub fn set_notify_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<(), AppError> {
    conn.execute(
        "UPDATE accounts SET notify_enabled = ?1 WHERE id = ?2",
        params![enabled as i32, id],
    )?;
    Ok(())
}

pub fn set_track_opens_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<(), AppError> {
    conn.execute(
        "UPDATE accounts SET track_opens_enabled = ?1 WHERE id = ?2",
        params![enabled as i32, id],
    )?;
    Ok(())
}

pub fn set_triage_enabled(conn: &Connection, account_id: &str, enabled: bool) -> Result<(), AppError> {
    conn.execute(
        "UPDATE accounts SET triage_enabled = ?1 WHERE id = ?2",
        rusqlite::params![enabled as i32, account_id],
    )?;
    Ok(())
}

pub fn set_hidden_from_aggregates(
    conn: &Connection,
    id: &str,
    hidden: bool,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE accounts SET hidden_from_aggregates = ?1 WHERE id = ?2",
        params![hidden as i32, id],
    )?;
    Ok(())
}

/// Every account that is hidden from aggregated views, in sidebar order. What
/// the MCP names when an unscoped search skipped something, and what
/// `server_search` uses to decide which servers to fan out to.
pub fn list_hidden(conn: &Connection) -> Result<Vec<Account>, AppError> {
    Ok(list(conn)?
        .into_iter()
        .filter(|a| a.hidden_from_aggregates)
        .collect())
}

/// The one predicate behind "hidden from aggregates" — a SQL fragment that is
/// true when the account behind `account_id_expr` is NOT hidden. Every
/// cross-account query embeds this (gotcha #36: one matcher), with whatever
/// expression names the message's account in that query (`m.account_id`,
/// `messages.account_id`, …).
///
/// **`NOT EXISTS`, so absence fails open.** A message whose account row is
/// missing stays visible: hidden is the explicit state, and a missing row is
/// not evidence of anything. It also means a fixture with no matching
/// `accounts` row behaves exactly as it did before the column existed.
///
/// The correlated reference is qualified on the *caller's* side, because
/// inside the subquery a bare `account_id` resolves against `ha` first. The
/// alias is `ha` rather than `a` because the list CTEs already use
/// `aggregates a`.
///
/// Two call patterns, deliberately different:
/// - the list/count queries apply it **always** — an account folder is still
///   an aggregate, so `list_all_inboxes(Some(ids))` hides a hidden member too;
/// - `search::search` applies it **only when `account_ids` is `None`**,
///   because search serves explicit contexts as well (the one way a hidden
///   account's mail is reachable by search is to name it).
pub fn visible_in_aggregates_sql(account_id_expr: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM accounts ha WHERE ha.id = {account_id_expr} AND ha.hidden_from_aggregates = 1)"
    )
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "
            CREATE TABLE accounts (
                id          TEXT PRIMARY KEY,
                email       TEXT NOT NULL UNIQUE,
                display_name TEXT,
                provider    TEXT NOT NULL,
                imap_host   TEXT NOT NULL,
                imap_port   INTEGER NOT NULL DEFAULT 993,
                smtp_host   TEXT NOT NULL,
                smtp_port   INTEGER NOT NULL DEFAULT 587,
                imap_security TEXT NOT NULL DEFAULT 'implicit',
                smtp_security TEXT NOT NULL DEFAULT 'starttls',
                imap_username TEXT,
                smtp_username TEXT,
                color       TEXT,
                is_active   INTEGER NOT NULL DEFAULT 1,
                sort_order  INTEGER NOT NULL DEFAULT 0,
                group_name  TEXT,
                notify_enabled INTEGER NOT NULL DEFAULT 1,
                track_opens_enabled INTEGER NOT NULL DEFAULT 0,
                hidden_from_aggregates INTEGER NOT NULL DEFAULT 0,
                triage_enabled INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .expect("schema");
        conn
    }

    fn sample_account(id: &str, email: &str) -> Account {
        Account {
            id: id.to_string(),
            email: email.to_string(),
            display_name: None,
            provider: "gmail".to_string(),
            imap_host: "imap.gmail.com".to_string(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            imap_security: "implicit".to_string(),
            smtp_security: "starttls".to_string(),
            imap_username: None,
            smtp_username: None,
            color: Some("#0a84ff".to_string()),
            is_active: true,
            sort_order: 0,
            group_name: None,
            notify_enabled: true,
            track_opens_enabled: false,
            hidden_from_aggregates: false,
            triage_enabled: false,
        }
    }

    #[test]
    fn upsert_inserts_new_account_and_returns_its_id() {
        let conn = setup_conn();
        let account = sample_account("acc-1", "new@example.com");

        let effective_id = upsert(&conn, &account).unwrap();

        assert_eq!(effective_id, "acc-1");
        let fetched = get_by_id(&conn, "acc-1").unwrap().unwrap();
        assert_eq!(fetched.email, "new@example.com");
    }

    #[test]
    fn upsert_on_email_conflict_preserves_id_and_user_settings() {
        let conn = setup_conn();
        let original = sample_account("acc-original", "chris@example.com");
        upsert(&conn, &original).unwrap();

        // Simulate the user customizing the account after connecting it —
        // exactly the state a re-auth must not clobber.
        conn.execute(
            "UPDATE accounts SET display_name = 'Work', color = '#ff0000', sort_order = 7,
             group_name = 'Northwind', notify_enabled = 0, track_opens_enabled = 1,
             hidden_from_aggregates = 1
             WHERE id = 'acc-original'",
            [],
        )
        .unwrap();

        // Re-authenticating mints a FRESH uuid and (plausibly) different
        // connection details before the caller ever knows a row exists.
        let mut reauth = sample_account("acc-fresh-uuid", "chris@example.com");
        reauth.imap_host = "imap.gmail.com".to_string();
        reauth.is_active = true;
        // Security mode and login name are CONNECTION fields, not user
        // settings — re-auth is exactly the gesture a user makes to fix wrong
        // server settings, so these must overwrite. If they were preserved
        // like display_name, a mistyped port/security could never be corrected
        // without deleting the account.
        reauth.imap_security = "starttls".to_string();
        reauth.smtp_security = "implicit".to_string();
        reauth.imap_username = Some("chris.login".to_string());
        reauth.smtp_username = Some("chris.smtp".to_string());
        reauth.color = Some("#0a84ff".to_string()); // caller's default; must be ignored
        reauth.display_name = Some("should not win".to_string()); // must be ignored

        let effective_id = upsert(&conn, &reauth).unwrap();

        // The original row's id wins — no second row, no orphaned mail.
        assert_eq!(effective_id, "acc-original");
        let accounts = list(&conn).unwrap();
        assert_eq!(accounts.len(), 1, "conflict must update in place, not insert a second row");

        let fetched = get_by_id(&conn, "acc-original").unwrap().unwrap();
        // Connection fields follow the fresh auth...
        assert_eq!(fetched.provider, "gmail");
        assert_eq!(fetched.imap_host, "imap.gmail.com");
        assert!(fetched.is_active);
        assert_eq!(fetched.imap_security, "starttls");
        assert_eq!(fetched.smtp_security, "implicit");
        assert_eq!(fetched.imap_username, Some("chris.login".to_string()));
        assert_eq!(fetched.smtp_username, Some("chris.smtp".to_string()));
        // ...but user settings from before the reconnect are untouched.
        assert_eq!(fetched.display_name, Some("Work".to_string()));
        assert_eq!(fetched.color, Some("#ff0000".to_string()));
        assert_eq!(fetched.sort_order, 7);
        assert_eq!(fetched.group_name, Some("Northwind".to_string()));
        assert!(!fetched.notify_enabled);
        assert!(fetched.track_opens_enabled);
        assert!(
            fetched.hidden_from_aggregates,
            "re-auth must not un-hide an account the user hid"
        );
    }

    #[test]
    fn hidden_from_aggregates_round_trips_and_lists() {
        let conn = setup_conn();
        upsert(&conn, &sample_account("acc-a", "a@example.com")).unwrap();
        upsert(&conn, &sample_account("acc-h", "h@example.com")).unwrap();
        assert!(list_hidden(&conn).unwrap().is_empty(), "default is visible");

        set_hidden_from_aggregates(&conn, "acc-h", true).unwrap();
        assert!(get_by_id(&conn, "acc-h").unwrap().unwrap().hidden_from_aggregates);
        assert!(!get_by_id(&conn, "acc-a").unwrap().unwrap().hidden_from_aggregates);
        let hidden: Vec<String> = list_hidden(&conn).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(hidden, vec!["acc-h"]);

        set_hidden_from_aggregates(&conn, "acc-h", false).unwrap();
        assert!(list_hidden(&conn).unwrap().is_empty());
    }

    /// The predicate is a string other modules splice into their SQL, so its
    /// shape is a contract: qualified correlation, the `ha` alias, and
    /// `NOT EXISTS` (fail open) — see the doc comment for why each matters.
    #[test]
    fn visible_in_aggregates_sql_is_a_qualified_not_exists() {
        assert_eq!(
            visible_in_aggregates_sql("m.account_id"),
            "NOT EXISTS (SELECT 1 FROM accounts ha WHERE ha.id = m.account_id AND ha.hidden_from_aggregates = 1)"
        );

        // Fail-open, proven rather than asserted: a message whose account row
        // is missing is visible, a hidden one is not, a visible one is.
        let conn = setup_conn();
        upsert(&conn, &sample_account("acc-a", "a@example.com")).unwrap();
        upsert(&conn, &sample_account("acc-h", "h@example.com")).unwrap();
        set_hidden_from_aggregates(&conn, "acc-h", true).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (account_id TEXT NOT NULL);
             INSERT INTO messages VALUES ('acc-a'), ('acc-h'), ('acc-gone');",
        )
        .unwrap();
        let sql = format!(
            "SELECT account_id FROM messages m WHERE {} ORDER BY account_id",
            visible_in_aggregates_sql("m.account_id")
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let visible: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(visible, vec!["acc-a", "acc-gone"]);
    }

    /// One matcher (gotcha #36). rusqlite is built without `trace`, so the
    /// only way to prove no aggregate query hand-rolls its own copy of the
    /// rule is to read the source: the column name must appear in the
    /// production half of none of the modules that embed the predicate — they
    /// reach it through `visible_in_aggregates_sql(` alone. A second spelling
    /// would drift the moment one of them changed. (Their test modules may
    /// name the column freely — fixture DDL and the setter both do.)
    #[test]
    fn aggregate_queries_reach_the_rule_only_through_the_one_predicate() {
        let modules: [(&str, &str); 6] = [
            ("messages", include_str!("messages.rs")),
            ("categories", include_str!("categories.rs")),
            ("inbox_groups", include_str!("inbox_groups.rs")),
            ("needs_you", include_str!("needs_you.rs")),
            ("nudges", include_str!("nudges.rs")),
            ("search", include_str!("search.rs")),
        ];
        for (name, src) in modules {
            let production = src
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first piece");
            assert!(
                !production.contains("hidden_from_aggregates"),
                "db/{name}.rs spells the column itself — use visible_in_aggregates_sql()"
            );
            assert!(
                production.contains("visible_in_aggregates_sql("),
                "db/{name}.rs no longer applies the hidden-account rule"
            );
        }
    }
}
