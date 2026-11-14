import { describe, expect, it } from "vitest";
import { draftConflictTarget } from "@/lib/draftConflict";

describe("draftConflictTarget", () => {
  it("reads the newer UID from the server-side conflict (another device saved)", () => {
    expect(
      draftConflictTarget(
        "Draft conflict: UID 1128 is no longer the current revision of draft 7f3e2a10 (now UID 1129); nothing was saved",
      ),
    ).toBe(1129);
  });

  it("reads it from an Error too", () => {
    expect(draftConflictTarget(new Error("Draft conflict: UID 9 is no longer ... (now UID 12); nothing"))).toBe(12);
  });

  it("is null for a busy claim, which names no revision", () => {
    expect(
      draftConflictTarget("Draft conflict: draft d1 is being saved by another writer (claim held until 2026-11-13T00:00:00Z); nothing was saved"),
    ).toBeNull();
  });

  it("is null for any other failure", () => {
    expect(draftConflictTarget("IMAP connect failed (now UID 5)")).toBeNull();
    expect(draftConflictTarget(undefined)).toBeNull();
    expect(draftConflictTarget("Draft conflict: (now UID 0)")).toBeNull();
  });
});
