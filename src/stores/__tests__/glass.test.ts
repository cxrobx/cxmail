import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  THEME_BASE_RGB,
  TRANSPARENCY_DEFAULT,
  transparencyToBlurRadius,
  type Theme,
} from "@/stores/uiStore";

/**
 * The desktop blur (`glass_macos`) and the frontend half that drives it.
 *
 * Every property here is one the window looks wrong without, not a restatement
 * of the arithmetic — and none of them is visible to a Playwright screenshot,
 * which renders a transparent window as flat black.
 */

const mocks = vi.hoisted(() => ({
  set: vi.fn((..._args: unknown[]) => Promise.resolve()),
  setRadius: vi.fn((..._args: unknown[]) => Promise.resolve()),
  reduceTransparency: vi.fn(() => Promise.resolve(false)),
  rtListener: null as null | ((event: { payload: boolean }) => void),
}));

vi.mock("@/lib/tauri", () => ({
  api: {
    glass: {
      set: mocks.set,
      setRadius: mocks.setRadius,
      reduceTransparency: mocks.reduceTransparency,
    },
    appearance: { setNative: () => Promise.resolve() },
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (event: { payload: boolean }) => void) => {
    if (name === "reduce-transparency-changed") mocks.rtListener = cb;
    return Promise.resolve(() => {});
  },
  emit: () => {},
}));

const ROOT = resolve(__dirname, "../../..");
const read = (rel: string) => readFileSync(join(ROOT, rel), "utf8");

describe("the desktop blur radius", () => {
  /**
   * The coupling is the whole reason there is one slider and not two: the more
   * desktop you let through, the more blur it takes to keep text legible on top
   * of it. A radius that fell as transparency rose would make the top of the
   * range unreadable, which is the state the dial exists to let you avoid.
   */
  it("rises with transparency, everywhere on the range", () => {
    const radii = Array.from({ length: 21 }, (_, i) => transparencyToBlurRadius(i / 20));
    for (let i = 1; i < radii.length; i += 1) {
      expect(radii[i]).toBeGreaterThanOrEqual(radii[i - 1]);
    }
    expect(radii[radii.length - 1]).toBeGreaterThan(radii[0]);
  });

  /**
   * Past roughly 48 a blurred wallpaper stops reading as one and becomes the
   * featureless smoke `NSVisualEffectView` produced; below 10 a translucent
   * window over a sharp desktop reads as a rendering fault. Out-of-range and
   * non-finite input must land inside the band, never reach the WindowServer.
   */
  it("stays inside the band where a wallpaper still reads as one", () => {
    for (const t of [-5, 0, 0.5, 1, 42, Number.NaN, Infinity, -Infinity]) {
      const r = transparencyToBlurRadius(t);
      expect(r, `t=${t}`).toBeGreaterThanOrEqual(10);
      expect(r, `t=${t}`).toBeLessThanOrEqual(48);
    }
  });

  /**
   * NaN is the real hole: `Math.min`/`Math.max` pass it straight through, and a
   * NaN radius reaches `invoke` as JSON `null`, which serde refuses for a `u8` —
   * the blur would silently vanish. It must mean what it means to the alphas
   * (`clampTransparency`): the default, so the two outputs of the dial agree.
   */
  it("treats a corrupt value as the default, exactly as the alphas do", () => {
    const r = transparencyToBlurRadius(Number.NaN);
    expect(Number.isFinite(r)).toBe(true);
    expect(r).toBe(transparencyToBlurRadius(TRANSPARENCY_DEFAULT));
  });

  /**
   * An integer, and that is load-bearing rather than tidy. `applyGlass` skips
   * the IPC when the rounded radius has not moved, which is what keeps a slider
   * drag a CSS operation — a float would change on every frame.
   */
  it("is an integer, so a full drag sends at most a few dozen calls", () => {
    const seen = new Set<number>();
    for (let i = 0; i <= 1000; i += 1) {
      const r = transparencyToBlurRadius(i / 1000);
      expect(Number.isInteger(r)).toBe(true);
      seen.add(r);
    }
    expect(seen.size).toBeLessThanOrEqual(40);
  });

  it("lands on 21 at CXMail's default transparency", () => {
    expect(transparencyToBlurRadius(TRANSPARENCY_DEFAULT)).toBe(21);
  });
});

describe("the opaque base colour", () => {
  /** Pull `--bg-primary: r g b;` out of a `[data-theme="…"]` block. */
  function bgPrimary(theme: Theme): number[] {
    const block = read("src/styles/globals.css").split(`[data-theme="${theme}"]`)[1];
    const m = block.split("}")[0].match(/--bg-primary:\s*(\d+)\s+(\d+)\s+(\d+)/);
    expect(m, `--bg-primary not found in ${theme}`).toBeTruthy();
    return m!.slice(1).map(Number);
  }

  /**
   * Glass off repaints the window in this colour. If it drifts from the token,
   * the opaque end of the slider shows a window a shade off the panes in it —
   * visible exactly at the corners and under the title bar.
   */
  it("matches --bg-primary in globals.css for both themes", () => {
    for (const theme of ["dark", "light"] as Theme[]) {
      expect([...THEME_BASE_RGB[theme]]).toEqual(bgPrimary(theme));
    }
  });

  /**
   * The launch colour is painted by `lib.rs` before any JavaScript runs, so it
   * is the third copy — and the one nobody would think to update.
   */
  it("is the colour lib.rs paints the window at launch", () => {
    const lib = read("src-tauri/src/lib.rs");
    const m = lib.match(/set_launch_background\(window,\s*(\d+),\s*(\d+),\s*(\d+)\)/);
    expect(m, "set_launch_background call not found in lib.rs").toBeTruthy();
    expect(m!.slice(1).map(Number)).toEqual([...THEME_BASE_RGB.dark]);
  });
});

describe("arming and driving the glass", () => {
  // Fresh module per test: `glassReady`, `lastSent` and the mirrored Reduce
  // Transparency flag are module state, exactly as in the app.
  async function fresh() {
    vi.resetModules();
    const mod = await import("@/stores/uiStore");
    mod.useUIStore.setState({ theme: "dark", transparency: TRANSPARENCY_DEFAULT });
    return mod;
  }
  const flush = () => new Promise((r) => setTimeout(r, 0));
  const alphaPane = () => document.documentElement.style.getPropertyValue("--alpha-pane");

  beforeEach(() => {
    mocks.set.mockClear();
    mocks.setRadius.mockClear();
    mocks.reduceTransparency.mockReset();
    mocks.reduceTransparency.mockImplementation(() => Promise.resolve(false));
    mocks.rtListener = null;
  });

  /**
   * The launch-flash guard (gotcha #59): the pre-render `applyTheme` in
   * `main.tsx` must move only the CSS. A native call there would clear the
   * window before React has painted anything into it.
   */
  it("sends nothing native before startGlass, then one full set", async () => {
    const { applyTheme, startGlass } = await fresh();
    applyTheme("dark");
    expect(mocks.set).not.toHaveBeenCalled();

    startGlass();
    await flush();
    expect(mocks.set).toHaveBeenCalledTimes(1);
    expect(mocks.set).toHaveBeenCalledWith(
      true,
      transparencyToBlurRadius(TRANSPARENCY_DEFAULT),
      THEME_BASE_RGB.dark,
    );
  });

  it("sends only radius changes during a drag, and only when the integer moves", async () => {
    const { startGlass, useUIStore } = await fresh();
    startGlass();
    await flush();
    mocks.set.mockClear();

    const radii = new Set<number>();
    for (let i = 0; i <= 200; i += 1) {
      const t = TRANSPARENCY_DEFAULT + (i / 200) * 0.5;
      useUIStore.getState().setTransparency(t);
      radii.add(transparencyToBlurRadius(t));
    }
    expect(mocks.set).not.toHaveBeenCalled();
    // One call per distinct radius after the one already sent at arm time.
    expect(mocks.setRadius).toHaveBeenCalledTimes(radii.size - 1);
  });

  it("turns glass genuinely off at zero, and back on with the theme's colour", async () => {
    const { startGlass, useUIStore } = await fresh();
    startGlass();
    await flush();

    useUIStore.getState().setTransparency(0);
    expect(mocks.set).toHaveBeenLastCalledWith(false, expect.any(Number), THEME_BASE_RGB.dark);

    useUIStore.getState().setTheme("light");
    useUIStore.getState().setTransparency(0.5);
    expect(mocks.set).toHaveBeenLastCalledWith(
      true,
      transparencyToBlurRadius(0.5),
      THEME_BASE_RGB.light,
    );
  });

  /**
   * The accessibility guarantee the material used to give for free. Pinned in
   * both halves — window AND tint — and without touching the stored slider, so
   * turning the setting off gives the user back the window they had.
   */
  it("honours Reduce Transparency at hydrate and live, without moving the slider", async () => {
    mocks.reduceTransparency.mockImplementation(() => Promise.resolve(true));
    const { startGlass, useUIStore, transparencyToAlphas } = await fresh();
    // An email dial at opaque would ask for a full veil — but with the window
    // pinned opaque there is nothing for a veil to cover.
    useUIStore.setState({ emailTransparency: 0 });
    startGlass();
    await flush();
    const veil = () => document.documentElement.style.getPropertyValue("--alpha-email-veil");
    expect(veil()).toBe("0");

    expect(mocks.set).toHaveBeenLastCalledWith(false, expect.any(Number), THEME_BASE_RGB.dark);
    expect(alphaPane()).toBe("1");
    expect(useUIStore.getState().transparency).toBe(TRANSPARENCY_DEFAULT);

    // A drag while the setting is on stays opaque and sends nothing native.
    mocks.set.mockClear();
    useUIStore.getState().setTransparency(0.8);
    expect(alphaPane()).toBe("1");
    expect(mocks.set).not.toHaveBeenCalled();
    expect(mocks.setRadius).not.toHaveBeenCalled();

    // The live flip back off restores the slider's window, not the old one.
    expect(mocks.rtListener).toBeTypeOf("function");
    mocks.rtListener!({ payload: false });
    expect(mocks.set).toHaveBeenLastCalledWith(
      true,
      transparencyToBlurRadius(0.8),
      THEME_BASE_RGB.dark,
    );
    expect(alphaPane()).toBe(String(transparencyToAlphas(0.8, "dark").pane));
    // …and the body's own dial comes back with it: opaque email over 80% glass.
    expect(veil()).toBe("1");

    // And on again, live.
    mocks.rtListener!({ payload: true });
    expect(mocks.set).toHaveBeenLastCalledWith(false, expect.any(Number), THEME_BASE_RGB.dark);
    expect(alphaPane()).toBe("1");
    expect(useUIStore.getState().transparency).toBe(0.8);
  });

  it("arms anyway when the hydrate read fails, treating it as off", async () => {
    mocks.reduceTransparency.mockImplementation(() => Promise.reject(new Error("no bridge")));
    const { startGlass } = await fresh();
    startGlass();
    await flush();
    expect(mocks.set).toHaveBeenCalledWith(true, expect.any(Number), THEME_BASE_RGB.dark);
  });
});
