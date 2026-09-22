import { useEffect, useState } from "react";
import { api, type ZoomStatus } from "@/lib/tauri";
import { AlertTriangle, Check, Loader2, X } from "lucide-react";

interface ZoomSettingsProps {
  onClose: () => void;
}

/**
 * Credentials panel for the Zoom Server-to-Server OAuth app.
 *
 * Same save-then-test flow and styling as `AIProviderSettings`. There is
 * deliberately no "create with Zoom" control here — Zoom is opt-in per MCP call
 * (`create_calendar_event` with `conference: "zoom"`), and the "Add Google Meet"
 * checkbox in `EventComposeModal` is untouched. This panel exists only so the
 * credentials have somewhere to be entered, and so a stuck link is visible
 * without reading a log.
 */
export default function ZoomSettings({ onClose }: ZoomSettingsProps) {
  const [status, setStatus] = useState<ZoomStatus | null>(null);
  const [accountId, setAccountId] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [ok, setOk] = useState(false);

  useEffect(() => {
    api.zoom
      .status()
      .then(setStatus)
      .catch((error) => setMessage(String(error)));
  }, []);

  const saveAndTest = async () => {
    setBusy(true);
    setMessage(null);
    setOk(false);
    try {
      const saved = await api.zoom.save({
        accountId: accountId.trim(),
        clientId: clientId.trim(),
        clientSecret: clientSecret.trim(),
      });
      setStatus(saved);
      // The granted-scope string is the whole point of the test: an app whose
      // scopes were never added authenticates fine and then fails every meeting
      // call, which is indistinguishable from a bad secret without this.
      setMessage(await api.zoom.test());
      setOk(true);
      setAccountId("");
      setClientId("");
      setClientSecret("");
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setMessage(null);
    setOk(false);
    try {
      setMessage(await api.zoom.test());
      setOk(true);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    setMessage(null);
    setOk(false);
    try {
      setStatus(await api.zoom.clear());
      setMessage("Zoom credentials removed");
      setOk(true);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!status) {
    return (
      <div className="flex items-center gap-2 p-4 text-sm text-content-muted">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading Zoom settings…
      </div>
    );
  }

  const complete = accountId.trim() && clientId.trim() && clientSecret.trim();
  const attention = status.orphanCount > 0 || status.unverifiedCount > 0;

  return (
    <div className="border-b border-border-subtle bg-elevated px-4 py-3">
      <div className="mb-3 flex items-start justify-between">
        <div>
          <div className="text-sm font-medium text-content">Zoom meetings</div>
          <div className="text-xs text-content-muted">
            Lets Claude back a calendar event with a real Zoom meeting — for screen shares that
            need computer audio, which Google Meet cannot carry on macOS.
          </div>
        </div>
        <button
          onClick={onClose}
          aria-label="Close Zoom settings"
          className="rounded p-1 text-content-muted hover:text-content"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {status.configured && (
        <div className="mb-3 flex items-center gap-2 text-xs text-content-secondary">
          <Check className="h-3.5 w-3.5 text-success" />
          Connected as account {status.maskedAccountId}
        </div>
      )}

      <div className="grid gap-2">
        <label className="text-xs text-content-muted">
          Account ID
          <input
            value={accountId}
            onChange={(event) => setAccountId(event.target.value)}
            placeholder={status.configured ? status.maskedAccountId ?? "" : "From your Zoom app"}
            className="mt-1 w-full rounded bg-input px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
        <label className="text-xs text-content-muted">
          Client ID
          <input
            value={clientId}
            onChange={(event) => setClientId(event.target.value)}
            placeholder={status.configured ? "•••••••• (stored)" : "From your Zoom app"}
            className="mt-1 w-full rounded bg-input px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
        <label className="text-xs text-content-muted">
          Client Secret
          <input
            type="password"
            value={clientSecret}
            onChange={(event) => setClientSecret(event.target.value)}
            placeholder={status.configured ? "•••••••• (stored)" : "From your Zoom app"}
            className="mt-1 w-full rounded bg-input px-3 py-1.5 text-sm text-content outline-none ring-1 ring-border focus:ring-accent"
          />
        </label>
      </div>

      <div className="mt-2 text-[11px] leading-relaxed text-content-muted">
        Create a <span className="text-content-secondary">Server-to-Server OAuth</span> app at
        marketplace.zoom.us, add the{" "}
        <code className="text-content-secondary">meeting:write</code>,{" "}
        <code className="text-content-secondary">meeting:update</code> and{" "}
        <code className="text-content-secondary">meeting:delete</code> scopes, then{" "}
        <span className="text-content-secondary">activate</span> it. Zoom uses granular scope
        names, so what the Scopes tab offers is{" "}
        <code className="text-content-secondary">meeting:write:meeting:admin</code> and friends —
        pick the <code className="text-content-secondary">:admin</code> variants, not{" "}
        <code className="text-content-secondary">:master</code> (that one is for master accounts
        managing sub-accounts). Test reports back what Zoom actually granted.
      </div>

      {message && (
        <div
          className={`mt-2 flex items-start gap-1.5 text-xs ${ok ? "text-success" : "text-error"}`}
        >
          {ok && <Check className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
          <span className="break-words">{message}</span>
        </div>
      )}

      {attention && (
        <div className="mt-3 flex items-start gap-2 rounded border border-warning/30 bg-warning/10 px-2.5 py-2 text-xs text-warning">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            {status.orphanCount > 0 && (
              <>
                {status.orphanCount} Zoom meeting{status.orphanCount === 1 ? "" : "s"} no longer
                have a calendar event.{" "}
              </>
            )}
            {status.unverifiedCount > 0 && (
              <>
                {status.unverifiedCount} could not be verified.{" "}
              </>
            )}
            None of these are deleted automatically — check them at zoom.us. Deleting a past
            meeting would destroy its recording and attendance report.
          </span>
        </div>
      )}

      <div className="mt-3 flex items-center gap-2">
        <button
          onClick={saveAndTest}
          disabled={busy || !complete}
          className="flex items-center gap-2 rounded bg-accent px-3 py-1.5 text-sm font-medium text-content hover:bg-accent-hover disabled:opacity-50"
        >
          {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          {busy ? "Testing…" : "Save & test"}
        </button>
        {status.configured && (
          <>
            <button
              onClick={test}
              disabled={busy}
              className="rounded border border-border px-3 py-1.5 text-sm text-content-secondary hover:text-content disabled:opacity-50"
            >
              Test
            </button>
            <button
              onClick={clear}
              disabled={busy}
              className="rounded px-3 py-1.5 text-sm text-content-muted hover:text-error disabled:opacity-50"
            >
              Remove
            </button>
          </>
        )}
      </div>
    </div>
  );
}
