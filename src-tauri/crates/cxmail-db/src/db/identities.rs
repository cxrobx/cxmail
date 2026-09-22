use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: Option<i64>,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub signature_html: Option<String>,
    pub is_default: bool,
}

pub fn insert(conn: &Connection, identity: &Identity) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO identities (account_id, email, display_name, signature_html, is_default) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![identity.account_id, identity.email, identity.display_name, identity.signature_html, identity.is_default as i32],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_by_account(conn: &Connection, account_id: &str) -> Result<Vec<Identity>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email, display_name, signature_html, is_default FROM identities WHERE account_id = ?1 ORDER BY is_default DESC",
    )?;
    let identities = stmt
        .query_map(params![account_id], |row| {
            Ok(Identity {
                id: row.get(0)?,
                account_id: row.get(1)?,
                email: row.get(2)?,
                display_name: row.get(3)?,
                signature_html: row.get(4)?,
                is_default: row.get::<_, i32>(5)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(identities)
}

pub fn update(conn: &Connection, identity: &Identity) -> Result<(), AppError> {
    conn.execute(
        "UPDATE identities SET email = ?2, display_name = ?3, signature_html = ?4, is_default = ?5 WHERE id = ?1",
        params![identity.id, identity.email, identity.display_name, identity.signature_html, identity.is_default as i32],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM identities WHERE id = ?1", params![id])?;
    Ok(())
}
