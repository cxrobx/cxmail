/**
 * `<input type="datetime-local">` speaks LOCAL wall-clock time — its `value`
 * and `min` are zone-less strings that the browser interprets in the user's
 * own zone.
 *
 * So the obvious `new Date().toISOString().slice(0, 16)` is WRONG for either
 * one: `toISOString()` is UTC, so west of Greenwich the string lands in the
 * future. Used as `min` at 11:47 PM in ET (UTC-4) it renders `03:47` the next
 * day, and the picker then refuses every time for the rest of the night —
 * silently, since a `min` violation just makes the field invalid rather than
 * saying anything. East of Greenwich it fails the other way and lets the user
 * pick a time already past.
 *
 * Shift by the zone offset first, then slice.
 */
export function toLocalDateTimeInput(value: Date = new Date()): string {
  const local = new Date(value.getTime() - value.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}
