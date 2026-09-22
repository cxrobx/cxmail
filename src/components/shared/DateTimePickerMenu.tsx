import { useState } from "react";
import { Calendar } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { toLocalDateTimeInput } from "@/lib/dateInput";

export interface DateTimePreset {
  icon: LucideIcon;
  label: string;
  sublabel: string;
  date: Date;
}

export interface DateTimePickerMenuProps {
  title: string;
  titleIcon?: LucideIcon;
  presets: DateTimePreset[];
  confirmLabel: string;
  onSelect: (isoDate: string) => void;
  children: React.ReactNode;
}

export function formatPresetDate(d: Date): string {
  return d.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export default function DateTimePickerMenu({
  title,
  titleIcon: TitleIcon,
  presets,
  confirmLabel,
  onSelect,
  children,
}: DateTimePickerMenuProps) {
  const [showCustom, setShowCustom] = useState(false);
  const [customDate, setCustomDate] = useState("");

  const handlePreset = (date: Date) => {
    onSelect(date.toISOString());
  };

  const handleCustom = () => {
    if (!customDate) return;
    onSelect(new Date(customDate).toISOString());
    setShowCustom(false);
    setCustomDate("");
  };

  return (
    <DropdownMenu.Root onOpenChange={(open) => { if (!open) { setShowCustom(false); setCustomDate(""); } }}>
      <DropdownMenu.Trigger asChild>{children}</DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          className="z-50 w-64 rounded-lg border border-border bg-base-solid p-1 shadow-xl"
          sideOffset={5}
          align="end"
        >
          <div className="px-2 py-1.5 text-xs font-medium text-content-muted">
            {TitleIcon ? (
              <div className="flex items-center gap-1.5">
                <TitleIcon className="h-3 w-3" />
                {title}
              </div>
            ) : (
              title
            )}
          </div>
          {presets.map((preset) => (
            <DropdownMenu.Item
              key={preset.label}
              onSelect={() => handlePreset(preset.date)}
              className="flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm text-content-secondary outline-none hover:bg-surface hover:text-content"
            >
              <preset.icon className="h-4 w-4 shrink-0 text-content-muted" />
              <div className="flex-1">
                <div>{preset.label}</div>
                <div className="text-xs text-content-muted">{preset.sublabel}</div>
              </div>
            </DropdownMenu.Item>
          ))}

          <DropdownMenu.Separator className="my-1 h-px bg-border" />

          {showCustom ? (
            <div className="px-2 py-1.5" onClick={(e) => e.stopPropagation()}>
              <input
                type="datetime-local"
                value={customDate}
                onChange={(e) => setCustomDate(e.target.value)}
                className="w-full rounded border border-border bg-sidebar px-2 py-1.5 text-sm text-content-secondary outline-none focus:border-accent"
                min={toLocalDateTimeInput()}
                onKeyDown={(e) => e.stopPropagation()}
              />
              <div className="mt-1.5 flex justify-end gap-1.5">
                <button
                  onClick={() => { setShowCustom(false); setCustomDate(""); }}
                  className="rounded px-2 py-1 text-xs text-content-secondary hover:text-content"
                >
                  Cancel
                </button>
                <button
                  onClick={handleCustom}
                  disabled={!customDate}
                  className="rounded bg-accent px-2 py-1 text-xs text-white hover:bg-accent-hover disabled:opacity-40"
                >
                  {confirmLabel}
                </button>
              </div>
            </div>
          ) : (
            <DropdownMenu.Item
              onSelect={(e) => { e.preventDefault(); setShowCustom(true); }}
              className="flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm text-content-secondary outline-none hover:bg-surface hover:text-content"
            >
              <Calendar className="h-4 w-4 shrink-0 text-content-muted" />
              <span>Pick date & time</span>
            </DropdownMenu.Item>
          )}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
