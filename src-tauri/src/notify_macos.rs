//! Native macOS notifications via UNUserNotificationCenter.
//!
//! Sends notifications with `userInfo` payloads (account_id, uid) and registers
//! a delegate to handle click events. When a notification is clicked, emits an
//! `open-email-from-notification` Tauri event so the frontend can navigate to
//! the specific email.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSNumber, NSString};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};

// Raw ObjC runtime FFI for class creation (bypasses MethodImplementation trait bounds)
extern "C" {
    fn objc_allocateClassPair(
        superclass: *const AnyClass,
        name: *const std::ffi::c_char,
        extra_bytes: usize,
    ) -> *mut AnyClass;
    fn objc_registerClassPair(cls: *mut AnyClass);
    fn class_addMethod(
        cls: *mut AnyClass,
        sel: Sel,
        imp: *const std::ffi::c_void,
        types: *const std::ffi::c_char,
    ) -> Bool;
    fn class_addProtocol(cls: *mut AnyClass, protocol: *const std::ffi::c_void) -> Bool;
    fn objc_getProtocol(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
}

/// Stored app handle for use in the ObjC delegate callback.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Category carrying the Approve/Deny buttons for a calendar-invite approval.
/// Registered once in [`init`]; a notification only shows action buttons if its
/// `categoryIdentifier` matches a registered category.
const APPROVAL_CATEGORY: &str = "cxmail.calendar-approval";
const ACTION_APPROVE: &str = "cxmail.approve";
const ACTION_DENY: &str = "cxmail.deny";
/// Category carrying the "Open in Claude" button on a new-mail notification.
const NEW_MAIL_CATEGORY: &str = "cxmail.new-mail";
const ACTION_OPEN_IN_CLAUDE: &str = "cxmail.open-in-claude";
/// macOS sends this identifier when the notification body itself is clicked.
const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";

// UNNotificationActionOptions
const ACTION_OPTION_DESTRUCTIVE: usize = 1 << 1;
// UNNotificationPresentationOptions — Sound | List | Banner.
const PRESENT_SOUND_LIST_BANNER: usize = (1 << 1) | (1 << 3) | (1 << 4);

/// Register the notification delegate class and set it on UNUserNotificationCenter.
/// Must be called once during app setup.
pub fn init(app: &AppHandle) {
    APP_HANDLE.get_or_init(|| app.clone());

    unsafe {
        let cls = register_delegate_class();
        let delegate: Retained<AnyObject> = msg_send![cls, new];

        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![&*center, setDelegate: &*delegate];

        // Leak the delegate so it lives for the app lifetime.
        std::mem::forget(delegate);

        register_notification_categories(&center);

        // Request notification authorization
        let options: usize = 0x04 | 0x02 | 0x01; // alert | sound | badge
        // Log the outcome. If the user has denied notifications, calendar
        // approvals silently never reach Notification Center, and this line is
        // the difference between a one-line answer and an afternoon (the
        // spellcheck lesson, gotcha #27).
        let block = block2::RcBlock::new(move |granted: Bool, _error: *const AnyObject| {
            if granted.as_bool() {
                log::info!("Notification authorization granted");
            } else {
                log::warn!(
                    "Notification authorization DENIED — calendar approval notifications will not appear"
                );
            }
        });
        let _: () = msg_send![
            &*center,
            requestAuthorizationWithOptions: options,
            completionHandler: &*block
        ];
    }
}

/// Register every notification category the app uses: Approve/Deny on a
/// calendar-invite approval, and Open in Claude on a new-mail notification.
///
/// **They must be registered together.** `setNotificationCategories:` replaces
/// the entire set rather than adding to it, so registering one category alone
/// silently unregisters the other — and an unregistered category's buttons
/// simply never render, with nothing logged and no error anywhere.
///
/// Buttons only render when the user's notification style for CXMail is
/// **Alerts**; on **Banners** the notification self-dismisses after a few
/// seconds and hides the actions behind a hover. That is a per-user System
/// Settings choice we cannot set from here — the dock bounce in `lib.rs` is the
/// signal that survives either setting, and the in-window approval card remains
/// the authoritative surface. New mail keeps exactly one action for the same
/// reason: macOS promotes a lone action to a visible button and pushes every
/// action after the first behind an "Options" menu, which on a banner you have
/// a few seconds to hit is the difference between usable and theoretical.
unsafe fn register_notification_categories(center: &AnyObject) {
    let approve_id = NSString::from_str(ACTION_APPROVE);
    let approve_title = NSString::from_str("Approve & Send");
    let approve: Retained<AnyObject> = msg_send![
        class!(UNNotificationAction),
        actionWithIdentifier: &*approve_id,
        title: &*approve_title,
        options: 0usize
    ];

    let deny_id = NSString::from_str(ACTION_DENY);
    let deny_title = NSString::from_str("Deny");
    let deny: Retained<AnyObject> = msg_send![
        class!(UNNotificationAction),
        actionWithIdentifier: &*deny_id,
        title: &*deny_title,
        options: ACTION_OPTION_DESTRUCTIVE
    ];

    let actions: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
    let _: () = msg_send![&*actions, addObject: &*approve];
    let _: () = msg_send![&*actions, addObject: &*deny];
    let intents: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];

    let category_id = NSString::from_str(APPROVAL_CATEGORY);
    let category: Retained<AnyObject> = msg_send![
        class!(UNNotificationCategory),
        categoryWithIdentifier: &*category_id,
        actions: &*actions,
        intentIdentifiers: &*intents,
        options: 0usize
    ];

    // New mail: a single "Open in Claude" action. No `.foreground` option —
    // the point of the handoff is landing in Ghostty, not raising CXMail.
    let claude_id = NSString::from_str(ACTION_OPEN_IN_CLAUDE);
    let claude_title = NSString::from_str("Open in Claude");
    let claude: Retained<AnyObject> = msg_send![
        class!(UNNotificationAction),
        actionWithIdentifier: &*claude_id,
        title: &*claude_title,
        options: 0usize
    ];

    let mail_actions: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
    let _: () = msg_send![&*mail_actions, addObject: &*claude];

    let mail_category_id = NSString::from_str(NEW_MAIL_CATEGORY);
    let mail_category: Retained<AnyObject> = msg_send![
        class!(UNNotificationCategory),
        categoryWithIdentifier: &*mail_category_id,
        actions: &*mail_actions,
        intentIdentifiers: &*intents,
        options: 0usize
    ];

    let category_array: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
    let _: () = msg_send![&*category_array, addObject: &*category];
    let _: () = msg_send![&*category_array, addObject: &*mail_category];
    let categories: Retained<AnyObject> =
        msg_send![class!(NSSet), setWithArray: &*category_array];
    let _: () = msg_send![center, setNotificationCategories: &*categories];
    log::info!("Registered notification categories: calendar approval, new mail");
}

/// Show the Approve/Deny notification for a pending calendar-invite approval.
///
/// `approval_id` round-trips through `userInfo` so the delegate can answer with
/// the same `mcp-approval-response` event the in-window card emits — one
/// decision path, whichever surface the user acts on.
pub fn show_approval_notification(approval_id: &str, title: &str, body: &str) {
    unsafe {
        let content: Retained<AnyObject> = msg_send![class!(UNMutableNotificationContent), new];
        let ns_title = NSString::from_str(title);
        let ns_body = NSString::from_str(body);
        let ns_category = NSString::from_str(APPROVAL_CATEGORY);
        let _: () = msg_send![&*content, setTitle: &*ns_title];
        let _: () = msg_send![&*content, setBody: &*ns_body];
        let _: () = msg_send![&*content, setCategoryIdentifier: &*ns_category];

        let user_info: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];
        let key_approval = NSString::from_str("approval_id");
        let val_approval = NSString::from_str(approval_id);
        let _: () = msg_send![&*user_info, setObject: &*val_approval, forKey: &*key_approval];
        let _: () = msg_send![&*content, setUserInfo: &*user_info];

        let default_sound: Retained<AnyObject> =
            msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![&*content, setSound: &*default_sound];

        // Deterministic identifier so the notification can be withdrawn once the
        // decision is made on another surface.
        let request_id = NSString::from_str(&approval_request_id(approval_id));
        let trigger: *const AnyObject = std::ptr::null();
        let request: Retained<AnyObject> = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: &*request_id,
            content: &*content,
            trigger: trigger
        ];

        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let error_block = block2::RcBlock::new(move |_error: *const AnyObject| {});
        let _: () = msg_send![
            &*center,
            addNotificationRequest: &*request,
            withCompletionHandler: &*error_block
        ];
    }
}

/// Withdraw a delivered approval notification — the decision was made in the
/// app window, so leaving a live Approve button in Notification Center would
/// invite a second decision on an approval that no longer exists.
pub fn clear_approval_notification(approval_id: &str) {
    unsafe {
        let request_id = NSString::from_str(&approval_request_id(approval_id));
        let ids: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
        let _: () = msg_send![&*ids, addObject: &*request_id];
        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![&*center, removeDeliveredNotificationsWithIdentifiers: &*ids];
        let _: () = msg_send![&*center, removePendingNotificationRequestsWithIdentifiers: &*ids];
    }
}

fn approval_request_id(approval_id: &str) -> String {
    format!("approval-{approval_id}")
}

/// Show a native macOS notification with account_id, folder and uid in userInfo.
///
/// `folder` is not decoration: the "Open in Claude" action resolves the message
/// through `(account_id, folder, uid)`, and no two of those three address a
/// message on their own.
pub fn show_notification(account_id: &str, folder: &str, uid: u32, title: &str, body: &str) {
    unsafe {
        let content: Retained<AnyObject> =
            msg_send![class!(UNMutableNotificationContent), new];
        let ns_title = NSString::from_str(title);
        let ns_body = NSString::from_str(body);
        let ns_category = NSString::from_str(NEW_MAIL_CATEGORY);
        let _: () = msg_send![&*content, setTitle: &*ns_title];
        let _: () = msg_send![&*content, setBody: &*ns_body];
        // Without a categoryIdentifier matching a REGISTERED category, macOS
        // renders the notification with no actions at all and says nothing.
        let _: () = msg_send![&*content, setCategoryIdentifier: &*ns_category];

        // Build userInfo dict with account_id, folder and uid
        let user_info: Retained<AnyObject> =
            msg_send![class!(NSMutableDictionary), new];
        let key_account = NSString::from_str("account_id");
        let val_account = NSString::from_str(account_id);
        let _: () = msg_send![&*user_info, setObject: &*val_account, forKey: &*key_account];
        let key_folder = NSString::from_str("folder");
        let val_folder = NSString::from_str(folder);
        let _: () = msg_send![&*user_info, setObject: &*val_folder, forKey: &*key_folder];
        let key_uid = NSString::from_str("uid");
        let val_uid = NSNumber::new_u32(uid);
        let _: () = msg_send![&*user_info, setObject: &*val_uid, forKey: &*key_uid];
        let _: () = msg_send![&*content, setUserInfo: &*user_info];

        // Sound
        let default_sound: Retained<AnyObject> =
            msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![&*content, setSound: &*default_sound];

        // Create request with unique identifier
        let request_id = NSString::from_str(&format!("{}-{}-{}", account_id, uid, new_uuid()));
        let trigger: *const AnyObject = std::ptr::null();
        let request: Retained<AnyObject> = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: &*request_id,
            content: &*content,
            trigger: trigger
        ];

        // Add to notification center
        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let error_block = block2::RcBlock::new(move |_error: *const AnyObject| {});
        let _: () = msg_send![
            &*center,
            addNotificationRequest: &*request,
            withCompletionHandler: &*error_block
        ];
    }
}

/// Show a summary notification (no per-message userInfo).
pub fn show_summary_notification(count: usize) {
    unsafe {
        let content: Retained<AnyObject> =
            msg_send![class!(UNMutableNotificationContent), new];
        let ns_title = NSString::from_str("CXMail");
        let ns_body = NSString::from_str(&format!("{} new messages", count));
        let _: () = msg_send![&*content, setTitle: &*ns_title];
        let _: () = msg_send![&*content, setBody: &*ns_body];

        let default_sound: Retained<AnyObject> =
            msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![&*content, setSound: &*default_sound];

        let request_id = NSString::from_str(&format!("summary-{}", new_uuid()));
        let trigger: *const AnyObject = std::ptr::null();
        let request: Retained<AnyObject> = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: &*request_id,
            content: &*content,
            trigger: trigger
        ];

        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let error_block = block2::RcBlock::new(move |_error: *const AnyObject| {});
        let _: () = msg_send![
            &*center,
            addNotificationRequest: &*request,
            withCompletionHandler: &*error_block
        ];
    }
}

/// Show a plain notification with no actions and no userInfo.
///
/// The reporting channel for a background notification action that failed:
/// there is no window guaranteed to be open, and the banner the user clicked
/// is already gone.
pub fn show_plain_notification(title: &str, body: &str) {
    unsafe {
        let content: Retained<AnyObject> = msg_send![class!(UNMutableNotificationContent), new];
        let ns_title = NSString::from_str(title);
        let ns_body = NSString::from_str(body);
        let _: () = msg_send![&*content, setTitle: &*ns_title];
        let _: () = msg_send![&*content, setBody: &*ns_body];

        let default_sound: Retained<AnyObject> =
            msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![&*content, setSound: &*default_sound];

        let request_id = NSString::from_str(&format!("plain-{}", new_uuid()));
        let trigger: *const AnyObject = std::ptr::null();
        let request: Retained<AnyObject> = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: &*request_id,
            content: &*content,
            trigger: trigger
        ];

        let center: Retained<AnyObject> =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let error_block = block2::RcBlock::new(move |_error: *const AnyObject| {});
        let _: () = msg_send![
            &*center,
            addNotificationRequest: &*request,
            withCompletionHandler: &*error_block
        ];
    }
}

fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Register an ObjC class conforming to UNUserNotificationCenterDelegate
/// using raw runtime functions.
fn register_delegate_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSObject) as *const AnyClass;
        let cls = objc_allocateClassPair(
            superclass,
            c"CXMailNotificationDelegate".as_ptr(),
            0,
        );
        assert!(!cls.is_null(), "Failed to create notification delegate class");

        // Add UNUserNotificationCenterDelegate protocol
        let proto = objc_getProtocol(c"UNUserNotificationCenterDelegate".as_ptr());
        if !proto.is_null() {
            class_addProtocol(cls, proto);
        }

        // Add didReceiveNotificationResponse handler
        // ObjC type encoding: v@:@@@ (void, self, sel, id, id, id)
        class_addMethod(
            cls,
            sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
            did_receive_response as *const std::ffi::c_void,
            c"v@:@@@".as_ptr(),
        );

        // Add willPresentNotification handler
        class_addMethod(
            cls,
            sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
            will_present_notification as *const std::ffi::c_void,
            c"v@:@@@".as_ptr(),
        );

        objc_registerClassPair(cls);
        &*cls
    })
}

/// Called when user clicks a notification.
extern "C" fn did_receive_response(
    _this: *const AnyObject,
    _cmd: Sel,
    _center: *const AnyObject,
    response: *const AnyObject,
    completion: *const AnyObject, // ObjC block
) {
    if response.is_null() || completion.is_null() {
        return;
    }

    unsafe {
        // response.notification.request.content.userInfo
        let notification: Retained<AnyObject> = msg_send![response, notification];
        let request: Retained<AnyObject> = msg_send![&*notification, request];
        let content: Retained<AnyObject> = msg_send![&*request, content];
        let user_info: Retained<AnyObject> = msg_send![&*content, userInfo];

        // Calendar approvals are answered from the notification itself. Handle
        // them before the mail-click path: their userInfo carries approval_id
        // instead of account_id/uid, so they would otherwise fall through and
        // do nothing at all.
        let key_approval = NSString::from_str("approval_id");
        let approval_obj: *const AnyObject = msg_send![&*user_info, objectForKey: &*key_approval];
        if !approval_obj.is_null() {
            let approval_nsstr: &NSString = &*(approval_obj as *const NSString);
            let approval_id = approval_nsstr.to_string();

            let action_obj: *const AnyObject = msg_send![response, actionIdentifier];
            let action = if action_obj.is_null() {
                String::new()
            } else {
                (*(action_obj as *const NSString)).to_string()
            };

            if let Some(app) = APP_HANDLE.get() {
                let decision = match action.as_str() {
                    ACTION_APPROVE => Some("approved"),
                    ACTION_DENY => Some("denied"),
                    // Clicking the notification body opens the app so the user
                    // can read the full recipient list before deciding — the
                    // buttons are a shortcut, not the only way through.
                    _ => {
                        if action == DEFAULT_ACTION {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        None
                    }
                };
                if let Some(decision) = decision {
                    log::info!("Calendar approval {approval_id} answered from notification: {decision}");
                    // Same event the in-window card emits, so delivery has a
                    // single implementation regardless of which surface acted.
                    let _ = app.emit(
                        "mcp-approval-response",
                        serde_json::json!({ "id": approval_id, "decision": decision }),
                    );
                }
            }

            let invoke_ptr = *(completion.byte_add(16) as *const *const std::ffi::c_void);
            let invoke: extern "C" fn(*const AnyObject) = std::mem::transmute(invoke_ptr);
            invoke(completion);
            return;
        }

        let key_account = NSString::from_str("account_id");
        let key_folder = NSString::from_str("folder");
        let key_uid = NSString::from_str("uid");

        let account_obj: *const AnyObject = msg_send![&*user_info, objectForKey: &*key_account];
        let folder_obj: *const AnyObject = msg_send![&*user_info, objectForKey: &*key_folder];
        let uid_obj: *const AnyObject = msg_send![&*user_info, objectForKey: &*key_uid];

        if !account_obj.is_null() && !uid_obj.is_null() {
            let account_nsstr: &NSString = &*(account_obj as *const NSString);
            let account_id = account_nsstr.to_string();
            let uid: u32 = msg_send![uid_obj, unsignedIntValue];
            // A notification delivered by a build that predates the folder key
            // can still be sitting in Notification Center after an upgrade.
            // New-mail notifications are inbox-only, so this default is the same
            // answer the sender would have written — not a guess papering over a
            // missing value.
            let folder = if folder_obj.is_null() {
                "INBOX".to_string()
            } else {
                (*(folder_obj as *const NSString)).to_string()
            };

            let action_obj: *const AnyObject = msg_send![response, actionIdentifier];
            let action = if action_obj.is_null() {
                String::new()
            } else {
                (*(action_obj as *const NSString)).to_string()
            };

            if action == ACTION_OPEN_IN_CLAUDE {
                log::info!(
                    "Open in Claude from notification: account_id={}, folder={}, uid={}",
                    account_id,
                    folder,
                    uid
                );
                if let Some(app) = APP_HANDLE.get() {
                    let app = app.clone();
                    // This callback runs on the main thread, and the handoff takes
                    // the DB mutex, fetches attachments over IMAP and spawns
                    // Ghostty. Doing that inline stalls the UI (gotchas #11, #20).
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<crate::AppState>();
                        match crate::commands::claude_handoff::open_email_in_claude(
                            app.clone(),
                            state,
                            account_id,
                            folder,
                            uid,
                        )
                        .await
                        {
                            Ok(handoff) => log::info!(
                                "Open in Claude from notification landed in {}",
                                handoff.working_dir
                            ),
                            Err(e) => {
                                // A background action leaves no window to paint an
                                // error into, so this would otherwise fail in total
                                // silence: the banner is gone and nothing happened.
                                log::warn!("Open in Claude from notification failed: {e}");
                                show_plain_notification(
                                    "Open in Claude failed",
                                    &e.to_string(),
                                );
                            }
                        }
                    });
                }
            } else {
                log::info!(
                    "Notification clicked: account_id={}, uid={}",
                    account_id,
                    uid
                );

                if let Some(app) = APP_HANDLE.get() {
                    // Show and focus the main window
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }

                    // Emit event so frontend can navigate to the email
                    let _ = app.emit(
                        "open-email-from-notification",
                        serde_json::json!({
                            "account_id": account_id,
                            "uid": uid
                        }),
                    );
                }
            }
        }

        // Call the completion handler block.
        // ObjC blocks have invoke pointer at offset 16 (after isa + flags).
        // Layout: [isa:8][flags:4][reserved:4][invoke:8][...]
        let invoke_ptr = *(completion.byte_add(16) as *const *const std::ffi::c_void);
        let invoke: extern "C" fn(*const AnyObject) = std::mem::transmute(invoke_ptr);
        invoke(completion);
    }
}

/// Called when a notification arrives while the app is in the foreground.
/// We return 0 (none) so foreground notifications are suppressed — the user
/// already sees new mail in the inbox.
extern "C" fn will_present_notification(
    _this: *const AnyObject,
    _cmd: Sel,
    _center: *const AnyObject,
    notification: *const AnyObject,
    completion: *const AnyObject, // ObjC block
) {
    if completion.is_null() {
        return;
    }
    unsafe {
        // New-mail notifications stay suppressed in the foreground — the inbox
        // already shows them. A calendar approval is the opposite case: it is a
        // request for a decision, and "the app is frontmost" does NOT mean the
        // user is looking at the window this app just rendered a card into.
        let mut options = 0usize;
        if !notification.is_null() {
            let request: Retained<AnyObject> = msg_send![notification, request];
            let content: Retained<AnyObject> = msg_send![&*request, content];
            let category_obj: *const AnyObject = msg_send![&*content, categoryIdentifier];
            if !category_obj.is_null()
                && (*(category_obj as *const NSString)).to_string() == APPROVAL_CATEGORY
            {
                options = PRESENT_SOUND_LIST_BANNER;
            }
        }
        let invoke_ptr = *(completion.byte_add(16) as *const *const std::ffi::c_void);
        let invoke: extern "C" fn(*const AnyObject, usize) = std::mem::transmute(invoke_ptr);
        invoke(completion, options);
    }
}
