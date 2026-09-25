/**
 * The window-transparency dial as pure arithmetic: one slider value in, the
 * three pane alphas out, with each theme's floor and sidebar lead.
 *
 * Lives outside `uiStore` so the vault palette (`vaultLook.ts`) can reason
 * about the same alphas the window will actually paint without importing the
 * store — the store imports the palette, and a cycle there would run the
 * store's rehydration before the palette module had evaluated. `uiStore`
 * re-exports everything here, so existing imports are unchanged.
 */

/**
 * A theme that can actually be painted. Everything downstream of
 * `resolveTheme` — the palette floors, `data-theme`, the native pin — is keyed
 * by this, never by the preference.
 */
export type Theme = "dark" | "light";

/**
 * How much of the desktop shows through the window, 0 → 1.
 *
 * A continuous amount rather than named presets. Presets were cxtasks' first
 * cut and were wrong for the same reason they would be wrong here: which value
 * is right depends entirely on the wallpaper behind the window, so the useful
 * range is a dial the user turns until it looks right, not three points
 * someone else picked.
 *
 * A separate axis from `Theme` rather than a fourth theme, because it composes
 * with both — "dark and nearly opaque" and "light and nearly glass" are equally
 * valid.
 *
 * 0 is a real destination, not a disabled state: transparency over a busy
 * wallpaper can make a mailbox genuinely hard to read, and the way back has to
 * be a drag away.
 *
 * The default sits below cxtasks' 0.38 on purpose. This is a reading app —
 * message bodies render in a `background: transparent` iframe (EmailFrame), so
 * body text lands directly on the glass rather than on a card over it.
 */
export const TRANSPARENCY_DEFAULT = 0.3;

/**
 * The floor each theme's pane alpha may reach at full transparency.
 *
 * Light needs a much higher floor than dark, and this is not a fudge factor.
 * Light glass over a DARK desktop does not read as airy, it reads as grey: at
 * the old 0.55 the sidebar let half a near-black wallpaper through and
 * composited to ~125, the same value as `--text-muted`, so every secondary
 * label vanished (2026-09-25, measured off a real screenshot). WKWebView cannot
 * use the brightening blend modes Apple's light materials rely on, so the only
 * lever is how much backdrop gets through. 0.82 is what keeps muted text at 3:1
 * over a black backdrop at the top of the slider — pinned by the legibility
 * test in `windowTransparency.test.ts`, which is the thing to consult before
 * lowering it.
 */
export const PANE_FLOOR: Record<Theme, number> = { dark: 0.25, light: 0.82 };

/**
 * How far the sidebar's alpha LEADS the content's, as a fraction, at full
 * transparency. The bar is always thinner than the work area — that thinness is
 * what makes chrome read as laid over the content rather than cut out of it.
 *
 * **This is where cxmail departs from cxtasks, and the palette is why.** cxtasks
 * boosts the slider before applying it to the sidebar (×1.9) and gives the bar
 * its own low floor, which its palette can carry because its bar is 42 against a
 * content of 24 — 1.75× lighter. cxmail's is 35 against 28, only 1.25×, and at
 * that ratio cxtasks' curve inverts the bar into a trench: at t=0.38 it would
 * composite to 12 against the content's 20 over a black wallpaper, i.e. darker
 * than the thing it sits on, which is the exact failure cxtasks' own note warns
 * about.
 *
 * The algebra behind the constant: over a black backdrop the bar stays lighter
 * only while `35·a_bar > 28·a_pane`, i.e. `a_bar > 0.8·a_pane`; a brighter
 * wallpaper only relaxes that, since the thinner surface gains more from it. So
 * the lead has to stay under 0.2, and 0.12 keeps a working margin at every
 * slider position. Light inverts the ordering (its bar is DARKER than its
 * content) and is far less constrained — 243 vs 248 needs only
 * `a_bar > 0.58·a_pane` — so its lead is set by legibility instead: the sidebar
 * carries most of the muted text in the app, and in light every point of lead
 * is more dark desktop behind that text. 0.05 keeps the bar visibly thinner
 * without spending the contrast the floor above bought.
 *
 * Expressed as a fraction OF THE PANE ALPHA rather than as a scaled result, so
 * `t = 0` yields 1 for both and the opaque end of the slider is genuinely
 * opaque — cxtasks' warning that `pane * 0.5` leaves the sidebar half
 * transparent at rest applies just as much here.
 */
export const SIDEBAR_LEAD: Record<Theme, number> = { dark: 0.12, light: 0.05 };

/**
 * Normalize a transparency into `[0, 1]`, treating anything non-finite as
 * "unset" rather than passing it through.
 *
 * `Math.min(1, Math.max(0, NaN))` is `NaN`, and a NaN alpha paints a pane fully
 * TRANSPARENT — the window disappears and only the text is left floating over
 * the desktop. cxtasks guards this where it reads localStorage; CXMail's read
 * site is zustand's `persist` rehydration, which validates nothing, so a
 * corrupted or hand-edited `cxmail-ui` entry (`"transparency": null`, or a
 * string) would land straight in the store. Guarding here covers both that path
 * and `setTransparency`, which are the only two ways a value gets in.
 *
 * Falls back to the default rather than to 0, so corrupt storage behaves like
 * no storage instead of silently turning the feature off.
 */
export function clampTransparency(value: number): number {
  if (!Number.isFinite(value)) return TRANSPARENCY_DEFAULT;
  return Math.min(1, Math.max(0, value));
}

/**
 * Turn one slider value into the three pane alphas.
 *
 * Three relationships hold at every position, which is why this is a formula
 * and not three sliders. SIDEBAR always sits below the content — see above.
 * SURFACE always sits above it: it backs hover rows, the search field and the
 * pane divider, which lie *on* the content, so a surface has to stay denser
 * than the thing it lies on or the depth inverts. And at `t = 0` all three are
 * 1, so the opaque end of the slider is genuinely opaque everywhere.
 *
 * `floor` replaces the theme's `PANE_FLOOR` and in practice only ever raises
 * it: a vault palette whose text cannot survive the theme's floor gets a higher
 * one (`vaultLook.ts::deriveVaultTheme`); everything else takes the default.
 */
export function transparencyToAlphas(
  transparency: number,
  theme: Theme,
  floor: number = PANE_FLOOR[theme],
) {
  const t = clampTransparency(transparency);
  const pane = 1 - t * (1 - floor);
  return {
    pane,
    sidebar: pane * (1 - t * SIDEBAR_LEAD[theme]),
    surface: pane + (1 - pane) * 0.5,
  };
}
