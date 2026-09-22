import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import ReadingPane from '@/components/mail/ReadingPane';
import { useMailStore } from '@/stores/mailStore';
import { api } from '@/lib/tauri';
import { openDraftForEdit } from '@/lib/draftCompose';
import type { MessageSummary } from '@/types/email';

// ThreadMessageCard is the component under test here, so it is deliberately
// NOT mocked. Everything below it is.
vi.mock('@/components/mail/EmailFrame', () => ({ default: () => null }));
vi.mock('@/components/mail/AttachmentList', () => ({ default: () => null }));
vi.mock('@/components/mail/ComposeModal', () => ({ default: () => null }));
vi.mock('@/components/mail/SnoozePopover', () => ({ default: () => null }));
vi.mock('@/components/mail/AISummary', () => ({ default: () => null }));
vi.mock('@/components/mail/SmartReplies', () => ({ default: () => null }));
vi.mock('@/components/mail/CalendarEventCard', () => ({ default: () => null }));
vi.mock('@/components/mail/TrackingBadge', () => ({ default: () => null }));
vi.mock('@/hooks/useUnsubscribe', () => ({
  useUnsubscribe: () => ({ handleUnsubscribeResult: vi.fn(), addToast: vi.fn() }),
}));
vi.mock('@/lib/draftCompose', () => ({ openDraftForEdit: vi.fn() }));
vi.mock('@/lib/tauri', () => ({
  api: {
    messages: { fetchBody: vi.fn(), getThread: vi.fn(), markRead: vi.fn() },
    identities: { list: vi.fn() },
  },
}));

const ACCOUNT = 'acct-cxv';
const DRAFTS = '[Gmail]/Drafts';

const member = (over: Partial<MessageSummary>): MessageSummary => ({
  uid: 0,
  account_id: ACCOUNT,
  folder_name: 'INBOX',
  subject: 'Re: Voltworks x CX Ventures',
  from_name: 'Nicholas McCormick',
  from_email: 'pat@voltworks.example',
  date: '2026-09-15T14:06:42Z',
  snippet: 'Apologies for the delay',
  is_read: true,
  is_flagged: false,
  has_attachments: false,
  size_bytes: 10,
  category: 'primary',
  is_muted: false,
  is_pinned: false,
  thread_count: 0,
  thread_draft_count: 0,
  thread_root_id: '<root@cxmail.app>',
  thread_has_unread: false,
  ...over,
});

const INBOX_MSG = member({ uid: 753 });
const DRAFT_MSG = member({
  uid: 712,
  folder_name: DRAFTS,
  from_name: 'Christopher Robinson',
  from_email: 'chris@cxventures.io',
  date: '2026-08-23T00:04:13Z',
  snippet: 'Hi Nick, First, an apology',
  is_read: false,
});

/** The list row the reading pane is opened from: one message exchanged, one
 * unsent draft — the shape the badge and the gate have to disagree about. */
const LIST_ROW = member({ uid: 753, thread_count: 0, thread_draft_count: 1 });

function open(row: MessageSummary = LIST_ROW) {
  useMailStore.setState({
    specialView: null,
    isUnifiedInbox: false,
    selectedGroupId: null,
    selectedAccountId: ACCOUNT,
    selectedFolder: 'INBOX',
    selectedMessageUid: row.uid,
    selectedMessageAccountId: ACCOUNT,
    selectedMessageFolder: 'INBOX',
    messages: [row],
    foldersByAccount: {
      [ACCOUNT]: [
        { id: 1, account_id: ACCOUNT, name: 'INBOX', display_name: null, folder_type: 'inbox', delimiter: '/', total_count: 1, unread_count: 0, uidvalidity: 1, uidnext: 2 },
        { id: 2, account_id: ACCOUNT, name: DRAFTS, display_name: null, folder_type: 'drafts', delimiter: '/', total_count: 1, unread_count: 0, uidvalidity: 1, uidnext: 2 },
      ],
    },
  });
  return render(<ReadingPane />);
}

describe('an unsent draft inside a thread', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    (api.messages.fetchBody as ReturnType<typeof vi.fn>).mockResolvedValue({
      uid: 753, subject: 'Re: Voltworks', from_name: 'Pat', from_email: 'pat@voltworks.example',
      to_list: [], cc_list: [], bcc_list: [], date: '2026-09-15T14:06:42Z',
      plain_text: 'body', sanitized_html: null, attachments: [], is_read: true,
      is_flagged: false, list_unsubscribe: null, list_unsubscribe_post: null,
      message_id: null, references: null,
    });
    (api.messages.getThread as ReturnType<typeof vi.fn>).mockResolvedValue([INBOX_MSG, DRAFT_MSG]);
    (api.messages.markRead as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);
    (api.identities.list as ReturnType<typeof vi.fn>).mockResolvedValue([]);
  });

  /** The reported bug: the draft rendered as an ordinary member, so a message
   * that was never sent read as part of the correspondence. */
  it('is labelled a draft rather than printed like a sent message', async () => {
    open();
    await waitFor(() => expect(screen.getByText('Draft')).toBeTruthy());

    // Its sender name — which is the user's own, identical to the Sent cards —
    // is NOT the label. That is what made it indistinguishable.
    expect(screen.queryByText('Christopher Robinson')).toBeNull();
    expect(screen.getByText('Nicholas McCormick')).toBeTruthy();
  });

  /** A read-only body is the one view of an unfinished reply nobody wants, and
   * rendering one is what made it look sent. */
  it('opens the composer instead of expanding, and fetches no body for it', async () => {
    open();
    const card = await screen.findByLabelText(/Continue editing draft/);

    fireEvent.click(card);

    expect(openDraftForEdit).toHaveBeenCalledWith(ACCOUNT, DRAFTS, 712);
    const fetched = (api.messages.fetchBody as ReturnType<typeof vi.fn>).mock.calls;
    expect(fetched.some((c) => c[2] === 712)).toBe(false);
  });

  /** Negative control: an ordinary member still expands in place. Without this
   * the test above would pass with every card routed to the composer. */
  it('leaves ordinary members expanding in place', async () => {
    open();
    await screen.findByLabelText(/Continue editing draft/);

    fireEvent.click(screen.getByText('Nicholas McCormick'));

    await waitFor(() =>
      expect(
        (api.messages.fetchBody as ReturnType<typeof vi.fn>).mock.calls.some((c) => c[2] === 753),
      ).toBe(true),
    );
    expect(openDraftForEdit).not.toHaveBeenCalled();
  });

  /** The gate. `thread_count` counts messages exchanged, so here it is 0 — and
   * gating the stack on it alone (the obvious way to stop drafts inflating the
   * badge) would mean the draft card never renders at all. */
  it('still opens the thread stack when the only sibling is the draft', async () => {
    open();
    await waitFor(() => expect(api.messages.getThread).toHaveBeenCalled());
    expect(await screen.findByText('Draft')).toBeTruthy();
  });

  /** …and a thread with neither siblings nor drafts must still not fire a
   * thread load, or every single message pays for one. */
  it('does not open the stack for a genuinely single message', async () => {
    open(member({ uid: 753, thread_count: 0, thread_draft_count: 0 }));
    await waitFor(() => expect(api.messages.fetchBody).toHaveBeenCalled());
    vi.advanceTimersByTime(500);
    expect(api.messages.getThread).not.toHaveBeenCalled();
  });

  /** A draft saved unread must not drag the mark-read fan-out into the drafts
   * folder — marking a draft `\Seen` on the server is a write nobody asked
   * for. The fan-out is folder-scoped, and this is what keeps it that way. */
  it('never marks the draft read', async () => {
    open();
    await screen.findByLabelText(/Continue editing draft/);
    vi.advanceTimersByTime(500);

    for (const call of (api.messages.markRead as ReturnType<typeof vi.fn>).mock.calls) {
      expect(call[1]).not.toBe(DRAFTS);
      expect(call[2]).not.toContain(712);
    }
  });
});
