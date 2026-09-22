/**
 * The settings panel for "Open in Claude" repo mappings.
 *
 * Two behaviours here are easy to get wrong in ways nothing else catches:
 * emptying a field must CLEAR the mapping rather than try to save `""` (which
 * the Rust side rejects, so the mapping would appear stuck), and the panel must
 * surface the backend's own refusal text — "must be absolute", "not an email
 * address or a domain" — because a generic failure gives the user nothing to
 * act on.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ClaudeRepoSettings from '@/components/mail/ClaudeRepoSettings';
import { api, type ClaudeRepo } from '@/lib/tauri';

vi.mock('@/lib/tauri', () => ({
  api: {
    claude: {
      listRepos: vi.fn(),
      setRepo: vi.fn(),
      clearRepo: vi.fn(),
    },
  },
}));

vi.mock('@/stores/accountStore', () => ({
  useAccountStore: (selector: (s: unknown) => unknown) =>
    selector({
      accounts: [
        { id: 'acct-cxv', email: 'chris@cxventures.io', color: null },
        { id: 'acct-pb', email: 'chris@pocketbuddy.org', color: null },
      ],
    }),
}));

vi.mock('@/stores/mailStore', () => ({
  useMailStore: (selector: (s: unknown) => unknown) =>
    selector({ inboxGroups: [{ id: 4, name: 'Northwind', color: '#0a84ff', icon: 'folder' }] }),
}));

const listMock = api.claude.listRepos as ReturnType<typeof vi.fn>;
const setMock = api.claude.setRepo as ReturnType<typeof vi.fn>;
const clearMock = api.claude.clearRepo as ReturnType<typeof vi.fn>;

const contactRow: ClaudeRepo = {
  scope: 'contact',
  contact: 'northwind.example',
  group_id: null,
  account_id: null,
  repo_path: '/Users/x/Projects/cxventures/clients/northwind',
  exists: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  listMock.mockResolvedValue([contactRow]);
  setMock.mockResolvedValue(contactRow);
  clearMock.mockResolvedValue(undefined);
});

async function renderPanel() {
  const user = userEvent.setup();
  render(<ClaudeRepoSettings onClose={() => {}} />);
  await screen.findByDisplayValue('/Users/x/Projects/cxventures/clients/northwind');
  return user;
}

describe('ClaudeRepoSettings', () => {
  it('lists every scope, in the order they are resolved', async () => {
    await renderPanel();
    const headings = screen
      .getAllByText(/Correspondents|Groups|Accounts|Everything else/)
      .map((el) => el.textContent);
    expect(headings).toEqual(['Correspondents', 'Groups', 'Accounts', 'Everything else']);
  });

  it('saves a path typed against a group', async () => {
    const user = await renderPanel();
    const field = screen.getByLabelText('Repo path for Northwind');
    await user.click(field);
    await user.type(field, '~/Projects/cxventures{Enter}');
    await waitFor(() =>
      expect(setMock).toHaveBeenCalledWith(
        { scope: 'group', group_id: 4 },
        '~/Projects/cxventures',
      ),
    );
  });

  it('emptying a field clears the mapping instead of saving a blank path', async () => {
    const user = await renderPanel();
    const field = screen.getByDisplayValue('/Users/x/Projects/cxventures/clients/northwind');
    await user.clear(field);
    await user.tab();
    await waitFor(() =>
      expect(clearMock).toHaveBeenCalledWith({ scope: 'contact', contact: 'northwind.example' }),
    );
    expect(setMock).not.toHaveBeenCalled();
  });

  it('shows the backend refusal verbatim, so the user knows what to fix', async () => {
    setMock.mockRejectedValue(
      new Error("Repo path must be absolute (or start with ~/), got 'Projects/cxventures'"),
    );
    const user = await renderPanel();
    const field = screen.getByLabelText('Repo path for chris@cxventures.io');
    await user.click(field);
    await user.type(field, 'Projects/cxventures{Enter}');
    expect(await screen.findByText(/must be absolute/)).toBeInTheDocument();
  });

  it('flags a mapping whose directory is not there', async () => {
    listMock.mockResolvedValue([{ ...contactRow, exists: false }]);
    render(<ClaudeRepoSettings onClose={() => {}} />);
    expect(await screen.findByLabelText('Directory not found')).toBeInTheDocument();
  });

  it('does not write anything when a field is left untouched', async () => {
    const user = await renderPanel();
    const field = screen.getByDisplayValue('/Users/x/Projects/cxventures/clients/northwind');
    await user.click(field);
    await user.tab();
    expect(setMock).not.toHaveBeenCalled();
    expect(clearMock).not.toHaveBeenCalled();
  });
});
