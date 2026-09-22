//! Enable native macOS continuous spell checking (the red squiggles) in the
//! WKWebView that hosts the app.
//!
//! Setting the HTML `spellcheck="true"` attribute on the compose editor and the
//! subject `<input>` is NOT sufficient in WKWebView: continuous spell checking
//! defaults to off, and until it's turned on natively, spelling is only checked
//! via the right-click context menu — never as as-you-type underlines.
//!
//! Two mechanisms, applied together because neither is guaranteed alone:
//!
//! 1. **`WebContinuousSpellCheckingEnabled` user default** (the reliable one).
//!    WebKit2's macOS `TextChecker` seeds its state from this `NSUserDefaults`
//!    key — it's how Safari persists "Check Spelling While Typing". Because the
//!    checker state is initialized lazily (first time an editable field is
//!    focused), writing the default during app setup lands before it's read.
//!
//! 2. **`_setContinuousSpellCheckingEnabled:` on the WKWebView** (WebKit SPI).
//!    Applies to the already-live view. Note the plain AppKit selector
//!    `setContinuousSpellCheckingEnabled:` (no underscore) is an `NSTextView`
//!    API that WKWebView does **not** implement — verified at runtime, it logged
//!    "does not respond". Every selector here is sent only behind a
//!    `respondsToSelector:` check, so a WebKit build lacking the SPI degrades to
//!    mechanism 1 instead of crashing on an unrecognized selector.
//!
//! The `with_webview` closure runs on the main thread, which AppKit requires
//! (gotcha #20).

use objc2::runtime::AnyObject;
use objc2::{class, msg_send, sel};
use objc2_foundation::NSString;
use tauri::{AppHandle, Manager};

/// Write the WebKit user defaults that turn on continuous spell + grammar
/// checking for every web view in this process.
fn set_webkit_defaults() {
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        if defaults.is_null() {
            log::warn!("spellcheck_macos: standardUserDefaults was null");
            return;
        }
        for key in [
            "WebContinuousSpellCheckingEnabled",
            "WebGrammarCheckingEnabled",
        ] {
            let ns_key = NSString::from_str(key);
            let _: () = msg_send![defaults, setBool: true, forKey: &*ns_key];
        }
        log::info!("spellcheck_macos: WebKit spell/grammar checking user defaults set");
    }
}

/// Turn on continuous spell + grammar checking for the main window's WKWebView.
/// Best-effort: logs and returns on any missing piece, never panics.
pub fn enable(app: &AppHandle) {
    // Mechanism 1 — independent of the webview, so do it unconditionally first.
    set_webkit_defaults();

    // Mechanism 2 — nudge the already-constructed view via WebKit SPI.
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("spellcheck_macos: no main webview window; relying on user defaults only");
        return;
    };

    let result = window.with_webview(|webview| {
        let wk = webview.inner() as *mut AnyObject;
        if wk.is_null() {
            log::warn!("spellcheck_macos: WKWebView pointer was null");
            return;
        }
        unsafe {
            let responds: bool =
                msg_send![wk, respondsToSelector: sel!(_setContinuousSpellCheckingEnabled:)];
            if responds {
                let _: () = msg_send![wk, _setContinuousSpellCheckingEnabled: true];
                log::info!("spellcheck_macos: continuous spell checking enabled via WebKit SPI");
            } else {
                log::info!(
                    "spellcheck_macos: WKWebView lacks _setContinuousSpellCheckingEnabled: — \
                     relying on the WebContinuousSpellCheckingEnabled user default"
                );
            }

            let responds_grammar: bool =
                msg_send![wk, respondsToSelector: sel!(_setGrammarCheckingEnabled:)];
            if responds_grammar {
                let _: () = msg_send![wk, _setGrammarCheckingEnabled: true];
            }
        }
    });

    if let Err(e) = result {
        log::warn!("spellcheck_macos: with_webview failed: {e}");
    }
}
