import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "@/components/layout/Sidebar";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import type { Account } from "@/types/email";

// The real AccountBadge is under test here — the drag suite mocks it away.
vi.mock("@/components/mail/FolderTree", () => ({ default: () => <div>FolderTree</div> }));
vi.mock("@/components/mail/InboxGroupEditor", () => ({ default: () => <div>InboxGroupEditor</div> }));
vi.mock("@/lib/tauri", () => ({
  api: {
    accounts: {
      reorder: vi.fn(() => Promise.resolve()),
      setGroup: vi.fn(() => Promise.resolve()),
      rename: vi.fn(() => Promise.resolve()),
      list: vi.fn(() => Promise.resolve([])),
    },
    schedule: { list: vi.fn(() => Promise.resolve([])) },
    needsYou: { list: vi.fn(() => Promise.resolve([])) },
    inboxGroups: { list: vi.fn(() => Promise.resolve([])) },
    folders: {
      list: vi.fn(() => Promise.resolve([])),
      sync: vi.fn(() => Promise.resolve([])),
    },
  },
}));

function makeAccount(n: number, color: string | null = null): Account {
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
    color,
    is_active: true,
    sort_order: n - 1,
    group_name: null,
    notify_enabled: true,
    track_opens_enabled: false,
    hidden_from_aggregates: false,
    triage_enabled: false,
  };
}

const row = (name: string) => screen.getByRole("button", { name: new RegExp(name) });
const selectedRows = () => Array.from(document.querySelectorAll('button[aria-current="true"]'));

async function renderSidebar() {
  await act(async () => {
    render(<Sidebar />);
  });
}

describe("Sidebar account selection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [makeAccount(1), makeAccount(2, "#a0aec0"), makeAccount(3)],
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
      foldersByAccount: {},
      inboxGroups: [],
      isSyncing: false,
    });
  });

  it("marks the account whose mailbox is open — in its own colour — and no other", async () => {
    useMailStore.setState({ isUnifiedInbox: false, selectedAccountId: "acc-2", selectedFolder: "INBOX" });
    await renderSidebar();

    const selected = row("Account 2");
    expect(selected).toHaveAttribute("aria-current", "true");
    // The account's colour at 15% behind the row, the name in the colour itself —
    // the same treatment FolderTree gives the selected folder beneath it.
    expect(selected).toHaveStyle({ backgroundColor: "#a0aec026" });
    expect(within(selected).getByText("Account 2")).toHaveStyle({ color: "#a0aec0" });

    expect(row("Account 1")).not.toHaveAttribute("aria-current");
    expect(row("Account 3")).not.toHaveAttribute("aria-current");
    expect(selectedRows()).toHaveLength(1);
  });

  it("follows a click on an account row, and leaves when All Inboxes is chosen", async () => {
    await renderSidebar();
    expect(selectedRows()).toHaveLength(0);

    fireEvent.click(row("Account 3"));
    expect(useMailStore.getState().selectedAccountId).toBe("acc-3");
    expect(row("Account 3")).toHaveAttribute("aria-current", "true");
    expect(selectedRows()).toHaveLength(1);

    fireEvent.click(row("All Inboxes"));
    expect(selectedRows()).toHaveLength(0);
  });

  // The auto-select effect fills `selectedAccountId` with the first account
  // whenever `!isUnifiedInbox` leaves it empty — every special view and every
  // inbox group. A rule that reads the id alone lights an account up under
  // Needs You; these two pin the terms that stop it.
  it("shows no account as selected in a special view, whatever selectedAccountId says", async () => {
    useMailStore.setState({ isUnifiedInbox: false, specialView: "needs_you", selectedAccountId: "acc-1" });
    await renderSidebar();
    expect(selectedRows()).toHaveLength(0);
  });

  it("shows no account as selected in an inbox group, whatever selectedAccountId says", async () => {
    useMailStore.setState({ isUnifiedInbox: false, selectedGroupId: 7, selectedAccountId: "acc-1" });
    await renderSidebar();
    expect(selectedRows()).toHaveLength(0);
  });
});
