import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildSchedulePresets } from "@/lib/schedulePresets";

describe("buildSchedulePresets", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("offers next week as SEVEN days out on a Monday night, not this morning", () => {
    // The reported bug: a row reading "Monday morning" at 11:47 PM on a Monday.
    // The date it resolves to was always correct — the label was not.
    vi.setSystemTime(new Date(2026, 7, 10, 23, 47, 0)); // Mon Aug 10

    const [tomorrow, nextWeek] = buildSchedulePresets();

    expect(tomorrow.label).toBe("Tomorrow morning");
    expect(tomorrow.date.getDate()).toBe(11); // Tue Aug 11

    expect(nextWeek.label).toBe("Next week");
    expect(nextWeek.date.getDay()).toBe(1); // Monday
    expect(nextWeek.date.getDate()).toBe(17); // Aug 17, not Aug 10
    expect(nextWeek.date.getHours()).toBe(9);
  });

  it("drops the duplicate row on a Sunday", () => {
    // Sunday is the one day next-Monday and tomorrow are the same instant.
    vi.setSystemTime(new Date(2026, 7, 9, 23, 47, 0)); // Sun Aug 9

    const presets = buildSchedulePresets();

    expect(presets).toHaveLength(1);
    expect(presets[0].key).toBe("tomorrow");
    expect(presets[0].date.getDate()).toBe(10); // Mon Aug 10
  });

  it("keeps both rows every other day of the week", () => {
    // Mon-Sat: the two presets must always resolve to different instants.
    for (const day of [10, 11, 12, 13, 14, 15]) {
      vi.setSystemTime(new Date(2026, 7, day, 23, 47, 0));

      const presets = buildSchedulePresets();
      expect(presets).toHaveLength(2);
      expect(presets[0].date.getTime()).not.toBe(presets[1].date.getTime());
    }
  });

  it("never resolves a preset into the past", () => {
    for (const day of [9, 10, 11, 12, 13, 14, 15]) {
      const now = new Date(2026, 7, day, 23, 47, 0);
      vi.setSystemTime(now);

      for (const preset of buildSchedulePresets()) {
        expect(preset.date.getTime()).toBeGreaterThan(now.getTime());
      }
    }
  });
});
