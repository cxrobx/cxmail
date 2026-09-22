use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingPixel {
    pub id: Option<i64>,
    pub pixel_code: String,
    pub account_id: String,
    pub message_id: Option<String>,
    pub to_email: String,
    pub subject: Option<String>,
    pub open_count: i32,
    pub first_open_at: Option<String>,
    pub last_open_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingConfig {
    pub is_enabled: bool,
    pub service_url: Option<String>,
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    pub api_key_configured: bool,
    pub last_synced: Option<String>,
}

const TRACKING_API_KEY: &str = "tracking:api_key";

pub fn insert_pixel(conn: &Connection, pixel: &TrackingPixel) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO tracking_pixels (pixel_code, account_id, message_id, to_email, subject) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![pixel.pixel_code, pixel.account_id, pixel.message_id, pixel.to_email, pixel.subject],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_by_account(conn: &Connection, account_id: &str, limit: i64, offset: i64) -> Result<Vec<TrackingPixel>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, pixel_code, account_id, message_id, to_email, subject, open_count, first_open_at, last_open_at, created_at
         FROM tracking_pixels WHERE account_id = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
    )?;
    let pixels = stmt
        .query_map(params![account_id, limit, offset], |row| {
            Ok(TrackingPixel {
                id: row.get(0)?,
                pixel_code: row.get(1)?,
                account_id: row.get(2)?,
                message_id: row.get(3)?,
                to_email: row.get(4)?,
                subject: row.get(5)?,
                open_count: row.get(6)?,
                first_open_at: row.get(7)?,
                last_open_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pixels)
}

pub fn list_all(conn: &Connection, limit: i64, offset: i64) -> Result<Vec<TrackingPixel>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, pixel_code, account_id, message_id, to_email, subject, open_count, first_open_at, last_open_at, created_at
         FROM tracking_pixels ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let pixels = stmt
        .query_map(params![limit, offset], |row| {
            Ok(TrackingPixel {
                id: row.get(0)?,
                pixel_code: row.get(1)?,
                account_id: row.get(2)?,
                message_id: row.get(3)?,
                to_email: row.get(4)?,
                subject: row.get(5)?,
                open_count: row.get(6)?,
                first_open_at: row.get(7)?,
                last_open_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pixels)
}

pub fn get_by_message_id(conn: &Connection, message_id: &str) -> Result<Option<TrackingPixel>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, pixel_code, account_id, message_id, to_email, subject, open_count, first_open_at, last_open_at, created_at
         FROM tracking_pixels WHERE message_id = ?1",
    )?;
    let mut rows = stmt.query_map(params![message_id], |row| {
        Ok(TrackingPixel {
            id: row.get(0)?,
            pixel_code: row.get(1)?,
            account_id: row.get(2)?,
            message_id: row.get(3)?,
            to_email: row.get(4)?,
            subject: row.get(5)?,
            open_count: row.get(6)?,
            first_open_at: row.get(7)?,
            last_open_at: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    match rows.next() {
        Some(pixel) => Ok(Some(pixel?)),
        None => Ok(None),
    }
}

pub fn get_by_code(conn: &Connection, pixel_code: &str) -> Result<Option<TrackingPixel>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, pixel_code, account_id, message_id, to_email, subject, open_count, first_open_at, last_open_at, created_at
         FROM tracking_pixels WHERE pixel_code = ?1",
    )?;
    let mut rows = stmt.query_map(params![pixel_code], |row| {
        Ok(TrackingPixel {
            id: row.get(0)?,
            pixel_code: row.get(1)?,
            account_id: row.get(2)?,
            message_id: row.get(3)?,
            to_email: row.get(4)?,
            subject: row.get(5)?,
            open_count: row.get(6)?,
            first_open_at: row.get(7)?,
            last_open_at: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    match rows.next() {
        Some(pixel) => Ok(Some(pixel?)),
        None => Ok(None),
    }
}

pub fn update_opens(conn: &Connection, pixel_code: &str, open_count: i32, first_open_at: Option<&str>, last_open_at: Option<&str>) -> Result<(), AppError> {
    conn.execute(
        "UPDATE tracking_pixels SET open_count = ?2, first_open_at = COALESCE(?3, first_open_at), last_open_at = ?4 WHERE pixel_code = ?1",
        params![pixel_code, open_count, first_open_at, last_open_at],
    )?;
    Ok(())
}

pub fn get_config(conn: &Connection) -> Result<TrackingConfig, AppError> {
    migrate_legacy_api_key(conn)?;
    let mut stmt = conn.prepare(
        "SELECT is_enabled, service_url, last_synced FROM tracking_config WHERE id = 1",
    )?;
    let mut rows = stmt.query_map([], |row| {
        Ok(TrackingConfig {
            is_enabled: row.get::<_, i32>(0)? != 0,
            service_url: row.get(1)?,
            api_key: None,
            api_key_configured: crate::keychain::get_credential(TRACKING_API_KEY)
                .ok()
                .flatten()
                .is_some(),
            last_synced: row.get(2)?,
        })
    })?;
    match rows.next() {
        Some(config) => Ok(config?),
        None => Ok(TrackingConfig {
            is_enabled: false,
            service_url: None,
            api_key: None,
            api_key_configured: false,
            last_synced: None,
        }),
    }
}

pub fn set_config(conn: &Connection, config: &TrackingConfig) -> Result<(), AppError> {
    if let Some(api_key) = config.api_key.as_deref().filter(|key| !key.trim().is_empty()) {
        crate::keychain::store_credential(TRACKING_API_KEY, api_key.trim())?;
    }
    conn.execute(
        "INSERT OR REPLACE INTO tracking_config (id, is_enabled, service_url, api_key, last_synced) VALUES (1, ?1, ?2, NULL, ?3)",
        params![config.is_enabled as i32, config.service_url, config.last_synced],
    )?;
    Ok(())
}

pub fn get_api_key(conn: &Connection) -> Result<Option<String>, AppError> {
    migrate_legacy_api_key(conn)?;
    crate::keychain::get_credential(TRACKING_API_KEY)
}

pub fn migrate_legacy_api_key(conn: &Connection) -> Result<(), AppError> {
    if crate::keychain::get_credential(TRACKING_API_KEY)?.is_none() {
        let legacy: Option<String> = conn
            .query_row(
                "SELECT api_key FROM tracking_config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(None);
        if let Some(key) = legacy.filter(|key| !key.trim().is_empty()) {
            crate::keychain::store_credential(TRACKING_API_KEY, key.trim())?;
        }
    }
    conn.execute("UPDATE tracking_config SET api_key = NULL WHERE api_key IS NOT NULL", [])?;
    Ok(())
}

pub fn update_last_synced(conn: &Connection, last_synced: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE tracking_config SET last_synced = ?1 WHERE id = 1",
        params![last_synced],
    )?;
    Ok(())
}
