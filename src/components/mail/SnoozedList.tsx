import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { SnoozedMessageView } from "@/types/email";
import { Clock, X } from "lucide-react";

export default function SnoozedList() {
  const [snoozed, setSnoozed] = useState<SnoozedMessageView[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  const loadSnoozed = async () => {
    setIsLoading(true);
    try {
      const result = await api.snooze.listSnoozed();
      setSnoozed(result);
    } catch (e) {
      console.error("Failed to load snoozed messages:", e);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadSnoozed();
  }, []);

  const handleUnsnooze = async (msg: SnoozedMessageView) => {
    try {
      await api.snooze.unsnooze(msg.account_id, msg.folder_name, msg.uid);
      setSnoozed((prev) => prev.filter((s) => !(s.account_id === msg.account_id && s.folder_name === msg.folder_name && s.uid === msg.uid)));
    } catch (e) {
      console.error("Failed to unsnooze:", e);
    }
  };

  if (isLoading) return <LoadingSpinner className="h-full" />;

  if (snoozed.length === 0) {
    return (
      <EmptyState
        icon={Clock}
        title="No snoozed messages"
        description="Snoozed messages will appear here"
      />
    );
  }

  return (
    <div className="h-full overflow-auto">
      {snoozed.map((msg) => (
        <div
          key={`${msg.account_id}-${msg.folder_name}-${msg.uid}`}
          className="flex items-center gap-3 border-b border-border-subtle px-3 py-2.5 hover:bg-base"
        >
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-sm font-medium text-content">
                {msg.from_name || msg.from_email || "Unknown"}
              </span>
            </div>
            <div className="truncate text-sm text-content-secondary">
              {msg.subject || "(no subject)"}
            </div>
            <div className="mt-0.5 flex items-center gap-1.5 text-xs text-accent">
              <Clock className="h-3 w-3" />
              <span>
                Until {new Date(msg.wake_at).toLocaleDateString(undefined, {
                  weekday: "short",
                  month: "short",
                  day: "numeric",
                  hour: "numeric",
                  minute: "2-digit",
                })}
              </span>
            </div>
          </div>
          <button
            onClick={() => handleUnsnooze(msg)}
            className="shrink-0 rounded p-1 text-content-muted hover:bg-surface hover:text-content-secondary"
            title="Unsnooze"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
