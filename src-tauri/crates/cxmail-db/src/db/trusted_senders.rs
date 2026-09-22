use crate::error::AppError;
use rusqlite::{params, Connection};

/// Add a sender to the global remote-image allowlist. Idempotent: re-adding
/// an existing sender is a no-op (we keep the original `added_at`).
pub fn add(conn: &Connection, sender_email: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR IGNORE INTO trusted_image_senders (sender_email) VALUES (?1)",
        params![sender_email.to_lowercase()],
    )?;
    Ok(())
}

pub fn remove(conn: &Connection, sender_email: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM trusted_image_senders WHERE sender_email = ?1",
        params![sender_email.to_lowercase()],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt =
        conn.prepare("SELECT sender_email FROM trusted_image_senders ORDER BY sender_email")?;
    let emails = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(emails)
}

pub fn is_trusted(conn: &Connection, sender_email: &str) -> Result<bool, AppError> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM trusted_image_senders WHERE sender_email = ?1",
            params![sender_email.to_lowercase()],
            |_| Ok(true),
        )
        .unwrap_or(false);
    Ok(exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE trusted_image_senders (
                sender_email TEXT PRIMARY KEY,
                added_at     INTEGER NOT NULL DEFAULT (unixepoch())
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn add_then_is_trusted() {
        let conn = fresh_conn();
        add(&conn, "Foo@Example.com").unwrap();
        assert!(is_trusted(&conn, "foo@example.com").unwrap());
        assert!(is_trusted(&conn, "FOO@EXAMPLE.COM").unwrap());
        assert!(!is_trusted(&conn, "bar@example.com").unwrap());
    }

    #[test]
    fn add_is_idempotent() {
        let conn = fresh_conn();
        add(&conn, "a@b.com").unwrap();
        add(&conn, "a@b.com").unwrap();
        add(&conn, "A@B.COM").unwrap();
        let listed = list(&conn).unwrap();
        assert_eq!(listed, vec!["a@b.com".to_string()]);
    }

    #[test]
    fn remove_is_case_insensitive() {
        let conn = fresh_conn();
        add(&conn, "x@y.com").unwrap();
        remove(&conn, "X@Y.COM").unwrap();
        assert!(!is_trusted(&conn, "x@y.com").unwrap());
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_orders_alphabetically() {
        let conn = fresh_conn();
        add(&conn, "c@x.com").unwrap();
        add(&conn, "a@x.com").unwrap();
        add(&conn, "b@x.com").unwrap();
        assert_eq!(list(&conn).unwrap(), vec!["a@x.com", "b@x.com", "c@x.com"]);
    }
}
