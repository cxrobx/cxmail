import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

/**
 * Guard against a colour utility that names a token nobody declared.
 *
 * Tailwind v4 generates utilities only for tokens declared in `@theme`. A class
 * like `bg-panel` with no `--color-panel` emits **no rule at all** — so the
 * element simply has no background. There is no build warning, no runtime error,
 * and `tsc` cannot see it: it is a string in a `className`.
 *
 * Not hypothetical. `bg-panel` shipped in the calendar event-detail modal and
 * `EventComposeModal` and rendered them *fully transparent* — the month grid and
 * event cards read straight through the dialog — surviving for months because both
 * are calendar surfaces nobody looked at closely. It was found by eyeballing a
 * screenshot.
 *
 * ## Two tiers, because this test gates a release
 *
 * `npm test -- --run` runs in both `ci.yml` and `release.yml`, so a false positive
 * here does not merely annoy — it blocks a release. The first version of this file
 * was one hard gate over a mixed population, patched with a hand-maintained
 * allowlist of Tailwind's non-colour values. That is fragile in exactly the wrong
 * direction: a Tailwind upgrade adding a utility would block shipping.
 *
 * So prefixes are split by how confidently an unknown token indicates a bug:
 *
 * | Tier            | Prefixes                                                   | Unknown token |
 * |-----------------|------------------------------------------------------------|---------------|
 * | **colour-only** | `bg` `ring` `fill` `stroke` `caret` `placeholder` `accent`  | **fails**     |
 * | **overloaded**  | `text` `border` `shadow` `outline` `decoration` `from` `via` `to` `divide` | **fails if within 2 edits** of a declared token (a typo); otherwise advisory |
 *
 * The overloaded prefixes carry sizes, weights, sides, widths and line styles, so
 * an unrecognized value there is more often a utility this file has not heard of
 * than a bug — but one 1–2 edits from a real token (`text-contnt` vs `content`) is
 * a misspelling, and a genuine new Tailwind utility is never that close to a
 * project token. That keeps the overloaded prefixes covered without putting a
 * Tailwind upgrade in the path of a release.
 *
 * `bg-panel` — the bug that motivated all of this — is in the always-fails tier.
 * The teeth are kept, the fragility is dropped, and most of the hand-maintained
 * allowlist disappears with it.
 *
 * ⚠️ The advisory uses `process.stderr.write`, NOT `console.warn`. Console output
 * is swallowed in this vitest/jsdom setup, which made an earlier version of this
 * tier silently report nothing at all — a safety net that looked real and was not.
 * Verified with a probe test before relying on it.
 */

const ROOT = resolve(__dirname, "../../..");
const SRC = join(ROOT, "src");
const THEME_FILE = join(ROOT, "src/styles/globals.css");

/**
 * Prefixes whose values are colours, with the specific non-colour utilities each
 * one also accepts. These lists are short and stable — unlike the overloaded
 * prefixes, which is the whole reason for the split.
 */
const COLOUR_ONLY: Record<string, RegExp | null> = {
  bg: /^(none|auto|cover|contain|fixed|local|scroll|center|top|bottom|left|right|(left|right)-(top|bottom)|(no-)?repeat(-[xy]|-round|-space)?|clip-.+|origin-.+|blend-.+|gradient-.+)$/,
  ring: /^(inset|offset(-.+)?)$/,
  fill: /^none$/,
  stroke: /^none$/,
  caret: null,
  placeholder: null,
  accent: /^auto$/,
};

/** Prefixes overloaded with non-colour utilities — typos fail, the rest advise. */
const OVERLOADED = [
  "text",
  "border",
  "shadow",
  "outline",
  "decoration",
  "from",
  "via",
  "to",
  "divide",
];

/** Tailwind's built-in palette families, which need no `@theme` declaration. */
const BUILTIN_FAMILIES =
  /^(slate|gray|grey|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-(50|100|200|300|400|500|600|700|800|900|950)$/;

/** Values any prefix may carry that are never project tokens. */
const UNIVERSAL = /^(black|white|transparent|current|inherit)$/;

/**
 * Noise suppression for the overloaded tier: these are Tailwind's own non-colour
 * values, skipped before the typo check so `border-b` is never mistaken for a
 * misspelled token. Additive only — a missing entry produces an advisory line, not
 * a failure.
 */
const OVERLOADED_NOISE =
  /^([trblxyse](-\d+)?|\d+|none|auto|xs|sm|base|md|lg|[2-9]?xl|left|center|right|justify|start|end|thin|extralight|light|normal|medium|semibold|bold|extrabold|solid|dashed|dotted|double|hidden|clip|ellipsis|wrap|nowrap|balance|pretty|inset|inner|offset|reverse|collapse|separate|slice|clone|top|bottom|opacity|uppercase|lowercase|capitalize|box|color)$/;

function declaredColourTokens(): Set<string> {
  const css = readFileSync(THEME_FILE, "utf8");
  const tokens = new Set<string>();
  for (const match of css.matchAll(/--color-([a-z0-9-]+)\s*:/gi)) {
    tokens.add(match[1].toLowerCase());
  }
  return tokens;
}

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === "__tests__") continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) sourceFiles(full, out);
    else if (/\.(tsx|ts)$/.test(entry)) out.push(full);
  }
  return out;
}

/**
 * Drop comment lines before scanning — `text-selection` inside the prose "block
 * native image-drag and text-selection here" is not a class.
 *
 * Only whole-line comments are removed, never a mid-line `//`, which would eat
 * the rest of a line containing a URL in a string and any real class beside it.
 */
function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .filter((line) => {
      const t = line.trimStart();
      return !t.startsWith("//") && !t.startsWith("*");
    })
    .join("\n");
}

interface Usage {
  file: string;
  utility: string;
  prefix: string;
  token: string;
}

function usages(file: string, prefixes: string[]): Usage[] {
  const text = stripComments(readFileSync(file, "utf8"));
  const re = new RegExp(
    String.raw`(?:^|[\s"'\`{}():])!?(?:[a-z-]+:)*(${prefixes.join("|")})-([a-z][a-z0-9-]*)(?:\/\d{1,3})?(?=[\s"'\`{}():]|$)`,
    "g",
  );
  const seen = new Set<string>();
  const out: Usage[] = [];
  for (const m of text.matchAll(re)) {
    const utility = `${m[1]}-${m[2]}`;
    if (seen.has(utility)) continue;
    seen.add(utility);
    out.push({
      file: file.replace(`${ROOT}/`, ""),
      utility,
      prefix: m[1],
      token: m[2].toLowerCase(),
    });
  }
  return out;
}

/** Edit distance, for "did you mean…". Small inputs, so the naive matrix is fine. */
function editDistance(a: string, b: string): number {
  const rows = Array.from({ length: a.length + 1 }, (_, i) =>
    Array.from({ length: b.length + 1 }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)),
  );
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      rows[i][j] = Math.min(
        rows[i - 1][j] + 1,
        rows[i][j - 1] + 1,
        rows[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
  }
  return rows[a.length][b.length];
}

/** The closest declared token, with its edit distance. */
function nearest(token: string, declared: Set<string>): { name: string; distance: number } | null {
  let best: { name: string; distance: number } | null = null;
  for (const name of declared) {
    const distance = editDistance(token, name);
    if (!best || distance < best.distance) best = { name, distance };
  }
  return best;
}

/**
 * Within this many edits of a declared token, an unknown value is a TYPO rather
 * than an unfamiliar Tailwind utility — `text-contnt` is one edit from `content`,
 * whereas a genuine new Tailwind utility is nowhere near a project token. That
 * distinction is what lets the overloaded prefixes be checked at all without
 * risking a release on a Tailwind upgrade.
 */
const TYPO_DISTANCE = 2;

/**
 * A suggestion only when it is actually close. An earlier version used
 * `max(3, len/2)`, which proposed "base" for "panel" — a wrong guess, and a wrong
 * guess is worse than none.
 */
function suggestion(token: string, declared: Set<string>): string {
  const best = nearest(token, declared);
  return best && best.distance <= TYPO_DISTANCE ? ` — did you mean "${best.name}"?` : "";
}

/** Declared tokens, for a failure message that is actionable without guessing. */
function declaredList(declared: Set<string>): string {
  return [...declared].sort().join(", ");
}

describe("Tailwind colour tokens", () => {
  const declared = declaredColourTokens();

  it("globals.css declares the tokens this test relies on", () => {
    // Sanity: if the parse breaks, every assertion below passes vacuously.
    expect(declared.size).toBeGreaterThan(5);
    for (const expected of ["surface", "elevated", "content", "accent", "border"]) {
      expect(declared, `--color-${expected} should be declared`).toContain(expected);
    }
    // The token whose absence caused the bug must STAY absent, or this test
    // quietly stops meaning anything the moment someone declares it instead of
    // fixing a usage.
    expect(declared).not.toContain("panel");
  });

  it("no colour-only utility names an undeclared token", () => {
    const offenders: string[] = [];

    for (const file of sourceFiles(SRC)) {
      for (const { utility, prefix, token, file: rel } of usages(
        file,
        Object.keys(COLOUR_ONLY),
      )) {
        if (declared.has(token)) continue;
        if (BUILTIN_FAMILIES.test(token) || UNIVERSAL.test(token)) continue;
        const allowed = COLOUR_ONLY[prefix];
        if (allowed?.test(token)) continue;
        offenders.push(`${rel}: ${utility}${suggestion(token, declared)}`);
      }
    }

    expect(
      offenders,
      `These utilities use a colour-only prefix with an undeclared --color-* token, so ` +
        `Tailwind emits NO rule and the style is silently missing — this is exactly how ` +
        `bg-panel left two modals transparent. Declare the token in ` +
        `src/styles/globals.css or use an existing one.\n  ${offenders.join("\n  ")}\n\n` +
        `Declared colour tokens: ${declaredList(declared)}\n`,
    ).toEqual([]);
  });

  it("catches a TYPO'd project token even on an overloaded prefix", () => {
    const typos: string[] = [];
    const unknown: string[] = [];

    for (const file of sourceFiles(SRC)) {
      for (const { utility, token, file: rel } of usages(file, OVERLOADED)) {
        if (declared.has(token)) continue;
        if (BUILTIN_FAMILIES.test(token) || UNIVERSAL.test(token)) continue;
        if (OVERLOADED_NOISE.test(token)) continue;

        const near = nearest(token, declared);
        if (near && near.distance <= TYPO_DISTANCE) {
          typos.push(`${rel}: ${utility} — did you mean "${near.name}"?`);
        } else {
          unknown.push(`${rel}: ${utility}`);
        }
      }
    }

    // Far-from-anything values are advisory: on these prefixes they are more
    // likely a Tailwind utility this file has not heard of than a bug, and this
    // test gates a release. `process.stderr.write`, NOT console.warn — console
    // output is swallowed in this vitest/jsdom setup, which made an earlier
    // version of this tier silently report nothing at all.
    if (unknown.length > 0) {
      process.stderr.write(
        `\n[tailwind-tokens] ${unknown.length} unrecognized value(s) on overloaded ` +
          `prefixes — advisory, not a failure:\n  ${unknown.join("\n  ")}\n`,
      );
    }

    expect(
      typos,
      `These are within ${TYPO_DISTANCE} edits of a declared token, so they are ` +
        `misspellings rather than unfamiliar Tailwind utilities — and a misspelled ` +
        `token emits no rule at all.\n  ${typos.join("\n  ")}\n`,
    ).toEqual([]);
  });
});
