//! IPC for the "Open in Claude" repo mappings.
//!
//! Thin over `db::claude_repos` — validation lives there so the MCP and any
//! future caller get the same rules, and `RepoKey` arrives as a tagged enum so
//! a contradictory scope (a contact row carrying a group id) cannot be spelled
//! on the wire at all.

use serde::Serialize;
use tauri::{AppHandle, State};

use super::claude_handoff;
use crate::db;
use crate::db::claude_repos::{RepoKey, RepoMapping, RepoScope};
use crate::error::AppError;
use crate::{AppState, LockExt};

/// A mapping plus whether its directory is on this filesystem right now.
///
/// `exists` is computed per read rather than stored: a repo on an unmounted
/// volume is still a correct mapping, and the answer changes without anybody
/// editing the row. The settings panel shows it so a stale path is visible
/// before a handoff silently falls back to the scratch directory.
#[derive(Debug, Clone, Serialize)]
pub struct ClaudeRepoView {
    pub scope: RepoScope,
    pub contact: Option<String>,
    pub group_id: Option<i64>,
    pub account_id: Option<String>,
    pub repo_path: String,
    pub exists: bool,
}

/// With a Herdr host the session runs there, so this Mac's filesystem says
/// nothing about the mapping: a repo can live on the host alone, and a mapping
/// missing there is reported by the handoff. No warning beats a wrong one.
fn view_for(mapping: RepoMapping, herdr_host: Option<&str>) -> ClaudeRepoView {
    let mut view = ClaudeRepoView::from(mapping);
    view.exists |= herdr_host.is_some();
    view
}

impl From<RepoMapping> for ClaudeRepoView {
    fn from(m: RepoMapping) -> Self {
        let exists = std::path::Path::new(&m.repo_path).is_dir();
        ClaudeRepoView {
            scope: m.scope,
            contact: m.contact,
            group_id: m.group_id,
            account_id: m.account_id,
            repo_path: m.repo_path,
            exists,
        }
    }
}

#[tauri::command]
pub async fn list_claude_repos(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ClaudeRepoView>, AppError> {
    let host = claude_handoff::herdr_host(&app)?;
    let conn = state.db.safe_lock();
    Ok(db::claude_repos::list(&conn)?
        .into_iter()
        .map(|m| view_for(m, host.as_deref()))
        .collect())
}

#[tauri::command]
pub async fn set_claude_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    key: RepoKey,
    repo_path: String,
) -> Result<ClaudeRepoView, AppError> {
    let host = claude_handoff::herdr_host(&app)?;
    let conn = state.db.safe_lock();
    Ok(view_for(db::claude_repos::set(&conn, &key, &repo_path)?, host.as_deref()))
}

#[tauri::command]
pub async fn clear_claude_repo(
    state: State<'_, AppState>,
    key: RepoKey,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::claude_repos::clear(&conn, &key)
}
