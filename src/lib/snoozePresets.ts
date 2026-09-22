/**
 * Shared snooze preset date math. Used by SnoozePopover (reading pane) and
 * the command palette's Snooze commands — keep the two surfaces identical.
 */

export function getLaterToday(): Date {
  const d = new Date();
  d.setHours(d.getHours() + 3);
  d.setMinutes(0, 0, 0);
  return d;
}

export function getTomorrowMorning(): Date {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  d.setHours(9, 0, 0, 0);
  return d;
}

export function getNextMonday(): Date {
  const d = new Date();
  const dayOfWeek = d.getDay();
  const daysUntilMonday = dayOfWeek === 0 ? 1 : 8 - dayOfWeek;
  d.setDate(d.getDate() + daysUntilMonday);
  d.setHours(9, 0, 0, 0);
  return d;
}
