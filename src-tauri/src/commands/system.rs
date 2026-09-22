//! System integration commands — "set CXMail as the default email client".
//!
//! `CFBundleURLTypes` in Info.plist (Part 1 of the mailto plan) only makes
//! CXMail a *candidate* handler for the `mailto:` scheme. Becoming the *default*
//! is a separate Launch Services binding, done here via a minimal CoreServices
//! FFI (no new crate — `notify_macos.rs` is the FFI precedent; `objc2_foundation`
//! gives us toll-free-bridged `NSString` → `CFStringRef`).
//!
//! Contract: `set_as_default_mail_client()` returning `true` means the change was
//! *requested* and Launch Services reported success — NOT that it silently took
//! effect. On macOS 26.4+ the OS may show a confirmation prompt; a sandboxed
//! (Mac App Store) build can't do it at all. This is a non-MAS Tauri app, so it
//! is not sandboxed and the call works; on Sonoma 14.5 it is silent.

const BUNDLE_ID: &str = "com.cxmail.app";

#[cfg(target_os = "macos")]
mod imp {
    use super::BUNDLE_ID;
    use objc2::rc::Retained;
    use objc2_foundation::NSString;

    // CoreFoundation / Launch Services opaque pointer + status types.
    type CFStringRef = *const std::ffi::c_void;
    type OSStatus = i32;

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSSetDefaultHandlerForURLScheme(
            in_url_scheme: CFStringRef,
            in_handler_bundle_id: CFStringRef,
        ) -> OSStatus;
        fn LSCopyDefaultHandlerForURLScheme(in_url_scheme: CFStringRef) -> CFStringRef;
    }

    /// `NSString` is toll-free bridged to `CFString`, so an `NSString` pointer is
    /// a valid `CFStringRef`. Caller must keep `s` alive across the FFI call.
    fn cfstr(s: &NSString) -> CFStringRef {
        (s as *const NSString).cast()
    }

    pub fn set_default() -> bool {
        let scheme = NSString::from_str("mailto");
        let bundle = NSString::from_str(BUNDLE_ID);
        // noErr == 0
        unsafe { LSSetDefaultHandlerForURLScheme(cfstr(&scheme), cfstr(&bundle)) == 0 }
    }

    pub fn is_default() -> bool {
        let scheme = NSString::from_str("mailto");
        let raw = unsafe { LSCopyDefaultHandlerForURLScheme(cfstr(&scheme)) };
        if raw.is_null() {
            return false;
        }
        // LSCopy… returns a +1 reference; `Retained::from_raw` adopts that
        // ownership and releases it on drop (CFString ⇄ NSString toll-free).
        let current = unsafe { Retained::from_raw(raw as *mut NSString) };
        match current {
            Some(s) => s.to_string().eq_ignore_ascii_case(BUNDLE_ID),
            None => false,
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn set_default() -> bool {
        false
    }
    pub fn is_default() -> bool {
        false
    }
}

/// Plain-Rust entry points so the tray menu handler (which isn't a Tauri
/// command) can reuse the same logic as the IPC commands below.
pub fn set_default_native() -> bool {
    imp::set_default()
}

pub fn is_default_native() -> bool {
    imp::is_default()
}

/// Request that CXMail become the default `mailto:` handler. Returns whether
/// Launch Services accepted the request (see the contract note at module top).
#[tauri::command]
pub fn set_as_default_mail_client() -> bool {
    let ok = set_default_native();
    log::info!("set_as_default_mail_client requested → {ok}");
    ok
}

/// Whether CXMail is currently the default `mailto:` handler.
#[tauri::command]
pub fn is_default_mail_client() -> bool {
    is_default_native()
}

// ─── Client-side error sink ───────────────────────────────────────────
//
// Release builds ship without DevTools (gotcha #19), so a `console.warn` in
// the frontend goes somewhere nobody can read — not the user, not us, not a
// support ticket. That is not hypothetical: `src/lib/updater.ts` swallowed
// every failed update check this way, and the only reason the dead updater
// endpoint was ever noticed is that tauri-plugin-updater logs its OWN error on
// the Rust side. The app's handler had been silent for weeks.
//
// Route anything a user would file a bug about through here instead, so it
// lands in ~/Library/Logs/com.cxmail.app/CXMail.log alongside the backend.

/// Record a frontend-side failure in the Rust log.
///
/// `scope` is a short call-site tag (e.g. `"updater"`); `message` is the
/// already-stringified error. Deliberately not `Result` — a logging call must
/// never introduce a new failure path at the site it is meant to make visible.
#[tauri::command]
pub fn log_client_error(scope: String, message: String) {
    // Truncate rather than trust the frontend with the log file's size: this is
    // reachable from the webview and an error string can carry a whole payload.
    const MAX: usize = 2000;
    let mut message = message;
    if message.len() > MAX {
        message.truncate(MAX);
        message.push_str("… (truncated)");
    }
    log::warn!("[client:{scope}] {message}");
}

// ─── Native appearance ────────────────────────────────────────────────
//
// See `appearance_macos` for why this exists at all. The one thing this layer
// adds is the **main-thread hop**: `#[tauri::command]` bodies run on Tokio
// workers, and `-[NSApplication setAppearance:]` is a main-thread-only AppKit
// setter, so calling it directly from here would be the threading violation in
// gotcha #20.

/// Set native chrome to dark or light, or hand it back to the system.
///
/// Called by the frontend whenever the theme changes, and once on mount so a
/// persisted light theme is applied even though `.setup()` pinned the window
/// dark before it could read localStorage.
///
/// `dark: None` is the `system` theme preference: it clears the pin instead of
/// setting one, which is both how native chrome starts following macOS and how
/// the webview regains a truthful `prefers-color-scheme`. See
/// `appearance_macos` — the two cannot be decoupled.
#[tauri::command]
pub async fn set_native_appearance(
    app: tauri::AppHandle,
    dark: Option<bool>,
) -> Result<(), crate::error::AppError> {
    #[cfg(target_os = "macos")]
    {
        // `run_on_main_thread` returns as soon as the closure is *queued*, not
        // when it completes. That is fine here: a fire-and-forget visual change
        // with no return value, and the frontend has already repainted.
        app.run_on_main_thread(move || {
            crate::appearance_macos::set_appearance(dark);
        })
        .map_err(|e| {
            crate::error::AppError::General(format!(
                "could not reach the main thread to set appearance: {e}"
            ))
        })?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, dark);
    }

    Ok(())
}
