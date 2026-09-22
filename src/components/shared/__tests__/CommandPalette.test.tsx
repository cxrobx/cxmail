import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import CommandPalette from "@/components/shared/CommandPalette";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import type { MessageSummary, SearchResult } from "@/types/email";

vi.mock("@/lib/tauri", () => ({
  api: {
    system: {
      isDefaultMailClient: vi.fn(() => Promise.resolve(false)),
      setAsDefaultMailClient: vi.fn(() => Promise.resolve(true)),
    },
    messages: {
      search: vi.fn(() => Promise.resolve([])),
      aiSearch: vi.fn(() =>
        Promise.resolve({ results: [], applied_filters: [], ai_failed: false }),
      ),
      serverSearch: vi.fn(() => Promise.resolve([])),
      archive: vi.fn(() => Promise.resolve()),
      delete: vi.fn(() => Promise.resolve()),
      togglePin: vi.fn(() => Promise.resolve()),
      toggleStar: vi.fn(() => Promise.resolve()),
      toggleMute: vi.fn(() => Promise.resolve()),
      markRead: vi.fn(() => Promise.resolve()),
      markUnread: vi.fn(() => Promise.resolve()),
      forceFullSync: vi.fn(() =>
        Promise.resolve({ new_count: 0, updated_count: 0, account_statuses: [] }),
      ),
      threadUidsInFolder: vi.fn(() => Promise.resolve([])),
    },
    snooze: { snooze: vi.fn(() => Promise.resolve()) },
    claude: { openEmail: vi.fn(() => Promise.resolve()) },
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

const sampleMessage = (overrides: Partial<MessageSummary> = {}): MessageSummary => ({
  uid: 42,
  account_id: "acc-1",
  folder_name: "INBOX",
  subject: "Quarterly report",
  from_name: "Ann",
  from_email: "ann@example.com",
  date: "2026-07-20T00:00:00Z",
  snippet: null,
  is_read: false,
  is_flagged: false,
  has_attachments: false,
  size_bytes: 100,
  category: "primary",
  is_muted: false,
  is_pinned: false,
  thread_count: 1,
  thread_draft_count: 0,
  thread_root_id: null,
  thread_has_unread: false,
  ...overrides,
});

const hitResult = (subject: string, uid = 99): SearchResult => ({
  account_id: "acc-1",
  folder_name: "INBOX",
  uid,
  subject,
  from_name: "Sender",
  from_email: "sender@example.com",
  date: "2026-07-20T00:00:00Z",
  snippet: "",
  is_read: false,
  has_attachments: false,
  score: 1,
});

const onClose = vi.fn();

async function renderPalette() {
  render(<CommandPalette open onClose={onClose} />);
  // Flush the open-effect's isDefaultMailClient promise.
  await act(async () => {});
}

function paletteInput(): HTMLInputElement {
  return screen.getByPlaceholderText("Type a command...");
}

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});

beforeEach(() => {
  vi.clearAllMocks();
  useAccountStore.setState({ accounts: [sampleAccount] });
  useMailStore.setState({
    selectedMessageUid: null,
    selectedMessageAccountId: null,
    selectedMessageFolder: null,
    selectedAccountId: null,
    selectedFolder: null,
    specialView: null,
    messages: [],
    inboxGroups: [],
    foldersByAccount: {},
    searchQuery: null,
    searchResults: [],
  });
  useUIStore.setState({ recentCommandIds: [], recentSearches: [], toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Message group", () => {
  it("is absent when no message is selected", async () => {
    await renderPalette();
    expect(screen.queryByText("Reply")).toBeNull();
    expect(screen.queryByText("Message")).toBeNull();
  });

  it("renders selection-aware actions with read-state-dependent labels", async () => {
    useMailStore.setState({
      selectedMessageUid: 42,
      selectedMessageAccountId: "acc-1",
      messages: [sampleMessage({ is_read: false })],
    });
    await renderPalette();
    expect(screen.getByText("Reply")).toBeInTheDocument();
    expect(screen.getByText("Archive")).toBeInTheDocument();
    expect(screen.getByText("Delete")).toBeInTheDocument();
    expect(screen.getByText("Pin")).toBeInTheDocument();
    expect(screen.getByText("Star")).toBeInTheDocument();
    expect(screen.getByText("Mark as Read")).toBeInTheDocument();
    expect(screen.getByText("Snooze: Later Today")).toBeInTheDocument();
    expect(screen.getByText("Open in Claude")).toBeInTheDocument();
  });

  it("flips label-dependent commands for a read/pinned/starred message", async () => {
    useMailStore.setState({
      selectedMessageUid: 42,
      selectedMessageAccountId: "acc-1",
      messages: [sampleMessage({ is_read: true, is_pinned: true, is_flagged: true })],
    });
    await renderPalette();
    expect(screen.getByText("Mark as Unread")).toBeInTheDocument();
    expect(screen.getByText("Unpin")).toBeInTheDocument();
    expect(screen.getByText("Unstar")).toBeInTheDocument();
  });

  it("omits label-dependent commands when the row is not in the current list", async () => {
    useMailStore.setState({
      selectedMessageUid: 42,
      selectedMessageAccountId: "acc-1",
      messages: [], // selection made from e.g. a search hit
    });
    await renderPalette();
    expect(screen.getByText("Archive")).toBeInTheDocument();
    expect(screen.queryByText(/^(Pin|Unpin)$/)).toBeNull();
    expect(screen.queryByText(/^Mark as/)).toBeNull();
  });
});

describe("search fallthrough", () => {
  it("always renders 'Search mail for' on any query, even gibberish", async () => {
    await renderPalette();
    fireEvent.change(paletteInput(), { target: { value: "zzxqvw" } });
    expect(screen.getByText('Search mail for "zzxqvw"')).toBeInTheDocument();
  });
});

describe("ranking", () => {
  it("puts the best label match first", async () => {
    await renderPalette();
    fireEvent.change(paletteInput(), { target: { value: "snoozed" } });
    const first = document.querySelector('[data-index="0"]');
    expect(first?.textContent).toContain("Snoozed");
  });
});

describe("category commands", () => {
  it("renders in normal views but is gated off inside special views", async () => {
    await renderPalette();
    expect(screen.getByText("Category: Primary")).toBeInTheDocument();

    act(() => {
      useMailStore.setState({ specialView: "snoozed" });
    });
    expect(screen.queryByText("Category: Primary")).toBeNull();
  });
});

describe("recents", () => {
  it("drops stale ids silently and clones live ones into Recent", async () => {
    useUIStore.setState({
      recentCommandIds: ["folder-acc-1-DELETED", "toggle-sidebar"],
    });
    await renderPalette();
    expect(screen.getByText("Recent")).toBeInTheDocument();
    // Recent clone + the View original both render.
    expect(screen.getAllByText("Toggle Sidebar")).toHaveLength(2);
    expect(screen.queryByText(/DELETED/)).toBeNull();
  });

  it("records run commands but never synthetic entries", async () => {
    await renderPalette();
    fireEvent.change(paletteInput(), { target: { value: "zzxqvw" } });
    fireEvent.click(screen.getByText('Search mail for "zzxqvw"'));
    expect(useUIStore.getState().recentCommandIds).toEqual([]);

    fireEvent.change(paletteInput(), { target: { value: "" } });
    fireEvent.click(screen.getByText("Toggle Sidebar"));
    expect(useUIStore.getState().recentCommandIds).toEqual(["toggle-sidebar"]);
  });
});

describe("keyboard navigation", () => {
  it("ArrowDown + Enter runs the highlighted command", async () => {
    await renderPalette();
    // Index 0 = New Message, index 1 = Snoozed (special views follow compose).
    fireEvent.keyDown(paletteInput(), { key: "ArrowDown" });
    fireEvent.keyDown(paletteInput(), { key: "Enter" });
    expect(useMailStore.getState().specialView).toBe("snoozed");
    expect(onClose).toHaveBeenCalled();
    expect(useUIStore.getState().recentCommandIds).toEqual(["view-snoozed"]);
  });
});

describe("inline hits", () => {
  it("debounces the FTS call and renders hits with sender sublabels", async () => {
    vi.useFakeTimers();
    const searchMock = vi.mocked(api.messages.search);
    searchMock.mockResolvedValue([hitResult("Budget review")]);
    await renderPalette();
    fireEvent.change(paletteInput(), { target: { value: "budget" } });
    expect(searchMock).not.toHaveBeenCalled();
    await act(async () => {
      vi.advanceTimersByTime(199);
    });
    expect(searchMock).not.toHaveBeenCalled();
    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    expect(searchMock).toHaveBeenCalledWith("budget", { prefix: true, limit: 5 });
    expect(screen.getByText("Budget review")).toBeInTheDocument();
    expect(screen.getByText("Sender")).toBeInTheDocument();
  });

  it("never renders a stale response after the query changes", async () => {
    vi.useFakeTimers();
    const searchMock = vi.mocked(api.messages.search);
    let resolveStale!: (v: SearchResult[]) => void;
    searchMock.mockImplementationOnce(
      () => new Promise<SearchResult[]>((resolve) => { resolveStale = resolve; }),
    );
    searchMock.mockResolvedValueOnce([hitResult("FRESH", 7)]);
    await renderPalette();

    fireEvent.change(paletteInput(), { target: { value: "stale query" } });
    await act(async () => {
      vi.advanceTimersByTime(200); // stale request in flight
    });
    fireEvent.change(paletteInput(), { target: { value: "fresh query" } });
    await act(async () => {
      vi.advanceTimersByTime(200); // fresh request resolves
    });
    await act(async () => {
      resolveStale([hitResult("STALE", 8)]); // stale finally lands — must be dropped
    });

    expect(screen.queryByText("STALE")).toBeNull();
    expect(screen.getByText("FRESH")).toBeInTheDocument();
  });
});
