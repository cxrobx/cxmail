import { api } from "@/lib/tauri";
import { useChatStore } from "../chatStore";
import type { ChatStarted } from "@/types/chat";

const seed = { account_id: "selected-account", folder: "Archive", uid: 42 };
beforeEach(() => {
  vi.restoreAllMocks();
  useChatStore.setState({ open: false, alive: false, starting: false, busy: false, gen: null, items: [], model: "" });
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
