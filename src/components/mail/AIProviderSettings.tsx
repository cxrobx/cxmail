import { useEffect, useState } from "react";
import {
  api,
  type AIProvider,
  type AIProviderSettings as Settings,
  type WriterModel,
  type WriterSettings,
} from "@/lib/tauri";
import { Check, Loader2, X } from "lucide-react";

const defaults: Record<AIProvider, Pick<Settings, "model" | "baseUrl">> = {
  openai: { model: "gpt-5.4-mini", baseUrl: "https://api.openai.com/v1" },
  anthropic: { model: "claude-sonnet-5", baseUrl: "https://api.anthropic.com/v1" },
  compatible: { model: "llama3.2", baseUrl: "http://127.0.0.1:11434/v1" },
};

interface AIProviderSettingsProps {
  onClose: () => void;
  onReady?: () => void;
}

export default function AIProviderSettings({ onClose, onReady }: AIProviderSettingsProps) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api.ai.getProviderSettings().then(setSettings).catch((error) => setMessage(String(error)));
  }, []);

  const selectProvider = (provider: AIProvider) => {
    if (!settings) return;
    setSettings({
      ...settings,
      provider,
      ...defaults[provider],
      apiKeyConfigured: false,
      maskedApiKey: null,
    });
    setApiKey("");
    setMessage(null);
  };

  const saveAndTest = async () => {
    if (!settings) return;
    setBusy(true);
    setMessage(null);
    try {
      const saved = await api.ai.saveProviderSettings({
        provider: settings.provider,
        model: settings.model,
        baseUrl: settings.baseUrl,
        apiKey: apiKey.trim() || null,
      });
      setSettings(saved);
      await api.ai.testProvider();
      setMessage("Connected");
      setApiKey("");
      onReady?.();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!settings) {
    return (
      <div className="flex items-center gap-2 p-4 text-sm text-content-muted">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading AI settings…
      </div>
    );
  }

  const keyRequired = settings.provider !== "compatible";

  return (
    <div className="border-t border-border-subtle bg-ai-surface px-4 py-3">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <div className="text-sm font-medium text-content">AI provider</div>
          <div className="text-xs text-content-muted">Your email is sent only to the provider you configure.</div>
        </div>
        <button onClick={onClose} aria-label="Close AI settings" className="rounded p-1 text-content-muted hover:text-content">
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="grid grid-cols-3 gap-2">
        {(["openai", "anthropic", "compatible"] as AIProvider[]).map((provider) => (
          <button
            key={provider}
            onClick={() => selectProvider(provider)}
            className={`rounded border px-2 py-1.5 text-xs capitalize ${settings.provider === provider ? "border-accent bg-accent/10 text-content" : "border-border text-content-muted hover:text-content"}`}
          >
            {provider === "compatible" ? "Local / compatible" : provider}
          </button>
        ))}
      </div>

      <div className="mt-3 grid gap-2">
        <label className="text-xs text-content-muted">
          Model
          <input
            value={settings.model}
            onChange={(event) => setSettings({ ...settings, model: event.target.value })}
            className="mt-1 w-full rounded bg-surface px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
        <label className="text-xs text-content-muted">
          Base URL
          <input
            value={settings.baseUrl}
            onChange={(event) => setSettings({ ...settings, baseUrl: event.target.value })}
            className="mt-1 w-full rounded bg-surface px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
        <label className="text-xs text-content-muted">
          API key {keyRequired ? "" : "(optional for local servers)"}
          <input
            type="password"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            placeholder={settings.maskedApiKey ?? (keyRequired ? "Paste API key" : "No key required")}
            className="mt-1 w-full rounded bg-surface px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
      </div>

      {message && (
        <div className={`mt-2 flex items-center gap-1.5 text-xs ${message === "Connected" ? "text-green-400" : "text-red-400"}`}>
          {message === "Connected" && <Check className="h-3.5 w-3.5" />}
          {message}
        </div>
      )}

      <button
        onClick={saveAndTest}
        disabled={busy || !settings.model.trim() || !settings.baseUrl.trim() || (keyRequired && !settings.apiKeyConfigured && !apiKey.trim())}
        className="mt-3 flex items-center gap-2 rounded bg-accent px-3 py-1.5 text-sm font-medium text-content hover:bg-accent-hover disabled:opacity-50"
      >
        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
        {busy ? "Testing…" : "Save & test"}
      </button>

      <WriterModelSection />
    </div>
  );
}

/**
 * Default model for agent draft writing — what an MCP compose/edit call with
 * `instruction` uses when it omits `writer_model` (the per-call parameter
 * still overrides, by design: model choice must not REQUIRE the UI).
 *
 * The dropdown is populated live from `agy models`; when that fails (agy
 * missing, not signed in, offline) it degrades to a free-text id field
 * rather than a dead control.
 */
export function WriterModelSection() {
  const [writer, setWriter] = useState<WriterSettings | null>(null);
  const [models, setModels] = useState<WriterModel[] | null>(null);
  const [manualId, setManualId] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.ai.getWriterSettings().then(setWriter).catch((error) => setStatus(String(error)));
    api.ai
      .listWriterModels()
      .then(setModels)
      .catch(() => setModels(null));
  }, []);

  const save = async (model: string | null) => {
    setSaving(true);
    setStatus(null);
    try {
      const saved = await api.ai.saveWriterModel(model);
      setWriter(saved);
      setManualId("");
      setStatus(`Saved — drafts will use ${saved.effectiveModel}`);
    } catch (error) {
      setStatus(String(error));
    } finally {
      setSaving(false);
    }
  };

  if (!writer) return null;

  return (
    <div className="mt-4 border-t border-border-subtle pt-3">
      <div className="text-sm font-medium text-content">Draft writing (agent)</div>
      <div className="text-xs text-content-muted">
        Model used when Claude delegates email writing to the on-device writer (Antigravity CLI).
        A per-call writer_model still overrides this.
      </div>

      {models ? (
        <label className="mt-2 block text-xs text-content-muted">
          Model
          <select
            aria-label="Writer model"
            value={writer.model ?? ""}
            disabled={saving}
            onChange={(event) => save(event.target.value || null)}
            className="mt-1 w-full rounded bg-surface px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent disabled:opacity-50"
          >
            <option value="">Default ({writer.builtinDefault})</option>
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.label}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <div className="mt-2 flex items-end gap-2">
          <label className="grow text-xs text-content-muted">
            Model id (agy unavailable — enter one manually)
            <input
              aria-label="Writer model id"
              value={manualId}
              onChange={(event) => setManualId(event.target.value)}
              placeholder={writer.model ?? writer.builtinDefault}
              className="mt-1 w-full rounded bg-surface px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
            />
          </label>
          <button
            onClick={() => save(manualId.trim() || null)}
            disabled={saving}
            className="rounded bg-accent px-3 py-1.5 text-sm font-medium text-content hover:bg-accent-hover disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      )}

      {status && (
        <div className={`mt-2 flex items-center gap-1.5 text-xs ${status.startsWith("Saved") ? "text-green-400" : "text-red-400"}`}>
          {status.startsWith("Saved") && <Check className="h-3.5 w-3.5" />}
          {status}
        </div>
      )}
    </div>
  );
}
