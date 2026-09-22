import { useState, useRef } from "react";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { AlertTriangle, ChevronDown, ChevronRight, Ungroup } from "lucide-react";
import AccountIcon from "./AccountIcon";
import { cn } from "@/lib/utils";
import type { Account } from "@/types/email";

interface AccountBadgeProps {
  account: Account;
  isCollapsed?: boolean;
  onToggle?: () => void;
  onClick?: () => void;
  /** When set, the account's background sync is failing — render the ambient
   * warning glyph. `title` carries the detail (last sync time + raw error)
   * as a native tooltip. */
  syncWarning?: { title: string };
  /** The list is showing this account — its inbox or one of its folders. The
   * row renders selected in the account's own colour, the treatment
   * `FolderTree` gives the folder beneath it, so a collapsed account still
   * says which mailbox is open. Without it the only feedback was `hover:`,
   * which leaves with the pointer and is near-invisible on the dark sidebar. */
  isSelected?: boolean;
}

export default function AccountBadge({
  account,
  isCollapsed,
  onToggle,
  onClick,
  syncWarning,
  isSelected = false,
}: AccountBadgeProps) {
  const Chevron = isCollapsed ? ChevronRight : ChevronDown;
  const accent = account.color || "#0a84ff";
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(account.display_name || "");
  const inputRef = useRef<HTMLInputElement>(null);
  const { setAccounts } = useAccountStore();

  const handleDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    setEditName(account.display_name || account.email.split("@")[0]);
    setIsEditing(true);
    setTimeout(() => inputRef.current?.select(), 50);
  };

  const handleSave = async () => {
    const name = editName.trim();
    if (name && name !== account.display_name) {
      await api.accounts.rename(account.id, name);
      const accounts = await api.accounts.list();
      setAccounts(accounts);
    }
    setIsEditing(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") handleSave();
    if (e.key === "Escape") setIsEditing(false);
  };

  return (
    <button
      onClick={onClick}
      aria-current={isSelected ? "true" : undefined}
      className="flex w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-surface"
      // Inline, like FolderTree's selected row: `${hex}26` is the colour at 15%
      // alpha — the same weight as `bg-accent/15` on every other selected
      // sidebar item — and an inline value keeps the tint over `hover:`.
      style={isSelected ? { backgroundColor: `${accent}26` } : undefined}
    >
      <AccountIcon account={account} size={28} />
      <div className="min-w-0 flex-1" onDoubleClick={handleDoubleClick}>
        {isEditing ? (
          <input
            ref={inputRef}
            value={editName}
            onChange={(e) => setEditName(e.target.value)}
            onBlur={handleSave}
            onKeyDown={handleKeyDown}
            onClick={(e) => e.stopPropagation()}
            className="w-full rounded bg-elevated px-1 py-0.5 text-sm text-content outline-none focus:ring-1 focus:ring-accent"
          />
        ) : (
          <p
            className={cn("truncate text-sm", isSelected ? "font-semibold" : "font-medium text-content")}
            style={isSelected ? { color: accent } : undefined}
          >
            {account.display_name || account.email.split("@")[0]}
          </p>
        )}
        <p className="truncate text-[11px] text-content-muted">{account.email}</p>
      </div>
      {syncWarning && (
        <span title={syncWarning.title}>
          <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-[#ff9f0a]" />
        </span>
      )}
      {account.hidden_from_aggregates && (
        // The hiding is otherwise invisible from the sidebar — the row looks
        // like every other account while its mail is missing from every
        // aggregate — so the row itself says so.
        <span
          title="Hidden from All Inboxes, account folders, inbox groups, Needs You, nudges, search and the dock badge. Click the account to see its mail."
          data-testid="hidden-from-aggregates"
        >
          <Ungroup className="h-3.5 w-3.5 shrink-0 text-content-muted" />
        </span>
      )}
      {onToggle && (
        <span
          onClick={(e) => { e.stopPropagation(); onToggle(); }}
          className="rounded p-0.5 hover:bg-elevated"
        >
          <Chevron className="h-3.5 w-3.5 shrink-0 text-content-muted" />
        </span>
      )}
    </button>
  );
}
