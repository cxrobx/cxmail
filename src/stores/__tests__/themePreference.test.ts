import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { resolveTheme, applyTheme, useUIStore, transparencyToAlphas } from "@/stores/uiStore";

/**
 * The `system` theme preference, and the two ways it goes wrong quietly.
 *
 * `system` is a deferral, not a palette: it has no `[data-theme]` block, no
 * pane floors and nothing to pin. Every one of those lookups is keyed by the
 * RESOLVED theme, and each fails silently if a preference reaches it — a
 * missing floor makes `transparencyToAlphas` return NaN, and a NaN alpha paints
 * the pane fully transparent, i.e. the window vanishes and leaves text floating
 * over the desktop. That is the same failure `clampTransparency` exists to
 * prevent, arriving by a different door.
 */

type Listener = () => void;

/** A `matchMedia` jsdom does not have, with a handle to fire a system flip. */
function stubMatchMedia(dark: boolean) {
  const listeners = new Set<Listener>();
  const mql = {
    matches: dark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_: string, l: Listener) => void listeners.add(l),
    removeEventListener: (_: string, l: Listener) => void listeners.delete(l),
  };
  vi.stubGlobal("matchMedia", () => mql);
  return {
    flip(next: boolean) {
      mql.matches = next;
      listeners.forEach((l) => l());
    },
  };
}

describe("resolveTheme", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("passes an explicit choice straight through", () => {
    stubMatchMedia(true);
    expect(resolveTheme("light")).toBe("light");
    expect(resolveTheme("dark")).toBe("dark");
  });

  it("reads the system appearance for `system`, both ways", () => {
    const media = stubMatchMedia(true);
    expect(resolveTheme("system")).toBe("dark");
    media.flip(false);
    expect(resolveTheme("system")).toBe("light");
  });

  it("falls back to dark where there is no matchMedia to ask", () => {
    // jsdom does not implement it, so this is the state every other test file
    // imports this module in — an unguarded read here would throw at import
    // time and take the whole suite down, not just this case.
    expect(typeof window.matchMedia).not.toBe("function");
    expect(resolveTheme("system")).toBe("dark");
  });
});

describe("applyTheme under the system preference", () => {
  beforeEach(() => useUIStore.setState({ theme: "dark", transparency: 0.5 }));
  afterEach(() => vi.unstubAllGlobals());

  it("paints the resolved theme, never the preference", () => {
    stubMatchMedia(false);
    applyTheme("system");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("uses the resolved theme's pane floors instead of producing a NaN alpha", () => {
    stubMatchMedia(false);
    applyTheme("system");
    const alphas = ["--alpha-pane", "--alpha-sidebar", "--alpha-surface"].map((v) =>
      Number(document.documentElement.style.getPropertyValue(v)),
    );
    for (const a of alphas) {
      expect(Number.isFinite(a)).toBe(true);
      expect(a).toBeGreaterThan(0);
    }
    // …and specifically LIGHT's floors, which sit well above dark's. Resolving
    // to the wrong side is legible rather than broken, so only a value check
    // catches it.
    expect(alphas[0]).toBeCloseTo(transparencyToAlphas(0.5, "light").pane, 10);
    expect(alphas[0]).not.toBeCloseTo(transparencyToAlphas(0.5, "dark").pane, 10);
  });
});

describe("following the system live", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  /**
   * The listener is registered at module load, so the stub has to be in place
   * BEFORE the import — hence `resetModules` + a dynamic import rather than the
   * top-of-file one. In the app that ordering is free: the webview always has
   * `matchMedia`.
   */
  async function freshStore(dark: boolean) {
    vi.resetModules();
    const media = stubMatchMedia(dark);
    const mod = await import("@/stores/uiStore");
    return { media, ...mod };
  }

  it("repaints on a system flip while the preference is `system`", async () => {
    const { media, useUIStore: store, applyTheme: apply } = await freshStore(true);
    store.setState({ theme: "system", transparency: 0.3 });
    apply("system");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    media.flip(false);
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("ignores a system flip once a theme is pinned", async () => {
    const { media, useUIStore: store, applyTheme: apply } = await freshStore(true);
    store.setState({ theme: "dark", transparency: 0.3 });
    apply("dark");

    media.flip(false);
    // Pinned means pinned — a light system must not drag a chosen dark window
    // with it. (In the app the pin also stops `prefers-color-scheme` from ever
    // reporting the change; this guard is what holds if that ever stops being
    // true.)
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });
});
