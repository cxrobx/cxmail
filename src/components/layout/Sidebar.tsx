import { useEffect, useState, useCallback, useRef } from "react";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import { api } from "@/lib/tauri";
import type { VoiceProfileStatus, InsightStatus, Archetype } from "@/types/email";
import AccountBadge from "@/components/accounts/AccountBadge";
import FolderTree from "@/components/mail/FolderTree";
import InboxGroupEditor from "@/components/mail/InboxGroupEditor";
import ClaudeRepoSettings from "@/components/mail/ClaudeRepoSettings";
import { cn } from "@/lib/utils";
import { isFailing } from "@/lib/syncHealth";
import {
  Inbox, Plus, RefreshCw, Clock, Send, BellRing, BellOff, CalendarDays, ListChecks, Terminal,
  Briefcase, Music, User, Folder, Tag, GripVertical,
  ChevronRight, FolderOpen, X, Sparkles, Loader2, BookOpen, Eye, EyeOff, Ungroup, Group,
  type LucideIcon,
} from "lucide-react";

const GROUP_ICONS: Record<string, LucideIcon> = {
  briefcase: Briefcase,
  music: Music,
  user: User,
  folder: Folder,
  tag: Tag,
};

export default function Sidebar() {
  const { accounts } = useAccountStore();
  const {
    selectedAccountId,
    setSelectedAccount,
    foldersByAccount,
    setAccountFolders,
    isUnifiedInbox,
    setUnifiedInbox,
    setAccountGroupInbox,
    selectedAccountGroup,
    specialView,
    setSpecialView,
    setSelectedFolder,
    selectedGroupId,
    setSelectedGroup,
    inboxGroups,
    setInboxGroups,
    isSyncing,
    syncHealthByAccount,
  } = useMailStore();

  const [collapsedAccounts, setCollapsedAccounts] = useState<Set<string>>(
    () => new Set(accounts.map((a) => a.id))
  );
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set());
  const [editingGroup, setEditingGroup] = useState<number | "new" | null>(null);
  const [showingClaudeRepos, setShowingClaudeRepos] = useState(false);
  const { setShowingAddAccount, setAccounts } = useAccountStore();

  // Account group state
  const [creatingGroup, setCreatingGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [accountContextMenu, setAccountContextMenu] = useState<{ x: number; y: number; accountId: string } | null>(null);
  const groupInputRef = useRef<HTMLInputElement>(null);

  // Voice profile state (per account)
  const [voiceStatuses, setVoiceStatuses] = useState<Record<string, VoiceProfileStatus>>({});
  const [insightStatuses, setInsightStatuses] = useState<Record<string, InsightStatus>>({});
  const [extractingVoiceId, setExtractingVoiceId] = useState<string | null>(null);
  const [learningEditsId, setLearningEditsId] = useState<string | null>(null);
  const [archetypesByAccount, setArchetypesByAccount] = useState<Record<string, Archetype[]>>({});
  const [clusteringId, setClusteringId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  // Count of pending scheduled sends — shown as a badge so a queued email is
  // glanceable without opening the Scheduled view. Refreshes on mount, whenever
  // a send is scheduled/cancelled/sent (cxmail:scheduled-changed), and on the
  // backend's scheduled-send-processed tick (re-broadcast by AppLayout).
  const [scheduledCount, setScheduledCount] = useState(0);
  const [needsYouCount, setNeedsYouCount] = useState(0);
  useEffect(() => {
    let alive = true;
    const refresh = () => {
      api.schedule
        .list()
        .then((rows) => { if (alive) setScheduledCount(rows.length); })
        .catch(() => {});
    };
    refresh();
    window.addEventListener("cxmail:scheduled-changed", refresh);
    return () => {
      alive = false;
      window.removeEventListener("cxmail:scheduled-changed", refresh);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const refresh = () => {
      api.needsYou.list()
        .then((rows) => { if (alive) setNeedsYouCount(rows.length); })
        .catch(() => {});
    };
    refresh();
    window.addEventListener("cxmail:needs-you-changed", refresh);
    window.addEventListener("cxmail:refresh-messages", refresh);
    return () => {
      alive = false;
      window.removeEventListener("cxmail:needs-you-changed", refresh);
      window.removeEventListener("cxmail:refresh-messages", refresh);
    };
  }, []);

  // Fetch voice profile + insight status + archetype list when menu opens on an account we haven't checked
  useEffect(() => {
    if (!accountContextMenu) return;
    const id = accountContextMenu.accountId;
    if (voiceStatuses[id] === undefined) {
      api.ai.getVoiceProfileStatus(id)
        .then((status) => setVoiceStatuses((prev) => ({ ...prev, [id]: status })))
        .catch(() => {});
    }
    if (insightStatuses[id] === undefined) {
      api.ai.getInsightStatus(id)
        .then((status) => setInsightStatuses((prev) => ({ ...prev, [id]: status })))
        .catch(() => {});
    }
    if (archetypesByAccount[id] === undefined) {
      api.ai.listArchetypes(id)
        .then((list) => setArchetypesByAccount((prev) => ({ ...prev, [id]: list })))
        .catch(() => {});
    }
  }, [accountContextMenu, voiceStatuses, insightStatuses, archetypesByAccount]);

  const handleRebuildVoiceProfile = useCallback(async (accountId: string) => {
    setExtractingVoiceId(accountId);
    setAccountContextMenu(null);
    addToast({ message: "Analyzing sent mail to build voice profile…", type: "info" });
    try {
      const status = await api.ai.extractVoiceProfile(accountId);
      setVoiceStatuses((prev) => ({ ...prev, [accountId]: status }));
      addToast({
        message: `Voice profile built from ${status.sample_count} sent messages`,
        type: "success",
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      addToast({ message: `Voice profile failed: ${msg}`, type: "error" });
    } finally {
      setExtractingVoiceId(null);
    }
  }, [addToast]);

  const handleClusterArchetypes = useCallback(
    async (accountId: string, recluster: boolean) => {
      if (
        recluster &&
        !window.confirm(
          "Re-clustering replaces the existing voice archetypes for this account. Continue?",
        )
      ) {
        setAccountContextMenu(null);
        return;
      }
      setClusteringId(accountId);
      setAccountContextMenu(null);
      addToast({
        message: recluster
          ? "Re-clustering sent mail into archetypes…"
          : "Clustering sent mail into archetypes…",
        type: "info",
      });
      try {
        const list = recluster
          ? await api.ai.reclusterArchetypes(accountId)
          : await api.ai.extractArchetypes(accountId);
        setArchetypesByAccount((prev) => ({ ...prev, [accountId]: list }));
        const names = list.map((a) => a.name).slice(0, 4).join(", ");
        addToast({
          message:
            list.length > 0
              ? `Found ${list.length} archetypes${names ? `: ${names}` : ""}`
              : "No archetypes detected",
          type: "success",
        });
      } catch (e: unknown) {
        const msg = e instanceof Error ? e.message : String(e);
        addToast({ message: `Archetype clustering failed: ${msg}`, type: "error" });
      } finally {
        setClusteringId(null);
      }
    },
    [addToast],
  );

  const handleLearnFromEdits = useCallback(async (accountId: string) => {
    setLearningEditsId(accountId);
    setAccountContextMenu(null);
    addToast({ message: "Analyzing your edits to learn preferences…", type: "info" });
    try {
      const result = await api.ai.learnFromEdits(accountId);
      const status = await api.ai.getInsightStatus(accountId);
      setInsightStatuses((prev) => ({ ...prev, [accountId]: status }));
      addToast({
        message: `Found ${result.extracted} patterns, ${result.activated} activated`,
        type: "success",
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      addToast({ message: `Learning failed: ${msg}`, type: "error" });
    } finally {
      setLearningEditsId(null);
    }
  }, [addToast]);

  // Compute unique group names from accounts
  const accountGroups = (() => {
    const groups: string[] = [];
    const seen = new Set<string>();
    for (const a of accounts) {
      if (a.group_name && !seen.has(a.group_name)) {
        seen.add(a.group_name);
        groups.push(a.group_name);
      }
    }
    return groups;
  })();

  const ungroupedAccounts = accounts.filter((a) => !a.group_name);
  const groupedAccountsMap = new Map<string, typeof accounts>();
  for (const a of accounts) {
    if (a.group_name) {
      const list = groupedAccountsMap.get(a.group_name) || [];
      list.push(a);
      groupedAccountsMap.set(a.group_name, list);
    }
  }

  // Sidebar display order: ungrouped accounts first, then each group's members.
  // Drag/reorder works in THIS space — a flat-array reorder is invisible once
  // rendering re-partitions rows by group, so indices, refs, and the persisted
  // order all follow display order.
  const displayAccounts = [
    ...ungroupedAccounts,
    ...accountGroups.flatMap((g) => groupedAccountsMap.get(g) || []),
  ];
  const displayIndexById = new Map(displayAccounts.map((a, i) => [a.id, i]));

  // Option+drag reorder state. Refs mirror the two indices so the pointerup
  // commit reads current values without setState-updater side effects.
  const [optionHeld, setOptionHeld] = useState(false);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  const dragIndexRef = useRef<number | null>(null);
  const dropIndexRef = useRef<number | null>(null);
  const accountListRef = useRef<HTMLDivElement>(null);
  const accountRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Track Option key
  useEffect(() => {
    const resetDrag = () => {
      dragIndexRef.current = null;
      dropIndexRef.current = null;
      setDragIndex(null);
      setDropIndex(null);
    };
    const down = (e: KeyboardEvent) => { if (e.key === "Alt" || e.altKey) setOptionHeld(true); };
    const up = (e: KeyboardEvent) => {
      if (e.key === "Alt" || !e.altKey) {
        setOptionHeld(false);
        resetDrag();
      }
    };
    const blur = () => { setOptionHeld(false); resetDrag(); };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  const handleDragStart = useCallback((e: React.PointerEvent, index: number) => {
    if (!optionHeld) return;
    const dragged = displayAccounts[index];
    if (!dragged) return;
    e.preventDefault();
    e.stopPropagation();
    dragIndexRef.current = index;
    dropIndexRef.current = index;
    setDragIndex(index);
    setDropIndex(index);

    const onMove = (ev: PointerEvent) => {
      if (!accountListRef.current) return;
      const y = ev.clientY;
      // Find which account slot the pointer is closest to
      let closest = index;
      let closestDist = Infinity;
      accountRefs.current.forEach((ref_, i) => {
        if (!ref_ || !displayAccounts[i]) return;
        const rect = ref_.getBoundingClientRect();
        const mid = rect.top + rect.height / 2;
        const dist = Math.abs(y - mid);
        if (dist < closestDist) {
          closestDist = dist;
          closest = i;
        }
      });
      dropIndexRef.current = closest;
      setDropIndex(closest);
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
    };

    const resetDrag = () => {
      dragIndexRef.current = null;
      dropIndexRef.current = null;
      setDragIndex(null);
      setDropIndex(null);
    };

    const onUp = () => {
      cleanup();
      const from = dragIndexRef.current;
      const to = dropIndexRef.current;
      if (from !== null && to !== null && from !== to) {
        const source = displayAccounts[from];
        const target = displayAccounts[to];
        if (source && target) {
          // The dragged account adopts the drop-target row's section: dropping
          // on a row in another group joins that group at that position;
          // dropping on an ungrouped row leaves the group. Same-section drops
          // are a plain reorder. Splice in display space and persist that as
          // the new flat order — sections stay contiguous, so display and
          // stored order agree.
          const newGroup = target.group_name ?? null;
          const groupChanged = (source.group_name ?? null) !== newGroup;
          const moved = groupChanged ? { ...source, group_name: newGroup } : source;
          const reordered = [...displayAccounts];
          reordered.splice(from, 1);
          reordered.splice(to, 0, moved);
          setAccounts(reordered);
          (async () => {
            if (groupChanged) await api.accounts.setGroup(moved.id, newGroup);
            await api.accounts.reorder(reordered.map((a) => a.id));
          })().catch(console.error);
        }
      }
      resetDrag();
    };

    const onCancel = () => {
      cleanup();
      resetDrag();
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  }, [optionHeld, displayAccounts, setAccounts]);

  // Close account context menu on click outside
  useEffect(() => {
    if (!accountContextMenu) return;
    const close = () => setAccountContextMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [accountContextMenu]);

  // Focus group name input
  useEffect(() => {
    if (creatingGroup) groupInputRef.current?.focus();
  }, [creatingGroup]);

  const toggleGroupCollapse = useCallback((groupName: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(groupName)) next.delete(groupName);
      else next.add(groupName);
      return next;
    });
  }, []);

  const handleSetAccountGroup = useCallback(async (accountId: string, groupName: string | null) => {
    await api.accounts.setGroup(accountId, groupName);
    const updated = await api.accounts.list();
    setAccounts(updated);
    setAccountContextMenu(null);
  }, [setAccounts]);

  const handleToggleNotify = useCallback(async (accountId: string, enabled: boolean) => {
    await api.accounts.setNotifyEnabled(accountId, enabled);
    const updated = await api.accounts.list();
    setAccounts(updated);
    setAccountContextMenu(null);
  }, [setAccounts]);

  // Hide/show an account in every aggregated view. The open list, Needs You,
  // the group counts and the folder badges all listen for
  // `cxmail:refresh-messages`, so one event refetches every surface.
  const handleToggleHidden = useCallback(async (accountId: string, hidden: boolean) => {
    await api.accounts.setHiddenFromAggregates(accountId, hidden);
    const updated = await api.accounts.list();
    setAccounts(updated);
    setAccountContextMenu(null);
    window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
  }, [setAccounts]);

  const handleToggleTracking = useCallback(async (accountId: string, enabled: boolean) => {
    await api.accounts.setTrackOpensEnabled(accountId, enabled);
    const updated = await api.accounts.list();
    setAccounts(updated);
    setAccountContextMenu(null);
  }, [setAccounts]);

  const handleCreateGroup = useCallback(async () => {
    const name = newGroupName.trim();
    if (!name) { setCreatingGroup(false); return; }
    // Assign the context-menu account to this new group if one was selected
    if (accountContextMenu) {
      await api.accounts.setGroup(accountContextMenu.accountId, name);
      const updated = await api.accounts.list();
      setAccounts(updated);
      setAccountContextMenu(null);
    }
    setCreatingGroup(false);
    setNewGroupName("");
  }, [newGroupName, accountContextMenu, setAccounts]);

  // Load inbox groups on mount
  useEffect(() => {
    api.inboxGroups.list().then(setInboxGroups).catch(console.error);
  }, [setInboxGroups]);

  // Refresh group counts after sync
  useEffect(() => {
    const handler = () => {
      api.inboxGroups.list().then(setInboxGroups).catch(console.error);
    };
    window.addEventListener("cxmail:refresh-messages", handler);
    return () => window.removeEventListener("cxmail:refresh-messages", handler);
  }, [setInboxGroups]);

  // Auto-select first account if none selected
  useEffect(() => {
    if (!selectedAccountId && !isUnifiedInbox && accounts.length > 0) {
      setSelectedAccount(accounts[0].id);
    }
    // Collapse any newly added accounts
    setCollapsedAccounts((prev) => {
      const next = new Set(prev);
      accounts.forEach((a) => { if (!next.has(a.id)) next.add(a.id); });
      return next.size === prev.size ? prev : next;
    });
  }, [accounts, selectedAccountId, isUnifiedInbox, setSelectedAccount]);

  // Load folders for all accounts on mount and when accounts change
  // Try cached list first; if empty (new account), sync from IMAP
  useEffect(() => {
    accounts.forEach(async (account) => {
      try {
        const cached = await api.folders.list(account.id);
        if (cached.length > 0) {
          setAccountFolders(account.id, cached);
        } else {
          const result = await api.folders.sync(account.id);
          setAccountFolders(account.id, result);
        }
      } catch (e) {
        console.error(`Failed to load folders for ${account.email}:`, e);
      }
    });
  }, [accounts, setAccountFolders]);

  const toggleCollapse = useCallback((accountId: string) => {
    setCollapsedAccounts((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  }, []);

  const handleSyncAll = () => {
    if (isSyncing) return;
    // Delegate to AppLayout's runBackgroundSync via the check-mail event.
    // This uses the centralized sync path with 4-minute timeout protection.
    window.dispatchEvent(new CustomEvent("cxmail:trigger-sync"));
  };

  if (accounts.length === 0) return null;

  return (
    <div className="flex h-full flex-col bg-sidebar">
      {/* Sync all button */}
      <div className="flex items-center gap-1 px-3 py-2">
        <button
          onClick={handleSyncAll}
          disabled={isSyncing}
          className="flex flex-1 items-center justify-center gap-1.5 rounded-md bg-surface px-2 py-1.5 text-xs text-content-secondary hover:bg-elevated hover:text-content disabled:opacity-50"
        >
          <RefreshCw className={`h-3 w-3 ${isSyncing ? "animate-spin" : ""}`} />
          {isSyncing ? "Syncing..." : "Sync All"}
        </button>
        <button
          onClick={() => setShowingAddAccount(true)}
          className="flex items-center justify-center rounded-md bg-surface p-1.5 text-content-secondary hover:bg-elevated hover:text-content"
          title="Add account"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={() => setShowingClaudeRepos(true)}
          className="flex items-center justify-center rounded-md bg-surface p-1.5 text-content-secondary hover:bg-elevated hover:text-content"
          title="Open in Claude — repos"
        >
          <Terminal className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Unified Inbox */}
      {accounts.length > 0 && (
        <div className="px-2 pb-1">
          <button
            onClick={setUnifiedInbox}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              isUnifiedInbox && !selectedAccountGroup
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <Inbox className="h-4 w-4 shrink-0" />
            <span className="flex-1">All Inboxes</span>
            {(() => {
              // Frontend-only sum, so the hidden rule is applied here as well
              // as in the DB queries the list itself reads.
              const visibleAccountIds = new Set(
                accounts.filter((a) => !a.hidden_from_aggregates).map((a) => a.id),
              );
              const totalUnread = Object.entries(foldersByAccount)
                .filter(([accountId]) => visibleAccountIds.has(accountId))
                .flatMap(([, folders]) => folders)
                .filter((f) => f.folder_type === "inbox")
                .reduce((sum, f) => sum + f.unread_count, 0);
              return totalUnread > 0 ? (
                <span className="text-xs font-medium text-content-secondary">{totalUnread}</span>
              ) : null;
            })()}
          </button>
        </div>
      )}

      {/* Account sections + virtual folders */}
      <div className="flex-1 overflow-auto" ref={accountListRef}>
        {(() => {
          // "Which account is the list showing?" needs all four terms. The
          // auto-select effect above writes the first account into
          // `selectedAccountId` whenever `!isUnifiedInbox` leaves it empty —
          // which is every special view and every inbox group — so the id on
          // its own claims an account is selected while Needs You or a group
          // is on screen.
          const isAccountScoped =
            !isUnifiedInbox && specialView === null && selectedGroupId === null;

          const renderAccount = (account: typeof accounts[0], index: number) => {
            const isCollapsed = collapsedAccounts.has(account.id);
            const accountFolders = foldersByAccount[account.id] || [];
            const isDragging = dragIndex === index;
            const isDropTarget = dropIndex === index && dragIndex !== null && dragIndex !== index;
            const health = syncHealthByAccount[account.id];
            const syncWarning = isFailing(health)
              ? {
                  title:
                    (health?.lastSuccessAt
                      ? `Hasn't synced since ${new Date(health.lastSuccessAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`
                      : "Hasn't synced since CXMail opened") +
                    (health?.lastError ? `\n${health.lastError}` : ""),
                }
              : undefined;

            return (
              <div
                key={account.id}
                ref={(el) => { accountRefs.current[index] = el; }}
                className={cn(
                  "transition-all duration-150",
                  isDragging && "opacity-50",
                  isDropTarget && "border-t-2 border-accent",
                )}
              >
                <div
                  className={cn(
                    "border-l-2 flex items-center",
                    optionHeld && "cursor-grab active:cursor-grabbing select-none",
                  )}
                  style={{ borderLeftColor: account.color || "#0a84ff" }}
                  onPointerDown={(e) => handleDragStart(e, index)}
                  // WKWebView: canceling pointerdown doesn't cancel mouse defaults —
                  // block native image-drag and text-selection here or the browser's
                  // drag session eats our pointermove/pointerup (pointercancel fires).
                  onMouseDown={(e) => { if (optionHeld) e.preventDefault(); }}
                  onDragStart={(e) => e.preventDefault()}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setAccountContextMenu({ x: e.clientX, y: e.clientY, accountId: account.id });
                  }}
                >
                  {optionHeld && (
                    <GripVertical className="h-3.5 w-3.5 shrink-0 text-content-muted ml-1 -mr-2" />
                  )}
                  <AccountBadge
                    account={account}
                    isCollapsed={isCollapsed}
                    onToggle={() => toggleCollapse(account.id)}
                    onClick={optionHeld ? undefined : () => setSelectedFolder("INBOX", account.id)}
                    syncWarning={syncWarning}
                    isSelected={isAccountScoped && selectedAccountId === account.id}
                  />
                </div>

                {!isCollapsed && (
                  <div className="pb-2">
                    <FolderTree
                      folders={accountFolders}
                      accountId={account.id}
                      accentColor={account.color || "#0a84ff"}
                    />
                  </div>
                )}
              </div>
            );
          };

          return (
            <>
              {/* Ungrouped accounts */}
              {ungroupedAccounts.map((account) =>
                renderAccount(account, displayIndexById.get(account.id) ?? -1)
              )}

              {/* Account groups */}
              {accountGroups.map((groupName) => {
                const groupAccounts = groupedAccountsMap.get(groupName) || [];
                const isGroupCollapsed = collapsedGroups.has(groupName);
                // The folder's list excludes hidden members, so its count must too.
                const groupUnread = groupAccounts
                  .filter((a) => !a.hidden_from_aggregates)
                  .reduce((sum, a) => {
                    const folders = foldersByAccount[a.id] || [];
                    return sum + folders.reduce((s, f) => s + f.unread_count, 0);
                  }, 0);

                const isGroupActive = selectedAccountGroup === groupName;

                return (
                  <div key={`group-${groupName}`} className="mt-1">
                    <div
                      className={cn(
                        "flex w-full items-center gap-1.5 px-3 py-1.5 transition-colors",
                        isGroupActive ? "bg-accent/15" : "hover:bg-surface"
                      )}
                    >
                      <span
                        onClick={(e) => { e.stopPropagation(); toggleGroupCollapse(groupName); }}
                        className="cursor-pointer rounded p-0.5 hover:bg-elevated"
                      >
                        <ChevronRight
                          className={cn(
                            "h-3 w-3 shrink-0 text-content-muted transition-transform duration-150",
                            !isGroupCollapsed && "rotate-90"
                          )}
                        />
                      </span>
                      <button
                        onClick={() => setAccountGroupInbox(groupName)}
                        className="flex flex-1 items-center gap-1.5 text-left"
                      >
                        <FolderOpen className={cn("h-3.5 w-3.5 shrink-0", isGroupActive ? "text-accent" : "text-content-muted")} />
                        <span className={cn(
                          "flex-1 text-xs font-semibold uppercase tracking-wider",
                          isGroupActive ? "text-accent" : "text-content-muted"
                        )}>
                          {groupName}
                        </span>
                        {groupUnread > 0 && (
                          <span className="text-xs font-medium text-content-secondary">{groupUnread}</span>
                        )}
                      </button>
                    </div>
                    {!isGroupCollapsed && (
                      <div>
                        {groupAccounts.map((account) =>
                          renderAccount(account, displayIndexById.get(account.id) ?? -1)
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </>
          );
        })()}

        {/* Inbox Groups */}
        {inboxGroups.length > 0 && (
          <div className="mt-1 border-t border-border px-2 pt-2 pb-1 space-y-0.5">
            <div className="flex items-center justify-between px-2 pb-1">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-content-muted">
                Groups
              </span>
              <button
                onClick={() => setEditingGroup("new")}
                className="text-content-muted hover:text-content"
                title="Add group"
              >
                <Plus className="h-3 w-3" />
              </button>
            </div>
            {inboxGroups.map((group) => {
              const Icon = GROUP_ICONS[group.icon] || Folder;
              return (
                <button
                  key={group.id}
                  onClick={() => setSelectedGroup(group.id)}
                  onDoubleClick={() => setEditingGroup(group.id)}
                  className={cn(
                    "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
                    selectedGroupId === group.id
                      ? "bg-accent/15 text-accent"
                      : "text-content-secondary hover:bg-surface"
                  )}
                >
                  <Icon className="h-4 w-4 shrink-0" style={{ color: group.color }} />
                  <span className="flex-1 truncate">{group.name}</span>
                  {group.unread_count > 0 && (
                    <span className="text-xs font-medium text-content-secondary">
                      {group.unread_count}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        )}

        {/* Virtual folders */}
        <div className="mt-1 border-t border-border px-2 pt-2 pb-2 space-y-0.5">
          <button
            onClick={() => setSpecialView("needs_you")}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              specialView === "needs_you"
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <ListChecks className="h-4 w-4 shrink-0" />
            <span className="flex-1">Needs You</span>
            {needsYouCount > 0 && <span className="text-xs font-medium text-content-secondary">{needsYouCount}</span>}
          </button>
          <button
            onClick={() => setSpecialView("snoozed")}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              specialView === "snoozed"
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <Clock className="h-4 w-4 shrink-0" />
            <span className="flex-1">Snoozed</span>
          </button>
          <button
            onClick={() => setSpecialView("scheduled")}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              specialView === "scheduled"
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <Send className="h-4 w-4 shrink-0" />
            <span className="flex-1">Scheduled</span>
            {scheduledCount > 0 && (
              <span className="ml-auto shrink-0 rounded-full bg-accent/20 px-1.5 py-0.5 text-[11px] font-medium leading-none text-accent">
                {scheduledCount}
              </span>
            )}
          </button>
          <button
            onClick={() => setSpecialView("followups")}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              specialView === "followups"
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <BellRing className="h-4 w-4 shrink-0" />
            <span className="flex-1">Follow-ups</span>
          </button>
          <button
            onClick={() => setSpecialView("calendar")}
            className={cn(
              "sidebar-item flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              specialView === "calendar"
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface"
            )}
          >
            <CalendarDays className="h-4 w-4 shrink-0" />
            <span className="flex-1">Calendar</span>
          </button>
        </div>
      </div>

      {showingClaudeRepos && (
        <ClaudeRepoSettings onClose={() => setShowingClaudeRepos(false)} />
      )}

      {editingGroup !== null && (
        <InboxGroupEditor
          groupId={editingGroup === "new" ? null : editingGroup}
          onClose={() => setEditingGroup(null)}
          onSaved={() => {
            setEditingGroup(null);
            api.inboxGroups.list().then(setInboxGroups).catch(console.error);
          }}
        />
      )}

      {/* Account right-click context menu */}
      {accountContextMenu && (
        <div
          className="fixed z-50 min-w-[160px] rounded-md border border-border bg-elevated py-1 shadow-lg"
          style={{ left: accountContextMenu.x, top: accountContextMenu.y }}
        >
          {/* Notifications toggle */}
          {(() => {
            const acct = accounts.find((a) => a.id === accountContextMenu.accountId);
            if (!acct) return null;
            const enabled = acct.notify_enabled;
            return (
              <>
                <button
                  onClick={() => handleToggleNotify(acct.id, !enabled)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
                >
                  {enabled ? <BellOff className="h-3.5 w-3.5" /> : <BellRing className="h-3.5 w-3.5" />}
                  {enabled ? "Mute notifications" : "Enable notifications"}
                </button>
                <button
                  onClick={() => handleToggleTracking(acct.id, !acct.track_opens_enabled)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
                >
                  {acct.track_opens_enabled ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                  {acct.track_opens_enabled ? "Disable open tracking" : "Enable open tracking"}
                </button>
                <button
                  onClick={() => handleToggleHidden(acct.id, !acct.hidden_from_aggregates)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
                >
                  {acct.hidden_from_aggregates ? <Group className="h-3.5 w-3.5" /> : <Ungroup className="h-3.5 w-3.5" />}
                  {acct.hidden_from_aggregates ? "Show in All Inboxes & groups" : "Hide from All Inboxes & groups"}
                </button>
                <div className="my-1 border-t border-border" />
              </>
            );
          })()}
          {/* Voice profile */}
          {(() => {
            const id = accountContextMenu.accountId;
            const status = voiceStatuses[id];
            const isExtracting = extractingVoiceId === id;
            const label = status?.exists ? "Rebuild voice profile" : "Build voice profile";
            return (
              <>
                <button
                  onClick={() => handleRebuildVoiceProfile(id)}
                  disabled={isExtracting}
                  className="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
                >
                  <span className="flex items-center gap-2">
                    {isExtracting ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Sparkles className="h-3.5 w-3.5" />
                    )}
                    {label}
                  </span>
                  {status?.exists && status.generated_at && (
                    <span className="pl-5 text-xs text-content-muted">
                      {status.sample_count} samples · {new Date(status.generated_at).toLocaleDateString()}
                    </span>
                  )}
                </button>
                <div className="my-1 border-t border-border" />
              </>
            );
          })()}
          {/* Archetypes — cluster sent mail into writing-style buckets */}
          {(() => {
            const id = accountContextMenu.accountId;
            const list = archetypesByAccount[id];
            const isClustering = clusteringId === id;
            const hasArchetypes = !!list && list.length > 0;
            const label = hasArchetypes
              ? `Re-cluster archetypes (${list!.length})`
              : "Cluster archetypes";
            return (
              <>
                <button
                  onClick={() => handleClusterArchetypes(id, hasArchetypes)}
                  disabled={isClustering}
                  className="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
                >
                  <span className="flex items-center gap-2">
                    {isClustering ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Sparkles className="h-3.5 w-3.5" />
                    )}
                    {label}
                  </span>
                  {hasArchetypes && (
                    <span className="pl-5 text-xs text-content-muted">
                      {list!
                        .slice(0, 3)
                        .map((a) => a.name)
                        .join(" · ")}
                      {list!.length > 3 ? ` · +${list!.length - 3}` : ""}
                    </span>
                  )}
                </button>
                <div className="my-1 border-t border-border" />
              </>
            );
          })()}
          {/* Learn from edits — only show when there's enough data */}
          {(() => {
            const id = accountContextMenu.accountId;
            const insight = insightStatuses[id];
            const isLearning = learningEditsId === id;
            if (!insight || insight.pending_edits < 3) return null;
            return (
              <>
                <button
                  onClick={() => handleLearnFromEdits(id)}
                  disabled={isLearning}
                  className="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
                >
                  <span className="flex items-center gap-2">
                    {isLearning ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <BookOpen className="h-3.5 w-3.5" />
                    )}
                    Learn from edits ({insight.pending_edits})
                  </span>
                  {insight.active_insights > 0 && (
                    <span className="pl-5 text-xs text-content-muted">
                      {insight.active_insights} active · {insight.total_insights} total
                    </span>
                  )}
                </button>
                <div className="my-1 border-t border-border" />
              </>
            );
          })()}
          {/* Existing groups to move into */}
          {accountGroups.length > 0 && (
            <>
              {accountGroups.map((g) => {
                const currentGroup = accounts.find((a) => a.id === accountContextMenu.accountId)?.group_name;
                if (g === currentGroup) return null;
                return (
                  <button
                    key={g}
                    onClick={() => handleSetAccountGroup(accountContextMenu.accountId, g)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                    Move to {g}
                  </button>
                );
              })}
            </>
          )}
          {/* Create new group */}
          {!creatingGroup ? (
            <button
              onClick={(e) => {
                e.stopPropagation();
                setCreatingGroup(true);
                setNewGroupName("");
              }}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
            >
              <Plus className="h-3.5 w-3.5" />
              New account folder...
            </button>
          ) : (
            <div className="flex items-center gap-1 px-3 py-1.5" onClick={(e) => e.stopPropagation()}>
              <input
                ref={groupInputRef}
                value={newGroupName}
                onChange={(e) => setNewGroupName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleCreateGroup();
                  if (e.key === "Escape") { setCreatingGroup(false); setAccountContextMenu(null); }
                }}
                className="flex-1 rounded bg-surface px-1.5 py-0.5 text-sm text-content outline-none focus:ring-1 focus:ring-accent"
                placeholder="Folder name"
              />
            </div>
          )}
          {/* Remove from group */}
          {accounts.find((a) => a.id === accountContextMenu.accountId)?.group_name && (
            <>
              <div className="my-1 border-t border-border" />
              <button
                onClick={() => handleSetAccountGroup(accountContextMenu.accountId, null)}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
              >
                <X className="h-3.5 w-3.5" />
                Remove from folder
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
