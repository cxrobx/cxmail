/**
 * The generic IMAP setup form. What is actually worth pinning here is the
 * PAYLOAD: the UI must send only what the user typed and let the backend
 * resolve the rest. A UI that helpfully invents a hostname would send the
 * user's password to a server nobody chose, and no backend validation can
 * catch that because the request looks deliberate.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AccountSetup from '@/components/accounts/AccountSetup';
import { api } from '@/lib/tauri';

vi.mock('@/lib/tauri', () => ({
  api: {
    auth: {
      startOAuth2: vi.fn(),
      addICloudAccount: vi.fn(),
      addImapAccount: vi.fn(),
      discoverMailConfig: vi.fn(),
    },
  },
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock('@/stores/accountStore', () => ({
  useAccountStore: () => ({ addAccount: vi.fn() }),
}));

const addImap = api.auth.addImapAccount as ReturnType<typeof vi.fn>;
const discover = api.auth.discoverMailConfig as ReturnType<typeof vi.fn>;

/** A tier-0 curated preset: the two-field path. */
const PRESET_HIT = {
  display_name: 'Fastmail', imap_host: 'imap.fastmail.com', imap_port: 993,
  imap_security: 'implicit', imap_username: null,
  smtp_host: 'smtp.fastmail.com', smtp_port: 465, smtp_security: 'implicit',
  smtp_username: null, source: 'preset', hint: 'Create an app password.',
};

/** A network tier: must be shown to the user before any password is sent. */
const NETWORK_HIT = {
  display_name: 'Example Mail', imap_host: 'imap.example.net', imap_port: 993,
  imap_security: 'implicit', imap_username: 'chris',
  smtp_host: 'smtp.example.net', smtp_port: 465, smtp_security: 'implicit',
  smtp_username: 'chris', source: 'mx', hint: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  addImap.mockResolvedValue(undefined);
  discover.mockResolvedValue(null);
});

async function openImapForm() {
  const user = userEvent.setup();
  render(<AccountSetup />);
  await user.click(screen.getByRole('button', { name: /Other mail account/i }));
  return user;
}

describe('generic IMAP account setup', () => {
  it('is reachable as a fourth option without disturbing the OAuth defaults', async () => {
    render(<AccountSetup />);
    // OAuth stays the headline path; generic IMAP is an addition, not a swap.
    expect(screen.getByRole('button', { name: /Connect Gmail/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Connect Outlook/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Connect iCloud/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Other mail account/i })).toBeTruthy();
  });

  it('keeps a known domain to two fields and sends no invented servers', async () => {
    discover.mockResolvedValue(PRESET_HIT);
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@fastmail.com');
    await user.type(screen.getByLabelText(/^Password$/i), 'app-password');

    await waitFor(() =>
      expect(screen.getByText(/Recognized as/i).textContent).toContain('Fastmail'),
    );

    await user.click(screen.getByRole('button', { name: /Connect account/i }));

    await waitFor(() => expect(addImap).toHaveBeenCalledTimes(1));
    const [email, password, settings] = addImap.mock.calls[0];
    expect(email).toBe('chris@fastmail.com');
    expect(password).toBe('app-password');
    // The preset id is a pointer for the backend to resolve; the UI must NOT
    // fill the host/port itself from its own copy of the table.
    expect(settings.preset_id).toBe('fastmail');
    expect(settings.imap_host).toBeNull();
    expect(settings.imap_port).toBeNull();
    expect(settings.smtp_host).toBeNull();
    expect(settings.smtp_port).toBeNull();
    expect(settings.imap_username).toBeNull();
    expect(settings.smtp_username).toBeNull();
  });

  it('opens server settings on its own for an unfamiliar domain', async () => {
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@cxventures.io');

    // Discovery found nothing, so the fields must be visible without the user
    // having to find a disclosure triangle.
    await waitFor(() => expect(screen.getByText(/Couldn't find settings/i)).toBeTruthy());
    expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy();
    expect(screen.getByLabelText(/Outgoing \(SMTP\) server/i)).toBeTruthy();
  });

  it('sends typed server settings, with ports as numbers', async () => {
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@cxventures.io');
    await user.type(screen.getByLabelText(/^Password$/i), 'secret');
    // The fields appear once discovery gives up; until then only the
    // disclosure button is on screen.
    await waitFor(() => expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy());
    await user.type(screen.getByLabelText(/Incoming \(IMAP\) server/i), 'mail.cxventures.io');
    await user.type(screen.getByLabelText(/Outgoing \(SMTP\) server/i), 'smtp.cxventures.io');

    const ports = screen.getAllByLabelText(/^Port$/i);
    await user.type(ports[0], '993');
    await user.type(ports[1], '465');

    await user.click(screen.getByRole('button', { name: /Connect account/i }));

    await waitFor(() => expect(addImap).toHaveBeenCalledTimes(1));
    const settings = addImap.mock.calls[0][2];
    expect(settings.preset_id).toBeNull();
    expect(settings.imap_host).toBe('mail.cxventures.io');
    expect(settings.smtp_host).toBe('smtp.cxventures.io');
    // Numbers, not strings — the Rust side deserializes Option<i32>.
    expect(settings.imap_port).toBe(993);
    expect(settings.smtp_port).toBe(465);
    expect(typeof settings.imap_port).toBe('number');
    // Nothing was discovered, so security must be left for the backend to
    // derive from the port rather than invented here.
    expect(settings.imap_security).toBeNull();
    expect(settings.smtp_security).toBeNull();
  });

  it('reveals server settings when verification fails', async () => {
    discover.mockResolvedValue(PRESET_HIT);
    addImap.mockRejectedValueOnce(
      new Error('IMAP connection to imap.fastmail.com:993 failed: LOGIN auth failed'),
    );
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@fastmail.com');
    await user.type(screen.getByLabelText(/^Password$/i), 'wrong');

    // A recognized domain starts collapsed...
    await waitFor(() => expect(screen.getByText(/Recognized as/i)).toBeTruthy());
    expect(screen.queryByLabelText(/Incoming \(IMAP\) server/i)).toBeNull();

    await user.click(screen.getByRole('button', { name: /Connect account/i }));

    // ...and opens on failure, because a failure means the settings are the
    // thing in question. The backend's message is shown verbatim: it names the
    // host and port it actually tried.
    await waitFor(() =>
      expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy(),
    );
    expect(screen.getByText(/imap\.fastmail\.com:993/)).toBeTruthy();
  });

  it('will not submit without both an address and a password', async () => {
    const user = await openImapForm();
    const submit = screen.getByRole('button', { name: /Connect account/i }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    await user.type(screen.getByLabelText(/Email Address/i), 'chris@fastmail.com');
    expect(submit.disabled).toBe(true);

    await user.type(screen.getByLabelText(/^Password$/i), 'pw');
    expect(submit.disabled).toBe(false);
  });

  it('shows a network-discovered server so the user sees it before sending a password', async () => {
    discover.mockResolvedValue(NETWORK_HIT);
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@cxventures.io');

    // DNS is spoofable without DNSSEC, so anything found over the network must
    // be visible — never silently used.
    await waitFor(() =>
      expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy(),
    );
    expect(
      (screen.getByLabelText(/Incoming \(IMAP\) server/i) as HTMLInputElement).value,
    ).toBe('imap.example.net');
    expect(screen.getByText(/found in your domain's DNS/i)).toBeTruthy();
  });

  it('lets the user open server settings manually while discovery is still running', async () => {
    // An unknown domain can spend seconds in the network tiers; the manual
    // route must not be gated behind that.
    discover.mockImplementation(() => new Promise(() => {})); // never resolves
    const user = await openImapForm();
    await user.type(screen.getByLabelText(/Email Address/i), 'chris@slow.example');
    expect(screen.queryByLabelText(/Incoming \(IMAP\) server/i)).toBeNull();

    await user.click(screen.getByRole('button', { name: /Server settings/i }));
    expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy();
  });

  /**
   * A slow lookup for an older address must not overwrite a newer one. The
   * 500ms debounce cancels most of these, so the guard only matters when the
   * first lookup was actually ISSUED and is merely slow to answer — which is
   * what this reproduces.
   */
  it('ignores a stale discovery result for a superseded address', async () => {
    let resolveFirst: (v: unknown) => void = () => {};
    discover
      .mockImplementationOnce(() => new Promise((r) => { resolveFirst = r; }))
      .mockResolvedValue(NETWORK_HIT);

    const user = await openImapForm();
    const field = screen.getByLabelText(/Email Address/i);
    await user.type(field, 'old@slow.example');

    // Let the debounce elapse so the FIRST lookup is genuinely in flight.
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(1), { timeout: 2000 });

    await user.clear(field);
    await user.type(field, 'chris@cxventures.io');
    await waitFor(() => expect(discover).toHaveBeenCalledTimes(2), { timeout: 2000 });
    await waitFor(() =>
      expect(screen.getByLabelText(/Incoming \(IMAP\) server/i)).toBeTruthy(),
    );

    // The stale response lands late with different data; it must be discarded.
    resolveFirst({ ...NETWORK_HIT, imap_host: 'imap.STALE.example' });
    await new Promise((r) => setTimeout(r, 60));
    expect(
      (screen.getByLabelText(/Incoming \(IMAP\) server/i) as HTMLInputElement).value,
    ).toBe('imap.example.net');
  });
});
