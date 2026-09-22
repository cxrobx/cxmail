use crate::db;
use crate::error::AppError;
use crate::keychain;
use crate::AppState;
use crate::LockExt;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<db::accounts::Account>, AppError> {
    let conn = state.open_read_conn()?;
    db::accounts::list(&conn)
}

/// Every keychain key an account of this provider can own.
///
/// The keychain has no list-by-prefix API, so removal has to enumerate. Keep
/// this in step with the `store_credential` call sites: `email::oauth2`
/// (`:access` / `:refresh` / `:expires`, plus the `:calendar:*` family),
/// `commands::auth` and `email::imap` (`:password`). A family missing here is
/// a credential that outlives the account silently.
pub(crate) fn credential_keys_for(provider: &str, email: &str) -> Vec<String> {
    match provider {
        "gmail" => {
            let mut keys = vec![
                format!("gmail:{email}:access"),
                format!("gmail:{email}:refresh"),
                format!("gmail:{email}:expires"),
            ];
            // Google Calendar is a second, independent grant on the same
            // account — connected separately, so it is easy to forget here.
            for suffix in ["access", "refresh", "expires", "scopes"] {
                keys.push(format!("gmail:{email}:calendar:{suffix}"));
            }
            keys
        }
        "outlook" => vec![
            format!("outlook:{email}:access"),
            format!("outlook:{email}:refresh"),
            format!("outlook:{email}:expires"),
        ],
        "icloud" => vec![format!("icloud:{email}:password")],
        "imap" => vec![format!("imap:{email}:password")],
        _ => Vec::new(),
    }
}

/// Remove an account and the credentials it owns.
///
/// `folders` / `messages` / etc. cascade off `accounts.id`, but the keychain
/// does not, so this used to orphan up to seven entries per account. That
/// matters more now that generic IMAP exists: `imap:{email}:password` is a
/// plaintext password rather than a revocable token, and remove-and-re-add is
/// the natural "fix my server settings" gesture — so the leak is on the
/// common path, not an edge case.
///
/// Deletion is best-effort and deliberately cannot abort the row delete: a
/// keychain that refuses must not leave the user with an account they cannot
/// remove. Failures are logged.
#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<(), AppError> {
    // Read the identity BEFORE the delete — afterwards there is nothing left
    // to derive the key names from.
    let account = {
        let conn = state.db.safe_lock();
        let account = db::accounts::get_by_id(&conn, &account_id)?;
        db::accounts::delete(&conn, &account_id)?;
        account
    };

    let Some(account) = account else {
        return Ok(());
    };

    let keys = credential_keys_for(&account.provider, &account.email);
    if keys.is_empty() {
        log::warn!(
            "remove_account: no credential key family known for provider {:?} — \
             credentials for {} may be orphaned",
            account.provider,
            account.email
        );
        return Ok(());
    }

    let mut removed = 0usize;
    for key in &keys {
        match keychain::delete_credential(key) {
            Ok(()) => removed += 1,
            Err(e) => log::warn!("remove_account: could not delete {key}: {e}"),
        }
    }
    log::info!(
        "remove_account: removed {} of {} credential keys for {}",
        removed,
        keys.len(),
        account.email
    );

    Ok(())
}

#[tauri::command]
pub async fn reorder_accounts(
    state: State<'_, AppState>,
    account_ids: Vec<String>,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::accounts::update_order(&conn, &account_ids)?;
    Ok(())
}

#[tauri::command]
pub async fn set_account_group(
    state: State<'_, AppState>,
    account_id: String,
    group_name: Option<String>,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::accounts::set_group(&conn, &account_id, group_name.as_deref())?;
    Ok(())
}

#[tauri::command]
pub async fn set_account_notify_enabled(
    state: State<'_, AppState>,
    account_id: String,
    enabled: bool,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::accounts::set_notify_enabled(&conn, &account_id, enabled)?;
    Ok(())
}

/// Hide an account from (or restore it to) every aggregated view. The dock
/// badge follows All Inboxes, and it is only refreshed on new mail or a
/// mark-read — so it is refreshed here too, with the DB guard dropped first,
/// or it would read stale until the next message arrived.
#[tauri::command]
pub async fn set_account_hidden_from_aggregates(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    hidden: bool,
) -> Result<(), AppError> {
    {
        let conn = state.db.safe_lock();
        db::accounts::set_hidden_from_aggregates(&conn, &account_id, hidden)?;
    }
    crate::notify::update_badge(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_account_track_opens_enabled(
    state: State<'_, AppState>,
    account_id: String,
    enabled: bool,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::accounts::set_track_opens_enabled(&conn, &account_id, enabled)?;
    Ok(())
}

#[tauri::command]
pub async fn rename_account(
    state: State<'_, AppState>,
    account_id: String,
    display_name: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    conn.execute(
        "UPDATE accounts SET display_name = ?1 WHERE id = ?2",
        rusqlite::params![display_name, account_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod credential_key_tests {
    use super::credential_keys_for;

    /// The key names must match what `store_credential` actually wrote — a
    /// typo here deletes nothing and leaves the credential behind silently.
    /// These strings are copied from `email::oauth2`, `commands::auth` and
    /// `email::imap::password_key`.
    #[test]
    fn gmail_covers_mail_and_the_separate_calendar_grant() {
        let keys = credential_keys_for("gmail", "chris@gmail.com");
        for expected in [
            "gmail:chris@gmail.com:access",
            "gmail:chris@gmail.com:refresh",
            "gmail:chris@gmail.com:expires",
            // Calendar is a second grant, connected independently — the one
            // most likely to be forgotten.
            "gmail:chris@gmail.com:calendar:access",
            "gmail:chris@gmail.com:calendar:refresh",
            "gmail:chris@gmail.com:calendar:expires",
            "gmail:chris@gmail.com:calendar:scopes",
        ] {
            assert!(keys.iter().any(|k| k == expected), "missing {expected}");
        }
        assert_eq!(keys.len(), 7);
    }

    /// Zoom's Server-to-Server credentials are **account-global**: one Zoom app
    /// backs every meeting CXMail creates, for every mail account. They must
    /// therefore never appear in any provider's key family, or removing one
    /// Gmail account would delete the Zoom registration out from under all the
    /// others — and the symptom would be "Zoom stopped working", days later,
    /// with nothing connecting it to the account that was removed.
    #[test]
    fn no_provider_claims_the_account_global_zoom_credentials() {
        for provider in ["gmail", "outlook", "icloud", "imap", "unknown"] {
            let keys = credential_keys_for(provider, "chris@gmail.com");
            assert!(
                !keys.iter().any(|key| key.starts_with("zoom:")),
                "{provider} must not own zoom: keys"
            );
        }
    }

    #[test]
    fn password_providers_own_exactly_one_key_each() {
        assert_eq!(
            credential_keys_for("icloud", "chris@icloud.com"),
            vec!["icloud:chris@icloud.com:password"]
        );
        // Byte-identical to `imap::password_key("imap", email)`.
        assert_eq!(
            credential_keys_for("imap", "chris@fastmail.com"),
            vec!["imap:chris@fastmail.com:password"]
        );
    }

    #[test]
    fn outlook_has_no_calendar_family() {
        let keys = credential_keys_for("outlook", "chris@outlook.com");
        assert_eq!(keys.len(), 3);
        assert!(!keys.iter().any(|k| k.contains("calendar")));
    }

    /// An unknown provider yields nothing, which the caller logs rather than
    /// silently treating as "cleaned up".
    #[test]
    fn an_unknown_provider_yields_no_keys() {
        assert!(credential_keys_for("jmap", "chris@example.com").is_empty());
    }

    #[test]
    fn generic_imap_keys_match_the_connect_paths_exactly() {
        let email = "chris@example.com";
        assert_eq!(
            credential_keys_for("imap", email)[0],
            crate::email::imap::password_key("imap", email)
        );
    }
}
