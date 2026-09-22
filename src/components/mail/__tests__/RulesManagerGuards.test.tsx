/**
 * Guards on the rule editor (gotcha #36). The MCP boundary refuses dead and
 * whole-mailbox rules; this pins the same refusals in the UI, which was the
 * only remaining way to create one.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import RulesManager from '@/components/mail/RulesManager';
import { api } from '@/lib/tauri';
import type { MailRule } from '@/types/email';

vi.mock('@/lib/tauri', () => ({
  api: {
    rules: {
      list: vi.fn(),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
      reclassifyAll: vi.fn(),
    },
  },
}));

const listMock = api.rules.list as ReturnType<typeof vi.fn>;
const createMock = api.rules.create as ReturnType<typeof vi.fn>;
const deleteMock = api.rules.delete as ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  listMock.mockResolvedValue([]);
  createMock.mockResolvedValue(1);
  deleteMock.mockResolvedValue(undefined);
});

async function openNewRuleForm() {
  const user = userEvent.setup();
  render(<RulesManager onClose={() => {}} />);
  await screen.findByText(/No rules yet/i);
  await user.click(screen.getByRole('button', { name: /New/i }));
  await screen.findByText('New Rule');
  return user;
}

describe('RulesManager guards', () => {
  it('does not offer "body" as a condition field', async () => {
    await openNewRuleForm();
    const fieldSelect = screen.getAllByRole('combobox')[0];
    const labels = within(fieldSelect)
      .getAllByRole('option')
      .map((o) => o.textContent?.toLowerCase());
    expect(labels).toEqual(['from', 'subject', 'to']);
    expect(labels).not.toContain('body');
  });

  it('refuses to save a rule whose condition value is blank, and says why', async () => {
    const user = await openNewRuleForm();
    await user.type(screen.getByPlaceholderText(/Block crypto spam/i), 'Half finished');
    // The condition value is left at its initial "".
    await user.click(screen.getByRole('button', { name: 'Create' }));

    const alert = await screen.findByTestId('rule-errors');
    expect(alert).toHaveTextContent(/matches every message/i);
    expect(alert).toHaveTextContent(/whole mailbox/i);
    expect(createMock).not.toHaveBeenCalled();
  });

  it('refuses to re-save a stored rule that has zero conditions', async () => {
    // The editor's per-row delete is hidden at one condition, so this state is
    // only reachable for a rule written by another client — exactly the case
    // that would otherwise match every message on the next sync.
    listMock.mockResolvedValue([
      {
        id: 3,
        account_id: null,
        name: 'Matches everything',
        is_active: true,
        priority: 0,
        conditions: [],
        actions: [{ action_type: 'mark_read', value: null }],
      } satisfies MailRule,
    ]);
    const user = userEvent.setup();
    render(<RulesManager onClose={() => {}} />);
    await user.click(await screen.findByText('Matches everything'));
    await screen.findByText('Edit Rule');
    await user.click(screen.getByRole('button', { name: 'Save' }));

    const alert = await screen.findByTestId('rule-errors');
    expect(alert).toHaveTextContent(/at least one condition/i);
    expect(alert).toHaveTextContent(/whole mailbox/i);
    expect(api.rules.update).not.toHaveBeenCalled();
  });

  it('refuses a nameless rule instead of silently doing nothing', async () => {
    const user = await openNewRuleForm();
    await user.type(screen.getByPlaceholderText(/value/i), 'crypto');
    await user.click(screen.getByRole('button', { name: 'Create' }));
    expect(await screen.findByTestId('rule-errors')).toHaveTextContent(/name/i);
    expect(createMock).not.toHaveBeenCalled();
  });

  it('clears the refusal once the problem is fixed, and then saves', async () => {
    const user = await openNewRuleForm();
    await user.type(screen.getByPlaceholderText(/Block crypto spam/i), 'Block crypto spam');
    await user.click(screen.getByRole('button', { name: 'Create' }));
    await screen.findByTestId('rule-errors');

    await user.type(screen.getByPlaceholderText(/value/i), 'crypto');
    expect(screen.queryByTestId('rule-errors')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() => expect(createMock).toHaveBeenCalledTimes(1));
    expect(createMock.mock.calls[0][0]).toMatchObject({
      name: 'Block crypto spam',
      conditions: [{ field: 'from', operator: 'contains', value: 'crypto' }],
      actions: [{ action_type: 'set_category', value: 'junk' }],
    });
  });

  it('lists an existing rule, annotates a legacy body rule, and deletes it', async () => {
    const legacy: MailRule = {
      id: 7,
      account_id: null,
      name: 'Legacy body rule',
      is_active: true,
      priority: 0,
      conditions: [{ field: 'body', operator: 'contains', value: 'invoice' }],
      actions: [{ action_type: 'mark_read', value: null }],
    };
    listMock.mockResolvedValue([legacy]);
    const user = userEvent.setup();
    render(<RulesManager onClose={() => {}} />);

    await screen.findByText('Legacy body rule');
    expect(screen.getByTestId('rule-warnings')).toHaveTextContent(/never match/i);

    const row = screen.getByText('Legacy body rule').closest('div.rounded-md')!;
    const trash = within(row as HTMLElement)
      .getAllByRole('button')
      .find((b) => b.querySelector('svg.lucide-trash2'))!;
    await user.click(trash);
    await waitFor(() => expect(deleteMock).toHaveBeenCalledWith(7));
  });
});
