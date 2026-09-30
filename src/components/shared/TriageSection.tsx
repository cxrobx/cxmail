import { useCallback, useEffect, useState } from "react";
import { Sparkles } from "lucide-react";
import { api } from "@/lib/tauri";
import type { Account, TriageStatus } from "@/types/email";
import { cn } from "@/lib/utils";

/**
 * The background AI triage pass, in the Settings dialog.
 *
 * It lives HERE and not in `AIProviderSettings`, which only renders as a setup
 * prompt when no provider is configured — a fine place to be told something is
 * missing and a bad place to discover a preference exists. Same reasoning that
 * moved appearance out of the command palette (gotcha #52).
 *
 * Three modes, and the middle one is the point: `shadow` classifies real mail
 * and records verdicts while changing nothing on screen, so thresholds can be
 * read against a real queue before anything acts on them. `on` is the only mode
 * where a verdict reaches Needs You, and even then it can only ADD a row.
 */
// Suggestions only — the field stays free text. A fixed dropdown would go
// stale the moment a model is released or retired, and the whole reason this
// control exists is that model availability changes outside our control: a
// hardcoded id that gets deprecated leaves triage silently dead with no
// recovery short of a rebuild. Reasoning effort has no such failure mode, which
// is why it is a constant in the backend and not a control here.
const MODEL_SUGGESTIONS = [
  "gpt-6-luna",
  "gpt-5.6-terra",
  "gpt-6.1-sol",
  "gpt-5.4-mini",
  "gpt-5.4-nano",
];

const MODES: { value: string; label: string; hint: string }[] = [
  { value: "off", label: "Off", hint: "Nothing is sent." },
  { value: "shadow", label: "Shadow", hint: "Classifies and records. Nothing changes on screen." },
  { value: "on", label: "On", hint: "Verdicts can add rows to Needs You. They never remove one." },
];

export default function TriageSection() {
  const [status, setStatus] = useState<TriageStatus | null>(null);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  // `Promise.resolve(...)` rather than calling `.then` on the result directly.
  // If the running backend has no `get_triage_status` — an older binary, a
  // partial upgrade, or a test harness that stubs invoke — the call returns
  // undefined, and a bare `.then` would throw during render and take the WHOLE
  // Settings dialog down with it. Theme and transparency have nothing to do
  // with triage and must not be reachable only when triage is.
  // Undefined status renders nothing, which is the honest fallback.
  const refresh = useCallback(() => {
    Promise.resolve(api.ai.getTriageStatus())
      .then((s) => setStatus(s ?? null))
      .catch((e) => setMessage(String(e)));
    Promise.resolve(api.accounts.list())
      .then((a) => setAccounts(a ?? []))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const run = async (fn: () => Promise<unknown>, after?: (r: unknown) => void) => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await fn();
      after?.(result);
      refresh();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status) return null;

  const active = MODES.find((m) => m.value === status.mode) ?? MODES[0];
  const enabledCount = accounts.filter((a) => a.triage_enabled).length;

  return (
    <section>
      <div className="mb-1.5 flex items-baseline justify-between">
        <h3 className="text-xs font-medium uppercase tracking-wider text-content-muted">
          Inbox triage
        </h3>
        <span className="tabular-nums text-[11px] text-content-muted">
          {status.verdicts} read · {status.pending} waiting
        </span>
      </div>

      <div className="flex gap-1 rounded-lg bg-surface p-1">
        {MODES.map(({ value, label }) => (
          <button
            key={value}
            onClick={() => run(() => api.ai.setTriageMode(value))}
            aria-pressed={status.mode === value}
            disabled={busy}
            className={cn(
              "flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50",
              status.mode === value
                ? "bg-elevated text-content shadow-sm"
                : "text-content-secondary hover:text-content",
            )}
          >
            {label}
          </button>
        ))}
      </div>

      <p className="mt-2.5 flex items-start gap-1.5 text-[11px] leading-snug text-content-faint">
        <Sparkles className="mt-px h-3 w-3 shrink-0" />
        <span>
          {active.hint} Reads the body of new mail on the accounts ticked below and judges whether
          someone is waiting on a reply, using the AI provider already configured for Reply with AI.
        </span>
      </p>

      {!status.provider_ready && status.mode !== "off" && (
        <p className="mt-2 text-[11px] leading-snug text-warning">
          No AI provider is configured, so the pass will do nothing until one is set up.
        </p>
      )}

      {accounts.length > 0 && (
        <div className="mt-3 overflow-hidden rounded-lg border border-border-subtle">
          <div className="flex items-baseline justify-between bg-surface px-2.5 py-1.5">
            <span className="text-[11px] font-medium text-content-secondary">Accounts</span>
            <span className="tabular-nums text-[11px] text-content-faint">
              {enabledCount} of {accounts.length}
            </span>
          </div>
          {/* Enabled first, then alphabetical. Nine identical rows in account
              order makes the one that matters hard to find; the ticked ones
              are the answer to "what is this doing", so they go on top. Capped
              height with a scroll so this section cannot push the mode control
              off the screen, which is exactly what the first version did. */}
          <div className="max-h-[132px] overflow-y-auto">
            {[...accounts]
              .sort(
                (a, b) =>
                  Number(b.triage_enabled) - Number(a.triage_enabled) ||
                  a.email.localeCompare(b.email),
              )
              .map((account) => (
                <label
                  key={account.id}
                  className={cn(
                    "flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-[11px] transition-colors hover:bg-surface",
                    account.triage_enabled ? "text-content" : "text-content-secondary",
                  )}
                >
                  <input
                    type="checkbox"
                    aria-label={`Triage ${account.email}`}
                    checked={account.triage_enabled}
                    disabled={busy}
                    onChange={(e) =>
                      run(() => api.ai.setAccountTriageEnabled(account.id, e.target.checked))
                    }
                    className="h-3 w-3 shrink-0 accent-accent"
                  />
                  <span className="truncate">{account.email}</span>
                </label>
              ))}
          </div>
        </div>
      )}

      {status.mode !== "off" && enabledCount === 0 && (
        <p className="mt-2 text-[11px] leading-snug text-warning">
          No accounts are ticked, so nothing will be read.
        </p>
      )}

      <label className="mt-3 block text-[11px] text-content-secondary">
        Model
        <input
          aria-label="Triage model"
          list="cx-triage-models"
          defaultValue={status.model}
          disabled={busy}
          onBlur={(e) => {
            if (e.target.value.trim() !== status.model) {
              run(() => api.ai.setTriageModel(e.target.value));
            }
          }}
          className="mt-1 w-full rounded bg-surface px-2 py-1 text-[11px] text-content outline-none ring-1 ring-border focus:ring-accent disabled:opacity-50"
        />
        <datalist id="cx-triage-models">
          {MODEL_SUGGESTIONS.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
      </label>

      <p className="mt-1.5 text-[11px] leading-snug text-content-faint">
        Separate from the model used for Reply with AI, so a model chosen to classify
        cheaply does not also write your drafts.
      </p>

      <div className="mt-3 flex items-center gap-2">
        <button
          onClick={() =>
            run(
              () => api.ai.runTriagePassNow(),
              (r) => {
                const s = r as { classified: number; withheld: number; failed: number; capped: boolean };
                setMessage(
                  s.capped
                    ? "Daily cap reached, nothing sent."
                    : `${s.classified} read, ${s.withheld} skipped, ${s.failed} failed.`,
                );
              },
            )
          }
          disabled={busy || status.mode === "off" || enabledCount === 0}
          className="rounded-md bg-surface px-3 py-1.5 text-xs font-medium text-content-secondary hover:text-content disabled:opacity-50"
        >
          {busy ? "Running…" : "Run a pass now"}
        </button>
        <span className="tabular-nums text-[11px] text-content-faint">
          {status.today}/{status.daily_cap} today
          {status.input_tokens > 0 && (
            <>
              {" · "}
              {((status.input_tokens + status.output_tokens) / 1000).toFixed(1)}k tokens
            </>
          )}
        </span>
      </div>

      {message && <p className="mt-2 text-[11px] text-content-muted">{message}</p>}
    </section>
  );
}
