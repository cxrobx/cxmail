use crate::error::AppError;
use rusqlite::{params, Connection};

pub fn record(
    conn: &Connection,
    account_id: &str,
    sender_email: &str,
    method: &str,
) -> Result<(), AppError> {
    let email = sender_email.to_lowercase();
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO unsubscribed_senders
         (account_id, sender_email, sender_domain, method)
         VALUES (?1, ?2, ?3, ?4)",
        params![account_id, email, domain, method],
    )?;
    Ok(())
}

pub fn list_emails(
    conn: &Connection,
    account_id: Option<&str>,
) -> Result<Vec<String>, AppError> {
    if let Some(acct) = account_id {
        let mut stmt = conn.prepare(
            "SELECT sender_email FROM unsubscribed_senders WHERE account_id = ?1",
        )?;
        let emails = stmt
            .query_map(params![acct], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(emails)
    } else {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT sender_email FROM unsubscribed_senders",
        )?;
        let emails = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(emails)
    }
}

pub fn remove(
    conn: &Connection,
    account_id: &str,
    sender_email: &str,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM unsubscribed_senders WHERE account_id = ?1 AND sender_email = ?2",
        params![account_id, sender_email.to_lowercase()],
    )?;
    Ok(())
}

pub fn check_senders(
    conn: &Connection,
    account_id: &str,
    from_emails: &[&str],
) -> Result<Vec<String>, AppError> {
    if from_emails.is_empty() {
        return Ok(Vec::new());
    }
    let mut matched = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT 1 FROM unsubscribed_senders WHERE account_id = ?1 AND sender_email = ?2",
    )?;
    for email in from_emails {
        let exists: bool = stmt
            .query_row(params![account_id, email.to_lowercase()], |_| Ok(true))
            .unwrap_or(false);
        if exists {
            matched.push(email.to_string());
        }
    }
    Ok(matched)
}
