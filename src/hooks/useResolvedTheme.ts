import { useSyncExternalStore } from "react";
import { resolveTheme, useUIStore, type Theme } from "@/stores/uiStore";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Re-render when macOS flips appearance. Only matters while the preference is
 * `system`, but it is cheap to always listen, and a subscription that depends
 * on the preference would miss the flip that happens in between.
 */
function subscribe(onChange: () => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => {};
  const mql = window.matchMedia(DARK_QUERY);
  mql.addEventListener("change", onChange);
  return () => mql.removeEventListener("change", onChange);
}

/**
 * The theme that is actually painted, not the one the user picked.
 *
 * Anything that styles its own document (the email iframe, the hover preview)
 * must read THIS, never `useUIStore(s => s.theme)`: that is a preference, and
 * under `system` it is neither `dark` nor `light`, so a `theme === "dark"`
 * check quietly takes the light branch and prints dark ink on dark glass.
 */
export function useResolvedTheme(): Theme {
  const preference = useUIStore((s) => s.theme);
  // The snapshot is the resolved theme itself, a string, so it is stable
  // between flips and useSyncExternalStore will not loop.
  return useSyncExternalStore(subscribe, () => resolveTheme(preference));
}
