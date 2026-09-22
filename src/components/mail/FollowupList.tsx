import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { FollowupReminder } from "@/types/email";
import { BellRing, X, Check } from "lucide-react";
import { cn } from "@/lib/utils";

export default function FollowupList() {
  const [reminders, setReminders] = useState<FollowupReminder[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  const loadReminders = async () => {
    setIsLoading(true);
    try {
      const result = await api.followup.list();
      setReminders(result);
    } catch (e) {
      console.error("Failed to load follow-up reminders:", e);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadReminders();
  }, []);

  const handleCancel = async (id: number) => {
    try {
      await api.followup.cancel(id);
      setReminders((prev) => prev.filter((r) => r.id !== id));
    } catch (e) {
      console.error("Failed to cancel reminder:", e);
    }
  };

  const handleDismiss = async (id: number) => {
    try {
      await api.followup.dismiss(id);
      setReminders((prev) => prev.filter((r) => r.id !== id));
    } catch (e) {
      console.error("Failed to dismiss reminder:", e);
    }
  };

  if (isLoading) return <LoadingSpinner className="h-full" />;

  if (reminders.length === 0) {
    return (
      <EmptyState
        icon={BellRing}
        title="No follow-up reminders"
        description="Set a reminder when sending to get notified if no reply"
      />
    );
  }

  return (
    <div className="h-full overflow-auto">
      {reminders.map((rem) => {
        const isFired = rem.status === "fired";
        const remindDate = new Date(rem.remind_at);
        const isPast = remindDate <= new Date();

        return (
          <div
            key={rem.id}
            className={cn(
              "flex items-center gap-3 border-b border-border-subtle px-3 py-2.5 hover:bg-base",
              isFired && "bg-warning/5"
            )}
          >
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-xs text-content-muted">To:</span>
                <span className="truncate text-sm font-medium text-content">
                  {rem.to_email}
                </span>
              </div>
              <div className="truncate text-sm text-content-secondary">
                {rem.subject || "(no subject)"}
              </div>
              <div className={cn(
                "mt-0.5 flex items-center gap-1.5 text-xs",
                isFired ? "text-warning" : "text-accent"
              )}>
                <BellRing className="h-3 w-3" />
                <span>
                  {isFired
                    ? "No reply received"
                    : isPast
                      ? "Checking for reply..."
                      : `Remind ${remindDate.toLocaleDateString(undefined, {
                          weekday: "short",
                          month: "short",
                          day: "numeric",
                          hour: "numeric",
                          minute: "2-digit",
                        })}`
                  }
                </span>
              </div>
            </div>
            {isFired ? (
              <button
                onClick={() => handleDismiss(rem.id)}
                className="shrink-0 rounded p-1 text-content-muted hover:bg-surface hover:text-content-secondary"
                title="Dismiss"
              >
                <Check className="h-3.5 w-3.5" />
              </button>
            ) : (
              <button
                onClick={() => handleCancel(rem.id)}
                className="shrink-0 rounded p-1 text-content-muted hover:bg-surface hover:text-content-secondary"
                title="Cancel reminder"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
