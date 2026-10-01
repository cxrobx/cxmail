import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { formatRelativeDate } from "@/lib/utils";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import { reviewSuggestedReply } from "@/lib/suggestedReply";
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
  const [lane, setLane] = useState<"needs_response" | "waiting">("needs_response");
  const [error, setError] = useState("");
  const [reviewing, setReviewing] = useState<string | null>(null);
  const generation = useRef(0);
  const setSelectedMessage = useMailStore((state) => state.setSelectedMessage);
  const selectedUid = useMailStore((state) => state.selectedMessageUid);
  const selectedAccountId = useMailStore((state) => state.selectedMessageAccountId);

  const load = useCallback(() => {
    const request = ++generation.current;
    return api.needsYou
      .list()
      .then((value) => { if (request === generation.current) { setItems(value); setError(""); } })
      .catch((error) => { if (request === generation.current) setError(String(error)); })
      .finally(() => { if (request === generation.current) setLoading(false); });
  }, []);

  useEffect(() => {
    void load();
    // Keep the queue honest while mail arrives or is triaged elsewhere.
    const refresh = () => void load();
    window.addEventListener("cxmail:refresh-messages", refresh);
    window.addEventListener("cxmail:intelligence-updated", refresh);
    return () => { generation.current++; window.removeEventListener("cxmail:refresh-messages", refresh); window.removeEventListener("cxmail:intelligence-updated", refresh); };
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
      setError(String(error));
      return;
    }
    setItems((current) => current.filter((candidate) =>
      candidate.uid !== item.uid || candidate.account_id !== item.account_id || candidate.folder_name !== item.folder_name
    ));
    window.dispatchEvent(new CustomEvent("cxmail:needs-you-changed"));
  };

  if (loading) return <LoadingSpinner className="h-full" />;
  const visible = items.filter((i) => (i.lane ?? "needs_response") === lane);
  const now = new Date();
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  const review = async (item: NeedsYouItem) => {
    const key = `${item.account_id}:${item.folder_name}:${item.uid}`;
    setReviewing(key); setError("");
    try { await reviewSuggestedReply(item); } catch (e) { setError(String(e)); }
    finally { setReviewing(null); }
  };

  return (
    <div className="h-full overflow-auto">
      <div className="border-b border-border-subtle px-4 py-3">
        <div className="text-sm font-semibold text-content">Needs You</div>
        <div className="text-xs text-content-muted">Requests, deadlines, and conversations waiting for a response.</div>
        <div className="mt-3 flex gap-1" role="tablist" aria-label="Action queue">
          {([['needs_response', 'Needs my response'], ['waiting', 'Waiting on them']] as const).map(([value, label]) => (
            <button key={value} role="tab" aria-selected={lane === value} onClick={() => setLane(value)} className={`rounded px-2 py-1.5 text-xs ${lane === value ? "bg-accent/15 text-accent" : "text-content-muted hover:bg-surface"}`}>
              {label} <span className="tabular-nums">{items.filter((i) => (i.lane ?? 'needs_response') === value).length}</span>
            </button>
          ))}
        </div>
      </div>
      {error && <div role="alert" className="px-4 py-2 text-xs text-red-400">{error} <button className="underline" onClick={() => void load()}>Retry</button></div>}
      {visible.length === 0 && !error && <EmptyState icon={Check} title={lane === "waiting" ? "No outstanding replies" : "You’re caught up"} description={lane === "waiting" ? "Conversations you have replied to will appear here while you wait." : "Messages asking for a reply, decision, follow-up, or review will appear here"} />}
      {visible.map((item) => {
        const meta = actionMeta[item.action_type] ?? actionMeta.reply;
        const Icon = meta.icon;
        const selected = selectedUid === item.uid && selectedAccountId === item.account_id;
        const repeats = item.duplicate_count ?? 1;
        return (
          <article
            key={`${item.account_id}:${item.folder_name}:${item.uid}`}
            className={`group border-b border-border-subtle ${selected ? "bg-accent/15" : ""}`}
          >
          <button
            onClick={() => setSelectedMessage(item.uid, item.account_id, item.folder_name)}
            className="flex w-full gap-3 px-4 pt-3 pb-2 text-left hover:bg-surface"
          >
            <span className={`mt-0.5 self-start rounded p-1.5 ${meta.color}`}>
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
                {item.request || (item.evidence ? `“${item.evidence}”` : item.reason)}
              </span>
              {item.deadline && <span className={`mt-1 block text-[11px] font-medium ${item.deadline < today ? "text-red-400" : "text-amber-400"}`}>{item.deadline < today ? "Overdue" : "Due"} · {item.deadline}</span>}
            </span>
          </button>
          <div className="flex items-center justify-end gap-2 px-4 pb-2">
            {item.suggestion && <button onClick={() => void review(item)} disabled={reviewing !== null} className="rounded bg-accent/10 px-2 py-1 text-[11px] font-medium text-accent disabled:opacity-50">{reviewing === `${item.account_id}:${item.folder_name}:${item.uid}` ? "Opening…" : "Review suggested reply"}</button>}
            <button
              title={repeats > 1 ? `Done / dismiss all ${repeats}` : "Done / dismiss"}
              aria-label={repeats > 1 ? `Dismiss all ${repeats} messages from Needs You` : "Dismiss from Needs You"}
              onClick={(event) => { event.stopPropagation(); void dismiss(item); }}
              className="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-content-muted hover:bg-elevated hover:text-content"
            >
              <Check className="h-3 w-3" />Done
            </button>
          </div>
          </article>
        );
      })}
    </div>
  );
}
