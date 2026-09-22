import { useEffect, useRef } from "react";
import { X } from "lucide-react";
import Kbd from "@/components/shared/Kbd";

interface KeyboardShortcutSheetProps {
  onClose: () => void;
}

const shortcuts = [
  {
    group: "Navigation",
    items: [
      { keys: ["j"], description: "Next message" },
      { keys: ["k"], description: "Previous message" },
      { keys: ["Esc"], description: "Deselect message" },
    ],
  },
  {
    group: "Actions",
    items: [
      { keys: ["c"], description: "Compose new email" },
      { keys: ["r"], description: "Reply" },
      { keys: ["⌘", "E"], description: "Archive" },
      { keys: ["#"], description: "Delete" },
      { keys: ["s"], description: "Toggle star" },
      { keys: ["p"], description: "Toggle pin" },
      { keys: ["m"], description: "Mute conversation" },
      { keys: ["/"], description: "Search" },
    ],
  },
  {
    group: "Interface",
    items: [
      { keys: ["\u2318", "K"], description: "Command palette" },
      { keys: ["\u2318", "P"], description: "Command palette" },
      { keys: ["\u2318", "N"], description: "Compose" },
      { keys: ["\u2318", "\\"], description: "Toggle sidebar" },
      { keys: ["?"], description: "Keyboard shortcuts" },
    ],
  },
];

export default function KeyboardShortcutSheet({ onClose }: KeyboardShortcutSheetProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-start justify-center pt-[15%]"
      onClick={onClose}
    >
      <div
        ref={ref}
        className="w-[420px] overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <h2 className="text-sm font-medium text-content">Keyboard Shortcuts</h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-content-secondary hover:text-content"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Shortcut groups */}
        <div className="space-y-4 p-4">
          {shortcuts.map((group) => (
            <div key={group.group}>
              <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
                {group.group}
              </h3>
              <div className="space-y-1.5">
                {group.items.map((item) => (
                  <div
                    key={`${item.description}-${item.keys.join("")}`}
                    className="flex items-center justify-between"
                  >
                    <span className="text-sm text-content-secondary">
                      {item.description}
                    </span>
                    <div className="flex items-center gap-0.5">
                      {item.keys.map((key, i) => (
                        <Kbd key={i}>{key}</Kbd>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
