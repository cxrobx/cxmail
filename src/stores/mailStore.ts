import { create } from "zustand";
import type { MessageSummary, Folder, EmailCategory, SearchResult, AppliedFilter, InboxGroup } from "@/types/email";
import type { AccountSyncHealth } from "@/lib/syncHealth";

export interface InlineComposeTarget {
  accountId: string;
  folder: string;
  uid: number;
  threadId?: string;
}

export interface InlineComposeProps {
  mode: "reply" | "reply-all" | "forward";
  defaultTo?: { name: string | null; email: string }[];
  defaultCc?: { name: string | null; email: string }[];
  defaultSubject?: string;
  defaultBody?: string;
  quotedHtml?: string;
  inReplyTo?: string;
  referencesHeader?: string;
  replyContext?: { accountId: string; folder: string; uid: number };
}

/** Prefill for the legacy docked ComposeModal, used by the `mailto:` deep-link
 * funnel (and anything else that wants to open a blank compose with fields
 * pre-populated). Maps directly onto ComposeModal's default* props. */
export interface ComposePrefill {
  to?: string[];
  cc?: string[];
  bcc?: string[];
  subject?: string;
  body?: string;
}

type SpecialView = null | "needs_you" | "snoozed" | "scheduled" | "followups" | "calendar";

function cacheKey(accountId: string | null, folder: string | null, category?: EmailCategory | null): string {
  return `${accountId ?? "unified"}:${folder ?? "INBOX"}:${category ?? "all"}`;
}

function groupCacheKey(groupId: number): string {
  return `inboxgroup:${groupId}`;
}

function messageSelectionKey(
  accountId: string | null | undefined,
  uid: number,
  folderName?: string | null,
): string {
  return `${accountId ?? "unknown"}:${folderName ?? "INBOX"}:${uid}`;
}

interface MailState {
  selectedAccountId: string | null;
  selectedFolder: string | null;
  selectedMessageUid: number | null;
  selectedMessageAccountId: string | null;
  // Folder the selected message actually lives in. Set when the selection
  // carries a known folder (search results span all folders, not just INBOX).
  // Null → ReadingPane derives the folder from the current view.
  selectedMessageFolder: string | null;
  selectedMessageUids: Set<string>;
  isUnifiedInbox: boolean;
  specialView: SpecialView;
  selectedCategory: EmailCategory | null;
  filterUnread: boolean;
  categoryCounts: Record<string, number>;
  messages: MessageSummary[];
  messageCache: Record<string, MessageSummary[]>;
  foldersByAccount: Record<string, Folder[]>;
  isLoading: boolean;
  isSyncing: boolean;
  isComposing: boolean;
  composePrefill: ComposePrefill | null;
  composeNonce: number;
  inlineComposeTarget: InlineComposeTarget | null;
  inlineComposeProps: InlineComposeProps | null;
  selectedGroupId: number | null;
  selectedAccountGroup: string | null;
  inboxGroups: InboxGroup[];
  searchQuery: string | null;
  searchResults: SearchResult[];
  /** Chips shown above search results; null when no structured parse ran. */
  searchFilters: AppliedFilter[] | null;
  /** True when the LLM parse was attempted but fell back to plain search. */
  searchAiFallback: boolean;
  /** True when the committed search was scoped to the selected account group. */
  searchScopedToGroup: boolean;
  /** The exact account ids the committed search was scoped to (`null` =
   * unscoped = every visible account). Re-runs — chip removal, load-more, the
   * server fallback — must reuse this rather than re-derive it from the view:
   * inside a hidden account the scope is that account, and re-deriving from
   * the account folder would silently widen to visible-only and lose it. */
  searchScopeIds: string[] | null;
  serverSearchStatus: "idle" | "searching" | "done" | "error";
  lastSyncedAt: string | null;
  syncError: string | null;
  /** Emails whose last sync failed with a dead token (needs_reauth), not a
   * transient error — StatusBar offers "Reconnect" for these instead of
   * just displaying the failure. */
  accountsNeedingReauth: string[];
  /** Per-account sync health (failure streaks, last success, last error),
   * keyed by account id. Drives the sidebar warning glyph; transitions are
   * computed in lib/syncHealth and announced from AppLayout. Session-scoped. */
  syncHealthByAccount: Record<string, AccountSyncHealth>;
  setSelectedGroup: (groupId: number | null) => void;
  setInboxGroups: (groups: InboxGroup[]) => void;
  setSelectedAccount: (id: string) => void;
  setSelectedFolder: (folder: string, accountId?: string) => void;
  setUnifiedInbox: () => void;
  setAccountGroupInbox: (groupName: string) => void;
  setSpecialView: (view: SpecialView) => void;
  setSelectedCategory: (category: EmailCategory | null) => void;
  toggleFilterUnread: () => void;
  setCategoryCounts: (counts: Record<string, number>) => void;
  setSelectedMessage: (uid: number | null, accountId?: string | null, folder?: string | null) => void;
  openCalendarMessage: (uid: number, accountId: string, folder: string) => void;
  setMultiSelect: (uids: Set<string>) => void;
  clearMultiSelect: () => void;
  setMessages: (messages: MessageSummary[]) => void;
  cacheMessages: (key: string, messages: MessageSummary[]) => void;
  setAccountFolders: (accountId: string, folders: Folder[]) => void;
  setLoading: (loading: boolean) => void;
  setSyncing: (syncing: boolean) => void;
  setComposing: (composing: boolean) => void;
  openComposeWith: (prefill: ComposePrefill) => void;
  setInlineCompose: (target: InlineComposeTarget, props: InlineComposeProps) => void;
  clearInlineCompose: () => void;
  markMessageRead: (uid: number, accountId?: string | null) => void;
  markMessageUnread: (uid: number, accountId?: string | null) => void;
  pendingArchiveIds: Set<string>;
  addPendingArchive: (key: string) => void;
  removePendingArchive: (key: string) => void;
  toggleMessagePin: (uid: number, accountId?: string | null) => void;
  toggleMessageMute: (uid: number, accountId?: string | null) => void;
  updateMessageCategory: (uid: number, category: EmailCategory, accountId?: string | null) => void;
  updateMessagesCategory: (uids: Set<string>, category: EmailCategory) => void;
  removeMessages: (keys: Set<string>) => void;
  setSearchResults: (
    query: string,
    results: SearchResult[],
    filters?: AppliedFilter[] | null,
    opts?: { aiFallback?: boolean; scopedToGroup?: boolean; scopeIds?: string[] },
  ) => void;
  /** Append results (dedup by account/folder/uid); used by load-more and the
   * server-search fallback. */
  appendSearchResults: (results: SearchResult[], fromServer?: boolean) => void;
  setServerSearchStatus: (status: "idle" | "searching" | "done" | "error") => void;
  clearSearch: () => void;
  setSyncCompleted: () => void;
  setSyncError: (error: string) => void;
  setAccountsNeedingReauth: (emails: string[]) => void;
  setSyncHealth: (syncHealthByAccount: Record<string, AccountSyncHealth>) => void;
  unsubscribedSenders: Set<string>;
  setUnsubscribedSenders: (emails: string[]) => void;
  addUnsubscribedSender: (email: string) => void;
}

export { cacheKey, groupCacheKey, messageSelectionKey };

export const useMailStore = create<MailState>((set, get) => ({
  selectedAccountId: null,
  selectedFolder: null,
  selectedMessageUid: null,
  selectedMessageAccountId: null,
  selectedMessageFolder: null,
  selectedMessageUids: new Set(),
  isUnifiedInbox: true,
  specialView: null,
  selectedCategory: "primary" as EmailCategory,
  filterUnread: false,
  categoryCounts: {},
  messages: [],
  messageCache: {},
  foldersByAccount: {},
  isLoading: false,
  isSyncing: false,
  isComposing: false,
  composePrefill: null,
  composeNonce: 0,
  inlineComposeTarget: null,
  inlineComposeProps: null,
  selectedGroupId: null,
  selectedAccountGroup: null,
  inboxGroups: [],
  searchQuery: null,
  searchResults: [],
  searchFilters: null,
  searchAiFallback: false,
  searchScopedToGroup: false,
  searchScopeIds: null,
  serverSearchStatus: "idle",
  lastSyncedAt: null,
  syncError: null,
  accountsNeedingReauth: [],
  syncHealthByAccount: {},
  pendingArchiveIds: new Set(),
  unsubscribedSenders: new Set(),
  setUnsubscribedSenders: (emails) =>
    set({ unsubscribedSenders: new Set(emails.map((e) => e.toLowerCase())) }),
  addUnsubscribedSender: (email) =>
    set((state) => ({
      unsubscribedSenders: new Set([...state.unsubscribedSenders, email.toLowerCase()]),
    })),
  setSelectedGroup: (groupId) =>
    set((state) => {
      const cached = groupId !== null ? state.messageCache[groupCacheKey(groupId)] : undefined;
      return {
        selectedGroupId: groupId,
        selectedAccountGroup: null,
        isUnifiedInbox: false,
        specialView: null,
        selectedFolder: null,
        selectedAccountId: null,
        selectedMessageUid: null,
        selectedMessageAccountId: null,
        selectedMessageUids: new Set(),
        selectedCategory: null,
        messages: cached ?? [],
        searchQuery: null,
        searchResults: [],
      };
    }),
  setInboxGroups: (inboxGroups) => set({ inboxGroups }),
  setSelectedAccount: (id) => set({ selectedAccountId: id }),
  setSelectedFolder: (folder, accountId) =>
    set((state) => {
      const resolvedAccountId = accountId ?? state.selectedAccountId;
      // If already viewing this folder/account, don't clear messages
      if (
        state.selectedFolder === folder &&
        state.selectedAccountId === resolvedAccountId &&
        !state.isUnifiedInbox &&
        state.specialView === null
      ) {
        return {};
      }
      // Restore cached messages for instant display
      const key = cacheKey(resolvedAccountId, folder, state.selectedCategory);
      const cached = state.messageCache[key];
      return {
        selectedFolder: folder,
        selectedAccountId: resolvedAccountId,
        isUnifiedInbox: false,
        specialView: null,
        selectedGroupId: null,
        selectedAccountGroup: null,
        selectedMessageUid: null,
        selectedMessageUids: new Set(),
        messages: cached ?? [],
        searchQuery: null,
        searchResults: [],
      };
    }),
  setUnifiedInbox: () =>
    set((state) => {
      const key = cacheKey(null, null, state.selectedCategory);
      const cached = state.messageCache[key];
      return {
        isUnifiedInbox: true,
        selectedAccountGroup: null,
        specialView: null,
        selectedGroupId: null,
        selectedFolder: null,
        selectedAccountId: null,
        selectedMessageUid: null,
        selectedMessageUids: new Set(),
        messages: cached ?? [],
        searchQuery: null,
        searchResults: [],
      };
    }),
  setAccountGroupInbox: (groupName) =>
    set((state) => {
      const key = cacheKey(`group:${groupName}`, null, state.selectedCategory);
      const cached = state.messageCache[key];
      return {
        isUnifiedInbox: true,
        selectedAccountGroup: groupName,
        specialView: null,
        selectedGroupId: null,
        selectedFolder: null,
        selectedAccountId: null,
        selectedMessageUid: null,
        selectedMessageAccountId: null,
        selectedMessageUids: new Set(),
        messages: cached ?? [],
        searchQuery: null,
        searchResults: [],
      };
    }),
  setSpecialView: (view) =>
    set({
      specialView: view,
      isUnifiedInbox: false,
      selectedAccountGroup: null,
      selectedGroupId: null,
      selectedFolder: null,
      selectedAccountId: null,
      selectedMessageUid: null,
      selectedMessageAccountId: null,
      selectedMessageUids: new Set(),
      selectedCategory: null,
      messages: [],
      searchQuery: null,
      searchResults: [],
    }),
  setSelectedCategory: (category) =>
    set((state) => {
      const accountKey = state.selectedAccountGroup
        ? `group:${state.selectedAccountGroup}`
        : state.isUnifiedInbox ? null : state.selectedAccountId;
      const key = cacheKey(
        accountKey,
        state.isUnifiedInbox ? null : state.selectedFolder,
        category,
      );
      const cached = state.messageCache[key];
      return {
        selectedCategory: category,
        selectedMessageUid: null,
        selectedMessageUids: new Set(),
        messages: cached ?? [],
      };
    }),
  toggleFilterUnread: () => set((state) => ({ filterUnread: !state.filterUnread })),
  setCategoryCounts: (categoryCounts) => set({ categoryCounts }),
  setSelectedMessage: (uid, accountId, folder) =>
    set({
      selectedMessageUid: uid,
      selectedMessageAccountId: accountId ?? null,
      // Reset to null when the caller doesn't supply a folder (normal list
      // clicks) so a stale search-result folder never leaks into the reader.
      selectedMessageFolder: folder ?? null,
      selectedMessageUids: new Set(),
      inlineComposeTarget: null,
      inlineComposeProps: null,
    }),
  openCalendarMessage: (uid, accountId, folder) =>
    set({
      selectedFolder: folder,
      selectedAccountId: accountId,
      isUnifiedInbox: false,
      selectedGroupId: null,
      selectedAccountGroup: null,
      selectedMessageUid: uid,
      selectedMessageAccountId: accountId,
      selectedMessageFolder: folder,
      selectedMessageUids: new Set(),
      inlineComposeTarget: null,
      inlineComposeProps: null,
      // deliberately NOT setting specialView → calendar stays in the center pane
    }),
  setMultiSelect: (uids) => set({ selectedMessageUids: uids }),
  clearMultiSelect: () => set({ selectedMessageUids: new Set() }),
  setMessages: (messages) => {
    const state = get();
    let key: string;
    if (state.selectedGroupId !== null) {
      key = groupCacheKey(state.selectedGroupId);
    } else {
      const accountKey = state.selectedAccountGroup
        ? `group:${state.selectedAccountGroup}`
        : state.isUnifiedInbox ? null : state.selectedAccountId;
      key = cacheKey(
        accountKey,
        state.isUnifiedInbox ? null : state.selectedFolder,
        state.selectedCategory,
      );
    }
    set((s) => ({
      messages,
      messageCache: { ...s.messageCache, [key]: messages },
    }));
  },
  cacheMessages: (key, messages) =>
    set((state) => ({
      messageCache: { ...state.messageCache, [key]: messages },
    })),
  setAccountFolders: (accountId, folders) =>
    set((state) => ({
      foldersByAccount: { ...state.foldersByAccount, [accountId]: folders },
    })),
  setLoading: (isLoading) => set({ isLoading }),
  setSyncing: (isSyncing) => set({ isSyncing }),
  // Opening a plain compose (Cmd+N / "c" / palette "New Message") must NOT
  // inherit a previous mailto prefill — even when a prefilled compose is already
  // open. So the open branch clears composePrefill AND bumps composeNonce, which
  // forces ComposeModal to remount with blank default* props (it reads them once
  // on mount). The mailto path uses openComposeWith, not this, so it's unaffected.
  setComposing: (isComposing) =>
    set((state) =>
      isComposing
        ? { isComposing: true, composePrefill: null, composeNonce: state.composeNonce + 1 }
        : { isComposing: false, composePrefill: null }),
  // Open the docked compose with prefilled fields. Bumping composeNonce forces
  // AppLayout to remount ComposeModal (its default* props are read once on
  // mount) so a new mailto re-populates even if a compose is already open.
  openComposeWith: (prefill) =>
    set((state) => ({
      composePrefill: prefill,
      composeNonce: state.composeNonce + 1,
      isComposing: true,
    })),
  setInlineCompose: (inlineComposeTarget, inlineComposeProps) =>
    set({ inlineComposeTarget, inlineComposeProps }),
  clearInlineCompose: () =>
    set({ inlineComposeTarget: null, inlineComposeProps: null }),
  markMessageRead: (uid, accountId) =>
    set((state) => ({
      // Opening a thread clears unread state for ALL its members from the
      // user's perspective: the representative becomes read, and any
      // unread older sibling is also cleared (its mark-read fan-out
      // happens server-side). Reflect that on the representative row by
      // clearing thread_has_unread alongside is_read so the bold/dot
      // styling drops immediately.
      messages: state.messages.map((m) =>
        m.uid === uid && (!accountId || m.account_id === accountId)
          ? { ...m, is_read: true, thread_has_unread: false }
          : m
      ),
    })),
  markMessageUnread: (uid, accountId) =>
    set((state) => ({
      // Leave thread_has_unread alone — backend fan-out corrects sibling
      // state on next sync; flipping it here would lie about siblings we
      // didn't actually re-mark.
      messages: state.messages.map((m) =>
        m.uid === uid && (!accountId || m.account_id === accountId)
          ? { ...m, is_read: false }
          : m
      ),
    })),
  addPendingArchive: (key) =>
    set((state) => {
      if (state.pendingArchiveIds.has(key)) return {};
      const next = new Set(state.pendingArchiveIds);
      next.add(key);
      return { pendingArchiveIds: next };
    }),
  removePendingArchive: (key) =>
    set((state) => {
      if (!state.pendingArchiveIds.has(key)) return {};
      const next = new Set(state.pendingArchiveIds);
      next.delete(key);
      return { pendingArchiveIds: next };
    }),
  toggleMessagePin: (uid, accountId) =>
    set((state) => ({
      messages: state.messages.map((m) =>
        m.uid === uid && (!accountId || m.account_id === accountId)
          ? { ...m, is_pinned: !m.is_pinned }
          : m
      ),
    })),
  toggleMessageMute: (uid, accountId) =>
    set((state) => ({
      messages: state.messages.filter(
        (m) => !(m.uid === uid && (!accountId || m.account_id === accountId))
      ),
    })),
  updateMessageCategory: (uid, category, accountId) =>
    set((state) => {
      if (state.selectedCategory && state.selectedCategory !== category) {
        return {
          messages: state.messages.filter(
            (m) => !(m.uid === uid && (!accountId || m.account_id === accountId))
          ),
        };
      }
      return {
        messages: state.messages.map((m) =>
          m.uid === uid && (!accountId || m.account_id === accountId)
            ? { ...m, category }
            : m
        ),
      };
    }),
  updateMessagesCategory: (uids, category) =>
    set((state) => {
      if (state.selectedCategory && state.selectedCategory !== category) {
        return {
          messages: state.messages.filter(
            (m) => !uids.has(messageSelectionKey(m.account_id, m.uid, m.folder_name))
          ),
          selectedMessageUid: null,
          selectedMessageUids: new Set(),
        };
      }
      return {
        messages: state.messages.map((m) =>
          uids.has(messageSelectionKey(m.account_id, m.uid, m.folder_name))
            ? { ...m, category }
            : m
        ),
        selectedMessageUids: new Set(),
      };
    }),
  removeMessages: (keys) =>
    set((state) => {
      const drop = (m: MessageSummary) => keys.has(messageSelectionKey(m.account_id, m.uid, m.folder_name));
      const nextCache: Record<string, MessageSummary[]> = {};
      for (const [k, list] of Object.entries(state.messageCache)) {
        nextCache[k] = list.filter((m) => !drop(m));
      }
      const selectedAccountId =
        state.selectedMessageAccountId ?? (!state.isUnifiedInbox ? state.selectedAccountId : null);
      const selectedFolder =
        state.isUnifiedInbox || state.selectedGroupId !== null ? "INBOX" : state.selectedFolder;
      const selectedKey = state.selectedMessageUid !== null
        ? messageSelectionKey(selectedAccountId, state.selectedMessageUid, selectedFolder)
        : null;
      const clearSelection = selectedKey !== null && keys.has(selectedKey);
      const nextMulti = new Set(state.selectedMessageUids);
      for (const k of keys) nextMulti.delete(k);
      return {
        messages: state.messages.filter((m) => !drop(m)),
        messageCache: nextCache,
        selectedMessageUids: nextMulti,
        ...(clearSelection
          ? { selectedMessageUid: null, selectedMessageAccountId: null }
          : {}),
      };
    }),
  setSearchResults: (query, results, filters, opts) =>
    set({
      searchQuery: query,
      searchResults: results,
      searchFilters: filters ?? null,
      searchAiFallback: opts?.aiFallback ?? false,
      searchScopedToGroup: opts?.scopedToGroup ?? false,
      searchScopeIds: opts?.scopeIds ?? null,
      serverSearchStatus: "idle",
      selectedMessageUid: null,
      selectedMessageAccountId: null,
    }),
  appendSearchResults: (results, fromServer) =>
    set((state) => {
      const seen = new Set(
        state.searchResults.map((r) => `${r.account_id}:${r.folder_name}:${r.uid}`),
      );
      const fresh = results
        .filter((r) => !seen.has(`${r.account_id}:${r.folder_name}:${r.uid}`))
        .map((r) => (fromServer ? { ...r, from_server: true } : r));
      if (fresh.length === 0) return {};
      return { searchResults: [...state.searchResults, ...fresh] };
    }),
  setServerSearchStatus: (serverSearchStatus) => set({ serverSearchStatus }),
  clearSearch: () =>
    set({
      searchQuery: null,
      searchResults: [],
      searchFilters: null,
      searchAiFallback: false,
      searchScopedToGroup: false,
      searchScopeIds: null,
      serverSearchStatus: "idle",
      selectedMessageUid: null,
      selectedMessageAccountId: null,
    }),
  setSyncCompleted: () =>
    set({ isSyncing: false, lastSyncedAt: new Date().toISOString(), syncError: null }),
  setSyncError: (error) =>
    set({ isSyncing: false, syncError: error }),
  setAccountsNeedingReauth: (accountsNeedingReauth) =>
    set({ accountsNeedingReauth }),
  setSyncHealth: (syncHealthByAccount) => set({ syncHealthByAccount }),
}));
