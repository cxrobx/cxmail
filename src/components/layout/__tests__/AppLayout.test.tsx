import { act, render, waitFor } from "@testing-library/react";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppLayout from "@/components/layout/AppLayout";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useWindowStore } from "@/stores/windowStore";

vi.mock("@/components/layout/Sidebar", () => ({ default: () => <div>Sidebar</div> }));
vi.mock("@/components/layout/StatusBar", () => ({ default: () => <div>StatusBar</div> }));
vi.mock("@/components/mail/MessageList", () => ({ default: () => <div>MessageList</div> }));
vi.mock("@/components/mail/ReadingPane", () => ({ default: () => <div>ReadingPane</div> }));
vi.mock("@/components/mail/ComposeModal", () => ({ default: () => <div>ComposeModal</div> }));
vi.mock("@/components/mail/SearchBar", () => ({ default: () => <div>SearchBar</div> }));
vi.mock("@/components/shared/CommandPalette", () => ({ default: () => <div>CommandPalette</div> }));
vi.mock("@/components/shared/KeyboardShortcutSheet", () => ({ default: () => <div>KeyboardShortcutSheet</div> }));
vi.mock("@/components/shared/McpApprovalModal", () => ({ default: () => <div>McpApprovalModal</div> }));
vi.mock("@/components/shared/UndoSendToast", () => ({ default: () => <div>UndoSendToast</div> }));
vi.mock("@/components/shared/Toast", () => ({ default: () => <div>Toast</div> }));
vi.mock("@/components/shared/FloatingWindowManager", () => ({ default: () => <div>FloatingWindowManager</div> }));
vi.mock("@/components/accounts/AccountSetup", () => ({ default: () => <div>AccountSetup</div> }));
vi.mock("@/hooks/useKeyboardShortcuts", () => ({ useKeyboardShortcuts: () => {} }));
vi.mock("@/lib/tauri", () => ({
  api: {
    messages: {
      sync: vi.fn(),
      syncAllInboxes: vi.fn(),
      listUnsubscribedSenders: vi.fn(() => Promise.resolve([])),
    },
  },
}));

const sampleAccount = {
  id: "acc-1",
  email: "person@example.com",
  display_name: "Personal",
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
  sort_order: 0,
  group_name: null,
  notify_enabled: true,
  track_opens_enabled: false,
  hidden_from_aggregates: false,
  triage_enabled: false,
};

describe("AppLayout sync events", () => {
  const listeners = new Map<string, (event: { payload: unknown }) => unknown>();
  const listenMock = vi.mocked(listen);
  const syncMock = vi.mocked(api.messages.sync);
  const syncAllInboxesMock = vi.mocked(api.messages.syncAllInboxes);
  const listUnsubscribedSendersMock = vi.mocked(api.messages.listUnsubscribedSenders);

  beforeEach(() => {
    listeners.clear();
    vi.clearAllMocks();

    listenMock.mockImplementation(async (event, callback) => {
      listeners.set(String(event), callback as (event: { payload: unknown }) => unknown);
      return () => {
        listeners.delete(String(event));
      };
    });
    syncMock.mockResolvedValue({ new_count: 0, updated_count: 0, account_statuses: [] });
    syncAllInboxesMock.mockResolvedValue({ new_count: 0, updated_count: 0, account_statuses: [] });
    listUnsubscribedSendersMock.mockResolvedValue([]);

    useAccountStore.setState({
      accounts: [sampleAccount],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    useMailStore.setState({
      isComposing: false,
      isSyncing: false,
      syncError: null,
      lastSyncedAt: null,
      selectedAccountId: null,
      selectedFolder: null,
      messages: [],
    });
    useWindowStore.setState({
      windows: [],
      nextZIndex: 100,
    });
  });

  async function renderLayout() {
    render(<AppLayout />);
    await waitFor(() => expect(listeners.get("idle-new-mail")).toBeDefined());
  }

  it("syncs the folder when idle-new-mail fires", async () => {
    await renderLayout();

    const handler = listeners.get("idle-new-mail");
    expect(handler).toBeDefined();

    await act(async () => {
      await handler?.({ payload: { account_id: "acc-1", folder: "INBOX" } });
    });

    expect(syncMock).toHaveBeenCalledWith("acc-1", "INBOX");
  });

  it("ignores duplicate idle-new-mail events while a sync is already in flight", async () => {
    let resolveSync: ((value: { new_count: number; updated_count: number; account_statuses: never[] }) => void) | null = null;
    syncMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveSync = resolve;
        }),
    );

    await renderLayout();
    const handler = listeners.get("idle-new-mail");
    expect(handler).toBeDefined();

    await act(async () => {
      void handler?.({ payload: { account_id: "acc-1", folder: "INBOX" } });
      void handler?.({ payload: { account_id: "acc-1", folder: "INBOX" } });
      await Promise.resolve();
    });

    expect(syncMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveSync?.({ new_count: 1, updated_count: 0, account_statuses: [] });
      await Promise.resolve();
    });
  });

  it("runs a real inbox sync when check-mail fires", async () => {
    await renderLayout();

    const handler = listeners.get("check-mail");
    expect(handler).toBeDefined();

    await act(async () => {
      await handler?.({ payload: {} });
    });

    expect(syncAllInboxesMock).toHaveBeenCalledTimes(1);
  });
});
