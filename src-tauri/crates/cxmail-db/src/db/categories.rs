use crate::error::AppError;
use rusqlite::{params, Connection};
use std::collections::HashMap;

pub fn get_sender_category(conn: &Connection, email: &str) -> Result<Option<String>, AppError> {
    // Try exact email match first, then domain match
    let result: Option<String> = conn
        .query_row(
            "SELECT category FROM sender_categories WHERE email_pattern = ?1",
            params![email],
            |row| row.get(0),
        )
        .ok();

    if result.is_some() {
        return Ok(result);
    }

    // Try domain match
    if let Some(domain) = email.split('@').nth(1) {
        let domain_result: Option<String> = conn
            .query_row(
                "SELECT category FROM sender_categories WHERE email_pattern = ?1",
                params![domain],
                |row| row.get(0),
            )
            .ok();
        return Ok(domain_result);
    }

    Ok(None)
}

pub fn set_sender_category(
    conn: &Connection,
    email_pattern: &str,
    category: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO sender_categories (email_pattern, category)
         VALUES (?1, ?2)
         ON CONFLICT(email_pattern) DO UPDATE SET category = excluded.category",
        params![email_pattern, category],
    )?;
    Ok(())
}

pub fn update_message_category(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    category: &str,
    source: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE messages SET category = ?1, category_source = ?2
         WHERE account_id = ?3 AND folder_name = ?4 AND uid = ?5",
        params![category, source, account_id, folder_name, uid],
    )?;
    Ok(())
}

pub fn get_message_categories(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uids: &[u32],
) -> Result<HashMap<u32, String>, AppError> {
    if uids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders: Vec<String> = uids.iter().enumerate().map(|(i, _)| format!("?{}", i + 3)).collect();
    let sql = format!(
        "SELECT uid, COALESCE(category, 'primary') FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&account_id, &folder_name];
    for uid in uids {
        params.push(uid as &dyn rusqlite::types::ToSql);
    }
    let mut result = HashMap::new();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (uid, cat) = row?;
        result.insert(uid, cat);
    }
    Ok(result)
}

pub fn count_unread_by_category(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<HashMap<String, u32>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(category, 'primary'), COUNT(*)
         FROM messages
         WHERE account_id = ?1 AND folder_name = ?2 AND is_read = 0
         GROUP BY category",
    )?;
    let mut counts = HashMap::new();
    let rows = stmt.query_map(params![account_id, folder_name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    for row in rows {
        let (cat, count) = row?;
        counts.insert(cat, count);
    }
    Ok(counts)
}

/// Category-tab counts for All Inboxes. Accounts hidden from aggregates are
/// not counted — the tabs sit above a list that does not show their mail.
pub fn count_unread_by_category_unified(
    conn: &Connection,
) -> Result<HashMap<String, u32>, AppError> {
    let sql = format!(
        "SELECT COALESCE(category, 'primary'), COUNT(*)
         FROM messages m
         JOIN folders f ON m.account_id = f.account_id AND m.folder_name = f.name
         WHERE f.folder_type = 'inbox' AND m.is_read = 0
           AND {}
         GROUP BY m.category",
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut counts = HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    for row in rows {
        let (cat, count) = row?;
        counts.insert(cat, count);
    }
    Ok(counts)
}

/// Category-tab counts for an account folder. Still an aggregate, so a hidden
/// member of the folder is not counted even though its id is named.
pub fn count_unread_by_category_for_accounts(
    conn: &Connection,
    account_ids: &[String],
) -> Result<HashMap<String, u32>, AppError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders: Vec<String> = account_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT COALESCE(category, 'primary'), COUNT(*)
         FROM messages
         WHERE folder_name = 'INBOX' AND is_read = 0 AND account_id IN ({})
           AND {}
         GROUP BY category",
        placeholders.join(", "),
        super::accounts::visible_in_aggregates_sql("messages.account_id")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        account_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
    let mut counts = HashMap::new();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    for row in rows {
        let (cat, count) = row?;
        counts.insert(cat, count);
    }
    Ok(counts)
}

/// The two aggregate counters skip a hidden account; the per-account counter
/// — what the tabs show inside the hidden account itself — does not.
#[cfg(test)]
mod aggregate_visibility_tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        for (id, hidden) in [("a", false), ("h", true)] {
            conn.execute(
                "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
                 VALUES (?1, ?1 || '@example.com','imap','imap.example.com','smtp.example.com')",
                params![id],
            )
            .unwrap();
            super::super::accounts::set_hidden_from_aggregates(&conn, id, hidden).unwrap();
            conn.execute(
                "INSERT INTO folders (account_id,name,folder_type) VALUES (?1,'INBOX','inbox')",
                params![id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (account_id,folder_name,uid,subject,date,is_read,category)
                 VALUES (?1,'INBOX',1,'hi','2026-08-01T00:00:00+00:00',0,'primary')",
                params![id],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn unified_counts_skip_the_hidden_account() {
        let conn = setup();
        let counts = count_unread_by_category_unified(&conn).unwrap();
        assert_eq!(counts.get("primary"), Some(&1));
    }

    #[test]
    fn account_folder_counts_skip_a_hidden_member_even_when_named() {
        let conn = setup();
        let counts =
            count_unread_by_category_for_accounts(&conn, &["a".to_string(), "h".to_string()])
                .unwrap();
        assert_eq!(counts.get("primary"), Some(&1));
    }

    #[test]
    fn the_hidden_accounts_own_tabs_still_count_its_mail() {
        let conn = setup();
        let counts = count_unread_by_category(&conn, "h", "INBOX").unwrap();
        assert_eq!(counts.get("primary"), Some(&1));
    }
}
