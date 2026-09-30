//! CXMail's native Dock context menu. AppKit asks the app delegate for this
//! menu on right click; its callback must stay on the main thread and must not
//! wait for SQLite or the Keychain. A worker refreshes a small local snapshot.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::NSString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::{db, AppState};

const MAX_ITEMS: usize = 8;
static APP: OnceLock<AppHandle> = OnceLock::new();
static ITEMS: OnceLock<Mutex<Vec<DockMail>>> = OnceLock::new();
static DISPLAYED: OnceLock<Mutex<Vec<DockMail>>> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct DockMail {
    title: String,
    account_id: String,
    folder_name: String,
    uid: u32,
}

fn message_selectors() -> [Sel; MAX_ITEMS] {
    [
        sel!(cxmailDockMessage0:),
        sel!(cxmailDockMessage1:),
        sel!(cxmailDockMessage2:),
        sel!(cxmailDockMessage3:),
        sel!(cxmailDockMessage4:),
        sel!(cxmailDockMessage5:),
        sel!(cxmailDockMessage6:),
        sel!(cxmailDockMessage7:),
    ]
}

extern "C" {
    fn object_getClass(obj: *const AnyObject) -> *mut AnyClass;
    fn class_addMethod(
        cls: *mut AnyClass,
        selector: Sel,
        imp: *const std::ffi::c_void,
        types: *const std::ffi::c_char,
    ) -> Bool;
}

/// Install on Tauri's existing delegate, preserving its launch, URL and reopen
/// handlers. Called once from `.setup()` on the AppKit main thread.
pub fn install(app: &AppHandle) {
    APP.get_or_init(|| app.clone());
    ITEMS.get_or_init(|| Mutex::new(Vec::new()));
    DISPLAYED.get_or_init(|| Mutex::new(Vec::new()));

    unsafe {
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if ns_app.is_null() {
            log::warn!("dock menu: NSApplication unavailable");
            return;
        }
        let delegate: *mut AnyObject = msg_send![ns_app, delegate];
        if delegate.is_null() {
            log::warn!("dock menu: application delegate unavailable");
            return;
        }
        let cls = object_getClass(delegate);
        if cls.is_null() {
            log::warn!("dock menu: application delegate class unavailable");
            return;
        }
        for selector in message_selectors().into_iter().chain([
            sel!(cxmailDockNeedsYou:),
            sel!(cxmailDockNewMessage:),
            sel!(cxmailDockCheckMail:),
        ]) {
            if !class_addMethod(
                cls,
                selector,
                dock_action as *const std::ffi::c_void,
                c"v@:@".as_ptr(),
            )
            .as_bool()
            {
                log::warn!("dock menu: failed to register action selector");
                return;
            }
        }
        if !class_addMethod(
            cls,
            sel!(applicationDockMenu:),
            application_dock_menu as *const std::ffi::c_void,
            c"@@:@".as_ptr(),
        )
        .as_bool()
        {
            log::warn!("dock menu: failed to register applicationDockMenu:");
            return;
        }
        let installed: Bool = msg_send![delegate, respondsToSelector: sel!(applicationDockMenu:)];
        log::info!(
            "dock menu: installed on application delegate (responds={})",
            installed.as_bool()
        );
    }

    let (refresh_tx, refresh_rx) = std::sync::mpsc::channel::<()>();
    for event in ["sync-account-done", "idle-new-mail"] {
        let tx = refresh_tx.clone();
        app.listen(event, move |_| {
            let _ = tx.send(());
        });
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        refresh(&app);
        let _ = refresh_rx.recv_timeout(Duration::from_secs(90));
    });
}

fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let result = state.open_read_conn().and_then(|conn| {
        db::needs_you::list(
            &conn,
            100,
            crate::email::triage_gate::effective_mode().may_surface(),
        )
    });
    let mut items = match result {
        Ok(items) => items,
        Err(error) => {
            log::warn!("dock menu: could not refresh Needs You: {error}");
            return;
        }
    };

    // A stored triage urgency is the strongest signal. Without one, show the
    // unread and newest actionable mail first. Needs You already excludes muted,
    // dismissed, stale and hidden-account messages.
    items.sort_by(|a, b| {
        b.urgency
            .unwrap_or(0)
            .cmp(&a.urgency.unwrap_or(0))
            .then_with(|| a.is_read.cmp(&b.is_read))
            .then_with(|| b.date.cmp(&a.date))
    });
    let snapshot = items
        .into_iter()
        .take(MAX_ITEMS)
        .map(|item| {
            let sender = item
                .from_name
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(item.from_email.as_deref())
                .unwrap_or("Unknown sender");
            let subject = item
                .subject
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("(no subject)");
            let urgency = match item.urgency {
                Some(4) => "Now · ",
                Some(3) => "Today · ",
                _ => "",
            };
            DockMail {
                title: format!(
                    "{}{}{} — {}",
                    if item.is_read { "" } else { "● " },
                    urgency,
                    compact(sender, 24),
                    compact(subject, 42)
                ),
                account_id: item.account_id,
                folder_name: item.folder_name,
                uid: item.uid,
            }
        })
        .collect();
    if let Some(cache) = ITEMS.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = snapshot;
            READY.store(true, Ordering::Release);
        }
    }
}

fn compact(value: &str, max: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    format!(
        "{}…",
        trimmed
            .chars()
            .take(max.saturating_sub(1))
            .collect::<String>()
    )
}

unsafe extern "C" fn application_dock_menu(
    delegate: *mut AnyObject,
    _selector: Sel,
    _application: *mut AnyObject,
) -> *mut AnyObject {
    let menu: Retained<AnyObject> = msg_send![class!(NSMenu), new];
    let snapshot = ITEMS
        .get()
        .and_then(|cache| cache.lock().ok())
        .map(|items| items.clone())
        .unwrap_or_default();
    if let Some(displayed) = DISPLAYED.get() {
        if let Ok(mut guard) = displayed.lock() {
            *guard = snapshot.clone();
        }
    }

    let header = add_item(&menu, delegate, "Needs You", sel!(cxmailDockNeedsYou:));
    let _: () = msg_send![header, setEnabled: Bool::NO];
    if snapshot.is_empty() {
        let label = if READY.load(Ordering::Acquire) {
            "Nothing needs your attention"
        } else {
            "Loading urgent mail…"
        };
        let empty = add_item(&menu, delegate, label, sel!(cxmailDockNeedsYou:));
        let _: () = msg_send![empty, setEnabled: Bool::NO];
    } else {
        for (index, item) in snapshot.into_iter().enumerate() {
            add_item(&menu, delegate, &item.title, message_selectors()[index]);
        }
    }
    let separator: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![&*menu, addItem: separator];
    add_item(&menu, delegate, "Open Needs You", sel!(cxmailDockNeedsYou:));
    add_item(&menu, delegate, "New Message", sel!(cxmailDockNewMessage:));
    add_item(
        &menu,
        delegate,
        "Check for New Mail",
        sel!(cxmailDockCheckMail:),
    );

    // Dock actions arrive with a nil sender, so each row has a distinct
    // selector and resolves against the snapshot captured when this menu was
    // built. AppKit's delegate method returns an autoreleased menu.
    let ptr = Retained::into_raw(menu) as *mut AnyObject;
    let _: *mut AnyObject = msg_send![ptr, autorelease];
    ptr
}

unsafe fn add_item(
    menu: &AnyObject,
    delegate: *mut AnyObject,
    title: &str,
    action: Sel,
) -> *mut AnyObject {
    let title = NSString::from_str(title);
    let key = NSString::from_str("");
    let item: *mut AnyObject = msg_send![menu, addItemWithTitle: &*title,
        action: action, keyEquivalent: &*key];
    let _: () = msg_send![item, setTarget: delegate];
    item
}

unsafe extern "C" fn dock_action(
    _delegate: *mut AnyObject,
    selector: Sel,
    _sender: *mut AnyObject,
) {
    let Some(app) = APP.get() else { return };

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(index) = message_selectors().iter().position(|&s| s == selector) {
        let item = DISPLAYED
            .get()
            .and_then(|cache| cache.lock().ok())
            .and_then(|items| items.get(index).cloned());
        if let Some(DockMail {
            account_id,
            folder_name,
            uid,
            ..
        }) = item
        {
            let _ = app.emit(
                "dock-open-message",
                serde_json::json!({
                    "account_id": account_id, "folder_name": folder_name, "uid": uid,
                }),
            );
        }
    } else if selector == sel!(cxmailDockNeedsYou:) {
        let _ = app.emit("dock-open-needs-you", ());
    } else if selector == sel!(cxmailDockNewMessage:) {
        let _ = app.emit("dock-new-message", ());
    } else if selector == sel!(cxmailDockCheckMail:) {
        let _ = app.emit("check-mail", ());
    }
}
