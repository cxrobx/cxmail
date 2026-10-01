import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ReplySuggestionCard from '../ReplySuggestionCard';
import { api } from '@/lib/tauri';
import type { ReplySuggestion } from '@/types/email';

vi.mock('@/lib/tauri', () => ({ api: { intelligence: { suggestion: vi.fn(), dismissSuggestion: vi.fn() } } }));
const suggestion = { body: 'I can review [confirm date]. <img onerror="bad">', model: 'test', created_at: '' };
describe('Reply suggestions', () => {
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(api.intelligence.suggestion).mockResolvedValue(suggestion); });
  it('renders inert text and revalidates before opening the composer', async () => {
    const onUse = vi.fn(); render(<ReplySuggestionCard accountId="a" folder="INBOX" uid={1} onUse={onUse} />);
    await screen.findByText(suggestion.body);
    expect(document.querySelector('img')).toBeNull();
    vi.mocked(api.intelligence.suggestion).mockResolvedValue(null);
    await userEvent.click(screen.getByRole('button', { name: 'Review reply' }));
    expect(onUse).not.toHaveBeenCalled();
    expect(screen.queryByText('Suggested reply')).not.toBeInTheDocument();
  });
  it('cannot replace an existing human reply', async () => {
    const onUse = vi.fn(); render(<ReplySuggestionCard accountId="a" folder="INBOX" uid={1} onUse={onUse} disabled />);
    expect(await screen.findByRole('button', { name: 'Review reply' })).toBeDisabled();
    expect(onUse).not.toHaveBeenCalled();
  });
  it('ignores a previous message lookup completing after navigation', async () => {
    let resolve!: (value: ReplySuggestion) => void;
    vi.mocked(api.intelligence.suggestion).mockReturnValueOnce(new Promise((r) => { resolve = r; })).mockResolvedValueOnce(null);
    const { rerender } = render(<ReplySuggestionCard accountId="a" folder="INBOX" uid={1} onUse={vi.fn()} />);
    rerender(<ReplySuggestionCard accountId="b" folder="Archive" uid={2} onUse={vi.fn()} />);
    await act(async () => resolve(suggestion));
    await waitFor(() => expect(screen.queryByText(suggestion.body)).not.toBeInTheDocument());
  });
});
