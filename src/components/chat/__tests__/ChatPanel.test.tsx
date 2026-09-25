/**
 * The chat panel's contract with the Rust side: the first message starts a
 * session before it is sent, events from a dead session are ignored, and a
 * permission card's buttons send exactly the answer they name — the Allow /
 * Deny click IS the gate on every mutation the chat can make.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ChatPanel from "@/components/chat/ChatPanel";
import { useChatStore } from "@/stores/chatStore";
import { api } from "@/lib/tauri";
import { openDraftForEdit } from "@/lib/draftCompose";
import type { ChatEvent } from "@/types/chat";

const handlers = vi.hoisted(() => ({} as Record<string, (e: { payload: unknown }) => void>));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  }),
}));
vi.mock("@/lib/tauri", () => ({
  api: {
    chat: {
      start: vi.fn(),
      send: vi.fn(),
      answerPermission: vi.fn(),
      interrupt: vi.fn(),
      stop: vi.fn(),
      continueInTerminal: vi.fn(),
    },
  },
}));
vi.mock("@/lib/draftCompose", () => ({ openDraftForEdit: vi.fn() }));
vi.mock("@/stores/accountStore", () => ({
  useAccountStore: (selector: (s: unknown) => unknown) =>
    selector({ accounts: [{ id: "acct-1", email: "me@example.com" }] }),
}));
vi.mock("@/stores/uiStore", () => ({
  useUIStore: (selector: (s: unknown) => unknown) => selector({ addToast: vi.fn() }),
}));

const start = api.chat.start as ReturnType<typeof vi.fn>;
const send = api.chat.send as ReturnType<typeof vi.fn>;
const answerPermission = api.chat.answerPermission as ReturnType<typeof vi.fn>;

const STARTED = {
  gen: 7,
  cwd: "/tmp/claude-chat",
  repo: null,
  missing_repo_path: null,
  readable_repos: ["/Users/x/Projects/northwind"],
  mcp_servers: ["cxmail"],
  seed_subject: null,
};

function fire(event: ChatEvent, gen = 7) {
  act(() => handlers["chat-event"]?.({ payload: { gen, event } }));
}

async function openAndSend(text: string) {
  const user = userEvent.setup();
  render(<ChatPanel />);
  act(() => useChatStore.getState().setOpen(true));
  await user.type(screen.getByPlaceholderText(/Ask Claude about your mail/), `${text}{Enter}`);
  return user;
}

beforeEach(() => {
  vi.clearAllMocks();
  start.mockResolvedValue(STARTED);
  send.mockResolvedValue(undefined);
  answerPermission.mockResolvedValue(undefined);
  useChatStore.setState({
    open: false,
    model: "",
    gen: null,
    started: null,
    sessionModel: null,
    items: [],
    busy: false,
    alive: false,
    starting: false,
  });
});

describe("ChatPanel", () => {
  it("renders nothing while closed, but still listens", () => {
    const { container } = render(<ChatPanel />);
    expect(container.querySelector("aside")).toBeNull();
    expect(handlers["chat-event"]).toBeTypeOf("function");
  });

  it("starts a session before sending the first message", async () => {
    await openAndSend("follow up with Dana");
    expect(start).toHaveBeenCalledWith(null, null);
    expect(send).toHaveBeenCalledWith("follow up with Dana");
    expect(start.mock.invocationCallOrder[0]).toBeLessThan(send.mock.invocationCallOrder[0]);
    expect(screen.getByText("follow up with Dana")).toBeTruthy();
  });

  it("streams a reply and ignores events from any other session", async () => {
    await openAndSend("hi");
    fire({ type: "text_delta", text: "Hel" });
    fire({ type: "text_delta", text: "lo" });
    fire({ type: "assistant_text", text: "Hello, **Dana**." });
    // A notice would persist if it got in — a stale text delta would not, since
    // the complete block overwrites it, so it could not prove the filter.
    fire({ type: "notice", text: "STALE from the previous chat" }, 6);
    expect(screen.getByText("Dana").tagName).toBe("STRONG");
    expect(screen.queryByText(/STALE/)).toBeNull();
  });

  it("Allow sends allow, once, and the card stops offering buttons", async () => {
    const user = await openAndSend("archive it");
    fire({
      type: "permission_request",
      request_id: "req-1",
      tool_name: "mcp__cxmail__archive_email",
      display_name: "Archive Email",
      input: { account_id: "6ed99f49-d35e", folder: "INBOX", uid: 9 },
      context: {
        account: "me@example.com",
        folder: "INBOX",
        messages: [{ subject: "Re: scope for Q4", from: "Dana Reyes" }],
        message_count: 1,
      },
    });
    // What a person checks: the question, the email, the sender, the account —
    // not the raw id, which only appears under Details.
    expect(screen.getByText("Archive this email?")).toBeTruthy();
    expect(screen.getByText("Re: scope for Q4")).toBeTruthy();
    expect(screen.getByText("from Dana Reyes")).toBeTruthy();
    expect(screen.getByText("in me@example.com · Inbox")).toBeTruthy();
    expect(screen.queryByText(/^account_id/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Allow" }));
    expect(answerPermission).toHaveBeenCalledWith("req-1", true, false);
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.getByText("Allowed")).toBeTruthy();
  });

  it("Deny sends deny, and 'Allow for this chat' remembers", async () => {
    const user = await openAndSend("clean up");
    fire({ type: "permission_request", request_id: "r-deny", tool_name: "mcp__cxmail__delete_email", display_name: "Delete Email", input: {}, context: null });
    await user.click(screen.getByRole("button", { name: "Deny" }));
    expect(answerPermission).toHaveBeenLastCalledWith("r-deny", false, false);
    fire({ type: "permission_request", request_id: "r-keep", tool_name: "mcp__cxmail__flag_email", display_name: "Flag Email", input: {}, context: null });
    await user.click(screen.getByRole("button", { name: "Allow for this chat" }));
    expect(answerPermission).toHaveBeenLastCalledWith("r-keep", true, true);
  });

  it("a finished draft opens in compose on its own account", async () => {
    const user = await openAndSend("write it");
    fire({ type: "tool_use", id: "t1", name: "mcp__cxmail__compose_draft", input: { subject: "Re: scope" } });
    fire({
      type: "tool_result",
      tool_use_id: "t1",
      is_error: false,
      text: "Draft saved to Drafts (UID 812) for me@example.com",
      draft: { account_id: null, account_email: "me@example.com", folder: "Drafts", uid: 812 },
    });
    await user.click(screen.getByRole("button", { name: /Open draft/ }));
    expect(openDraftForEdit).toHaveBeenCalledWith("acct-1", "Drafts", 812);
  });

  it("a failed start says so in the panel instead of failing silently", async () => {
    start.mockRejectedValueOnce(new Error("Claude Code CLI (`claude`) was not found."));
    await openAndSend("hello");
    expect(send).not.toHaveBeenCalled();
    expect(screen.getByText(/Couldn't start Claude: .*not found/)).toBeTruthy();
  });
});
