import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import ReadingPane from '@/components/mail/ReadingPane';
import { useMailStore } from '@/stores/mailStore';
import { api } from '@/lib/tauri';

// Heavy children — this suite only cares which mailbox the body is fetched from.
vi.mock('@/components/mail/EmailFrame', () => ({ default: () => null }));
vi.mock('@/components/mail/AttachmentList', () => ({ default: () => null }));
vi.mock('@/components/mail/ComposeModal', () => ({ default: () => null }));
vi.mock('@/components/mail/ThreadMessageCard', () => ({ default: () => null }));
vi.mock('@/components/mail/SnoozePopover', () => ({ default: () => null }));
vi.mock('@/components/mail/AISummary', () => ({ default: () => null }));
vi.mock('@/components/mail/SmartReplies', () => ({ default: () => null }));
vi.mock('@/components/mail/CalendarEventCard', () => ({ default: () => null }));
vi.mock('@/components/mail/TrackingBadge', () => ({ default: () => null }));
vi.mock('@/hooks/useUnsubscribe', () => ({
  useUnsubscribe: () => ({ unsubscribedSenders: new Set(), unsubscribe: vi.fn() }),
}));

vi.mock('@/lib/tauri', () => ({
  api: {
    messages: {
      fetchBody: vi.fn(),
      getThread: vi.fn(),
      markRead: vi.fn(),
    },
    identities: { list: vi.fn() },
  },
}));

const OWNING_ACCOUNT = 'acct-owning-the-message';
const LAST_BROWSED_ACCOUNT = 'acct-browsed-earlier';

describe('ReadingPane account resolution', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.messages.fetchBody as ReturnType<typeof vi.fn>).mockResolvedValue({
      uid: 107350,
      subject: 'New skill available',
      from_name: 'LinkedIn',
      from_email: 'news@linkedin.com',
      to_list: [],
      cc_list: [],
      bcc_list: [],
      date: '2026-07-30T18:38:44Z',
      plain_text: 'body',
      sanitized_html: null,
      attachments: [],
      is_read: false,
      is_flagged: false,
      list_unsubscribe: null,
      list_unsubscribe_post: null,
      message_id: null,
      references: null,
    });
    (api.messages.getThread as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (api.identities.list as ReturnType<typeof vi.fn>).mockResolvedValue([]);
  });

  // Regression: opening a message from Needs You (a view that is neither the
  // unified inbox nor a group) fetched the UID from whatever account was
  // browsed last, producing "Not found: Message UID N not found" on a message
  // that was cached under its real account the whole time.
  it('fetches from the account the selected message belongs to, not the last browsed one', async () => {
    useMailStore.setState({
      specialView: 'needs_you',
      isUnifiedInbox: false,
      selectedGroupId: null,
      selectedAccountId: LAST_BROWSED_ACCOUNT,
      selectedFolder: 'INBOX',
      selectedMessageUid: 107350,
      selectedMessageAccountId: OWNING_ACCOUNT,
      selectedMessageFolder: 'INBOX',
    });

    render(<ReadingPane />);

    await waitFor(() => expect(api.messages.fetchBody).toHaveBeenCalled());
    expect(api.messages.fetchBody).toHaveBeenCalledWith(OWNING_ACCOUNT, 'INBOX', 107350);
  });

  it('still falls back to the browsed account for a selection that carries none', async () => {
    useMailStore.setState({
      specialView: null,
      isUnifiedInbox: false,
      selectedGroupId: null,
      selectedAccountId: LAST_BROWSED_ACCOUNT,
      selectedFolder: 'Archive',
      selectedMessageUid: 42,
      selectedMessageAccountId: null,
      selectedMessageFolder: null,
    });

    render(<ReadingPane />);

    await waitFor(() => expect(api.messages.fetchBody).toHaveBeenCalled());
    expect(api.messages.fetchBody).toHaveBeenCalledWith(LAST_BROWSED_ACCOUNT, 'Archive', 42);
  });
});
