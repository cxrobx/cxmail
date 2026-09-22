//! Let the frontend drive the window's desktop blur (`glass_macos`).
//!
//! Two setters rather than one, because they answer different questions and run
//! at very different rates. `set_window_glass` is the *state* — glass on or off,
//! once after first paint and again on every theme change. `set_blur_radius` is
//! the *amount*, driven by the transparency slider, and only called when the
//! rounded radius actually changes during a drag.
//!
//! All three hop to the main thread: `#[tauri::command]` bodies run on Tokio
//! workers and every AppKit call underneath is main-thread-only (gotcha #20).

use crate::error::AppError;

/// Turn the glass on (with a starting radius) or off (repainting the window
/// opaque in the theme's base colour).
///
/// The RGB triple is required even when enabling, so that disabling has
/// somewhere to go without a second round trip — and so the colour is always
/// the frontend's, never a constant in here that drifts from `globals.css`.
#[tauri::command]
pub async fn set_window_glass(
    app: tauri::AppHandle,
    enabled: bool,
    radius: u8,
    r: u8,
    g: u8,
    b: u8,
) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        // Cloned for the closure: `run_on_main_thread` is called ON `app`, so
        // moving the same handle inside would borrow it and move it at once.
        let handle = app.clone();
        app.run_on_main_thread(move || {
            crate::glass_macos::with_main(&handle, |window| {
                crate::glass_macos::set_state(window, enabled, radius, r, g, b);
            });
        })
        .map_err(|e| {
            AppError::General(format!("could not reach the main thread to set glass: {e}"))
        })?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, enabled, radius, r, g, b);
    }

    Ok(())
}

/// Change the blur radius without touching the window's glass state.
///
/// A no-op on the window when glass is off — `glass_macos::set_radius` records
/// the value and applies nothing — so this layer does not have to track state
/// the frontend already owns.
#[tauri::command]
pub async fn set_blur_radius(app: tauri::AppHandle, radius: u8) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        app.run_on_main_thread(move || {
            crate::glass_macos::with_main(&handle, |window| {
                crate::glass_macos::set_radius(window, radius);
            });
        })
        .map_err(|e| {
            AppError::General(format!("could not reach the main thread to set blur: {e}"))
        })?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, radius);
    }

    Ok(())
}

/// Is Reduce Transparency on right now?
///
/// The frontend needs it at hydrate so the CSS tint matches the window on the
/// first paint — the native side goes opaque on its own, but the panes would
/// otherwise stay translucent over it and the sidebar would land a shade off
/// its token. Changes after that arrive as a `reduce-transparency-changed`
/// event.
///
/// Unlike the setters this WAITS for the main thread and returns its answer.
/// Reading it on the Tokio worker this body runs on is gotcha #20's shape:
/// AppKit reads off the main thread can come back zeroed, and a zeroed BOOL
/// here is `false` — an accessibility setting silently ignored. (cxtasks reads
/// it on the worker; this is a deliberate departure.)
#[tauri::command]
pub async fn get_reduce_transparency(app: tauri::AppHandle) -> Result<bool, AppError> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(crate::glass_macos::reduce_transparency());
        })
        .map_err(|e| {
            AppError::General(format!(
                "could not reach the main thread to read Reduce Transparency: {e}"
            ))
        })?;
        return rx.await.map_err(|_| {
            AppError::General("main thread dropped the Reduce Transparency read".into())
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(false)
    }
}
