//! Creating Keychain items that **every** CXMail binary can read.
//!
//! # Why this module exists (gotcha #31)
//!
//! A prompt-free Keychain read needs THREE things to line up, not two:
//!
//! 1. the reading binary is signed with `TeamIdentifier=CCYV5HQZCM`;
//! 2. the item's **partition list** contains `teamid:CCYV5HQZCM`;
//! 3. the item's **ACL trusted-application list** names that binary.
//!
//! `security set-generic-password-partition-list` writes (2). The prompt is
//! gated by (3) — ACL `entry 1`, `authorizations: decrypt …` — and **no
//! `security` subcommand can write it**. `SecKeychainAddGenericPassword`, which
//! is what `keyring` calls, sets that list to *the creating application alone*.
//! So an item minted by `cxmail` is unreadable by `cxmail-helper` without a
//! prompt, forever, no matter how correctly everything is signed.
//!
//! The only moment the list can be set without an authorization prompt is at
//! **creation**, via `SecKeychainItemCreateFromContent`'s `initialAccess`
//! parameter. Changing it afterwards (`SecKeychainItemSetAccess`) needs the
//! `change_acl` authorization, whose trusted-app list is empty on every item we
//! have — that always prompts for the login password.
//!
//! `security-framework` 2.11.1 declares the `SecAccess` *type* and nothing else
//! — no `SecAccessCreate`, no `SecTrustedApplication`, no
//! `SecKeychainItemCreateFromContent` — so the four calls are declared here.
//! They are deprecated (10.10) but present and functional; every failure path
//! degrades to the plain `keyring` create, i.e. today's behaviour.

use std::path::PathBuf;

/// Keychain service name shared by every CXMail credential.
pub const SERVICE_NAME: &str = "cxmail";

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_void, CString};
    use std::os::raw::c_char;
    use std::path::PathBuf;

    type OSStatus = i32;
    type CFTypeRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;
    type SecAccessRef = *mut c_void;
    type SecTrustedApplicationRef = *mut c_void;
    type SecKeychainItemRef = *mut c_void;
    type SecKeychainRef = *mut c_void;

    #[repr(C)]
    struct SecKeychainAttribute {
        tag: u32,
        length: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct SecKeychainAttributeList {
        count: u32,
        attr: *mut SecKeychainAttribute,
    }

    // FourCharCodes from <Security/SecKeychainItem.h> / <Security/SecKeychain.h>.
    const K_SEC_GENERIC_PASSWORD_ITEM_CLASS: u32 = 0x6765_6e70; // 'genp'
    const K_SEC_SERVICE_ITEM_ATTR: u32 = 0x7376_6365; // 'svce'
    const K_SEC_ACCOUNT_ITEM_ATTR: u32 = 0x6163_6374; // 'acct'
    const K_SEC_LABEL_ITEM_ATTR: u32 = 0x6c61_626c; // 'labl'
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    /// `errSecDuplicateItem` — the item already exists, so there is nothing to
    /// create and the caller should modify in place instead.
    pub const ERR_SEC_DUPLICATE_ITEM: OSStatus = -25299;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFAllocatorDefault: CFAllocatorRef;
        // Declared as an opaque symbol: we only ever pass its address.
        static kCFTypeArrayCallBacks: c_void;
        fn CFArrayCreate(
            allocator: CFAllocatorRef,
            values: *const CFTypeRef,
            num_values: CFIndex,
            callbacks: *const c_void,
        ) -> CFArrayRef;
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTrustedApplicationCreateFromPath(
            path: *const c_char,
            app: *mut SecTrustedApplicationRef,
        ) -> OSStatus;
        fn SecAccessCreate(
            descriptor: CFStringRef,
            trusted_list: CFArrayRef,
            access_ref: *mut SecAccessRef,
        ) -> OSStatus;
        fn SecKeychainItemCreateFromContent(
            item_class: u32,
            attr_list: *mut SecKeychainAttributeList,
            length: u32,
            data: *const c_void,
            keychain_ref: SecKeychainRef,
            initial_access: SecAccessRef,
            item_ref: *mut SecKeychainItemRef,
        ) -> OSStatus;
    }

    /// RAII guard so every early return releases its CF object exactly once.
    struct CfOwned(CFTypeRef);
    impl Drop for CfOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// Create the item with an ACL naming `trusted`, returning the raw
    /// `OSStatus` on failure so the caller can special-case duplicates.
    pub fn create_with_access(
        service: &str,
        account: &str,
        secret: &str,
        trusted: &[PathBuf],
    ) -> Result<(), (OSStatus, &'static str)> {
        if trusted.is_empty() {
            return Err((0, "no trusted binary paths resolved"));
        }

        unsafe {
            // 1. One SecTrustedApplication per binary. For Developer ID-signed
            //    code this stores the *designated requirement*
            //    (`identifier "x" and … certificate leaf[subject.OU] = TEAMID`),
            //    not a path or cdhash — so it keeps matching across rebuilds and
            //    a binary that also exists outside the bundle. That is why the
            //    loose `target/release/cxmail-mcp` needs no entry of its own:
            //    it shares `identifier "cxmail-mcp"` with the bundled copy.
            let mut apps: Vec<CfOwned> = Vec::with_capacity(trusted.len());
            for path in trusted {
                let Some(path_str) = path.to_str() else { continue };
                let Ok(c_path) = CString::new(path_str) else {
                    continue;
                };
                let mut app: SecTrustedApplicationRef = std::ptr::null_mut();
                let status = SecTrustedApplicationCreateFromPath(c_path.as_ptr(), &mut app);
                if status == 0 && !app.is_null() {
                    apps.push(CfOwned(app.cast_const()));
                } else {
                    // A missing sibling binary must not sink the whole write.
                    log::warn!("keychain ACL: skipping {path_str} (OSStatus {status})");
                }
            }
            if apps.is_empty() {
                return Err((0, "no trusted applications could be created"));
            }

            let refs: Vec<CFTypeRef> = apps.iter().map(|a| a.0).collect();
            let array = CFArrayCreate(
                kCFAllocatorDefault,
                refs.as_ptr(),
                refs.len() as CFIndex,
                std::ptr::addr_of!(kCFTypeArrayCallBacks).cast(),
            );
            if array.is_null() {
                return Err((0, "CFArrayCreate returned null"));
            }
            let _array = CfOwned(array);

            // 2. The descriptor is the name shown in any future auth dialog.
            let Ok(c_desc) = CString::new(service) else {
                return Err((0, "service name is not a valid C string"));
            };
            let descriptor = CFStringCreateWithCString(
                kCFAllocatorDefault,
                c_desc.as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            );
            if descriptor.is_null() {
                return Err((0, "CFStringCreateWithCString returned null"));
            }
            let _descriptor = CfOwned(descriptor);

            let mut access: SecAccessRef = std::ptr::null_mut();
            let status = SecAccessCreate(descriptor, array, &mut access);
            if status != 0 || access.is_null() {
                return Err((status, "SecAccessCreate"));
            }
            let _access = CfOwned(access.cast_const());

            // 3. Attributes. `labl` matches what SecKeychainAddGenericPassword
            //    sets today, so repaired items keep displaying as "cxmail".
            let mut svce = service.as_bytes().to_vec();
            let mut acct = account.as_bytes().to_vec();
            let mut labl = service.as_bytes().to_vec();
            let mut attrs = [
                SecKeychainAttribute {
                    tag: K_SEC_SERVICE_ITEM_ATTR,
                    length: svce.len() as u32,
                    data: svce.as_mut_ptr().cast(),
                },
                SecKeychainAttribute {
                    tag: K_SEC_ACCOUNT_ITEM_ATTR,
                    length: acct.len() as u32,
                    data: acct.as_mut_ptr().cast(),
                },
                SecKeychainAttribute {
                    tag: K_SEC_LABEL_ITEM_ATTR,
                    length: labl.len() as u32,
                    data: labl.as_mut_ptr().cast(),
                },
            ];
            let mut list = SecKeychainAttributeList {
                count: attrs.len() as u32,
                attr: attrs.as_mut_ptr(),
            };

            let secret_bytes = secret.as_bytes();
            let mut item: SecKeychainItemRef = std::ptr::null_mut();
            let status = SecKeychainItemCreateFromContent(
                K_SEC_GENERIC_PASSWORD_ITEM_CLASS,
                &mut list,
                secret_bytes.len() as u32,
                secret_bytes.as_ptr().cast(),
                std::ptr::null_mut(), // default keychain
                access,
                &mut item,
            );
            if !item.is_null() {
                CFRelease(item.cast_const());
            }
            if status != 0 {
                return Err((status, "SecKeychainItemCreateFromContent"));
            }
            Ok(())
        }
    }
}

/// Absolute paths whose code signature should be trusted on every credential
/// this app creates: the `.app` bundle plus each sibling binary inside it.
///
/// Returns an empty vec when the running executable is not inside a bundle —
/// unsigned local builds (`cargo`, `tauri dev`, the loose `cxmail-mcp`) must
/// never mint items, because an ad-hoc signature yields a `cdhash`-pinned ACL
/// entry that dies on the next rebuild. Those processes use the file store as
/// primary anyway (see [`crate::keychain`]).
pub fn trusted_binary_paths() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    // …/cxmail.app/Contents/MacOS/<bin>  ->  macos_dir, contents, bundle
    let Some(macos_dir) = exe.parent() else {
        return Vec::new();
    };
    let Some(bundle) = macos_dir.parent().and_then(|c| c.parent()) else {
        return Vec::new();
    };
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return Vec::new();
    }

    let mut paths = vec![bundle.to_path_buf()];
    for bin in ["cxmail-helper", "cxmail-mcp", "cxmail-tracker"] {
        let p = macos_dir.join(bin);
        if p.exists() {
            paths.push(p);
        }
    }
    paths
}

/// Create a generic-password item whose ACL trusts every CXMail binary.
///
/// `Ok(false)` means the item already existed and nothing was written — the
/// caller should modify it in place (which preserves its ACL). `Err` means the
/// caller should fall back to the plain `keyring` create.
#[cfg(target_os = "macos")]
pub fn create_with_shared_access(account_key: &str, value: &str) -> Result<bool, String> {
    let trusted = trusted_binary_paths();
    match imp::create_with_access(SERVICE_NAME, account_key, value, &trusted) {
        Ok(()) => Ok(true),
        Err((status, _)) if status == imp::ERR_SEC_DUPLICATE_ITEM => Ok(false),
        Err((status, what)) => Err(format!("{what} failed (OSStatus {status})")),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn create_with_shared_access(_account_key: &str, _value: &str) -> Result<bool, String> {
    Err("shared-access Keychain items are macOS-only".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_paths_empty_outside_a_bundle() {
        // The test harness runs from target/debug/deps, which is not a bundle.
        // This is the guard that stops unsigned builds minting cdhash-pinned
        // ACL entries.
        assert!(trusted_binary_paths().is_empty());
    }

    /// End-to-end proof of the FFI against a **throwaway service name**, so the
    /// one-time repair is never the first time `SecAccessCreate` +
    /// `SecKeychainItemCreateFromContent` run on this machine.
    ///
    /// `#[ignore]` because it writes to the real login Keychain. Run it
    /// deliberately:
    ///
    /// ```text
    /// cargo test --lib keychain::macos_acl -- --ignored --nocapture
    /// ```
    ///
    /// `/usr/bin/security` is in the trusted list **for this probe item only**
    /// — that is what lets the assertions read it back without a GUI prompt,
    /// and it is precisely what real credentials must not grant.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn probe_item_acl_names_every_trusted_binary() {
        use std::process::Command;

        const SVC: &str = "cxmail-acl-selftest";
        const ACCT: &str = "probe";
        const SECRET: &str = "probe-value-do-not-keep";

        let cleanup = || {
            let _ = Command::new("/usr/bin/security")
                .args(["delete-generic-password", "-s", SVC, "-a", ACCT])
                .output();
        };
        cleanup(); // in case a previous run aborted

        let bundle = PathBuf::from("/Applications/cxmail.app");
        assert!(bundle.exists(), "install the app before running this probe");
        let macos_dir = bundle.join("Contents/MacOS");
        let mut trusted = vec![bundle.clone()];
        for bin in ["cxmail-helper", "cxmail-mcp", "cxmail-tracker"] {
            trusted.push(macos_dir.join(bin));
        }
        trusted.push(PathBuf::from("/usr/bin/security"));

        let created = super::imp::create_with_access(SVC, ACCT, SECRET, &trusted);
        assert!(created.is_ok(), "create failed: {created:?}");

        // Reading the DATA back through /usr/bin/security proves SecAccessCreate
        // actually installed our trusted list — an unmodified default ACL would
        // trust only this test binary and force a prompt here.
        let out = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SVC, "-a", ACCT, "-w"])
            .output()
            .expect("security find-generic-password");
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if got != SECRET {
            cleanup();
            panic!("readback mismatch: {got:?} (stderr: {})", String::from_utf8_lossy(&out.stderr));
        }

        cleanup();
    }
}
