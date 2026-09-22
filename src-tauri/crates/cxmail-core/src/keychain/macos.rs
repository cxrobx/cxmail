use crate::error::AppError;
use keyring::Entry;

const SERVICE_NAME: &str = "cxmail";

/// Store a credential in macOS Keychain.
///
/// The account key should follow the pattern: "{provider}:{email}:{type}"
/// e.g., "gmail:user@gmail.com:access" or "gmail:user@gmail.com:refresh"
///
/// Two distinct paths, and the split is the whole point (gotcha #31):
///
/// * **Existing item** — `keyring`'s `set_password` is find-then-modify
///   (`SecKeychainItemModifyAttributesAndData`), which rewrites the data and
///   leaves the ACL and partition list untouched. Hourly token refreshes take
///   this path, which is why they never re-break access.
/// * **New item** — `keyring` would call `SecKeychainAddGenericPassword`, whose
///   default ACL trusts *only the creating binary*. That is what left
///   `cxmail-helper` prompting on items the app minted. Creating through
///   [`macos_acl`] instead names every CXMail binary in the ACL up front.
///
/// Any failure in the shared-access path falls back to the plain create, so the
/// worst case is exactly the old behaviour rather than a lost credential.
pub fn store_credential(account_key: &str, value: &str) -> Result<(), AppError> {
    // A miss here is the common case for a brand-new credential and costs one
    // prompt-free lookup. An *error* (rather than `None`) means we could not
    // determine existence — fall through to the create, which reports
    // errSecDuplicateItem as `Ok(false)` if the item was there after all.
    let exists = matches!(get_credential(account_key), Ok(Some(_)));

    if !exists {
        match super::macos_acl::create_with_shared_access(account_key, value) {
            Ok(true) => return Ok(()),
            Ok(false) => {} // already existed after all — modify in place below
            Err(e) => {
                log::warn!(
                    "keychain: shared-access create failed for {account_key} ({e}); \
                     falling back to a single-binary item"
                );
            }
        }
    }

    set_via_keyring(account_key, value)
}

/// Plain `keyring` write: modifies an existing item in place, or creates one
/// with the restrictive default ACL.
fn set_via_keyring(account_key: &str, value: &str) -> Result<(), AppError> {
    let entry = Entry::new(SERVICE_NAME, account_key)
        .map_err(|e| AppError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    entry
        .set_password(value)
        .map_err(|e| AppError::Keychain(format!("Failed to store credential: {}", e)))?;
    Ok(())
}

/// Retrieve a credential from macOS Keychain.
///
/// Returns None if the credential is not found (rather than an error).
pub fn get_credential(account_key: &str) -> Result<Option<String>, AppError> {
    let entry = Entry::new(SERVICE_NAME, account_key)
        .map_err(|e| AppError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Keychain(format!(
            "Failed to retrieve credential: {}",
            e
        ))),
    }
}

/// Every account name stored under our service.
///
/// Reads item **attributes only** — never the encrypted data — so it needs no
/// ACL authorization and cannot raise a prompt. That distinction matters:
/// `security dump-keychain -a`, the obvious shell equivalent, *does* prompt
/// (it reads ACLs), hangs on the first dialog, and silently truncates its
/// output. Do not reach for it as a substitute.
#[cfg(target_os = "macos")]
pub fn list_account_keys() -> Result<Vec<String>, AppError> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

    let results = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE_NAME)
        .limit(Limit::All)
        .load_attributes(true)
        .search()
        .map_err(|e| AppError::Keychain(format!("Failed to list keychain items: {e}")))?;

    let mut keys: Vec<String> = results
        .iter()
        .filter_map(|r| r.simplify_dict())
        .filter_map(|d| d.get("acct").cloned())
        .filter(|a| !a.is_empty())
        .collect();
    keys.sort();
    keys.dedup();
    Ok(keys)
}

#[cfg(not(target_os = "macos"))]
pub fn list_account_keys() -> Result<Vec<String>, AppError> {
    Ok(Vec::new())
}

/// Delete a credential from macOS Keychain.
///
/// Silently succeeds if the credential doesn't exist.
pub fn delete_credential(account_key: &str) -> Result<(), AppError> {
    let entry = Entry::new(SERVICE_NAME, account_key)
        .map_err(|e| AppError::Keychain(format!("Failed to create keyring entry: {}", e)))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Keychain(format!(
            "Failed to delete credential: {}",
            e
        ))),
    }
}
