import { describe, it, expect, vi, afterEach } from "vitest";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  emailVeilAlphas,
  transparencyToAlphas,
  TRANSPARENCY_DEFAULT,
  type Theme,
} from "@/stores/uiStore";

/**
 * The message body's dial, and the veil that implements it.
 *
 * The veil is solved, not tuned: one extra layer, in the colour of whatever the
 * body lies on, whose alpha makes the stack let through exactly as much desktop
 * as a single surface at the email's transparency. Every test here checks that
 * claim against compositing arithmetic rather than restating the formula —
 * because each way of getting it wrong still renders, just not as asked.
 */

const ROOT = resolve(__dirname, "../../..");
const read = (rel: string) => readFileSync(join(ROOT, rel), "utf8");

const THEMES: Theme[] = ["dark", "light"];
const POSITIONS = [0, 0.1, 0.25, TRANSPARENCY_DEFAULT, 0.5, 0.75, 0.9, 1];
/** Black and white wallpapers — the two extremes any real one sits between. */
const BACKDROPS = [0, 255];
/** One channel of `colour` at `alpha`, laid over `under`. */
const over = (colour: number, alpha: number, under: number) => colour * alpha + under * (1 - alpha);

describe("the message-body veil", () => {
  /** Equal dials must mean no veil at all — that is what makes upgrading, and a
   * user who never touches the new slider, look exactly as before. */
  it("adds nothing while the email dial matches the window", () => {
    for (const theme of THEMES) {
      for (const t of POSITIONS) {
        const v = emailVeilAlphas(t, t, theme);
        expect(v.pane).toBeCloseTo(0, 12);
        expect(v.card).toBeCloseTo(0, 12);
      }
    }
  });

  /** The single-message view: pane + veil, same colour, must be
   * indistinguishable from one pane at the email's transparency. */
  it("composites to exactly a pane at the email's transparency, over any wallpaper", () => {
    for (const theme of THEMES) {
      for (const w of POSITIONS) {
        for (const e of POSITIONS.filter((x) => x <= w)) {
          const W = transparencyToAlphas(w, theme);
          const E = transparencyToAlphas(e, theme);
          const { pane: v } = emailVeilAlphas(w, e, theme);
          for (const bg of BACKDROPS) {
            const colour = 100; // any value: both layers are the same token
            const actual = over(colour, v, over(colour, W.pane, bg));
            const target = over(colour, E.pane, bg);
            expect(actual, `${theme} w=${w} e=${e} bg=${bg}`).toBeCloseTo(target, 9);
          }
        }
      }
    }
  });

  /** A thread card is already two layers deep (pane, then card), so its veil is
   * solved against that stack — the desktop let through must match the same
   * card drawn at the email's transparency. */
  it("lets the same desktop through a thread card as a card at the email's transparency", () => {
    for (const theme of THEMES) {
      for (const w of POSITIONS) {
        for (const e of POSITIONS.filter((x) => x <= w)) {
          const W = transparencyToAlphas(w, theme);
          const E = transparencyToAlphas(e, theme);
          const { card: v } = emailVeilAlphas(w, e, theme);
          const actual = (1 - W.pane) * (1 - W.surface) * (1 - v);
          const target = (1 - E.pane) * (1 - E.surface);
          expect(actual, `${theme} w=${w} e=${e}`).toBeCloseTo(target, 9);
        }
      }
    }
  });

  it("holds the body fully opaque at zero, whatever the window", () => {
    for (const theme of THEMES) {
      for (const w of POSITIONS) {
        const W = transparencyToAlphas(w, theme);
        const { pane, card } = emailVeilAlphas(w, 0, theme);
        expect((1 - W.pane) * (1 - pane), `${theme} pane w=${w}`).toBeCloseTo(0, 12);
        expect((1 - W.pane) * (1 - W.surface) * (1 - card), `${theme} card w=${w}`).toBeCloseTo(0, 12);
      }
    }
  });

  /** The cap. The email dial can only add opacity: set above the window, it
   * gives exactly the window — no veil — never a body glassier than its frame. */
  it("never makes the email more see-through than the window", () => {
    for (const theme of THEMES) {
      for (const w of POSITIONS) {
        for (const e of POSITIONS.filter((x) => x > w)) {
          const v = emailVeilAlphas(w, e, theme);
          expect(v.pane, `${theme} w=${w} e=${e}`).toBeCloseTo(0, 12);
          expect(v.card, `${theme} w=${w} e=${e}`).toBeCloseTo(0, 12);
        }
      }
    }
  });

  /** An opaque window is the one place the formula would divide by zero, and a
   * NaN veil would be an invalid colour — painted as no veil at all, silently. */
  it("stays finite and in range at the edges and on corrupt input", () => {
    const cases: [number, number][] = [
      [0, 0], [0, 1], [1, 0], [1, 1],
      [Number.NaN, 0.2], [0.4, Number.NaN], [Number.NaN, Number.NaN],
      [-3, 5], [Infinity, -Infinity],
    ];
    for (const theme of THEMES) {
      for (const [w, e] of cases) {
        const v = emailVeilAlphas(w, e, theme);
        for (const a of [v.pane, v.card]) {
          expect(Number.isFinite(a), `${theme} w=${w} e=${e}`).toBe(true);
          expect(a).toBeGreaterThanOrEqual(0);
          expect(a).toBeLessThanOrEqual(1);
        }
      }
    }
    // Corrupt storage means the default — for this dial exactly as for the alphas.
    expect(emailVeilAlphas(Number.NaN, Number.NaN, "dark")).toEqual(
      emailVeilAlphas(TRANSPARENCY_DEFAULT, TRANSPARENCY_DEFAULT, "dark"),
    );
  });
});

describe("the veil tokens and where they are painted", () => {
  const css = read("src/styles/globals.css");
  const count = (text: string, needle: string) => text.split(needle).length - 1;

  /** Same colour as the layer beneath is what makes the composite exact. A
   * pane-coloured veil on a surface-coloured card would turn the card two-tone. */
  it("paints each veil in the colour of the surface it lies on", () => {
    const theme = css.split("@theme")[1].split("\n}")[0];
    expect(theme).toMatch(/--color-email-veil:\s*rgb\(var\(--bg-primary\) \/ var\(--alpha-email-veil\)\);/);
    expect(theme).toMatch(
      /--color-email-veil-card:\s*rgb\(var\(--bg-surface\) \/ var\(--alpha-email-veil-card\)\);/,
    );
  });

  it("defaults both veils to 0 before hydrate", () => {
    const root = css.split(/^:root \{/m)[1].split("}")[0];
    expect(root).toMatch(/--alpha-email-veil:\s*0\s*;/);
    expect(root).toMatch(/--alpha-email-veil-card:\s*0\s*;/);
  });

  /** Body only, by decision: the header rows above stay on the window's glass. */
  it("is painted once on the reading pane's body and once on a thread card's body", () => {
    const pane = read("src/components/mail/ReadingPane.tsx");
    expect(count(pane, "bg-email-veil")).toBe(1);
    expect(count(pane, "bg-email-veil-card")).toBe(0);
    const card = read("src/components/mail/ThreadMessageCard.tsx");
    expect(count(card, "bg-email-veil-card")).toBe(1);
  });
});

describe("upgrading from the single-dial store", () => {
  afterEach(() => localStorage.removeItem("cxmail-ui"));

  /** Seeding from the window's value makes the veil exactly 0 on upgrade: the
   * new dial exists, starts where the user's window already is, and changes
   * nothing until it is moved. */
  it("seeds the email dial from the window's stored value", async () => {
    localStorage.setItem(
      "cxmail-ui",
      JSON.stringify({ state: { transparency: 0.6, theme: "dark" }, version: 1 }),
    );
    vi.resetModules();
    const { useUIStore } = await import("@/stores/uiStore");
    expect(useUIStore.getState().transparency).toBe(0.6);
    expect(useUIStore.getState().emailTransparency).toBe(0.6);
  });

  it("starts a fresh install with the email matching the window default", async () => {
    vi.resetModules();
    const { useUIStore } = await import("@/stores/uiStore");
    expect(useUIStore.getState().emailTransparency).toBe(useUIStore.getState().transparency);
  });
});
