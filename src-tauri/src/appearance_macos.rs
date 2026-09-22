//! Pin the app's native appearance to match the UI's theme.
//!
//! # Why this is needed
//!
//! **Native** chrome — `NSMenu` popups, `<select>` popups, sheets, scrollbars,
//! the title bar — is drawn by AppKit using `NSApp.effectiveAppearance`, which
//! follows the *system* setting. Nothing in the web layer can influence it. So
//! on a Mac in Light mode, CXMail's dark UI produces a light-grey menu erupting
//! out of a dark window.
//!
//! It was load-bearing rather than cosmetic while the window's blur was an
//! `NSVisualEffectMaterial::UnderWindowBackground`, which renders against the
//! *window's* `NSAppearance` — with nothing pinning it, a dark UI on a
//! light-mode Mac tinted its glass light. The blur is now `glass_macos`'s plain
//! CGS Gaussian, which carries no tint and reads no appearance; the material
//! survives only as its fallback (`vibrancy_macos`), where this still matters.
//!
//! The UI theme is user-selectable, so this is a **function of that choice**
//! rather than a constant: whichever way the web layer goes, native chrome must
//! follow it, not the system. Getting this backwards is worse than not having
//! it — a light UI with a dark right-click menu is the same bug inverted.
//!
//! # …and why "System" has to UNPIN rather than pin
//!
//! Under the `system` theme preference the right move is not to pin the
//! resolved value — it is to pin nothing (`nil`). Two reasons, and the second
//! is the load-bearing one:
//!
//! 1. Unpinned, AppKit already follows the system, so native chrome (and the
//!    fallback material, when it is in use) track macOS for free.
//! 2. `NSApp.appearance` is what the WKWebView derives `prefers-color-scheme`
//!    from. Pin it and the web layer reads back whatever *we* set, so the
//!    frontend's `matchMedia` listener would never fire again and the window
//!    would sit at whatever it was at launch until relaunched. The native and
//!    web halves therefore cannot be changed independently.
//!
//! Note this is also exactly what `tauri.conf.json`'s window `"theme": "Dark"`
//! does — tao's `set_ns_theme` sets `NSApp.appearance` and nothing on the
//! window itself (tao 0.35 `platform_impl/macos/window.rs`). So passing `None`
//! here undoes the configured boot pin too; there is no second window-level
//! appearance that also needs clearing.
//!
//! **Main thread only.** AppKit setters like `-[NSApplication setAppearance:]`
//! are main-thread APIs (gotcha #20). Tauri's `.setup()` runs on the main
//! thread inside `did_finish_launching`, so calling it there is fine. Calling
//! it from a `#[tauri::command]` is NOT — those run on Tokio workers, so the
//! command hops across with `AppHandle::run_on_main_thread`.
//!
//! Ported from cxtasks.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

/// Pin the app to the dark or light system appearance, or (`None`) stop
/// pinning it and follow the system again.
///
/// Safe to call before any window exists; AppKit applies the appearance to
/// windows created later. Must be called on the main thread.
pub fn set_appearance(dark: Option<bool>) {
    // Guard the same way `notify_macos` does: an unbundled process still has a
    // valid NSApplication under Tauri, so this is safe either way, but keeping
    // the ObjC surface small is worth it — an uncaught ObjC exception aborts
    // the process rather than returning an error.
    unsafe {
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if ns_app.is_null() {
            log::warn!("appearance: NSApplication unavailable; leaving system appearance");
            return;
        }
        // `nil` is a meaningful argument here, not a failure: it clears the
        // override and hands the app back to the system appearance.
        let appearance: *mut AnyObject = match dark {
            None => std::ptr::null_mut(),
            Some(dark) => {
                // The two documented `NSAppearanceName` constants. Passing an
                // unknown name to `appearanceNamed:` returns nil rather than
                // raising, which the null check below turns into a warning
                // instead of a process abort.
                let name = if dark {
                    "NSAppearanceNameDarkAqua"
                } else {
                    "NSAppearanceNameAqua"
                };
                let ns_name = NSString::from_str(name);
                let named: *mut AnyObject =
                    msg_send![class!(NSAppearance), appearanceNamed: &*ns_name];
                if named.is_null() {
                    log::warn!("appearance: {name} not found");
                    return;
                }
                named
            }
        };
        let _: () = msg_send![ns_app, setAppearance: appearance];

        // Read it back so the log says what actually took effect rather than
        // what we asked for — and under `None` that is the only way to know
        // which way the system actually resolved.
        let effective: *mut AnyObject = msg_send![ns_app, effectiveAppearance];
        if !effective.is_null() {
            let n: Retained<NSString> = msg_send![effective, name];
            log::info!("appearance: native chrome pinned to {n}");
        }
    }
}
