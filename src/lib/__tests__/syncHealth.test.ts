import { describe, it, expect } from "vitest";
import {
  applySyncStatuses,
  isFailing,
  FAILING_THRESHOLD,
  type AccountSyncHealth,
} from "../syncHealth";
import type { AccountSyncStatus } from "@/types/email";

const NOW = "2026-08-17T12:00:00.000Z";

function status(overrides: Partial<AccountSyncStatus> = {}): AccountSyncStatus {
  return {
    account_id: "acct-1",
    email: "chris@cxventures.io",
    success: false,
    error: "IMAP error: Connection timed out",
    new_count: 0,
    needs_reauth: false,
    ...overrides,
  };
}

/** Run `n` consecutive batches of the same statuses through the fold. */
function runTicks(
  start: Record<string, AccountSyncHealth>,
  batches: AccountSyncStatus[][],
  announceFailures = true,
) {
  let map = start;
  let last: ReturnType<typeof applySyncStatuses> = { next: map, announcements: [] };
  for (const batch of batches) {
    last = applySyncStatuses(map, batch, NOW, announceFailures);
    map = last.next;
  }
  return { map, last };
}

describe("applySyncStatuses", () => {
  it("announces exactly once, when the streak crosses the threshold", () => {
    const batches = Array.from({ length: FAILING_THRESHOLD }, () => [status()]);
    let map: Record<string, AccountSyncHealth> = {};
    const announced: string[] = [];
    for (const batch of batches) {
      const r = applySyncStatuses(map, batch, NOW, true);
      map = r.next;
      announced.push(...r.announcements.map((a) => a.type));
    }
    expect(announced).toEqual(["error"]);
    expect(map["acct-1"].announced).toBe(true);
    expect(isFailing(map["acct-1"])).toBe(true);
  });

  it("does not re-announce on continued failure past the threshold", () => {
    const { last } = runTicks({}, Array.from({ length: FAILING_THRESHOLD + 3 }, () => [status()]));
    expect(last.announcements).toEqual([]);
  });

  it("stays silent below the threshold", () => {
    const { map, last } = runTicks({}, Array.from({ length: FAILING_THRESHOLD - 1 }, () => [status()]));
    expect(last.announcements).toEqual([]);
    expect(isFailing(map["acct-1"])).toBe(false);
  });

  it("announces recovery once, and only after an announced failure", () => {
    const fails = Array.from({ length: FAILING_THRESHOLD }, () => [status()]);
    const { map } = runTicks({}, fails);
    const r = applySyncStatuses(map, [status({ success: true, error: null })], NOW, true);
    expect(r.announcements).toEqual([
      { message: "chris@cxventures.io is syncing again", type: "success" },
    ]);
    expect(r.next["acct-1"]).toEqual({
      consecutiveFailures: 0,
      lastError: null,
      lastSuccessAt: NOW,
      announced: false,
    });
  });

  it("recovers silently when the outage was never announced", () => {
    const { map } = runTicks({}, [[status()], [status()]]);
    const r = applySyncStatuses(map, [status({ success: true, error: null })], NOW, true);
    expect(r.announcements).toEqual([]);
    expect(r.next["acct-1"].consecutiveFailures).toBe(0);
  });

  it("never announces a needs_reauth failure, but still counts the streak", () => {
    const batches = Array.from({ length: FAILING_THRESHOLD + 1 }, () => [
      status({ needs_reauth: true, error: "invalid_grant" }),
    ]);
    const { map, last } = runTicks({}, batches);
    expect(last.announcements).toEqual([]);
    expect(map["acct-1"].announced).toBe(false);
    expect(isFailing(map["acct-1"])).toBe(true);
  });

  it("announceFailures=false records without announcing, and a later announced tick fires", () => {
    // All-accounts-failed ticks: suppressed.
    const { map } = runTicks({}, Array.from({ length: FAILING_THRESHOLD }, () => [status()]), false);
    expect(map["acct-1"].announced).toBe(false);
    expect(isFailing(map["acct-1"])).toBe(true);
    // Outage narrows to this account: the very next announceable tick fires.
    const r = applySyncStatuses(map, [status()], NOW, true);
    expect(r.announcements.map((a) => a.type)).toEqual(["error"]);
  });

  it("leaves accounts absent from the batch untouched — absence is not recovery", () => {
    const { map } = runTicks({}, [[status()], [status()]]);
    // The 60s-skip case: the account does not appear in the next batch at all.
    const r = applySyncStatuses(map, [status({ account_id: "acct-2", email: "other@x.com", success: true })], NOW, true);
    expect(r.next["acct-1"].consecutiveFailures).toBe(2);
  });

  it("keeps the latest error for the tooltip", () => {
    const { map } = runTicks({}, [[status({ error: "first" })], [status({ error: "second" })]]);
    expect(map["acct-1"].lastError).toBe("second");
  });

  it("mentions the outage duration when a success time is known", () => {
    const earlier = "2026-08-17T11:13:00.000Z"; // 47 minutes before NOW
    let map: Record<string, AccountSyncHealth> = {};
    map = applySyncStatuses(map, [status({ success: true, error: null })], earlier, true).next;
    const { last } = runTicks(map, Array.from({ length: FAILING_THRESHOLD }, () => [status()]));
    expect(last.announcements[0].message).toBe(
      "chris@cxventures.io hasn't synced in 47 minutes — mail may be delayed",
    );
  });

  it("falls back to a generic message when the account never synced this session", () => {
    const { last } = runTicks({}, Array.from({ length: FAILING_THRESHOLD }, () => [status()]));
    expect(last.announcements[0].message).toBe(
      "chris@cxventures.io can't sync — mail may be delayed",
    );
  });
});
