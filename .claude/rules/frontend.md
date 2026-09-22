---
paths:
  - "src/**/*.tsx"
  - "src/**/*.ts"
  - "src/**/*.css"
---

# Frontend Patterns

## Tech Stack
React 19, TypeScript, Vite, Tailwind CSS 4, Radix UI, Lucide React, Framer Motion, TipTap, Zustand

## Component Organization

| Directory | Purpose |
|-----------|---------|
| `src/components/layout/` | AppLayout, Sidebar, StatusBar |
| `src/components/mail/` | ComposeModal, MessageList, ReadingPane, EmailFrame, SearchBar, AISummary, AIWritingMenu, CalendarEventCard, CategoryTabs, FollowupList, FollowupPopover, ScheduledView, SmartReplies, SnoozePopover, SnoozedList, TrackingBadge |
| `src/components/accounts/` | AccountSetup, AccountBadge |
| `src/components/shared/` | CommandPalette, ErrorBoundary, LoadingSpinner, EmptyState, UndoSendToast, McpApprovalModal |

## State Management

Three Zustand stores in `src/stores/`:
- **accountStore** — Account list, sync status
- **mailStore** — Selected account/folder/message, message lists, composing state, unified inbox, account group filtering
- **uiStore** — UI preferences (persisted to localStorage), undo send state, sidebar width

### mailStore Key State

| Field | Default | Purpose |
|-------|---------|---------|
| `isUnifiedInbox` | `true` | Show combined inbox across accounts |
| `selectedCategory` | `"primary"` | Active category tab filter |
| `selectedAccountGroup` | `null` | When set, unified inbox filters to group's accounts only |
| `selectedGroupId` | `null` | Smart inbox group (rules-based) — different from account groups |

Transitions: `setUnifiedInbox()` clears `selectedAccountGroup`; `setAccountGroupInbox(name)` sets both `isUnifiedInbox` and `selectedAccountGroup`; `setSelectedFolder()` clears both.

## IPC Bridge

All Tauri IPC calls go through typed wrappers in `src/lib/tauri.ts`. Never call `invoke()` directly from components — use the `api` namespace:
```
api.accounts.list()
api.messages.fetch(accountId, folder, page)
api.compose.send(accountId, email, delay)
```

## Error Logging

**`console.warn`/`console.error` is invisible in production.** Release builds
ship without DevTools (gotcha #19), so anything written only to the console
cannot be read by the user, by a support ticket, or by us. A dead updater
endpoint hid behind exactly that for weeks.

For any failure a user would file a bug about, route it to the Rust log so it
lands in `~/Library/Logs/com.cxmail.app/CXMail.log` beside the backend:

```ts
void api.system.logClientError("updater", String(error));   // scope, message
```

Never rejects (the wrapper swallows it) and returns `void` — a logging call
must not add a failure path at the site it exists to make visible. The message
is truncated at 2000 chars in Rust, since this is reachable from the webview.
Keep the `console.*` line too; it is still useful under `tauri dev`.

Not every console call needs converting. The bar is *would a user report this
and would we need the detail* — a transient toast the user can see is already
visible; a silent catch in code that only runs in PROD is not.

## Styling Conventions

- Use `cn()` from `src/lib/utils.ts` for conditional classNames (clsx + tailwind-merge)
- Dark theme only: backgrounds `#1a1a1a` / `#1e1e1e` / `#2d2d2d`, borders `#3d3d3d`
- Primary blue: `#0a84ff`, amber warning: `#ff9f0a`
- Modal pattern: `fixed inset-0` backdrop with centered card (see `McpApprovalModal.tsx`)
- Icons from lucide-react, typically `h-4 w-4` or `h-3.5 w-3.5`

## Rich Text Editor

TipTap with extensions: StarterKit, Underline, Link, Placeholder. Used in ComposeModal for email composition.

## Keyboard Shortcuts

Defined in `src/hooks/useKeyboardShortcuts.ts`, registered in `AppLayout.tsx`. Uses standard email client bindings.
