import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "@/components/layout/Sidebar";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import type { Account, Folder } from "@/types/email";

// The real AccountBadge renders the "hidden" glyph under test here.
vi.mock("@/components/mail/FolderTree", () => ({ default: () => <div>FolderTree</div> }));
vi.mock("@/components/mail/InboxGroupEditor", () => ({ default: () => <div>InboxGroupEditor</div> }));
vi.mock("@/lib/tauri", () => ({
  api: {
    accounts: {
      reorder: vi.fn(() => Promise.resolve()),
      setGroup: vi.fn(() => Promise.resolve()),
      rename: vi.fn(() => Promise.resolve()),
      setHiddenFromAggregates: vi.fn(() => Promise.resolve()),
      list: vi.fn(() => Promise.resolve([])),
    },
    schedule: { list: vi.fn(() => Promise.resolve([])) },
    needsYou: { list: vi.fn(() => Promise.resolve([])) },
    inboxGroups: { list: vi.fn(() => Promise.resolve([])) },
    folders: {
      list: vi.fn(() => Promise.resolve([])),
      sync: vi.fn(() => Promise.resolve([])),
    },
    // Opening the context menu fetches these for the account.
    ai: {
      getVoiceProfileStatus: vi.fn(() => Promise.resolve({ exists: false, sample_count: 0, generated_at: null })),
      getInsightStatus: vi.fn(() => Promise.resolve({ exists: false })),
      listArchetypes: vi.fn(() => Promise.resolve([])),
    },
  },
}));

function makeAccount(n: number, opts: { hidden?: boolean; group?: string | null } = {}): Account {
  return {
    id: `acc-${n}`,
    email: `person${n}@example.com`,
    display_name: `Account ${n}`,
    provider: "gmail",
    imap_host: "imap.gmail.com",
    imap_port: 993,
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    imap_security: "implicit",
    smtp_security: "starttls",
    imap_username: null,
    smtp_username: null,
    color: null,
    is_active: true,
    sort_order: n - 1,
    group_name: opts.group ?? null,
    notify_enabled: true,
    track_opens_enabled: false,
    hidden_from_aggregates: opts.hidden ?? false,
    triage_enabled: false,
  };
}

function inbox(accountId: string, unread: number): Folder {
  return {
    id: 1,
    account_id: accountId,
    name: "INBOX",
    display_name: null,
    folder_type: "inbox",
    delimiter: "/",
    total_count: unread,
    unread_count: unread,
    uidvalidity: null,
    uidnext: null,
  };
}

const row = (name: string) => screen.getByRole("button", { name: new RegExp(name) });

async function renderSidebar() {
  await act(async () => {
    render(<Sidebar />);
  });
}

// Sidebar reloads each account's folders on mount from `api.folders.list`, so
// the counts under test have to come from the mock, not from the store alone.
const FOLDERS: Record<string, Folder[]> = {
  "acc-1": [inbox("acc-1", 5)],
  "acc-2": [inbox("acc-2", 7)],
  "acc-3": [inbox("acc-3", 11)],
};

describe("Sidebar — accounts hidden from aggregates", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.folders.list).mockImplementation((id: string) => Promise.resolve(FOLDERS[id] ?? []));
    useAccountStore.setState({
      accounts: [
        makeAccount(1, { group: "BUSINESS" }),
        makeAccount(2, { group: "BUSINESS", hidden: true }),
        makeAccount(3),
      ],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    useMailStore.setState({
      selectedAccountId: null,
      isUnifiedInbox: true,
      selectedAccountGroup: null,
      specialView: null,
      selectedGroupId: null,
      selectedFolder: null,
      foldersByAccount: { ...FOLDERS },
      inboxGroups: [],
      isSyncing: false,
    });
  });

  // These two sums never touch the DB, so the rule has to be applied in the
  // component — each is one line, and each is a place the count would lie.
  it("leaves the hidden account's unread out of the All Inboxes count", async () => {
    await renderSidebar();
    // 5 + 11, not 5 + 7 + 11.
    expect(within(row("All Inboxes")).getByText("16")).toBeInTheDocument();
  });

  it("leaves the hidden account's unread out of its account folder's count", async () => {
    await renderSidebar();
    // BUSINESS = acc-1 (5) + acc-2 (7, hidden) → 5.
    expect(within(row("BUSINESS")).getByText("5")).toBeInTheDocument();
  });

  it("marks the hidden account's row, and says how to reach its mail", async () => {
    await renderSidebar();
    const glyphs = screen.getAllByTestId("hidden-from-aggregates");
    expect(glyphs).toHaveLength(1);
    expect(row("Account 2")).toContainElement(glyphs[0]);
    expect(glyphs[0]).toHaveAttribute("title", expect.stringContaining("Click the account to see its mail"));
  });

  it("toggles the flag from the account's context menu and refreshes every surface", async () => {
    // What the backend hands back after the first toggle: 3 now hidden, 2 still hidden.
    vi.mocked(api.accounts.list).mockResolvedValue([
      makeAccount(1, { group: "BUSINESS" }),
      makeAccount(2, { group: "BUSINESS", hidden: true }),
      makeAccount(3, { hidden: true }),
    ]);
    const refresh = vi.fn();
    window.addEventListener("cxmail:refresh-messages", refresh);
    await renderSidebar();

    fireEvent.contextMenu(row("Account 3"));
    const hide = screen.getByRole("button", { name: "Hide from All Inboxes & groups" });
    await act(async () => {
      fireEvent.click(hide);
    });
    expect(api.accounts.setHiddenFromAggregates).toHaveBeenCalledWith("acc-3", true);
    expect(api.accounts.list).toHaveBeenCalled();
    expect(refresh).toHaveBeenCalled();

    // The hidden account offers the inverse.
    fireEvent.contextMenu(row("Account 2"));
    const show = screen.getByRole("button", { name: "Show in All Inboxes & groups" });
    await act(async () => {
      fireEvent.click(show);
    });
    expect(api.accounts.setHiddenFromAggregates).toHaveBeenLastCalledWith("acc-2", false);
    window.removeEventListener("cxmail:refresh-messages", refresh);
  });
});
