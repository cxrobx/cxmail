import { useState, useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onOpenUrl, getCurrent } from "@tauri-apps/plugin-deep-link";
import { parseMailto, plainBodyToComposeHtml } from "@/lib/mailto";
import Sidebar from "./Sidebar";
import StatusBar from "./StatusBar";
import MessageList from "@/components/mail/MessageList";
import ReadingPane from "@/components/mail/ReadingPane";
import CalendarView from "@/components/mail/CalendarView";
import ComposeModal from "@/components/mail/ComposeModal";
import SearchBar from "@/components/mail/SearchBar";
import CommandPalette from "@/components/shared/CommandPalette";
import KeyboardShortcutSheet from "@/components/shared/KeyboardShortcutSheet";
import SettingsDialog from "@/components/shared/SettingsDialog";
import McpApprovalModal from "@/components/shared/McpApprovalModal";
import UndoSendToast from "@/components/shared/UndoSendToast";
import Toast from "@/components/shared/Toast";
import { checkForAppUpdate } from "@/lib/updater";
import FloatingWindowManager from "@/components/shared/FloatingWindowManager";
import ChatPanel from "@/components/chat/ChatPanel";
import AccountSetup from "@/components/accounts/AccountSetup";
import { useUIStore, applyTheme } from "@/stores/uiStore";
import { useVaultLookSync } from "@/hooks/useVaultLookSync";
import { useWindowStore } from "@/stores/windowStore";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useChatStore } from "@/stores/chatStore";
import { api } from "@/lib/tauri";
import { applySyncStatuses } from "@/lib/syncHealth";
import type { McpActivity, OutgoingEmail, SavedDraftRef } from "@/types/email";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { X, PenSquare, Settings, Sparkles } from "lucide-react";

const SYNC_INTERVAL_MS = 300_000; // Poll every 5 minutes

// Friendly toast text for an MCP activity, varied by kind/tool.
function mcpActivityToastMessage(env: McpActivity): string {
  if (env.kind === "draft-created") return "New draft from Claude";
  if (env.kind === "draft-updated") return "Draft updated by Claude";
  if (env.kind === "calendar") {
    return env.tool === "send_calendar_invites"
      ? "Calendar invites sent by Claude"
      : "Calendar updated by Claude";
  }
  if (env.kind === "calendar-approval-request") {
    return "Claude is waiting for calendar invite approval";
  }
  switch (env.tool) {
    case "archive_email": return "Archived by Claude";
    case "move_email": return "Moved by Claude";
    case "flag_email": return "Flag updated by Claude";
    case "delete_email": return "Deleted by Claude";
    case "bulk_delete_emails": return "Emails deleted by Claude";
    case "create_group": return "Group created by Claude";
    case "update_group": return "Group updated by Claude";
    case "delete_group": return "Group deleted by Claude";
    default: return "Updated by Claude";
  }
}

export default function AppLayout() {
  const { sidebarWidth, sidebarCollapsed, messageListWidth, setMessageListWidth, theme, density, showShortcutSheet, toggleShortcutSheet, calendarFullScreen, setSettingsOpen } = useUIStore();
  const { showingAddAccount, setShowingAddAccount } = useAccountStore();
  const { isComposing, setComposing, setSyncing, setSyncCompleted, setSyncError, setAccountsNeedingReauth, specialView, composePrefill, composeNonce } = useMailStore();
  const openWindow = useWindowStore((s) => s.openWindow);
  const accounts = useAccountStore((s) => s.accounts);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [isDraggingDivider, setIsDraggingDivider] = useState(false);
  const dividerStartX = useRef(0);
  const dividerStartWidth = useRef(0);
  const syncInFlightRef = useRef(false);
  const idleSyncInFlightRef = useRef(new Set<string>());
  // De-dupe mailto deliveries: warm onOpenUrl + cold getCurrent can both surface
  // the same launch URL, and debug builds double-fire (one with an empty URL).
  const lastMailtoRef = useRef<{ url: string; at: number }>({ url: "", at: 0 });

  // Sync theme to <html> so CSS variables cascade to body and all descendants.
  //
  // `applyTheme` rather than a bare `setAttribute`, because two other layers
  // have to move with it: the pane alphas (their floors are theme-dependent, so
  // leaving them applies dark's floor to a light window) and AppKit's
  // `NSApp.appearance`, which native chrome renders against. Kept as an
  // effect as well as `setTheme`'s own call so a theme arriving from anywhere —
  // a rehydrated store, a future settings sync — still moves all three.
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  // The Obsidian vault's palette, via Onyx — asked only while the theme is
  // `vault` or Settings is open. A palette change repaints through
  // `setVaultLook`, not through the effect above.
  useVaultLookSync();

  useEffect(() => {
    const timer = window.setTimeout(() => void checkForAppUpdate(), 8_000);
    return () => window.clearTimeout(timer);
  }, []);

  const toggleCommandPalette = useCallback(() => {
    setCommandPaletteOpen((prev) => !prev);
  }, []);

  /**
   * Unpin the docked composer into a draggable, resizable floating window,
   * carrying the in-progress message across. Mirrors ReadingPane's inline
   * pop-out: ComposeModal's `handlePopOut` has already flushed the draft via
   * `saveOrEdit()` before invoking this, so `draftRef` points at the live draft
   * UID and the floating window won't re-mint it. Recipients are passed in full
   * (to/cc/bcc) and attachments come from live in-memory state, not draft
   * metadata — see gotcha #25.
   */
  const handleComposePopOut = useCallback(
    (snapshot: OutgoingEmail, draftRef: SavedDraftRef | null, composeAccountId?: string) => {
      openWindow({
        type: "compose",
        title: snapshot.subject || "New Message",
        props: {
          mode: "new",
          accountId: composeAccountId,
          defaultTo: snapshot.to.map((r) => r.email),
          defaultCc: snapshot.cc.map((r) => r.email),
          defaultBcc: snapshot.bcc.map((r) => r.email),
          defaultSubject: snapshot.subject,
          defaultBody: snapshot.html_body,
          defaultAttachments: snapshot.attachments,
          draftContext:
            draftRef && composeAccountId
              ? { accountId: composeAccountId, folder: draftRef.folder, uid: draftRef.uid }
              : undefined,
        },
      });
      setComposing(false);
    },
    [openWindow, setComposing],
  );

  const rafRef = useRef(0);

  const handleDividerMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDraggingDivider(true);
    dividerStartX.current = e.clientX;
    dividerStartWidth.current = messageListWidth;
  }, [messageListWidth]);

  useEffect(() => {
    if (!isDraggingDivider) return;
    const handleMouseMove = (e: MouseEvent) => {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(() => {
        const delta = e.clientX - dividerStartX.current;
        const newWidth = Math.min(Math.max(dividerStartWidth.current + delta, 200), 700);
        setMessageListWidth(newWidth);
      });
    };
    const handleMouseUp = () => {
      cancelAnimationFrame(rafRef.current);
      setIsDraggingDivider(false);
    };
    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      cancelAnimationFrame(rafRef.current);
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [isDraggingDivider, setMessageListWidth]);

  useKeyboardShortcuts({ onCommandPalette: toggleCommandPalette, onShortcutSheet: toggleShortcutSheet });

  // Single funnel for every `mailto:` source: OS-default-handler opens (warm and
  // cold), the Rust on_navigation emit, and in-app email-iframe link clicks.
  // Surfaces the window, then opens a prefilled compose. Stable (empty deps) so
  // the wiring effect below subscribes exactly once.
  const handleMailtoUrl = useCallback(async (url: string) => {
    if (!url || !/^mailto:/i.test(url)) return;
    const now = Date.now();
    if (url === lastMailtoRef.current.url && now - lastMailtoRef.current.at < 1500) return;
    lastMailtoRef.current = { url, at: now };

    const parsed = parseMailto(url);
    try {
      const win = getCurrentWindow();
      await win.show();
      await win.unminimize();
      await win.setFocus();
    } catch (e) {
      console.error("mailto: failed to surface window", e);
    }
    useMailStore.getState().openComposeWith({
      to: parsed.to,
      cc: parsed.cc,
      bcc: parsed.bcc,
      subject: parsed.subject,
      // The body is plain text (RFC 6068) but ComposeModal's defaultBody is an
      // HTML contract — escape it so a hostile mailto link can't inject nodes
      // into the privileged compose editor (see plainBodyToComposeHtml).
      body: parsed.body ? plainBodyToComposeHtml(parsed.body) : undefined,
    });
  }, []);

  // Wire all mailto sources once. getCurrent() is the cold-start source of truth
  // (the plugin's internal launch-URL emit is fire-and-forget and is lost if no
  // onOpenUrl listener is mounted yet); it must run exactly once — re-running it
  // on a later mount would re-open the launch compose. Keeping this effect's only
  // dependency the stable handleMailtoUrl guarantees that.
  useEffect(() => {
    let cancelled = false;
    const unsubs: (() => void)[] = [];
    const wire = async () => {
      // 1. OS default handler, app already running (warm).
      const offOpenUrl = await onOpenUrl((urls) => {
        urls.forEach((u) => void handleMailtoUrl(u));
      });
      // 2. OS default handler, cold launch — read the surviving launch URL(s).
      const current = await getCurrent();
      if (!cancelled && current) current.forEach((u) => void handleMailtoUrl(u));
      // 3. In-app top-level mailto navigations (Rust external-links on_navigation).
      const offEmit = await listen<string>("mailto-open", (e) => void handleMailtoUrl(e.payload));
      if (cancelled) {
        offOpenUrl();
        offEmit();
        return;
      }
      unsubs.push(offOpenUrl, offEmit);
    };
    // Swallow rejections: in a non-Tauri context (e.g. the jsdom test env) the
    // deep-link plugin bindings reject — that must not surface as an unhandled
    // rejection. The mailto sources simply stay unwired there.
    void wire().catch((e) => console.error("mailto: wiring failed", e));
    // 4. In-app email-iframe mailto clicks. EmailFrame dispatches this DOM event
    //    instead of shelling out (which, post-default-registration, would bounce
    //    the URL back to CXMail).
    const domHandler = (e: Event) => {
      void handleMailtoUrl((e as CustomEvent<string>).detail);
    };
    window.addEventListener("cxmail:mailto", domHandler);
    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
      window.removeEventListener("cxmail:mailto", domHandler);
    };
  }, [handleMailtoUrl]);

  // Clear any stale isSyncing flag left over from a previous unclean unmount
  // (HMR reload, force-quit mid-sync). A stale flag blocks the in-flight guard
  // in runBackgroundSync and causes the sidebar to show "Syncing..." forever.
  useEffect(() => {
    setSyncCompleted();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runBackgroundSync = useCallback(async () => {
    if (accounts.length === 0) return;
    if (syncInFlightRef.current || useMailStore.getState().isSyncing) return;

    syncInFlightRef.current = true;
    setSyncing(true);

    try {
      // Safety timeout: if backend hangs (e.g., blocked mutex), give up after 4 minutes
      const result = await Promise.race([
        api.messages.syncAllInboxes(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error("Sync timed out after 4 minutes")), 240_000)
        ),
      ]);
      const failed = result.account_statuses.filter((s) => !s.success);
      const reauthEmails = result.account_statuses
        .filter((s) => s.needs_reauth)
        .map((s) => s.email);
      setAccountsNeedingReauth(reauthEmails);
      const allFailed = failed.length === result.account_statuses.length && failed.length > 0;
      // A chronically failing account is STATE, not an event — the sidebar
      // warning glyph carries it. Toasts fire only on transitions: once when
      // an account crosses into failing (~3 consecutive ticks) and once on
      // recovery. All-accounts-failed is a global condition (network down,
      // wake from sleep) with its own StatusBar treatment, so it records
      // health without announcing per-account.
      const { next, announcements } = applySyncStatuses(
        useMailStore.getState().syncHealthByAccount,
        result.account_statuses,
        new Date().toISOString(),
        !allFailed,
      );
      useMailStore.getState().setSyncHealth(next);
      for (const a of announcements) {
        useUIStore.getState().addToast({ message: a.message, type: a.type, duration: 8000 });
      }
      for (const s of failed) {
        console.error(`Sync failed for ${s.email}: ${s.error}`);
      }
      if (allFailed) {
        setSyncError(`All accounts failed to sync`);
      } else {
        setSyncCompleted();
      }
    } catch (e) {
      console.error("Background sync failed:", e);
      setSyncError(e instanceof Error ? e.message : "Sync failed");
    } finally {
      syncInFlightRef.current = false;
      window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
    }
  }, [accounts.length, setSyncing, setSyncCompleted, setSyncError, setAccountsNeedingReauth]);

  // Listen for background scheduler events to trigger refreshes
  useEffect(() => {
    const unlisteners: (() => void)[] = [];
    const setup = async () => {
      const u1 = await listen("snooze-wakeup", () => {
        // Force re-render by clearing and re-setting messages
        // The MessageList will re-fetch on next render cycle
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      });
      const u2 = await listen("scheduled-send-processed", () => {
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
        // A due send just fired → the pending count dropped; refresh the badge.
        window.dispatchEvent(new CustomEvent("cxmail:scheduled-changed"));
      });
      const u3 = await listen<{ id: number; to_email: string; subject: string | null }[]>("followup-reminder-fired", (event) => {
        const fired = event.payload;
        if (fired.length > 0) {
          const first = fired[0];
          // Show notification via Notification API
          if (Notification.permission === "granted") {
            new Notification("Follow-up Reminder", {
              body: `No reply received for: ${first.subject || "your message"} (to ${first.to_email})`,
            });
          }
        }
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      });
      const u4 = await listen<{ account_id: string; folder: string }>("idle-new-mail", async (event) => {
        const accountId = event.payload.account_id;
        const folder = event.payload.folder || "INBOX";
        if (!accountId) return;

        const key = `${accountId}:${folder}`;
        if (idleSyncInFlightRef.current.has(key)) return;

        idleSyncInFlightRef.current.add(key);
        try {
          await Promise.race([
            api.messages.sync(accountId, folder),
            new Promise<never>((_, reject) =>
              setTimeout(() => reject(new Error("IDLE sync timed out")), 60_000)
            ),
          ]);
          window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
        } catch (e) {
          console.error("IDLE sync failed:", e);
        } finally {
          idleSyncInFlightRef.current.delete(key);
        }
      });
      const u5 = await listen("check-mail", async () => {
        await runBackgroundSync();
      });
      const u6 = await listen<string[]>("mail-from-unsubscribed-sender", (event) => {
        const senders = [...new Set(event.payload)];
        useUIStore.getState().addToast({
          message: `New mail from unsubscribed sender${senders.length > 1 ? "s" : ""}: ${senders.join(", ")}`,
          type: "info",
          duration: 8000,
        });
      });
      const u7 = await listen<{ account_id: string; status: string; error?: string }>("idle-status", (event) => {
        const { account_id: idleAccountId, status, error } = event.payload;
        if (status === "disconnected" && error) {
          console.warn(`IDLE disconnected for ${idleAccountId}: ${error}`);
        }
      });
      const u8 = await listen("sync-account-done", () => {
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      });
      // Per-batch progress from a multi-batch folder backfill (large folders
      // like All Mail) — repaint the list as rows land, not just at the end.
      const u11 = await listen("folder-sync-progress", () => {
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      });
      const u9 = await listen<{ account_id: string; uid: number }>("open-email-from-notification", (event) => {
        const { account_id, uid } = event.payload;
        if (account_id && uid) {
          // Navigate to the specific email
          const mailStore = useMailStore.getState();
          mailStore.setUnifiedInbox();
          mailStore.setSelectedMessage(uid, account_id);
        }
      });
      // Live updates from the standalone cxmail-mcp process (drafts/emails it
      // mutated). Re-emit as DOM events that drive existing refresh plumbing
      // (MessageList listens for cxmail:refresh-messages) plus the new modal
      // reload + row-flash listeners. See src-tauri/src/mcp/bridge.rs.
      const u10 = await listen<McpActivity>("mcp-activity", (event) => {
        const env = event.payload;
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
        window.dispatchEvent(new CustomEvent("cxmail:mcp-activity", { detail: env }));
        if (env.kind.startsWith("draft-")) {
          window.dispatchEvent(new CustomEvent("cxmail:draft-updated", { detail: env }));
        }
        useUIStore.getState().addToast({
          message: mcpActivityToastMessage(env),
          type: "info",
          duration: 2500,
        });
      });
      unlisteners.push(u1, u2, u3, u4, u5, u6, u7, u8, u9, u10, u11);
    };
    setup();
    return () => unlisteners.forEach((u) => u());
  }, [runBackgroundSync]);

  // Allow other components (e.g., Sidebar "Sync All") to trigger the centralized sync
  useEffect(() => {
    const handler = () => { void runBackgroundSync(); };
    window.addEventListener("cxmail:trigger-sync", handler);
    return () => window.removeEventListener("cxmail:trigger-sync", handler);
  }, [runBackgroundSync]);

  // Load unsubscribed senders on mount
  const setUnsubscribedSenders = useMailStore((s) => s.setUnsubscribedSenders);
  useEffect(() => {
    api.messages.listUnsubscribedSenders().then(setUnsubscribedSenders).catch(console.error);
  }, [setUnsubscribedSenders]);

  // Background poll: sync all inboxes periodically
  useEffect(() => {
    if (accounts.length === 0) return;
    // Run first sync shortly after mount (give IDLE a chance to connect first)
    const initialTimeout = setTimeout(() => {
      void runBackgroundSync();
    }, 5_000);
    const interval = setInterval(() => {
      void runBackgroundSync();
    }, SYNC_INTERVAL_MS);
    return () => {
      clearTimeout(initialTimeout);
      clearInterval(interval);
    };
  }, [accounts.length, runBackgroundSync]);

  // Sync on app focus after being away (e.g., wake from sleep)
  useEffect(() => {
    if (accounts.length === 0) return;
    let lastVisible = Date.now();
    const handleVisibility = () => {
      if (document.visibilityState === "visible") {
        if (Date.now() - lastVisible > 60_000) {
          void runBackgroundSync();
        }
      } else {
        lastVisible = Date.now();
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);
    return () => document.removeEventListener("visibilitychange", handleVisibility);
  }, [accounts.length, runBackgroundSync]);

  return (
    /* The root paints NO background, and that is load-bearing rather than
       tidiness. One opaque layer across the whole window means every pane
       composites over THAT instead of over the blurred desktop, so turning
       the sidebar's alpha down moves it toward the content's colour rather than
       toward the desktop — "much more transparent" renders as "slightly less
       contrast" and reads as the setting doing nothing at all. Each pane below
       paints its own; together they tile the window with no gaps. This is
       cxtasks gotcha #22, and it cost the most there. */
    <div className="flex h-screen flex-col" data-theme={theme} data-density={density}>
      {/* Title bar drag region. `deep` — not a bare attribute — because
          drag.js resolves a bare one as `el === composedPath[0]`, so only a
          direct hit on that exact element drags and every child is a dead
          spot. The Compose button needs no opt-out: isClickableElement
          already excludes BUTTON. See gotcha #51. */}
      <div
        data-tauri-drag-region="deep"
        className="flex h-8 shrink-0 items-center justify-between border-b border-border-subtle bg-base pl-20 pr-3"
      >
        <span className="text-xs font-medium text-content-muted">
          CXMail
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={() => openWindow({ type: "compose", title: "New Message", props: { mode: "new" } })}
            className="flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs text-content-secondary transition-colors hover:bg-surface hover:text-content"
            title="Compose (Cmd+N)"
          >
            <PenSquare className="h-3.5 w-3.5" />
            Compose
          </button>
          <button
            onClick={() => useChatStore.getState().toggle()}
            className="flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs text-content-secondary transition-colors hover:bg-surface hover:text-content"
            title="Chat with Claude (Cmd+L)"
          >
            <Sparkles className="h-3.5 w-3.5" />
            Claude
          </button>
          {/* Needs no drag opt-out: `isDragRegion`'s clickable-element check
              already excludes BUTTON. See gotcha #51. */}
          <button
            onClick={() => setSettingsOpen(true)}
            className="rounded-md p-1 text-content-secondary transition-colors hover:bg-surface hover:text-content"
            title="Settings"
            aria-label="Settings"
          >
            <Settings className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* Main content */}
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        {!sidebarCollapsed && (
          <div
            className="shrink-0 border-r border-border-subtle"
            style={{ width: sidebarWidth }}
          >
            <Sidebar />
          </div>
        )}

        {specialView === "calendar" && calendarFullScreen ? (
          /* Full-screen calendar: fills the entire content area (sidebar
             aside). No message list, divider, or reading pane — events open
             as floating popups. This is mutually exclusive with the split
             layout below, so CalendarView mounts exactly once. */
          <div className="flex min-w-0 flex-1 flex-col bg-base">
            <CalendarView fullScreen />
          </div>
        ) : (
          <>
            {/* Message list */}
            <div
              className={`flex shrink-0 flex-col overflow-hidden bg-base ${isDraggingDivider ? "pointer-events-none select-none" : ""}`}
              style={{ width: messageListWidth }}
            >
              <SearchBar />
              <MessageList />
            </div>

            {/* Draggable divider */}
            <div
              onMouseDown={handleDividerMouseDown}
              className={`w-[3px] shrink-0 cursor-col-resize transition-colors hover:bg-accent ${isDraggingDivider ? "bg-accent" : "bg-surface"}`}
            />

            {/* Reading pane. min-w-0 is required so the flex item can shrink
                below the iframe's intrinsic content width — without it, a wide
                email body pushes the pane past its allocated flex space and the
                parent's overflow-hidden clips text on the right edge. */}
            <div className={`flex min-w-0 flex-1 flex-col bg-base ${isDraggingDivider ? "pointer-events-none select-none" : ""}`}>
              <ReadingPane />
            </div>
          </>
        )}

        {/* Chat with Claude — a pane on the right edge; renders nothing while
            closed but stays mounted so a running turn keeps its events. */}
        <ChatPanel />
      </div>

      {/* Status bar */}
      <StatusBar />

      {/* Command palette */}
      <CommandPalette open={commandPaletteOpen} onClose={() => setCommandPaletteOpen(false)} />

      {/* Keyboard shortcut cheatsheet */}
      {showShortcutSheet && <KeyboardShortcutSheet onClose={toggleShortcutSheet} />}

      {/* Appearance settings */}
      <SettingsDialog />

      {/* MCP approval modal */}
      <McpApprovalModal />

      {/* Undo send toast */}
      <UndoSendToast />
      <Toast />

      {/* Floating windows (email pop-outs and compose) */}
      <FloatingWindowManager />

      {/* Legacy compose modal (keyboard shortcut + mailto deep-link prefill).
          key={composeNonce} forces a remount so a fresh mailto re-reads the
          default* props even when a compose is already open. */}
      {isComposing && (
        <ComposeModal
          key={composeNonce}
          onClose={() => setComposing(false)}
          onPopOut={handleComposePopOut}
          defaultTo={composePrefill?.to}
          defaultCc={composePrefill?.cc}
          defaultBcc={composePrefill?.bcc}
          defaultSubject={composePrefill?.subject}
          defaultBody={composePrefill?.body}
        />
      )}

      {/* Add account modal overlay */}
      {showingAddAccount && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-overlay">
          {/* max-h + scroll, NOT a fixed height: the generic IMAP form's
              server-settings disclosure is taller than the 2-field iCloud
              form, and `overflow-hidden` around a fixed height clipped it
              rather than scrolling it. */}
          <div className="relative flex max-h-[85vh] w-[460px] flex-col overflow-hidden rounded-xl bg-base-solid shadow-2xl">
            <button
              onClick={() => setShowingAddAccount(false)}
              className="absolute top-3 right-3 z-10 flex h-7 w-7 items-center justify-center rounded-full bg-elevated text-content-secondary hover:bg-elevated hover:text-content"
            >
              <X className="h-4 w-4" />
            </button>
            <div className="flex min-h-[420px] flex-1 items-center justify-center overflow-y-auto">
              <AccountSetup />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
