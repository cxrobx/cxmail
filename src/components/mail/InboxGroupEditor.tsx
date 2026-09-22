import { useState, useEffect } from "react";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useAccountStore } from "@/stores/accountStore";
import {
  X, Plus, Trash2,
  Briefcase, Music, User, Folder, Tag, Mail, Star, Heart, Zap, Globe,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

const ICON_OPTIONS: { value: string; icon: LucideIcon }[] = [
  { value: "briefcase", icon: Briefcase },
  { value: "music", icon: Music },
  { value: "user", icon: User },
  { value: "folder", icon: Folder },
  { value: "tag", icon: Tag },
  { value: "mail", icon: Mail },
  { value: "star", icon: Star },
  { value: "heart", icon: Heart },
  { value: "zap", icon: Zap },
  { value: "globe", icon: Globe },
];

const COLOR_OPTIONS = [
  "#0a84ff", "#ff375f", "#30d158", "#ff9f0a", "#bf5af2",
  "#64d2ff", "#ff6482", "#ffd60a", "#ac8e68", "#98989d",
];

const FIELD_OPTIONS = [
  { value: "from_email", label: "From (email)" },
  { value: "from_name", label: "From (name)" },
  { value: "subject", label: "Subject" },
  { value: "to_list", label: "To" },
];

const OPERATOR_OPTIONS = [
  { value: "contains", label: "contains" },
  { value: "equals", label: "equals" },
  { value: "starts_with", label: "starts with" },
  { value: "ends_with", label: "ends with" },
];

interface RuleRow {
  field: string;
  operator: string;
  value: string;
}

interface Props {
  groupId: number | null; // null = create new
  onClose: () => void;
  onSaved: () => void;
}

export default function InboxGroupEditor({ groupId, onClose, onSaved }: Props) {
  const { inboxGroups } = useMailStore();
  const { accounts } = useAccountStore();
  const [name, setName] = useState("");
  const [color, setColor] = useState(COLOR_OPTIONS[0]);
  const [icon, setIcon] = useState("folder");
  const [selectedAccountIds, setSelectedAccountIds] = useState<Set<string>>(new Set());
  const [rules, setRules] = useState<RuleRow[]>([]);
  const [saving, setSaving] = useState(false);

  // Load existing group data
  useEffect(() => {
    if (groupId !== null) {
      const group = inboxGroups.find((g) => g.id === groupId);
      if (group) {
        setName(group.name);
        setColor(group.color);
        setIcon(group.icon);
        setSelectedAccountIds(new Set(group.account_ids));
        setRules(
          group.rules.map((r) => ({ field: r.field, operator: r.operator, value: r.value }))
        );
      }
    }
  }, [groupId, inboxGroups]);

  const toggleAccount = (accountId: string) => {
    setSelectedAccountIds((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  };

  const addRule = () => {
    setRules([...rules, { field: "from_email", operator: "contains", value: "" }]);
  };

  const removeRule = (index: number) => {
    setRules(rules.filter((_, i) => i !== index));
  };

  const updateRule = (index: number, updates: Partial<RuleRow>) => {
    setRules(rules.map((r, i) => (i === index ? { ...r, ...updates } : r)));
  };

  const handleSave = async () => {
    if (!name.trim()) return;
    const validRules = rules.filter((r) => r.value.trim());
    const accountIds = Array.from(selectedAccountIds);
    setSaving(true);
    try {
      if (groupId !== null) {
        await api.inboxGroups.update(groupId, name.trim(), color, icon, validRules, accountIds);
      } else {
        await api.inboxGroups.create(name.trim(), color, icon, validRules, accountIds);
      }
      onSaved();
    } catch (e) {
      console.error("Failed to save inbox group:", e);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (groupId === null) return;
    try {
      await api.inboxGroups.delete(groupId);
      onSaved();
    } catch (e) {
      console.error("Failed to delete inbox group:", e);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-overlay"
      onClick={onClose}
    >
      <div
        className="w-[480px] max-h-[90vh] overflow-y-auto overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border-subtle bg-surface px-4 py-3">
          <h2 className="text-sm font-medium text-content">
            {groupId !== null ? "Edit Group" : "New Inbox Group"}
          </h2>
          <button onClick={onClose} className="text-content-muted hover:text-content">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4 px-4 py-4">
          {/* Name */}
          <div>
            <label className="mb-1 block text-xs font-medium text-content-secondary">Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Business, Music, Personal"
              className="w-full rounded-md border border-border bg-surface px-3 py-2 text-sm text-content placeholder:text-content-muted focus:border-accent focus:outline-none"
              autoFocus
            />
          </div>

          {/* Color + Icon row */}
          <div className="flex gap-4">
            <div className="flex-1">
              <label className="mb-1 block text-xs font-medium text-content-secondary">Color</label>
              <div className="flex flex-wrap gap-1.5">
                {COLOR_OPTIONS.map((c) => (
                  <button
                    key={c}
                    onClick={() => setColor(c)}
                    className={cn(
                      "h-6 w-6 rounded-full border-2 transition-transform",
                      color === c ? "border-content scale-110" : "border-transparent"
                    )}
                    style={{ backgroundColor: c }}
                  />
                ))}
              </div>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-content-secondary">Icon</label>
              <div className="flex flex-wrap gap-1">
                {ICON_OPTIONS.map(({ value, icon: Icon }) => (
                  <button
                    key={value}
                    onClick={() => setIcon(value)}
                    className={cn(
                      "rounded-md p-1.5 transition-colors",
                      icon === value
                        ? "bg-accent/20 text-accent"
                        : "text-content-muted hover:bg-surface hover:text-content"
                    )}
                  >
                    <Icon className="h-4 w-4" />
                  </button>
                ))}
              </div>
            </div>
          </div>

          {/* Accounts */}
          <div>
            <label className="mb-1.5 block text-xs font-medium text-content-secondary">
              Accounts <span className="font-normal text-content-muted">(include all mail from)</span>
            </label>
            <div className="space-y-1">
              {accounts.map((account) => (
                <label
                  key={account.id}
                  className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 hover:bg-surface"
                >
                  <input
                    type="checkbox"
                    checked={selectedAccountIds.has(account.id)}
                    onChange={() => toggleAccount(account.id)}
                    className="h-3.5 w-3.5 rounded border-border accent-accent"
                  />
                  <span
                    className="h-2 w-2 shrink-0 rounded-full"
                    style={{ backgroundColor: account.color || "#0a84ff" }}
                  />
                  <span className="text-sm text-content">{account.email}</span>
                  {account.display_name && (
                    <span className="text-xs text-content-muted">({account.display_name})</span>
                  )}
                  {account.hidden_from_aggregates && (
                    // Membership is suppressed for a hidden account, so say so
                    // here rather than offer a checkbox that does nothing.
                    <span
                      className="ml-auto rounded bg-surface px-1.5 py-0.5 text-[10px] text-content-muted"
                      title="This account is hidden from All Inboxes & groups; its mail will not appear in this group until it is shown again."
                    >
                      hidden from groups
                    </span>
                  )}
                </label>
              ))}
              {accounts.length === 0 && (
                <p className="text-xs text-content-muted px-2">No accounts configured</p>
              )}
            </div>
          </div>

          {/* Sender / subject rules */}
          <div>
            <div className="mb-1.5 flex items-center justify-between">
              <label className="text-xs font-medium text-content-secondary">
                Senders & Rules <span className="font-normal text-content-muted">(optional, match any)</span>
              </label>
              <button
                onClick={addRule}
                className="flex items-center gap-1 text-xs text-accent hover:text-accent-hover"
              >
                <Plus className="h-3 w-3" /> Add rule
              </button>
            </div>
            {rules.length === 0 ? (
              <p className="text-xs text-content-muted px-2 py-1">
                No sender rules — group will match by accounts only
              </p>
            ) : (
              <div className="space-y-2">
                {rules.map((rule, i) => (
                  <div key={i} className="flex items-center gap-1.5">
                    <select
                      value={rule.field}
                      onChange={(e) => updateRule(i, { field: e.target.value })}
                      className="rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content"
                    >
                      {FIELD_OPTIONS.map((f) => (
                        <option key={f.value} value={f.value}>{f.label}</option>
                      ))}
                    </select>
                    <select
                      value={rule.operator}
                      onChange={(e) => updateRule(i, { operator: e.target.value })}
                      className="rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content"
                    >
                      {OPERATOR_OPTIONS.map((o) => (
                        <option key={o.value} value={o.value}>{o.label}</option>
                      ))}
                    </select>
                    <input
                      type="text"
                      value={rule.value}
                      onChange={(e) => updateRule(i, { value: e.target.value })}
                      placeholder="e.g. spotify.com"
                      className="min-w-0 flex-1 rounded-md border border-border bg-surface px-2 py-1.5 text-xs text-content placeholder:text-content-muted focus:border-accent focus:outline-none"
                    />
                    <button
                      onClick={() => removeRule(i)}
                      className="shrink-0 text-content-muted hover:text-error"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border-subtle px-4 py-3">
          <div>
            {groupId !== null && (
              <button
                onClick={handleDelete}
                className="text-xs text-error hover:text-error/80"
              >
                Delete Group
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={onClose}
              className="rounded-md bg-elevated px-4 py-2 text-sm text-content-secondary hover:bg-elevated"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={!name.trim() || saving}
              className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-content hover:bg-accent-hover disabled:opacity-50"
            >
              {saving ? "Saving..." : groupId !== null ? "Save" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
