import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { ScheduledEmail, OutgoingEmail } from "@/types/email";
import { Send, X, Play } from "lucide-react";

export default function ScheduledView() {
  const [scheduled, setScheduled] = useState<ScheduledEmail[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  const loadScheduled = async () => {
    setIsLoading(true);
    try {
      const result = await api.schedule.list();
      setScheduled(result);
    } catch (e) {
      console.error("Failed to load scheduled emails:", e);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadScheduled();
  }, []);

  const handleCancel = async (id: number) => {
    try {
      await api.schedule.cancel(id);
      setScheduled((prev) => prev.filter((s) => s.id !== id));
      window.dispatchEvent(new CustomEvent("cxmail:scheduled-changed"));
    } catch (e) {
      console.error("Failed to cancel scheduled email:", e);
    }
  };

  const handleSendNow = async (item: ScheduledEmail) => {
    try {
      const email: OutgoingEmail = JSON.parse(item.email_json);
      await api.schedule.cancel(item.id);
      await api.compose.send(item.account_id, email);
      setScheduled((prev) => prev.filter((s) => s.id !== item.id));
      window.dispatchEvent(new CustomEvent("cxmail:scheduled-changed"));
    } catch (e) {
      console.error("Failed to send now:", e);
    }
  };

  if (isLoading) return <LoadingSpinner className="h-full" />;

  if (scheduled.length === 0) {
    return (
      <EmptyState
        icon={Send}
        title="No scheduled emails"
        description="Scheduled emails will appear here"
      />
    );
  }

  return (
    <div className="h-full overflow-auto">
      {scheduled.map((item) => {
        let email: OutgoingEmail | null = null;
        try {
          email = JSON.parse(item.email_json);
        } catch { /* ignore */ }

        const recipientStr = email?.to.map((r) => r.name || r.email).join(", ") || "Unknown";

        return (
          <div
            key={item.id}
            className="flex items-center gap-3 border-b border-border-subtle px-3 py-2.5 hover:bg-base"
          >
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-medium text-content">
                  To: {recipientStr}
                </span>
              </div>
              <div className="truncate text-sm text-content-secondary">
                {email?.subject || "(no subject)"}
              </div>
              <div className="mt-0.5 flex items-center gap-1.5 text-xs text-accent">
                <Send className="h-3 w-3" />
                <span>
                  Sends {new Date(item.send_at).toLocaleDateString(undefined, {
                    weekday: "short",
                    month: "short",
                    day: "numeric",
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </span>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <button
                onClick={() => handleSendNow(item)}
                className="rounded p-1 text-content-muted hover:bg-surface hover:text-success"
                title="Send now"
              >
                <Play className="h-3.5 w-3.5" />
              </button>
              <button
                onClick={() => handleCancel(item.id)}
                className="rounded p-1 text-content-muted hover:bg-surface hover:text-error"
                title="Cancel"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
