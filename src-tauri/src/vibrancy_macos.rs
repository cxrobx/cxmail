//! The FALLBACK blur, for a macOS where the real one is missing.
//!
//! # Not the primary path any more
//!
//! `glass_macos` is. `NSVisualEffectView` is a *material*, not a blur — Apple's
//! radius, Apple's tint and Apple's saturation boost welded into one enum case
//! with no dial on any of it — and in dark mode `UnderWindowBackground` reads
//! as flat milky grey that destroys the colour and shape of the wallpaper
//! behind it. `glass_macos` asks the WindowServer for a plain Gaussian blur at
//! a radius we choose instead.
//!
//! This module stays because that blur is a *private* symbol a future macOS may
//! stop exporting. When `glass_macos`'s lookup comes back empty, a material is
//! much better than nothing: at `--alpha-pane: 0.25` a window with no blur at
//! all is illegible, not merely plainer. So this is the safety net, called from
//! `glass_macos::install_fallback_once` and from nowhere else.
//!
//! # Why this can't be done in CSS
//!
//! The obvious approach — `backdrop-filter: blur()` on the app's root element —
//! does nothing useful here, and it fails in a way that looks like a tuning
//! problem rather than a category error. `backdrop-filter` blurs the element's
//! *backdrop root*, which for anything inside the page is the page itself. It
//! has no access to what is behind the **window**. So on a transparent window
//! it blurs a transparent nothing and you get a crisp, unblurred desktop
//! showing through, with the filter correctly applied to zero pixels. Only the
//! window server can blur what is behind a window — which is equally true of
//! `glass_macos`.
//!
//! # The two halves have to agree
//!
//! This supplies the blur; the *tint* on top of it is the frontend's `--alpha-*`
//! tokens (`globals.css`). Either half alone looks broken — blur with no tint is
//! an unreadable smear of wallpaper, tint with no blur is a flat translucent
//! panel with the desktop legible through it.
//!
//! # Main thread
//!
//! `NSVisualEffectView` construction and insertion are AppKit calls, so this is
//! main-thread-only (gotcha #20). Its one caller is reached from `.setup()` or
//! through `commands::glass`, which hops with `run_on_main_thread`.
//!
//! Ported from cxtasks, where the material and state were argued out against a
//! real window; see gotcha #52 for the parts specific to cxmail.

#[cfg(target_os = "macos")]
use tauri::WebviewWindow;

/// Install the material behind the main window's webview.
///
/// **At most once per window.** Every call inserts another effect view, and
/// stacked views compound the tint; `glass_macos::install_fallback_once` owns
/// that guarantee. It is never re-applied on a theme change either:
/// `UnderWindowBackground` derives its light/dark rendering from the window's
/// `NSAppearance`, which `appearance_macos` pins to follow the UI theme, so the
/// theme toggle moves it for free.
///
/// Nor is it removed at zero transparency: `glass_macos` makes the window
/// opaque there and the panes cover it completely, and under Reduce
/// Transparency the material goes opaque on its own.
///
/// Failure is logged, never fatal. A window with no blur is a cosmetic
/// degradation; the app is entirely usable, and on a Mac too old for the
/// material this is the correct outcome rather than a failed launch.
#[cfg(target_os = "macos")]
pub fn apply(window: &WebviewWindow) {
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};

    // `UnderWindowBackground` over `HudWindow`: it is the material AppKit uses
    // for full-window backgrounds, so it picks up the desktop tint the way a
    // native translucent window does. HudWindow is tuned for small floating
    // panels and reads noticeably flatter and darker across a 1500pt window.
    //
    // `Active` rather than `FollowsWindowActiveState`: mail is a window you
    // glance at while working in another one, and the "follows" state drops the
    // blur to flat grey the moment focus leaves. That would make the effect
    // visible almost exclusively when it isn't being looked at.
    //
    // Radius `None` — the NSWindow already clips its content view to the system
    // corner radius. Passing a value here rounds the *effect view* separately,
    // and any mismatch with the system radius shows as a hairline of desktop
    // caught between the two curves at each corner.
    match apply_vibrancy(
        window,
        NSVisualEffectMaterial::UnderWindowBackground,
        Some(NSVisualEffectState::Active),
        None,
    ) {
        Ok(()) => log::info!("vibrancy: UnderWindowBackground applied"),
        Err(e) => log::warn!("vibrancy: not applied ({e}); window stays opaque"),
    }
}
