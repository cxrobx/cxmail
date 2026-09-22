import { create } from "zustand";
import { persist } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import { api } from "@/lib/tauri";

export type DensityMode = "comfortable" | "compact" | "ultra-compact";

/**
 * A theme that can actually be painted. Everything downstream of
 * `resolveTheme` — the palette floors, `data-theme`, the native pin — is keyed
 * by this, never by the preference.
 */
export type Theme = "dark" | "light";

/**
 * What the user *chose*. `system` is a deferral, not a third palette: it has no
 * floors, no CSS block and nothing to pin, and must be run through
 * `resolveTheme` before it reaches any of them.
 */
export type ThemePreference = Theme | "system";

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
 * Dark text on a bright wallpaper loses legibility far faster than light text
 * on a dark one, so the same slider position has to mean less transparency in
 * light or the top of the range is simply unusable there.
 */
const PANE_FLOOR: Record<Theme, number> = { dark: 0.25, light: 0.55 };

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
 * `a_bar > 0.58·a_pane` — but a matching lead keeps the two themes feeling the
 * same, so it gets one too.
 *
 * Expressed as a fraction OF THE PANE ALPHA rather than as a scaled result, so
 * `t = 0` yields 1 for both and the opaque end of the slider is genuinely
 * opaque — cxtasks' warning that `pane * 0.5` leaves the sidebar half
 * transparent at rest applies just as much here.
 */
const SIDEBAR_LEAD: Record<Theme, number> = { dark: 0.12, light: 0.1 };

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * What macOS is showing right now, or `dark` where there is nothing to ask.
 *
 * Guarded rather than called bare, because this module is imported by tests
 * that touch none of it: vitest runs on jsdom, and **jsdom does not implement
 * `matchMedia` at all**, so an unguarded read throws at import time and takes
 * every store test down with it.
 *
 * `dark` is the right fallback: it matches the store's default, the boot pin in
 * `lib.rs` and `tauri.conf.json`'s window theme, so a webview that cannot
 * answer lands where the app was already going to start.
 */
function systemTheme(): Theme {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return "dark";
  return window.matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

/** Collapse a preference to the theme that actually gets painted. */
export function resolveTheme(preference: ThemePreference): Theme {
  return preference === "system" ? systemTheme() : preference;
}

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
function clampTransparency(value: number): number {
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
 */
export function transparencyToAlphas(transparency: number, theme: Theme) {
  const t = clampTransparency(transparency);
  const pane = 1 - t * (1 - PANE_FLOOR[theme]);
  return {
    pane,
    sidebar: pane * (1 - t * SIDEBAR_LEAD[theme]),
    surface: pane + (1 - pane) * 0.5,
  };
}

/**
 * The message body's own dial, as a VEIL laid over the glass it sits on.
 *
 * The body is where transparency stops being atmosphere and starts costing
 * reading: `EmailFrame` paints no background of its own, so a message's text
 * lands directly on the reading pane's glass and whatever is behind the window
 * runs straight under the words. So the body gets a second dial.
 *
 * **Never more see-through than the window.** The email dial can only ADD
 * opacity. The blur behind it belongs to the window — one CGS blur per window,
 * tuned for the window's own value — so a body glassier than its surroundings
 * would sit over a desktop blurred for less.
 *
 * **A veil, not a scoped token, and the reason is how `@theme` resolves.**
 * `--color-base` is declared on `:root` with `var(--alpha-pane)` inside it, and a
 * custom property's `var()`s are substituted where it is DECLARED — descendants
 * inherit the already-resolved colour. Overriding `--alpha-pane` on the body
 * therefore changes nothing at all, silently. Instead the body paints one extra
 * layer in the SAME colour as the surface beneath it, with an alpha solved so the
 * two layers let through exactly as much desktop as one pane at the email's
 * transparency would:
 *
 *     (1 − under)(1 − veil) = 1 − target   ⇒   veil = (target − under) / (1 − under)
 *
 * Same colour on both layers makes that exact, not an approximation.
 *
 * Two veils, because the body sits on two different things. In the
 * single-message view it lies on the reading pane (`bg-base`). In a thread it
 * lies on a card (`bg-surface`) that is itself laid over that pane, so its veil
 * is surface-coloured — a pane-coloured one would turn the card two-tone — and is
 * solved against the card's two-layer coverage, targeting the same card drawn at
 * the email's transparency.
 */
export function emailVeilAlphas(transparency: number, emailTransparency: number, theme: Theme) {
  const windowT = clampTransparency(transparency);
  // No explicit cap needed: a veil can only ADD coverage, so an email dial set
  // glassier than the window solves to a negative alpha, which `solveVeil`
  // clamps to 0 — exactly the window. The cap is the arithmetic of layering,
  // not a rule someone can forget to apply.
  const emailT = clampTransparency(emailTransparency);
  const w = transparencyToAlphas(windowT, theme);
  const e = transparencyToAlphas(emailT, theme);
  const cardUnder = 1 - (1 - w.pane) * (1 - w.surface);
  const cardTarget = 1 - (1 - e.pane) * (1 - e.surface);
  return { pane: solveVeil(w.pane, e.pane), card: solveVeil(cardUnder, cardTarget) };
}

/**
 * The alpha a layer needs over coverage `under` to reach coverage `target`.
 * Nothing to add when the layer beneath is already opaque — which is also what
 * keeps the opaque end of the window dial from dividing by zero. The clamp at 0
 * is load-bearing: it IS the email dial's cap (`emailVeilAlphas`), and it also
 * absorbs float error when the two dials are equal.
 */
function solveVeil(under: number, target: number): number {
  if (under >= 1) return 0;
  return Math.min(1, Math.max(0, (target - under) / (1 - under)));
}

/**
 * Push the transparency to the DOM: the three pane alphas, plus the two veils
 * that hold the message body at the email dial (`emailVeilAlphas`).
 *
 * Takes the THEME as well, and must be re-run on every theme change — the
 * mapping is theme-dependent (`PANE_FLOOR`), so leaving it alone when the theme
 * flips silently applies dark's floor to a light window, which is the
 * unreadable case this exists to prevent. `applyTheme` calls it for that
 * reason; there is no path that sets one without the other.
 *
 * The CSS write comes FIRST and is never gated on the native call. That
 * ordering is what keeps a drag smooth: the panes repaint every frame from these
 * three variables, and `applyGlass` only reaches AppKit on the frames where the
 * rounded blur radius actually changes. A failed IPC call can therefore leave
 * the blur stale, but never strand the window half-transparent.
 *
 * This used to be a pure CSS change, back when the blur was an
 * `NSVisualEffectView` installed once at startup and left alone. It isn't any
 * more: `glass_macos` takes a radius, and the radius is part of the same dial.
 */
export function applyWindow(transparency: number, theme: Theme, emailTransparency: number): void {
  const windowT = effectiveTransparency(transparency);
  const { pane, sidebar, surface } = transparencyToAlphas(windowT, theme);
  // Reduce Transparency pins both dials to 0, so the veils are 0 there too —
  // everything they would sit on is already opaque.
  const veil = emailVeilAlphas(windowT, effectiveTransparency(emailTransparency), theme);
  const root = document.documentElement;
  root.style.setProperty("--alpha-pane", String(pane));
  root.style.setProperty("--alpha-sidebar", String(sidebar));
  root.style.setProperty("--alpha-surface", String(surface));
  root.style.setProperty("--alpha-email-veil", String(veil.pane));
  root.style.setProperty("--alpha-email-veil-card", String(veil.card));
  applyGlass(transparency, theme);
}

/**
 * The desktop blur radius, as a function of the same slider.
 *
 * One dial, not two, and the coupling is not arbitrary: the more of the desktop
 * the panes let through, the more blur it takes to keep text on top of it
 * readable. Two independent controls would let you pick the one combination
 * that is always wrong — wide open and perfectly sharp — and would turn the
 * useful setting into a two-dimensional hunt instead of a drag.
 *
 * The band is narrower than the WindowServer's (`glass_macos::BLUR_MIN/MAX` is
 * only a sanity bound). Past roughly 48 a blurred wallpaper stops being a
 * wallpaper and becomes the same featureless smoke `NSVisualEffectView`
 * produced — the look this whole path exists to get away from. The floor is 10
 * rather than 0 because a slightly transparent window over a razor-sharp
 * desktop reads as a rendering fault, not a design.
 */
const BLUR_RADIUS_MIN = 10;
const BLUR_RADIUS_MAX = 48;

export function transparencyToBlurRadius(transparency: number): number {
  // `clampTransparency`, not a bare `Math.min`/`Math.max` pair: those pass NaN
  // straight through, `Math.round(NaN)` is NaN, and NaN reaches `invoke` as
  // JSON `null`, which serde refuses for a `u8` — so a corrupt `cxmail-ui`
  // entry would take the blur out entirely. Using the clamp the alphas use also
  // keeps the dial's two outputs agreeing on what a corrupt value means: the
  // DEFAULT, for both. (cxtasks maps NaN to the floor, which here would pair
  // default-translucent panes with the thinnest blur.)
  const t = clampTransparency(transparency);
  return Math.round(BLUR_RADIUS_MIN + t * (BLUR_RADIUS_MAX - BLUR_RADIUS_MIN));
}

/**
 * Each theme's opaque base colour, as the RGB triple Rust paints the window in
 * when glass is off.
 *
 * Must track `--bg-primary` in `globals.css`. Duplicated rather than read back
 * from the computed style because the dark value is also the LAUNCH colour,
 * which `lib.rs` paints before any JavaScript has run — so it lives in Rust a
 * third time. `glass.test.ts` reads all three and fails on drift.
 */
export const THEME_BASE_RGB: Record<Theme, readonly [number, number, number]> = {
  dark: [28, 26, 23],
  light: [248, 247, 245],
};

/**
 * Glass is off until the first paint, then tracks the slider.
 *
 * `glassReady` exists because the native call has to come AFTER React has
 * painted. The window is `transparent: true`, and a clear window around an
 * empty webview is bare wallpaper with three floating traffic lights (gotcha
 * #59) — so `lib.rs` boots it opaque and `startGlass` flips it once there is a
 * mailbox to look at. Every earlier `applyWindow`, including `main.tsx`'s
 * pre-render `applyTheme`, moves only the CSS.
 *
 * `lastSent` is the IPC guard. `applyWindow` runs on every frame of a slider
 * drag, but the radius is an integer over a 38-wide band, so it changes at most
 * 38 times across a full sweep. Comparing before sending is what keeps the drag
 * the CSS operation it was designed as.
 */
let glassReady = false;
let lastSent: { enabled: boolean; radius: number; theme: Theme } | null = null;

/**
 * macOS's **Accessibility → Display → Reduce transparency**, mirrored here.
 *
 * It pins the EFFECTIVE transparency to 0 — not merely the window's blur. The
 * native side goes opaque on its own (`glass_macos::set_state` refuses glass
 * while it is on), but the panes carry their own alpha, so leaving the CSS alone
 * would composite a translucent sidebar over an opaque window and land it a
 * shade off its own token. "The slider is at 0 right now" covers both halves
 * with one rule, and it is simply true: 0 is the opaque end of this dial.
 *
 * The stored slider is NOT moved. `transparency` stays where the user put it,
 * so turning the setting back off restores their window rather than stranding
 * them at opaque with no memory of what they had.
 */
let reduceTransparency = false;

/** What the window should actually render at, once accessibility has its say. */
function effectiveTransparency(transparency: number): number {
  return reduceTransparency ? 0 : transparency;
}

/**
 * Push the window's glass state to AppKit.
 *
 * Off at transparency 0 — a real off, not a radius of zero. At 0 the panes are
 * fully opaque, so the blur would be invisible anyway; making the NSWindow
 * genuinely opaque there means the opaque end gives an ordinary window with an
 * ordinary shadow, not a transparent one that happens to be painted over.
 *
 * Failures are swallowed for the reason `applyTheme` documents — no Tauri
 * bridge in a plain `vite dev` tab or a vitest run — and a window that renders
 * without its blur beats a white screen.
 */
export function applyGlass(transparency: number, theme: Theme): void {
  if (!glassReady) return;
  const t = clampTransparency(effectiveTransparency(transparency));
  const enabled = t > 0;
  const radius = transparencyToBlurRadius(t);
  if (
    lastSent !== null &&
    lastSent.enabled === enabled &&
    lastSent.radius === radius &&
    lastSent.theme === theme
  ) {
    return;
  }
  // Radius-only when nothing else moved: the common case during a drag, and it
  // skips re-running the window setup (opacity, background, shadow).
  const radiusOnly = lastSent !== null && lastSent.enabled && enabled && lastSent.theme === theme;
  lastSent = { enabled, radius, theme };
  try {
    const call = radiusOnly
      ? api.glass.setRadius(radius)
      : api.glass.set(enabled, radius, THEME_BASE_RGB[theme]);
    void call.catch(() => {
      // Not macOS, or the window is gone. The CSS half still applied.
    });
  } catch {
    // No Tauri runtime at all. CSS-only transparency is the correct degradation.
  }
}

/**
 * Arm the glass, once, after the first paint.
 *
 * Called from `main.tsx` behind a double `requestAnimationFrame`: one frame to
 * get the render scheduled, the second to land after it has been composited. A
 * single frame fires early often enough to show the flash it exists to prevent,
 * and a `setTimeout` would only be a guess at the number rAF measures.
 */
export function startGlass(): void {
  const paint = () => {
    const { transparency, emailTransparency, theme } = useUIStore.getState();
    applyWindow(transparency, resolveTheme(theme), emailTransparency);
  };

  // Read the accessibility setting BEFORE arming, so a user who has it on never
  // sees a frame of glass. A failed read means not-macOS or no Tauri bridge,
  // and `false` is the right answer for both.
  try {
    void api.glass
      .reduceTransparency()
      .catch(() => false)
      .then((on) => {
        reduceTransparency = on;
        glassReady = true;
        paint();
      });
  } catch {
    glassReady = true;
    paint();
  }

  // Someone turns this on because they are struggling NOW, so it has to apply
  // without a relaunch. The native half is the observer in `glass_macos`; this
  // is the CSS half.
  try {
    void listen<boolean>("reduce-transparency-changed", (event) => {
      reduceTransparency = event.payload;
      // The guard in `applyGlass` compares against the last values SENT, and
      // the native side has already moved underneath it — so clear it, or a
      // flip back to glass can be skipped as a no-op and leave the window opaque.
      lastSent = null;
      paint();
    }).catch(() => {
      // No Tauri event bus. The hydrate read above still applied.
    });
  } catch {
    // Same, thrown synchronously.
  }
}

/**
 * Push the theme to all three layers that have to agree.
 *
 * `data-theme` drives the CSS variables in `globals.css`. The pane alphas
 * follow, because their floors are theme-dependent. The native call drives
 * AppKit's `NSApp.appearance`, which CSS cannot reach — skip it and the
 * right-click NSMenu, sheets and scrollbars stay on the system theme (as does
 * the `NSVisualEffectView` fallback, on a Mac that has fallen back to it). The
 * glass moves too, through `applyWindow`: turning it off repaints the window
 * opaque in the theme's base colour, so that colour has to follow the theme.
 */
export function applyTheme(preference: ThemePreference): void {
  const theme = resolveTheme(preference);
  document.documentElement.setAttribute("data-theme", theme);
  const { transparency, emailTransparency } = useUIStore.getState();
  applyWindow(transparency, theme, emailTransparency);
  // Fire-and-forget, and defended twice on purpose. `invoke` throws
  // SYNCHRONOUSLY when the Tauri IPC bridge is not on `window` (a plain
  // `vite dev` browser tab, or a vitest run), which a bare `.catch()` would
  // sail straight past.
  try {
    // Deliberately the PREFERENCE, not `theme`: under `system` the native side
    // must clear the pin rather than set the resolved value. Sending the
    // resolved value would look identical for one frame and then freeze —
    // `NSApp.appearance` is what the webview derives `prefers-color-scheme`
    // from, so a pin makes `watchSystemTheme` below permanently silent.
    void api.appearance.setNative(preference === "system" ? null : theme === "dark").catch(() => {
      // Not macOS, or no window yet. The CSS half still applied.
    });
  } catch {
    // No Tauri runtime at all. CSS-only theming is the correct degradation.
  }
}

/**
 * Repaint when macOS flips, for as long as the preference stays `system`.
 *
 * Registered at module load rather than from a React effect on purpose:
 * `main.tsx` calls `applyTheme` before the first render precisely so no frame
 * paints at the wrong theme, and an effect-based listener would leave that
 * pre-render window unwatched. There is exactly one listener for the life of
 * the app, so it is never torn down.
 *
 * Reads the preference at fire time instead of unsubscribing when it changes:
 * the query keeps matching either way, and a stale-closure bug here would
 * either repaint a pinned window or stop tracking a system one.
 */
function watchSystemTheme(): void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  window.matchMedia(DARK_QUERY).addEventListener("change", () => {
    if (useUIStore.getState().theme === "system") applyTheme("system");
  });
}
watchSystemTheme();

interface ActiveSend {
  sendId: string;
  subject: string;
  startedAt: number;
  delaySeconds: number;
  draftCleanup?: { accountId: string; folder: string; uid: number };
  /** Captured at send-click time. Logged on send-completed only — never on cancel/fail. */
  voiceLearning?: {
    accountId: string;
    aiDraft: string;
    sentBody: string;
    recipientEmail?: string | null;
  };
}

export interface Toast {
  id: string;
  message: string;
  type: "success" | "error" | "info";
  action?: { label: string; onClick: () => void };
  duration?: number;
  /** Fires only when the duration timer elapses — not on action click or manual dismiss. */
  onExpire?: () => void;
}

interface UIState {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  listPaneHeight: number;
  messageListWidth: number;
  theme: ThemePreference;
  density: DensityMode;
  /** 0 = fully opaque, 1 = as translucent as the current theme allows. */
  transparency: number;
  /** How see-through the MESSAGE BODY may be, on the same 0 → 1 scale as
   * `transparency` and never more see-through than it (`emailVeilAlphas`). */
  emailTransparency: number;
  /** The settings dialog. Chrome state like the rest of this store. */
  settingsOpen: boolean;
  undoSendDelaySeconds: number;
  activeSend: ActiveSend | null;
  showShortcutSheet: boolean;
  calendarFullScreen: boolean;
  /** Last account used as From on a sent/scheduled compose — the new-compose
   * default when no account context (unified inbox) exists. */
  lastFromAccountId: string | null;
  /** Last 8 Enter-committed search queries, newest first (persisted). */
  recentSearches: string[];
  /** Last 8 command-palette command ids run, newest first (persisted). */
  recentCommandIds: string[];
  toasts: Toast[];
  setSidebarWidth: (w: number) => void;
  toggleSidebar: () => void;
  toggleShortcutSheet: () => void;
  toggleCalendarFullScreen: () => void;
  setListPaneHeight: (h: number) => void;
  setMessageListWidth: (w: number) => void;
  setTheme: (theme: ThemePreference) => void;
  setDensity: (density: DensityMode) => void;
  setTransparency: (value: number) => void;
  setEmailTransparency: (value: number) => void;
  setSettingsOpen: (open: boolean) => void;
  setUndoSendDelay: (seconds: number) => void;
  setActiveSend: (send: ActiveSend | null) => void;
  setLastFromAccountId: (id: string) => void;
  addRecentSearch: (query: string) => void;
  addRecentCommand: (id: string) => void;
  addToast: (toast: Omit<Toast, "id">) => void;
  removeToast: (id: string) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      sidebarWidth: 240,
      sidebarCollapsed: false,
      listPaneHeight: 40,
      messageListWidth: 500,
      theme: "dark",
      density: "comfortable",
      transparency: TRANSPARENCY_DEFAULT,
      emailTransparency: TRANSPARENCY_DEFAULT,
      settingsOpen: false,
      undoSendDelaySeconds: 5,
      activeSend: null,
      showShortcutSheet: false,
      calendarFullScreen: false,
      lastFromAccountId: null,
      recentSearches: [],
      recentCommandIds: [],
      toasts: [],
      setSidebarWidth: (sidebarWidth) => set({ sidebarWidth }),
      toggleShortcutSheet: () =>
        set((state) => ({ showShortcutSheet: !state.showShortcutSheet })),
      toggleSidebar: () =>
        set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
      toggleCalendarFullScreen: () =>
        set((state) => ({ calendarFullScreen: !state.calendarFullScreen })),
      setListPaneHeight: (listPaneHeight) => set({ listPaneHeight }),
      setMessageListWidth: (messageListWidth) => set({ messageListWidth }),
      setTheme: (theme) => {
        // Paint first, store second — `applyTheme` moves the CSS variables, the
        // pane alphas and native chrome together, and a re-render that beat it
        // would show one frame of the new theme at the old theme's alphas.
        applyTheme(theme);
        set({ theme });
      },
      setDensity: (density) => set({ density }),
      setTransparency: (value) => {
        const next = clampTransparency(value);
        // Runs on every frame of a slider drag, and the DOM write is what the
        // user is actually watching; putting the Zustand set first would
        // re-render the dialog before the window updates.
        const { theme, emailTransparency } = useUIStore.getState();
        applyWindow(next, resolveTheme(theme), emailTransparency);
        set({ transparency: next });
      },
      setEmailTransparency: (value) => {
        const next = clampTransparency(value);
        // Same order as `setTransparency`: the window repaints before the dialog
        // re-renders.
        const { theme, transparency } = useUIStore.getState();
        applyWindow(transparency, resolveTheme(theme), next);
        set({ emailTransparency: next });
      },
      setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
      setUndoSendDelay: (undoSendDelaySeconds) => set({ undoSendDelaySeconds }),
      setActiveSend: (activeSend) => set({ activeSend }),
      setLastFromAccountId: (lastFromAccountId) => set({ lastFromAccountId }),
      addRecentSearch: (query) =>
        set((state) => {
          const q = query.trim();
          if (!q) return {};
          return {
            recentSearches: [q, ...state.recentSearches.filter((s) => s !== q)].slice(0, 8),
          };
        }),
      addRecentCommand: (id) =>
        set((state) => ({
          recentCommandIds: [id, ...state.recentCommandIds.filter((c) => c !== id)].slice(0, 8),
        })),
      addToast: (toast) =>
        set((state) => ({
          toasts: [...state.toasts, { ...toast, id: crypto.randomUUID() }],
        })),
      removeToast: (id) =>
        set((state) => ({
          toasts: state.toasts.filter((t) => t.id !== id),
        })),
    }),
    {
      name: "cxmail-ui",
      version: 2,
      migrate: (persistedState, version) => {
        const s = persistedState as Partial<UIState> | undefined;
        if (s && version < 1) {
          delete (s as { messageListWidth?: number }).messageListWidth;
        }
        // v2 added the message-body dial. Seed it from the window's own value:
        // equal dials mean a veil of exactly 0, so an upgrade looks identical,
        // and the new slider starts where the user's window already is rather
        // than at a default that would silently re-tint their mail.
        if (s && version < 2 && typeof s.emailTransparency !== "number") {
          s.emailTransparency =
            typeof s.transparency === "number" ? s.transparency : TRANSPARENCY_DEFAULT;
        }
        return s as UIState;
      },
      partialize: (state) => {
        const { toasts, activeSend, settingsOpen, ...persisted } = state;
        return persisted;
      },
    }
  )
);
