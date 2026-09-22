use crate::db;
use crate::email::imap;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<db::folders::FolderRow>, AppError> {
    let conn = state.open_read_conn()?;
    db::folders::list_by_account(&conn, &account_id)
}

#[tauri::command]
pub async fn sync_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<db::folders::FolderRow>, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?
    };

    log::info!("Starting IMAP folder sync for {} ({})", account.email, account.provider);
    let imap_folders = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        async {
            let mut session = imap::connect_for_account(&account).await?;
            log::info!("IMAP connected, listing folders...");
            let folders = imap::list_folders(&mut session, &account.provider).await?;
            log::info!("Got {} folders, disconnecting...", folders.len());
            let _ = imap::disconnect(session, &account.email).await;
            Ok::<Vec<imap::ImapFolder>, AppError>(folders)
        }
    )
    .await
    .map_err(|_| AppError::Imap(format!("Folder sync timed out for {}", account.email)))??;

    {
        let conn = state.db.safe_lock();
        for folder in &imap_folders {
            db::folders::upsert(
                &conn,
                &account_id,
                &folder.name,
                None,
                &folder.folder_type,
                folder.delimiter.as_deref(),
                folder.special_use,
            )?;
        }
    }

    let conn = state.db.safe_lock();
    db::folders::list_by_account(&conn, &account_id)
}

#[tauri::command]
pub async fn create_folder(
    state: State<'_, AppState>,
    account_id: String,
    folder_name: String,
) -> Result<Vec<db::folders::FolderRow>, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    imap::create_folder(&mut session, &folder_name).await?;

    // Re-sync folder list so the new folder appears with correct metadata
    let imap_folders = imap::list_folders(&mut session, &account.provider).await?;
    let _ = imap::disconnect(session, &account.email).await;

    {
        let conn = state.db.safe_lock();
        for folder in &imap_folders {
            db::folders::upsert(
                &conn,
                &account_id,
                &folder.name,
                None,
                &folder.folder_type,
                folder.delimiter.as_deref(),
                folder.special_use,
            )?;
        }
    }

    let conn = state.db.safe_lock();
    db::folders::list_by_account(&conn, &account_id)
}

#[tauri::command]
pub async fn delete_folder(
    state: State<'_, AppState>,
    account_id: String,
    folder_name: String,
) -> Result<Vec<db::folders::FolderRow>, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    imap::delete_folder(&mut session, &folder_name).await?;
    let _ = imap::disconnect(session, &account.email).await;

    // Remove from local DB and return updated list
    {
        let conn = state.db.safe_lock();
        db::folders::delete(&conn, &account_id, &folder_name)?;
    }

    let conn = state.db.safe_lock();
    db::folders::list_by_account(&conn, &account_id)
}

#[tauri::command]
pub async fn rename_folder(
    state: State<'_, AppState>,
    account_id: String,
    old_name: String,
    new_name: String,
) -> Result<Vec<db::folders::FolderRow>, AppError> {
    let account = {
        let conn = state.db.safe_lock();
        db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound(format!("Account {} not found", account_id)))?
    };

    let mut session = imap::connect_for_account(&account).await?;
    imap::rename_folder(&mut session, &old_name, &new_name).await?;

    // Re-sync folder list
    let imap_folders = imap::list_folders(&mut session, &account.provider).await?;
    let _ = imap::disconnect(session, &account.email).await;

    {
        let conn = state.db.safe_lock();
        // Remove old folder entry
        db::folders::delete(&conn, &account_id, &old_name)?;
        for folder in &imap_folders {
            db::folders::upsert(
                &conn,
                &account_id,
                &folder.name,
                None,
                &folder.folder_type,
                folder.delimiter.as_deref(),
                folder.special_use,
            )?;
        }
    }

    let conn = state.db.safe_lock();
    db::folders::list_by_account(&conn, &account_id)
}
