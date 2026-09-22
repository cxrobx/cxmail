import { getNextMonday, getTomorrowMorning } from "@/lib/snoozePresets";

export interface SchedulePreset {
  key: "tomorrow" | "next-week";
  label: string;
  date: Date;
}

/**
 * Schedule-send presets for the compose menu, resolved fresh each time it opens.
 *
 * Wording matches SnoozePopover deliberately, because it shares the same math:
 * `getNextMonday` always jumps to the FOLLOWING Monday, so on a Monday it means
 * seven days out. Labelling that "Monday morning" is unreadable on a Monday
 * night — hence "Next week", with the caller rendering the resolved date under
 * it so the row can never be ambiguous.
 *
 * The icons live with the caller; this module stays React-free so the collision
 * rule below is testable on its own.
 */
export function buildSchedulePresets(): SchedulePreset[] {
  const tomorrow = getTomorrowMorning();
  const nextWeek = getNextMonday();

  const presets: SchedulePreset[] = [
    { key: "tomorrow", label: "Tomorrow morning", date: tomorrow },
  ];

  // On a Sunday, next Monday IS tomorrow — offering both is two rows and one
  // outcome, so drop the duplicate rather than ask the user to spot it.
  if (nextWeek.getTime() !== tomorrow.getTime()) {
    presets.push({ key: "next-week", label: "Next week", date: nextWeek });
  }

  return presets;
}
