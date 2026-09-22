import { Clock, Sun, CalendarDays } from "lucide-react";
import DateTimePickerMenu, { formatPresetDate } from "@/components/shared/DateTimePickerMenu";
import type { DateTimePreset } from "@/components/shared/DateTimePickerMenu";
import { getLaterToday, getNextMonday, getTomorrowMorning } from "@/lib/snoozePresets";

interface SnoozePopoverProps {
  onSnooze: (wakeAt: string) => void;
  children: React.ReactNode;
}

export default function SnoozePopover({ onSnooze, children }: SnoozePopoverProps) {
  const presets: DateTimePreset[] = [
    { icon: Clock, label: "Later today", sublabel: formatPresetDate(getLaterToday()), date: getLaterToday() },
    { icon: Sun, label: "Tomorrow morning", sublabel: formatPresetDate(getTomorrowMorning()), date: getTomorrowMorning() },
    { icon: CalendarDays, label: "Next week", sublabel: formatPresetDate(getNextMonday()), date: getNextMonday() },
  ];

  return (
    <DateTimePickerMenu
      title="Snooze until"
      presets={presets}
      confirmLabel="Snooze"
      onSelect={onSnooze}
    >
      {children}
    </DateTimePickerMenu>
  );
}
