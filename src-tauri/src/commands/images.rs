use crate::error::AppError;
use base64::Engine;
use futures::StreamExt;
use std::io::Cursor;
use std::time::Duration;
use tauri_plugin_clipboard_manager::ClipboardExt;

const MAX_DOWNLOAD_BYTES: usize = 25 * 1024 * 1024;
const MAX_DECODED_ALLOC: u64 = 64 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetch raw image bytes from either a `data:image/...;base64,...` URL or an
/// `https://` URL. Capped at MAX_DOWNLOAD_BYTES; https reads are streamed so
/// an oversized payload aborts before fully buffering.
///
/// SSRF note: we refuse non-`https` schemes and refuse non-`image/*` content
/// types, but we do NOT reject public hostnames that resolve to private IP
/// ranges. The 25 MB / 15 s caps bound the worst case; full SSRF defense
/// would require a custom resolver.
async fn fetch_image_bytes(src: &str) -> Result<Vec<u8>, AppError> {
    // data: URL branch
    if src.starts_with("data:image/") {
        let comma = src
            .find(',')
            .ok_or_else(|| AppError::Parse("data: URL missing comma separator".into()))?;
        let header = &src[..comma];
        if !header.contains(";base64") {
            return Err(AppError::Parse(
                "only base64-encoded data: URLs are supported".into(),
            ));
        }
        let encoded = &src[comma + 1..];
        if encoded.len().saturating_mul(3) / 4 > MAX_DOWNLOAD_BYTES {
            return Err(AppError::General("image exceeds maximum size".into()));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(|e| AppError::Parse(format!("data: URL decode failed: {e}")))?;
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            return Err(AppError::General("image exceeds maximum size".into()));
        }
        return Ok(bytes);
    }

    // https: URL branch (HTTPS only)
    let parsed = url::Url::parse(src)
        .map_err(|e| AppError::Parse(format!("invalid image URL: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::General(
            "only https image URLs are allowed".into(),
        ));
    }

    // https_only(true) enforces the scheme check on every URL in the redirect
    // chain — the up-front `parsed.scheme() != "https"` guard only covers the
    // initial src, so without this a 302 to http:// would silently downgrade.
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .https_only(true)
        .build()
        .map_err(|e| AppError::General(format!("client build failed: {e}")))?;

    let resp = client
        .get(src)
        .send()
        .await
        .map_err(|e| AppError::General(format!("image fetch failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::General(format!(
            "image fetch returned {}",
            status
        )));
    }

    let is_image = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false);
    if !is_image {
        return Err(AppError::Parse("response is not an image".into()));
    }

    if let Some(len) = resp.content_length() {
        if len as usize > MAX_DOWNLOAD_BYTES {
            return Err(AppError::General("image exceeds maximum size".into()));
        }
    }

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::General(format!("stream error: {e}")))?;
        if buf.len() + chunk.len() > MAX_DOWNLOAD_BYTES {
            return Err(AppError::General("image exceeds maximum size".into()));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

#[tauri::command]
pub async fn save_image_to_path(src: String, dest_path: String) -> Result<(), AppError> {
    if dest_path.is_empty() {
        return Err(AppError::General("destination path is empty".into()));
    }
    let bytes = fetch_image_bytes(&src).await?;
    std::fs::write(&dest_path, &bytes).map_err(AppError::Io)?;
    log::info!(
        "save_image_to_path: wrote {} bytes to {}",
        bytes.len(),
        dest_path
    );
    Ok(())
}

#[tauri::command]
pub async fn copy_image_to_clipboard(
    app: tauri::AppHandle,
    src: String,
) -> Result<(), AppError> {
    let bytes = fetch_image_bytes(&src).await?;

    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| AppError::Parse(format!("image format detection failed: {e}")))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_ALLOC);
    reader.limits(limits);
    let dyn_img = reader
        .decode()
        .map_err(|e| AppError::Parse(format!("image decode failed: {e}")))?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    let tauri_img = tauri::image::Image::new(&raw, w, h);
    app.clipboard()
        .write_image(&tauri_img)
        .map_err(|e| AppError::General(format!("clipboard write failed: {e}")))?;
    Ok(())
}
