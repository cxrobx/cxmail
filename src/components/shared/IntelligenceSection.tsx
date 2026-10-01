import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import type { EmbeddingSettings, IntelligenceAccount, IntelligenceStatus } from "@/types/email";

export default function IntelligenceSection() {
  const accounts = useAccountStore((s) => s.accounts);
  const [status, setStatus] = useState<IntelligenceStatus | null>(null);
  const [settings, setSettings] = useState<EmbeddingSettings>({ source: "local", model: "nomic-embed-text" });
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  useEffect(() => {
    let active = true;
    Promise.resolve(api.intelligence.status()).then((value) => {
      if (active && value) { setStatus(value); setSettings(value.embeddings); }
    }).catch((e) => { if (active) setNotice(String(e)); });
    return () => { active = false; };
  }, []);

  // While a backlog is being embedded, refresh the count so progress is visible.
  const indexing = !!status && status.pending > 0 && status.accounts.some((o) => o.semantic_search);
  useEffect(() => {
    if (!indexing) return;
    let active = true;
    const timer = window.setInterval(() => {
      Promise.resolve(api.intelligence.status()).then((value) => { if (active && value) setStatus(value); }).catch(() => {});
    }, 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [indexing]);

  const changeAccount = async (options: IntelligenceAccount) => {
    setBusy(true); setNotice("");
    try {
      await api.intelligence.setAccount(options);
      setStatus(await api.intelligence.status());
      window.dispatchEvent(new CustomEvent("cxmail:intelligence-updated"));
    } catch (e) { setNotice(String(e)); } finally { setBusy(false); }
  };
  const configure = async (test: boolean) => {
    setBusy(true); setNotice("");
    try {
      await api.intelligence.saveEmbeddings(settings);
      setNotice(test ? await api.intelligence.testEmbeddings() : "Embedding settings saved.");
      setStatus(await api.intelligence.status());
    } catch (e) { setNotice(String(e)); } finally { setBusy(false); }
  };
  const run = async () => {
    setBusy(true); setNotice("");
    try {
      const result = await api.intelligence.runPass();
      setNotice(result.busy ? "An indexing pass is already running." : `${result.drafted} replies prepared · ${result.embedded} passages indexed${result.capped ? " · daily limit reached" : ""}${result.failed ? ` · ${result.failed} requests failed; check your AI and embedding connection` : ""}`);
      setStatus(await api.intelligence.status());
      window.dispatchEvent(new CustomEvent("cxmail:intelligence-updated"));
    } catch (e) { setNotice(String(e)); } finally { setBusy(false); }
  };
  const button = "rounded border border-border px-2 py-1 text-xs text-content-secondary hover:bg-elevated disabled:opacity-50";

  return (
    <section className="space-y-3" aria-label="Mail intelligence">
      <h3 className="text-xs font-medium uppercase tracking-wider text-content-muted">Mail intelligence</h3>
      <p className="text-xs leading-relaxed text-content-secondary">Prepare replies in your writing style using your AI provider. Review them in Needs You or the message before sending. Sensitive content is withheld or redacted.</p>
      <p className="text-xs leading-relaxed text-content-secondary">Meaning search finds related messages and document passages. Choose which accounts can be indexed in the background.</p>
      <div className="space-y-2">
        {accounts.map((account) => {
          const options = status?.accounts.find((o) => o.account_id === account.id) ?? { account_id: account.id, auto_draft: false, semantic_search: false };
          return (
            <div key={account.id} className="rounded-lg bg-surface p-2.5">
              <div className="mb-1.5 truncate text-xs font-medium text-content">{account.email}</div>
              {account.hidden_from_aggregates && <p className="mb-1.5 text-[11px] text-content-muted">Hidden accounts are skipped by background preparation and indexing.</p>}
              <div className="flex flex-wrap gap-x-4 gap-y-1.5 text-xs text-content-secondary">
                <label className="flex items-center gap-1.5"><input type="checkbox" checked={options.auto_draft} disabled={busy || !status} onChange={(e) => void changeAccount({ ...options, auto_draft: e.target.checked })} aria-label={`Prepare replies for ${account.email}`} />Prepare replies</label>
                <label className="flex items-center gap-1.5"><input type="checkbox" checked={options.semantic_search} disabled={busy || !status} onChange={(e) => void changeAccount({ ...options, semantic_search: e.target.checked })} aria-label={`Meaning search for ${account.email}`} />Meaning search</label>
              </div>
            </div>
          );
        })}
      </div>
      <label className="block text-xs text-content-secondary">Embedding service
        <select className="mt-1 w-full rounded border border-border bg-input p-1.5 text-content" value={settings.source} disabled={busy} onChange={(e) => setSettings({ source: e.target.value as EmbeddingSettings["source"], model: e.target.value === "local" ? "nomic-embed-text" : "text-embedding-3-small" })}>
          <option value="local">Local Ollama</option><option value="provider">My OpenAI-compatible provider</option>
        </select>
      </label>
      <p className="text-[11px] leading-relaxed text-content-muted">{settings.source === "local" ? "Requires Ollama running with the selected model installed. Message contents stay on this computer during indexing." : "Indexing sends redacted passages to your configured AI provider. Anthropic does not offer this embedding endpoint."} Ask mail sends the retrieved passages to your AI provider when you click it.</p>
      <label className="block text-xs text-content-secondary">Embedding model<input className="mt-1 w-full rounded border border-border bg-input px-2 py-1.5 text-content" value={settings.model} disabled={busy} onChange={(e) => setSettings({ ...settings, model: e.target.value })} /></label>
      <div className="flex flex-wrap gap-2">
        <button className={button} disabled={busy || !settings.model.trim()} onClick={() => void configure(false)}>Save</button>
        <button className={button} disabled={busy || !settings.model.trim()} onClick={() => void configure(true)}>Save & test</button>
        <button className={button} disabled={busy || !status} onClick={() => void run()}>{busy ? "Working…" : "Prepare & index now"}</button>
      </div>
      {status && <p className="text-[11px] text-content-muted">{status.embedded.toLocaleString()} of {status.chunks.toLocaleString()} passages indexed for meaning search{indexing ? `, ${status.pending.toLocaleString()} still to index (newest mail first)` : ""}. Local indexing has no daily limit; a remote provider indexes up to 800 passages a day. Up to 40 replies are prepared per day.</p>}
      {notice && <p role="status" className="text-xs text-content-secondary">{notice}</p>}
    </section>
  );
}
