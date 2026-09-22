import type { AccountSyncStatus } from "@/types/email";

/** Consecutive failed sync ticks before an account counts as "failing".
 * Background sync runs ~every 5 minutes, so 3 ≈ 15 minutes of outage. */
export const FAILING_THRESHOLD = 3;

export interface AccountSyncHealth {
  consecutiveFailures: number;
  lastError: string | null;
  /** ISO timestamp of the last successful sync this session; null until one lands. */
  lastSuccessAt: string | null;
  /** True once the failure toast for the current outage has been shown —
   * gates the single recovery toast and prevents re-announcing every tick. */
  announced: boolean;
}

export interface SyncAnnouncement {
  message: string;
  type: "error" | "success";
}

export function isFailing(health: AccountSyncHealth | undefined): boolean {
  return (health?.consecutiveFailures ?? 0) >= FAILING_THRESHOLD;
}

const EMPTY: AccountSyncHealth = {
  consecutiveFailures: 0,
  lastError: null,
  lastSuccessAt: null,
  announced: false,
};

function outageSpan(lastSuccessAt: string, nowIso: string): string {
  const mins = Math.max(1, Math.round((Date.parse(nowIso) - Date.parse(lastSuccessAt)) / 60_000));
  return mins >= 120 ? `${Math.round(mins / 60)} hours` : `${mins} minutes`;
}

function failureMessage(email: string, health: AccountSyncHealth, nowIso: string): string {
  return health.lastSuccessAt
    ? `${email} hasn't synced in ${outageSpan(health.lastSuccessAt, nowIso)} — mail may be delayed`
    : `${email} can't sync — mail may be delayed`;
}

/**
 * Fold one background-sync batch into the per-account health map and compute
 * which transition announcements (if any) to show.
 *
 * Semantics that matter:
 * - Only accounts present in `statuses` are touched. The backend deliberately
 *   omits an account that hit the per-account 60s skip, and absence must not
 *   read as recovery — only an explicit success resets a failure streak.
 * - A failure is announced once, when the streak crosses FAILING_THRESHOLD;
 *   recovery is announced only if the failure was announced. A silent outage
 *   recovers silently.
 * - `announceFailures: false` records failures without announcing (used when
 *   every account failed at once — that's a global condition with its own
 *   StatusBar treatment, not per-account news, and it must not fire N toasts
 *   after a wake-from-sleep). `announced` stays false, so the toast still
 *   fires later if the outage narrows to specific accounts.
 * - `needs_reauth` failures are never announced here — the StatusBar
 *   "Reconnect" prompt owns that state.
 */
export function applySyncStatuses(
  prev: Record<string, AccountSyncHealth>,
  statuses: AccountSyncStatus[],
  nowIso: string,
  announceFailures: boolean,
): { next: Record<string, AccountSyncHealth>; announcements: SyncAnnouncement[] } {
  const next = { ...prev };
  const announcements: SyncAnnouncement[] = [];

  for (const s of statuses) {
    const health = next[s.account_id] ?? EMPTY;
    if (s.success) {
      if (health.announced) {
        announcements.push({ message: `${s.email} is syncing again`, type: "success" });
      }
      next[s.account_id] = {
        consecutiveFailures: 0,
        lastError: null,
        lastSuccessAt: nowIso,
        announced: false,
      };
    } else {
      const updated: AccountSyncHealth = {
        ...health,
        consecutiveFailures: health.consecutiveFailures + 1,
        lastError: s.error ?? "Unknown error",
      };
      if (
        announceFailures &&
        !s.needs_reauth &&
        !updated.announced &&
        updated.consecutiveFailures >= FAILING_THRESHOLD
      ) {
        announcements.push({ message: failureMessage(s.email, updated, nowIso), type: "error" });
        updated.announced = true;
      }
      next[s.account_id] = updated;
    }
  }

  return { next, announcements };
}
