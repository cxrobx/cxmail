import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import { useChatStore } from "@/stores/chatStore";
import {
  Search,
  Inbox,
  Send,
  FileText,
  Trash2,
  Archive,
  PenSquare,
  User,
  Sun,
  Monitor,
  Moon,
  SidebarClose,
  Layers,
  Rows3,
  RefreshCw,
  Mail,
  Clock,
  BellRing,
  CalendarDays,
  Reply,
  Pin,
  Star,
  BellOff,
  MailOpen,
  Sparkles,
  Bell,
  Users,
  Tag,
  ShieldAlert,
  Settings,
} from "lucide-react";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { rankCommands } from "@/lib/commandScore";
import { archiveMessage, deleteMessage } from "@/lib/messageActions";
import { getLaterToday, getNextMonday, getTomorrowMorning } from "@/lib/snoozePresets";
import { commitGlobalSearch } from "@/lib/searchFilters";
import Kbd from "@/components/shared/Kbd";
import type { EmailCategory, SearchResult } from "@/types/email";

interface Command {
  id: string;
  label: string;
  category: string;
  icon: typeof Search;
  action: () => void;
  keywords?: string;
  /** Keyboard-shortcut hint rendered as right-aligned keycaps. */
  keys?: string[];
  /** Muted secondary line (used by inline search hits for the sender). */
  sublabel?: string;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

export default function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [isDefaultMail, setIsDefaultMail] = useState(false);
  const [hits, setHits] = useState<SearchResult[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const { accounts } = useAccountStore();
  const {
    foldersByAccount,
    setSelectedFolder,
    setUnifiedInbox,
    setComposing,
    setSpecialView,
    setSelectedCategory,
    setAccountGroupInbox,
    setSelectedGroup,
    inboxGroups,
    selectedMessageUid,
    selectedMessageAccountId,
    selectedMessageFolder,
    selectedAccountId,
    selectedFolder,
    specialView,
    messages,
    setSelectedMessage,
    toggleMessagePin,
    toggleMessageMute,
    markMessageRead,
    markMessageUnread,
  } = useMailStore();
  const {
    toggleSidebar,
    setTheme,
    theme,
    setDensity,
    density,
    setSettingsOpen,
    recentCommandIds,
    addRecentCommand,
  } = useUIStore();

  // Build command list
  const commands = useMemo<Command[]>(() => {
    const cmds: Command[] = [];

    // Message actions for the selected message — rendered first. Target
    // resolution deliberately diverges from useKeyboardShortcuts by preferring
    // selectedMessageFolder (search results span folders; the hook doesn't).
    const actionAccountId = selectedMessageAccountId ?? selectedAccountId;
    const actionFolder =
      selectedMessageFolder ?? selectedFolder ?? (selectedMessageAccountId ? "INBOX" : null);
    const selectedMessage = messages.find(
      (m) =>
        m.uid === selectedMessageUid
        && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
    );

    if (selectedMessageUid !== null && actionAccountId && actionFolder) {
      const uid = selectedMessageUid;
      const acct = actionAccountId;
      const folder = actionFolder;
      const ctx = { accountId: acct, folder, uid, selectedMessageAccountId };

      cmds.push({
        id: "msg-reply",
        label: "Reply",
        category: "Message",
        icon: Reply,
        // Parity with the "r" shortcut; ReadingPane owns quoting/prefill.
        action: () => { onClose(); window.dispatchEvent(new CustomEvent("cxmail:compose-reply")); },
        keywords: "reply respond answer",
        keys: ["R"],
      });

      cmds.push({
        id: "msg-archive",
        label: "Archive",
        category: "Message",
        icon: Archive,
        action: () => { onClose(); void archiveMessage(ctx); },
        keywords: "archive remove inbox",
        keys: ["⌘", "E"],
      });

      cmds.push({
        id: "msg-delete",
        label: "Delete",
        category: "Message",
        icon: Trash2,
        action: () => { onClose(); void deleteMessage(ctx); },
        keywords: "delete trash remove",
        keys: ["#"],
      });

      // Label-dependent commands are omitted (not mislabeled) when the row
      // isn't in the current list — e.g. selection made from a search hit.
      if (selectedMessage) {
        cmds.push({
          id: "msg-toggle-pin",
          label: selectedMessage.is_pinned ? "Unpin" : "Pin",
          category: "Message",
          icon: Pin,
          action: () => {
            onClose();
            api.messages
              .togglePin(acct, folder, uid, !selectedMessage.is_pinned)
              .catch(console.error);
            toggleMessagePin(uid, acct);
          },
          keywords: "pin unpin stick top",
          keys: ["P"],
        });

        cmds.push({
          id: "msg-toggle-star",
          label: selectedMessage.is_flagged ? "Unstar" : "Star",
          category: "Message",
          icon: Star,
          action: () => {
            onClose();
            api.messages
              .toggleStar(acct, folder, uid, !selectedMessage.is_flagged)
              .catch(console.error);
          },
          keywords: "star flag favorite",
          keys: ["S"],
        });
      }

      cmds.push({
        id: "msg-mute",
        label: "Mute",
        category: "Message",
        icon: BellOff,
        action: () => {
          onClose();
          api.messages.toggleMute(acct, folder, uid, true).catch(console.error);
          toggleMessageMute(uid, acct);
          // Advance selection — mirrors the "m" shortcut in useKeyboardShortcuts.
          const idx = messages.findIndex(
            (m) =>
              m.uid === uid
              && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
          );
          const next = messages[idx + 1] || messages[idx - 1];
          setSelectedMessage(next?.uid ?? null, next?.account_id);
        },
        keywords: "mute silence thread notifications",
        keys: ["M"],
      });

      if (selectedMessage) {
        cmds.push({
          id: "msg-toggle-read",
          label: selectedMessage.is_read ? "Mark as Unread" : "Mark as Read",
          category: "Message",
          icon: MailOpen,
          action: () => {
            onClose();
            if (selectedMessage.is_read) {
              api.messages.markUnread(acct, folder, [uid]).catch(console.error);
              markMessageUnread(uid, acct);
            } else {
              api.messages.markRead(acct, folder, [uid]).catch(console.error);
              markMessageRead(uid, acct);
            }
          },
          keywords: "read unread mark seen",
        });
      }

      const snoozePresets = [
        { id: "msg-snooze-later", label: "Snooze: Later Today", icon: Clock, when: getLaterToday },
        { id: "msg-snooze-tomorrow", label: "Snooze: Tomorrow", icon: Sun, when: getTomorrowMorning },
        { id: "msg-snooze-next-week", label: "Snooze: Next Week", icon: CalendarDays, when: getNextMonday },
      ];
      for (const preset of snoozePresets) {
        cmds.push({
          id: preset.id,
          label: preset.label,
          category: "Message",
          icon: preset.icon,
          // Mirrors ReadingPane's SnoozePopover handler: snooze, then clear
          // the selection so the reading pane doesn't show a gone message.
          action: async () => {
            onClose();
            try {
              await api.snooze.snooze(acct, folder, uid, preset.when().toISOString());
              useMailStore.getState().setSelectedMessage(null);
            } catch (e) {
              console.error("Failed to snooze:", e);
            }
          },
          keywords: "snooze later remind defer",
        });
      }

      cmds.push({
        id: "msg-open-claude",
        label: "Open in Claude",
        category: "Message",
        icon: Sparkles,
        action: () => { onClose(); api.claude.openEmail(acct, folder, uid).catch(console.error); },
        keywords: "claude ai assistant handoff",
      });
      cmds.push({
        id: "msg-ask-claude-chat",
        label: "Ask Claude About This Email",
        category: "Message",
        icon: Sparkles,
        action: () => { onClose(); void useChatStore.getState().askAboutEmail({ account_id: acct, folder, uid }); },
        keywords: "claude chat ai assistant ask",
      });
    }

    // Compose
    cmds.push({
      id: "compose",
      label: "New Message",
      category: "Actions",
      icon: PenSquare,
      action: () => { setComposing(true); onClose(); },
      keywords: "compose write email new",
      keys: ["⌘", "N"],
    });

    // Unified inbox
    if (accounts.length > 1) {
      cmds.push({
        id: "unified-inbox",
        label: "All Inboxes",
        category: "Navigation",
        icon: Layers,
        action: () => { setUnifiedInbox(); onClose(); },
        keywords: "unified combined all",
      });
    }

    // Special views (sidebar icon parity: Clock/Send/BellRing/CalendarDays)
    const specialViews = [
      { id: "view-snoozed", label: "Snoozed", view: "snoozed" as const, icon: Clock, keywords: "snoozed sleeping wake" },
      { id: "view-scheduled", label: "Scheduled", view: "scheduled" as const, icon: Send, keywords: "scheduled send later outgoing" },
      { id: "view-followups", label: "Follow-ups", view: "followups" as const, icon: BellRing, keywords: "followup follow up reminders" },
      { id: "view-calendar", label: "Calendar", view: "calendar" as const, icon: CalendarDays, keywords: "calendar events meetings" },
    ];
    for (const view of specialViews) {
      cmds.push({
        id: view.id,
        label: view.label,
        category: "Navigation",
        icon: view.icon,
        action: () => { setSpecialView(view.view); onClose(); },
        keywords: view.keywords,
      });
    }

    // Category tabs (ids/icons mirror CategoryTabs.tsx). Gated on
    // !specialView: setSelectedCategory doesn't clear specialView, and
    // MessageList renders special views before category filters — running
    // one from Snoozed/Scheduled/etc. would be an invisible no-op.
    if (!specialView) {
      const categories: { id: EmailCategory | null; label: string; icon: typeof Inbox }[] = [
        { id: null, label: "All", icon: Inbox },
        { id: "primary", label: "Primary", icon: User },
        { id: "updates", label: "Updates", icon: Bell },
        { id: "social", label: "Social", icon: Users },
        { id: "promotions", label: "Promotions", icon: Tag },
        { id: "junk", label: "Junk", icon: ShieldAlert },
      ];
      for (const cat of categories) {
        cmds.push({
          id: `category-${cat.id ?? "all"}`,
          label: `Category: ${cat.label}`,
          category: "Navigation",
          icon: cat.icon,
          action: () => { setSelectedCategory(cat.id); onClose(); },
          keywords: `category tab filter ${cat.label}`,
        });
      }
    }

    // Account groups (unique group_name values, in account order)
    const seenGroups = new Set<string>();
    for (const account of accounts) {
      const name = account.group_name;
      if (!name || seenGroups.has(name)) continue;
      seenGroups.add(name);
      cmds.push({
        id: `account-group-${name}`,
        label: `Group: ${name}`,
        category: "Navigation",
        icon: Users,
        action: () => { setAccountGroupInbox(name); onClose(); },
        keywords: `account group inbox ${name}`,
      });
    }

    // Smart inbox groups (rules-based; empty until Sidebar loads them)
    for (const group of inboxGroups) {
      cmds.push({
        id: `smart-group-${group.id}`,
        label: `Smart Group: ${group.name}`,
        category: "Navigation",
        icon: Layers,
        action: () => { setSelectedGroup(group.id); onClose(); },
        keywords: `smart inbox group rules ${group.name}`,
      });
    }

    // Per-account folders
    for (const account of accounts) {
      const folders = foldersByAccount[account.id] || [];
      for (const folder of folders) {
        const iconMap: Record<string, typeof Inbox> = {
          inbox: Inbox, sent: Send, drafts: FileText,
          trash: Trash2, archive: Archive,
        };
        const displayName = folder.name === "INBOX" ? "Inbox" : folder.name.replace("[Gmail]/", "");
        cmds.push({
          id: `folder-${account.id}-${folder.name}`,
          label: `${displayName}`,
          category: accounts.length > 1 ? account.email : "Folders",
          icon: iconMap[folder.folder_type || ""] || Inbox,
          action: () => { setSelectedFolder(folder.name, account.id); onClose(); },
          keywords: `folder ${displayName} ${account.email}`,
        });
      }
    }

    // Account switching
    for (const account of accounts) {
      cmds.push({
        id: `account-${account.id}`,
        label: account.email,
        category: "Accounts",
        icon: User,
        action: () => { setSelectedFolder("INBOX", account.id); onClose(); },
        keywords: `account switch ${account.email} ${account.provider}`,
      });
    }

    // UI actions
    cmds.push({
      id: "chat-claude",
      label: "Chat with Claude",
      category: "Actions",
      icon: Sparkles,
      action: () => { onClose(); useChatStore.getState().setOpen(true); },
      keywords: "claude chat ai assistant ask agent",
      keys: ["⌘", "L"],
    });

    cmds.push({
      id: "toggle-sidebar",
      label: "Toggle Sidebar",
      category: "View",
      icon: SidebarClose,
      action: () => { toggleSidebar(); onClose(); },
      keywords: "sidebar hide show panel",
      keys: ["⌘", "\\"],
    });

    cmds.push({
      id: "open-settings",
      label: "Settings",
      category: "View",
      icon: Settings,
      action: () => { setSettingsOpen(true); onClose(); },
      keywords: "settings preferences appearance theme density transparency glass translucent opacity window",
    });

    // Cycles dark → light → system → dark. The label and icon describe the
    // NEXT state, not the current one, so the palette row reads as the action
    // it performs — same as it did when this was a two-way toggle.
    const nextTheme = theme === "dark" ? "light" : theme === "light" ? "system" : "dark";
    const NEXT_THEME_LABEL = { dark: "Dark", light: "Light", system: "System" } as const;
    const NEXT_THEME_ICON = { dark: Moon, light: Sun, system: Monitor } as const;
    cmds.push({
      id: "toggle-theme",
      label: `Switch to ${NEXT_THEME_LABEL[nextTheme]} Theme`,
      category: "View",
      icon: NEXT_THEME_ICON[nextTheme],
      action: () => { setTheme(nextTheme); onClose(); },
      keywords: "theme dark light system auto mode appearance",
    });

    const densities = [
      { id: "comfortable", label: "Comfortable" },
      { id: "compact", label: "Compact" },
      { id: "ultra-compact", label: "Ultra Compact" },
    ] as const;
    for (const d of densities) {
      if (d.id !== density) {
        cmds.push({
          id: `density-${d.id}`,
          label: `Density: ${d.label}`,
          category: "View",
          icon: Rows3,
          action: () => { setDensity(d.id); onClose(); },
          keywords: `density ${d.label} spacing`,
        });
      }
    }

    // Sync actions
    cmds.push({
      id: "force-full-sync",
      label: "Force Full Sync",
      category: "Sync",
      icon: RefreshCw,
      action: async () => {
        onClose();
        const { setSyncing, setSyncCompleted, setSyncError } = useMailStore.getState();
        setSyncing(true);
        try {
          await Promise.race([
            api.messages.forceFullSync(),
            new Promise<never>((_, reject) =>
              setTimeout(() => reject(new Error("Force sync timed out after 4 minutes")), 240_000)
            ),
          ]);
          setSyncCompleted();
          window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
        } catch (e) {
          setSyncError(e instanceof Error ? e.message : "Force sync failed");
        }
      },
      keywords: "sync refresh reload force reset checkpoint",
    });

    // Set as default email client (mailto: handler)
    cmds.push({
      id: "set-default-mail",
      label: isDefaultMail
        ? "CXMail is Your Default Email Client ✓"
        : "Set CXMail as Default Email Client",
      category: "Actions",
      icon: Mail,
      action: async () => {
        onClose();
        try {
          const ok = await api.system.setAsDefaultMailClient();
          if (ok) setIsDefaultMail(true);
          useUIStore.getState().addToast({
            message: ok
              ? "Requested — confirm in the system prompt if one appears."
              : "Couldn't set CXMail as default email client.",
            type: ok ? "info" : "error",
            duration: 5000,
          });
        } catch (e) {
          useUIStore.getState().addToast({
            message: `Failed to set default email client: ${e}`,
            type: "error",
            duration: 5000,
          });
        }
      },
      keywords: "default mail client mailto macOS email handler set",
    });

    return cmds;
  }, [
    accounts,
    foldersByAccount,
    theme,
    density,
    isDefaultMail,
    inboxGroups,
    messages,
    selectedMessageUid,
    selectedMessageAccountId,
    selectedMessageFolder,
    selectedAccountId,
    selectedFolder,
    specialView,
    setSelectedFolder,
    setUnifiedInbox,
    setComposing,
    setSpecialView,
    setSelectedCategory,
    setAccountGroupInbox,
    setSelectedGroup,
    setSelectedMessage,
    toggleMessagePin,
    toggleMessageMute,
    markMessageRead,
    markMessageUnread,
    toggleSidebar,
    setTheme,
    setDensity,
    onClose,
  ]);

  const trimmedQuery = query.trim();

  // Debounced inline FTS hits (SearchBar's own idiom: setTimeout + cancelled
  // flag as the stale guard).
  useEffect(() => {
    if (!open || trimmedQuery.length < 2) {
      setHits([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const results = await api.messages.search(trimmedQuery, { prefix: true, limit: 5 });
        if (!cancelled) setHits(results);
      } catch {
        if (!cancelled) setHits([]);
      }
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [open, trimmedQuery]);

  const hitCommands = useMemo<Command[]>(
    () =>
      hits.map((hit) => ({
        id: `hit-${hit.account_id}-${hit.folder_name}-${hit.uid}`,
        label: hit.subject || "(no subject)",
        // NOT the snippet — it carries U+E000/E001 highlight markers.
        sublabel: hit.from_name || hit.from_email || undefined,
        category: "Messages",
        icon: Mail,
        action: async () => {
          onClose();
          // Commit first (setSearchResults clears the selection), then the
          // 3-arg select records the hit's real folder for the reading pane.
          await commitGlobalSearch(trimmedQuery);
          useMailStore.getState().setSelectedMessage(hit.uid, hit.account_id, hit.folder_name);
        },
      })),
    [hits, trimmedQuery, onClose],
  );

  const commandById = useMemo(
    () => new Map(commands.map((c) => [c.id, c] as const)),
    [commands],
  );

  // `recent:`-prefixed clones avoid duplicate React keys against the
  // originals below; stale ids (deleted folders/groups) drop silently.
  const recentCommands = useMemo<Command[]>(
    () =>
      recentCommandIds
        .map((id) => {
          const cmd = commandById.get(id);
          return cmd ? { ...cmd, id: `recent:${id}`, category: "Recent" } : null;
        })
        .filter((c): c is Command => c !== null)
        .slice(0, 5),
    [recentCommandIds, commandById],
  );

  // Final list. Empty query: recents + everything. Non-empty: ranked commands
  // (regrouped contiguously by category, insertion order = first appearance in
  // score order, so the consecutive-category render works unchanged), then
  // inline message hits, then the always-present search fallthrough — the
  // latter two are never fuzzy-scored.
  const filtered = useMemo(() => {
    if (!trimmedQuery) return [...recentCommands, ...commands];
    const ranked = rankCommands(query, commands);
    const byCategory = new Map<string, Command[]>();
    for (const cmd of ranked) {
      const list = byCategory.get(cmd.category);
      if (list) list.push(cmd);
      else byCategory.set(cmd.category, [cmd]);
    }
    const searchCommand: Command = {
      id: "search-query",
      label: `Search mail for "${trimmedQuery}"`,
      category: "Search",
      icon: Search,
      action: () => {
        onClose();
        void commitGlobalSearch(trimmedQuery);
      },
    };
    return [...[...byCategory.values()].flat(), ...hitCommands, searchCommand];
  }, [commands, query, trimmedQuery, hitCommands, recentCommands, onClose]);

  // Runs a command and records it for the Recent group. Synthetic entries
  // (inline hits, the search fallthrough) are excluded; Recent clones record
  // their underlying id.
  const runCommand = useCallback(
    (cmd: Command) => {
      const baseId = cmd.id.startsWith("recent:") ? cmd.id.slice("recent:".length) : cmd.id;
      if (!baseId.startsWith("hit-") && baseId !== "search-query") {
        addRecentCommand(baseId);
      }
      cmd.action();
    },
    [addRecentCommand],
  );

  // Reset on open (focus is handled by Dialog.Content's onOpenAutoFocus)
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      // Refresh the default-mail-client checkmark each time the palette opens.
      api.system.isDefaultMailClient().then(setIsDefaultMail).catch(() => {});
    }
  }, [open]);

  // Reset index when the user edits the query; only clamp when the list
  // length shifts for other reasons (async hits arriving) so arrow-key
  // position isn't yanked back to the top.
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);
  useEffect(() => {
    setSelectedIndex((i) => Math.min(i, Math.max(filtered.length - 1, 0)));
  }, [filtered.length]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" && filtered[selectedIndex]) {
      e.preventDefault();
      runCommand(filtered[selectedIndex]);
    }
    // Escape is handled by Radix Dialog (onOpenChange → onClose).
  }, [filtered, selectedIndex, runCommand]);

  // Scroll selected into view
  useEffect(() => {
    const el = listRef.current?.querySelector(`[data-index="${selectedIndex}"]`) as HTMLElement | null;
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  // Group by category
  const grouped: { category: string; items: (Command & { globalIndex: number })[] }[] = [];
  let currentCat = "";
  filtered.forEach((cmd, i) => {
    if (cmd.category !== currentCat) {
      currentCat = cmd.category;
      grouped.push({ category: currentCat, items: [] });
    }
    grouped[grouped.length - 1].items.push({ ...cmd, globalIndex: i });
  });

  return (
    <Dialog.Root open={open} onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[100]" />
        <Dialog.Content
          className="fixed left-1/2 top-[15%] z-[101] w-[500px] -translate-x-1/2 overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
          onOpenAutoFocus={(e) => {
            e.preventDefault();
            inputRef.current?.focus();
          }}
          onKeyDown={handleKeyDown}
        >
          <Dialog.Title className="sr-only">Command palette</Dialog.Title>
          <Dialog.Description className="sr-only">
            Search commands, folders, and mail
          </Dialog.Description>
        {/* Input */}
        <div className="flex items-center gap-2 border-b border-border-subtle px-4 py-3">
          <Search className="h-4 w-4 shrink-0 text-content-muted" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Type a command..."
            className="flex-1 bg-transparent text-sm text-content placeholder-content-muted outline-none"
          />
        </div>

        {/* Results */}
        <div ref={listRef} className="max-h-[320px] overflow-auto py-1">
          {filtered.length === 0 && (
            <p className="px-4 py-6 text-center text-sm text-content-muted">No results</p>
          )}
          {grouped.map((group, gi) => (
            // Index-qualified key: the same category can legitimately appear in
            // two non-adjacent blocks (e.g. "Actions" split around Navigation).
            <div key={`${group.category}-${gi}`}>
              <p className="px-4 pt-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-content-faint">
                {group.category}
              </p>
              {group.items.map((cmd) => {
                const Icon = cmd.icon;
                return (
                  <button
                    key={cmd.id}
                    data-index={cmd.globalIndex}
                    onClick={() => runCommand(cmd)}
                    className={cn(
                      "flex w-full items-center gap-3 px-4 py-2 text-left text-sm transition-colors",
                      cmd.globalIndex === selectedIndex
                        ? "bg-accent/15 text-content"
                        : "text-content-secondary hover:bg-surface"
                    )}
                  >
                    <Icon className="h-4 w-4 shrink-0 text-content-muted" />
                    <span className="flex min-w-0 flex-1 items-baseline gap-2">
                      <span className="truncate">{cmd.label}</span>
                      {cmd.sublabel && (
                        <span className="truncate text-xs text-content-muted">
                          {cmd.sublabel}
                        </span>
                      )}
                    </span>
                    {cmd.keys && (
                      <span className="flex shrink-0 items-center gap-0.5">
                        {cmd.keys.map((key, i) => (
                          <Kbd key={i}>{key}</Kbd>
                        ))}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          ))}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
