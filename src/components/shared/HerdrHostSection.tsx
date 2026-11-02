import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/tauri";

/**
 * Which Mac's Herdr takes Claude handoffs. Blank is this Mac, so one build works
 * on a Mac running Herdr itself and on one sending its sessions elsewhere.
 *
 * It saves on blur and Enter, like the Onyx address. What comes back is what was
 * saved, trimmed, and a value the backend refuses stays in the field with the
 * reason beside it, so it can be fixed rather than retyped.
 */
export default function HerdrHostSection() {
  const [draft, setDraft] = useState("");
  const saved = useRef("");
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const host = await api.system.getHerdrHost();
        if (!cancelled) {
          saved.current = host;
          setDraft(host);
          setLoaded(true);
        }
      } catch (e) {
        if (!cancelled) setError("Could not load the Herdr host. Reopen Settings to retry.");
        void api.system.logClientError("herdr-host", String(e));
      }
    }
    void load();
    return () => { cancelled = true; };
  }, []);

  async function save() {
    if (!loaded || draft === saved.current) return;
    setError(null);
    try {
      const host = await api.system.setHerdrHost(draft);
      saved.current = host;
      setDraft(host);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      void api.system.logClientError("herdr-host", String(e));
    }
  }

  return (
    <section>
      <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
        Claude handoff
      </h3>
      <label className="block">
        <span className="text-xs text-content">Herdr host</span>
        <input
          value={draft}
          disabled={!loaded}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => void save()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
          }}
          placeholder="This Mac"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          aria-describedby="herdr-host-description"
          className="mt-1 w-full rounded-md border border-border bg-input px-2 py-1 font-mono text-[11px] text-content outline-none focus:border-accent disabled:opacity-50"
        />
      </label>
      <p id="herdr-host-description" className="mt-1 text-[11px] text-content-secondary">
        Leave blank to use Herdr on this Mac, or enter an SSH target such as user@host to open
        Claude sessions there.
      </p>
      {error && <p role="alert" className="mt-2 text-[11px] text-red-400">{error}</p>}
    </section>
  );
}
