# IPC Commands

All commands defined as `#[tauri::command]` in Rust, invoked from TypeScript via `@tauri-apps/api/core` invoke().

## Account Commands (commands/accounts.rs)

```rust
#[tauri::command]
async fn add_account(state: State<'_, AppState>, provider: String, email: String) -> Result<Account, AppError>

#[tauri::command]
async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, AppError>

#[tauri::command]
async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<(), AppError>

#[tauri::command]
async fn test_connection(state: State<'_, AppState>, account_id: String) -> Result<ConnectionStatus, AppError>
```

## Auth Commands (commands/auth.rs)

```rust
#[tauri::command]
async fn start_oauth2(app: AppHandle, provider: String) -> Result<String, AppError>
// Returns auth URL to open in browser

#[tauri::command]
async fn handle_oauth2_callback(state: State<'_, AppState>, code: String, state_param: String) -> Result<Account, AppError>
// Exchanges code for tokens, stores in Keychain, returns new account

#[tauri::command]
async fn refresh_access_token(state: State<'_, AppState>, account_id: String) -> Result<(), AppError>
```

## Folder Commands (commands/folders.rs)

```rust
#[tauri::command]
async fn list_folders(state: State<'_, AppState>, account_id: String) -> Result<Vec<Folder>, AppError>

#[tauri::command]
async fn sync_folders(state: State<'_, AppState>, account_id: String) -> Result<Vec<Folder>, AppError>
// Fetches from IMAP, updates cache, returns updated list
```

## Message Commands (commands/messages.rs)

```rust
#[tauri::command]
async fn fetch_messages(state: State<'_, AppState>, account_id: String, folder: String, page: u32, page_size: u32) -> Result<MessagePage, AppError>
// Returns paginated message list (headers only, from cache or IMAP)

#[tauri::command]
async fn fetch_message_body(state: State<'_, AppState>, account_id: String, folder: String, uid: u32) -> Result<MessageDetail, AppError>
// Returns full message with sanitized HTML body

#[tauri::command]
async fn sync_folder(state: State<'_, AppState>, account_id: String, folder: String) -> Result<SyncResult, AppError>
// Incremental sync, returns new/changed count

#[tauri::command]
async fn mark_as_read(state: State<'_, AppState>, account_id: String, folder: String, uids: Vec<u32>) -> Result<(), AppError>

#[tauri::command]
async fn mark_as_unread(state: State<'_, AppState>, account_id: String, folder: String, uids: Vec<u32>) -> Result<(), AppError>
```

## Tauri Events (Backend → Frontend)

```rust
app.emit("sync-progress", SyncProgress { account_id, folder, current, total });
app.emit("sync-complete", SyncComplete { account_id, folder, new_count, updated_count });
app.emit("connection-status", ConnectionStatus { account_id, status: "connected" | "disconnected" | "error" });
app.emit("account-added", Account { ... });
```

## TypeScript Wrappers (src/lib/tauri.ts)

```typescript
import { invoke } from '@tauri-apps/api/core';
import type { Account, Folder, MessagePage, MessageDetail, SyncResult, ConnectionStatus } from '@/types/email';

export const api = {
  accounts: {
    list: () => invoke<Account[]>('list_accounts'),
    add: (provider: string, email: string) => invoke<Account>('add_account', { provider, email }),
    remove: (accountId: string) => invoke<void>('remove_account', { accountId }),
    testConnection: (accountId: string) => invoke<ConnectionStatus>('test_connection', { accountId }),
  },
  auth: {
    startOAuth2: (provider: string) => invoke<string>('start_oauth2', { provider }),
    handleCallback: (code: string, stateParam: string) => invoke<Account>('handle_oauth2_callback', { code, stateParam }),
  },
  folders: {
    list: (accountId: string) => invoke<Folder[]>('list_folders', { accountId }),
    sync: (accountId: string) => invoke<Folder[]>('sync_folders', { accountId }),
  },
  messages: {
    fetch: (accountId: string, folder: string, page: number, pageSize: number) =>
      invoke<MessagePage>('fetch_messages', { accountId, folder, page, pageSize }),
    fetchBody: (accountId: string, folder: string, uid: number) =>
      invoke<MessageDetail>('fetch_message_body', { accountId, folder, uid }),
    sync: (accountId: string, folder: string) =>
      invoke<SyncResult>('sync_folder', { accountId, folder }),
    markRead: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>('mark_as_read', { accountId, folder, uids }),
    markUnread: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>('mark_as_unread', { accountId, folder, uids }),
  },
} as const;
```

## TypeScript Types (src/types/email.ts)

```typescript
export interface Account {
  id: string;
  email: string;
  displayName: string | null;
  provider: 'gmail' | 'icloud' | 'outlook';
  color: string | null;
  isActive: boolean;
}

export interface Folder {
  id: number;
  accountId: string;
  name: string;
  displayName: string | null;
  folderType: 'inbox' | 'sent' | 'drafts' | 'trash' | 'spam' | 'archive' | 'other';
  totalCount: number;
  unreadCount: number;
}

export interface MessageSummary {
  uid: number;
  subject: string | null;
  fromName: string | null;
  fromEmail: string;
  date: string;
  snippet: string | null;
  isRead: boolean;
  isFlagged: boolean;
  hasAttachments: boolean;
}

export interface MessagePage {
  messages: MessageSummary[];
  total: number;
  page: number;
  pageSize: number;
  hasMore: boolean;
}

export interface MessageDetail {
  uid: number;
  subject: string | null;
  fromName: string | null;
  fromEmail: string;
  toList: EmailAddress[];
  ccList: EmailAddress[];
  date: string;
  plainText: string | null;
  sanitizedHtml: string | null;
  attachments: AttachmentMeta[];
  isRead: boolean;
  isFlagged: boolean;
}

export interface EmailAddress {
  name: string | null;
  email: string;
}

export interface AttachmentMeta {
  filename: string | null;
  contentType: string;
  sizeBytes: number;
  isInline: boolean;
}

export interface SyncResult {
  newCount: number;
  updatedCount: number;
}

export interface ConnectionStatus {
  accountId: string;
  status: 'connected' | 'disconnected' | 'error';
}
```

## AppState (src-tauri/src/lib.rs)

```rust
pub struct AppState {
    /// `Arc` so a detached background task can hold the connection without an
    /// `AppHandle`. Every `state.db.safe_lock()` call site is unaffected —
    /// `Arc<Mutex<T>>` derefs to `Mutex<T>`.
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub db_path: PathBuf,
    pub pending_sends: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    pub sync_in_flight: AtomicBool,
}
```

Managed state registered in the Tauri builder's setup hook:
```rust
app.manage(AppState {
    db: Arc::new(Mutex::new(conn)),
    db_path,
    pending_sends: Mutex::new(HashMap::new()),
    sync_in_flight: AtomicBool::new(false),
});
```

Lock through `LockExt::safe_lock`, never `.lock().unwrap()` — a thread that
panicked mid-sync should not poison the mutex into cascade-crashing the app.

**Code outside the app crate cannot reach this.** `cxmail-email` does not link
Tauri, so `email::oauth2` and `email::gcal_invite` take a
`cxmail_core::AppCtx` — an event sink plus the same `Arc<Mutex<Connection>>` —
built by `crate::app_ctx(&app_handle)`.
