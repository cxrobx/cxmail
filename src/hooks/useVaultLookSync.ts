import { useEffect } from "react";
import { api } from "@/lib/tauri";
import { parseVaultPalette } from "@/lib/vaultLook";
import { useUIStore } from "@/stores/uiStore";

/** Meeting Copilot re-asks on the same minute; Onyx itself re-measures the vault
 * on every Obsidian theme change and every 30 s. */
const POLL_MS = 60_000;

let loggedRefusal: string | null = null;

/**
 * Keep the vault palette current while anyone is looking at it.
 *
 * Asks Onyx now, whenever the window regains focus (the moment someone who just
 * changed their Obsidian theme comes back to look), and once a minute — but only
 * while the theme is `vault` or Settings is open. Otherwise CXMail never talks
 * to Onyx at all, so a user who has never heard of it sees no traffic and no
 * option they cannot use.
 *
 * Every answer goes through `parseVaultPalette` and `setVaultLook`, which keeps
 * the last good palette when Onyx does not answer and repaints only when the
 * palette actually changed.
 */
export function useVaultLookSync(): void {
  const theme = useUIStore((s) => s.theme);
  const settingsOpen = useUIStore((s) => s.settingsOpen);
  const onyxUrl = useUIStore((s) => s.onyxUrl);
  const active = theme === "vault" || settingsOpen;

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const check = () => {
      let pending: Promise<unknown>;
      try {
        pending = api.vaultLook.fetch(onyxUrl);
      } catch {
        return; // No Tauri bridge (plain `vite dev`, tests). Nothing to ask.
      }
      pending
        .then((result) => {
          if (!cancelled) useUIStore.getState().setVaultLook(parseVaultPalette(result));
        })
        .catch((err: unknown) => {
          // Only an address that is not this Mac rejects. Treat it as "Onyx did
          // not answer" — Settings shows the address, and keeping the last
          // palette is kinder than dropping the window's colours. Logged once
          // per address, since this is polled.
          if (!cancelled) useUIStore.getState().setVaultLook(null);
          if (loggedRefusal !== onyxUrl) {
            loggedRefusal = onyxUrl;
            void api.system.logClientError("vault-look", String(err));
          }
        });
    };
    check();
    const timer = window.setInterval(check, POLL_MS);
    window.addEventListener("focus", check);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", check);
    };
  }, [active, onyxUrl]);
}
