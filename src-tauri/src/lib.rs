mod commands;
pub mod appearance_macos;
// Stays in the app crate rather than moving with `email/`: its six `crate::notify`
// calls are tauri/objc2-bound, and `lib.rs` is its only consumer.
pub mod glass_macos;
pub mod idle;
pub mod launchd;
pub mod notify;
pub mod notify_macos;
pub mod plugins;
pub mod spellcheck_macos;
pub mod tracker;
pub mod vibrancy_macos;

// `error`, `keychain`, `secrets` and `LockExt` now live in `cxmail-core`, which
// every crate can depend on. Re-exported at their historical paths so the ~83
// files that say `crate::error::AppError`, the three `bin/` targets that say
// `cxmail_lib::error::AppError`, and both `examples/` keep resolving unedited.
pub use cxmail_core::{error, keychain, secrets, LockExt};
pub use cxmail_db::db;
pub use cxmail_email::email;
pub use cxmail_mcp::mcp;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Listener, Manager};

/// The Tauri half of [`cxmail_core::EventSink`].
///
/// `email::oauth2` and `email::gcal_invite` used to take an `AppHandle` purely
/// to emit and to reach `state.db`. They now take a `cxmail_core::AppCtx`, and
/// this is where one gets built — the only place in the tree that knows both
/// that events go out over Tauri and where the connection lives.
struct TauriEvents(tauri::AppHandle);

impl cxmail_core::EventSink for TauriEvents {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        let _ = tauri::Emitter::emit(&self.0, event, payload);
    }
}

/// Build an [`AppCtx`] from a handle. Returns `None` before `AppState` is
/// managed, which in practice means before setup finished.
pub fn app_ctx(app: &tauri::AppHandle) -> Option<cxmail_core::AppCtx> {
    let state = app.try_state::<AppState>()?;
    Some(cxmail_core::AppCtx::new(
        std::sync::Arc::new(TauriEvents(app.clone())),
        state.db.clone(),
    ))
}

pub struct AppState {
    /// `Arc` so a detached background task can hold the connection without an
    /// `AppHandle` — see `cxmail_core::AppCtx`. Every `state.db.safe_lock()`
    /// call site is unaffected; `Arc<Mutex<_>>` derefs to `Mutex<_>`.
    pub db: std::sync::Arc<Mutex<rusqlite::Connection>>,
    /// Database path used to open short-lived read-only connections on demand.
    /// This avoids serializing all UI reads through a single mutexed connection.
    pub db_path: PathBuf,
    pub pending_sends: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    /// Set while a `sync_all_inboxes` is running. Both the frontend-triggered
    /// IPC and the backend periodic safety-net sync check this to avoid
    /// racing each other. Use compare_exchange/swap for atomic claiming.
    pub sync_in_flight: AtomicBool,
}

impl AppState {
    pub fn open_read_conn(&self) -> Result<rusqlite::Connection, crate::error::AppError> {
        let conn = rusqlite::Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(conn)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("external-links")
                .on_navigation(|webview, url| {
                    let scheme = url.scheme();
                    // Allow Tauri internal and data URLs
                    if scheme == "tauri" || scheme == "about" || scheme == "data" || scheme == "blob" {
                        return true;
                    }
                    // Allow dev server
                    if url.host_str() == Some("localhost") {
                        return true;
                    }
                    // mailto: — handle in-app. Emit to the frontend funnel, which
                    // parses it (RFC 6068) and opens a prefilled compose. We must
                    // NOT shell out to `open` here: once CXMail is the default
                    // mailto handler, that would bounce the URL right back to us.
                    if scheme == "mailto" {
                        let _ = webview.app_handle().emit("mailto-open", url.as_str());
                        return false;
                    }
                    // Other external URLs (http/https) open in the system browser.
                    if scheme == "http" || scheme == "https" {
                        let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
                        return false;
                    }
                    true
                })
                .build(),
        )
        .on_window_event(|window, event| {
            // Cmd+W and red close button hide the window instead of quitting
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::start_oauth2,
            commands::auth::start_calendar_oauth,
            commands::auth::calendar_connection_status,
            commands::auth::add_icloud_account,
            commands::auth::add_imap_account,
            commands::auth::discover_mail_config,
            commands::accounts::list_accounts,
            commands::accounts::remove_account,
            commands::accounts::rename_account,
            commands::accounts::reorder_accounts,
            commands::accounts::set_account_group,
            commands::accounts::set_account_notify_enabled,
            commands::accounts::set_account_hidden_from_aggregates,
            commands::triage::get_triage_status,
            commands::triage::set_triage_mode,
            commands::triage::set_triage_model,
            commands::triage::set_triage_effort,
            commands::triage::set_account_triage_enabled,
            commands::triage::run_triage_pass_now,
            commands::accounts::set_account_track_opens_enabled,
            commands::folders::list_folders,
            commands::folders::sync_folders,
            commands::folders::create_folder,
            commands::folders::delete_folder,
            commands::folders::rename_folder,
            commands::messages::fetch_messages,
            commands::messages::fetch_message_body,
            commands::messages::get_cached_message_body,
            commands::messages::sync_folder,
            commands::messages::mark_as_read,
            commands::messages::mark_as_unread,
            commands::messages::fetch_unified_inbox,
            commands::messages::sync_all_inboxes,
            commands::messages::force_full_sync,
            commands::messages::move_messages,
            commands::messages::archive_messages,
            commands::messages::delete_messages,
            commands::messages::toggle_star,
            commands::messages::toggle_mute,
            commands::messages::toggle_pin,
            commands::messages::search_messages,
            commands::messages::ai_search_messages,
            commands::messages::search_with_filters,
            commands::messages::server_search,
            commands::messages::get_thread,
            commands::messages::get_thread_uids_in_folder,
            commands::messages::search_contacts,
            commands::compose::send_email,
            commands::compose::save_draft,
            commands::compose::edit_draft,
            commands::compose::download_attachment,
            commands::compose::fetch_outgoing_attachments,
            commands::compose::cancel_send,
            commands::snooze::snooze_message,
            commands::snooze::unsnooze_message,
            commands::snooze::list_snoozed_messages,
            commands::schedule::schedule_send,
            commands::schedule::list_scheduled,
            commands::schedule::cancel_scheduled,
            commands::schedule::edit_scheduled,
            commands::settings::list_identities,
            commands::settings::create_identity,
            commands::settings::update_identity,
            commands::settings::delete_identity,
            commands::settings::list_send_as,
            commands::settings::reply_from_for_message,
            commands::settings::suggest_send_as,
            commands::settings::remove_send_as,
            commands::plugins::list_plugins,
            commands::plugins::reload_plugins,
            commands::plugins::run_plugin,
            commands::import::import_mbox,
            commands::import::spam_classify,
            commands::import::spam_train,
            commands::summarize::summarize_message,
            commands::summarize::get_ai_api_key,
            commands::summarize::save_ai_api_key,
            commands::inference::get_ai_provider_settings,
            commands::inference::save_ai_provider_settings,
            commands::inference::test_ai_provider,
            commands::inference::get_writer_settings,
            commands::inference::save_writer_model,
            commands::inference::list_writer_models,
            commands::zoom::zoom_connection_status,
            commands::zoom::save_zoom_credentials,
            commands::zoom::clear_zoom_credentials,
            commands::zoom::test_zoom_connection,
            commands::needs_you::list_needs_you,
            commands::needs_you::dismiss_needs_you,
            commands::needs_you::dismiss_needs_you_group,
            commands::nudges::list_nudges,
            commands::nudges::dismiss_nudge,
            commands::license::get_license_status,
            commands::license::refresh_license_status,
            commands::license::activate_license,
            commands::ai::ai_generate_reply,
            commands::ai::ai_rewrite_text,
            commands::ai::ai_adjust_tone,
            commands::ai::ai_proofread,
            commands::ai::ai_validate_draft,
            commands::ai::ai_smart_replies,
            commands::ai::ai_suggest_subject,
            commands::ai::ai_extract_voice_profile,
            commands::ai::ai_get_voice_profile_status,
            commands::ai::ai_log_reply_edit,
            commands::ai::ai_get_recipient_profile,
            commands::ai::ai_refresh_recipient_profile,
            commands::ai::ai_generate_compose,
            commands::ai::ai_list_archetypes,
            commands::ai::ai_extract_archetypes,
            commands::ai::ai_recluster_archetypes,
            commands::ai::ai_rename_archetype,
            commands::ai::ai_delete_archetype,
            commands::ai::ai_get_insight_status,
            commands::ai::ai_learn_from_edits,
            commands::tracking::get_tracking_config,
            commands::tracking::set_tracking_config,
            commands::tracking::list_tracking_pixels,
            commands::tracking::get_tracking_pixel,
            commands::tracking::get_tracking_pixel_for_message,
            commands::tracking::sync_tracking_events,
            commands::tracking::test_tracker_connection,
            commands::categories::get_category_counts,
            commands::categories::set_message_category,
            commands::categories::set_messages_category,
            commands::followup::create_followup_reminder,
            commands::followup::cancel_followup_reminder,
            commands::followup::dismiss_followup_reminder,
            commands::followup::list_followup_reminders,
            commands::messages::reclassify_inbox_messages,
            commands::messages::unsubscribe,
            commands::messages::unsubscribe_sender,
            commands::messages::list_unsubscribed_senders,
            commands::messages::record_unsubscribed_sender,
            commands::messages::remove_unsubscribed_sender,
            commands::image_trust::trust_image_sender,
            commands::image_trust::untrust_image_sender,
            commands::image_trust::is_image_sender_trusted,
            commands::image_trust::list_trusted_image_senders,
            commands::images::save_image_to_path,
            commands::images::copy_image_to_clipboard,
            commands::templates::list_templates,
            commands::templates::create_template,
            commands::templates::update_template,
            commands::templates::delete_template,
            commands::rules::list_mail_rules,
            commands::rules::create_mail_rule,
            commands::rules::update_mail_rule,
            commands::rules::delete_mail_rule,
            commands::compose::read_file_as_base64,
            commands::calendar::get_calendar_events,
            commands::calendar::rsvp_event,
            commands::calendar::list_upcoming_events,
            commands::calendar::list_calendar_events_in_range,
            commands::calendar::dismiss_calendar_event,
            commands::calendar::list_google_calendars,
            commands::calendar::create_google_calendar_event,
            commands::calendar::update_google_calendar_event,
            commands::calendar::delete_google_calendar_event,
            commands::calendar::send_google_calendar_invites,
            commands::calendar::sync_google_calendar_now,
            commands::calendar::resolve_google_calendar_conflict,
            commands::inbox_groups::list_inbox_groups,
            commands::inbox_groups::create_inbox_group,
            commands::inbox_groups::update_inbox_group,
            commands::inbox_groups::delete_inbox_group,
            commands::inbox_groups::fetch_inbox_group_messages,
            commands::settings::install_background_sync,
            commands::settings::uninstall_background_sync,
            commands::settings::is_background_sync_installed,
            commands::claude_handoff::open_email_in_claude,
            commands::claude_handoff::get_claude_prompt,
            commands::claude_repos::list_claude_repos,
            commands::claude_repos::set_claude_repo,
            commands::claude_repos::clear_claude_repo,
            commands::chat::chat_start,
            commands::chat::chat_send,
            commands::chat::chat_answer_permission,
            commands::chat::chat_interrupt,
            commands::chat::chat_stop,
            commands::chat::chat_continue_in_terminal,
            commands::system::set_as_default_mail_client,
            commands::system::is_default_mail_client,
            commands::system::log_client_error,
            commands::system::set_native_appearance,
            commands::glass::set_window_glass,
            commands::glass::set_blur_radius,
            commands::glass::get_reduce_transparency,
            commands::vault_look::fetch_vault_look,
        ])
        .setup(|app| {
            // Install logging in BOTH debug and release builds. Production
            // logs land in ~/Library/Logs/com.cxmail.app/. Without this, prod
            // bugs are invisible — see Apr 2026 sync-stuck incident.
            //
            // Note: Builder::default() already includes Stdout + LogDir as
            // default targets, so we use .targets() to REPLACE them and
            // avoid double-writing every log line.
            let targets: Vec<tauri_plugin_log::Target> = if cfg!(debug_assertions) {
                vec![
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ]
            } else {
                vec![tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                )]
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .max_file_size(5_000_000) // 5 MB per file
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                    .targets(targets)
                    .build(),
            )?;

            // Initialize database
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            fs::create_dir_all(&app_dir)?;

            let db_path = app_dir.join("cxmail.db");
            let conn = rusqlite::Connection::open(&db_path)?;
            // Retry for up to 5s if another process holds a write lock
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            db::schema::initialize(&conn)?;
            db::tracking::migrate_legacy_api_key(&conn)?;

            // One-time migration of credentials from the legacy encrypted file
            // store into the macOS Keychain. No-op unless this is the signed app
            // bundle; idempotent. Leaves the file store intact so local unsigned
            // tools (e.g. the cxmail-mcp binary) keep working. See keychain::mod.
            if let Err(e) = crate::keychain::migrate_file_store_to_keychain() {
                log::warn!("credential store migration skipped: {e}");
            }

            // One-time repair of Keychain ACLs so cxmail-helper / cxmail-mcp can
            // read items the app minted, without an authorization prompt. Must
            // run here: the signed app is the only process trusted on every
            // item, so it is the only one that can read them prompt-free in
            // order to rewrite them. Gated on a marker file; see gotcha #31.
            if let Err(e) = crate::keychain::repair_keychain_acls() {
                log::warn!("keychain ACL repair skipped: {e}");
            }

            log::info!("Database initialized at {:?}", db_path);

            // Pin native chrome (NSMenu, <select> popups, sheets, scrollbars)
            // so it matches the UI instead of the system setting. `.setup()` is
            // the main thread, which AppKit requires.
            //
            // Dark is the *boot* default only, matching `tauri.conf.json`'s
            // window `"theme": "Dark"` — the real choice lives in the
            // frontend's localStorage, which Rust cannot read from here. The
            // frontend calls `set_native_appearance` on mount to correct it.
            // That leaves at most a frame of native mismatch on a light-theme
            // launch, and only for chrome that is not on screen yet.
            appearance_macos::set_appearance(Some(true));

            // Paint the window OPAQUE for launch, and let the frontend turn the
            // glass on once it has something to show (`startGlass` in
            // `uiStore.ts`, behind a double rAF in `main.tsx`).
            //
            // The window is `"transparent": true` in `tauri.conf.json`, so
            // without this it boots as a clear pane: between window creation and
            // React's first paint there is nothing in it, and what the user sees
            // is bare wallpaper with three traffic lights floating on it. The old
            // `NSVisualEffectView` hid that gap by filling it with a material; a
            // CGS blur paints nothing of its own, so the launch state has to be
            // opaque and the flip deferred (gotcha #59).
            //
            // Dark's `--bg-primary`, matching the `"theme": "Dark"` boot default
            // and the appearance pin above. A light-theme launch is corrected by
            // the same frontend call that enables the glass. `glass.test.ts`
            // pins this triplet against `globals.css`.
            #[cfg(target_os = "macos")]
            glass_macos::with_main(&app.handle().clone(), |window| {
                glass_macos::set_launch_background(window, 28, 26, 23);
            });

            // Reduce Transparency used to be free — `NSVisualEffectView` went
            // opaque by itself. A raw CGS blur reads no such setting, so we now
            // watch for it. Registered once; the observer lives as long as the
            // process (see `glass_macos` for why its block is leaked).
            #[cfg(target_os = "macos")]
            glass_macos::install_accessibility_observer(&app.handle().clone());

            // Anchor the IMAP uptime clock and stamp a launch marker.
            //
            // Both exist for the sync-timeout investigation (gotcha #45): the
            // log had no unambiguous "the app started here" line, so failure
            // rate could not be plotted against uptime — which is the whole
            // test that separates a session leak (rate climbs with uptime,
            // resets on relaunch) from a bad network (rate is flat). Without
            // the explicit call the clock would start at the first IMAP
            // connection instead, hiding exactly the early-uptime interval the
            // comparison needs.
            crate::email::imap::note_process_start();
            log::info!(
                "CXMail launched: version={} pid={}",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            );

            // Initialize plugin manager
            let plugins_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("cxmail")
                .join("plugins");
            let mut plugin_manager = plugins::manager::PluginManager::new(plugins_dir);
            plugin_manager.discover();

            app.manage(commands::plugins::PluginState {
                manager: Mutex::new(plugin_manager),
            });

            // FTS5 search integrity net. The index lives in the DB and is
            // maintained by triggers (schema v40), so app/helper/MCP writes
            // all index automatically. This only repairs drift in either
            // direction (ghosts or missing rows) or outright corruption —
            // and never panics (the old Tantivy init could brick launch).
            db::search::verify_or_rebuild(&conn);

            // Reclaim disk from the abandoned Tantivy index directories.
            for dir in ["search_index", "search_index_v2"] {
                let p = app_dir.join(dir);
                if p.exists() {
                    match fs::remove_dir_all(&p) {
                        Ok(()) => log::info!("Removed obsolete Tantivy index dir {:?}", p),
                        Err(e) => log::warn!("Failed to remove obsolete index dir {:?}: {e}", p),
                    }
                }
            }

            app.manage(AppState {
                db: std::sync::Arc::new(Mutex::new(conn)),
                db_path: db_path.clone(),
                pending_sends: Mutex::new(HashMap::new()),
                sync_in_flight: AtomicBool::new(false),
            });

            // Calendar-invite approvals no longer expire on their own, so one
            // the user never answered would wedge its event permanently:
            // `request_invite_approval` refuses to raise a second request while
            // a decision is still pending. Clear anything older than a day.
            {
                let state = app.state::<AppState>();
                let conn = state.db.safe_lock();
                match db::gcal::sweep_stale_invite_approvals(&conn, 24) {
                    Ok(0) => {}
                    Ok(count) => {
                        log::info!("Cleared {count} stale calendar invite approval(s)")
                    }
                    Err(error) => log::warn!("Invite approval sweep failed: {error}"),
                }
            }

            // Heal messages stored before classification moved into the insert.
            //
            // `classify_headers` used to be called by hand after
            // `apply_header_batch`, and only the app's full-sync path did it —
            // `idle.rs` and `bin/helper.rs` insert through the same function and
            // did not. Since the helper is a KeepAlive LaunchAgent that runs
            // whether or not the app is open, most arriving mail was stored with
            // `category = NULL`, which `COALESCE(category, 'primary')` renders as
            // Primary: 11,943 inbox messages on the live mailbox, 58.6% of the
            // last 45 days. New mail can no longer land that way; this clears
            // what already did.
            //
            // Scoped to `category IS NULL`, so it is idempotent by construction
            // — after one pass it matches nothing — and it never revisits a
            // category a rule or the user decided. Spawned rather than awaited:
            // it is a one-time backfill over thousands of rows and must not hold
            // the window's startup, and it takes the DB lock in the same small
            // scopes the button does (gotcha #11).
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    match commands::messages::reclassify_impl(&state, true).await {
                        Ok(r) if r.total == 0 => {}
                        Ok(r) => log::info!(
                            "Category backfill: {} unclassified message(s), {} updated",
                            r.total,
                            r.changed
                        ),
                        Err(e) => log::warn!("Category backfill failed: {e}"),
                    }
                });
            }

            // Approval responses originate in McpApprovalModal and are written
            // to the shared DB so the standalone MCP process can observe the
            // decision without exposing a network listener.
            {
                let approval_handle = app.handle().clone();
                app.listen("mcp-approval-response", move |event| {
                    let Ok(payload) =
                        serde_json::from_str::<serde_json::Value>(event.payload())
                    else {
                        log::warn!("Ignoring malformed MCP approval response");
                        return;
                    };
                    let Some(id) = payload["id"].as_str() else {
                        return;
                    };
                    let approved = payload["decision"].as_str() == Some("approved");
                    let Some(state) = approval_handle.try_state::<AppState>() else {
                        return;
                    };
                    // Scope the guard: the delivery task spawned below locks the
                    // same mutex, and holding it across the spawn would park that
                    // task on a lock this closure still owns (gotcha #11).
                    let flipped = {
                        let conn = state.db.safe_lock();
                        match db::gcal::respond_to_invite_approval(&conn, id, approved) {
                            Ok(flipped) => flipped,
                            Err(error) => {
                                log::warn!("Failed to persist MCP approval response: {error}");
                                return;
                            }
                        }
                    };
                    // The decision is made; withdraw the banner so a live Approve
                    // button can't answer an approval that no longer exists.
                    #[cfg(not(debug_assertions))]
                    notify_macos::clear_approval_notification(id);

                    // `flipped == false` means no pending approval matched — a
                    // double-click, or a decision the staleness sweep already
                    // cleared. Nothing to deliver, and not worth alarming about.
                    if !approved || !flipped {
                        return;
                    }
                    // The MCP no longer waits for this decision, so approving is
                    // what actually notifies attendees. Do it here, off the event
                    // callback, so a Google round trip can't stall the UI.
                    let delivery_handle = approval_handle.clone();
                    let approval_id = id.to_string();
                    // Built from the `state` already resolved above rather than
                    // re-querying: the guard at the top of this closure has
                    // proven it, and a second fallible lookup here would add an
                    // unreachable branch that reads like a real failure path.
                    let ctx = cxmail_core::AppCtx::new(
                        std::sync::Arc::new(TauriEvents(delivery_handle.clone())),
                        state.db.clone(),
                    );
                    tauri::async_runtime::spawn(async move {
                        match email::gcal_invite::deliver_approved_invite(&ctx, &approval_id)
                            .await
                        {
                            Ok(email::gcal_invite::DeliveryOutcome::Sent(recipients)) => {
                                log::info!(
                                    "Calendar invites delivered to {} recipient(s)",
                                    recipients.len()
                                );
                            }
                            Ok(email::gcal_invite::DeliveryOutcome::NothingToDo) => {
                                log::info!(
                                    "Calendar approval {approval_id} had nothing to deliver"
                                );
                            }
                            Err(error) => {
                                // The user pressed Approve and is entitled to know
                                // it did not go out — silence here would recreate
                                // the failure mode this whole change removes.
                                log::warn!("Calendar invite delivery failed: {error}");
                                let _ = delivery_handle.emit(
                                    "calendar-invites-failed",
                                    serde_json::json!({ "error": error.to_string() }),
                                );
                            }
                        }
                    });
                });
            }

            // Initialize native macOS notification delegate for click-to-open.
            // UNUserNotificationCenter can throw an NSException in unsigned
            // dev builds (bundleProxyForCurrentProcess is nil), and the throw
            // happens later during NSApplicationDidFinishLaunching — outside
            // the scope of any catch around `init` — which hard-crashes the
            // app. Skip it entirely in debug builds.
            #[cfg(not(debug_assertions))]
            match objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                notify_macos::init(app.handle());
            })) {
                Ok(()) => {}
                Err(e) => {
                    log::warn!(
                        "notify_macos::init threw ObjC exception — native notifications disabled: {:?}",
                        e
                    );
                }
            }

            // Enable native continuous spell checking (red squiggles) in the
            // WKWebView — the HTML `spellcheck` attribute alone doesn't turn it
            // on. Runs in debug + release; safe via respondsToSelector guard.
            spellcheck_macos::enable(app.handle());

            // Write lock file so the background daemon knows the main app is running
            let lock_path = app_dir.join("app.lock");
            fs::write(&lock_path, std::process::id().to_string())?;

            // Build system tray icon
            let tray_menu = MenuBuilder::new(app)
                .text("open", "Open CXMail")
                .text("check", "Check for New Mail")
                .text("set-default-mail", "Set as Default Email Client")
                .separator()
                .text("quit", "Quit CXMail")
                .build()?;

            let icon_bytes = include_bytes!("../icons/tray-icon.png");
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(Image::from_bytes(icon_bytes)?)
                .icon_as_template(true)
                .tooltip("CXMail")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "open" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "check" => {
                            let _ = app.emit("check-mail", serde_json::json!({}));
                        }
                        "set-default-mail" => {
                            let ok = commands::system::set_default_native();
                            log::info!("Tray: set default mail client → {ok}");
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Auto-install background sync LaunchAgent on first production launch
            if !cfg!(debug_assertions) && !launchd::is_installed() {
                if let Err(e) = launchd::install_launch_agent() {
                    log::warn!("Failed to auto-install background sync: {}", e);
                }
            }

            // Start IMAP IDLE watchers for real-time new mail notifications
            let idle_shutdown = idle::start_idle_watchers(app.handle().clone());

            // ── MCP → app live bridge ──────────────────────────────────────
            // The standalone `cxmail-mcp` process writes one JSON Envelope per
            // connection to a Unix socket when it mutates a draft/email; we
            // re-emit each as a "mcp-activity" frontend event so the app
            // live-refreshes (no polling). See `mcp/bridge.rs`.
            {
                let socket = mcp::bridge::socket_path();
                // Clear any stale socket from a previous (crashed) run before binding.
                let _ = fs::remove_file(&socket);
                let bridge_handle = app.handle().clone();
                // CRITICAL: `UnixListener::bind` registers the socket fd with the
                // Tokio reactor, so it MUST run inside the runtime. `.setup()`
                // executes on the main thread OUTSIDE any runtime — binding here
                // synchronously panics ("there is no reactor running"), and because
                // setup runs inside the ObjC `did_finish_launching` callback that
                // panic can't unwind and aborts the whole app. So do the bind (and
                // the accept loop) inside the spawned task, which runs on Tauri's
                // Tokio runtime.
                tauri::async_runtime::spawn(async move {
                    let listener = match tokio::net::UnixListener::bind(&socket) {
                        Ok(l) => {
                            log::info!("MCP live bridge listening at {:?}", socket);
                            l
                        }
                        Err(e) => {
                            // EADDRINUSE → another app instance owns the socket
                            // (gotcha #12). The app still runs fine, just without
                            // the live bridge — bind failure is non-fatal by design.
                            log::warn!(
                                "MCP live bridge bind {:?} failed (another instance?): {e}",
                                socket
                            );
                            return;
                        }
                    };
                    loop {
                        match listener.accept().await {
                            Ok((mut stream, _addr)) => {
                                let conn_handle = bridge_handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    use tokio::io::AsyncReadExt;
                                    let mut buf = Vec::new();
                                    if let Err(e) = stream.read_to_end(&mut buf).await {
                                        log::debug!("MCP bridge: read failed: {e}");
                                        return;
                                    }
                                    // serde_json tolerates the trailing newline as whitespace.
                                    match serde_json::from_slice::<mcp::bridge::Envelope>(&buf) {
                                        Ok(env) => {
                                            log::info!(
                                                "MCP bridge: received {} (tool={:?}, account={}, folder={}, uid={}, old_uid={:?}) → emitting mcp-activity",
                                                env.kind, env.tool, env.account_id, env.folder, env.uid, env.old_uid
                                            );
                                            if let Err(e) = conn_handle.emit("mcp-activity", &env) {
                                                log::warn!("MCP bridge: emit failed: {e}");
                                            }
                                            if env.kind == "calendar-approval-request" {
                                                if let Some(approval_id) = env.approval_id.as_deref()
                                                {
                                                    let (event, zoom_join_url) = {
                                                        let Some(state) =
                                                            conn_handle.try_state::<AppState>()
                                                        else {
                                                            return;
                                                        };
                                                        let conn = state.db.safe_lock();
                                                        (
                                                            db::gcal::get_event(
                                                                &conn,
                                                                i64::from(env.uid),
                                                            )
                                                            .ok()
                                                            .flatten(),
                                                            db::zoom::get_by_local_event(
                                                                &conn,
                                                                i64::from(env.uid),
                                                            )
                                                            .ok()
                                                            .flatten()
                                                            .and_then(|link| link.join_url),
                                                        )
                                                    };
                                                    if let Some(event) = event {
                                                        let attendees = event
                                                            .attendees_json
                                                            .as_deref()
                                                            .and_then(|json| {
                                                                serde_json::from_str::<
                                                                    Vec<email::gcal::EventAttendee>,
                                                                >(json)
                                                                .ok()
                                                            })
                                                            .unwrap_or_default()
                                                            .into_iter()
                                                            .filter_map(|attendee| attendee.email)
                                                            .collect::<Vec<_>>()
                                                            .join(", ");
                                                        let request = serde_json::json!({
                                                            "id": approval_id,
                                                            "action": "send calendar invites",
                                                            "description": "Claude created or updated this event without notifying attendees. Review every recipient before approving delivery.",
                                                            "details": {
                                                                "Event": event.summary.unwrap_or_else(|| "Event".to_string()),
                                                                "Starts": email::gcal_invite::format_event_start(&event.dtstart),
                                                                "Attendees": attendees,
                                                                // Zoom first: on a Zoom-backed event
                                                                // `hangout_link` is NULL, so the old
                                                                // hardcoded "Meet" row read "None"
                                                                // while the user approved notifying
                                                                // attendees about a Zoom call. The
                                                                // join link is part of what is being
                                                                // approved, so it has to be shown.
                                                                "Conferencing": zoom_join_url
                                                                    .map(|url| format!("Zoom — {url}"))
                                                                    .or_else(|| event
                                                                        .hangout_link
                                                                        .map(|url| format!("Google Meet — {url}")))
                                                                    .unwrap_or_else(|| "None".to_string())
                                                            }
                                                        });
                                                        if let Err(error) = conn_handle
                                                            .emit("mcp-approval-request", &request)
                                                        {
                                                            log::warn!(
                                                                "Failed to show calendar approval: {error}"
                                                            );
                                                        }
                                                        // The approval card renders inside the main
                                                        // window, so a backgrounded CXMail showed it
                                                        // to nobody. Bounce the dock icon until the
                                                        // app is looked at — this is the one signal
                                                        // that survives Focus modes and whatever the
                                                        // user's notification style is set to.
                                                        if let Some(window) =
                                                            conn_handle.get_webview_window("main")
                                                        {
                                                            let _ = window.request_user_attention(
                                                                Some(tauri::UserAttentionType::Critical),
                                                            );
                                                        }
                                                        // …and a notification with Approve/Deny, so
                                                        // the decision can be made without leaving
                                                        // whatever the user is actually doing.
                                                        #[cfg(not(debug_assertions))]
                                                        {
                                                            let summary = request["details"]["Event"]
                                                                .as_str()
                                                                .unwrap_or("Calendar event");
                                                            let recipients = request["details"]
                                                                ["Attendees"]
                                                                .as_str()
                                                                .unwrap_or("attendees");
                                                            notify_macos::show_approval_notification(
                                                                approval_id,
                                                                "Send calendar invites?",
                                                                &format!("{summary} → {recipients}"),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            log::debug!("MCP bridge: parse failed: {e}");
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                log::warn!("MCP bridge: accept error: {e}");
                                // Brief backoff to avoid a hot loop on a broken listener.
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                        }
                    }
                });
            }

            // Spawn background scheduler for snooze wake-ups and scheduled sends
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                let mut tick_count: u64 = 0;
                loop {
                    interval.tick().await;
                    tick_count += 1;

                    let Some(state) = app_handle.try_state::<AppState>() else {
                        continue;
                    };

                    // 1. Process due snooze wake-ups
                    let due_snoozes = {
                        let conn = state.db.safe_lock();
                        db::snoozed::list_due(&conn).unwrap_or_default()
                    };
                    if !due_snoozes.is_empty() {
                        let count = due_snoozes.len();
                        {
                            let conn = state.db.safe_lock();
                            for snooze in &due_snoozes {
                                if let Err(e) = db::snoozed::delete(&conn, snooze.id) {
                                    log::error!("Failed to delete snoozed message {}: {e}", snooze.id);
                                }
                            }
                        }
                        log::info!("{} snoozed message(s) woke up", count);
                        if let Err(e) = app_handle.emit("snooze-wakeup", count) {
                            log::error!("Failed to emit snooze-wakeup event: {e}");
                        }
                    }

                    // 1b. AI triage pass.
                    //
                    // Returns immediately in `off`, which is the default and
                    // is what every account ships as. In `shadow` it records
                    // verdicts and changes nothing on screen; only `on` lets
                    // them reach Needs You.
                    //
                    // Every fourth tick — two minutes — rather than every one:
                    // five messages per pass at that cadence still clears a
                    // few-hundred-message backlog inside an hour, and it keeps
                    // the provider request rate looking like a trickle rather
                    // than a burst. The pass takes the DB lock only to read a
                    // batch and to write each verdict, never across the HTTP
                    // await (gotcha #11).
                    if tick_count % 4 == 0 {
                        if let Err(e) = commands::triage::run_pass(&state).await {
                            log::warn!("triage pass failed: {e}");
                        }
                    }

                    // 2. Process due scheduled sends
                    let due_sends = {
                        let conn = state.db.safe_lock();
                        db::scheduled::list_due(&conn).unwrap_or_default()
                    };
                    for scheduled in &due_sends {
                        let email_result: Result<email::smtp::OutgoingEmail, _> =
                            serde_json::from_str(&scheduled.email_json);
                        let mut email = match email_result {
                            Ok(e) => e,
                            Err(_) => {
                                let conn = state.db.safe_lock();
                                if let Err(e) = db::scheduled::update_status(
                                    &conn,
                                    scheduled.id,
                                    "failed",
                                    Some("Invalid email data"),
                                ) {
                                    log::error!("Failed to update scheduled email {} status to failed: {e}", scheduled.id);
                                }
                                continue;
                            }
                        };

                        let account = {
                            let conn = state.db.safe_lock();
                            db::accounts::get_by_id(&conn, &scheduled.account_id)
                                .ok()
                                .flatten()
                        };

                        match account {
                            Some(account) => {
                                // Per-account tracking gate evaluated at fire time, so
                                // disabling an account after scheduling suppresses the pixel.
                                commands::compose::maybe_inject_tracking_pixel(
                                    &state.db, &account, &mut email,
                                )
                                .await;
                                match email::smtp::send_email(&account, &email).await {
                                    Ok(_msg_id) => {
                                        let conn = state.db.safe_lock();
                                        if let Err(e) = db::scheduled::update_status(
                                            &conn,
                                            scheduled.id,
                                            "sent",
                                            None,
                                        ) {
                                            log::error!("Failed to update scheduled email {} status to sent: {e}", scheduled.id);
                                        }
                                        log::info!(
                                            "Scheduled email {} sent successfully",
                                            scheduled.id
                                        );
                                    }
                                    Err(e) => {
                                        let conn = state.db.safe_lock();
                                        if let Err(db_err) = db::scheduled::update_status(
                                            &conn,
                                            scheduled.id,
                                            "failed",
                                            Some(&e.to_string()),
                                        ) {
                                            log::error!("Failed to update scheduled email {} status to failed: {db_err}", scheduled.id);
                                        }
                                        log::error!(
                                            "Scheduled email {} failed: {}",
                                            scheduled.id,
                                            e
                                        );
                                    }
                                }
                            }
                            None => {
                                let conn = state.db.safe_lock();
                                if let Err(e) = db::scheduled::update_status(
                                    &conn,
                                    scheduled.id,
                                    "failed",
                                    Some("Account not found"),
                                ) {
                                    log::error!("Failed to update scheduled email {} status to failed: {e}", scheduled.id);
                                }
                            }
                        }
                    }
                    if !due_sends.is_empty() {
                        if let Err(e) = app_handle.emit("scheduled-send-processed", due_sends.len()) {
                            log::error!("Failed to emit scheduled-send-processed event: {e}");
                        }
                    }

                    // 3. Process follow-up reminders
                    // First, proactively auto-cancel any pending reminders that already got replies
                    {
                        let conn = state.db.safe_lock();
                        let cancelled = db::followup::auto_cancel_replied(&conn).unwrap_or(0);
                        if cancelled > 0 {
                            log::info!("{} follow-up reminder(s) auto-cancelled (reply received)", cancelled);
                        }
                    }

                    // Then, fire any due reminders that still have no reply
                    let due_followups = {
                        let conn = state.db.safe_lock();
                        db::followup::list_due(&conn).unwrap_or_default()
                    };
                    let mut fired_subjects: Vec<serde_json::Value> = Vec::new();
                    for reminder in &due_followups {
                        let reply_exists = {
                            let conn = state.db.safe_lock();
                            db::followup::check_reply_exists(
                                &conn,
                                &reminder.sent_message_id,
                                &reminder.sender_email,
                            )
                            .unwrap_or(false)
                        };

                        let conn = state.db.safe_lock();
                        if reply_exists {
                            if let Err(e) = db::followup::update_status(&conn, reminder.id, "auto-cancelled") {
                                log::error!("Failed to auto-cancel follow-up reminder {}: {e}", reminder.id);
                            }
                        } else {
                            if let Err(e) = db::followup::update_status(&conn, reminder.id, "fired") {
                                log::error!("Failed to update follow-up reminder {} to fired: {e}", reminder.id);
                            }
                            fired_subjects.push(serde_json::json!({
                                "id": reminder.id,
                                "to_email": reminder.to_email,
                                "subject": reminder.subject,
                            }));
                        }
                    }
                    if !fired_subjects.is_empty() {
                        log::info!("{} follow-up reminder(s) fired", fired_subjects.len());
                        if let Err(e) = app_handle.emit("followup-reminder-fired", &fired_subjects) {
                            log::error!("Failed to emit followup-reminder-fired event: {e}");
                        }
                    }

                    // Push local Calendar changes every 30-second tick and
                    // pull Google state every fourth tick (two minutes).
                    // The full pass pushes first, making the returning server
                    // representation authoritative. Accounts without a
                    // Calendar grant are skipped without touching mail tokens.
                    let calendar_handle = app_handle.clone();
                    let should_pull_calendar = tick_count % 4 == 0;
                    tauri::async_runtime::spawn(async move {
                        let Some(state) = calendar_handle.try_state::<AppState>() else {
                            return;
                        };
                        let result = if should_pull_calendar {
                            email::gcal_sync::sync_all(&state.db).await
                        } else {
                            email::gcal_sync::push_all(&state.db).await
                        };
                        if let Err(error) = result {
                            log::warn!("Google Calendar background sync failed: {error}");
                        } else if should_pull_calendar {
                            if let Err(error) =
                                calendar_handle.emit("calendar-sync-complete", ())
                            {
                                log::debug!("Failed to emit calendar-sync-complete: {error}");
                            }
                        }

                        // Reap Zoom tombstones every tick — one indexed SELECT
                        // that is almost always empty, and it is what finishes a
                        // delete interrupted by a crash or a revoked credential.
                        if let Err(error) =
                            email::zoom_sync::flush_pending_deletes(&state.db).await
                        {
                            log::warn!("Zoom delete reaper failed: {error}");
                        }
                        // Reconcile on the pull tick, and deliberately AFTER
                        // `sync_all`: the reconciler's change detection compares
                        // the mirror against what Zoom is believed to hold, so
                        // running it against a just-refreshed mirror is what makes
                        // a reschedule done in Google Calendar's own web UI — the
                        // case no local command observes — reach Zoom at all.
                        if should_pull_calendar {
                            if let Err(error) = email::zoom_sync::reconcile_all(&state.db).await {
                                log::warn!("Zoom reconcile failed: {error}");
                            }
                        }
                    });

                    // 3b. IMAP session ledger summary (gotcha #45)
                    // Every 10 minutes (20 ticks × 30s). The per-failure line in
                    // connect_for_account only fires when something has already
                    // gone wrong, which cannot show the counters were flat
                    // beforehand — and "flat" is the observation that acquits a
                    // session leak. This samples on a fixed cadence so the trend
                    // exists whether or not anything fails.
                    if tick_count % 20 == 0 {
                        let rows = crate::email::imap::ledger_snapshot();
                        if !rows.is_empty() {
                            let detail = rows
                                .iter()
                                .map(|(email, l)| {
                                    format!(
                                        "{}={}o/{}c/{}d/{}a/{}susp/{}unacct",
                                        email,
                                        l.opened,
                                        l.closed_clean,
                                        l.dropped,
                                        l.abandoned,
                                        l.slot_suspect(),
                                        l.unaccounted()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" ");
                            log::info!(
                                "imap ledger [uptime={}s] {}",
                                crate::email::imap::uptime_secs(),
                                detail
                            );
                        }
                    }

                    // 4. Periodic database maintenance
                    // Every 6 hours (720 ticks × 30s): purge cached bodies older than 90 days
                    if tick_count % 720 == 0 {
                        let conn = state.db.safe_lock();
                        match db::messages::purge_old_bodies(&conn, 90) {
                            Ok(0) => {}
                            Ok(n) => log::info!("Purged {} old cached message bodies", n),
                            Err(e) => log::warn!("Body purge failed: {}", e),
                        }
                    }
                    // Every 24 hours (2880 ticks × 30s): VACUUM and optimize
                    if tick_count % 2880 == 0 {
                        let conn = state.db.safe_lock();
                        if let Err(e) = conn.execute_batch("VACUUM; PRAGMA optimize;") {
                            log::warn!("Database maintenance failed: {}", e);
                        } else {
                            log::info!("Database maintenance complete (VACUUM + optimize)");
                        }
                    }

                    // 5. Backend safety-net sync
                    // Every 5 minutes (10 ticks × 30s): kick off a full sync if
                    // nothing else has. Spawned on a separate task so the rest
                    // of this 30s tick (snooze, scheduled sends, follow-ups,
                    // maintenance) isn't delayed by IMAP I/O.
                    if tick_count % 10 == 0 {
                        let app_for_sync = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let Some(state) = app_for_sync.try_state::<AppState>() else {
                                return;
                            };
                            log::info!("Backend safety-net sync: starting");
                            match commands::messages::sync_all_inboxes_inner(
                                state.inner(),
                                &app_for_sync,
                            )
                            .await
                            {
                                Ok(result) => {
                                    log::info!(
                                        "Backend safety-net sync: done ({} new, {} updated)",
                                        result.new_count,
                                        result.updated_count
                                    );
                                }
                                Err(e) => {
                                    log::warn!("Backend safety-net sync: failed: {}", e);
                                }
                            }
                        });
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::Reopen { .. } => {
                    // Re-show window when dock icon is clicked
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                tauri::RunEvent::Exit => {
                    // Delete lock file so background daemon can take over
                    if let Ok(app_dir) = app_handle.path().app_data_dir() {
                        let _ = fs::remove_file(app_dir.join("app.lock"));
                    }
                    // Best-effort: remove the MCP bridge socket. Harmless either
                    // way — a stale socket is cleared at the next bind.
                    let _ = fs::remove_file(mcp::bridge::socket_path());
                }
                _ => {}
            }
        });
}
