import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";

export default function MenuBarSection() {
  const [visible, setVisible] = useState(true);
  const [busy, setBusy] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const value = await api.system.getShowInMenuBar();
        if (!cancelled) {
          setVisible(value);
          setLoaded(true);
        }
      } catch (e) {
        if (!cancelled) setError("Could not load the menu bar setting. Reopen Settings to retry.");
        void api.system.logClientError("menu-bar", String(e));
      } finally {
        if (!cancelled) setBusy(false);
      }
    }
    void load();
    return () => { cancelled = true; };
  }, []);

  async function toggle() {
    setBusy(true);
    setError(null);
    try {
      await api.system.setShowInMenuBar(!visible);
      setVisible(!visible);
    } catch (e) {
      setError("Could not save the menu bar setting. Please try again.");
      void api.system.logClientError("menu-bar", String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
        Menu bar
      </h3>
      <div className="flex items-center justify-between gap-3">
        <div>
          <p id="menu-bar-label" className="text-xs text-content">Show in menu bar</p>
          <p id="menu-bar-description" className="mt-1 text-[11px] text-content-secondary">
            Show the CXMail icon in the macOS menu bar.
          </p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={visible}
          aria-labelledby="menu-bar-label"
          aria-describedby="menu-bar-description"
          disabled={busy || !loaded}
          onClick={() => void toggle()}
          className={cn(
            "flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50",
            visible ? "bg-accent" : "bg-elevated border border-border",
          )}
        >
          <span className={cn(
            "h-4 w-4 rounded-full bg-white shadow transition-transform",
            visible ? "translate-x-4" : "translate-x-0",
          )} />
        </button>
      </div>
      {error && <p role="alert" className="mt-2 text-[11px] text-red-400">{error}</p>}
    </section>
  );
}
