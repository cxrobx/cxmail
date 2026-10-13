import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/tauri";
import { HUMANIZE_INSTRUCTION, SOFTEN_TONE } from "@/lib/selectionFeedback";
import SelectionFeedback, { ComposerSelectionFeedback } from "../SelectionFeedback";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));
const selection = { text: "selected passage", anchor: { left: 200, top: 200, right: 320, bottom: 220 } };
beforeEach(() => { vi.restoreAllMocks(); vi.mocked(writeText).mockClear(); });

it("waits for an editor view and cleans up safely after that view unmounts", async () => {
  const editor = new Editor({ element: null, extensions: [StarterKit], content: "<p>Draft</p>" });
  const { unmount } = render(<ComposerSelectionFeedback editor={editor} />);
  act(() => { editor.mount(document.createElement("div")); });
  await waitFor(() => expect(editor.isInitialized).toBe(true));
  act(() => { editor.unmount(); });
  expect(() => unmount()).not.toThrow();
  editor.destroy();
});

it.each([
  ["Shorten", "concise"], ["Soften", SOFTEN_TONE],
])("%s uses the existing tone command and applies the saved draft selection", async (label, tone) => {
  const call = vi.spyOn(api.ai, "adjustTone").mockResolvedValue("rewritten");
  const apply = vi.fn().mockReturnValue(true), close = vi.fn();
  render(<SelectionFeedback mode="editable" selection={selection} onApply={apply} onClose={close} />);
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Give Claude feedback on selection" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: label }));
  await waitFor(() => expect(apply).toHaveBeenCalledWith("rewritten"));
  expect(call).toHaveBeenCalledWith(selection.text, tone);
  expect(close).toHaveBeenCalled();
});

it("humanizes read-only text in the bubble with Copy and Use in reply", async () => {
  const call = vi.spyOn(api.ai, "rewriteText").mockResolvedValue("Natural version");
  const reply = vi.fn();
  render(<SelectionFeedback mode="readonly" selection={selection} onClose={vi.fn()} onAsk={vi.fn()} onUseInReply={reply} />);
  fireEvent.click(screen.getByRole("button", { name: "Humanize" }));
  expect(await screen.findByText("Natural version")).toBeInTheDocument();
  expect(call).toHaveBeenCalledWith(selection.text, HUMANIZE_INSTRUCTION);
  fireEvent.click(screen.getByRole("button", { name: "Copy" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith("Natural version"));
  fireEvent.click(screen.getByRole("button", { name: "Use in reply" }));
  expect(reply).toHaveBeenCalledWith("Natural version");
});

it("routes free text to chat for mail and to rewriting for drafts; composition Enter does not send", async () => {
  const ask = vi.fn().mockResolvedValue(undefined), close = vi.fn();
  const rewrite = vi.spyOn(api.ai, "rewriteText").mockResolvedValue("Changed");
  const { unmount } = render(<SelectionFeedback mode="readonly" selection={selection} onClose={close} onAsk={ask} onUseInReply={vi.fn()} />);
  const input = screen.getByRole("textbox");
  fireEvent.change(input, { target: { value: "Explain this" } });
  fireEvent.keyDown(input, { key: "Enter", isComposing: true });
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  expect(ask).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(ask).toHaveBeenCalledWith(selection.text, "Explain this"));
  expect(rewrite).not.toHaveBeenCalled();
  unmount();
  const apply = vi.fn().mockReturnValue(true);
  render(<SelectionFeedback mode="editable" selection={selection} onClose={close} onApply={apply} />);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "More direct" } });
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
  await waitFor(() => expect(apply).toHaveBeenCalledWith("Changed"));
  expect(rewrite).toHaveBeenCalledWith(selection.text, "More direct");
});

it("shows a copyable fallback when the saved draft range changed", async () => {
  vi.spyOn(api.ai, "adjustTone").mockResolvedValue("Fallback");
  const close = vi.fn();
  render(<SelectionFeedback mode="editable" selection={selection} onClose={close} onApply={() => false} />);
  fireEvent.click(screen.getByRole("button", { name: "Shorten" }));
  expect(await screen.findByText(/Your draft changed/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
  expect(close).not.toHaveBeenCalled();
});

it("keeps errors visible and does not apply a result after dismissal", async () => {
  vi.spyOn(api.ai, "adjustTone").mockRejectedValueOnce(new Error("Offline"));
  const apply = vi.fn();
  const { unmount } = render(<SelectionFeedback mode="editable" selection={selection} onClose={vi.fn()} onApply={apply} />);
  fireEvent.click(screen.getByRole("button", { name: "Shorten" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Offline");
  let resolve!: (text: string) => void;
  vi.spyOn(api.ai, "adjustTone").mockReturnValue(new Promise((done) => { resolve = done; }));
  fireEvent.click(screen.getByRole("button", { name: "Shorten" }));
  unmount();
  await act(async () => { resolve("Late result"); });
  expect(apply).not.toHaveBeenCalled();
});

it("closes on Escape and outside clicks while chips preserve selection on mousedown", () => {
  const close = vi.fn();
  render(<SelectionFeedback mode="editable" selection={selection} onClose={close} onApply={vi.fn()} />);
  expect(fireEvent.mouseDown(screen.getByRole("button", { name: "Shorten" }))).toBe(false);
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
  expect(close).toHaveBeenCalledTimes(1);
  fireEvent.pointerDown(document.body);
  expect(close).toHaveBeenCalledTimes(2);
});

it("shows all controls immediately without moving focus or starting inference", () => {
  const previous = document.createElement("button");
  document.body.append(previous);
  previous.focus();
  const call = vi.spyOn(api.ai, "adjustTone");
  const interact = vi.fn();
  render(<SelectionFeedback mode="editable" selection={selection} onClose={vi.fn()} onApply={vi.fn()} onInteract={interact} />);
  expect(document.activeElement).toBe(previous);
  expect(screen.getByRole("button", { name: "Shorten" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Soften" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Humanize" })).toBeInTheDocument();
  expect(screen.getByRole("textbox")).toBeInTheDocument();
  expect(call).not.toHaveBeenCalled();
  expect(interact).not.toHaveBeenCalled();
  fireEvent.focus(screen.getByRole("textbox"));
  expect(interact).toHaveBeenCalled();
  previous.remove();
});
