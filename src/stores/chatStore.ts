/**
 * The chat panel's state. The conversation lives in a `claude -p` process on
 * the Rust side (`commands::chat`); this store is the transcript and the
 * controls. Only the panel's open state and the model choice persist — a
 * transcript whose process is gone would be a history you cannot continue.
 */
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { api } from "@/lib/tauri";
import { applyChatEvent } from "@/lib/chatEvents";
import type { ChatEnvelope, ChatItem, ChatSeed, ChatStarted } from "@/types/chat";

interface ChatState {
  open: boolean;
  /** Blank = the CLI's own default model. Applies from the next new chat. */
  model: string;
  /** Generation of the live process; events from any other are stale. */
  gen: number | null;
  started: ChatStarted | null;
  sessionModel: string | null;
  items: ChatItem[];
  busy: boolean;
  alive: boolean;
  starting: boolean;

  setOpen: (open: boolean) => void;
  toggle: () => void;
  setModel: (model: string) => void;
  startNew: (seed?: ChatSeed) => Promise<boolean>;
  askAboutEmail: (seed: ChatSeed) => Promise<void>;
  send: (text: string, images?: string[]) => Promise<void>;
  answer: (requestId: string, allow: boolean, remember: boolean) => Promise<void>;
  interrupt: () => Promise<void>;
  stop: () => Promise<void>;
  continueInTerminal: () => Promise<void>;
  handleEnvelope: (env: ChatEnvelope) => void;
}

/** The process is gone, so a question it asked can no longer be answered. */
const expirePending = (items: ChatItem[]): ChatItem[] =>
  items.map((i) => (i.kind === "permission" && i.state === "pending" ? { ...i, state: "expired" as const } : i));

const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));
let noticeSeq = 0;
const notice = (text: string, tone: "info" | "error" = "error"): ChatItem => ({
  kind: "notice",
  id: `local-${++noticeSeq}`,
  text,
  tone,
});

export const useChatStore = create<ChatState>()(
  persist(
    (set, get) => ({
      open: false,
      model: "",
      gen: null,
      started: null,
      sessionModel: null,
      items: [],
      busy: false,
      alive: false,
      starting: false,

      setOpen: (open) => set({ open }),
      toggle: () => set((s) => ({ open: !s.open })),
      setModel: (model) => set({ model }),

      startNew: async (seed) => {
        set({ starting: true, items: [], busy: false, alive: false, started: null, sessionModel: null, gen: null });
        try {
          const started = await api.chat.start(seed ?? null, get().model || null);
          // `alive` goes true here, not on `ready`: the CLI emits its init only
          // once the first message arrives, and the input must be usable first.
          set({ started, gen: started.gen, alive: true, starting: false });
          return true;
        } catch (e) {
          set({ starting: false, items: [notice(`Couldn't start Claude: ${errText(e)}`)] });
          return false;
        }
      },

      askAboutEmail: async (seed) => {
        set({ open: true });
        await get().startNew(seed);
      },

      send: async (text, images = []) => {
        const trimmed = text.trim();
        if ((!trimmed && images.length === 0) || get().busy || get().starting) return;
        if (!get().alive) {
          const ok = await get().startNew();
          if (!ok) return;
        }
        set((s) => ({
          busy: true,
          items: [...s.items, { kind: "user", id: `u-${Date.now()}`, text: trimmed, images }],
        }));
        try {
          await api.chat.send(trimmed, images);
        } catch (e) {
          set((s) => ({ busy: false, items: [...s.items, notice(errText(e))] }));
        }
      },

      answer: async (requestId, allow, remember) => {
        const state = allow ? (remember ? "allowed_always" : "allowed") : "denied";
        set((s) => ({
          items: s.items.map((i) =>
            i.kind === "permission" && i.id === requestId ? { ...i, state } : i,
          ),
        }));
        try {
          await api.chat.answerPermission(requestId, allow, remember);
        } catch (e) {
          set((s) => ({
            items: [
              ...s.items.map((i) =>
                i.kind === "permission" && i.id === requestId ? { ...i, state: "expired" as const } : i,
              ),
              notice(errText(e)),
            ],
          }));
        }
      },

      interrupt: async () => {
        try {
          await api.chat.interrupt();
        } catch (e) {
          set((s) => ({ items: [...s.items, notice(errText(e))] }));
        }
      },

      stop: async () => {
        await api.chat.stop().catch(() => {});
        // `gen: null` also drops the old process's own `exited`, which is what
        // would otherwise have expired its open questions.
        set((s) => ({ alive: false, busy: false, gen: null, items: expirePending(s.items) }));
      },

      continueInTerminal: async () => {
        try {
          await api.chat.continueInTerminal();
          set((s) => ({
            alive: false,
            busy: false,
            gen: null,
            items: [...expirePending(s.items), notice("Continued in Ghostty. This chat has ended here.", "info")],
          }));
        } catch (e) {
          set((s) => ({ items: [...s.items, notice(errText(e))] }));
        }
      },

      handleEnvelope: ({ gen, event }) => {
        if (gen !== get().gen) return;
        if (event.type === "ready") set({ sessionModel: event.model });
        set((s) => {
          const next = applyChatEvent({ items: s.items, busy: s.busy, alive: s.alive }, event);
          return { items: next.items, busy: next.busy, alive: next.alive };
        });
      },
    }),
    {
      name: "cxmail-chat",
      version: 1,
      partialize: (s) => ({ open: s.open, model: s.model }),
    },
  ),
);
