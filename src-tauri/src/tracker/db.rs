use rusqlite::{params, Connection, Result};
use sha2::{Digest, Sha256};

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS pixels (
            pixel_code  TEXT PRIMARY KEY,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS open_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            pixel_code  TEXT NOT NULL REFERENCES pixels(pixel_code),
            opened_at   TEXT NOT NULL DEFAULT (datetime('now')),
            ip_hash     TEXT,
            user_agent  TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_open_events_code ON open_events(pixel_code);
        CREATE INDEX IF NOT EXISTS idx_open_events_time ON open_events(opened_at);

        CREATE TABLE IF NOT EXISTS api_keys (
            key_hash    TEXT PRIMARY KEY,
            label       TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    Ok(())
}

pub fn register_pixel(conn: &Connection, pixel_code: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO pixels (pixel_code) VALUES (?1)",
        params![pixel_code],
    )?;
    Ok(())
}

pub fn pixel_exists(conn: &Connection, pixel_code: &str) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pixels WHERE pixel_code = ?1)",
        params![pixel_code],
        |row| row.get(0),
    )?;
    Ok(exists)
}

/// Check if an open event with the same pixel_code and ip_hash exists within the last 5 minutes.
pub fn is_duplicate_open(conn: &Connection, pixel_code: &str, ip_hash: &str) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM open_events
            WHERE pixel_code = ?1 AND ip_hash = ?2
            AND opened_at > datetime('now', '-5 minutes')
        )",
        params![pixel_code, ip_hash],
        |row| row.get(0),
    )?;
    Ok(exists)
}

pub fn record_open(
    conn: &Connection,
    pixel_code: &str,
    ip_hash: &str,
    user_agent: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO open_events (pixel_code, ip_hash, user_agent) VALUES (?1, ?2, ?3)",
        params![pixel_code, ip_hash, user_agent],
    )?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct PixelEvents {
    pub pixel_code: String,
    pub open_count: i32,
    pub first_open_at: Option<String>,
    pub last_open_at: Option<String>,
}

/// Get aggregated open events for all pixels with events since the given timestamp.
pub fn get_events_since(conn: &Connection, since: &str) -> Result<Vec<PixelEvents>> {
    let mut stmt = conn.prepare(
        "SELECT
            pixel_code,
            COUNT(*) as open_count,
            MIN(opened_at) as first_open_at,
            MAX(opened_at) as last_open_at
         FROM open_events
         WHERE pixel_code IN (
            SELECT DISTINCT pixel_code FROM open_events WHERE opened_at > ?1
         )
         GROUP BY pixel_code",
    )?;
    let events = stmt
        .query_map(params![since], |row| {
            Ok(PixelEvents {
                pixel_code: row.get(0)?,
                open_count: row.get(1)?,
                first_open_at: row.get(2)?,
                last_open_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(events)
}

pub fn store_api_key_hash(conn: &Connection, key_hash: &str, label: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT INTO api_keys (key_hash, label) VALUES (?1, ?2)",
        params![key_hash, label],
    )?;
    Ok(())
}

pub fn validate_api_key(conn: &Connection, raw_key: &str) -> Result<bool> {
    let hash = hash_api_key(raw_key);
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM api_keys WHERE key_hash = ?1)",
        params![hash],
        |row| row.get(0),
    )?;
    Ok(exists)
}

pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Hash an IP address with a daily rotating salt for privacy-preserving dedup.
pub fn hash_ip(ip: &str) -> String {
    let date_salt = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut hasher = Sha256::new();
    hasher.update(ip.as_bytes());
    hasher.update(date_salt.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn cleanup_old_events(conn: &Connection, retention_days: u32) -> Result<usize> {
    let affected = conn.execute(
        "DELETE FROM open_events WHERE opened_at < datetime('now', ?1)",
        params![format!("-{} days", retention_days)],
    )?;
    Ok(affected)
}
