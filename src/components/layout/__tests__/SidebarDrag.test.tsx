import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "@/components/layout/Sidebar";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import type { Account } from "@/types/email";

vi.mock("@/components/accounts/AccountBadge", () => ({
  default: ({ account }: { account: Account }) => (
    <div data-testid={`badge-${account.id}`}>{account.email}</div>
  ),
}));
vi.mock("@/components/mail/FolderTree", () => ({ default: () => <div>FolderTree</div> }));
vi.mock("@/components/mail/InboxGroupEditor", () => ({ default: () => <div>InboxGroupEditor</div> }));
vi.mock("@/lib/tauri", () => ({
  api: {
    accounts: {
      reorder: vi.fn(() => Promise.resolve()),
      setGroup: vi.fn(() => Promise.resolve()),
    },
    schedule: {
      list: vi.fn(() => Promise.resolve([])),
    },
    needsYou: {
      list: vi.fn(() => Promise.resolve([])),
    },
    inboxGroups: {
      list: vi.fn(() => Promise.resolve([])),
    },
    folders: {
      list: vi.fn(() => Promise.resolve([])),
      sync: vi.fn(() => Promise.resolve([])),
    },
  },
}));

function makeAccount(n: number, groupName: string | null = null): Account {
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
    group_name: groupName,
    notify_enabled: true,
    track_opens_enabled: false,
    hidden_from_aggregates: false,
    triage_enabled: false,
  };
}

const ROW_HEIGHT = 40;

describe("Sidebar Option+drag account reorder", () => {
  const reorderMock = vi.mocked(api.accounts.reorder);
  const setGroupMock = vi.mocked(api.accounts.setGroup);

  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [makeAccount(1), makeAccount(2), makeAccount(3)],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    useMailStore.setState({
      selectedAccountId: null,
      isUnifiedInbox: true,
      selectedAccountGroup: null,
      foldersByAccount: {},
      inboxGroups: [],
      isSyncing: false,
    });
  });

  async function renderSidebar() {
    await act(async () => {
      render(<Sidebar />);
    });
    // badge → drag wrapper (onPointerDown) → row div (registered in accountRefs)
    const wrappers = [1, 2, 3].map(
      (n) => screen.getByTestId(`badge-acc-${n}`).parentElement as HTMLElement,
    );
    const rows = wrappers.map((w) => w.parentElement as HTMLElement);
    rows.forEach((row, i) => {
      vi.spyOn(row, "getBoundingClientRect").mockReturnValue({
        top: i * ROW_HEIGHT,
        bottom: (i + 1) * ROW_HEIGHT,
        height: ROW_HEIGHT,
        left: 0,
        right: 200,
        width: 200,
        x: 0,
        y: i * ROW_HEIGHT,
        toJSON: () => ({}),
      } as DOMRect);
    });
    return { wrappers, rows };
  }

  it("commits the reorder on pointerup and persists via api.accounts.reorder", async () => {
    const { wrappers } = await renderSidebar();

    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    // Drag row 0 down onto row 2 (clientY at row 2's midpoint)
    fireEvent.pointerDown(wrappers[0], { clientY: ROW_HEIGHT / 2 });
    fireEvent.pointerMove(window, { clientY: 2 * ROW_HEIGHT + ROW_HEIGHT / 2 });
    fireEvent.pointerUp(window);

    expect(reorderMock).toHaveBeenCalledTimes(1);
    expect(reorderMock).toHaveBeenCalledWith(["acc-2", "acc-3", "acc-1"]);
    expect(setGroupMock).not.toHaveBeenCalled();
    expect(useAccountStore.getState().accounts.map((a) => a.id)).toEqual([
      "acc-2",
      "acc-3",
      "acc-1",
    ]);
  });

  it("does not reorder when dropped on the starting row", async () => {
    const { wrappers } = await renderSidebar();

    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    fireEvent.pointerDown(wrappers[1], { clientY: ROW_HEIGHT + ROW_HEIGHT / 2 });
    fireEvent.pointerMove(window, { clientY: ROW_HEIGHT + ROW_HEIGHT / 2 + 5 });
    fireEvent.pointerUp(window);

    expect(reorderMock).not.toHaveBeenCalled();
    expect(useAccountStore.getState().accounts.map((a) => a.id)).toEqual([
      "acc-1",
      "acc-2",
      "acc-3",
    ]);
  });

  it("dropping on a grouped row moves the account into that group at that position", async () => {
    // acc-1 and acc-2 are ungrouped; acc-3 lives in "Work". Dragging acc-1
    // onto acc-3's row must move acc-1 into Work next to acc-3.
    useAccountStore.setState({
      accounts: [makeAccount(1), makeAccount(2), makeAccount(3, "Work")],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    const { wrappers } = await renderSidebar();

    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    fireEvent.pointerDown(wrappers[0], { clientY: ROW_HEIGHT / 2 });
    fireEvent.pointerMove(window, { clientY: 2 * ROW_HEIGHT + ROW_HEIGHT / 2 });
    fireEvent.pointerUp(window);

    expect(setGroupMock).toHaveBeenCalledWith("acc-1", "Work");
    await waitFor(() => expect(reorderMock).toHaveBeenCalledTimes(1));
    expect(reorderMock).toHaveBeenCalledWith(["acc-2", "acc-3", "acc-1"]);
    const accounts = useAccountStore.getState().accounts;
    expect(accounts.map((a) => a.id)).toEqual(["acc-2", "acc-3", "acc-1"]);
    expect(accounts.find((a) => a.id === "acc-1")?.group_name).toBe("Work");
  });

  it("dropping on an ungrouped row moves the account out of its group", async () => {
    useAccountStore.setState({
      accounts: [makeAccount(1), makeAccount(2), makeAccount(3, "Work")],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    const { wrappers } = await renderSidebar();

    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    fireEvent.pointerDown(wrappers[2], { clientY: 2 * ROW_HEIGHT + ROW_HEIGHT / 2 });
    fireEvent.pointerMove(window, { clientY: ROW_HEIGHT / 2 });
    fireEvent.pointerUp(window);

    expect(setGroupMock).toHaveBeenCalledWith("acc-3", null);
    await waitFor(() => expect(reorderMock).toHaveBeenCalledTimes(1));
    expect(reorderMock).toHaveBeenCalledWith(["acc-3", "acc-1", "acc-2"]);
    const accounts = useAccountStore.getState().accounts;
    expect(accounts.map((a) => a.id)).toEqual(["acc-3", "acc-1", "acc-2"]);
    expect(accounts.find((a) => a.id === "acc-3")?.group_name).toBeNull();
  });

  it("pointercancel aborts the drag: no commit, and listeners are detached", async () => {
    const { wrappers } = await renderSidebar();

    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    fireEvent.pointerDown(wrappers[0], { clientY: ROW_HEIGHT / 2 });
    fireEvent.pointerMove(window, { clientY: 2 * ROW_HEIGHT + ROW_HEIGHT / 2 });
    fireEvent.pointerCancel(window);
    // A stray pointerup after cancel must not commit either
    fireEvent.pointerUp(window);

    expect(reorderMock).not.toHaveBeenCalled();
    expect(useAccountStore.getState().accounts.map((a) => a.id)).toEqual([
      "acc-1",
      "acc-2",
      "acc-3",
    ]);
  });
});
