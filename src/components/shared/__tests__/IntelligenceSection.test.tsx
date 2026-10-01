import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import IntelligenceSection from '../IntelligenceSection';
import { api } from '@/lib/tauri';
import { useAccountStore } from '@/stores/accountStore';
import type { Account, IntelligenceStatus } from '@/types/email';

vi.mock('@/lib/tauri', () => ({ api: { intelligence: { status: vi.fn(), setAccount: vi.fn(), saveEmbeddings: vi.fn(), testEmbeddings: vi.fn(), runPass: vi.fn() } } }));
const status: IntelligenceStatus = { accounts: [{ account_id: 'a', auto_draft: false, semantic_search: false }], embeddings: { source: 'local', model: 'nomic-embed-text' }, chunks: 10, embedded: 0, pending: 0, suggestions: 0 };
describe('Mail intelligence settings', () => {
  beforeEach(() => {
    vi.clearAllMocks(); vi.mocked(api.intelligence.status).mockResolvedValue(status);
    useAccountStore.setState({ accounts: [{ id: 'a', email: 'me@example.com' } as Account] });
  });
  it('starts off and only enables the account the user selects', async () => {
    render(<IntelligenceSection />);
    const replies = await screen.findByRole('checkbox', { name: 'Prepare replies for me@example.com' });
    await waitFor(() => expect(replies).toBeEnabled());
    expect(replies).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Meaning search for me@example.com' })).not.toBeChecked();
    expect(api.intelligence.runPass).not.toHaveBeenCalled(); expect(api.intelligence.testEmbeddings).not.toHaveBeenCalled();
    await userEvent.click(replies);
    expect(api.intelligence.setAccount).toHaveBeenCalledWith({ account_id: 'a', auto_draft: true, semantic_search: false });
  });
  it('shows indexing progress and keeps refreshing it while a backlog remains', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const indexing = { ...status, accounts: [{ account_id: 'a', auto_draft: false, semantic_search: true }], chunks: 24332, embedded: 12400, pending: 11932 };
      vi.mocked(api.intelligence.status).mockResolvedValueOnce(indexing).mockResolvedValue({ ...indexing, embedded: 24332, pending: 0 });
      render(<IntelligenceSection />);
      expect(await screen.findByText(/12,400 of 24,332 passages indexed.*11,932 still to index/)).toBeInTheDocument();
      await vi.advanceTimersByTimeAsync(5000);
      expect(await screen.findByText(/24,332 of 24,332 passages indexed for meaning search\. Local/)).toBeInTheDocument();
      const calls = vi.mocked(api.intelligence.status).mock.calls.length;
      await vi.advanceTimersByTimeAsync(15000);
      expect(api.intelligence.status).toHaveBeenCalledTimes(calls);
    } finally { vi.useRealTimers(); }
  });
  it('saves embedding preferences without sending a connection test implicitly', async () => {
    render(<IntelligenceSection />); await screen.findByDisplayValue('nomic-embed-text');
    await userEvent.selectOptions(screen.getByLabelText('Embedding service'), 'provider');
    await userEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    expect(api.intelligence.saveEmbeddings).toHaveBeenCalledWith({ source: 'provider', model: 'text-embedding-3-small' });
    expect(api.intelligence.testEmbeddings).not.toHaveBeenCalled();
  });
});
