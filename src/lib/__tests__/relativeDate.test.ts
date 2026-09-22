import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { formatRelativeDate } from "../utils";

// Fixed "now" so the relative branches are deterministic.
const NOW = new Date("2026-09-07T12:00:00Z");

describe("formatRelativeDate", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("omits the year for a date in the current year", () => {
    expect(formatRelativeDate("2026-05-14T09:00:00Z")).toBe("May 14");
  });

  it("includes the year for a date in an earlier year", () => {
    expect(formatRelativeDate("2025-01-15T09:00:00Z")).toBe("Jan 15, 2025");
  });

  it("includes the year for an old date, not just last year", () => {
    expect(formatRelativeDate("2019-10-04T09:00:00Z")).toBe("Oct 4, 2019");
  });

  it("still uses relative units inside the first week", () => {
    expect(formatRelativeDate("2026-09-04T12:00:00Z")).toBe("3d");
    expect(formatRelativeDate("2026-09-07T09:00:00Z")).toBe("3h");
    expect(formatRelativeDate("2026-09-07T11:30:00Z")).toBe("30m");
    expect(formatRelativeDate("2026-09-07T11:59:59Z")).toBe("now");
  });
});
