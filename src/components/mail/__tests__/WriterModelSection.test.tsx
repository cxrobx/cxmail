/**
 * The default-writer-model setting (agent draft writing via `agy`).
 *
 * Three behaviours worth pinning: choosing the "Default" option must save
 * `null` (saving `""` would be rejected by the Rust side and the setting would
 * appear stuck — same trap ClaudeRepoSettings pins); the dropdown only exists
 * when `agy models` answered, degrading to a free-text id field otherwise
 * rather than a dead control; and a save failure surfaces the backend's own
 * refusal text (it names valid ids), not a generic error.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { WriterModelSection } from '@/components/mail/AIProviderSettings';
import { api, type WriterSettings } from '@/lib/tauri';

vi.mock('@/lib/tauri', () => ({
  api: {
    ai: {
      getProviderSettings: vi.fn(),
      getWriterSettings: vi.fn(),
      saveWriterModel: vi.fn(),
      listWriterModels: vi.fn(),
    },
  },
}));

const mocked = api.ai as unknown as {
  getWriterSettings: ReturnType<typeof vi.fn>;
  saveWriterModel: ReturnType<typeof vi.fn>;
  listWriterModels: ReturnType<typeof vi.fn>;
};

const settings = (model: string | null): WriterSettings => ({
  model,
  effectiveModel: model ?? 'gemini-3.7-flash-high',
  builtinDefault: 'gemini-3.7-flash-high',
});

const MODELS = [
  { id: 'gemini-3.7-flash-high', label: 'Gemini 3.7 Flash (High)' },
  { id: 'gemini-3.6-flash-low', label: 'Gemini 3.6 Flash (Low)' },
];

beforeEach(() => {
  vi.clearAllMocks();
  mocked.getWriterSettings.mockResolvedValue(settings(null));
  mocked.listWriterModels.mockResolvedValue(MODELS);
  mocked.saveWriterModel.mockImplementation(async (model: string | null) =>
    settings(model),
  );
});

describe('WriterModelSection', () => {
  it('lists agy models plus a Default option naming the built-in', async () => {
    render(<WriterModelSection />);
    const select = await screen.findByLabelText('Writer model');
    const labels = Array.from(select.querySelectorAll('option')).map(
      (o) => o.textContent,
    );
    expect(labels).toEqual([
      'Default (gemini-3.7-flash-high)',
      'Gemini 3.7 Flash (High)',
      'Gemini 3.6 Flash (Low)',
    ]);
  });

  it('selecting a model saves its id; selecting Default saves null, never ""', async () => {
    const user = userEvent.setup();
    render(<WriterModelSection />);
    const select = await screen.findByLabelText('Writer model');

    await user.selectOptions(select, 'gemini-3.6-flash-low');
    await waitFor(() =>
      expect(mocked.saveWriterModel).toHaveBeenCalledWith('gemini-3.6-flash-low'),
    );
    expect(
      await screen.findByText(/Saved — drafts will use gemini-3.6-flash-low/),
    ).toBeTruthy();

    await user.selectOptions(select, '');
    await waitFor(() => expect(mocked.saveWriterModel).toHaveBeenLastCalledWith(null));
  });

  it('degrades to a free-text id field when agy models is unavailable', async () => {
    mocked.listWriterModels.mockRejectedValue(new Error('agy not found'));
    const user = userEvent.setup();
    render(<WriterModelSection />);

    const input = await screen.findByLabelText('Writer model id');
    expect(screen.queryByLabelText('Writer model')).toBeNull();

    await user.type(input, 'gemini-3.1-pro-high');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(mocked.saveWriterModel).toHaveBeenCalledWith('gemini-3.1-pro-high'),
    );
  });

  it('surfaces the backend refusal text on a bad id', async () => {
    mocked.listWriterModels.mockRejectedValue(new Error('agy not found'));
    mocked.saveWriterModel.mockRejectedValue(
      '"--x" is not a valid writer model id — expected a bare id like gemini-3.7-flash-high (see `agy models`)',
    );
    const user = userEvent.setup();
    render(<WriterModelSection />);

    await user.type(await screen.findByLabelText('Writer model id'), '--x');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText(/not a valid writer model id/)).toBeTruthy();
  });
});
