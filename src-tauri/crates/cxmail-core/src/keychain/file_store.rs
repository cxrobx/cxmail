//! File-based credential store.
//!
//! Stores credentials in an encrypted JSON file in the app data directory.
//! Avoids macOS Keychain access prompts that occur with unsigned dev builds.
//! For production (code-signed) builds, switch to the `macos` module (keyring crate).

use crate::error::AppError;
use crate::LockExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

static STORE: Mutex<Option<CredentialStore>> = Mutex::new(None);

struct CredentialStore {
    path: PathBuf,
    data: HashMap<String, String>,
}

impl CredentialStore {
    fn load(path: PathBuf) -> Self {
        let data = if path.exists() {
            let raw = std::fs::read(&path).unwrap_or_default();
            // XOR obfuscation with a fixed key (not cryptographic security,
            // but prevents casual reading; same protection level as Keychain
            // for an unsigned app that anyone can attach a debugger to)
            let decrypted = xor_bytes(&raw, KEY);
            serde_json::from_slice(&decrypted).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self { path, data }
    }

    fn save(&self) -> Result<(), AppError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Keychain(format!("Failed to create credential dir: {}", e)))?;
        }
        let json = serde_json::to_vec(&self.data)
            .map_err(|e| AppError::Keychain(format!("Failed to serialize credentials: {}", e)))?;
        let encrypted = xor_bytes(&json, KEY);
        std::fs::write(&self.path, &encrypted)
            .map_err(|e| AppError::Keychain(format!("Failed to write credentials: {}", e)))?;
        Ok(())
    }
}

const KEY: &[u8] = b"cxmail-credential-store-v1-key!!";

fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn credential_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app")
        .join("credentials.dat")
}

fn with_store<F, R>(f: F) -> Result<R, AppError>
where
    F: FnOnce(&mut CredentialStore) -> Result<R, AppError>,
{
    let mut guard = STORE.safe_lock();
    if guard.is_none() {
        *guard = Some(CredentialStore::load(credential_path()));
    }
    f(guard.as_mut().unwrap())
}

/// Store a credential.
pub fn store_credential(account_key: &str, value: &str) -> Result<(), AppError> {
    with_store(|store| {
        store.data.insert(account_key.to_string(), value.to_string());
        store.save()
    })
}

/// Retrieve a credential. Returns None if not found.
pub fn get_credential(account_key: &str) -> Result<Option<String>, AppError> {
    with_store(|store| Ok(store.data.get(account_key).cloned()))
}

/// Delete a credential. Silently succeeds if not found.
pub fn delete_credential(account_key: &str) -> Result<(), AppError> {
    with_store(|store| {
        store.data.remove(account_key);
        store.save()
    })
}

/// Return every stored credential as `(key, value)` pairs. Used by the one-time
/// file-store → Keychain migration in the signed app (see `keychain::mod`).
pub fn all_credentials() -> Result<Vec<(String, String)>, AppError> {
    with_store(|store| {
        Ok(store
            .data
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    })
}
