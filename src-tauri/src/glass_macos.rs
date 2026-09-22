//! Blur the desktop behind the window with a radius we choose.
//!
//! # Why this replaced `vibrancy_macos` as the blur
//!
//! `NSVisualEffectView` is not a blur. It is a **material**: a blur radius
//! Apple picked, plus their tint, plus a saturation boost, welded together and
//! exposed as one enum case. You cannot dial any of it. In dark mode
//! `UnderWindowBackground` renders as a flat milky grey that eats both the
//! colour and the shape of whatever is behind the window, so the wallpaper
//! stops reading as a wallpaper and becomes generic smoke. The transparency
//! slider could never recover it, because the slider only moves the tint
//! painted *on top* (`--alpha-*`); the smear underneath was fixed.
//!
//! `CGSSetWindowBackgroundBlurRadius` is the other thing: a plain Gaussian
//! blur of the desktop behind the window, at a radius we pass in, with no
//! material on top at all. The wallpaper keeps its hue and its large shapes,
//! and the pane tokens are then the only thing colouring it. That is the
//! difference between "translucent grey panel" and glass.
//!
//! Two routes that look like they would get there, and don't. Choosing a
//! different `NSVisualEffectMaterial` case: every case bakes in its own tint
//! and radius, and none of them is a plain blur (cxtasks walked all of them).
//! And `backdrop-filter`, which cannot see past the window at all — it blurs
//! the page's backdrop root, not the desktop (gotcha #52).
//!
//! # The catch, stated plainly
//!
//! It is a private WindowServer symbol. It is not in any SDK, Apple never
//! promised it, and it can disappear in an OS update. Two things make that an
//! acceptable trade here rather than a landmine:
//!
//! 1. It is resolved with `dlsym` at runtime, never linked. A missing symbol is
//!    an `Option::None` we can branch on, not a launch-time dyld abort.
//! 2. When it is missing we fall back to `vibrancy_macos` — the material this
//!    app shipped until now. The degradation is "the window looks like last
//!    month", a cosmetic regression nobody loses mail to.
//!
//! The one door this closes is the Mac App Store, and CXMail closed it long
//! ago: it drives Ghostty over Apple Events, installs a LaunchAgent helper, and
//! spawns `agy` as a subprocess. It ships Developer ID direct, and notarization
//! does not inspect for private API use.
//!
//! # Reduce Transparency is ours to honour now
//!
//! `NSVisualEffectView` went opaque on its own when the user turned on
//! **System Settings → Accessibility → Display → Reduce transparency**. A raw
//! CGS blur does not — nothing in the WindowServer path reads that setting — so
//! switching blur source silently drops an accessibility behaviour we used to
//! get for free. `reduce_transparency` reads it and `set_state` refuses glass
//! while it is on, whatever the slider says.
//!
//! It is read live rather than cached at launch, and observed, because it is a
//! toggle someone flips *because they are struggling right now*. A setting that
//! needs an app restart to take effect is one they will conclude is broken.
//!
//! # Main thread
//!
//! Every AppKit call here is main-thread-only (gotcha #20). `.setup()` already
//! runs there; `commands::glass` hops with `run_on_main_thread`.
//!
//! Ported from cxtasks (767a8fb). The launch-flash and corner traps are gotcha
//! #59; CXMail's own radius curve, which is NOT cxtasks', is in `uiStore.ts`.

#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use tauri::{Emitter, Manager, WebviewWindow};

/// The radius range this module will hand the WindowServer.
///
/// A sanity bound, not the design range — the frontend's curve
/// (`transparencyToBlurRadius`) sits well inside it. It exists so a corrupt
/// value that got past the frontend still cannot ask the WindowServer for
/// something absurd. The floor is not 0: 0 means "no blur", and the way to get
/// no blur is to not enable glass, which is a different code path with a
/// different window background.
pub const BLUR_MIN: u8 = 4;
pub const BLUR_MAX: u8 = 64;

/// `dlsym` handle meaning "search every loaded image", which is where the
/// WindowServer symbols live once AppKit has pulled SkyLight in.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

type CgsConnection = usize;
type SetBlurFn = unsafe extern "C" fn(CgsConnection, c_int, c_int) -> c_int;
type ConnectionFn = unsafe extern "C" fn() -> CgsConnection;

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Is the real blur available on this machine?
pub fn is_available() -> bool {
    set_blur_fn().is_some() && cgs_connection().is_some()
}

/// What the frontend last asked for, so a Reduce-Transparency flip can be
/// honoured without a round trip to the webview.
///
/// The alternative — emitting an event and waiting for the frontend to call
/// back — makes an accessibility setting depend on the webview being
/// responsive, which is exactly the moment it might not be.
#[derive(Clone, Copy)]
struct Desired {
    enabled: bool,
    radius: u8,
    base: (u8, u8, u8),
}

static DESIRED: Mutex<Option<Desired>> = Mutex::new(None);

/// Is **Accessibility → Display → Reduce transparency** on?
///
/// Read fresh every time. It is one `objc_msgSend` against a cached workspace
/// singleton, and caching it is how you ship the bug where the setting only
/// applies after a relaunch. Main thread only, like everything here.
pub fn reduce_transparency() -> bool {
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return false;
        }
        msg_send![workspace, accessibilityDisplayShouldReduceTransparency]
    }
}

/// Record what the frontend wants, then apply what the machine allows.
///
/// The split is the point: `DESIRED` is the *preference* and survives a
/// Reduce-Transparency flip in both directions, so turning the setting back off
/// restores the glass the user had rather than dropping them at some default.
pub fn set_state(window: &WebviewWindow, enabled: bool, radius: u8, r: u8, g: u8, b: u8) {
    if let Ok(mut slot) = DESIRED.lock() {
        *slot = Some(Desired { enabled, radius, base: (r, g, b) });
    }
    apply_desired(window);
}

/// Re-apply the stored preference against the current accessibility setting.
/// Called by the observer, and safe before any preference has been stored — a
/// window with nothing requested yet keeps its opaque launch background.
pub fn refresh(window: &WebviewWindow) {
    apply_desired(window);
}

fn apply_desired(window: &WebviewWindow) {
    let Some(desired) = DESIRED.lock().ok().and_then(|slot| *slot) else {
        return;
    };
    let (r, g, b) = desired.base;
    if desired.enabled && !reduce_transparency() {
        enable(window, desired.radius);
    } else {
        disable(window, r, g, b);
    }
}

/// Make the window a transparent pane and turn the blur on.
///
/// Deliberately **not** called from `.setup()`. Between window creation and the
/// frontend's first paint there is nothing to look at but the desktop, and a
/// window that is already clear during that gap shows bare wallpaper with
/// floating traffic lights (gotcha #59). `lib.rs` leaves the window opaque and
/// the frontend calls this once it has painted — which is also the first moment
/// the saved theme and transparency are known, so the radius is right on the
/// first try rather than snapping a frame later.
fn enable(window: &WebviewWindow, radius: u8) {
    let Some(ns_window) = ns_window(window) else {
        log::warn!("glass: no NSWindow; leaving the window opaque");
        return;
    };

    unsafe {
        let _: () = msg_send![ns_window, setOpaque: false];

        // Alpha 0.01, not 0. A window whose background is *fully* clear and
        // which also draws a shadow makes AppKit chamfer the corners — a
        // hairline notch of desktop cut across each rounded corner where the
        // shadow's mask and the clear background disagree. One hundredth of
        // black is invisible and stops it.
        let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
        let nearly_clear: *mut AnyObject = msg_send![clear, colorWithAlphaComponent: 0.01f64];
        let _: () = msg_send![ns_window, setBackgroundColor: nearly_clear];

        // Re-assert the shadow after changing opacity. AppKit caches the shadow
        // path against the window's opaque-ness, so without the invalidate the
        // window keeps the *opaque* shadow it computed at creation and the
        // corners draw a second, misaligned edge.
        let _: () = msg_send![ns_window, setHasShadow: true];
        let _: () = msg_send![ns_window, invalidateShadow];
    }

    if is_available() {
        apply_radius(window, radius.clamp(BLUR_MIN, BLUR_MAX));
    } else {
        // The material is strictly worse but it is a blur, and a window with
        // no blur at all at `--alpha-pane: 0.25` is unreadable rather than
        // merely plainer. With the window cleared above, this is exactly the
        // configuration CXMail shipped before the port.
        install_fallback_once(window);
    }
}

/// Install the `NSVisualEffectView` fallback — once, ever.
///
/// `vibrancy_macos::apply` inserts a NEW effect view on every call, and
/// `enable` runs on every theme change and every glass on/off edge. Without
/// this guard a machine missing the CGS symbol would stack one more view per
/// toggle — the exact failure `vibrancy_macos`'s own doc warns about. (cxtasks
/// calls it unguarded; this is a deliberate departure.) It never needs
/// removing: at transparency 0 the window is opaque and the panes cover it,
/// and under Reduce Transparency the material goes opaque by itself.
fn install_fallback_once(window: &WebviewWindow) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    log::warn!("glass: CGS blur unavailable; falling back to NSVisualEffectView");
    crate::vibrancy_macos::apply(window);
}

/// Put the window back to an opaque pane and drop the blur.
///
/// The radius goes to 0 *before* the background goes opaque. The other order
/// leaves one frame where an opaque window still carries a blur region, which
/// the WindowServer paints as a grey halo just outside the frame.
fn disable(window: &WebviewWindow, r: u8, g: u8, b: u8) {
    apply_radius(window, 0);
    set_launch_background(window, r, g, b);
}

/// Paint the window opaque in the given colour, with no blur.
///
/// The launch state, and the state `disable` returns to. The colour comes from
/// the caller because only the frontend knows which theme was saved — `lib.rs`
/// passes dark's `--bg-primary` at launch, and the frontend corrects it.
pub fn set_launch_background(window: &WebviewWindow, r: u8, g: u8, b: u8) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    unsafe {
        let color: *mut AnyObject = msg_send![
            class!(NSColor),
            colorWithRed: f64::from(r) / 255.0,
            green: f64::from(g) / 255.0,
            blue: f64::from(b) / 255.0,
            alpha: 1.0f64,
        ];
        let _: () = msg_send![ns_window, setOpaque: true];
        let _: () = msg_send![ns_window, setBackgroundColor: color];
        // Same reason as in `enable`: the shadow path is cached against the
        // opaque-ness, and this is the other edge of that flip.
        let _: () = msg_send![ns_window, invalidateShadow];
    }
}

/// Change the blur radius on an already-enabled window.
///
/// Separate from `set_state` because the transparency slider drives it
/// continuously and has no business re-running the window setup on every step.
///
/// Updates the stored preference as well as the window, or a
/// Reduce-Transparency flip would restore the radius the slider had several
/// drags ago. Applies nothing while the setting is on: the window is opaque,
/// and writing a blur region behind it is the grey-halo bug `disable` orders
/// its two steps to avoid.
pub fn set_radius(window: &WebviewWindow, radius: u8) {
    let radius = radius.clamp(BLUR_MIN, BLUR_MAX);
    let mut glass_on = false;
    if let Ok(mut slot) = DESIRED.lock() {
        if let Some(desired) = slot.as_mut() {
            desired.radius = radius;
            glass_on = desired.enabled;
        }
    }
    if glass_on && !reduce_transparency() {
        apply_radius(window, radius);
    }
}

/// The raw call. Takes an unclamped radius so `disable` can pass 0 — the one
/// value outside the public range that means something: it removes the blur
/// region entirely rather than shrinking it.
fn apply_radius(window: &WebviewWindow, radius: u8) {
    let (Some(set_blur), Some(connection)) = (set_blur_fn(), cgs_connection()) else {
        return;
    };
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    // The window number is the WindowServer's handle, and it is only assigned
    // once the window is ordered in. A window that has never been shown reports
    // 0 or a negative value, and passing that blurs some *other* window or
    // nothing at all — hence the guard rather than a cast.
    let number: isize = unsafe { msg_send![ns_window, windowNumber] };
    if number <= 0 {
        log::warn!("glass: window not yet on screen (number {number}); blur skipped");
        return;
    }
    let status = unsafe { set_blur(connection, number as c_int, c_int::from(radius)) };
    if status != 0 {
        // CGError. Logged, not surfaced: the panes still paint, and a window
        // whose blur failed is plainer, not broken.
        log::warn!("glass: CGSSetWindowBackgroundBlurRadius({radius}) returned {status}");
    }
}

/// Resolve the `NSWindow` behind a Tauri window. Null only if the window is
/// gone, which can genuinely happen if this races a close.
fn ns_window(window: &WebviewWindow) -> Option<*mut AnyObject> {
    let ptr = window.ns_window().ok()?;
    (!ptr.is_null()).then(|| ptr.cast())
}

fn set_blur_fn() -> Option<SetBlurFn> {
    static FN: OnceLock<Option<SetBlurFn>> = OnceLock::new();
    *FN.get_or_init(|| dlsym_fn(b"CGSSetWindowBackgroundBlurRadius\0"))
}

/// The WindowServer connection for this process.
///
/// Two names because the symbol was renamed across OS versions and both still
/// ship on some; taking whichever answers is cheaper than version-sniffing.
/// The function pointer is resolved once but *called* every time — the
/// `DefaultConnectionForThread` variant is per-thread, so caching the value
/// would hand one thread another thread's connection.
fn cgs_connection() -> Option<CgsConnection> {
    static FN: OnceLock<Option<ConnectionFn>> = OnceLock::new();
    let function = (*FN.get_or_init(|| {
        dlsym_fn(b"CGSDefaultConnectionForThread\0").or_else(|| dlsym_fn(b"CGSMainConnectionID\0"))
    }))?;
    let connection = unsafe { function() };
    (connection != 0).then_some(connection)
}

/// Look a symbol up in the loaded images and reinterpret it as a function
/// pointer. `transmute_copy` rather than `transmute` because `T` is a generic
/// whose size the compiler cannot prove pointer-sized; every instantiation here
/// is an `extern "C" fn`, which is.
fn dlsym_fn<T>(symbol: &[u8]) -> Option<T> {
    unsafe {
        let ptr = dlsym(RTLD_DEFAULT, symbol.as_ptr().cast());
        if ptr.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&ptr))
        }
    }
}

/// Resolve the main window and hand it to `f`, so `lib.rs` and the command
/// layer stay one-liners and neither repeats the label.
pub fn with_main<F: FnOnce(&WebviewWindow)>(app: &tauri::AppHandle, f: F) {
    match app.get_webview_window("main") {
        Some(window) => f(&window),
        // `tauri.conf.json` sets no label, so it is Tauri's default "main". If
        // that ever changes this warns rather than silently shipping an
        // unblurred window.
        None => log::warn!("glass: no window labelled \"main\"; skipping"),
    }
}

/// Re-apply the glass whenever the accessibility display options change.
///
/// Registered once, at setup, and never torn down — the observer lives as long
/// as the process. The block is deliberately leaked for that reason: a dropped
/// `RcBlock` leaves the notification centre holding a dangling pointer, which
/// crashes at the first flip rather than at registration.
///
/// `queue: nil` means the block runs on whichever thread posts, and AppKit
/// posts this one on the main thread — where the setters underneath have to run
/// anyway. Passing an operation queue would move it off.
///
/// ⚠ This cannot be exercised with `defaults write com.apple.universalaccess
/// reduceTransparency`: that edits the plist WITHOUT posting the notification,
/// so the observer never fires. A fresh process does read the new value (the
/// hydrate path), so test hydrate that way and the live flip with the real
/// System Settings toggle.
pub fn install_accessibility_observer(app: &tauri::AppHandle) {
    let handle = app.clone();
    let block = block2::RcBlock::new(move |_note: *mut AnyObject| {
        // Logged because this is the only externally-triggered path in the
        // module: without it, "never fired" and "fired and did nothing" look
        // identical from the outside.
        let on = reduce_transparency();
        log::info!("glass: reduce transparency -> {on}");
        with_main(&handle, refresh);
        // The window is only half of it. The panes carry their own alpha, so
        // without this they stay translucent over a now-opaque window and the
        // sidebar lands a shade off its token. Fire-and-forget: the native half
        // has already applied, and it is the half that matters.
        let _ = handle.emit("reduce-transparency-changed", on);
    });

    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            log::warn!("glass: no NSWorkspace; Reduce Transparency will need a relaunch");
            return;
        }
        let center: *mut AnyObject = msg_send![workspace, notificationCenter];
        // Spelled out rather than imported: the constant lives in AppKit, which
        // this crate reaches only through raw `msg_send!`. The string is the
        // constant's value, checked against the AppKit headers.
        let name = NSString::from_str("NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification");
        let null = std::ptr::null_mut::<AnyObject>();
        let _token: *mut AnyObject = msg_send![
            center,
            addObserverForName: &*name,
            object: null,
            queue: null,
            usingBlock: &*block,
        ];
    }

    // See the doc comment: the notification centre holds this by pointer.
    std::mem::forget(block);
    log::info!(
        "glass: watching Reduce Transparency (currently {}; CGS blur {})",
        if reduce_transparency() { "on" } else { "off" },
        if is_available() { "available" } else { "MISSING, will fall back" }
    );
}
