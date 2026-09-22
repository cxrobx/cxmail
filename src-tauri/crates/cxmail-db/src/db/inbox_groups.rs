use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxGroup {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxGroupRule {
    pub id: i64,
    pub group_id: i64,
    pub field: String,
    pub operator: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InboxGroupWithDetails {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub sort_order: i32,
    pub rules: Vec<InboxGroupRule>,
    pub account_ids: Vec<String>,
    pub unread_count: u32,
}

pub fn list_groups(conn: &Connection) -> Result<Vec<InboxGroupWithDetails>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, color, icon, sort_order FROM inbox_groups ORDER BY sort_order, id",
    )?;
    let groups: Vec<InboxGroup> = stmt
        .query_map([], |row| {
            Ok(InboxGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                sort_order: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        let rules = list_rules(conn, group.id)?;
        let account_ids = list_account_ids(conn, group.id)?;
        let unread_count = count_unread_for_group(conn, &rules, &account_ids)?;
        result.push(InboxGroupWithDetails {
            id: group.id,
            name: group.name,
            color: group.color,
            icon: group.icon,
            sort_order: group.sort_order,
            rules,
            account_ids,
            unread_count,
        });
    }
    Ok(result)
}

pub fn list_rules(conn: &Connection, group_id: i64) -> Result<Vec<InboxGroupRule>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, field, operator, value FROM inbox_group_rules WHERE group_id = ?1",
    )?;
    let rules = stmt
        .query_map(params![group_id], |row| {
            Ok(InboxGroupRule {
                id: row.get(0)?,
                group_id: row.get(1)?,
                field: row.get(2)?,
                operator: row.get(3)?,
                value: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rules)
}

pub fn list_account_ids(conn: &Connection, group_id: i64) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT account_id FROM inbox_group_accounts WHERE group_id = ?1",
    )?;
    let ids = stmt
        .query_map(params![group_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

pub fn create_group(
    conn: &Connection,
    name: &str,
    color: &str,
    icon: &str,
) -> Result<i64, AppError> {
    let max_order: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM inbox_groups",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);

    conn.execute(
        "INSERT INTO inbox_groups (name, color, icon, sort_order) VALUES (?1, ?2, ?3, ?4)",
        params![name, color, icon, max_order + 1],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_group(
    conn: &Connection,
    id: i64,
    name: &str,
    color: &str,
    icon: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE inbox_groups SET name = ?1, color = ?2, icon = ?3 WHERE id = ?4",
        params![name, color, icon, id],
    )?;
    Ok(())
}

pub fn delete_group(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM inbox_groups WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn set_rules(
    conn: &Connection,
    group_id: i64,
    rules: &[(String, String, String)],
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM inbox_group_rules WHERE group_id = ?1",
        params![group_id],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO inbox_group_rules (group_id, field, operator, value) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (field, operator, value) in rules {
        stmt.execute(params![group_id, field, operator, value])?;
    }
    Ok(())
}

pub fn set_account_ids(
    conn: &Connection,
    group_id: i64,
    account_ids: &[String],
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM inbox_group_accounts WHERE group_id = ?1",
        params![group_id],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO inbox_group_accounts (group_id, account_id) VALUES (?1, ?2)",
    )?;
    for account_id in account_ids {
        stmt.execute(params![group_id, account_id])?;
    }
    Ok(())
}

/// The account half of [`build_group_filter`]: any message in these accounts.
fn build_account_conditions(account_ids: &[String]) -> (Vec<String>, Vec<String>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();
    for account_id in account_ids {
        conditions.push("account_id = ?".to_string());
        values.push(account_id.clone());
    }
    (conditions, values)
}

/// The rules half of [`build_group_filter`], usable on its own.
///
/// Split out so [`groups_with_rules_matching`] can ask "do this group's RULES
/// match?" without the account membership that is OR'd in beside them —
/// `claude_repos::resolve_for_message` ranks those two signals differently.
/// One matcher, not two: the repo you land in has to agree with the group you
/// see in the sidebar, and it only does that if both read the same SQL.
fn build_rule_conditions(rules: &[InboxGroupRule]) -> (Vec<String>, Vec<String>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();

    for rule in rules {
        let column = match rule.field.as_str() {
            "from_email" => "from_email",
            "from_name" => "from_name",
            "subject" => "subject",
            "to_list" => "to_list",
            _ => continue,
        };

        match rule.operator.as_str() {
            "contains" => {
                conditions.push(format!("{} LIKE ?", column));
                values.push(format!("%{}%", rule.value));
            }
            "equals" => {
                conditions.push(format!("{} = ?", column));
                values.push(rule.value.clone());
            }
            "starts_with" => {
                conditions.push(format!("{} LIKE ?", column));
                values.push(format!("{}%", rule.value));
            }
            "ends_with" => {
                conditions.push(format!("{} LIKE ?", column));
                values.push(format!("%{}", rule.value));
            }
            _ => continue,
        }
    }

    (conditions, values)
}

/// Build a WHERE clause fragment combining account filter and rules (OR'd).
fn build_group_filter(
    rules: &[InboxGroupRule],
    account_ids: &[String],
) -> (String, Vec<String>) {
    let (mut conditions, mut values) = build_account_conditions(account_ids);
    let (rule_conditions, rule_values) = build_rule_conditions(rules);
    conditions.extend(rule_conditions);
    values.extend(rule_values);

    if conditions.is_empty() {
        return ("0".to_string(), vec![]);
    }

    (format!("({})", conditions.join(" OR ")), values)
}

/// The ids of every group whose **rules** match this one message, in the order
/// the sidebar lists them.
///
/// Account membership is deliberately not consulted: it says the group contains
/// this mailbox, not that it contains this message, and
/// `claude_repos::resolve_for_message` wants those ranked apart. Groups with no
/// rules are skipped rather than treated as matching everything — an empty
/// condition list is vacuously true (the same trap `MailRule` has, gotcha #36).
pub fn groups_with_rules_matching(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<i64>, AppError> {
    let mut stmt =
        conn.prepare("SELECT id FROM inbox_groups ORDER BY sort_order, id")?;
    let group_ids: Vec<i64> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut matched = Vec::new();
    for group_id in group_ids {
        let rules = list_rules(conn, group_id)?;
        let (conditions, values) = build_rule_conditions(&rules);
        if conditions.is_empty() {
            continue;
        }
        let sql = format!(
            "SELECT 1 FROM messages
              WHERE account_id = ? AND folder_name = ? AND uid = ? AND ({})
              LIMIT 1",
            conditions.join(" OR ")
        );
        let mut binds: Vec<rusqlite::types::Value> = vec![
            account_id.to_string().into(),
            folder_name.to_string().into(),
            i64::from(uid).into(),
        ];
        binds.extend(values.into_iter().map(rusqlite::types::Value::from));
        let hit: Option<i64> = conn
            .query_row(&sql, rusqlite::params_from_iter(binds.iter()), |row| {
                row.get(0)
            })
            .optional()?;
        if hit.is_some() {
            matched.push(group_id);
        }
    }
    Ok(matched)
}

/// The ids of every group this account belongs to, in sidebar order.
pub fn groups_containing_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<i64>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT g.id FROM inbox_groups g
           JOIN inbox_group_accounts ga ON ga.group_id = g.id
          WHERE ga.account_id = ?1
          ORDER BY g.sort_order, g.id",
    )?;
    let ids = stmt
        .query_map(params![account_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

fn count_unread_for_group(
    conn: &Connection,
    rules: &[InboxGroupRule],
    account_ids: &[String],
) -> Result<u32, AppError> {
    let (filter, values) = build_group_filter(rules, account_ids);
    // The hidden-account rule is ANDed OUTSIDE `build_group_filter`, so an
    // explicit member and a rule match are both suppressed, and the composed
    // filter that test pins byte-for-byte is untouched.
    let sql = format!(
        "SELECT COUNT(*) FROM messages WHERE folder_name = 'INBOX' AND is_read = 0 AND {} AND {}",
        filter,
        super::accounts::visible_in_aggregates_sql("messages.account_id")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
    let count: u32 = stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// Fetch messages matching a group's accounts + rules (across all INBOX folders),
/// collapsed one row per thread (latest representative within INBOX).
pub fn list_messages_for_group(
    conn: &Connection,
    group_id: i64,
    page: u32,
    page_size: u32,
    unread_only: bool,
) -> Result<(Vec<super::messages::MessageRow>, u32), AppError> {
    let rules = list_rules(conn, group_id)?;
    let account_ids = list_account_ids(conn, group_id)?;
    let (filter, values) = build_group_filter(&rules, &account_ids);
    let unread_filter = if unread_only { " AND m.is_read = 0" } else { "" };

    // Hidden wins everywhere: membership and rule matches alike (the rule is
    // ANDed outside `build_group_filter`, whose SQL is pinned by test).
    let eligible_predicate = format!(
        "m.folder_name = 'INBOX' AND m.is_muted = 0{} AND {}
         AND {}
         AND NOT EXISTS (SELECT 1 FROM snoozed_messages s WHERE s.account_id = m.account_id AND s.folder_name = m.folder_name AND s.uid = m.uid)",
        unread_filter,
        filter,
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );

    let count_sql = format!(
        "WITH eligible AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         )
         SELECT COUNT(*) FROM (SELECT DISTINCT account_id, group_key FROM eligible)",
        predicate = eligible_predicate
    );

    let mut stmt = conn.prepare(&count_sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
    let total: u32 = stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;

    let offset = page * page_size;
    let list_sql = format!(
        "WITH eligible AS (
            SELECT m.uid, m.account_id, m.subject, m.from_name, m.from_email, m.date, m.snippet,
                   m.is_read, m.is_flagged, m.has_attachments, m.size_bytes,
                   m.category, m.is_muted, m.is_pinned, m.folder_name,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         ),
         keys AS (SELECT DISTINCT account_id, group_key FROM eligible),
         aggregates AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key,
                   {aggregate_columns}
              FROM messages m
              JOIN keys k
                ON k.account_id = m.account_id
               AND k.group_key = COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid)
             GROUP BY m.account_id, group_key
         ),
         ranked AS (
            SELECT e.*, a.total_count, a.draft_count, a.has_unread_any,
                   ROW_NUMBER() OVER (
                     PARTITION BY e.account_id, e.group_key
                     ORDER BY e.is_pinned DESC, datetime(e.date) DESC, e.uid DESC
                   ) AS rn
              FROM eligible e
              JOIN aggregates a
                ON a.account_id = e.account_id AND a.group_key = e.group_key
         )
         SELECT uid, account_id, subject, from_name, from_email, date, snippet,
                is_read, is_flagged, has_attachments, size_bytes,
                category, is_muted, is_pinned,
                (total_count - 1) AS thread_count,
                draft_count AS thread_draft_count,
                group_key AS thread_root_id,
                has_unread_any AS thread_has_unread
           FROM ranked
          WHERE rn = 1
          ORDER BY is_pinned DESC, datetime(date) DESC
          LIMIT {} OFFSET {}",
        page_size, offset,
        predicate = eligible_predicate,
        aggregate_columns = super::messages::thread_aggregate_columns(None),
    );

    let mut stmt = conn.prepare(&list_sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
    let messages = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(super::messages::MessageRow {
                uid: row.get(0)?,
                account_id: row.get(1)?,
                folder_name: Some("INBOX".to_string()),
                subject: row.get(2)?,
                from_name: row.get(3)?,
                from_email: row.get(4)?,
                date: row.get(5)?,
                snippet: row.get(6)?,
                is_read: row.get::<_, i32>(7)? != 0,
                is_flagged: row.get::<_, i32>(8)? != 0,
                has_attachments: row.get::<_, i32>(9)? != 0,
                size_bytes: row.get(10)?,
                category: row.get(11)?,
                is_muted: row.get::<_, i32>(12)? != 0,
                is_pinned: row.get::<_, i32>(13)? != 0,
                thread_count: row.get::<_, i32>(14).unwrap_or(0) as u32,
                thread_draft_count: row.get::<_, i32>(15).unwrap_or(0) as u32,
                thread_root_id: row.get(16)?,
                thread_has_unread: row.get::<_, i32>(17).unwrap_or(0) != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((messages, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(field: &str, operator: &str, value: &str) -> InboxGroupRule {
        InboxGroupRule {
            id: 0,
            group_id: 1,
            field: field.into(),
            operator: operator.into(),
            value: value.into(),
        }
    }

    /// `build_group_filter` was split into an account half and a rule half so
    /// the rules could be evaluated alone. Splitting a SQL builder that four
    /// live queries depend on is the kind of change that silently shifts the
    /// group unread counts in the sidebar, so this pins the composed output
    /// byte-for-byte — including the order the two halves are concatenated in,
    /// which is what keeps the bound values lined up with the placeholders.
    #[test]
    fn splitting_the_filter_did_not_change_the_sql_it_composes() {
        let rules = vec![
            rule("from_email", "contains", "northwind"),
            rule("subject", "equals", "Weekly"),
            rule("from_name", "starts_with", "Dana"),
            rule("to_list", "ends_with", "@northwind.example"),
            rule("body", "contains", "ignored"),
            rule("subject", "matches_regex", "ignored"),
        ];
        let accounts = vec!["acct-a".to_string(), "acct-b".to_string()];

        let (sql, values) = build_group_filter(&rules, &accounts);
        assert_eq!(
            sql,
            "(account_id = ? OR account_id = ? OR from_email LIKE ? OR subject = ? \
             OR from_name LIKE ? OR to_list LIKE ?)"
        );
        assert_eq!(
            values,
            vec![
                "acct-a",
                "acct-b",
                "%northwind%",
                "Weekly",
                "Dana%",
                "%@northwind.example",
            ]
        );

        // An unmatchable group still has to produce a false predicate, not an
        // empty string that would splice into `WHERE … AND ` and be a syntax
        // error — or worse, `WHERE 1`.
        assert_eq!(build_group_filter(&[], &[]), ("0".to_string(), vec![]));
    }

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE inbox_groups (
                id INTEGER PRIMARY KEY, name TEXT NOT NULL,
                color TEXT NOT NULL DEFAULT '#0a84ff', icon TEXT NOT NULL DEFAULT 'folder',
                sort_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE inbox_group_rules (
                id INTEGER PRIMARY KEY, group_id INTEGER NOT NULL,
                field TEXT NOT NULL, operator TEXT NOT NULL, value TEXT NOT NULL
             );
             CREATE TABLE inbox_group_accounts (
                group_id INTEGER NOT NULL, account_id TEXT NOT NULL,
                PRIMARY KEY (group_id, account_id)
             );
             CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                subject TEXT, from_name TEXT, from_email TEXT, to_list TEXT, cc_list TEXT,
                date TEXT NOT NULL DEFAULT '2026-01-01',
                is_read INTEGER NOT NULL DEFAULT 0, is_muted INTEGER NOT NULL DEFAULT 0,
                is_flagged INTEGER NOT NULL DEFAULT 0, is_pinned INTEGER NOT NULL DEFAULT 0,
                has_attachments INTEGER NOT NULL DEFAULT 0, size_bytes INTEGER NOT NULL DEFAULT 0,
                snippet TEXT, category TEXT, message_id TEXT, thread_root_id TEXT,
                UNIQUE(account_id, folder_name, uid)
             );
             CREATE TABLE snoozed_messages (
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL
             );
             -- The predicates the group queries embed read these two tables;
             -- a fixture without them dies with `no such table: accounts`
             -- (the hidden-account rule) or `no such table: folders` (the
             -- unsent-draft rule). Both fail open on an empty table, so no
             -- rows are needed — only the table.
             CREATE TABLE accounts (
                id TEXT PRIMARY KEY, hidden_from_aggregates INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE folders (
                account_id TEXT NOT NULL, name TEXT NOT NULL, folder_type TEXT
             );
             INSERT INTO inbox_groups (id, name, sort_order) VALUES
                (1, 'Business', 0), (2, 'Music', 1), (3, 'Northwind', 3);
             INSERT INTO inbox_group_rules (group_id, field, operator, value) VALUES
                (2, 'from_email', 'contains', 'spotify'),
                (3, 'subject', 'contains', 'Northwind');
             INSERT INTO inbox_group_accounts (group_id, account_id) VALUES (1, 'acct-cxv');
             INSERT INTO messages (account_id, folder_name, uid, subject, from_email) VALUES
                ('acct-cxv', 'INBOX', 1, 'Northwind weekly sync', 'dana@northwind.example'),
                ('acct-cxv', 'INBOX', 2, 'Your monthly report', 'no-reply@spotify.com'),
                ('acct-cxv', 'INBOX', 3, 'lunch?', 'friend@example.com');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn rule_matching_finds_only_the_groups_whose_rules_hit() {
        let conn = test_db();
        assert_eq!(
            groups_with_rules_matching(&conn, "acct-cxv", "INBOX", 1).unwrap(),
            vec![3],
            "the Northwind subject rule"
        );
        assert_eq!(
            groups_with_rules_matching(&conn, "acct-cxv", "INBOX", 2).unwrap(),
            vec![2],
            "the Spotify sender rule"
        );
        assert_eq!(
            groups_with_rules_matching(&conn, "acct-cxv", "INBOX", 3).unwrap(),
            Vec::<i64>::new(),
            "no rule matches, and Business — which has no rules at all — must \
             not match everything"
        );
        assert_eq!(
            groups_with_rules_matching(&conn, "acct-cxv", "INBOX", 999).unwrap(),
            Vec::<i64>::new(),
            "a uid that isn't in the DB matches nothing rather than erroring"
        );
    }

    /// Hidden wins everywhere. `acct-warm` is BOTH an explicit member of
    /// Business AND the sender of a message the Northwind rule matches; once it is
    /// hidden, neither route puts its mail in a group — in the list or in the
    /// sidebar's unread count. `acct-cxv` alongside it proves the group itself
    /// still works.
    #[test]
    fn a_hidden_account_is_absent_from_groups_by_membership_and_by_rule() {
        let conn = test_db();
        conn.execute_batch(
            "INSERT INTO accounts (id, hidden_from_aggregates) VALUES
                ('acct-cxv', 0), ('acct-warm', 1);
             INSERT INTO inbox_group_accounts (group_id, account_id) VALUES (1, 'acct-warm');
             INSERT INTO messages (account_id, folder_name, uid, subject, from_email) VALUES
                ('acct-warm', 'INBOX', 1, 'warmup: Northwind numbers', 'warm@peer.test'),
                ('acct-warm', 'INBOX', 2, 'warmup: plain', 'warm@peer.test');",
        )
        .unwrap();

        let ids = |group: i64| -> Vec<String> {
            list_messages_for_group(&conn, group, 0, 50, false)
                .unwrap()
                .0
                .into_iter()
                .filter_map(|m| m.account_id)
                .collect()
        };
        // Business (membership): three cxv rows, no warm rows.
        assert_eq!(ids(1), vec!["acct-cxv"; 3]);
        // Northwind (rule on subject): the cxv row only, not the warm one that matches.
        assert_eq!(ids(3), vec!["acct-cxv"]);

        let unread: Vec<(String, u32)> = list_groups(&conn)
            .unwrap()
            .into_iter()
            .map(|g| (g.name, g.unread_count))
            .collect();
        assert_eq!(
            unread,
            vec![("Business".into(), 3), ("Music".into(), 1), ("Northwind".into(), 1)]
        );

        // Un-hide and both routes light up again — the negative control.
        conn.execute("UPDATE accounts SET hidden_from_aggregates = 0", []).unwrap();
        assert_eq!(ids(1).len(), 5);
        assert_eq!(ids(3).len(), 2);
    }

    #[test]
    fn membership_lookup_is_separate_from_rule_matching() {
        let conn = test_db();
        assert_eq!(
            groups_containing_account(&conn, "acct-cxv").unwrap(),
            vec![1],
            "Business contains the account; Northwind matches its mail by rule only"
        );
        assert_eq!(
            groups_containing_account(&conn, "acct-unknown").unwrap(),
            Vec::<i64>::new()
        );
    }
}
