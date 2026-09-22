import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toLocalDateTimeInput } from "@/lib/dateInput";

describe("toLocalDateTimeInput", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns the LOCAL wall clock, not UTC", () => {
    // 11:47 PM — the case that broke the picker. `toISOString().slice(0, 16)`
    // returns the UTC instant, which west of Greenwich is the following day.
    const now = new Date(2026, 7, 10, 23, 47, 0);
    vi.setSystemTime(now);

    expect(toLocalDateTimeInput()).toBe("2026-08-10T23:47");
  });

  it("never lands in the future, at any zone offset", () => {
    // The `min` floor must not exclude times the user can legitimately pick.
    // Asserted against getTimezoneOffset() rather than a literal so this holds
    // wherever the suite runs, not only in ET.
    const now = new Date(2026, 7, 10, 23, 47, 0);
    vi.setSystemTime(now);

    const [datePart, timePart] = toLocalDateTimeInput().split("T");
    const [y, m, d] = datePart.split("-").map(Number);
    const [hh, mm] = timePart.split(":").map(Number);
    const roundTripped = new Date(y, m - 1, d, hh, mm);

    // Equal to now (to the minute), never ahead of it.
    expect(roundTripped.getTime()).toBeLessThanOrEqual(now.getTime());
    expect(now.getTime() - roundTripped.getTime()).toBeLessThan(60_000);
  });

  it("formats an explicit date the same way", () => {
    expect(toLocalDateTimeInput(new Date(2026, 0, 5, 9, 0, 0))).toBe("2026-01-05T09:00");
  });
});
