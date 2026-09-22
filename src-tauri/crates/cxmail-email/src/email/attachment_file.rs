//! Reading an attachment off disk.
//!
//! Shared by the Tauri compose path (which base64-encodes the bytes for IPC)
//! and the MCP send path (which feeds them straight to `MessageBuilder`), so it
//! sits below both rather than in `commands/`. The 25 MB cap is enforced here
//! precisely because there are two callers — a limit that lives in one of them
//! is a limit the other forgets.

use crate::error::AppError;

/// Read a file from disk, enforce the 25 MB attachment cap, and return
/// (filename, MIME type, raw bytes). Shared between the Tauri compose path
/// (which then base64-encodes for IPC) and the MCP path (which feeds bytes
/// straight into `MessageBuilder::attachment`).
pub fn read_attachment_from_path(
    path: &str,
) -> Result<(String, String, Vec<u8>), AppError> {
    let filepath = std::path::PathBuf::from(path);
    let data = std::fs::read(&filepath).map_err(AppError::Io)?;

    if data.len() > 25 * 1024 * 1024 {
        return Err(AppError::General(
            "File exceeds 25 MB attachment limit".to_string(),
        ));
    }

    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_string();

    let content_type = guess_content_type(&filename);
    Ok((filename, content_type, data))
}

pub fn guess_content_type(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "ppt" | "pptx" => "application/vnd.ms-powerpoint",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "ics" => "text/calendar",
        _ => "application/octet-stream",
    }
    .to_string()
}
