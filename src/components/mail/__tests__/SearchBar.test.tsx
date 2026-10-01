import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SearchBar from '@/components/mail/SearchBar';
import { useMailStore } from '@/stores/mailStore';
import { useUIStore } from '@/stores/uiStore';
import { useAccountStore } from '@/stores/accountStore';
import { api } from '@/lib/tauri';
import type { Account, SearchResult } from '@/types/email';

// Mock the tauri api module
vi.mock('@/lib/tauri', () => ({
  api: {
    intelligence: { answer: vi.fn() },
    messages: {
      aiSearch: vi.fn(),
      search: vi.fn(),
      searchContacts: vi.fn(),
      searchWithFilters: vi.fn(),
      serverSearch: vi.fn(),
    },
  },
}));

const sampleResult: SearchResult = {
  account_id: 'acc-1',
  folder_name: 'INBOX',
  uid: 7,
  subject: 'Budget review',
  from_name: 'Sarah',
  from_email: 'sarah@x.com',
  date: '2026-06-01T10:00:00Z',
  snippet: 'the budget numbers',
  is_read: true,
  has_attachments: false,
  score: 1,
};

function account(id: string, group: string | null, hidden = false): Account {
  return {
    id,
    email: `${id}@example.com`,
    display_name: null,
    provider: 'gmail',
    imap_host: 'imap.gmail.com',
    imap_port: 993,
    smtp_host: 'smtp.gmail.com',
    smtp_port: 587,
    imap_security: 'implicit',
    smtp_security: 'starttls',
    imap_username: null,
    smtp_username: null,
    color: null,
    is_active: true,
    sort_order: 0,
    group_name: group,
    notify_enabled: true,
    track_opens_enabled: false,
    hidden_from_aggregates: hidden,
    triage_enabled: false,
  };
}

describe('SearchBar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [account('acc-visible', 'BUSINESS'), account('acc-hidden', 'BUSINESS', true)],
    });
    useMailStore.setState({
      isUnifiedInbox: true,
      specialView: null,
      selectedGroupId: null,
      selectedAccountId: null,
      selectedAccountGroup: null,
    });
    vi.mocked(api.messages.search).mockResolvedValue([]);
    vi.mocked(api.messages.searchContacts).mockResolvedValue([]);
    vi.mocked(api.messages.serverSearch).mockResolvedValue([]);
    vi.mocked(api.messages.aiSearch).mockResolvedValue({
      results: [],
      applied_filters: [],
      used_ai: false,
      ai_failed: false,
    });
    useMailStore.getState().clearSearch();
    useUIStore.setState({ recentSearches: [] });
  });

  it('renders the search input with the stable data attribute for the / shortcut', () => {
    render(<SearchBar />);
    const input = screen.getByPlaceholderText(/search emails/i);
    expect(input).toBeInTheDocument();
    expect(input).toHaveAttribute('type', 'text');
    expect(input).toHaveAttribute('data-search-input');
  });

  it('updates input value when typing', async () => {
    const user = userEvent.setup();
    render(<SearchBar />);

    const input = screen.getByPlaceholderText(/search emails/i);
    await user.type(input, 'hello');

    expect(input).toHaveValue('hello');
  });

  it('shows clear button when input has a value', async () => {
    const user = userEvent.setup();
    render(<SearchBar />);

    const input = screen.getByPlaceholderText(/search emails/i);
    await user.type(input, 'test');

    // The X clear button should appear
    const clearButton = screen.getByRole('button', { name: 'Clear search' });
    expect(clearButton).toBeInTheDocument();
  });

  it('clears input when clear button is clicked', async () => {
    const user = userEvent.setup();
    render(<SearchBar />);

    const input = screen.getByPlaceholderText(/search emails/i);
    await user.type(input, 'test');
    expect(input).toHaveValue('test');

    const clearButton = screen.getByRole('button', { name: 'Clear search' });
    await user.click(clearButton);

    expect(input).toHaveValue('');
  });

  it('shows local prefix-search results in the dropdown while typing', async () => {
    vi.mocked(api.messages.search).mockResolvedValue([sampleResult]);
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.type(screen.getByPlaceholderText(/search emails/i), 'budg');

    await waitFor(() => {
      expect(screen.getByText('Budget review')).toBeInTheDocument();
    });
    expect(api.messages.search).toHaveBeenCalledWith(
      'budg',
      expect.objectContaining({ prefix: true }),
    );
    // The AI path must NOT fire per keystroke — Enter only.
    expect(api.messages.aiSearch).not.toHaveBeenCalled();
  });

  it('opens the active dropdown row with ArrowDown + Enter', async () => {
    vi.mocked(api.messages.search).mockResolvedValue([sampleResult]);
    const user = userEvent.setup();
    render(<SearchBar />);

    const input = screen.getByPlaceholderText(/search emails/i);
    await user.type(input, 'budg');
    await waitFor(() => {
      expect(screen.getByText('Budget review')).toBeInTheDocument();
    });

    await user.keyboard('{ArrowDown}{Enter}');

    const state = useMailStore.getState();
    expect(state.selectedMessageUid).toBe(7);
    expect(state.selectedMessageAccountId).toBe('acc-1');
    expect(state.selectedMessageFolder).toBe('INBOX');
    expect(state.searchQuery).toBe('budg');
  });

  it('Enter without an active row commits via aiSearch and records a recent search', async () => {
    vi.mocked(api.messages.aiSearch).mockResolvedValue({
      results: Array.from({ length: 12 }, (_, i) => ({ ...sampleResult, uid: i + 1 })),
      applied_filters: [{ kind: 'keywords', value: 'budget' }],
      used_ai: true,
      ai_failed: false,
    });
    const user = userEvent.setup();
    render(<SearchBar />);

    const input = screen.getByPlaceholderText(/search emails/i);
    await user.type(input, 'budget');
    await user.keyboard('{Enter}');

    await waitFor(() => {
      expect(useMailStore.getState().searchQuery).toBe('budget');
    });
    expect(useMailStore.getState().searchResults).toHaveLength(12);
    expect(useMailStore.getState().searchFilters).toEqual([
      { kind: 'keywords', value: 'budget' },
    ]);
    expect(useUIStore.getState().recentSearches).toEqual(['budget']);
    // 12 local hits ≥ 10 → no automatic server fallback.
    expect(api.messages.serverSearch).not.toHaveBeenCalled();
  });

  it('kicks off the server-search fallback when local results are thin', async () => {
    vi.mocked(api.messages.aiSearch).mockResolvedValue({
      results: [sampleResult],
      applied_filters: [{ kind: 'keywords', value: 'rarething' }],
      used_ai: false,
      ai_failed: false,
    });
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.type(screen.getByPlaceholderText(/search emails/i), 'rarething');
    await user.keyboard('{Enter}');

    await waitFor(() => {
      expect(api.messages.serverSearch).toHaveBeenCalled();
    });
  });

  it('shows recent searches when focusing an empty input and re-runs on click', async () => {
    useUIStore.setState({ recentSearches: ['quarterly numbers'] });
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.click(screen.getByPlaceholderText(/search emails/i));

    expect(screen.getByText('Recent searches')).toBeInTheDocument();
    await user.click(screen.getByText('quarterly numbers'));

    await waitFor(() => {
      expect(api.messages.aiSearch).toHaveBeenCalledWith('quarterly numbers', undefined);
    });
  });
});

describe('SearchBar — accounts hidden from aggregates', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.messages.search).mockResolvedValue([]);
    vi.mocked(api.messages.searchContacts).mockResolvedValue([]);
    vi.mocked(api.messages.serverSearch).mockResolvedValue([]);
    vi.mocked(api.messages.aiSearch).mockResolvedValue({
      results: [],
      applied_filters: [],
      used_ai: false,
      ai_failed: false,
    });
    useMailStore.getState().clearSearch();
    useUIStore.setState({ recentSearches: [] });
    useAccountStore.setState({
      accounts: [account('acc-visible', 'BUSINESS'), account('acc-hidden', 'BUSINESS', true)],
    });
    // The view state persists across tests in this module; start each from All Inboxes.
    useMailStore.setState({
      isUnifiedInbox: true,
      specialView: null,
      selectedGroupId: null,
      selectedAccountId: null,
      selectedAccountGroup: null,
    });
  });

  it('inside the hidden account, searches THAT account — the one way its mail is reachable', async () => {
    useMailStore.setState({
      isUnifiedInbox: false,
      specialView: null,
      selectedGroupId: null,
      selectedAccountId: 'acc-hidden',
      selectedAccountGroup: null,
    });
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.type(screen.getByPlaceholderText(/search emails/i), 'warm');
    await waitFor(() => {
      expect(api.messages.search).toHaveBeenCalledWith(
        'warm',
        expect.objectContaining({ accountIds: ['acc-hidden'] }),
      );
    });

    await user.keyboard('{Enter}');
    await waitFor(() => {
      expect(api.messages.aiSearch).toHaveBeenCalledWith('warm', ['acc-hidden']);
    });
    // The committed scope is stored so re-runs (chips, load-more, server
    // fallback) cannot silently widen back to visible-only.
    expect(useMailStore.getState().searchScopeIds).toEqual(['acc-hidden']);
    expect(useMailStore.getState().searchScopedToGroup).toBe(true);
  });

  it('in an account folder, searches the visible members only', async () => {
    useMailStore.setState({
      isUnifiedInbox: true,
      specialView: null,
      selectedGroupId: null,
      selectedAccountId: null,
      selectedAccountGroup: 'BUSINESS',
    });
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.type(screen.getByPlaceholderText(/search emails/i), 'budget');
    await user.keyboard('{Enter}');

    await waitFor(() => {
      expect(api.messages.aiSearch).toHaveBeenCalledWith('budget', ['acc-visible']);
    });
  });

  it('in All Inboxes, stays unscoped — the backend reads that as every visible account', async () => {
    const user = userEvent.setup();
    render(<SearchBar />);

    await user.type(screen.getByPlaceholderText(/search emails/i), 'budget');
    await user.keyboard('{Enter}');

    await waitFor(() => {
      expect(api.messages.aiSearch).toHaveBeenCalledWith('budget', undefined);
    });
    expect(useMailStore.getState().searchScopeIds).toBeNull();
  });

  it('answers within the chosen account and opens the actual cited message', async () => {
    useMailStore.setState({ isUnifiedInbox: false, selectedAccountId: 'acc-hidden' });
    vi.mocked(api.intelligence.answer).mockResolvedValue({ text: 'Give fourteen days notice [1].', sources: [{ result: { ...sampleResult, account_id: 'acc-hidden', folder_name: 'Archive' }, source: 'Attachment 1: policy.pdf', text: 'Fourteen days', semantic: true }] });
    render(<SearchBar />);
    await userEvent.type(screen.getByPlaceholderText(/search emails/i), 'How much notice?');
    await userEvent.click(screen.getByRole('button', { name: 'Ask mail' }));
    expect(api.intelligence.answer).toHaveBeenCalledWith('How much notice?', ['acc-hidden']);
    await userEvent.click(await screen.findByRole('button', { name: /\[1\].*policy.pdf/ }));
    expect(useMailStore.getState().selectedMessageAccountId).toBe('acc-hidden');
    expect(useMailStore.getState().selectedMessageFolder).toBe('Archive');
    expect(useMailStore.getState().selectedMessageUid).toBe(7);
  });

  it('shows an answer that cites nothing, flagged as unverified', async () => {
    vi.mocked(api.intelligence.answer).mockResolvedValue({ text: 'None of these messages give a date.', sources: [{ result: sampleResult, source: 'Email', text: 'Kickoff soon', semantic: false }], uncited: true });
    render(<SearchBar />);
    await userEvent.type(screen.getByPlaceholderText(/search emails/i), 'When is kickoff?');
    await userEvent.click(screen.getByRole('button', { name: 'Ask mail' }));
    expect(await screen.findByText('None of these messages give a date.')).toBeInTheDocument();
    expect(screen.getByText(/doesn't cite any of the messages/)).toBeInTheDocument();
  });

  it('does not flag a cited answer', async () => {
    vi.mocked(api.intelligence.answer).mockResolvedValue({ text: 'Kickoff is Monday [1].', sources: [{ result: sampleResult, source: 'Email', text: 'Kickoff Monday', semantic: false }], uncited: false });
    render(<SearchBar />);
    await userEvent.type(screen.getByPlaceholderText(/search emails/i), 'When is kickoff?');
    await userEvent.click(screen.getByRole('button', { name: 'Ask mail' }));
    expect(await screen.findByText('Kickoff is Monday [1].')).toBeInTheDocument();
    expect(screen.queryByText(/doesn't cite any of the messages/)).not.toBeInTheDocument();
  });

  it('drops a late answer after changing account scope', async () => {
    let resolve!: (value: { text: string; sources: [] }) => void;
    vi.mocked(api.intelligence.answer).mockReturnValue(new Promise((r) => { resolve = r; }));
    render(<SearchBar />);
    await userEvent.type(screen.getByPlaceholderText(/search emails/i), 'What is the policy?');
    await userEvent.click(screen.getByRole('button', { name: 'Ask mail' }));
    act(() => useMailStore.setState({ isUnifiedInbox: false, selectedAccountId: 'acc-hidden' }));
    await act(async () => resolve({ text: 'Stale answer from another account', sources: [] }));
    expect(screen.queryByText('Stale answer from another account')).not.toBeInTheDocument();
  });
});
