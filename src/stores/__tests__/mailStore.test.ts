import { describe, it, expect, beforeEach } from 'vitest';
import { cacheKey, messageSelectionKey, useMailStore } from '@/stores/mailStore';
import type { MessageSummary, SearchResult } from '@/types/email';

// Reset store between tests
beforeEach(() => {
  useMailStore.setState({
    selectedAccountId: null,
    selectedFolder: null,
    selectedMessageUid: null,
    selectedMessageAccountId: null,
    selectedMessageUids: new Set(),
    isUnifiedInbox: true,
    selectedAccountGroup: null,
    selectedGroupId: null,
    specialView: null,
    selectedCategory: "primary",
    categoryCounts: {},
    messages: [],
    messageCache: {},
    foldersByAccount: {},
    isLoading: false,
    isSyncing: false,
    isComposing: false,
    searchQuery: null,
    searchResults: [],
    lastSyncedAt: null,
    syncError: null,
  });
});

const makeMessage = (overrides: Partial<MessageSummary>): MessageSummary => ({
  uid: 1,
  account_id: 'acc-1',
  folder_name: 'INBOX',
  subject: 'Subject',
  from_name: 'Sender',
  from_email: 'sender@example.com',
  date: '2026-03-30T12:00:00Z',
  snippet: 'Snippet',
  is_read: false,
  is_flagged: false,
  has_attachments: false,
  size_bytes: 1024,
  category: 'primary',
  is_muted: false,
  is_pinned: false,
  thread_count: 0,
  thread_draft_count: 0,
  thread_root_id: null,
  thread_has_unread: false,
  ...overrides,
});

describe('mailStore', () => {
  it('has correct initial state', () => {
    const state = useMailStore.getState();
    expect(state.selectedAccountId).toBeNull();
    expect(state.selectedFolder).toBeNull();
    expect(state.selectedMessageUid).toBeNull();
    expect(state.isUnifiedInbox).toBe(true);
    expect(state.selectedCategory).toBe("primary");
    expect(state.messages).toEqual([]);
    expect(state.searchQuery).toBeNull();
    expect(state.searchResults).toEqual([]);
    expect(state.isLoading).toBe(false);
    expect(state.isSyncing).toBe(false);
  });

  it('setSelectedFolder updates folder and clears selection', () => {
    // Set an initial account so the folder change has context
    useMailStore.getState().setSelectedAccount('acc-1');
    useMailStore.getState().setSelectedFolder('INBOX');

    const state = useMailStore.getState();
    expect(state.selectedFolder).toBe('INBOX');
    expect(state.selectedAccountId).toBe('acc-1');
    expect(state.isUnifiedInbox).toBe(false);
    expect(state.selectedMessageUid).toBeNull();
  });

  it('setUnifiedInbox clears account, folder, and messages', () => {
    // Set some state first
    useMailStore.getState().setSelectedAccount('acc-1');
    useMailStore.getState().setSelectedFolder('INBOX');
    useMailStore.getState().setSelectedMessage(42);

    // Switch to unified inbox
    useMailStore.getState().setUnifiedInbox();

    const state = useMailStore.getState();
    expect(state.isUnifiedInbox).toBe(true);
    expect(state.selectedAccountGroup).toBeNull();
    expect(state.selectedAccountId).toBeNull();
    expect(state.selectedFolder).toBeNull();
    expect(state.selectedMessageUid).toBeNull();
    expect(state.messages).toEqual([]);
    expect(state.searchQuery).toBeNull();
    expect(state.searchResults).toEqual([]);
  });

  it('setAccountGroupInbox filters unified inbox by group', () => {
    useMailStore.getState().setAccountGroupInbox('Business');

    const state = useMailStore.getState();
    expect(state.isUnifiedInbox).toBe(true);
    expect(state.selectedAccountGroup).toBe('Business');
    expect(state.selectedAccountId).toBeNull();
    expect(state.selectedFolder).toBeNull();
    expect(state.selectedGroupId).toBeNull();
    expect(state.specialView).toBeNull();
  });

  it('setSelectedFolder clears account group', () => {
    useMailStore.getState().setAccountGroupInbox('Business');
    useMailStore.getState().setSelectedAccount('acc-1');
    useMailStore.getState().setSelectedFolder('INBOX');

    const state = useMailStore.getState();
    expect(state.selectedAccountGroup).toBeNull();
    expect(state.isUnifiedInbox).toBe(false);
  });

  it('markMessageRead updates the correct message', () => {
    const messages: MessageSummary[] = [
      {
        uid: 1,
        account_id: 'acc-1',
        subject: 'Hello',
        from_name: 'Alice',
        from_email: 'alice@example.com',
        date: '2026-03-30T12:00:00Z',
        snippet: 'Hi there',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 1024,
        category: null,
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
      {
        uid: 2,
        account_id: 'acc-1',
        subject: 'World',
        from_name: 'Bob',
        from_email: 'bob@example.com',
        date: '2026-03-30T13:00:00Z',
        snippet: 'Hey',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 512,
        category: null,
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
    ];

    useMailStore.getState().setMessages(messages);
    useMailStore.getState().markMessageRead(1, 'acc-1');

    const state = useMailStore.getState();
    expect(state.messages[0].is_read).toBe(true);
    expect(state.messages[1].is_read).toBe(false);
  });

  it('markMessageRead only updates the matching account in unified views', () => {
    const messages: MessageSummary[] = [
      {
        uid: 7,
        account_id: 'acc-1',
        subject: 'First',
        from_name: 'Alice',
        from_email: 'alice@example.com',
        date: '2026-03-30T12:00:00Z',
        snippet: 'Hi there',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 1024,
        category: null,
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
      {
        uid: 7,
        account_id: 'acc-2',
        subject: 'Second',
        from_name: 'Bob',
        from_email: 'bob@example.com',
        date: '2026-03-30T13:00:00Z',
        snippet: 'Hey',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 512,
        category: null,
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
    ];

    useMailStore.getState().setMessages(messages);
    useMailStore.getState().markMessageRead(7, 'acc-2');

    const state = useMailStore.getState();
    expect(state.messages[0].is_read).toBe(false);
    expect(state.messages[1].is_read).toBe(true);
  });

  it('updateMessagesCategory uses account-aware message keys', () => {
    const messages: MessageSummary[] = [
      {
        uid: 4,
        account_id: 'acc-1',
        subject: 'Account one',
        from_name: 'Alice',
        from_email: 'alice@example.com',
        date: '2026-03-30T12:00:00Z',
        snippet: 'Hi there',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 1024,
        category: 'primary',
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
      {
        uid: 4,
        account_id: 'acc-2',
        subject: 'Account two',
        from_name: 'Bob',
        from_email: 'bob@example.com',
        date: '2026-03-30T13:00:00Z',
        snippet: 'Hey',
        is_read: false,
        is_flagged: false,
        has_attachments: false,
        size_bytes: 512,
        category: 'primary',
        is_muted: false,
        is_pinned: false,
        thread_count: 0,
        thread_draft_count: 0,
        thread_root_id: null,
        thread_has_unread: false,
      },
    ];

    useMailStore.setState({ selectedCategory: null });
    useMailStore.getState().setMessages(messages);
    useMailStore.getState().updateMessagesCategory(
      new Set([messageSelectionKey('acc-2', 4)]),
      'updates',
    );

    const state = useMailStore.getState();
    expect(state.messages[0].category).toBe('primary');
    expect(state.messages[1].category).toBe('updates');
  });

  it('removeMessages evicts only the matching account and folder from live and cached messages', () => {
    const inbox = makeMessage({ uid: 9, account_id: 'acc-1', folder_name: 'INBOX', subject: 'Inbox' });
    const archive = makeMessage({ uid: 9, account_id: 'acc-1', folder_name: 'Archive', subject: 'Archive' });
    const otherAccount = makeMessage({ uid: 9, account_id: 'acc-2', folder_name: 'INBOX', subject: 'Other' });
    const inboxKey = messageSelectionKey('acc-1', 9, 'INBOX');
    const archiveKey = messageSelectionKey('acc-1', 9, 'Archive');

    useMailStore.setState({
      selectedAccountId: 'acc-1',
      selectedFolder: 'INBOX',
      isUnifiedInbox: false,
      messages: [inbox, archive, otherAccount],
      messageCache: {
        [cacheKey('acc-1', 'INBOX', 'primary')]: [inbox],
        [cacheKey(null, null, 'primary')]: [inbox, otherAccount],
        [cacheKey('acc-1', 'Archive', 'primary')]: [archive],
      },
      selectedMessageUid: 9,
      selectedMessageAccountId: 'acc-1',
      selectedMessageUids: new Set([inboxKey, archiveKey]),
    });

    useMailStore.getState().removeMessages(new Set([inboxKey]));

    const state = useMailStore.getState();
    expect(state.messages.map((m) => m.subject)).toEqual(['Archive', 'Other']);
    expect(state.messageCache[cacheKey('acc-1', 'INBOX', 'primary')]).toEqual([]);
    expect(state.messageCache[cacheKey(null, null, 'primary')]).toEqual([otherAccount]);
    expect(state.messageCache[cacheKey('acc-1', 'Archive', 'primary')]).toEqual([archive]);
    expect(state.selectedMessageUid).toBeNull();
    expect(state.selectedMessageAccountId).toBeNull();
    expect(state.selectedMessageUids).toEqual(new Set([archiveKey]));
  });

  const searchResult = (overrides: Partial<SearchResult> = {}): SearchResult => ({
    account_id: 'acc-1',
    folder_name: 'INBOX',
    uid: 5,
    subject: 'Test email',
    from_name: 'Test Sender',
    from_email: 'test@example.com',
    date: '2026-06-01T10:00:00Z',
    snippet: 'This is a test',
    is_read: false,
    has_attachments: false,
    score: 0.95,
    ...overrides,
  });

  it('setSearchResults stores query, results, and filter chips, clears selected message', () => {
    useMailStore.getState().setSelectedMessage(10);

    useMailStore
      .getState()
      .setSearchResults('test', [searchResult()], [{ kind: 'keywords', value: 'test' }], {
        aiFallback: true,
        scopedToGroup: true,
      });

    const state = useMailStore.getState();
    expect(state.searchQuery).toBe('test');
    expect(state.searchResults).toHaveLength(1);
    expect(state.searchResults[0].subject).toBe('Test email');
    expect(state.searchFilters).toEqual([{ kind: 'keywords', value: 'test' }]);
    expect(state.searchAiFallback).toBe(true);
    expect(state.searchScopedToGroup).toBe(true);
    expect(state.selectedMessageUid).toBeNull();
  });

  it('appendSearchResults dedups by account/folder/uid and tags server hits', () => {
    useMailStore.getState().setSearchResults('test', [searchResult({ uid: 1 })]);

    useMailStore
      .getState()
      .appendSearchResults(
        [searchResult({ uid: 1 }), searchResult({ uid: 2, folder_name: '[Gmail]/All Mail' })],
        true,
      );

    const state = useMailStore.getState();
    expect(state.searchResults).toHaveLength(2);
    expect(state.searchResults[0].from_server).toBeUndefined();
    expect(state.searchResults[1].uid).toBe(2);
    expect(state.searchResults[1].from_server).toBe(true);
  });

  it('clearSearch resets search state including chips and server status', () => {
    useMailStore
      .getState()
      .setSearchResults('query', [searchResult({ uid: 1 })], [{ kind: 'from', value: 'a' }]);
    useMailStore.getState().setServerSearchStatus('searching');

    useMailStore.getState().clearSearch();

    const state = useMailStore.getState();
    expect(state.searchQuery).toBeNull();
    expect(state.searchResults).toEqual([]);
    expect(state.searchFilters).toBeNull();
    expect(state.serverSearchStatus).toBe('idle');
    expect(state.selectedMessageUid).toBeNull();
  });
});
