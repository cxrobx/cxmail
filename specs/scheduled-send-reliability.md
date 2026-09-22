# Scheduled-Send Reliability

**Status:** Proposed (spec only — no implementation)
**Related:** gotcha #23 (ISO vs `datetime('now')`), `src-tauri/crates/cxmail-db/src/db/scheduled.rs`, `src-tauri/src/lib.rs` scheduler loop, `com.cxmail.app.helper` LaunchAgent

## Problem

Scheduled sends (and snooze wake-ups) are driven by a 30-second `tokio::time::interval` loop that lives **inside the running app** (`src-tauri/src/lib.rs`, ~line 485). Consequences:

1. **App must be open.** If CXMail is quit, nothing scans `scheduled_emails`, so a send scheduled for a time while the app was closed does not go out until the app is next launched.
2. **Mac must be awake.** LaunchAgents/timers do not fire during sleep, so a send scheduled during sleep fires on next wake (late).

A related **time-comparison** bug (gotcha #23) once caused a 9 AM send to leave at ~UTC-midnight the next day (~11 h late) because an ISO-8601 text column was compared lexicographically against `datetime('now')`.

## Current behavior (what is already fixed vs. still open)

- ✅ **Time comparison is fixed.** `db::scheduled::list_due` uses `WHERE datetime(send_at) <= datetime('now') AND status = 'pending'`, normalizing both operands. Locked by regression test `list_due_returns_iso_row_dated_today_at_utc_midnight` (a row dated today's UTC date but in the past — the only fixture shape that catches the lexicographic bug). Snooze/follow-up/calendar scans were fixed the same way. **This spec does not re-fix it; it formalizes it as an invariant so any new sender path reuses `list_due` rather than reimplementing the comparison.**
- ❌ **Durability is still open.** Sending only happens while the app is open and the Mac is awake. The `com.cxmail.app.helper` LaunchAgent currently does IMAP IDLE for *incoming* mail only — it has no scheduled-send responsibility.

## Goals

1. A scheduled send fires at its target time even if the **app is closed**, provided the Mac is awake.
2. If the Mac was **asleep** at the target time, the send fires **promptly on next wake** (bounded lateness), never deferred further.
3. **Never double-send:** if both the app loop and a background sender are alive, exactly one delivers each row.
4. Correct-time-comparison invariant is preserved by construction (shared `list_due`).

## Non-goals

- Guaranteeing on-the-dot delivery while asleep (a serverless local client cannot without a server relay; out of scope).
- A cloud/server send relay.
- Send approval semantics (see `specs/mcp-send-approval-bridge.md`).

## Proposed design

### Sender in the helper sidecar, driven by launchd

Give the existing signed `cxmail-helper` binary a scheduled-send responsibility so sending no longer depends on the app window:

- The helper already has DB access and IMAP connectivity; extend it with the SMTP send path (or factor the app's `commands/compose.rs` send into a shared module both call).
- Drive it from a LaunchAgent with a short `StartInterval` (e.g. 30–60 s) **or** keep the helper resident and run the same 30 s loop there. A resident helper also lets a `ThrottleInterval`-guarded relaunch cover crashes.
- On wake, launchd runs the agent, so a send that came due during sleep is picked up within one interval of wake (satisfies Goal 2).

The **app keeps its own loop** for the common case (app open). Both call the same `list_due` + shared send.

### Double-send prevention (claim/lock)

With two potential senders, a row must be claimed atomically before sending:

- Add a lifecycle to `scheduled_emails.status`: `pending → sending → sent` (plus `failed`).
- Claim with a single transactional statement so only one worker wins:
  ```sql
  UPDATE scheduled_emails
     SET status = 'sending', claimed_at = datetime('now')
   WHERE id = ?1 AND status = 'pending';
  -- proceed only if changes() == 1
  ```
- On SMTP success → `sent`; on failure → `failed` with `error_message` (and a bounded retry policy: N attempts, then `failed`, surfaced in the Scheduled view).
- A stale-claim reaper resets `sending` rows older than a timeout back to `pending` (covers a worker that died mid-send). Use `datetime(claimed_at) < datetime('now', '-M minutes')` — same wrapped-comparison invariant.

### Time-comparison invariant (formalized)

- All due-scan predicates wrap **both** operands in `datetime()`; storage stays ISO-8601 (frontend display parses the trailing `Z` as UTC — do not switch storage to space format).
- Any new sender path MUST call the shared `db::scheduled::list_due` (and snooze/follow-up equivalents), never hand-roll a comparison.
- The regression fixture must share today's UTC date but be in the past (`date('now') || 'T00:00:00.001Z'`); a naive 2020/2999 fixture does not catch the bug.

## Failure & sleep semantics

| Condition | Behavior |
|-----------|----------|
| App open, due | App loop sends (claim guards against helper racing) |
| App closed, Mac awake, due | Helper (launchd) sends |
| Mac asleep at target time | Fires within one interval of next wake |
| Worker dies mid-send | Row stuck in `sending`; reaper returns it to `pending` after timeout |
| SMTP failure | `failed` + error surfaced; bounded retry |

## Security

- The helper needs SMTP credential access (it already reads IMAP credentials via the same store); no new secret surface beyond that. No credentials in SQLite (invariant #1).
- Claim/lock changes are additive columns on `scheduled_emails`; no credential or body content moves.

## Testing

- Claim race: two concurrent claim attempts on one `pending` row → exactly one `changes() == 1`, the other `0`.
- Reaper: a `sending` row older than the timeout returns to `pending`; a fresh one does not.
- Due scan: reuse the gotcha #23 fixture (today's UTC date, in the past) against the shared `list_due`.
- Retry: SMTP failure increments attempts and lands in `failed` after N.

## Open questions

- Resident helper loop vs. launchd `StartInterval` relaunch — resident is simpler for the 30 s cadence and matches the app loop; launchd relaunch is more crash-resilient. (Leaning resident with a `KeepAlive`/`ThrottleInterval` guard.)
- Retry count / backoff policy defaults (proposal: 3 attempts, linear backoff, then `failed`).
- Should the app loop **defer** to the helper when the helper is known-resident, to avoid two live loops? (Claim/lock makes it safe either way; deferring is an optimization, not a correctness need.)
