import * as ContextMenu from "@radix-ui/react-context-menu";
import { useEffect, useRef, useState } from "react";
import { User, Bell, Users, Tag, Check, MailMinus, UserMinus, Sparkles, Clipboard, Trash2, Inbox, MessageSquare } from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import { cn } from "@/lib/utils";
import { api } from "@/lib/tauri";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { useChatStore } from "@/stores/chatStore";
import { useUnsubscribe } from "@/hooks/useUnsubscribe";
import { uidsForThreadAction } from "@/lib/threadActions";
import type { MessageSummary, EmailCategory } from "@/types/email";

// Module-level Option tracker. Window keydown for Option DOES fire on this
// WebView (verified — Sidebar's identical pattern works for Option-drag).
// One install per module load. The frozen snapshot read at contextmenu
// time prevents the gotcha #16 click race: state is written here only by
// keydown/keyup/blur, never by the contextmenu handler — and the menu's
// onSelect reads `altHeldRef`, frozen at right-click time.
let isOptionPressed = false;
let trackerInstalled = false;
/** Last path segment, for a toast that has to fit on one line. */
function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

function installOptionTracker() {
  if (trackerInstalled || typeof window === "undefined") return;
  trackerInstalled = true;
  window.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Alt") isOptionPressed = true;
  });
  window.addEventListener("keyup", (e: KeyboardEvent) => {
    if (e.key === "Alt") isOptionPressed = false;
  });
  window.addEventListener("blur", () => { isOptionPressed = false; });
}
installOptionTracker();

const CATEGORIES: {
  id: EmailCategory;
  label: string;
  icon: typeof User;
}[] = [
  { id: "primary", label: "Primary", icon: User },
  { id: "updates", label: "Updates", icon: Bell },
  { id: "social", label: "Social", icon: Users },
  { id: "promotions", label: "Promotions", icon: Tag },
];

interface EmailContextMenuProps {
  message: MessageSummary;
  children: React.ReactNode;
  // Class for the Radix Trigger wrapper. Defaults to "h-full" for the
  // virtualized inbox (fixed-height cells). Search results live in a
  // normal flow container, so they pass "contents" to avoid stretching.
  triggerClassName?: string;
}

export default function EmailContextMenu({ message, children, triggerClassName = "h-full" }: EmailContextMenuProps) {
  const selectedAccountId = useMailStore((s) => s.selectedAccountId);
  const selectedFolder = useMailStore((s) => s.selectedFolder);
  const foldersByAccount = useMailStore((s) => s.foldersByAccount);
  const messages = useMailStore((s) => s.messages);
  const selectedMessageUids = useMailStore((s) => s.selectedMessageUids);
  const updateMessageCategory = useMailStore((s) => s.updateMessageCategory);
  const updateMessagesCategory = useMailStore((s) => s.updateMessagesCategory);
  const setCategoryCounts = useMailStore((s) => s.setCategoryCounts);
  const removeMessages = useMailStore((s) => s.removeMessages);

  const { handleUnsubscribeResult, addToast } = useUnsubscribe();

  // Sticky-on-true snapshot: set at menu open, can flip to true (but not
  // back to false) by pressing Option while the menu is open. Never
  // listens to keyup — that's what caused the gotcha #16 click race.
  // Reset only on menu close.
  const [altHeld, setAltHeld] = useState(false);
  const altHeldRef = useRef(false);
  const [menuOpen, setMenuOpen] = useState(false);

  // While the menu is open, watch Option keydown/keyup so the label
  // tracks the live state — pressing flips to "Copy Claude Prompt",
  // releasing flips back. Per gotcha #16 (mode B): this re-introduces
  // the click race, where releasing Option to mouse onto the menu item
  // can flip altHeldRef to false before the click fires. Accepted by the
  // user for the live-update UX.
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Alt") return;
      const pressed = e.type === "keydown";
      altHeldRef.current = pressed;
      setAltHeld(pressed);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
    };
  }, [menuOpen]);

  const currentCategory = message.category ?? "primary";
  const accountId = message.account_id ?? selectedAccountId;
  // Operate on the message's own folder when known (correct for cross-folder
  // search results); fall back to the selected folder for inbox rows where
  // folder_name is unset.
  const folder = message.folder_name ?? selectedFolder ?? "INBOX";
  const messageFolder = folder;
  const currentMessageKey = messageSelectionKey(accountId, message.uid, messageFolder);
  const sameAccountSelection = accountId
    ? messages.filter(
        (candidate) => {
          const candidateAccountId = candidate.account_id ?? selectedAccountId;
          const candidateFolder = candidate.folder_name ?? folder;
          return candidateAccountId === accountId
          && candidateFolder === messageFolder
          && selectedMessageUids.has(
            messageSelectionKey(candidateAccountId, candidate.uid, candidateFolder),
          );
        },
      )
    : [];
  const isMultiSelect =
    selectedMessageUids.has(currentMessageKey) && sameAccountSelection.length > 1;

  // Offer "Move to Inbox" only when the message lives in the account's
  // archive folder ([Gmail]/All Mail, Archive).
  const isArchiveFolder = accountId
    ? (foldersByAccount[accountId] ?? []).some(
        (f) => f.name === messageFolder && f.folder_type === "archive",
      )
    : false;

  const handleUnsubscribe = async () => {
    if (!accountId) return;
    try {
      const result = await api.messages.unsubscribe(accountId, folder, message.uid);
      await handleUnsubscribeResult(result, {
        accountId,
        senderEmail: message.from_email ?? undefined,
      });
    } catch {
      addToast({ message: "No unsubscribe option for this message", type: "error" });
    }
  };

  const handleUnsubscribeSender = async () => {
    if (!accountId || !message.from_email) return;
    try {
      const result = await api.messages.unsubscribeSender(accountId, message.from_email);
      await handleUnsubscribeResult(result, {
        accountId,
        senderEmail: message.from_email,
      });
    } catch {
      addToast({ message: "No unsubscribe option for this sender", type: "error" });
    }
  };

  const handleOpenInClaude = async () => {
    if (!accountId) return;
    try {
      const handoff = await api.claude.openEmail(accountId, folder, message.uid);
      // Which repo it landed in is otherwise invisible — the Ghostty window
      // looks identical wherever it cd'ed — so a wrong mapping would go
      // unnoticed until Claude answered about the wrong project.
      if (handoff.missing_repo_path) {
        addToast({
          message: `Opened in Claude — ${handoff.missing_repo_path} isn't there, so it started in the scratch folder`,
          type: "error",
        });
      } else if (handoff.repo) {
        addToast({
          message: `Opened in Claude — ${basename(handoff.repo.repo_path)} (${handoff.repo.source})`,
          type: "success",
        });
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("Open in Claude failed:", msg);
      addToast({ message: msg || "Failed to open in Claude", type: "error" });
    }
  };

  const handleCopyClaudePrompt = async () => {
    if (!accountId) return;
    try {
      const prompt = await api.claude.getPrompt(accountId, folder, message.uid);
      await clipboardWriteText(prompt);
      addToast({ message: "Claude prompt copied to clipboard", type: "success" });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("Copy Claude prompt failed:", msg);
      addToast({ message: msg || "Failed to copy Claude prompt", type: "error" });
    }
  };

  const handleDelete = async () => {
    if (!accountId) return;
    const targets = isMultiSelect
      ? sameAccountSelection
      : [message];
    const keys = new Set(
      targets.map((t) => messageSelectionKey(t.account_id ?? accountId, t.uid, t.folder_name ?? folder)),
    );
    removeMessages(keys);

    const uidSet = new Set<number>();
    await Promise.all(
      targets.map(async (t) => {
        const expanded = await uidsForThreadAction(t, accountId, folder);
        for (const u of expanded) uidSet.add(u);
      }),
    );
    const uids = Array.from(uidSet);
    if (uids.length === 0) return;
    try {
      await api.messages.delete(accountId, folder, uids);
    } catch (e) {
      console.error("Failed to delete:", e);
      addToast({ message: "Failed to move to Trash", type: "error" });
    }
  };

  const handleMoveToInbox = async () => {
    if (!accountId) return;
    const targets = isMultiSelect ? sameAccountSelection : [message];
    const keys = new Set(
      targets.map((t) => messageSelectionKey(t.account_id ?? accountId, t.uid, t.folder_name ?? folder)),
    );
    removeMessages(keys);

    // No thread expansion: in Gmail's All Mail a thread includes your own
    // sent replies — expanding would move those into the Inbox too.
    const uids = targets.map((t) => t.uid);
    try {
      // Gmail supports UID MOVE (RFC 6851), so move_message never hits its
      // COPY+\Deleted+EXPUNGE fallback — expunging from All Mail would
      // trash the message instead of unarchiving it.
      await api.messages.move(accountId, folder, uids, "INBOX");
      addToast({
        message: uids.length > 1 ? `Moved ${uids.length} to Inbox` : "Moved to Inbox",
        type: "success",
      });
    } catch (e) {
      console.error("Failed to move to Inbox:", e);
      addToast({ message: "Failed to move to Inbox", type: "error" });
    }
  };

  const handleCategoryChange = async (category: EmailCategory) => {
    if (!accountId) return;

    if (isMultiSelect) {
      const selectedKeys = new Set(
        sameAccountSelection.map((selected) =>
          messageSelectionKey(selected.account_id ?? accountId, selected.uid, selected.folder_name ?? folder),
        ),
      );
      const uids = sameAccountSelection.map((selected) => selected.uid);
      updateMessagesCategory(selectedKeys, category);
      try {
        await api.categories.setBatchCategory(accountId, folder, uids, category);
        const counts = await api.categories.getCounts(selectedAccountId);
        setCategoryCounts(counts);
      } catch (e) {
        console.error("Failed to update categories:", e);
      }
    } else {
      if (category === currentCategory) return;
      updateMessageCategory(message.uid, category, accountId);
      try {
        await api.categories.setCategory(accountId, folder, message.uid, category);
        const counts = await api.categories.getCounts(selectedAccountId);
        setCategoryCounts(counts);
      } catch (e) {
        console.error("Failed to update category:", e);
      }
    }
  };

  return (
    <ContextMenu.Root
      onOpenChange={(open) => {
        setMenuOpen(open);
        if (open) {
          // Snapshot Option state at open time. The useEffect above will
          // also flip altHeld to true if Option is pressed AFTER open.
          altHeldRef.current = isOptionPressed;
          setAltHeld(isOptionPressed);
        } else {
          altHeldRef.current = false;
          setAltHeld(false);
        }
      }}
    >
      <ContextMenu.Trigger asChild>
        <div className={triggerClassName}>{children}</div>
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="z-50 min-w-[180px] rounded-lg border border-border bg-base-solid p-1 shadow-xl">
          <ContextMenu.Item
            onSelect={() => {
              if (altHeldRef.current) handleCopyClaudePrompt();
              else handleOpenInClaude();
            }}
            disabled={!accountId}
            className={cn(
              "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm outline-none",
              accountId
                ? "text-content-secondary hover:bg-surface hover:text-content"
                : "cursor-not-allowed text-content-muted opacity-50"
            )}
          >
            {altHeld ? (
              <Clipboard className="h-4 w-4 shrink-0" />
            ) : (
              <Sparkles className="h-4 w-4 shrink-0" />
            )}
            <span className="flex-1">
              {altHeld ? "Copy Claude Prompt" : "Open in Claude"}
            </span>
          </ContextMenu.Item>
          <ContextMenu.Item
            onSelect={() => {
              if (accountId) void useChatStore.getState().askAboutEmail({ account_id: accountId, folder, uid: message.uid });
            }}
            disabled={!accountId}
            className={cn(
              "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm outline-none",
              accountId
                ? "text-content-secondary hover:bg-surface hover:text-content"
                : "cursor-not-allowed text-content-muted opacity-50"
            )}
          >
            <MessageSquare className="h-4 w-4 shrink-0" />
            <span className="flex-1">Ask Claude in Chat</span>
          </ContextMenu.Item>
          <ContextMenu.Separator className="my-1 h-px bg-border" />
          <ContextMenu.Label className="px-2 py-1.5 text-xs font-medium text-content-muted">
            {isMultiSelect
              ? `Move ${sameAccountSelection.length} messages to`
              : "Move to category"}
          </ContextMenu.Label>
          {CATEGORIES.map((cat) => {
            const isActive = !isMultiSelect && currentCategory === cat.id;
            const Icon = cat.icon;
            return (
              <ContextMenu.Item
                key={cat.id}
                onSelect={() => handleCategoryChange(cat.id)}
                disabled={isActive}
                className={cn(
                  "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm outline-none",
                  isActive
                    ? "text-accent"
                    : "text-content-secondary hover:bg-surface hover:text-content"
                )}
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span className="flex-1">{cat.label}</span>
                {isActive && <Check className="h-3.5 w-3.5 shrink-0" />}
              </ContextMenu.Item>
            );
          })}
          <ContextMenu.Separator className="my-1 h-px bg-border" />
          <ContextMenu.Item
            onSelect={handleUnsubscribe}
            className="flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm text-content-secondary outline-none hover:bg-surface hover:text-content"
          >
            <MailMinus className="h-4 w-4 shrink-0" />
            <span className="flex-1">Unsubscribe</span>
          </ContextMenu.Item>
          {message.from_email && (
            <ContextMenu.Item
              onSelect={handleUnsubscribeSender}
              className="flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm text-content-secondary outline-none hover:bg-surface hover:text-content"
            >
              <UserMinus className="h-4 w-4 shrink-0" />
              <span className="flex-1">Unsubscribe from sender</span>
            </ContextMenu.Item>
          )}
          <ContextMenu.Separator className="my-1 h-px bg-border" />
          {isArchiveFolder && (
            <ContextMenu.Item
              onSelect={handleMoveToInbox}
              disabled={!accountId}
              className={cn(
                "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm outline-none",
                accountId
                  ? "text-content-secondary hover:bg-surface hover:text-content"
                  : "cursor-not-allowed text-content-muted opacity-50"
              )}
            >
              <Inbox className="h-4 w-4 shrink-0" />
              <span className="flex-1">
                {isMultiSelect
                  ? `Move ${sameAccountSelection.length} to Inbox`
                  : "Move to Inbox"}
              </span>
            </ContextMenu.Item>
          )}
          <ContextMenu.Item
            onSelect={handleDelete}
            disabled={!accountId}
            className={cn(
              "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm outline-none",
              accountId
                ? "text-content-secondary hover:bg-surface hover:text-content"
                : "cursor-not-allowed text-content-muted opacity-50"
            )}
          >
            <Trash2 className="h-4 w-4 shrink-0" />
            <span className="flex-1">
              {isMultiSelect
                ? `Move ${sameAccountSelection.length} to Trash`
                : "Move to Trash"}
            </span>
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
