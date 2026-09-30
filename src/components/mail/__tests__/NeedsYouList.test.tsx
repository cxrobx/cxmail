import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import NeedsYouList from '@/components/mail/NeedsYouList';
import { useMailStore } from '@/stores/mailStore';
import { api } from '@/lib/tauri';
import type { NeedsYouItem } from '@/types/email';

vi.mock('@/lib/tauri', () => ({
  api: {
    needsYou: {
      list: vi.fn(),
      dismiss: vi.fn(),
      dismissGroup: vi.fn(),
    },
  },
}));

const alertRow: NeedsYouItem = {
  uid: 1,
  account_id: 'acc-1',
  folder_name: 'INBOX',
  subject: 'Container finance_api in Container Manager stopped unexpectedly',
  from_name: 'Synology',
  from_email: 'sns@synologynotification.com',
  date: '2026-07-30T18:00:00Z',
  snippet: 'It stopped unexpectedly.',
  is_read: false,
  has_attachments: false,
  action_type: 'alert',
  reason: 'System alert reporting a failure',
  evidence: 'Container finance_api in Container Manager stopped unexpectedly',
  duplicate_count: 3,
  members: [
    { account_id: 'acc-1', folder_name: 'INBOX', uid: 1 },
    { account_id: 'acc-1', folder_name: 'INBOX', uid: 2 },
    { account_id: 'acc-1', folder_name: 'INBOX', uid: 3 },
  ],
  score: 4,
  urgency: null,
};

describe('NeedsYouList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.needsYou.list as ReturnType<typeof vi.fn>).mockResolvedValue([alertRow]);
    (api.needsYou.dismissGroup as ReturnType<typeof vi.fn>).mockResolvedValue(3);
  });

  it('shows the matched evidence and how many messages a row stands for', async () => {
    render(<NeedsYouList />);
    // Evidence replaces the canned reason — the old list printed the same
    // sentence on every row, which is what made the queue look broken.
    await screen.findByText(/“Container finance_api in Container Manager stopped unexpectedly”/);
    expect(screen.getByText('×3')).toBeInTheDocument();
  });

  it('dismisses every message behind a collapsed row', async () => {
    render(<NeedsYouList />);
    const dismiss = await screen.findByRole('button', { name: /Dismiss all 3 messages/i });
    await userEvent.click(dismiss);

    await waitFor(() => expect(api.needsYou.dismissGroup).toHaveBeenCalledWith(alertRow.members));
    // Not the single-message call — that would leave 2 behind and the row
    // would return on the next refresh.
    expect(api.needsYou.dismiss).not.toHaveBeenCalled();
  });

  it('opens the message against its own account and folder', async () => {
    render(<NeedsYouList />);
    // The subject appears twice (subject line + quoted evidence); either one
    // bubbles the click to the row button.
    const [subject] = await screen.findAllByText(/Container finance_api/);
    await userEvent.click(subject);

    const state = useMailStore.getState();
    expect(state.selectedMessageUid).toBe(1);
    expect(state.selectedMessageAccountId).toBe('acc-1');
    expect(state.selectedMessageFolder).toBe('INBOX');
  });
});
