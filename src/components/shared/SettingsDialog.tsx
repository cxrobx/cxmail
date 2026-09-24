import { useCallback, useEffect, useRef, useState } from "react";
import { Mail, Monitor, Moon, Sun, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useUIStore, type DensityMode, type ThemePreference } from "@/stores/uiStore";
import TriageSection from "./TriageSection";
import SendAsSection from "./SendAsSection";

// "System" sits last rather than first: the two explicit choices are the ones
// with muscle memory behind them, and moving them would retrain a click that
// already works.
const THEMES: { value: ThemePreference; label: string; icon: typeof Sun }[] = [
  { value: "dark", label: "Dark", icon: Moon },
  { value: "light", label: "Light", icon: Sun },
  { value: "system", label: "System", icon: Monitor },
];

const DENSITIES: { value: DensityMode; label: string }[] = [
  { value: "comfortable", label: "Comfortable" },
  { value: "compact", label: "Compact" },
  { value: "ultra-compact", label: "Ultra" },
];

/**
 * Appearance settings.
 *
 * CXMail had no settings surface at all — theme and density were reachable only
 * through the command palette, which is a fine way to *run* a command and a bad
 * way to discover that a preference exists. A slider is not a command at all,
 * so the transparency dial forced the question.
 *
 * The dialog deliberately does NOT dim the app behind it. Every control in here
 * changes how the window itself looks, so the thing being adjusted has to stay
 * visible while it is adjusted; a standard scrim would black out the exact
 * surfaces the slider is moving. The `fixed inset-0` layer is a click-outside
 * catcher, nothing more — the same shape `KeyboardShortcutSheet` uses.
 *
 * The panel is `bg-base-solid` rather than `bg-base` for the reason spelled out
 * in `globals.css`: at high transparency a translucent settings panel over a
 * translucent window is two sheets of glass and no readable text.
 */
export default function SettingsDialog() {
  const open = useUIStore((s) => s.settingsOpen);
  const setOpen = useUIStore((s) => s.setSettingsOpen);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const density = useUIStore((s) => s.density);
  const setDensity = useUIStore((s) => s.setDensity);
  const transparency = useUIStore((s) => s.transparency);
  const setTransparency = useUIStore((s) => s.setTransparency);
  const emailTransparency = useUIStore((s) => s.emailTransparency);
  const setEmailTransparency = useUIStore((s) => s.setEmailTransparency);

  // Dragging is done in the PAGE, with a transform — deliberately not with
  // `data-tauri-drag-region`, which moves the OS window (gotcha #51). Grabbing
  // this dialog's header must move the dialog, not slide CXMail across the
  // desktop, and the two are one attribute apart.
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number } | null>(null);

  // Re-centre on each open. A dialog that reappears wherever it was dragged
  // three days ago reads as broken placement rather than as memory.
  useEffect(() => {
    if (open) setOffset({ x: 0, y: 0 });
  }, [open]);

  const onHeaderPointerDown = useCallback(
    (e: React.PointerEvent) => {
      // Let the close button behave like a button.
      if ((e.target as HTMLElement).closest("button")) return;
      drag.current = { x: e.clientX - offset.x, y: e.clientY - offset.y };
      const move = (ev: PointerEvent) => {
        if (!drag.current) return;
        setOffset({ x: ev.clientX - drag.current.x, y: ev.clientY - drag.current.y });
      };
      const up = () => {
        drag.current = null;
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [offset.x, offset.y],
  );

  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, setOpen]);

  if (!open) return null;

  const pct = Math.round(transparency * 100);
  const emailPct = Math.round(emailTransparency * 100);
  // The email dial never outruns the window's. Say so, rather than leave a
  // slider that looks broken because dragging it past the window does nothing.
  const emailCapped = emailPct > pct;

  const segment = (active: boolean) =>
    cn(
      "flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-1.5 text-xs transition-colors",
      active
        ? "bg-elevated text-content shadow-macos"
        : "text-content-secondary hover:text-content",
    );

  return (
    <div
      className="fixed inset-0 z-[100] flex items-start justify-center pt-[10vh]"
      onClick={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-label="Settings"
        className="flex max-h-[80vh] w-[420px] flex-col overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
        style={{ transform: `translate(${offset.x}px, ${offset.y}px)` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          onPointerDown={onHeaderPointerDown}
          className="flex shrink-0 cursor-grab select-none items-center justify-between border-b border-border-subtle px-4 py-3 active:cursor-grabbing"
        >
          <h2 className="text-sm font-medium text-content">Settings</h2>
          <button
            onClick={() => setOpen(false)}
            aria-label="Close settings"
            className="rounded p-1 text-content-secondary hover:text-content"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 space-y-5 overflow-y-auto p-4">
          <section>
            <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
              Theme
            </h3>
            <div className="flex gap-1 rounded-lg bg-surface p-1">
              {THEMES.map(({ value, label, icon: Icon }) => (
                <button
                  key={value}
                  onClick={() => setTheme(value)}
                  aria-pressed={theme === value}
                  className={segment(theme === value)}
                >
                  <Icon className="h-3.5 w-3.5" />
                  {label}
                </button>
              ))}
            </div>
          </section>

          <section>
            <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
              Density
            </h3>
            <div className="flex gap-1 rounded-lg bg-surface p-1">
              {DENSITIES.map(({ value, label }) => (
                <button
                  key={value}
                  onClick={() => setDensity(value)}
                  aria-pressed={density === value}
                  className={segment(density === value)}
                >
                  {label}
                </button>
              ))}
            </div>
          </section>

          <section>
            <div className="mb-1.5 flex items-baseline justify-between">
              <h3 className="text-xs font-medium uppercase tracking-wider text-content-muted">
                Window transparency
              </h3>
              <span className="tabular-nums text-[11px] text-content-muted">{pct}%</span>
            </div>
            <input
              type="range"
              className="cx-slider"
              min={0}
              max={100}
              step={1}
              value={pct}
              aria-label="Window transparency"
              // `--fill` drives the track's filled portion; see globals.css.
              style={{ ["--fill" as string]: `${pct}%` }}
              onChange={(e) => setTransparency(Number(e.target.value) / 100)}
            />
            <div className="mt-1 flex justify-between text-[11px] text-content-faint">
              <span>Opaque</span>
              <span>Glass</span>
            </div>
            <p className="mt-2.5 flex items-start gap-1.5 text-[11px] leading-snug text-content-faint">
              <Monitor className="mt-px h-3 w-3 shrink-0" />
              {/* Worth saying out loud: the same percentage is not the same
                  result in both themes, and without this the light window looks
                  like the slider is broken at the top of its range. */}
              <span>
                Light mode holds more opacity at the same setting — dark text loses to a bright
                wallpaper much faster than light text does.
              </span>
            </p>
          </section>

          <section>
            <div className="mb-1.5 flex items-baseline justify-between">
              <h3 className="text-xs font-medium uppercase tracking-wider text-content-muted">
                Email transparency
              </h3>
              <span className="tabular-nums text-[11px] text-content-muted">
                {emailCapped ? `capped at ${pct}%` : `${emailPct}%`}
              </span>
            </div>
            <input
              type="range"
              className="cx-slider"
              min={0}
              max={100}
              step={1}
              value={emailPct}
              aria-label="Email transparency"
              style={{ ["--fill" as string]: `${emailPct}%` }}
              onChange={(e) => setEmailTransparency(Number(e.target.value) / 100)}
            />
            <div className="mt-1 flex justify-between text-[11px] text-content-faint">
              <span>Opaque</span>
              {/* Not "Glass": anywhere at or past the window's own value the body
                  is exactly as see-through as the window, never more. */}
              <span>Same as window</span>
            </div>
            <p className="mt-2.5 flex items-start gap-1.5 text-[11px] leading-snug text-content-faint">
              <Mail className="mt-px h-3 w-3 shrink-0" />
              <span>
                The body of the message you are reading. Its text sits right on the glass, so
                it can be held more opaque than the rest of the window — never less.
              </span>
            </p>
          </section>

          <SendAsSection />

          <TriageSection />
        </div>
      </div>
    </div>
  );
}
