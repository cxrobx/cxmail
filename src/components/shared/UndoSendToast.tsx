import { useEffect, useRef, useState } from "react";
import { useUIStore } from "@/stores/uiStore";
import { api } from "@/lib/tauri";
import { listen } from "@tauri-apps/api/event";
import { motion, AnimatePresence } from "framer-motion";
import { X } from "lucide-react";

type SendFailedPayload = { sendId: string; error: string };

export default function UndoSendToast() {
  const { activeSend, setActiveSend } = useUIStore();
  const [progress, setProgress] = useState(100);
  const [failure, setFailure] = useState<{ subject: string; error: string } | null>(null);
  const lastSubjectRef = useRef<string>("");
  // Mirror of activeSend so the listener (registered once) can read the latest
  // pending draft cleanup for whatever send just finished.
  const activeSendRef = useRef(activeSend);

  useEffect(() => {
    activeSendRef.current = activeSend;
    if (activeSend?.subject) lastSubjectRef.current = activeSend.subject;
  }, [activeSend]);

  // Listen for backend send events
  useEffect(() => {
    const unlisteners: (() => void)[] = [];

    const setup = async () => {
      const u1 = await listen<string>("send-completed", (event) => {
        const current = activeSendRef.current;
        // Only run draft cleanup + log voice-learning edit if SMTP actually succeeded.
        if (current && current.sendId === event.payload) {
          if (current.draftCleanup) {
            const { accountId, folder, uid } = current.draftCleanup;
            api.messages
              .delete(accountId, folder, [uid])
              .then(() => {
                window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
              })
              .catch((e) => console.error("Failed to clean up sent draft:", e));
          }
          if (current.voiceLearning) {
            const { accountId, aiDraft, sentBody, recipientEmail } = current.voiceLearning;
            api.ai
              .logReplyEdit(accountId, aiDraft, sentBody, recipientEmail)
              .catch(() => {});
          }
        }
        setActiveSend(null);
      });
      const u2 = await listen<string>("send-cancelled", () => {
        setActiveSend(null);
      });
      const u3 = await listen<SendFailedPayload>("send-failed", (event) => {
        setActiveSend(null);
        setFailure({
          subject: lastSubjectRef.current || "(no subject)",
          error: event.payload?.error ?? "Unknown SMTP error",
        });
      });
      unlisteners.push(u1, u2, u3);
    };

    setup();
    return () => unlisteners.forEach((u) => u());
  }, [setActiveSend]);

  // Progress countdown
  useEffect(() => {
    if (!activeSend) {
      setProgress(100);
      return;
    }

    const { startedAt, delaySeconds } = activeSend;
    const totalMs = delaySeconds * 1000;

    const tick = () => {
      const elapsed = Date.now() - startedAt;
      const remaining = Math.max(0, 100 - (elapsed / totalMs) * 100);
      setProgress(remaining);

      if (remaining <= 0) {
        // Timer expired — backend will handle the actual send
        return;
      }
    };

    tick();
    const interval = setInterval(tick, 50);
    return () => clearInterval(interval);
  }, [activeSend]);

  const handleCancel = async () => {
    if (!activeSend) return;
    try {
      await api.compose.cancelSend(activeSend.sendId);
    } catch (e) {
      console.error("Failed to cancel send:", e);
    }
    setActiveSend(null);
  };

  return (
    <AnimatePresence>
      {activeSend && (
        <motion.div
          key="active-send"
          initial={{ y: 80, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 80, opacity: 0 }}
          transition={{ type: "spring", damping: 25, stiffness: 300 }}
          className="fixed bottom-6 left-1/2 z-[100] -translate-x-1/2"
        >
          <div className="flex items-center gap-3 rounded-lg border border-border bg-surface px-4 py-3 shadow-2xl">
            <div className="flex-1">
              <div className="text-sm font-medium text-content">Sending...</div>
              <div className="mt-0.5 text-xs text-content-secondary truncate max-w-[200px]">
                {activeSend.subject}
              </div>
              {/* Progress bar */}
              <div className="mt-2 h-1 w-48 overflow-hidden rounded-full bg-elevated">
                <div
                  className="h-full rounded-full bg-accent transition-[width] duration-100 ease-linear"
                  style={{ width: `${progress}%` }}
                />
              </div>
            </div>
            <button
              onClick={handleCancel}
              className="shrink-0 rounded-md border border-[#555] px-3 py-1.5 text-sm font-medium text-content hover:bg-elevated"
            >
              Undo
            </button>
            <button
              onClick={() => setActiveSend(null)}
              className="shrink-0 rounded p-1 text-content-muted hover:text-content"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </motion.div>
      )}
      {failure && (
        <motion.div
          key="send-failure"
          initial={{ y: 80, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 80, opacity: 0 }}
          transition={{ type: "spring", damping: 25, stiffness: 300 }}
          className="fixed bottom-6 left-1/2 z-[100] -translate-x-1/2"
        >
          <div className="flex items-start gap-3 rounded-lg border border-[#ff453a] bg-surface px-4 py-3 shadow-2xl max-w-[440px]">
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-[#ff6961]">Send failed</div>
              <div className="mt-0.5 text-xs text-content-secondary truncate">
                {failure.subject}
              </div>
              <div className="mt-1 text-xs text-content-secondary break-words">
                {failure.error}
              </div>
            </div>
            <button
              onClick={() => setFailure(null)}
              className="shrink-0 rounded p-1 text-content-muted hover:text-content"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
