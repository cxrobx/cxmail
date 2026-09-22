import { useEffect, useMemo, useState } from "react";
import { Inbox, User, Bell, Users, Tag, MailOpen, ShieldAlert, Filter } from "lucide-react";
import { cn } from "@/lib/utils";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useAccountStore } from "@/stores/accountStore";
import type { EmailCategory } from "@/types/email";
import RulesManager from "./RulesManager";

const CATEGORIES: {
  id: EmailCategory | null;
  label: string;
  icon: typeof Inbox;
}[] = [
  { id: null, label: "All", icon: Inbox },
  { id: "primary", label: "Primary", icon: User },
  { id: "updates", label: "Updates", icon: Bell },
  { id: "social", label: "Social", icon: Users },
  { id: "promotions", label: "Promotions", icon: Tag },
  { id: "junk", label: "Junk", icon: ShieldAlert },
];

export default function CategoryTabs() {
  const selectedCategory = useMailStore((s) => s.selectedCategory);
  const categoryCounts = useMailStore((s) => s.categoryCounts);
  const setSelectedCategory = useMailStore((s) => s.setSelectedCategory);
  const setCategoryCounts = useMailStore((s) => s.setCategoryCounts);
  const filterUnread = useMailStore((s) => s.filterUnread);
  const toggleFilterUnread = useMailStore((s) => s.toggleFilterUnread);
  const selectedAccountId = useMailStore((s) => s.selectedAccountId);
  const selectedAccountGroup = useMailStore((s) => s.selectedAccountGroup);
  const accounts = useAccountStore((s) => s.accounts);
  const [showRules, setShowRules] = useState(false);

  const accountGroupIds = useMemo(() => {
    if (!selectedAccountGroup) return [];
    return accounts.filter((a) => a.group_name === selectedAccountGroup).map((a) => a.id);
  }, [selectedAccountGroup, accounts]);

  useEffect(() => {
    const ids = accountGroupIds.length > 0 ? accountGroupIds : undefined;
    api.categories
      .getCounts(selectedAccountId, ids)
      .then(setCategoryCounts)
      .catch(console.error);
  }, [selectedAccountId, accountGroupIds, setCategoryCounts]);

  return (
    <div className="flex items-center gap-0.5 border-b border-border-subtle px-2 py-1 bg-base overflow-x-auto min-w-0">
      {CATEGORIES.map((cat) => {
        const isActive = selectedCategory === cat.id;
        const count = cat.id ? (categoryCounts[cat.id] ?? 0) : 0;
        const Icon = cat.icon;

        return (
          <button
            key={cat.id ?? "all"}
            onClick={() => setSelectedCategory(cat.id)}
            className={cn(
              "flex shrink-0 items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              isActive
                ? "bg-accent/15 text-accent"
                : "text-content-secondary hover:bg-surface hover:text-content"
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            <span>{cat.label}</span>
            {cat.id && count > 0 && (
              <span
                className={cn(
                  "ml-0.5 min-w-[16px] rounded-full px-1 text-center text-[10px] font-semibold",
                  isActive
                    ? "bg-accent/20 text-accent"
                    : "bg-surface text-content-muted"
                )}
              >
                {count}
              </span>
            )}
          </button>
        );
      })}
      <div className="ml-auto flex shrink-0 items-center gap-0.5">
        <button
          onClick={toggleFilterUnread}
          className={cn(
            "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
            filterUnread
              ? "bg-accent/15 text-accent"
              : "text-content-secondary hover:bg-surface hover:text-content"
          )}
        >
          <MailOpen className="h-3.5 w-3.5" />
          <span>Unread</span>
        </button>
        <button
          onClick={() => setShowRules(true)}
          title="Mail rules"
          className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium text-content-secondary transition-colors hover:bg-surface hover:text-content"
        >
          <Filter className="h-3.5 w-3.5" />
        </button>
      </div>
      {showRules && <RulesManager onClose={() => setShowRules(false)} />}
    </div>
  );
}
