use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

use super::db as tracker_db;
use crate::LockExt;

/// 1x1 transparent GIF (43 bytes)
const TRANSPARENT_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00,
    0x80, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x21,
    0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44,
    0x01, 0x00, 0x3b,
];

pub struct AppState {
    pub db: std::sync::Mutex<rusqlite::Connection>,
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/t/{code}", get(serve_pixel))
        .route("/api/pixels", post(register_pixel))
        .route("/api/events", get(get_events))
        .route("/api/health", get(health))
        .with_state(state)
}

/// Serve 1x1 tracking pixel and record the open event.
async fn serve_pixel(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Strip .png extension if present
    let pixel_code = code.trim_end_matches(".png");

    // Always serve the GIF regardless of whether we can process the open
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/gif")
        .header(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate")
        .header(header::PRAGMA, "no-cache")
        .header(header::EXPIRES, "0")
        .body(Body::from(TRANSPARENT_GIF.to_vec()))
        .unwrap();

    // Process the open event asynchronously
    let conn = state.db.safe_lock();
    if tracker_db::pixel_exists(&conn, pixel_code).unwrap_or(false) {
        let ip = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .unwrap_or("unknown");
        let ip_hash = tracker_db::hash_ip(ip);

        if !tracker_db::is_duplicate_open(&conn, pixel_code, &ip_hash).unwrap_or(true) {
            let user_agent = headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok());
            let _ = tracker_db::record_open(&conn, pixel_code, &ip_hash, user_agent);
        }
    }

    response
}

#[derive(serde::Deserialize)]
struct RegisterPixelRequest {
    pixel_code: String,
}

/// Register a new pixel code (called by CXMail desktop before sending).
async fn register_pixel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RegisterPixelRequest>,
) -> impl IntoResponse {
    // Validate API key
    let api_key = match extract_bearer_token(&headers) {
        Some(key) => key,
        None => return StatusCode::UNAUTHORIZED,
    };

    let conn = state.db.safe_lock();
    if !tracker_db::validate_api_key(&conn, &api_key).unwrap_or(false) {
        return StatusCode::UNAUTHORIZED;
    }

    match tracker_db::register_pixel(&conn, &payload.pixel_code) {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(serde::Deserialize)]
struct EventsQuery {
    since: Option<String>,
}

/// Return aggregated open events since a given timestamp.
async fn get_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> impl IntoResponse {
    let api_key = match extract_bearer_token(&headers) {
        Some(key) => key,
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({}))),
    };

    let conn = state.db.safe_lock();
    if !tracker_db::validate_api_key(&conn, &api_key).unwrap_or(false) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})));
    }

    let since = query.since.unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    match tracker_db::get_events_since(&conn, &since) {
        Ok(events) => (StatusCode::OK, Json(serde_json::json!({ "events": events }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Failed to fetch events" }))),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}
