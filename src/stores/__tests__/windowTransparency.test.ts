import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { transparencyToAlphas, TRANSPARENCY_DEFAULT, type Theme } from "@/stores/uiStore";

/**
 * The window-transparency curve, pinned against the palette it was derived from.
 *
 * The formula is three relationships, not three numbers, and each of them fails
 * *invisibly* when broken — the window still renders, it just stops reading as a
 * window. The expensive one is the sidebar: cxtasks' curve (a ×1.9 boost on the
 * slider plus the bar's own low floor) is correct for cxtasks' palette and
 * inverts CXMail's bar into a trench, because CXMail's bar is only 1.25× lighter
 * than its content where cxtasks' is 1.75×. See `SIDEBAR_LEAD`.
 *
 * So the palette is read out of `globals.css` rather than duplicated here. If
 * someone moves `--bg-sidebar` closer to `--bg-primary`, that is exactly when
 * the lead constant needs revisiting, and this test is what says so.
 */

const CSS = readFileSync(
  join(resolve(__dirname, "../../.."), "src/styles/globals.css"),
  "utf8",
);

/** Pull an `--bg-*: r g b;` triplet out of a `[data-theme="…"]` block. */
function paletteChannel(theme: Theme, token: string): number {
  const block = CSS.split(`[data-theme="${theme}"]`)[1];
  expect(block, `no [data-theme="${theme}"] block`).toBeTruthy();
  const m = block.split("}")[0].match(new RegExp(`--${token}:\\s*(\\d+)\\s+(\\d+)\\s+(\\d+)`));
  expect(m, `--${token} not found in ${theme}`).toBeTruthy();
  // Relative luminance is not needed — every CXMail background is a near-neutral
  // warm grey, so the red channel orders them identically and keeps the
  // arithmetic legible.
  return Number(m![1]);
}

/** What a translucent surface actually composites to over a given backdrop. */
const over = (colour: number, alpha: number, backdrop: number) =>
  colour * alpha + backdrop * (1 - alpha);

const THEMES: Theme[] = ["dark", "light"];
/** Black and white wallpapers — the two extremes any real one sits between. */
const BACKDROPS = [0, 255];
const POSITIONS = [0, 0.1, 0.25, TRANSPARENCY_DEFAULT, 0.5, 0.75, 0.9, 1];

describe("transparencyToAlphas", () => {
  it("is genuinely opaque at zero — all three, both themes", () => {
    for (const theme of THEMES) {
      const { pane, sidebar, surface } = transparencyToAlphas(0, theme);
      expect(pane).toBe(1);
      expect(sidebar).toBe(1);
      expect(surface).toBe(1);
    }
  });

  it("clamps out-of-range input instead of producing a NaN or >1 alpha", () => {
    for (const theme of THEMES) {
      expect(transparencyToAlphas(-1, theme)).toEqual(transparencyToAlphas(0, theme));
      expect(transparencyToAlphas(2, theme)).toEqual(transparencyToAlphas(1, theme));
      for (const a of Object.values(transparencyToAlphas(Number.NaN, theme))) {
        // NaN propagates through `Math.min`/`Math.max`, and a NaN alpha paints
        // the pane fully transparent — the window would vanish.
        expect(Number.isFinite(a)).toBe(true);
      }
    }
  });

  it("keeps the sidebar thinner than the content and the surface denser", () => {
    for (const theme of THEMES) {
      for (const t of POSITIONS) {
        const { pane, sidebar, surface } = transparencyToAlphas(t, theme);
        expect(sidebar).toBeLessThanOrEqual(pane);
        expect(surface).toBeGreaterThanOrEqual(pane);
        for (const a of [pane, sidebar, surface]) {
          expect(a).toBeGreaterThan(0);
          expect(a).toBeLessThanOrEqual(1);
        }
      }
    }
  });

  /**
   * The one that catches a copied-from-cxtasks curve.
   *
   * A thinner bar is only chrome-over-content while it still composites LIGHTER
   * than the content it sits on (dark) — push the lead too far and it goes
   * darker, which reads as a trench cut through the window rather than a bar
   * laid over it. Light inverts the relationship (its bar is painted darker than
   * its content), so the invariant is "the painted ordering survives
   * compositing", in whichever direction the palette states it.
   */
  it("never inverts the sidebar against the content, over any wallpaper", () => {
    for (const theme of THEMES) {
      const bar = paletteChannel(theme, "bg-sidebar");
      const content = paletteChannel(theme, "bg-primary");
      const barIsLighter = bar > content;

      for (const t of POSITIONS) {
        const { pane, sidebar } = transparencyToAlphas(t, theme);
        for (const bg of BACKDROPS) {
          const composited = { bar: over(bar, sidebar, bg), content: over(content, pane, bg) };
          const stillLighter = composited.bar > composited.content;
          expect(
            t === 0 ? composited.bar !== composited.content : stillLighter === barIsLighter,
            `${theme} sidebar inverted at t=${t} over backdrop ${bg}: ` +
              `bar ${composited.bar.toFixed(1)} vs content ${composited.content.toFixed(1)}`,
          ).toBe(true);
        }
      }
    }
  });

  it("keeps a hover surface distinguishable from the pane it lies on", () => {
    for (const theme of THEMES) {
      const surfaceColour = paletteChannel(theme, "bg-surface");
      const content = paletteChannel(theme, "bg-primary");
      for (const t of POSITIONS) {
        const { pane, surface } = transparencyToAlphas(t, theme);
        for (const bg of BACKDROPS) {
          // Direction flips with the wallpaper — a denser surface gains less
          // from a bright one — but a hover row that composites to the SAME
          // value as its pane is a hover row you cannot see.
          const delta = Math.abs(over(surfaceColour, surface, bg) - over(content, pane, bg));
          expect(delta, `${theme} surface indistinguishable at t=${t} over ${bg}`).toBeGreaterThan(2);
        }
      }
    }
  });
});

describe("the pane / floating split", () => {
  const themeBlock = CSS.split("@theme")[1].split("\n}")[0];
  const decl = (name: string) =>
    themeBlock.match(new RegExp(`--color-${name}:([^;]+);`))?.[1] ?? "";

  it("gives every pane token an alpha and every floating token none", () => {
    // A pane composites over the blurred desktop and must carry the window's
    // transparency.
    for (const [token, alpha] of [
      ["base", "--alpha-pane"],
      ["sidebar", "--alpha-sidebar"],
      ["surface", "--alpha-surface"],
    ]) {
      expect(decl(token), `--color-${token} must resolve through ${alpha}`).toContain(alpha);
    }
    // A menu or dialog portals to the document root, so its backdrop is the
    // transparent window itself — translucent there is text on raw wallpaper.
    for (const token of ["base-solid", "surface-solid", "elevated", "input"]) {
      expect(decl(token), `--color-${token} must stay opaque`).not.toContain("--alpha-");
      expect(decl(token).trim(), `--color-${token} must be declared`).not.toBe("");
    }
  });

  it("keeps each solid token the same colour as its alpha-aware twin", () => {
    // This is what lets an alpha-aware child sit inside a solid parent with no
    // edit (AccountSetup inside the add-account modal): same triplet composites
    // to the identical result.
    expect(decl("base-solid").trim()).toBe("rgb(var(--bg-primary))");
    expect(decl("surface-solid").trim()).toBe("rgb(var(--bg-surface))");
  });

  it("leaves the body transparent so it cannot paint over the blur", () => {
    const body = CSS.split(/^body \{/m)[1].split("}")[0];
    expect(body).toMatch(/background:\s*transparent/);
  });

  it("declares a fully-opaque pre-hydrate fallback for every alpha", () => {
    // Without these the alphas are invalid at computed-value time before
    // `applyTheme` runs, and the first frame is bare glass with
    // floating text.
    const root = CSS.split(/^:root \{/m)[1].split("}")[0];
    for (const a of ["--alpha-pane", "--alpha-sidebar", "--alpha-surface"]) {
      expect(root).toMatch(new RegExp(`${a}:\\s*1\\s*;`));
    }
  });
});
