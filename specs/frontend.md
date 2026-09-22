# Frontend Components

React 19 + TypeScript + Tailwind CSS 4 + Radix UI + Zustand.

## Design System

- **Dark theme**: bg `#1e1e1e`, surface `#2d2d2d`, border `#3d3d3d`, text `#e0e0e0`, accent `#0a84ff`
- **Font**: system font stack (-apple-system, BlinkMacSystemFont)
- **Icons**: Lucide React
- **Animations**: Framer Motion for transitions (sidebar collapse, pane transitions)
- **Density**: comfortable default, compact and ultra-compact modes (Phase 3)

## Utility Function

```typescript
// src/lib/utils.ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

## Zustand Stores

### mailStore.ts
```typescript
interface MailState {
  selectedAccountId: string | null;
  selectedFolder: string | null;
  selectedMessageUid: number | null;
  messages: MessageSummary[];
  folders: Folder[];
  isLoading: boolean;

  setSelectedAccount: (id: string) => void;
  setSelectedFolder: (folder: string) => void;
  setSelectedMessage: (uid: number | null) => void;
  setMessages: (messages: MessageSummary[]) => void;
  setFolders: (folders: Folder[]) => void;
  markMessageRead: (uid: number) => void;
}
```

### accountStore.ts
```typescript
interface AccountState {
  accounts: Account[];
  isSetupComplete: boolean;

  setAccounts: (accounts: Account[]) => void;
  addAccount: (account: Account) => void;
  removeAccount: (id: string) => void;
}
```

### uiStore.ts
```typescript
interface UIState {
  sidebarWidth: number;        // pixels, default 240
  sidebarCollapsed: boolean;
  listPaneHeight: number;      // percentage, default 40
  theme: 'dark' | 'light';    // dark default

  setSidebarWidth: (w: number) => void;
  toggleSidebar: () => void;
  setListPaneHeight: (h: number) => void;
}
```

Persist uiStore to localStorage so layout survives restarts.

## Components

### AppLayout.tsx
- Three-pane layout: sidebar | message list / reading pane
- Sidebar is resizable (drag handle) and collapsible
- Message list and reading pane split vertically, resizable
- Custom draggable title bar (Tauri `data-tauri-drag-region`)

### Sidebar.tsx
- Account badge at top (avatar with initials, unread count)
- Folder tree below
- Collapsible with animation (Framer Motion)

### FolderTree.tsx
- Recursive rendering of folder hierarchy
- Bold + count badge for folders with unread messages
- Click to select folder → updates mailStore

### MessageList.tsx
- Virtual scrolling via @tanstack/react-virtual (handles thousands of messages)
- Each row: sender avatar (initials), sender name, subject, date, unread dot
- Bold text for unread messages
- Click to select → loads message body in reading pane
- Pagination: scroll to bottom triggers next page fetch

### MessageListItem.tsx
- Single row in the message list
- Shows: from (name or email), subject, date (relative), snippet
- Unread: bold text + blue dot indicator
- Flagged: star icon
- Has attachments: paperclip icon
- Hover state: subtle background change

### ReadingPane.tsx
- Message header: from, to, cc, date, subject
- Action buttons: reply, forward, archive, delete, star (Phase 2 — placeholder in Phase 1)
- EmailFrame below for HTML content
- "Images blocked" banner when remote images are stripped

### EmailFrame.tsx
- `<iframe sandbox="allow-same-origin" srcdoc={sanitizedHtml} />`
- Auto-resize iframe height to fit content
- Intercept link clicks → open in system browser
- Handle light/dark theme for email content

### AccountSetup.tsx
- Shown when no accounts are configured
- Provider selection: Gmail, iCloud (Phase 2), Outlook (Phase 2)
- "Connect with Google" button triggers OAuth2 flow
- Loading state while waiting for callback
- Success/error feedback

### StatusBar.tsx
- Bottom of window
- Shows: connection status (green dot = connected), last synced time, sync progress
- Listens to Tauri events: `sync-progress`, `sync-complete`, `connection-status`

### Shared Components
- **EmptyState**: centered icon + message for empty folders
- **LoadingSpinner**: consistent loading indicator
- **ErrorBoundary**: catches React errors, shows fallback UI

## Hooks

### useMessages.ts
Wraps message IPC calls. Handles loading states, error handling, and mailStore updates.

### useFolders.ts
Wraps folder IPC calls. Fetches folders on account selection.

### useAccount.ts
Wraps account and auth IPC calls. Manages OAuth2 flow state.

### useTauriEvent.ts
Generic hook for listening to Tauri events from the Rust backend.
```typescript
function useTauriEvent<T>(event: string, handler: (payload: T) => void) {
  useEffect(() => {
    const unlisten = listen<T>(event, (e) => handler(e.payload));
    return () => { unlisten.then(fn => fn()); };
  }, [event, handler]);
}
```
