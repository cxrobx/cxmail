import { useState, useEffect } from "react";
import { X, Plus, Trash2, ArrowLeft, ShieldAlert, RefreshCw, AlertTriangle } from "lucide-react";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { mailRuleWarnings, validateMailRule } from "@/lib/mailRuleValidation";
import type { MailRule, ReclassifyResult, RuleCondition, RuleAction } from "@/types/email";

interface Props {
  onClose: () => void;
}

// "body" is deliberately absent: rules are evaluated at classification time
// against headers only, so a body condition can never match (gotcha #36).
const FIELD_OPTIONS = [
  { value: "from", label: "From" },
  { value: "subject", label: "Subject" },
  { value: "to", label: "To" },
];

const OPERATOR_OPTIONS = [
  { value: "contains", label: "contains" },
  { value: "equals", label: "equals" },
  { value: "starts_with", label: "starts with" },
  { value: "ends_with", label: "ends with" },
];

const ACTION_OPTIONS: { value: string; label: string }[] = [
  { value: "set_category:junk", label: "Move to Junk" },
  { value: "set_category:primary", label: "Move to Primary" },
  { value: "set_category:updates", label: "Move to Updates" },
  { value: "set_category:social", label: "Move to Social" },
  { value: "set_category:promotions", label: "Move to Promotions" },
  { value: "mark_read", label: "Mark as read" },
  { value: "mark_flagged", label: "Star" },
];

function emptyRule(): MailRule {
  return {
    id: null,
    account_id: null,
    name: "",
    is_active: true,
    priority: 0,
    conditions: [{ field: "from", operator: "contains", value: "" }],
    actions: [{ action_type: "set_category", value: "junk" }],
  };
}

function encodeAction(a: RuleAction): string {
  if (a.action_type === "set_category" && a.value) return `set_category:${a.value}`;
  return a.action_type;
}

function decodeAction(encoded: string): RuleAction {
  if (encoded.startsWith("set_category:")) {
    return { action_type: "set_category", value: encoded.split(":")[1] ?? "junk" };
  }
  return { action_type: encoded, value: null };
}

export default function RulesManager({ onClose }: Props) {
  const [rules, setRules] = useState<MailRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<MailRule | null>(null);
  const [saving, setSaving] = useState(false);
  const [reclassifying, setReclassifying] = useState(false);
  const [reclassifyResult, setReclassifyResult] = useState<ReclassifyResult | null>(null);
  const [errors, setErrors] = useState<string[]>([]);

  const load = async () => {
    setLoading(true);
    try {
      const list = await api.rules.list();
      setRules(list);
    } catch (e) {
      console.error("Failed to load rules:", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const startNew = () => {
    setErrors([]);
    setEditing(emptyRule());
  };
  const startEdit = (r: MailRule) => {
    setErrors([]);
    setEditing({ ...r });
  };

  // Any edit invalidates the previously shown refusal.
  const patchEditing = (patch: Partial<MailRule>) => {
    if (!editing) return;
    setErrors([]);
    setEditing({ ...editing, ...patch });
  };

  const handleReclassify = async () => {
    setReclassifying(true);
    setReclassifyResult(null);
    try {
      const result = await api.rules.reclassifyAll();
      setReclassifyResult(result);
    } catch (e) {
      console.error("Failed to reclassify messages:", e);
    } finally {
      setReclassifying(false);
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await api.rules.delete(id);
      await load();
    } catch (e) {
      console.error("Failed to delete rule:", e);
    }
  };

  const handleSave = async () => {
    if (!editing) return;
    // Refuse rules the classifier can't run — or would run against everything.
    // Mirrors validate_rule_* at the MCP boundary; see gotcha #36.
    const problems = validateMailRule(editing);
    if (problems.length > 0) {
      setErrors(problems);
      return;
    }
    setErrors([]);

    setSaving(true);
    try {
      const payload: MailRule = { ...editing };
      if (editing.id === null) {
        await api.rules.create(payload);
      } else {
        await api.rules.update(payload);
      }
      setEditing(null);
      await load();
    } catch (e) {
      console.error("Failed to save rule:", e);
    } finally {
      setSaving(false);
    }
  };

  const updateCondition = (index: number, patch: Partial<RuleCondition>) => {
    if (!editing) return;
    patchEditing({
      conditions: editing.conditions.map((c, i) => (i === index ? { ...c, ...patch } : c)),
    });
  };

  const addCondition = () => {
    if (!editing) return;
    patchEditing({
      conditions: [...editing.conditions, { field: "from", operator: "contains", value: "" }],
    });
  };

  const removeCondition = (index: number) => {
    if (!editing) return;
    patchEditing({
      conditions: editing.conditions.filter((_, i) => i !== index),
    });
  };

  const updateAction = (index: number, encoded: string) => {
    if (!editing) return;
    patchEditing({
      actions: editing.actions.map((a, i) => (i === index ? decodeAction(encoded) : a)),
    });
  };

  const addAction = () => {
    if (!editing) return;
    patchEditing({
      actions: [...editing.actions, { action_type: "set_category", value: "junk" }],
    });
  };

  const removeAction = (index: number) => {
    if (!editing) return;
    patchEditing({
      actions: editing.actions.filter((_, i) => i !== index),
    });
  };

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-overlay"
      onClick={onClose}
    >
      <div
        className="w-[560px] max-h-[90vh] overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border-subtle bg-surface px-4 py-3 shrink-0">
          <div className="flex items-center gap-2">
            {editing && (
              <button
                onClick={() => setEditing(null)}
                className="text-content-muted hover:text-content"
              >
                <ArrowLeft className="h-4 w-4" />
              </button>
            )}
            <h2 className="text-sm font-medium text-content">
              {editing
                ? editing.id === null
                  ? "New Rule"
                  : "Edit Rule"
                : "Mail Rules"}
            </h2>
          </div>
          <button onClick={onClose} className="text-content-muted hover:text-content">
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto">
          {!editing && (
            <div className="px-4 py-4">
              <div className="mb-3 flex items-center justify-between">
                <p className="text-xs text-content-muted">
                  Rules run during sync. Use them to auto-route known spam senders
                  to Junk, or flag/read messages matching patterns.
                </p>
                <button
                  onClick={startNew}
                  className="flex shrink-0 items-center gap-1 rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-content hover:bg-accent-hover"
                >
                  <Plus className="h-3 w-3" /> New
                </button>
              </div>

              {/* Backfill: apply current rules + junk detector to existing messages */}
              <div className="mb-3 rounded-md border border-border-subtle bg-surface p-2.5">
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <div className="text-xs font-medium text-content">
                      Reclassify existing messages
                    </div>
                    <div className="text-[11px] text-content-muted">
                      Runs the junk detector and active rules against every
                      message already in your inbox. Leaves manual overrides
                      alone.
                    </div>
                  </div>
                  <button
                    onClick={handleReclassify}
                    disabled={reclassifying}
                    className="flex shrink-0 items-center gap-1 rounded-md bg-elevated px-2.5 py-1 text-xs font-medium text-content-secondary hover:bg-elevated hover:text-content disabled:opacity-50"
                  >
                    <RefreshCw
                      className={cn("h-3 w-3", reclassifying && "animate-spin")}
                    />
                    {reclassifying ? "Running…" : "Run"}
                  </button>
                </div>
                {reclassifyResult && (
                  <div className="mt-2 text-[11px] text-content-muted">
                    Scanned <span className="text-content">{reclassifyResult.total}</span>
                    {" · "}
                    updated <span className="text-content">{reclassifyResult.changed}</span>
                    {" · "}
                    moved to Junk <span className="text-content">{reclassifyResult.junk}</span>
                    {reclassifyResult.skipped_user_override > 0 && (
                      <>
                        {" · "}
                        kept <span className="text-content">{reclassifyResult.skipped_user_override}</span> user overrides
                      </>
                    )}
                  </div>
                )}
              </div>

              {loading ? (
                <p className="text-xs text-content-muted">Loading…</p>
              ) : rules.length === 0 ? (
                <div className="rounded-md border border-border-subtle bg-surface p-4 text-center">
                  <ShieldAlert className="mx-auto mb-2 h-5 w-5 text-content-muted" />
                  <p className="text-xs text-content-muted">
                    No rules yet. Create one to automatically route or flag
                    messages.
                  </p>
                </div>
              ) : (
                <div className="space-y-1.5">
                  {rules.map((r) => {
                    // Rules stored before this guard (or written by another
                    // client) can be dead or dangerously broad — say so
                    // instead of showing them as plain "active".
                    const warnings = mailRuleWarnings(r);
                    return (
                    <div
                      key={r.id ?? 0}
                      className="rounded-md border border-border-subtle bg-surface px-3 py-2 hover:border-border"
                    >
                     <div className="flex items-center gap-2">
                      <div
                        className={cn(
                          "h-1.5 w-1.5 shrink-0 rounded-full",
                          r.is_active ? "bg-green-500" : "bg-content-muted"
                        )}
                      />
                      <button
                        onClick={() => startEdit(r)}
                        className="min-w-0 flex-1 text-left"
                      >
                        <div className="truncate text-sm text-content">{r.name}</div>
                        <div className="truncate text-xs text-content-muted">
                          {r.conditions.length} condition
                          {r.conditions.length !== 1 ? "s" : ""} ·{" "}
                          {r.actions.map((a) => encodeAction(a)).join(", ")}
                        </div>
                      </button>
                      <button
                        onClick={() => r.id !== null && handleDelete(r.id)}
                        className="shrink-0 text-content-muted hover:text-error"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                     </div>
                     {warnings.length > 0 && (
                       <ul
                         data-testid="rule-warnings"
                         className="mt-1.5 space-y-1 border-t border-border-subtle pt-1.5 text-[11px] text-warning"
                       >
                         {warnings.map((w, i) => (
                           <li key={i} className="flex items-start gap-1.5">
                             <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                             <span>{w}</span>
                           </li>
                         ))}
                       </ul>
                     )}
                    </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {editing && (
            <div className="space-y-4 px-4 py-4">
              {/* Name + active */}
              <div>
                <label className="mb-1 block text-xs font-medium text-content-secondary">
                  Name
                </label>
                <input
                  type="text"
                  value={editing.name}
                  onChange={(e) => patchEditing({ name: e.target.value })}
                  placeholder="e.g. Block crypto spam"
                  className="w-full rounded-md border border-border bg-surface px-3 py-2 text-sm text-content placeholder:text-content-muted focus:border-accent focus:outline-none"
                  autoFocus
                />
              </div>

              <label className="flex cursor-pointer items-center gap-2">
                <input
                  type="checkbox"
                  checked={editing.is_active}
                  onChange={(e) => patchEditing({ is_active: e.target.checked })}
                  className="h-3.5 w-3.5 rounded border-border accent-accent"
                />
                <span className="text-xs text-content-secondary">Active</span>
              </label>

              {/* Conditions */}
              <div>
                <div className="mb-1.5 flex items-center justify-between">
                  <label className="text-xs font-medium text-content-secondary">
                    When <span className="font-normal text-content-muted">(all must match)</span>
                  </label>
                  <button
                    onClick={addCondition}
                    className="flex items-center gap-1 text-xs text-accent hover:text-accent-hover"
                  >
                    <Plus className="h-3 w-3" /> Add
                  </button>
                </div>
                <div className="space-y-2">
                  {editing.conditions.map((c, i) => (
                    <div key={i} className="flex items-center gap-1.5">
                      <select
                        value={c.field}
                        onChange={(e) => updateCondition(i, { field: e.target.value })}
                        className="rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content"
                      >
                        {/* A rule stored before this guard may hold a field we
                            no longer offer ("body", or something unknown).
                            Surface it rather than letting the select silently
                            display the wrong option; saving stays blocked
                            until it's changed. */}
                        {!FIELD_OPTIONS.some((f) => f.value === c.field) && (
                          <option value={c.field}>
                            {c.field || "(blank)"} — never matches
                          </option>
                        )}
                        {FIELD_OPTIONS.map((f) => (
                          <option key={f.value} value={f.value}>
                            {f.label}
                          </option>
                        ))}
                      </select>
                      <select
                        value={c.operator}
                        onChange={(e) => updateCondition(i, { operator: e.target.value })}
                        className="rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content"
                      >
                        {OPERATOR_OPTIONS.map((o) => (
                          <option key={o.value} value={o.value}>
                            {o.label}
                          </option>
                        ))}
                      </select>
                      <input
                        type="text"
                        value={c.value}
                        onChange={(e) => updateCondition(i, { value: e.target.value })}
                        placeholder="value"
                        className="min-w-0 flex-1 rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content placeholder:text-content-muted focus:border-accent focus:outline-none"
                      />
                      {editing.conditions.length > 1 && (
                        <button
                          onClick={() => removeCondition(i)}
                          className="shrink-0 text-content-muted hover:text-error"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>

              {/* Actions */}
              <div>
                <div className="mb-1.5 flex items-center justify-between">
                  <label className="text-xs font-medium text-content-secondary">
                    Then
                  </label>
                  <button
                    onClick={addAction}
                    className="flex items-center gap-1 text-xs text-accent hover:text-accent-hover"
                  >
                    <Plus className="h-3 w-3" /> Add
                  </button>
                </div>
                <div className="space-y-2">
                  {editing.actions.map((a, i) => (
                    <div key={i} className="flex items-center gap-1.5">
                      <select
                        value={encodeAction(a)}
                        onChange={(e) => updateAction(i, e.target.value)}
                        className="min-w-0 flex-1 rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content"
                      >
                        {ACTION_OPTIONS.map((o) => (
                          <option key={o.value} value={o.value}>
                            {o.label}
                          </option>
                        ))}
                      </select>
                      {editing.actions.length > 1 && (
                        <button
                          onClick={() => removeAction(i)}
                          className="shrink-0 text-content-muted hover:text-error"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Footer (only in edit mode) */}
        {editing && (
          <div className="border-t border-border-subtle px-4 py-3 shrink-0">
            {errors.length > 0 && (
              <div
                role="alert"
                data-testid="rule-errors"
                className="mb-3 rounded-md border border-error/40 bg-error/10 p-2.5"
              >
                <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-error">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                  This rule can't be saved
                </div>
                <ul className="space-y-1 pl-5 text-[11px] text-content-secondary list-disc">
                  {errors.map((e, i) => (
                    <li key={i}>{e}</li>
                  ))}
                </ul>
              </div>
            )}
            <div className="flex items-center justify-end gap-2">
            <button
              onClick={() => setEditing(null)}
              className="rounded-md bg-elevated px-4 py-2 text-sm text-content-secondary hover:bg-elevated"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-content hover:bg-accent-hover disabled:opacity-50"
            >
              {saving ? "Saving…" : editing.id === null ? "Create" : "Save"}
            </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
