/**
 * The one place the `dtstart` string contract is interpreted.
 *
 * `dtstart` is a **three-way tagged contract** (gotcha #21), produced by
 * `email::detect_events` and `db::gcal::normalize_event_times`:
 *
 * | Shape                  | Meaning                                  |
 * |------------------------|------------------------------------------|
 * | `2026-08-13T16:30:00Z` | a real instant, UTC                      |
 * | `2026-08-13T16:30:00`  | floating — wall-clock, zone unknown      |
 * | `2026-08-13`           | **all day** — a date, not an instant     |
 *
 * The third arm is the one that bites, and it bit twice: `new Date("2026-08-13")`
 * parses as UTC **midnight** and then renders in local time, so an Eastern user is
 * shown *the previous evening at 8:00 PM* — a day early, with a time the event
 * does not have. `CalendarView` guarded that and `CalendarEventCard` did not,
 * which is why the two surfaces disagreed with each other on screen.
 *
 * Extracted because the check now has two callers and it is a contract, not a
 * detail: two copies drift, and the drift is invisible until someone compares two
 * views of the same event.
 */

/**
 * True when `dtstart` denotes a whole day rather than an instant.
 *
 * Keyed on the absence of `T`, which is the actual contract signal, rather than
 * `length === 10` — equivalent for well-formed input, but it does not silently
 * reclassify a malformed value as timed.
 */
export function isAllDay(dtstart: string | null | undefined): boolean {
  if (!dtstart) return false;
  return !dtstart.includes("T");
}

/**
 * Parse a `dtstart`/`dtend` into a `Date`, honouring the contract.
 *
 * An all-day value is built from its components so it lands on **local**
 * midnight; anything else is left to `Date`, which handles the `Z` and floating
 * forms correctly. Returns `null` rather than an `Invalid Date`, so callers must
 * decide what to render instead of accidentally printing "Invalid Date".
 */
export function parseEventDate(value: string | null | undefined): Date | null {
  if (!value) return null;

  if (isAllDay(value)) {
    const parts = value.split("-").map(Number);
    if (parts.length !== 3 || !parts.every((n) => Number.isFinite(n))) return null;
    const [year, month, day] = parts;
    const local = new Date(year, month - 1, day);
    return Number.isNaN(local.getTime()) ? null : local;
  }

  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/** `Thu, Aug 13` — the date, with no time, in the viewer's locale. */
export function formatEventDay(value: string): string | null {
  const date = parseEventDate(value);
  if (!date) return null;
  return date.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}
