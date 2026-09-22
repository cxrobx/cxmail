import { BellRing, Clock, CalendarDays } from "lucide-react";
import DateTimePickerMenu, { formatPresetDate } from "@/components/shared/DateTimePickerMenu";
import type { DateTimePreset } from "@/components/shared/DateTimePickerMenu";

interface FollowupPopoverProps {
  onSetReminder: (remindAt: string) => void;
  children: React.ReactNode;
}

function addDays(days: number): Date {
  const d = new Date();
  d.setDate(d.getDate() + days);
  d.setHours(9, 0, 0, 0);
  return d;
}

export default function FollowupPopover({ onSetReminder, children }: FollowupPopoverProps) {
  const presets: DateTimePreset[] = [
    { icon: Clock, label: "In 1 day", sublabel: formatPresetDate(addDays(1)), date: addDays(1) },
    { icon: Clock, label: "In 2 days", sublabel: formatPresetDate(addDays(2)), date: addDays(2) },
    { icon: Clock, label: "In 3 days", sublabel: formatPresetDate(addDays(3)), date: addDays(3) },
    { icon: CalendarDays, label: "In 1 week", sublabel: formatPresetDate(addDays(7)), date: addDays(7) },
    { icon: CalendarDays, label: "In 2 weeks", sublabel: formatPresetDate(addDays(14)), date: addDays(14) },
  ];

  return (
    <DateTimePickerMenu
      title="Remind if no reply"
      titleIcon={BellRing}
      presets={presets}
      confirmLabel="Set Reminder"
      onSelect={onSetReminder}
    >
      {children}
    </DateTimePickerMenu>
  );
}
