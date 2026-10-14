import { api } from "@/lib/tauri";
import { useChatStore } from "../chatStore";
import type { ChatStarted } from "@/types/chat";

const seed = { account_id: "selected-account", folder: "Archive", uid: 42 };
beforeEach(() => {
  vi.restoreAllMocks();
  useChatStore.setState({ open: false, alive: false, starting: false, busy: false, gen: null, items: [], model: "", seed: null });
});

it("loads the selected email before sending the quoted passage and note", async () => {
  const start = vi.spyOn(api.chat, "start").mockResolvedValue({ gen: 5 } as ChatStarted);
  const send = vi.spyOn(api.chat, "send").mockResolvedValue(undefined);
  await useChatStore.getState().askAboutPassage(seed, "Email words\nsecond line", "What does this mean?");
  expect(start).toHaveBeenCalledWith(seed, null);
  expect(send).toHaveBeenCalledWith(expect.stringContaining("> Email words\n> second line"), []);
  expect(send.mock.calls[0][0]).toContain("My instruction:\nWhat does this mean?");
  expect(useChatStore.getState()).toMatchObject({ open: true, gen: 5, busy: true });
});

it("does not send into a fresh unseeded chat when email-context startup fails", async () => {
  vi.spyOn(api.chat, "start").mockRejectedValue(new Error("Launch failed"));
  const send = vi.spyOn(api.chat, "send");
  await expect(useChatStore.getState().askAboutPassage(seed, "Passage", "Note")).rejects.toThrow("Couldn't start Claude");
  expect(send).not.toHaveBeenCalled();
  expect(useChatStore.getState().items[0]).toMatchObject({ kind: "notice", tone: "error" });
});

it("follows up in a live chat about the same email instead of starting over", async () => {
  useChatStore.setState({ alive: true, gen: 3, seed: { ...seed }, items: [{ kind: "user", id: "u1", text: "earlier", images: [] }] });
  const start = vi.spyOn(api.chat, "start");
  const send = vi.spyOn(api.chat, "send").mockResolvedValue(undefined);
  expect(useChatStore.getState().wouldReplaceChat(seed)).toBe(false);
  await useChatStore.getState().askAboutPassage(seed, "Passage", "And this?");
  expect(start).not.toHaveBeenCalled();
  expect(send).toHaveBeenCalledTimes(1);
  expect(useChatStore.getState().items.map((i) => i.kind)).toEqual(["user", "user"]);
});

it("refuses rather than silently drops a follow-up while Claude is answering", async () => {
  useChatStore.setState({ alive: true, busy: true, gen: 3, seed: { ...seed } });
  const send = vi.spyOn(api.chat, "send");
  await expect(useChatStore.getState().askAboutPassage(seed, "Passage", "Note")).rejects.toThrow("still answering");
  expect(send).not.toHaveBeenCalled();
});

it("reports that a different email would end a conversation the user has had", () => {
  const other = { ...seed, uid: 43 };
  useChatStore.setState({ alive: true, gen: 3, seed: { ...seed }, items: [] });
  expect(useChatStore.getState().wouldReplaceChat(other)).toBe(false); // nothing said yet
  useChatStore.setState({ items: [{ kind: "user", id: "u1", text: "hi", images: [] }] });
  expect(useChatStore.getState().wouldReplaceChat(other)).toBe(true);
  useChatStore.setState({ alive: false });
  expect(useChatStore.getState().wouldReplaceChat(other)).toBe(false); // the chat already ended
});
