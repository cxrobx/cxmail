/**
 * Dependency-free ranking for the command palette. Replaces the old unranked
 * in-order-subsequence filter: results are scored per field (label > keywords
 * > category), with a recall-guarantee fallback that scores the concatenated
 * haystack exactly like the old filter matched it — so anything the old
 * palette found still appears, just ranked low.
 */

export interface Scorable {
  label: string;
  category: string;
  keywords?: string;
}

const LABEL_WEIGHT = 3;
const KEYWORDS_WEIGHT = 1.5;
const CATEGORY_WEIGHT = 1;
/** Fallback tier weight for the concatenated label+category+keywords haystack. */
const HAYSTACK_WEIGHT = 0.5;

function isWordBoundary(text: string, index: number): boolean {
  if (index === 0) return true;
  return !/[a-z0-9]/i.test(text[index - 1]);
}

/**
 * Score one query against one field. 0 = no match; matches always score >= 1.
 * Substring fast path: base 100, +40 prefix / +25 word-boundary, -0.5/char of
 * field length. Otherwise greedy in-order chars: +1 per char, +5 contiguous,
 * +8 at a word boundary, same length penalty.
 */
function scoreField(query: string, field: string): number {
  if (!field) return 0;
  const text = field.toLowerCase();

  const idx = text.indexOf(query);
  if (idx !== -1) {
    let score = 100;
    if (idx === 0) score += 40;
    else if (isWordBoundary(text, idx)) score += 25;
    score -= 0.5 * text.length;
    return Math.max(score, 1);
  }

  let score = 0;
  let searchFrom = 0;
  let prevMatch = -2;
  for (const ch of query) {
    const found = text.indexOf(ch, searchFrom);
    if (found === -1) return 0;
    score += 1;
    if (found === prevMatch + 1) score += 5;
    if (isWordBoundary(text, found)) score += 8;
    prevMatch = found;
    searchFrom = found + 1;
  }
  score -= 0.5 * text.length;
  return Math.max(score, 1);
}

/** Score a query against an item. 0 = no match (item should be hidden). */
export function scoreCommand(query: string, item: Scorable): number {
  const q = query.trim().toLowerCase();
  if (!q) return 0;

  const best = Math.max(
    scoreField(q, item.label) * LABEL_WEIGHT,
    item.keywords ? scoreField(q, item.keywords) * KEYWORDS_WEIGHT : 0,
    scoreField(q, item.category) * CATEGORY_WEIGHT,
  );
  if (best > 0) return best;

  // Recall guarantee: the old filter matched the query as a subsequence of
  // "label category keywords" concatenated, so chars could span field
  // boundaries. Score that haystack (ranked low) so no old match is lost.
  const haystack = `${item.label} ${item.category} ${item.keywords ?? ""}`;
  return scoreField(q, haystack) * HAYSTACK_WEIGHT;
}

/**
 * Rank items by score (desc), dropping non-matches. Ties break by shorter
 * label, then original index (stable).
 */
export function rankCommands<T extends Scorable>(query: string, items: T[]): T[] {
  if (!query.trim()) return items.slice();
  const scored: { item: T; index: number; score: number }[] = [];
  items.forEach((item, index) => {
    const score = scoreCommand(query, item);
    if (score > 0) scored.push({ item, index, score });
  });
  scored.sort(
    (a, b) =>
      b.score - a.score
      || a.item.label.length - b.item.label.length
      || a.index - b.index,
  );
  return scored.map((s) => s.item);
}
