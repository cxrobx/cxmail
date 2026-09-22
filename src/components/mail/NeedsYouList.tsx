import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { formatRelativeDate } from "@/lib/utils";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { NeedsYouAction, NeedsYouItem } from "@/types/email";
import {
  Check,
  CircleHelp,
  FileCheck,
  MessageCircleReply,
  Paperclip,
  RotateCcw,
  TriangleAlert,
} from "lucide-react";

const actionMeta: Record<NeedsYouAction, { label: string; icon: typeof CircleHelp; color: string }> = {
  decision: { label: "Decision", icon: CircleHelp, color: "text-amber-400 bg-amber-400/10" },
  reply: { label: "Reply", icon: MessageCircleReply, color: "text-blue-400 bg-blue-400/10" },
  follow_up: { label: "Follow up", icon: RotateCcw, color: "text-violet-400 bg-violet-400/10" },
  review: { label: "Review", icon: FileCheck, color: "text-emerald-400 bg-emerald-400/10" },
  alert: { label: "Alert", icon: TriangleAlert, color: "text-red-400 bg-red-400/10" },
};

export default function NeedsYouList() {
  const [items, setItems] = useState<NeedsYouItem[]>([]);
  const [loading, setLoading] = useState(true);
  const setSelectedMessage = useMailStore((state) => state.setSelectedMessage);
  const selectedUid = useMailStore((state) => state.selectedMessageUid);
  const selectedAccountId = useMailStore((state) => state.selectedMessageAccountId);

  const load = useCallback(() => {
    return api.needsYou
      .list()
      .then(setItems)
      .catch((error) => console.error("Failed to load Needs You:", error))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    void load();
    // Keep the queue honest while mail arrives or is triaged elsewhere.
    const refresh = () => void load();
    window.addEventListener("cxmail:refresh-messages", refresh);
    return () => window.removeEventListener("cxmail:refresh-messages", refresh);
  }, [load]);

  const dismiss = async (item: NeedsYouItem) => {
    // Dismiss every message behind the row, not just the representative —
    // otherwise a collapsed alert storm reappears with its next member.
    const members = item.members?.length
      ? item.members
      : [{ account_id: item.account_id, folder_name: item.folder_name, uid: item.uid }];
    try {
      await api.needsYou.dismissGroup(members);
    } catch (error) {
      console.error("Failed to dismiss:", error);
      return;
    }
    setItems((current) => current.filter((candidate) =>
      candidate.uid !== item.uid || candidate.account_id !== item.account_id || candidate.folder_name !== item.folder_name
    ));
    window.dispatchEvent(new CustomEvent("cxmail:needs-you-changed"));
  };

  if (loading) return <LoadingSpinner className="h-full" />;
  if (items.length === 0) {
    return (
      <EmptyState
        icon={Check}
        title="You’re caught up"
        description="Messages asking for a reply, decision, follow-up, or review will appear here"
      />
    );
  }

  return (
    <div className="h-full overflow-auto">
      <div className="border-b border-border-subtle px-4 py-3">
        <div className="text-sm font-semibold text-content">Needs You</div>
        <div className="text-xs text-content-muted">A private, on-device action queue. Dismiss anything that doesn’t need your attention.</div>
      </div>
      {items.map((item) => {
        const meta = actionMeta[item.action_type] ?? actionMeta.reply;
        const Icon = meta.icon;
        const selected = selectedUid === item.uid && selectedAccountId === item.account_id;
        const repeats = item.duplicate_count ?? 1;
        return (
          <button
            key={`${item.account_id}:${item.folder_name}:${item.uid}`}
            onClick={() => setSelectedMessage(item.uid, item.account_id, item.folder_name)}
            className={`group flex w-full gap-3 border-b border-border-subtle px-4 py-3 text-left hover:bg-surface ${selected ? "bg-accent/15" : ""}`}
          >
            <span className={`mt-0.5 rounded p-1.5 ${meta.color}`}>
              <Icon className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-2">
                <span className={`truncate text-sm text-content ${item.is_read ? "font-medium" : "font-semibold"}`}>
                  {item.from_name || item.from_email || "Unknown sender"}
                </span>
                <span className={`shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium ${meta.color}`}>{meta.label}</span>
                {repeats > 1 && (
                  <span
                    className="shrink-0 rounded-full bg-elevated px-1.5 py-0.5 text-[10px] font-medium text-content-muted"
                    title={`${repeats} messages collapsed into this item`}
                  >
                    ×{repeats}
                  </span>
                )}
                <span className="ml-auto shrink-0 text-xs text-content-muted">{formatRelativeDate(item.date)}</span>
              </span>
              <span className="mt-0.5 flex items-center gap-1 truncate text-sm text-content-secondary">
                {item.has_attachments && <Paperclip className="h-3 w-3 shrink-0" />}
                {item.subject || "(no subject)"}
              </span>
              {/* Evidence: the sentence that actually triggered the match. Falls
                  back to the category reason when nothing quotable was found. */}
              <span className="mt-0.5 block truncate text-xs text-content-muted">
                {item.evidence ? `“${item.evidence}”` : item.reason}
              </span>
            </span>
            <span
              role="button"
              tabIndex={0}
              title={repeats > 1 ? `Done / dismiss all ${repeats}` : "Done / dismiss"}
              aria-label={repeats > 1 ? `Dismiss all ${repeats} messages from Needs You` : "Dismiss from Needs You"}
              onClick={(event) => { event.stopPropagation(); void dismiss(item); }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.stopPropagation();
                  void dismiss(item);
                }
              }}
              className="self-center rounded p-1.5 text-content-muted opacity-0 hover:bg-elevated hover:text-content group-hover:opacity-100"
            >
              <Check className="h-4 w-4" />
            </span>
          </button>
        );
      })}
    </div>
  );
}
