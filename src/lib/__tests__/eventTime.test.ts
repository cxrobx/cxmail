import { describe, it, expect } from "vitest";
import { isAllDay, parseEventDate, formatEventDay } from "@/lib/eventTime";

describe("the dtstart three-way contract", () => {
  it("classifies all three arms", () => {
    expect(isAllDay("2026-08-13")).toBe(true);
    expect(isAllDay("2026-08-13T16:30:00Z")).toBe(false); // UTC instant
    expect(isAllDay("2026-08-13T16:30:00")).toBe(false); // floating
    expect(isAllDay("")).toBe(false);
    expect(isAllDay(null)).toBe(false);
    expect(isAllDay(undefined)).toBe(false);
  });

  /**
   * The bug this module exists to prevent. `new Date("2026-08-13")` is UTC
   * midnight, so rendering it in any negative-offset zone shows the PREVIOUS day.
   * Building from components makes it local midnight, and the assertions below
   * are timezone-independent so they hold on CI as well as on a Mac in ET.
   */
  it("parses an all-day date as LOCAL midnight, not UTC midnight", () => {
    const local = parseEventDate("2026-08-13")!;
    expect(local.getFullYear()).toBe(2026);
    expect(local.getMonth()).toBe(7); // August, 0-indexed
    expect(local.getDate()).toBe(13);
    expect(local.getHours()).toBe(0);
    expect(local.getMinutes()).toBe(0);

    // The naive version this replaces: identical only in UTC, and a day early
    // anywhere west of it.
    const naive = new Date("2026-08-13");
    if (naive.getTimezoneOffset() > 0) {
      expect(naive.getDate()).toBe(12);
      expect(local.getDate()).not.toBe(naive.getDate());
    }
  });

  it("leaves a UTC instant alone", () => {
    const utc = parseEventDate("2026-08-13T16:30:00Z")!;
    expect(utc.toISOString()).toBe("2026-08-13T16:30:00.000Z");
  });

  it("treats a floating time as local wall-clock", () => {
    const floating = parseEventDate("2026-08-13T16:30:00")!;
    expect(floating.getHours()).toBe(16);
    expect(floating.getMinutes()).toBe(30);
    expect(floating.getDate()).toBe(13);
  });

  it("returns null rather than an Invalid Date, so nothing renders 'Invalid Date'", () => {
    for (const bad of ["", "not-a-date", "2026-99", "----", null, undefined]) {
      expect(parseEventDate(bad as string)).toBeNull();
    }
    expect(formatEventDay("not-a-date")).toBeNull();
  });

  it("formats an all-day value on its own date", () => {
    // Locale-independent: assert it names the right day-of-month and not the one
    // before, which is the actual failure mode.
    const shown = formatEventDay("2026-08-13")!;
    expect(shown).toContain("13");
    expect(shown).not.toContain("12");
  });
});
