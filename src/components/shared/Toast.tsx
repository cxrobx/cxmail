import { useEffect, useRef } from "react";
import { useUIStore } from "@/stores/uiStore";
import type { Toast as ToastType } from "@/stores/uiStore";
import { motion, AnimatePresence } from "framer-motion";
import { CheckCircle2, AlertCircle, Info, X } from "lucide-react";

const ICONS = {
  success: CheckCircle2,
  error: AlertCircle,
  info: Info,
} as const;

const ICON_COLORS = {
  success: "text-green-400",
  error: "text-red-400",
  info: "text-blue-400",
} as const;

function ToastItem({ toast, onDismiss }: { toast: ToastType; onDismiss: () => void }) {
  const duration = toast.duration ?? (toast.action ? 6000 : 4000);
  const Icon = ICONS[toast.type];
  const onExpireRef = useRef(toast.onExpire);
  onExpireRef.current = toast.onExpire;

  useEffect(() => {
    const timer = setTimeout(() => {
      onExpireRef.current?.();
      onDismiss();
    }, duration);
    return () => clearTimeout(timer);
  }, [duration, onDismiss]);

  return (
    <motion.div
      layout
      initial={{ y: 20, opacity: 0, scale: 0.95 }}
      animate={{ y: 0, opacity: 1, scale: 1 }}
      exit={{ y: 20, opacity: 0, scale: 0.95 }}
      transition={{ type: "spring", damping: 25, stiffness: 300 }}
    >
      <div className="flex items-center gap-3 rounded-lg border border-border bg-surface px-4 py-3 shadow-2xl">
        <Icon className={`h-4 w-4 shrink-0 ${ICON_COLORS[toast.type]}`} />
        <span className="text-sm text-content">{toast.message}</span>
        {toast.action && (
          <button
            onClick={() => {
              toast.action!.onClick();
              onDismiss();
            }}
            className="shrink-0 rounded-md border border-[#555] px-3 py-1 text-xs font-medium text-content hover:bg-elevated"
          >
            {toast.action.label}
          </button>
        )}
        <button
          onClick={onDismiss}
          className="shrink-0 rounded p-1 text-content-muted hover:text-content"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    </motion.div>
  );
}

export default function Toast() {
  const toasts = useUIStore((s) => s.toasts);
  const removeToast = useUIStore((s) => s.removeToast);

  return (
    <div className="fixed bottom-16 left-1/2 z-[100] flex -translate-x-1/2 flex-col gap-2">
      <AnimatePresence mode="popLayout">
        {toasts.map((toast) => (
          <ToastItem
            key={toast.id}
            toast={toast}
            onDismiss={() => removeToast(toast.id)}
          />
        ))}
      </AnimatePresence>
    </div>
  );
}
