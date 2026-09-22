# Two-finger swipe actions on message rows

**Status:** Design approved · Ready for implementation plan
**Date:** 2026-05-20
**Scope:** Frontend only. No Rust / IPC changes.

## Summary

Add Apple-Mail-style two-finger trackpad swipes to rows in the message list.

- Swipe **left** → archive (deferred 5s, with undo toast). Orange panel reveals from the right.
- Swipe **right** → toggle read/unread. Neutral panel reveals from the left; label flips based on current state.
- Row content translates under the user's fingers; an action panel sits behind. The action commits if `|offset| ≥ threshold` at release; otherwise the row springs back.

## Motivation

CXMail is macOS-first and already mirrors Apple Mail conventions (right-click context menu, sandboxed HTML frame, similar density). Users moving from Apple Mail expect this gesture; today they must reach for the context menu or keyboard. The IPC for both actions already exists, so the change is purely a UX layer on top of existing primitives.

## Non-goals

- Touch-screen support (CXMail is macOS-only; no macOS touchscreens exist).
- Native AppKit `NSGestureRecognizer` plumbing. We accept a web-event approximation.
- Custom per-folder swipe configuration (Mail.app lets you pick the action). Out of scope for v1.
- iOS Mail-style "swipe-half-to-reveal-buttons-then-tap" mode. Single commit-on-release is the only mode.
- Bulk swipe on multi-selection. Swipe acts on the swiped row only; selection state is untouched.

## Architecture

Three new pieces, two existing files modified, one store extension:

| File | Status | Purpose |
|------|--------|---------|
| `src/hooks/useSwipeAction.ts` | NEW | Wheel-event state machine. Returns offset, revealed action, phase, and a `ref` to bind. |
| `src/components/mail/SwipeableRow.tsx` | NEW | Visual wrapper: absolute-positioned action panel behind, row content on top with `translateX(offset)`. |
| `src/components/mail/MessageListItem.tsx` | MODIFIED | Wrap outer `<div>` in `SwipeableRow`. Pass `onArchive`, `onToggleRead`, `isRead`, `canArchive`. No other behavioral changes. |
| `src/components/mail/MessageList.tsx` | MODIFIED | Wire callbacks → store actions → IPC. Add `overscroll-behavior-x: none` to the virtualized scroll container. |
| `src/stores/mailStore.ts` | MODIFIED | Add `markMessageUnread(uid, accountId)` mirroring `markMessageRead` (set `is_read: false`, leave `thread_has_unread` alone). Add `pendingArchiveIds: Set<string>` plus helpers `addPendingArchive`, `removePendingArchive`. Provide a `getVisibleMessages()` selector (or inline filter in `MessageList`) that excludes pending IDs. |

**IPC is reused as-is.** No Rust changes:

```ts
api.messages.archive(accountId, folder, uids)   // existing
api.messages.markRead(accountId, folder, uids)  // existing
api.messages.markUnread(accountId, folder, uids) // existing
```

## Component contracts

### `useSwipeAction` hook

```ts
type SwipePhase = 'idle' | 'tracking' | 'committing' | 'snapping';

interface UseSwipeActionOptions {
  isRead: boolean;          // drives the right-swipe label ('Read' vs 'Unread')
  canArchive: boolean;      // false in Archive / Drafts / Scheduled folders
  rowWidthPx?: number;      // optional override; defaults to measured ref width
  onArchive: () => void;    // fires once, the moment commit begins
  onToggleRead: () => void; // fires once, the moment commit begins
}

interface UseSwipeActionResult {
  offset: number;                       // -W..+W in px; drives translateX on the row
  revealed: 'archive' | 'read' | null;  // which panel is showing (or null at rest)
  phase: SwipePhase;
  bind: { ref: (el: HTMLDivElement | null) => void };
}

function useSwipeAction(opts: UseSwipeActionOptions): UseSwipeActionResult;
```

### Hook internals

**Constants (all declared at top of the file):**

```ts
const MIN_ABS_DELTA_X    = 2;      // px floor per event (filter noise)
const DIRECTION_LOCK_PX  = 8;      // |accumDeltaX| to enter `tracking`
const H_OVER_V_RATIO     = 1.5;    // horizontal-dominant heuristic
const END_IDLE_MS        = 150;    // no-wheel idle → gesture ended
const MOMENTUM_DECAY_PX  = 1.5;    // |deltaX| below this counts as decaying
const MOMENTUM_DECAY_RUN = 3;      // consecutive decay events to bail
const THRESHOLD_FRAC     = 0.25;   // commit threshold = 25% of row width
const THRESHOLD_MIN_PX   = 80;
const THRESHOLD_MAX_PX   = 140;
const SNAP_BACK_MS       = 200;
const COMMIT_GLIDE_MS    = 180;
```

**State machine:**

1. **`idle`** — listen for `wheel`. Reject and stay idle if:
   - `event.ctrlKey || event.shiftKey` (pinch-zoom / shift-wheel intercept).
   - `event.deltaMode !== 0` (non-pixel wheels normalize differently; safest to skip).
   - `Math.abs(deltaX) < MIN_ABS_DELTA_X`.
   - Direction starts vertical (`Math.abs(deltaY) > Math.abs(deltaX) * H_OVER_V_RATIO`) — set a `verticalRejected` latch for the rest of this wheel burst so trailing diagonal events don't sneak in.
   - The first horizontally-dominant event arrives: **immediately `preventDefault()`** to head off WKWebView swipe-back and horizontal rubber-band. Begin accumulating `accumDeltaX`. Do not yet enter `tracking`.
2. **`idle` → `tracking`** when `|accumDeltaX| >= DIRECTION_LOCK_PX`. Lock direction sign. Reject mid-gesture sign flips (clamp to 0 on the unlocked side).
3. **`tracking`** — every wheel event:
   - `preventDefault()`.
   - If `canArchive === false` and direction is left, clamp `offset` at 0 (panel never reveals).
   - Update `offset` and `revealed`. Reset the `END_IDLE_MS` timer.
   - **Momentum bail:** track a running count of consecutive events with `|deltaX| < MOMENTUM_DECAY_PX`. When the count hits `MOMENTUM_DECAY_RUN`, treat the gesture as ended *now* using the offset at the start of the decay run (not the current offset). This prevents momentum from inflating offset past threshold after the user has lifted their fingers.
4. **`tracking` → `committing`** when the idle timer fires AND `|offset_at_end| >= threshold`. Ignore further wheel events. Call the appropriate `onArchive` / `onToggleRead` callback immediately, then animate `offset` to `±rowWidth` over `COMMIT_GLIDE_MS`. Both callbacks are synchronous from the hook's perspective — they enqueue store updates and IPC fire-and-forget.
5. **`tracking` → `snapping`** when the idle timer fires AND `|offset_at_end| < threshold`. Animate `offset` back to 0 over `SNAP_BACK_MS`.
6. **`snapping` / `committing` → `idle`** at animation end. Reset accumulators.

**Threshold computation:**

```ts
const threshold = clamp(rowWidth * THRESHOLD_FRAC, THRESHOLD_MIN_PX, THRESHOLD_MAX_PX);
```

Measured **once at gesture start** (first wheel event), cached for the rest of the gesture. Resizing the message-list pane mid-gesture doesn't change the commit point.

**Effect / listener wiring:**

- `useEffect` attaches the wheel listener via `node.addEventListener("wheel", h, { passive: false, capture: true })`.
- Latest callbacks held in a ref, refreshed via a separate effect, so the wheel listener is attached exactly once per row element and never reattached just because a parent re-rendered.
- Cleanup clears any pending idle timer, animation timer, and removes the listener. Idempotent so React 18/19 StrictMode dev-mode double-mount is safe.
- The hook also keys an internal swipe-reset on the row's `message.id`. When the virtualized list reuses a DOM node for a different message, the hook detects the change and forces `offset = 0`, `phase = 'idle'`.
- While `phase !== 'idle'`, set a data attribute (`data-swiping="true"`) on the row so siblings (e.g. the existing `HoverCard` preview) can suppress themselves via CSS / context.

**Reduced motion:** if `window.matchMedia('(prefers-reduced-motion: reduce)').matches`, both `SNAP_BACK_MS` and `COMMIT_GLIDE_MS` clamp to 0.

### `SwipeableRow` component

```tsx
interface SwipeableRowProps {
  isRead: boolean;
  canArchive: boolean;
  onArchive: () => void;
  onToggleRead: () => void;
  children: React.ReactNode;
}
```

Structure:

```
<div ref={bind.ref} className="relative overflow-hidden" data-swiping={...}>
  {/* Left panel — reveals on right swipe (read/unread) */}
  <div className="absolute inset-y-0 left-0 ... flex items-center px-4 bg-[neutral]">
    {isRead ? 'Mark unread' : 'Mark read'}
  </div>
  {/* Right panel — reveals on left swipe (archive) */}
  <div className="absolute inset-y-0 right-0 ... flex items-center justify-end px-4 bg-[#ff9f0a]">
    Archive
  </div>
  {/* Row content */}
  <div style={{ transform: `translateX(${offset}px)`, transition: ... }}>
    {children}
  </div>
</div>
```

Action-panel widths are full-row; the offset value alone determines how much is visible. The panel becomes visually "active" (filled background) once `|offset| >= threshold` — gives the user a tactile cue that release will commit. Below threshold, the panel renders at reduced opacity (~0.5).

The `transition` on the content `<div>` is `none` while `phase === 'tracking'` (so the row tracks fingers with no lag), and `transform 200ms ease-out` (or `180ms` for commit) during `snapping` / `committing`.

### `MessageListItem` changes

Wrap the outermost element in `<SwipeableRow>` with these props:

```tsx
<SwipeableRow
  isRead={!isUnread}
  canArchive={canArchiveInThisFolder}
  onArchive={handleSwipeArchive}
  onToggleRead={handleSwipeToggleRead}
>
  {/* existing content */}
</SwipeableRow>
```

Where:
- `canArchiveInThisFolder = currentFolder !== ARCHIVE_FOLDER && currentFolder !== 'Drafts' && currentFolder !== 'Scheduled'`.
- `handleSwipeArchive` and `handleSwipeToggleRead` are defined in `MessageList` and threaded down via props.

While `data-swiping="true"`, the existing `HoverCard` (`Radix HoverCard` for the body preview) is suppressed by adding a guard in its `onOpenChange`: if the row has `data-swiping`, force `open = false`. Minor footprint, no refactor.

Click handling (existing `handleClick` in `MessageListItem`) is unchanged: if a wheel gesture is in progress and a click arrives, the wheel handler will have already prevented anything; clicks during snap-back are accepted as normal (low priority concern).

### `MessageList` callbacks

```ts
const handleSwipeArchive = (message: Message) => {
  const folder = message.folder_name ?? "INBOX";
  // Hide first — uids fetch happens lazily on expiry so the row vanishes immediately.
  mailStore.addPendingArchive(message.id);
  uiStore.showUndoToast({
    label: 'Archived',
    durationMs: 5000,
    onUndo: () => mailStore.removePendingArchive(message.id),
    onExpire: async () => {
      mailStore.removePendingArchive(message.id);
      const uids = await api.messages.threadUidsInFolder(
        message.account_id,
        folder,
        message.thread_root_id ?? message.message_id,
      );
      await api.messages.archive(message.account_id, folder, uids).catch(console.error);
    },
  });
};

const handleSwipeToggleRead = async (message: Message) => {
  const folder = message.folder_name ?? "INBOX";
  const isCurrentlyUnread = !message.is_read || message.thread_has_unread;
  if (isCurrentlyUnread) {
    mailStore.markMessageRead(message.uid, message.account_id);  // existing
    await api.messages.markRead(message.account_id, folder, [message.uid]);
  } else {
    mailStore.markMessageUnread(message.uid, message.account_id); // NEW
    await api.messages.markUnread(message.account_id, folder, [message.uid]);
  }
};
```

**Folder resolution rationale:** `selectedFolder` is null in unified inbox / account-group inbox views (the default per recent commits). Each message carries `folder_name`; that's the source of truth for which folder owns the UID. Fallback to `INBOX` for safety.

**Thread vs single UID:** Archive operates on the whole thread (matches existing context-menu archive behavior). Mark read/unread operates on the representative UID only — `markMessageRead` already clears `thread_has_unread` to reflect the thread visually, and the backend fan-out handles the rest. This matches `useKeyboardShortcuts.ts` existing behavior for `e` (archive) and `r` (mark read).

## Data flow

### Read/unread swipe

```
wheel events
  → useSwipeAction.offset updates
  → SwipeableRow translates
  → release past threshold
  → handleSwipeToggleRead(message)
       → mailStore.markMessageRead/Unread (optimistic UI)
       → api.messages.markRead/markUnread (IPC, fire-and-forget)
  → COMMIT_GLIDE_MS animation
  → offset back to 0 (read panel briefly visible, then row stays)
```

### Archive swipe (deferred)

```
wheel events
  → useSwipeAction.offset updates
  → release past threshold
  → handleSwipeArchive(message)
       → mailStore.addPendingArchive(message.id)  // row vanishes from filtered list
       → uiStore.showUndoToast({ durationMs: 5000, onUndo, onExpire })
  → COMMIT_GLIDE_MS animation (row glides off left)

[t = 0..5000ms]
  ├── user clicks Undo:
  │      → mailStore.removePendingArchive(message.id)  // row reappears
  │      → toast dismisses
  │      → no IPC call
  └── timer expires:
         → mailStore.removePendingArchive (so removal is "real")
         → api.messages.archive(...)  // IPC fires, IMAP MOVE happens
         → row gone for real
```

**Pending-archive set semantics:**

- `mailStore.messages` is unchanged. A new getter `getVisibleMessages()` (or just inline filter in `MessageList`) returns `messages.filter(m => !pendingArchiveIds.has(m.id))`.
- The sync path (`syncAllInboxes` → `setMessages`) must not flicker the row back. When new messages arrive, the existing dedup-by-`account_id-uid` logic continues to work; pending IDs stay in the set until expiry, so the filter still hides them.
- If the app reloads during the 5s window, the pending set is not persisted. The row simply re-appears (not yet archived). This is acceptable — undo timers don't survive restarts anywhere in the app.

## Visual design

| State | Right swipe (read/unread) | Left swipe (archive) |
|-------|---------------------------|----------------------|
| Panel background | `bg-[#3d3d3d]` (existing border color, neutral dark) | `bg-[#ff9f0a]` (existing amber warning) |
| Panel text | `text-white` | `text-white` |
| Icon | `Mail` (read) / `MailOpen` (unread) from lucide-react | `Archive` from lucide-react |
| Below-threshold opacity | `0.5` | `0.5` |
| At/above threshold | `1.0` + subtle scale-up of label | `1.0` + subtle scale-up of label |
| Row z-index | Above panel | Above panel |
| Border-radius | Inherits row radius (rounded corners stay) | Inherits row radius |

All colors come from the existing CSS palette in `.claude/rules/frontend.md` — no new tokens.

## Edge cases

| Case | Behavior |
|------|----------|
| Vertical scroll dominant | Gesture rejected; `verticalRejected` latch prevents diagonal events later in the burst from triggering swipe. |
| User swipes left in Archive / Drafts / Scheduled folder | `canArchive=false`. Offset clamps at 0; right swipe (read/unread) still works. |
| Multi-selected rows; user swipes one of them | Swipe acts on the swiped row only. Selection unchanged. |
| Message gets recycled (virtualization) mid-snap | Hook detects `message.id` change, forces `offset=0`, `phase='idle'`. No flicker. |
| `prefers-reduced-motion` | Snap/commit animations collapse to 0ms. Functional behavior unchanged. |
| Magic Mouse horizontal tilt | Triggers the swipe. Acceptable — JS can't distinguish trackpad from Magic Mouse. |
| `ctrlKey` / `shiftKey` held during wheel | Treated as zoom / orthogonal scroll. Swipe rejected. |
| Momentum scroll after finger lift carries offset past threshold | Detected by `MOMENTUM_DECAY_RUN` heuristic — commits using the pre-decay offset, so accidental commits from momentum don't happen. Imperfect but mitigates the common case. |
| User undoes archive after 5s | The Undo button is gone; row is archived for real. Cmd+Z is not wired (out of scope). |
| Network error during deferred archive IPC | Toast is already gone. Row stays hidden (still in `pendingArchiveIds`?). We clear pendingArchive on expiry *before* IPC, so IPC failure leaves the row visible — matches existing failure mode of context-menu archive. Log to console. |
| User triggers archive on Row A, then Row B, then undoes A | Each archive gets its own toast/timer. Undo-A restores A without affecting B's timer. |

## Error handling

- IPC failures (`markRead`, `markUnread`, `archive`) log to console. No retry, no user-facing error toast. Matches existing context-menu behavior — these actions are idempotent and the next sync corrects state.
- Animation timer cleanup is wrapped in `try/catch` in the unmount path (defensive, not strictly necessary).
- Wheel listener is removed on unmount; if a wheel arrives between `committing` start and unmount, the phase guard ignores it.

## Testing

**Unit tests** (`src/hooks/__tests__/useSwipeAction.test.ts`):
- Dominant vertical wheel does not enter `tracking`.
- `ctrlKey` / `shiftKey` events ignored.
- `canArchive=false` clamps left swipes at 0.
- Threshold scales with measured row width and clamps to `[80, 140]`.
- Momentum-decay bail commits using offset before decay started.
- Idle timer resets on each wheel event and fires after `END_IDLE_MS`.
- Reduced-motion media query clamps animation durations.

**Component tests** (`src/components/mail/__tests__/SwipeableRow.test.tsx`):
- Renders read panel on right swipe, archive panel on left.
- Label flips between "Mark read" and "Mark unread" based on `isRead`.
- Calls `onArchive` / `onToggleRead` once at commit start.
- Resets visual state when wrapped child's key changes (virtualization recycle).

**Manual smoke tests** (recorded in PR description):
1. Two-finger swipe right on an unread row → row clears unread styling, row stays.
2. Two-finger swipe right on a read row → row gets unread dot, row stays.
3. Two-finger swipe left → row vanishes, toast appears for 5s, IMAP MOVE confirmed via DB query after.
4. Swipe left → click Undo → row returns, no IPC fired (verify via `cargo log` or network panel).
5. Two-finger vertical scroll over rows → no spurious swipe triggers.
6. Swipe partway and release → springs back, no action.
7. Resize message-list pane during swipe → threshold doesn't shift mid-gesture.

## Out of scope (follow-up)

- Per-folder swipe configuration (Mail.app feature).
- Long-swipe-commits-without-release (Mail.app full-swipe variant).
- Cmd+Z to restore the just-archived message.
- Touch-screen / iPad support.
- Native AppKit `NSGestureRecognizer` plumbing for true release semantics.

## Open implementation questions

None at design time. Resolve during implementation:

1. Whether `pendingArchiveIds` lives in `mailStore` or `uiStore`. Leaning `mailStore` since it directly affects what `messages` renders.
2. Whether `showUndoToast` is a new method on `uiStore` or a new component. Existing `UndoSendToast` is a close pattern — likely just extend it.
3. Lucide icon sizing — match existing row icons (`h-4 w-4`).
