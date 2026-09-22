use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TemplateRow {
    pub id: i64,
    pub account_id: Option<String>,
    pub name: String,
    pub subject: String,
    pub html_body: String,
    pub plain_body: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list(conn: &Connection, account_id: Option<&str>) -> Result<Vec<TemplateRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, name, subject, html_body, plain_body, created_at, updated_at
         FROM email_templates
         WHERE account_id IS NULL OR account_id = ?1
         ORDER BY name ASC",
    )?;
    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok(TemplateRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                subject: row.get(3)?,
                html_body: row.get(4)?,
                plain_body: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn insert(
    conn: &Connection,
    account_id: Option<&str>,
    name: &str,
    subject: &str,
    html_body: &str,
    plain_body: Option<&str>,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO email_templates (account_id, name, subject, html_body, plain_body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![account_id, name, subject, html_body, plain_body],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    subject: &str,
    html_body: &str,
    plain_body: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE email_templates SET name = ?1, subject = ?2, html_body = ?3, plain_body = ?4, updated_at = datetime('now')
         WHERE id = ?5",
        params![name, subject, html_body, plain_body, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM email_templates WHERE id = ?1", params![id])?;
    Ok(())
}
