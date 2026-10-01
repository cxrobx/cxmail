import { useEffect, useRef, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { api } from "@/lib/tauri";
import type { ReplySuggestion } from "@/types/email";

interface Props { accountId: string; folder: string; uid: number; onUse: (body: string) => void; disabled?: boolean }
export default function ReplySuggestionCard({ accountId, folder, uid, onUse, disabled }: Props) {
  const identity = `${accountId}|${folder}|${uid}`;
  const currentIdentity = useRef(identity);
  currentIdentity.current = identity;
  const [ready, setReady] = useState<{ identity: string; suggestion: ReplySuggestion } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true, version = 0;
    setReady(null); setError(""); setBusy(false);
    const load = () => { const request = ++version; return Promise.resolve(api.intelligence.suggestion(accountId, folder, uid)).then((suggestion) => {
      if (active && request === version) setReady(suggestion ? { identity, suggestion } : null);
    }).catch(() => { if (active && request === version) setReady(null); }); };
    void load();
    window.addEventListener("cxmail:intelligence-updated", load);
    window.addEventListener("cxmail:refresh-messages", load);
    return () => { active = false; window.removeEventListener("cxmail:intelligence-updated", load); window.removeEventListener("cxmail:refresh-messages", load); };
  }, [accountId, folder, uid, identity]);
  if (!ready || ready.identity !== identity) return null;
  const use = async () => {
    setBusy(true); setError("");
    try {
      const current = await api.intelligence.suggestion(accountId, folder, uid);
      if (currentIdentity.current !== identity) return;
      if (!current) { setReady(null); return; }
      onUse(current.body);
    } catch (e) { if (currentIdentity.current === identity) setError(String(e)); } finally { if (currentIdentity.current === identity) setBusy(false); }
  };
  const dismiss = async () => {
    setBusy(true);
    try { await api.intelligence.dismissSuggestion(accountId, folder, uid); if (currentIdentity.current === identity) setReady(null); window.dispatchEvent(new CustomEvent("cxmail:intelligence-updated")); }
    catch (e) { if (currentIdentity.current === identity) setError(String(e)); } finally { if (currentIdentity.current === identity) setBusy(false); }
  };
  return (
    <div className="shrink-0 border-b border-border-subtle bg-accent/5 px-4 py-3">
      <div className="flex items-center gap-2 text-xs font-medium text-content"><Sparkles className="h-3.5 w-3.5 text-accent" />Suggested reply
        <button className="ml-auto rounded p-1 text-content-muted hover:bg-elevated" aria-label="Dismiss suggested reply" disabled={busy} onClick={() => void dismiss()}><X className="h-3.5 w-3.5" /></button>
      </div>
      <p className="my-2 max-h-28 overflow-auto whitespace-pre-wrap text-xs leading-relaxed text-content-secondary">{ready.suggestion.body}</p>
      <div className="flex flex-wrap items-center gap-2">
        <button disabled={busy || disabled} onClick={() => void use()} className="rounded bg-accent px-2.5 py-1.5 text-xs text-white disabled:opacity-50">Review reply</button>
        <span className="text-[11px] text-content-muted">{disabled ? "Finish your current reply first." : "Check facts and fill placeholders before sending."}</span>
      </div>
      {error && <p role="alert" className="mt-2 text-xs text-red-400">{error}</p>}
    </div>
  );
}
