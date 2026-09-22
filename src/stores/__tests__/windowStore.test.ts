import { describe, it, expect, beforeEach } from "vitest";
import { useWindowStore, draftWindowKey, emailWindowKey } from "@/stores/windowStore";

beforeEach(() => {
  useWindowStore.setState({ windows: [], nextZIndex: 100 });
});

const windows = () => useWindowStore.getState().windows;
const open = (key?: string, type: "compose" | "email" = "compose") =>
  useWindowStore.getState().openWindow({ type, title: "t", props: {}, key });
const topZ = () => Math.max(...windows().map((w) => w.zIndex));

describe("openWindow identity", () => {
  // THE regression (2026-08-25): a draft that was slow to open got clicked
  // five times and five compose windows opened. A window that names what it
  // shows must be opened at most once.
  it("opening the same key again returns the open window instead of stacking another", () => {
    const key = draftWindowKey("acct", "Drafts", 619);
    const first = open(key);
    expect(open(key)).toBe(first);
    expect(open(key)).toBe(first);
    expect(windows()).toHaveLength(1);
  });

  it("a re-open raises the window and un-minimizes it — the user asked for it, so show it", () => {
    const key = draftWindowKey("acct", "Drafts", 619);
    const id = open(key);
    open(); // something else on top
    useWindowStore.getState().minimizeWindow(id);
    expect(windows().find((w) => w.id === id)?.isMinimized).toBe(true);

    expect(open(key)).toBe(id);
    const w = windows().find((x) => x.id === id)!;
    expect(w.isMinimized).toBe(false);
    expect(w.zIndex).toBe(topZ());
  });

  it("different identities are different windows, and no identity is always a new window", () => {
    const a = open(draftWindowKey("acct", "Drafts", 1));
    const b = open(draftWindowKey("acct", "Drafts", 2));
    const c = open(emailWindowKey("acct", "INBOX", 1));
    const d = open();
    const e = open();
    expect(new Set([a, b, c, d, e]).size).toBe(5);
    expect(windows()).toHaveLength(5);
  });

  it("reading a message and editing a draft at the same triple are two windows", () => {
    expect(draftWindowKey("a", "Drafts", 7)).not.toBe(emailWindowKey("a", "Drafts", 7));
  });

  it("closing the window frees its key", () => {
    const key = draftWindowKey("acct", "Drafts", 619);
    const id = open(key);
    useWindowStore.getState().closeWindow(id);
    expect(open(key)).not.toBe(id);
    expect(windows()).toHaveLength(1);
  });
});

describe("setWindowKey — the key follows the draft", () => {
  it("after autosave moves the draft to a new UID, the new UID focuses the window and the old one opens fresh", () => {
    const before = draftWindowKey("acct", "Drafts", 619);
    const after = draftWindowKey("acct", "Drafts", 620);
    const id = open(before);
    useWindowStore.getState().setWindowKey(id, after);

    expect(open(after)).toBe(id);
    expect(windows()).toHaveLength(1);
    // The expunged UID is nobody's identity any more.
    expect(open(before)).not.toBe(id);
    expect(windows()).toHaveLength(2);
  });

  it("a fresh compose gains an identity on its first save", () => {
    const id = open();
    const key = draftWindowKey("acct", "Drafts", 1);
    useWindowStore.getState().setWindowKey(id, key);
    expect(useWindowStore.getState().focusWindowByKey(key)).toBe(id);
  });

  it("re-keying to the key it already has leaves the state object untouched (no listener churn)", () => {
    // ComposeModal reports its draft ref from an effect that runs on the window
    // manager's renders. A store write on every one of those would re-render
    // the manager, which re-runs the effect: a loop. Same key → same state.
    const key = draftWindowKey("acct", "Drafts", 619);
    const keyed = open(key);
    const bare = open();
    const before = useWindowStore.getState();
    useWindowStore.getState().setWindowKey(keyed, key);
    useWindowStore.getState().setWindowKey(bare, null);
    useWindowStore.getState().setWindowKey("window-does-not-exist", key);
    expect(useWindowStore.getState()).toBe(before);
  });

  it("focusWindowByKey answers null when nothing is open under that key", () => {
    expect(useWindowStore.getState().focusWindowByKey(draftWindowKey("a", "Drafts", 1))).toBeNull();
  });
});
