import { describe, it, expect, vi, beforeEach } from 'vitest';
import { check } from '@tauri-apps/plugin-updater';
import { api } from '@/lib/tauri';
import { checkForAppUpdate } from '@/lib/updater';

vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }));
vi.mock('@/stores/uiStore', () => ({
  useUIStore: { getState: () => ({ addToast: vi.fn() }) },
}));
vi.mock('@/lib/tauri', () => ({
  api: {
    license: { getStatus: vi.fn() },
    system: { logClientError: vi.fn() },
  },
}));

// checkForAppUpdate early-returns unless PROD, which is exactly the branch under
// test — so the suite is worthless without this.
vi.stubEnv('PROD', true);

const status = (maskedKey: string | null) => ({
  active: true,
  maskedKey,
  validatedAt: null,
  offlineGrace: false,
  developmentBuild: false,
});

describe('checkForAppUpdate — owner builds must not self-replace', () => {
  beforeEach(() => vi.clearAllMocks());

  // The whole point. A published build carries no owner bypass, so installing
  // one over an owner build drops the maintainer behind the "Activate CXMail"
  // gate with no key — one click on an update toast locks them out of their
  // own mail. Today this is masked by 1.0.0 == 1.0.0 offering no update; it
  // goes live the moment a higher version is published.
  it('does not even check for an update on an owner build', async () => {
    vi.mocked(api.license.getStatus).mockResolvedValue(status('OWNER'));
    await checkForAppUpdate();
    expect(check).not.toHaveBeenCalled();
  });

  it('checks normally for a licensed customer build', async () => {
    vi.mocked(api.license.getStatus).mockResolvedValue(status('CXM-••••-1234'));
    vi.mocked(check).mockResolvedValue(null as never);
    await checkForAppUpdate();
    expect(check).toHaveBeenCalledOnce();
  });

  it('checks normally when no license is present', async () => {
    vi.mocked(api.license.getStatus).mockResolvedValue(status(null));
    vi.mocked(check).mockResolvedValue(null as never);
    await checkForAppUpdate();
    expect(check).toHaveBeenCalledOnce();
  });

  // The guard must never become the reason customers stop receiving updates.
  it('falls through to the check when the license status cannot be read', async () => {
    vi.mocked(api.license.getStatus).mockRejectedValue(new Error('IPC down'));
    vi.mocked(check).mockResolvedValue(null as never);
    await checkForAppUpdate();
    expect(check).toHaveBeenCalledOnce();
  });

  // Regression guard for the reason this file exists at all: a failed check
  // used to vanish into a console no release build has.
  it('reports a failed check to the Rust log, not just the console', async () => {
    vi.mocked(api.license.getStatus).mockResolvedValue(status(null));
    vi.mocked(check).mockRejectedValue(new Error('endpoint 500'));
    await checkForAppUpdate();
    expect(api.system.logClientError).toHaveBeenCalledWith('updater', expect.stringContaining('endpoint 500'));
  });
});
