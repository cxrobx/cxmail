import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/tauri";
import { useChatStore } from "@/stores/chatStore";
import MessageBody from "../MessageBody";
import type { MessageDetail } from "@/types/email";
import type { ChatStarted } from "@/types/chat";

vi.mock("@/hooks/useResolvedTheme", () => ({ useResolvedTheme: () => "light" }));
const detail: MessageDetail = {
  uid: 42, subject: "Subject", from_name: null, from_email: "sender@example.test",
  to_list: [], cc_list: [], bcc_list: [], date: null, plain_text: "Original email words",
  sanitized_html: "<p>Original email words</p>", attachments: [], is_read: true, is_flagged: false,
  list_unsubscribe: null, list_unsubscribe_post: null, message_id: "<id@example.test>",
  references: "<parent@example.test>", in_reply_to: null,
};
beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(api.imageTrust, "isTrusted").mockResolvedValue(false);
  useChatStore.setState({ alive: false, busy: false, starting: false, gen: null, items: [], model: "" });
});
function relay(frame: HTMLIFrameElement, text = "Original email words", source = frame.contentWindow) {
  fireEvent(window, new MessageEvent("message", {
    source, data: { type: "cxmail-selection", text, rect: { left: 10, top: 60, right: 100, bottom: 80 } },
  }));
}

it("rejects other sources, freezes on input focus, and seeds chat with the owning frame's email", async () => {
  const start = vi.spyOn(api.chat, "start").mockResolvedValue({ gen: 3 } as ChatStarted);
  const send = vi.spyOn(api.chat, "send").mockResolvedValue(undefined);
  const { container } = render(<>
    <MessageBody detail={detail} accountId="thread-account" folder="Sent" />
    <MessageBody detail={{ ...detail, uid: 77 }} accountId="selected-account" folder="INBOX" />
  </>);
  const frame = container.querySelector("iframe")!;
  relay(frame, "Spoofed", window);
  expect(screen.queryByRole("dialog")).toBeNull();
  relay(frame);
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  fireEvent.focus(screen.getByRole("textbox"));
  relay(frame, ""); // Losing native selection while entering feedback must be harmless.
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Explain this" } });
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
  await waitFor(() => expect(send).toHaveBeenCalled());
  expect(start).toHaveBeenCalledWith({ account_id: "thread-account", folder: "Sent", uid: 42 }, null);
  expect(send).toHaveBeenCalledWith(expect.stringContaining("> Original email words"), []);
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("a received email's chips ask the chat about the passage and never rewrite it", async () => {
  const start = vi.spyOn(api.chat, "start").mockResolvedValue({ gen: 4 } as ChatStarted);
  const send = vi.spyOn(api.chat, "send").mockResolvedValue(undefined);
  const tone = vi.spyOn(api.ai, "adjustTone");
  const { container } = render(<MessageBody detail={detail} accountId="frame-account" folder="Archive" />);
  relay(container.querySelector("iframe")!);
  expect(screen.queryByRole("button", { name: "Shorten" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Prove it" }));
  await waitFor(() => expect(send).toHaveBeenCalled());
  expect(start).toHaveBeenCalledWith({ account_id: "frame-account", folder: "Archive", uid: 42 }, null);
  expect(send.mock.calls[0][0]).toContain("> Original email words");
  expect(send.mock.calls[0][0]).toContain("Supported, Partially supported, Not supported, or No evidence");
  expect(tone).not.toHaveBeenCalled();
  expect(detail.sanitized_html).toBe("<p>Original email words</p>");
});

it("dismisses untouched email feedback on selection clearing and clicks inside the iframe", () => {
  const { container } = render(<MessageBody detail={detail} accountId="frame-account" folder="INBOX" />);
  const frame = container.querySelector("iframe")!;
  relay(frame); relay(frame, "");
  expect(screen.queryByRole("dialog")).toBeNull();
  relay(frame);
  fireEvent(window, new MessageEvent("message", { source: frame.contentWindow, data: { type: "cxmail-iframe-pointerdown" } }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
