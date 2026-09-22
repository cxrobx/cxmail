import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getLaterToday, getNextMonday, getTomorrowMorning } from "@/lib/snoozePresets";

describe("snoozePresets", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("later today is +3h with minutes/seconds zeroed", () => {
    vi.setSystemTime(new Date(2026, 6, 20, 14, 37, 42)); // Mon Jul 20, 14:37:42
    const d = getLaterToday();
    expect(d.getHours()).toBe(17);
    expect(d.getMinutes()).toBe(0);
    expect(d.getSeconds()).toBe(0);
    expect(d.getDate()).toBe(20);
  });

  it("tomorrow morning is next day at 09:00", () => {
    vi.setSystemTime(new Date(2026, 6, 20, 22, 15, 0));
    const d = getTomorrowMorning();
    expect(d.getDate()).toBe(21);
    expect(d.getHours()).toBe(9);
    expect(d.getMinutes()).toBe(0);
  });

  it("next Monday from a Sunday is +1 day", () => {
    vi.setSystemTime(new Date(2026, 6, 19, 10, 0, 0)); // Sun Jul 19
    const d = getNextMonday();
    expect(d.getDay()).toBe(1);
    expect(d.getDate()).toBe(20);
    expect(d.getHours()).toBe(9);
  });

  it("next Monday from a Wednesday is +5 days", () => {
    vi.setSystemTime(new Date(2026, 6, 22, 10, 0, 0)); // Wed Jul 22
    const d = getNextMonday();
    expect(d.getDay()).toBe(1);
    expect(d.getDate()).toBe(27);
    expect(d.getHours()).toBe(9);
  });
});
