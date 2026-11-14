/**
 * The newer revision a `Draft conflict:` error names (`… (now UID N) …`), or
 * null for any other failure. Both conflict errors `edit_draft` returns use
 * this wording: the server-side check (another device saved) and the local
 * claim (Claude saved through the MCP on this Mac).
 */
export function draftConflictTarget(error: unknown): number | null {
  const text = typeof error === "string" ? error : error instanceof Error ? error.message : String(error ?? "");
  if (!text.includes("Draft conflict:")) return null;
  const m = /\(now UID (\d+)\)/.exec(text);
  if (!m) return null;
  const uid = Number(m[1]);
  return Number.isSafeInteger(uid) && uid > 0 ? uid : null;
}
