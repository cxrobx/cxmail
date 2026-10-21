import { describe, it, expect, vi, beforeEach } from 'vitest';
import { openMessageWindow } from '@/lib/openMessageWindow';
import { openDraftForEdit } from '@/lib/draftCompose';
import { useWindowStore } from '@/stores/windowStore';
import { useMailStore } from '@/stores/mailStore';

vi.mock('@/lib/draftCompose', () => ({ openDraftForEdit: vi.fn() }));

describe('openMessageWindow', () => {
  beforeEach(() => {
    vi.mocked(openDraftForEdit).mockClear();
    useWindowStore.setState({ windows: [] } as never);
    useMailStore.setState({ foldersByAccount: {} });
  });

  it('pops a received message into one keyed email window, however many times it is asked', () => {
    openMessageWindow('acct', 'INBOX', 42, 'Fwd: Chris Tax Info');
    openMessageWindow('acct', 'INBOX', 42, 'Fwd: Chris Tax Info');
    const wins = useWindowStore.getState().windows;
    expect(wins).toHaveLength(1);
    expect(wins[0]).toMatchObject({ type: 'email', title: 'Fwd: Chris Tax Info', props: { accountId: 'acct', folder: 'INBOX', uid: 42 } });
    expect(openDraftForEdit).not.toHaveBeenCalled();
  });

  it('opens a draft in the composer instead of a read-only window', () => {
    openMessageWindow('acct', '[Gmail]/Drafts', 7, 'Fwd: Music Expenses');
    expect(openDraftForEdit).toHaveBeenCalledWith('acct', '[Gmail]/Drafts', 7);
    expect(useWindowStore.getState().windows).toHaveLength(0);
  });

  it('recognizes a custom-named drafts folder from the synced folder list', () => {
    useMailStore.setState({
      foldersByAccount: { acct: [{ name: 'Brouillons', folder_type: 'drafts' } as never] },
    });
    openMessageWindow('acct', 'Brouillons', 3, null);
    expect(openDraftForEdit).toHaveBeenCalledWith('acct', 'Brouillons', 3);
  });
});
