/** A fetch must not overwrite typing or a newer reload that happened while it waited. */
export async function fetchDraftReload<T>(
  fetch: () => Promise<T>,
  snapshot: () => string | null,
  isCurrent: () => boolean,
): Promise<{ kind: "ready"; value: T } | { kind: "edited" } | { kind: "superseded" }> {
  const before = snapshot();
  const value = await fetch();
  if (!isCurrent()) return { kind: "superseded" };
  if (snapshot() !== before) return { kind: "edited" };
  return { kind: "ready", value };
}
