import { create } from "zustand";

export interface FloatingWindowState {
  id: string;
  type: "email" | "compose";
  title: string;
  position: { x: number; y: number };
  size: { w: number; h: number };
  zIndex: number;
  isMinimized: boolean;
  props: Record<string, unknown>;
  /**
   * Identity of the thing the window shows, when it has one — see
   * `draftWindowKey` / `emailWindowKey`. `openWindow` with a key that is
   * already open FOCUSES that window instead of opening a second one; that is
   * what makes clicking a draft (or an email) idempotent. Fresh composes have
   * no identity until their first autosave lands (`setWindowKey`).
   */
  key?: string;
}

/** The window editing draft `(accountId, folder, uid)`. */
export function draftWindowKey(accountId: string, folder: string, uid: number): string {
  return `compose:${accountId}|${folder}|${uid}`;
}

/** The window reading message `(accountId, folder, uid)`. */
export function emailWindowKey(accountId: string, folder: string, uid: number): string {
  return `email:${accountId}|${folder}|${uid}`;
}

interface WindowStore {
  windows: FloatingWindowState[];
  nextZIndex: number;
  openWindow: (opts: {
    type: "email" | "compose";
    title: string;
    props: Record<string, unknown>;
    position?: { x: number; y: number };
    size?: { w: number; h: number };
    key?: string;
  }) => string;
  /** Restore + raise the window with this key; returns its id, or null if none is open. */
  focusWindowByKey: (key: string) => string | null;
  /**
   * Re-key an open window. A compose window's draft moves to a new UID on every
   * autosave (edit_draft expunges and re-appends), so the key has to follow it
   * or the next click on the draft's row opens a second copy. A no-op when the
   * key is unchanged — it is called from a render-time effect and must not
   * churn the store.
   */
  setWindowKey: (id: string, key: string | null) => void;
  closeWindow: (id: string) => void;
  updatePosition: (id: string, position: { x: number; y: number }) => void;
  updateSize: (id: string, size: { w: number; h: number }) => void;
  focusWindow: (id: string) => void;
  minimizeWindow: (id: string) => void;
  restoreWindow: (id: string) => void;
}

let idCounter = 0;

export const useWindowStore = create<WindowStore>((set, get) => ({
  windows: [],
  nextZIndex: 100,

  openWindow: (opts) => {
    if (opts.key) {
      const existing = get().focusWindowByKey(opts.key);
      if (existing) return existing;
    }
    const id = `window-${++idCounter}`;
    const { nextZIndex, windows } = get();
    // Cascade new windows slightly offset from previous
    const offset = (windows.length % 5) * 30;
    const newWindow: FloatingWindowState = {
      id,
      type: opts.type,
      title: opts.title,
      position: opts.position ?? { x: 200 + offset, y: 100 + offset },
      size: opts.size ?? { w: 560, h: 480 },
      zIndex: nextZIndex,
      isMinimized: false,
      props: opts.props,
      key: opts.key,
    };
    set({ windows: [...windows, newWindow], nextZIndex: nextZIndex + 1 });
    return id;
  },

  focusWindowByKey: (key) => {
    const existing = get().windows.find((w) => w.key === key);
    if (!existing) return null;
    get().restoreWindow(existing.id);
    return existing.id;
  },

  setWindowKey: (id, key) => {
    const next = key ?? undefined;
    set((s) => {
      const target = s.windows.find((w) => w.id === id);
      // Same key (or no such window): return the SAME state object so zustand
      // skips its listeners — a fresh array here would re-render every window
      // subscriber, and the caller re-runs on those renders.
      if (!target || target.key === next) return s;
      return { windows: s.windows.map((w) => (w.id === id ? { ...w, key: next } : w)) };
    });
  },

  closeWindow: (id) => {
    set((s) => ({ windows: s.windows.filter((w) => w.id !== id) }));
  },

  updatePosition: (id, position) => {
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, position } : w)),
    }));
  },

  updateSize: (id, size) => {
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, size } : w)),
    }));
  },

  focusWindow: (id) => {
    const { nextZIndex } = get();
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, zIndex: nextZIndex } : w)),
      nextZIndex: nextZIndex + 1,
    }));
  },

  minimizeWindow: (id) => {
    set((s) => ({
      windows: s.windows.map((w) => (w.id === id ? { ...w, isMinimized: true } : w)),
    }));
  },

  restoreWindow: (id) => {
    const { nextZIndex } = get();
    set((s) => ({
      windows: s.windows.map((w) =>
        w.id === id ? { ...w, isMinimized: false, zIndex: nextZIndex } : w
      ),
      nextZIndex: nextZIndex + 1,
    }));
  },
}));
