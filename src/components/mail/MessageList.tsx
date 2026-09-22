import { useRef, useCallback, useEffect, useState, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { useAccountStore } from "@/stores/accountStore";
import { useUIStore, type DensityMode } from "@/stores/uiStore";
import { api } from "@/lib/tauri";
import MessageListItem from "./MessageListItem";
import EmailContextMenu from "./EmailContextMenu";
import SnoozedList from "./SnoozedList";
import ScheduledView from "./ScheduledView";
import FollowupList from "./FollowupList";
import CalendarView from "./CalendarView";
import NeedsYouList from "./NeedsYouList";
import CategoryTabs from "./CategoryTabs";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import { groupMessagesByDate } from "@/lib/dateGroups";
import { uidsForThreadAction } from "@/lib/threadActions";
import { filtersFromChips, runServerSearch } from "@/lib/searchFilters";
import { formatRelativeDate } from "@/lib/utils";
import { openDraftForEdit } from "@/lib/draftCompose";
import { isDraftFolder as isDraftFolderShared } from "@/lib/draftFolder";
import type { AppliedFilter, MessageSummary, McpActivity, Nudge } from "@/types/email";
import { AlertCircle, Cloud, Inbox, Loader2, Paperclip, Search, X } from "lucide-react";

/** Render an FTS snippet, wrapping U+E000…U+E001 marker pairs in <mark>.
 * Plain React text nodes only — never dangerouslySetInnerHTML. */
function HighlightedSnippet({ text }: { text: string }) {
  const segments = text.split(/[\uE000\uE001]/);
  return (
    <>
      {segments.map((seg, i) =>
        i % 2 === 1 ? (
          <mark key={i} className="rounded-sm bg-amber-500/25 px-0.5 text-amber-200">
            {seg}
          </mark>
        ) : (
          <span key={i}>{seg}</span>
        ),
      )}
    </>
  );
}

const DENSITY_HEIGHTS: Record<DensityMode, { header: number; message: number }> = {
  comfortable:    { header: 28, message: 88 },
  compact:        { header: 24, message: 52 },
  "ultra-compact": { header: 22, message: 28 },
};

export default function MessageList() {
  const parentRef = useRef<HTMLDivElement>(null);
  const lastClickedIndexRef = useRef<number | null>(null);
  const latestLoadRequestRef = useRef(0);
  const density = useUIStore((s) => s.density);
  const heights = DENSITY_HEIGHTS[density];
  const {
    selectedAccountId,
    selectedFolder,
    selectedMessageUid,
    selectedMessageAccountId,
    selectedMessageUids,
    isUnifiedInbox,
    specialView,
    selectedCategory,
    selectedGroupId,
    inboxGroups,
    messages,
    isLoading,
    setMessages,
    setSelectedMessage,
    setMultiSelect,
    setLoading,
    isSyncing,
    searchQuery,
    searchResults,
    clearSearch,
  } = useMailStore();
  const searchFilters = useMailStore((s) => s.searchFilters);
  const searchAiFallback = useMailStore((s) => s.searchAiFallback);
  const searchScopedToGroup = useMailStore((s) => s.searchScopedToGroup);
  const searchScopeIds = useMailStore((s) => s.searchScopeIds);
  const serverSearchStatus = useMailStore((s) => s.serverSearchStatus);
  const setSearchResults = useMailStore((s) => s.setSearchResults);
  const appendSearchResults = useMailStore((s) => s.appendSearchResults);
  const selectedAccountGroup = useMailStore((s) => s.selectedAccountGroup);
  const filterUnread = useMailStore((s) => s.filterUnread);
  const foldersByAccount = useMailStore((s) => s.foldersByAccount);
  const pendingArchiveIds = useMailStore((s) => s.pendingArchiveIds);
  const addPendingArchive = useMailStore((s) => s.addPendingArchive);
  const removePendingArchive = useMailStore((s) => s.removePendingArchive);
  const markMessageRead = useMailStore((s) => s.markMessageRead);
  const markMessageUnread = useMailStore((s) => s.markMessageUnread);
  const accounts = useAccountStore((s) => s.accounts);
  const addToast = useUIStore((s) => s.addToast);

  // Stalled-conversation prompts, keyed by the row they attach to. Loaded once
  // per message-list refresh rather than per row: `list_nudges` is a single
  // deterministic pass over the mailbox, and calling it per row would run it
  // hundreds of times for one screen.
  const [nudges, setNudges] = useState<Map<string, Nudge>>(new Map());

  const loadNudges = useCallback(async () => {
    try {
      const rows = await api.nudges.list();
      setNudges(
        new Map(rows.map((n) => [`${n.account_id}-${n.folder_name}-${n.uid}`, n])),
      );
    } catch (e) {
      // A nudge is an enhancement to a row that renders fine without it —
      // never let this failure take the inbox down with it.
      console.error("Failed to load nudges:", e);
    }
  }, []);

  useEffect(() => {
    void loadNudges();
  }, [loadNudges, messages]);

  const handleDismissNudge = useCallback(
    async (nudge: Nudge) => {
      // Optimistic: the badge disappears on click, because waiting on a DB
      // round-trip to un-render one word reads as a broken button.
      setNudges((prev) => {
        const next = new Map(prev);
        next.delete(`${nudge.account_id}-${nudge.folder_name}-${nudge.uid}`);
        return next;
      });
      try {
        await api.nudges.dismiss(nudge.account_id, nudge.thread_key, nudge.kind);
      } catch (e) {
        console.error("Failed to dismiss nudge:", e);
        void loadNudges(); // put it back — it was never actually dismissed
      }
    },
    [loadNudges],
  );

  const isDraftFolder = useCallback(
    (accountId: string | null | undefined, folderName: string | null | undefined) =>
      isDraftFolderShared(foldersByAccount, accountId, folderName),
    [foldersByAccount],
  );

  // Derive account IDs for the selected account group
  const accountGroupIds = useMemo(() => {
    if (!selectedAccountGroup) return [];
    return accounts.filter((a) => a.group_name === selectedAccountGroup).map((a) => a.id);
  }, [selectedAccountGroup, accounts]);

  // Stable string for dependency arrays (avoids false triggers from array identity)
  const accountGroupKey = accountGroupIds.join(",");

  const [hasMore, setHasMore] = useState(false);
  const [page, setPage] = useState(0);

  // Search results pagination (reset when a new query commits).
  const [searchLoadingMore, setSearchLoadingMore] = useState(false);
  const [searchExhausted, setSearchExhausted] = useState(false);
  useEffect(() => {
    setSearchExhausted(false);
    setSearchLoadingMore(false);
  }, [searchQuery]);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Transient highlight key for a row Claude just changed (matches dateGroups'
  // `msg-${account_id}-${uid}` item key). Cleared after the flash animation.
  const [recentlyChangedKey, setRecentlyChangedKey] = useState<string | null>(null);
  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pageSize = 50;

  // Only update messages if the list actually changed (prevents re-render flash after sync)
  const smartSetMessages = useCallback(
    (newMessages: typeof messages, append: boolean) => {
      const current = useMailStore.getState().messages;
      if (append) {
        const existingKeys = new Set(current.map(m => `${m.account_id}-${m.uid}`));
        const filtered = newMessages.filter(m => !existingKeys.has(`${m.account_id}-${m.uid}`));
        setMessages([...current, ...filtered]);
        return;
      }
      // Skip update if list is identical (same UIDs, same order, same read/flag state)
      if (
        current.length === newMessages.length &&
        current.every(
          (m, i) =>
            m.uid === newMessages[i].uid &&
            m.is_read === newMessages[i].is_read &&
            m.is_flagged === newMessages[i].is_flagged &&
            m.is_pinned === newMessages[i].is_pinned &&
            m.is_muted === newMessages[i].is_muted &&
            m.category === newMessages[i].category &&
            m.thread_count === newMessages[i].thread_count
        )
      ) {
        return; // No changes — skip re-render
      }
      setMessages(newMessages);
    },
    [setMessages]
  );

  const loadMessages = useCallback(
    async (pageNum: number) => {
      const requestId = ++latestLoadRequestRef.current;
      setLoadError(null);

      const isLatestRequest = () => latestLoadRequestRef.current === requestId;
      const withTimeout = <T,>(promise: Promise<T>, ms = 30_000): Promise<T> =>
        new Promise<T>((resolve, reject) => {
          const timer = window.setTimeout(
            () => reject(new Error("Request timed out — backend may be busy syncing")),
            ms,
          );
          promise.then(
            (value) => {
              window.clearTimeout(timer);
              resolve(value);
            },
            (error) => {
              window.clearTimeout(timer);
              reject(error);
            },
          );
        });

      // Inbox group view
      if (selectedGroupId !== null) {
        try {
          const result = await withTimeout(api.inboxGroups.fetchMessages(selectedGroupId, pageNum, pageSize, filterUnread));
          if (!isLatestRequest()) return;
          smartSetMessages(result.messages, pageNum !== 0);
          setHasMore(result.has_more);
          setPage(pageNum);
        } catch (e) {
          if (!isLatestRequest()) return;
          console.error("Failed to fetch group messages:", e);
          setLoadError(e instanceof Error ? e.message : "Failed to load messages");
        } finally {
          if (isLatestRequest()) {
            setLoading(false);
          }
        }
        return;
      }
      if (isUnifiedInbox) {
        try {
          const ids = accountGroupIds.length > 0 ? accountGroupIds : undefined;
          const result = await withTimeout(api.messages.fetchUnifiedInbox(pageNum, pageSize, selectedCategory, ids, filterUnread));
          if (!isLatestRequest()) return;
          smartSetMessages(result.messages, pageNum !== 0);
          setHasMore(result.has_more);
          setPage(pageNum);
        } catch (e) {
          if (!isLatestRequest()) return;
          console.error("Failed to fetch unified inbox:", e);
          setLoadError(e instanceof Error ? e.message : "Failed to load messages");
        } finally {
          if (isLatestRequest()) {
            setLoading(false);
          }
        }
        return;
      }
      if (!selectedAccountId || !selectedFolder) return;
      try {
        // Category tabs only render for INBOX (see showCategoryTabs below);
        // other folders must not inherit the inbox's tab filter invisibly.
        const effectiveCategory = selectedFolder === "INBOX" ? selectedCategory : null;
        const result = await withTimeout(api.messages.fetch(
          selectedAccountId,
          selectedFolder,
          pageNum,
          pageSize,
          effectiveCategory,
          filterUnread,
        ));
        if (!isLatestRequest()) return;
        smartSetMessages(result.messages, pageNum !== 0);
        setHasMore(result.has_more);
        setPage(pageNum);
      } catch (e) {
        if (!isLatestRequest()) return;
        console.error("Failed to fetch messages:", e);
        setLoadError(e instanceof Error ? e.message : "Failed to load messages");
      } finally {
        if (isLatestRequest()) {
          setLoading(false);
        }
      }
    },
    [selectedAccountId, selectedFolder, isUnifiedInbox, selectedCategory, selectedGroupId, accountGroupIds, filterUnread, smartSetMessages, setLoading]
  );

  // Load messages from DB on folder/account/group switch
  useEffect(() => {
    let cancelled = false;
    if (selectedGroupId !== null || isUnifiedInbox || (selectedAccountId && selectedFolder)) {
      if (messages.length === 0) {
        setLoading(true);
      }
      // Always load cached data immediately
      loadMessages(0);
      // Then sync non-INBOX folders from IMAP in the background and reload
      if (selectedAccountId && selectedFolder && selectedFolder !== "INBOX" && !isUnifiedInbox && selectedGroupId === null) {
        api.messages.sync(selectedAccountId, selectedFolder)
          .then(() => { if (!cancelled) loadMessages(0); })
          .catch((e) => console.error("Folder sync failed:", e));
      }
    }
    return () => { cancelled = true; };
  }, [selectedAccountId, selectedFolder, isUnifiedInbox, selectedCategory, selectedGroupId, accountGroupKey, filterUnread]);

  // Reload from DB when background sync or IDLE delivers new mail (debounced)
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const handler = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        if (selectedGroupId !== null || isUnifiedInbox || (selectedAccountId && selectedFolder)) {
          loadMessages(0);
        }
      }, 300);
    };
    window.addEventListener("cxmail:refresh-messages", handler);
    return () => {
      window.removeEventListener("cxmail:refresh-messages", handler);
      if (timer) clearTimeout(timer);
    };
  }, [selectedAccountId, selectedFolder, isUnifiedInbox, selectedCategory, selectedGroupId, accountGroupKey, filterUnread, loadMessages]);

  // Briefly flash the row Claude just created/updated/flagged. Move/archive/delete
  // remove the row from the list, so the toast alone communicates those.
  useEffect(() => {
    const handler = (e: Event) => {
      const env = (e as CustomEvent<McpActivity>).detail;
      if (!env || !env.account_id) return;
      setRecentlyChangedKey(`msg-${env.account_id}-${env.uid}`);
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
      flashTimerRef.current = setTimeout(() => setRecentlyChangedKey(null), 1500);
    };
    window.addEventListener("cxmail:mcp-activity", handler);
    return () => {
      window.removeEventListener("cxmail:mcp-activity", handler);
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
    };
  }, []);

  const visibleMessages = useMemo(() => {
    let list = messages;
    if (filterUnread) list = list.filter((m) => !m.is_read);
    if (pendingArchiveIds.size > 0) {
      list = list.filter((m) => {
        const aid = m.account_id ?? selectedAccountId;
        const fld = m.folder_name ?? selectedFolder ?? "INBOX";
        return !pendingArchiveIds.has(messageSelectionKey(aid, m.uid, fld));
      });
    }
    return list;
  }, [messages, filterUnread, pendingArchiveIds, selectedAccountId, selectedFolder]);

  const handleSwipeArchive = useCallback(
    (message: MessageSummary) => {
      const aid = message.account_id ?? selectedAccountId;
      const fld = message.folder_name ?? selectedFolder ?? "INBOX";
      if (!aid) return;
      const key = messageSelectionKey(aid, message.uid, fld);
      addPendingArchive(key);
      addToast({
        message: "Archived",
        type: "success",
        duration: 5000,
        action: {
          label: "Undo",
          onClick: () => removePendingArchive(key),
        },
        onExpire: () => {
          removePendingArchive(key);
          uidsForThreadAction(message, aid, fld)
            .then((uids) =>
              uids.length > 0 ? api.messages.archive(aid, fld, uids) : Promise.resolve(),
            )
            .catch((err) => console.error("[swipe-archive] IPC failed", err));
        },
      });
    },
    [selectedAccountId, selectedFolder, addPendingArchive, removePendingArchive, addToast],
  );

  const handleSwipeToggleRead = useCallback(
    (message: MessageSummary) => {
      const aid = message.account_id ?? selectedAccountId;
      const fld = message.folder_name ?? selectedFolder ?? "INBOX";
      if (!aid) return;
      const isCurrentlyUnread = !message.is_read || message.thread_has_unread;
      if (isCurrentlyUnread) {
        markMessageRead(message.uid, aid);
        api.messages
          .markRead(aid, fld, [message.uid])
          .catch((err) => console.error("[swipe-toggle-read] markRead failed", err));
      } else {
        markMessageUnread(message.uid, aid);
        api.messages
          .markUnread(aid, fld, [message.uid])
          .catch((err) => console.error("[swipe-toggle-read] markUnread failed", err));
      }
    },
    [selectedAccountId, selectedFolder, markMessageRead, markMessageUnread],
  );

  // Build a flat index of only message items for shift-click range calculation
  const displayItems = useMemo(() => groupMessagesByDate(visibleMessages), [visibleMessages]);
  const messageIndices = useMemo(() => {
    const map = new Map<string, number>();
    messages.forEach((m, i) => {
      const accountId = m.account_id ?? selectedAccountId;
      const folder = m.folder_name ?? selectedFolder ?? "INBOX";
      map.set(messageSelectionKey(accountId, m.uid, folder), i);
    });
    return map;
  }, [messages, selectedAccountId, selectedFolder]);

  const handleMessageClick = useCallback(
    (message: (typeof messages)[0], e: React.MouseEvent) => {
      const messageAccountId = message.account_id ?? selectedAccountId;
      const messageFolder = message.folder_name ?? selectedFolder ?? "INBOX";
      const clickedMsgIndex = messageIndices.get(messageSelectionKey(messageAccountId, message.uid, messageFolder));
      if (clickedMsgIndex === undefined) return;

      if (e.shiftKey && lastClickedIndexRef.current !== null) {
        // Shift-click: select range from anchor to clicked
        const from = Math.min(lastClickedIndexRef.current, clickedMsgIndex);
        const to = Math.max(lastClickedIndexRef.current, clickedMsgIndex);
        const uids = new Set<string>();
        for (let i = from; i <= to; i++) {
          const candidate = messages[i];
          const candidateAccountId = candidate.account_id ?? selectedAccountId;
          const candidateFolder = candidate.folder_name ?? selectedFolder ?? "INBOX";
          if (candidateAccountId === messageAccountId && candidateFolder === messageFolder) {
            uids.add(messageSelectionKey(candidateAccountId, candidate.uid, candidateFolder));
          }
        }
        setMultiSelect(uids);
      } else {
        // Normal click: single select, clear multi-select
        const clickAccountId = message.account_id ?? selectedAccountId;
        const folder = selectedFolder;
        if (clickAccountId && folder && isDraftFolder(clickAccountId, folder)) {
          lastClickedIndexRef.current = clickedMsgIndex;
          void openDraftForEdit(clickAccountId, folder, message.uid);
          return;
        }
        setSelectedMessage(message.uid, message.account_id);
        lastClickedIndexRef.current = clickedMsgIndex;
      }
    },
    [messages, messageIndices, setSelectedMessage, setMultiSelect, selectedAccountId, selectedFolder, isDraftFolder]
  );

  const virtualizer = useVirtualizer({
    count: displayItems.length,
    getScrollElement: () => parentRef.current,
    estimateSize: (index) => displayItems[index]?.type === "header" ? heights.header : heights.message,
    getItemKey: (index) => displayItems[index]?.key ?? index,
    overscan: 10,
  });

  // Keep the selected search-result row in view when navigating between hits.
  // block:"nearest" makes this a no-op if the row is already on-screen.
  const selectedResultRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (!searchQuery) return;
    selectedResultRef.current?.scrollIntoView({ block: "nearest" });
  }, [searchQuery, selectedMessageUid, selectedMessageAccountId]);

  // Render special views (after all hooks to satisfy Rules of Hooks)
  if (specialView === "needs_you") return <NeedsYouList />;
  if (specialView === "snoozed") return <SnoozedList />;
  if (specialView === "scheduled") return <ScheduledView />;
  if (specialView === "followups") return <FollowupList />;
  if (specialView === "calendar") return <CalendarView />;

  // Search results view
  if (searchQuery) {
    // The scope the search was COMMITTED with, not re-derived from the view:
    // inside a hidden account that is the account itself, and re-deriving
    // from the account folder would widen a chip removal or load-more to
    // visible-only and make its results vanish.
    const scopeIds = searchScopeIds ?? undefined;
    // What the scope banner names: the account folder, or the single hidden
    // account the search was pinned to.
    const scopeLabel = selectedAccountGroup
      ?? (scopeIds?.length === 1
        ? (() => {
            const a = accounts.find((x) => x.id === scopeIds[0]);
            return a ? a.display_name || a.email : null;
          })()
        : null);

    const removeChip = async (chip: AppliedFilter) => {
      if (!searchFilters) return;
      const remaining = searchFilters.filter(
        (c) => !(c.kind === chip.kind && c.value === chip.value),
      );
      if (remaining.length === 0) {
        clearSearch();
        return;
      }
      const f = filtersFromChips(remaining);
      if (scopeIds) f.account_ids = scopeIds;
      try {
        const results = await api.messages.searchWithFilters(f);
        setSearchResults(searchQuery, results, remaining, {
          scopedToGroup: searchScopedToGroup,
          scopeIds,
        });
      } catch (e) {
        console.error("Failed to re-run search after chip removal:", e);
      }
    };

    const searchAllAccounts = async () => {
      if (!searchFilters) return;
      try {
        const results = await api.messages.searchWithFilters(filtersFromChips(searchFilters));
        setSearchResults(searchQuery, results, searchFilters, { scopedToGroup: false });
      } catch (e) {
        console.error("Failed to widen search to all accounts:", e);
      }
    };

    const loadMoreResults = async () => {
      if (!searchFilters || searchLoadingMore) return;
      setSearchLoadingMore(true);
      try {
        const f = filtersFromChips(searchFilters);
        if (scopeIds) f.account_ids = scopeIds;
        const offset = searchResults.filter((r) => !r.from_server).length;
        const more = await api.messages.searchWithFilters(f, { offset });
        if (more.length < 50) setSearchExhausted(true);
        appendSearchResults(more);
      } catch (e) {
        console.error("Failed to load more search results:", e);
      }
      setSearchLoadingMore(false);
    };

    const triggerServerSearch = () => {
      const chips = searchFilters ?? [{ kind: "keywords", value: searchQuery }];
      void runServerSearch(searchQuery, filtersFromChips(chips), scopeIds);
    };

    return (
      <div className="flex h-full flex-col">
        <div className="border-b border-border-subtle px-3 py-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-content-secondary">
              {searchResults.length} result{searchResults.length !== 1 ? "s" : ""} for &ldquo;{searchQuery}&rdquo;
              {searchScopedToGroup && scopeLabel && (
                <>
                  {" "}in {scopeLabel} &middot;{" "}
                  <button onClick={() => void searchAllAccounts()} className="text-accent hover:underline">
                    Search all
                  </button>
                </>
              )}
            </span>
            <button onClick={clearSearch} className="text-xs text-content-muted hover:text-content flex items-center gap-1">
              <X className="h-3 w-3" /> Clear
            </button>
          </div>
          {searchFilters && searchFilters.length > 0 && (
            <div className="mt-1.5 flex flex-wrap items-center gap-1">
              {searchFilters.map((chip) => (
                <span
                  key={`${chip.kind}:${chip.value}`}
                  className="inline-flex items-center gap-1 rounded-full bg-accent/15 px-2 py-0.5 text-xs text-accent"
                >
                  {chip.kind === "keywords" ? chip.value : `${chip.kind}: ${chip.value}`}
                  <button
                    onClick={() => void removeChip(chip)}
                    className="text-accent/70 hover:text-accent"
                    aria-label={`Remove filter ${chip.kind}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </span>
              ))}
            </div>
          )}
          {searchAiFallback && (
            <div className="mt-1 text-[11px] text-content-muted">
              Couldn&rsquo;t parse with AI — using plain search
            </div>
          )}
          {serverSearchStatus === "searching" && (
            <div className="mt-1 flex items-center gap-1.5 text-[11px] text-content-muted">
              <Loader2 className="h-3 w-3 animate-spin" /> Searching server…
            </div>
          )}
        </div>
        <div className="flex-1 overflow-auto">
          {searchResults.length === 0 && (
            <div className="flex flex-col items-center gap-2 px-4 py-8 text-center">
              <Search className="h-6 w-6 text-content-muted" />
              <p className="text-sm text-content-muted">
                Nothing found locally for &ldquo;{searchQuery}&rdquo;
              </p>
            </div>
          )}
          {searchResults.map((r) => {
            const isSelected =
              selectedMessageUid === r.uid &&
              (selectedMessageAccountId === null || selectedMessageAccountId === r.account_id);
            const asMessage: MessageSummary = {
              uid: r.uid,
              account_id: r.account_id,
              folder_name: r.folder_name,
              subject: r.subject,
              from_name: r.from_name,
              from_email: r.from_email,
              date: r.date,
              snippet: r.snippet,
              is_read: r.is_read,
              is_flagged: false,
              has_attachments: r.has_attachments,
              size_bytes: 0,
              category: null,
              is_muted: false,
              is_pinned: false,
              thread_count: 1,
              thread_draft_count: 0,
              thread_root_id: null,
              thread_has_unread: false,
            };
            return (
            <EmailContextMenu key={`${r.account_id}-${r.folder_name}-${r.uid}`} message={asMessage} triggerClassName="contents">
            <button
              ref={isSelected ? selectedResultRef : undefined}
              onClick={() => setSelectedMessage(r.uid, r.account_id, r.folder_name)}
              className={`flex w-full flex-col gap-0.5 border-b border-border-subtle px-4 py-3 text-left hover:bg-surface ${
                isSelected ? "bg-accent/20" : ""
              }`}
            >
              <div className="flex items-baseline justify-between gap-2">
                <span className={`flex min-w-0 items-center gap-1.5 text-sm text-content ${r.is_read ? "font-medium" : "font-semibold"}`}>
                  {!r.is_read && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
                  <span className="truncate">{r.from_name || r.from_email || "Unknown"}</span>
                </span>
                <span className="flex shrink-0 items-center gap-1.5 text-xs text-content-muted">
                  {r.from_server && <Cloud className="h-3 w-3 text-accent" aria-label="Found on server" />}
                  {r.has_attachments && <Paperclip className="h-3 w-3" />}
                  <span>{r.folder_name}</span>
                  {r.date && <span>&middot; {formatRelativeDate(r.date)}</span>}
                </span>
              </div>
              <span className={`truncate text-sm ${r.is_read ? "text-content-secondary" : "font-medium text-content"}`}>
                {r.subject || "(no subject)"}
              </span>
              {r.snippet && (
                <span className="truncate text-xs text-content-muted">
                  <HighlightedSnippet text={r.snippet} />
                </span>
              )}
            </button>
            </EmailContextMenu>
            );
          })}
          {searchResults.filter((r) => !r.from_server).length >= 50 && !searchExhausted && (
            <button
              onClick={() => void loadMoreResults()}
              disabled={searchLoadingMore}
              className="flex w-full items-center justify-center gap-2 border-b border-border-subtle px-4 py-2.5 text-xs text-content-secondary hover:bg-surface disabled:opacity-50"
            >
              {searchLoadingMore && <Loader2 className="h-3 w-3 animate-spin" />}
              Load more
            </button>
          )}
          <button
            onClick={triggerServerSearch}
            disabled={serverSearchStatus === "searching"}
            className="flex w-full items-center justify-center gap-2 px-4 py-2.5 text-xs text-content-secondary hover:bg-surface disabled:opacity-50"
          >
            <Cloud className="h-3.5 w-3.5" />
            {serverSearchStatus === "searching"
              ? "Searching server…"
              : serverSearchStatus === "done"
                ? "Search server again"
                : "Search on server"}
          </button>
        </div>
      </div>
    );
  }

  if (!selectedFolder && !isUnifiedInbox && selectedGroupId === null) {
    return (
      <EmptyState
        icon={Inbox}
        title="Select a folder"
        description="Choose a folder from the sidebar to view messages"
      />
    );
  }

  if (isLoading && messages.length === 0) {
    return <LoadingSpinner className="h-full" />;
  }

  if (loadError && messages.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-content-secondary">
        <AlertCircle className="h-8 w-8 text-content-muted" />
        <p className="text-sm font-medium">Unable to load messages</p>
        <p className="max-w-[260px] text-center text-xs text-content-muted">{loadError}</p>
        <button
          onClick={() => {
            setLoading(true);
            setLoadError(null);
            loadMessages(0);
          }}
          className="mt-1 rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-accent/80"
        >
          Retry
        </button>
      </div>
    );
  }

  const activeGroup = selectedGroupId !== null
    ? inboxGroups.find((g) => g.id === selectedGroupId)
    : null;

  if (messages.length === 0) {
    const title = filterUnread ? "No unread messages" : "No messages";
    const description = filterUnread
      ? (activeGroup
          ? `No unread messages match "${activeGroup.name}" rules`
          : selectedAccountGroup
            ? `No unread messages in "${selectedAccountGroup}"`
            : "You're all caught up")
      : (activeGroup
          ? `No messages match "${activeGroup.name}" rules`
          : selectedAccountGroup
            ? `No messages in "${selectedAccountGroup}"`
            : "This folder is empty");
    return <EmptyState icon={Inbox} title={title} description={description} />;
  }

  const showCategoryTabs = !activeGroup && (isUnifiedInbox || selectedFolder === "INBOX");

  return (
    <div className="flex h-full flex-col">
      {showCategoryTabs && <CategoryTabs />}
      {activeGroup && (
        <div className="flex items-center gap-2 border-b border-border-subtle px-3 py-1.5">
          <div className="h-2 w-2 rounded-full" style={{ backgroundColor: activeGroup.color }} />
          <span className="text-xs font-medium text-content-secondary">{activeGroup.name}</span>
          <span className="text-[10px] text-content-muted">({messages.length} messages)</span>
        </div>
      )}
      {isSyncing && messages.length > 0 && (
        <div className="flex items-center gap-2 border-b border-border-subtle px-3 py-1">
          <div className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
          <span className="text-[11px] text-content-muted">Checking for new mail...</span>
        </div>
      )}
      <div key={density} ref={parentRef} className="flex-1 overflow-auto" style={{ overscrollBehaviorX: "none" }}>
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const item = displayItems[virtualItem.index];
          return (
            <div
              key={virtualItem.key}
              ref={virtualizer.measureElement}
              data-index={virtualItem.index}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: item.type === "header" ? heights.header : heights.message,
                overflow: "hidden",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              {item.type === "header" ? (
                <div className="flex items-center justify-between bg-base px-4 py-1">
                  <span className="text-[11px] font-semibold uppercase tracking-wider text-content-muted">
                    {item.label}
                  </span>
                  <span className="text-[10px] text-content-muted">{item.count}</span>
                </div>
              ) : (
                <EmailContextMenu message={item.message}>
                  <MessageListItem
                    message={item.message}
                    flash={item.key === recentlyChangedKey}
                    isSelected={selectedMessageUid === item.message.uid && (!selectedMessageAccountId || selectedMessageAccountId === item.message.account_id)}
                    isMultiSelected={selectedMessageUids.has(
                      messageSelectionKey(
                        item.message.account_id ?? selectedAccountId,
                        item.message.uid,
                        item.message.folder_name ?? selectedFolder ?? "INBOX",
                      ),
                    )}
                    onClick={(e) => handleMessageClick(item.message, e)}
                    accountId={selectedAccountId || item.message.account_id}
                    folder={selectedFolder || (selectedGroupId !== null ? "INBOX" : null)}
                    onSwipeArchive={handleSwipeArchive}
                    onSwipeToggleRead={handleSwipeToggleRead}
                    nudge={nudges.get(
                      `${item.message.account_id}-${item.message.folder_name ?? selectedFolder ?? "INBOX"}-${item.message.uid}`,
                    )}
                    onDismissNudge={handleDismissNudge}
                  />
                </EmailContextMenu>
              )}
            </div>
          );
        })}
      </div>
      {hasMore && (
        <button
          onClick={() => loadMessages(page + 1)}
          className="w-full py-3 text-center text-sm text-content-secondary hover:text-content"
        >
          Load more
        </button>
      )}
      </div>
    </div>
  );
}
