import { useUIStore } from "@/stores/uiStore";
import { useMailStore } from "@/stores/mailStore";
import { useWindowStore } from "@/stores/windowStore";
import { api } from "@/lib/tauri";
import type { UnsubscribeResult } from "@/types/email";

export function useUnsubscribe() {
  const addToast = useUIStore((s) => s.addToast);
  const openWindow = useWindowStore((s) => s.openWindow);
  const addUnsubscribedSender = useMailStore((s) => s.addUnsubscribedSender);

  const handleUnsubscribeResult = async (
    result: UnsubscribeResult,
    options?: {
      postAction?: { label: string; onClick: () => void };
      accountId?: string;
      senderEmail?: string;
    },
  ) => {
    const { postAction, accountId, senderEmail } = options ?? {};

    if (result.type === "one-click") {
      // Backend auto-records; update local set
      if (senderEmail) addUnsubscribedSender(senderEmail);
      addToast({
        message: result.confirmed
          ? "Unsubscribed successfully"
          : "Unsubscribe request sent (may take a moment to process)",
        type: result.confirmed ? "success" : "info",
        duration: 6000,
        action: postAction,
      });
    } else if (result.type === "browser") {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(result.url);
      addToast({
        message: "Opening unsubscribe page...",
        type: "info",
        duration: 8000,
        action:
          accountId && senderEmail
            ? {
                label: "Mark as unsubscribed",
                onClick: () => {
                  api.messages.recordUnsubscribedSender(accountId, senderEmail, "browser").catch(console.error);
                  addUnsubscribedSender(senderEmail);
                },
              }
            : undefined,
      });
    } else if (result.type === "mailto") {
      openWindow({
        type: "compose",
        title: "Unsubscribe",
        props: {
          mode: "new",
          defaultTo: result.email,
          defaultSubject: result.subject || "Unsubscribe",
          defaultBody: result.body || "",
        },
      });
      if (accountId && senderEmail) {
        addToast({
          message: "Compose unsubscribe email",
          type: "info",
          duration: 8000,
          action: {
            label: "Mark as unsubscribed",
            onClick: () => {
              api.messages.recordUnsubscribedSender(accountId, senderEmail, "mailto").catch(console.error);
              addUnsubscribedSender(senderEmail);
            },
          },
        });
      }
    }
  };

  return { handleUnsubscribeResult, addToast };
}
