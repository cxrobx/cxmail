pub mod macos;
pub mod macos_acl;
pub mod file_store;

use crate::error::AppError;
use std::collections::HashSet;

// Credential-store backend is chosen by RUNTIME CAPABILITY, not by
// `debug_assertions`. The macOS Keychain is only reliable for the distributed,
// code-signed `.app` bundle (and its bundled, signed helper). Unsigned local
// binaries — `cxmail-mcp`, `cxmail-helper` dev builds, `cargo`/`tauri dev` —
// cannot access the Keychain without access-denied errors or auth prompts
// (gotcha #6), so the file store is their primary. One code path serves both:
// the signed app WRITES to the Keychain (file-store fallback covers the one-time
// migration window); everything else writes the file store.
//
// READS are ordered per-process to avoid prompts on the hot path but still reach
// everything: the app tries Keychain then file store, local tools try file store
// then Keychain. That last fallback is not optional — without it an unbundled
// binary cannot see any credential the installed app created, which is precisely
// how `cxmail-mcp` ended up reporting "Calendar is not connected" for a calendar
// the app had already synced. See `get_credential`.

/// True when this process is the distributed, code-signed `.app` bundle (or its
/// bundled helper) — i.e. running from inside `…/cxmail.app/Contents/…`. That is
/// the only context where the macOS Keychain is dependable, so it decides which
/// store is *primary*: true → Keychain, false → file store. It no longer means
/// "never touch the Keychain" — reads fall back across the boundary in both
/// directions so neither process is blind to the other's credentials.
fn use_keychain() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(is_bundle_path))
        .unwrap_or(false)
}

/// Pure predicate: does this executable path live inside a macOS `.app` bundle?
/// Extracted from [`use_keychain`] so the store-selection rule is unit-testable
/// without depending on where the test binary happens to run from.
fn is_bundle_path(exe_path: &str) -> bool {
    exe_path.contains(".app/Contents/")
}

/// Retrieve a credential, trying both stores in whichever order avoids a prompt.
///
/// The signed app prefers the Keychain and falls back to the file store on miss
/// or error (the one-time migration window, plus any transient Keychain failure).
///
/// Local tools read the **file store first** and only consult the Keychain for
/// keys it does not have. That ordering is load-bearing in both directions:
///
/// * Reading the file store first means the credentials `cxmail-mcp` already
///   uses every day (mail tokens, present in both stores since the migration)
///   never touch the Keychain, so the common path stays prompt-free.
/// * Falling through to the Keychain at all is what makes a credential written
///   *only* by the installed app reachable from an unbundled binary. Calendar
///   grants are the case that exposed this: the app stored them in the Keychain,
///   the MCP read only the file store, and every calendar tool reported
///   "Calendar is not connected" while the app synced perfectly.
///
/// An unsigned binary may get a Keychain ACL prompt on that fallback. That is
/// acceptable for keys it would otherwise be unable to read at all, and a denial
/// degrades to `None` rather than an error (gotcha #6).
pub fn get_credential(account_key: &str) -> Result<Option<String>, AppError> {
    if use_keychain() {
        match macos::get_credential(account_key) {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {} // not in Keychain yet — fall through to the file store
            Err(e) => log::warn!("keychain get failed for {account_key}, using file store: {e}"),
        }
        return file_store::get_credential(account_key);
    }

    if let Some(value) = file_store::get_credential(account_key)? {
        return Ok(Some(value));
    }
    match macos::get_credential(account_key) {
        Ok(value) => Ok(value),
        Err(e) => {
            log::warn!("keychain fallback failed for {account_key}: {e}");
            Ok(None)
        }
    }
}

/// Store a credential. In the signed app, writes to the Keychain, falling back to
/// the file store only if the Keychain write fails. Local tools write the file
/// store directly.
pub fn store_credential(account_key: &str, value: &str) -> Result<(), AppError> {
    if use_keychain() {
        match macos::store_credential(account_key, value) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("keychain store failed for {account_key}, using file store: {e}"),
        }
    }
    file_store::store_credential(account_key, value)
}

/// Delete a credential from **both** stores, so a token can't linger in one and
/// be resurrected by the other's fallback path in [`get_credential`].
///
/// Best-effort on the Keychain side for unbundled binaries: they may lack the
/// ACL to delete a key the signed app created, and failing the whole call for
/// that would leave the file store untouched too. A stale Keychain entry is
/// logged rather than fatal.
pub fn delete_credential(account_key: &str) -> Result<(), AppError> {
    let file_res = file_store::delete_credential(account_key);
    if use_keychain() {
        return macos::delete_credential(account_key);
    }
    if let Err(e) = macos::delete_credential(account_key) {
        log::warn!("keychain delete skipped for {account_key}: {e}");
    }
    file_res
}

/// One-time migration: copy any credentials present in the file store but missing
/// from the Keychain into the Keychain. Runs only in the signed app (a no-op
/// elsewhere) and is idempotent, so it is safe to call on every launch. The file
/// store is intentionally left intact so local unsigned tools (e.g. `cxmail-mcp`)
/// keep reading it. Returns the number of credentials copied.
pub fn migrate_file_store_to_keychain() -> Result<usize, AppError> {
    if !use_keychain() {
        return Ok(0);
    }
    let file_creds = file_store::all_credentials()?;
    let keychain_has: HashSet<String> = file_creds
        .iter()
        .filter(|(k, _)| matches!(macos::get_credential(k), Ok(Some(_))))
        .map(|(k, _)| k.clone())
        .collect();

    let mut migrated = 0usize;
    for (key, value) in plan_migration(&file_creds, &keychain_has) {
        match macos::store_credential(&key, &value) {
            Ok(()) => migrated += 1,
            Err(e) => log::warn!("keychain migration failed for {key}: {e}"),
        }
    }
    if migrated > 0 {
        log::info!("migrated {migrated} credential(s) from file store to keychain");
    }
    Ok(migrated)
}

/// Marker recording that the one-time ACL repair has run. Deleting this file
/// forces the repair to run again on the next launch — that is the supported
/// way to retry after a failure.
const ACL_REPAIR_MARKER: &str = "keychain-acl-repair-v1.done";

fn acl_repair_marker_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.cxmail.app")
        .join(ACL_REPAIR_MARKER)
}

/// One-time repair of existing Keychain items so every CXMail binary can read
/// them without an authorization prompt (gotcha #31).
///
/// Runs **only in the signed app bundle**, which is the one process already
/// named in every item's ACL — so it reads each secret prompt-free, something
/// no shell script can do (`/usr/bin/security` is untrusted on nearly all of
/// them, so `find-generic-password -w` would fire one GUI dialog per item).
///
/// Delete-then-recreate is not a stylistic choice: the trusted-application list
/// can only be set through `SecKeychainItemCreateFromContent`'s `initialAccess`.
/// `SecKeychainItemSetAccess` needs the `change_acl` authorization, whose
/// trusted list is empty on every item we hold, so it always prompts for the
/// login password.
///
/// The delete→recreate window is the only point where a credential could go
/// missing. The value is held in memory across it and written back with the old
/// restrictive ACL if the recreate fails, so a failure costs prompts, not a
/// re-authentication.
pub fn repair_keychain_acls() -> Result<usize, AppError> {
    if !use_keychain() {
        return Ok(0);
    }
    // An unsigned or non-bundled build would mint cdhash-pinned ACL entries
    // that die on its next rebuild — strictly worse than leaving items alone.
    if macos_acl::trusted_binary_paths().is_empty() {
        return Ok(0);
    }
    let marker = acl_repair_marker_path();
    if marker.exists() {
        return Ok(0);
    }

    let keys = macos::list_account_keys()?;
    if keys.is_empty() {
        // Either a fresh install or an enumeration hiccup. Write no marker, so
        // a real run still happens once there is something to repair.
        return Ok(0);
    }

    let mut repaired = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for key in &keys {
        let value = match macos::get_credential(key) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                log::warn!("keychain ACL repair: cannot read {key}, leaving it alone: {e}");
                failed.push(key.clone());
                continue;
            }
        };

        if let Err(e) = macos::delete_credential(key) {
            log::warn!("keychain ACL repair: delete failed for {key}: {e}");
            failed.push(key.clone());
            continue;
        }

        if let Err(e) = macos_acl::create_with_shared_access(key, &value) {
            log::error!("keychain ACL repair: recreate failed for {key} ({e}); restoring");
            if let Err(e2) = macos::store_credential(key, &value) {
                log::error!(
                    "keychain ACL repair: RESTORE FAILED for {key}: {e2} \
                     — this credential must be re-authenticated"
                );
            }
            failed.push(key.clone());
            continue;
        }

        match macos::get_credential(key) {
            Ok(Some(v)) if v == value => repaired += 1,
            _ => {
                log::error!("keychain ACL repair: readback mismatch for {key}");
                failed.push(key.clone());
            }
        }
    }

    log::info!(
        "keychain ACL repair: {repaired}/{} item(s) now readable by every CXMail binary{}",
        keys.len(),
        if failed.is_empty() {
            String::new()
        } else {
            format!("; {} failed: {}", failed.len(), failed.join(", "))
        }
    );

    // Written even on partial failure: repeating a delete/recreate sweep over
    // live credentials on every launch is worse than a logged, retryable miss.
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = if failed.is_empty() {
        format!("repaired {repaired} item(s)\n")
    } else {
        format!("repaired {repaired} item(s)\nfailed: {}\n", failed.join(", "))
    };
    if let Err(e) = std::fs::write(&marker, body) {
        log::warn!("keychain ACL repair: could not write marker {marker:?}: {e}");
    }

    Ok(repaired)
}

/// Pure decision helper: given every file-store credential and the set of keys
/// already present in the Keychain, return the `(key, value)` pairs that still
/// need copying. Extracted so migration logic is unit-testable without touching
/// the real Keychain or the real `credentials.dat`.
fn plan_migration(
    file_creds: &[(String, String)],
    keychain_has: &HashSet<String>,
) -> Vec<(String, String)> {
    file_creds
        .iter()
        .filter(|(k, _)| !keychain_has.contains(k))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_bundle_path_true_for_signed_app_and_helper() {
        // The distributed, code-signed binaries run from inside the bundle.
        assert!(is_bundle_path("/Applications/cxmail.app/Contents/MacOS/cxmail"));
        assert!(is_bundle_path(
            "/Applications/cxmail.app/Contents/MacOS/cxmail-helper"
        ));
    }

    #[test]
    fn is_bundle_path_false_for_local_unsigned_binaries() {
        // These must use the file store — this is the exact regression that
        // broke the MCP when selection was gated on `debug_assertions`.
        assert!(!is_bundle_path(
            "/Users/x/Projects/cxmail/src-tauri/target/release/cxmail-mcp"
        ));
        assert!(!is_bundle_path(
            "/Users/x/Projects/cxmail/src-tauri/target/debug/deps/cxmail-1a2b3c"
        ));
        assert!(!is_bundle_path("/usr/local/bin/cxmail-mcp"));
    }

    #[test]
    fn plan_migration_copies_only_missing_keys() {
        let file = vec![
            ("gmail:a@x.com:access".to_string(), "tok_a".to_string()),
            ("gmail:a@x.com:refresh".to_string(), "ref_a".to_string()),
            ("gmail:b@x.com:access".to_string(), "tok_b".to_string()),
        ];
        let mut have = HashSet::new();
        have.insert("gmail:a@x.com:access".to_string()); // already in keychain
        let plan = plan_migration(&file, &have);
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().any(|(k, _)| k == "gmail:a@x.com:refresh"));
        assert!(plan.iter().any(|(k, _)| k == "gmail:b@x.com:access"));
        assert!(!plan.iter().any(|(k, _)| k == "gmail:a@x.com:access"));
    }

    #[test]
    fn plan_migration_empty_when_all_present() {
        let file = vec![("k".to_string(), "v".to_string())];
        let mut have = HashSet::new();
        have.insert("k".to_string());
        assert!(plan_migration(&file, &have).is_empty());
    }

    #[test]
    fn plan_migration_copies_all_when_keychain_empty() {
        let file = vec![
            ("k1".to_string(), "v1".to_string()),
            ("k2".to_string(), "v2".to_string()),
        ];
        let plan = plan_migration(&file, &HashSet::new());
        assert_eq!(plan.len(), 2);
    }
}
