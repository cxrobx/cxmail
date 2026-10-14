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
- Dark mode preserves sender backgrounds and readable foregrounds. `src/lib/emailTheme.ts` resolves inline CSS and legacy colors, composites translucent backgrounds through ancestors, and corrects unreadable text to at least 4.5:1 contrast. Hover previews use the same preparation without adding scripts to their sandbox. Original sanitized HTML remains the source for each theme change.

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
- **SelectionFeedback**: selecting message or draft text immediately opens
  Shorten, Soften, Humanize, and a free-text instruction field. Showing the
  bubble keeps focus on the selected text; interacting with it freezes the range.
  Enter submits; Shift+Enter adds a line; Escape or an outside click dismisses.
  The bubble is portalled to the document body and clamped to the viewport.

### Selection feedback

- The bubble opens on selection without taking focus, as one compact row (three
  chips and a short field). It grows to the full text box only when the field is
  focused or there is a result, error or question to show. A lone word of 24
  characters or fewer gets no bubble (`wantsFeedback`): that selection is
  usually made to retype, bold or copy it.
- Draft actions use the existing `ai_adjust_tone` and `ai_rewrite_text` commands,
  then replace the saved plain-text range as one isolated undo step. Formatting
  within that range is dropped; a blank line in the result becomes a paragraph
  break and a single newline a line break. Styled HTML blocks, signatures, quoted history,
  non-text selections, and draft selections over 4,000 characters are excluded.
  If the range's text changed during inference, the result stays in the bubble
  with Copy instead of overwriting the draft.
- The chips depend on where the text is. A draft gets Shorten / Soften /
  Humanize and a "Change…" field that rewrite in place. Received mail (via
  `MessageBody` in the reading pane, expanded thread cards and pop-out views)
  gets `READ_ACTIONS` instead: ELI5 / Prove it / Next steps and an "Ask Claude…"
  field. Every received-mail action is a question for the in-app Claude chat,
  sent with the quoted passage and that body's own `(account_id, folder, uid)`:
  as a follow-up when the live chat is already about that email, otherwise as a
  new chat, which first asks before ending a conversation the user has had.
  Received mail is never rewritten, so it has no result, Copy or Use in reply.
  Prove it works from the chat's mailbox and vault tools; the chat has no web
  access, so its prompt asks for "No evidence" when it cannot check.
- `public/email-frame.js` relays bounded selections and frame-local rectangles
  over `postMessage`; `EmailFrame` validates the owning source window and geometry.
  Text right-click remains native. The existing iframe sandbox and CSP are retained.
- Humanize uses a fixed natural-language instruction, without voice-profile
  context. All rewrite output still passes through backend `call_inference_prose`.

Unit tests cover command routing, range protection, undo, relay validation, and
chat context. `scripts/check-selection-feedback-webkit.mjs` drives the actual
components via `selection-feedback.html` on a running Vite server. It requires
Playwright with WebKit installed; `CXMAIL_PLAYWRIGHT_MODULE`,
`CXMAIL_WEBKIT_EXECUTABLE`, and `CXMAIL_TEST_URL` support an existing runtime.

`scripts/check-email-theme-webkit.mjs` drives `email-theme.html` with synthetic
receipts to check actual text contrast, nested light/dark surfaces, colored
buttons, alpha and legacy colors, previews, theme flips and blocked images.
It uses the same Playwright environment variables as the selection check.

## Hooks

### useQuickAskSelection.ts
Publishes the selected message's own account, folder and UID through the typed
IPC wrapper, with serialized updates and bounded startup retries. Multiple
selection publishes no single UID. Rust resolves the display label locally and
serves it through a per-user Unix socket for Quick Ask; no email body or screen
capture is included. Native window identity is read on the AppKit main thread.

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
