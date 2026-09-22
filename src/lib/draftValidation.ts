/**
 * Deterministic pre-send checks for a compose draft.
 *
 * Pure functions over a plain snapshot — no editor, no IPC, no clock — so the
 * whole set is unit-testable and runs instantly with zero API cost. The LLM
 * review (`api.ai.validateDraft`) is a second, slower wave that covers what
 * cannot be decided mechanically: tone, unanswered questions, pinned-rule
 * compliance.
 *
 * Precision over recall is the rule here. A false positive makes Validate feel
 * like noise and trains the user to ignore it, which costs more than the miss
 * would have. Every heuristic below is anchored (word boundaries, exact-token
 * matching) and declines when it is unsure.
 */
import type { AiDraftFinding } from "@/types/email";

export type FindingSeverity = "error" | "warning" | "info";
export type FindingSource = "instant" | "ai";

/**
 * What clicking the finding's primary button does.
 *
 * `replace` and `subject` are mechanical edits we can make correctly on our
 * own. `ask` hands off to the chat side of the panel with a seeded instruction
 * — the hybrid's whole point, for findings where the fix needs judgement (or
 * information we don't have, like the URL that should have been pasted in).
 * `null` means there is nothing to automate and the user has to act.
 */
export type FindingAction =
  | { kind: "replace"; find: string; with: string; label: string }
  | { kind: "append"; text: string; label: string }
  | { kind: "subject"; with: string; label: string }
  | { kind: "ask"; prompt: string; label: string }
  | null;

export interface Finding {
  id: string;
  severity: FindingSeverity;
  source: FindingSource;
  /** Where in the draft it lives — this is what the overview nav maps. */
  where: string;
  /** 2–3 words, for the chip. */
  short: string;
  title: string;
  detail: string;
  /** Exact text from the draft, when the finding points at a span of it. */
  quote?: string;
  /**
   * The proposed wording, shown whatever the action turns out to be.
   *
   * Kept separate from `action` on purpose: a finding we can't apply
   * mechanically still has advice worth reading, and burying it inside an
   * `ask` prompt would hide the most useful sentence in the finding.
   */
  suggestion?: string;
  /** e.g. "pinned rule" — shown as a badge. */
  badge?: string;
  action: FindingAction;
}

export interface DraftSnapshot {
  subject: string;
  to: { name: string | null; email: string }[];
  cc: { name: string | null; email: string }[];
  bcc: { name: string | null; email: string }[];
  /** Body as plain text, signature and quoted history already excluded. */
  bodyText: string;
  /** Body as HTML — only used for link checks. */
  bodyHtml: string;
  attachmentCount: number;
  /** Present when this draft is a reply; absent for a fresh compose. */
  inReplyTo: string | null;
}

/* ── Address checks ─────────────────────────────────────────── */

/**
 * Deliberately permissive. This is a typo net, not an RFC 5322 parser — the
 * SMTP server is the authority, and a regex strict enough to reject exotic-but-
 * legal addresses would block real mail. It catches the mistakes people
 * actually make: a missing @, a missing TLD, a stray space.
 */
const ADDRESS_RE = /^[^\s@,;]+@[^\s@,;.]+(\.[^\s@,;.]+)+$/;

export function isPlausibleAddress(email: string): boolean {
  return ADDRESS_RE.test(email.trim());
}

/* ── Placeholder checks ─────────────────────────────────────── */

/**
 * Unfilled template slots. `[Name]`, `{{company}}`, `<INSERT DATE>` and the
 * bare markers TODO / TBD / TK / XXX.
 *
 * `{{…}}` and `<ALLCAPS>` are unambiguous syntax — nobody writes those in
 * prose. Square brackets are the hard case, because `[Name]` and `[as we
 * discussed]` are the same shape, so that pattern is marked `ambiguous` and
 * has to clear `looksLikeSlot` below.
 */
const PLACEHOLDER_PATTERNS: { re: RegExp; ambiguous: boolean }[] = [
  { re: /\[[A-Za-z0-9 _.\-]{1,30}\]/g, ambiguous: true },
  { re: /\{\{[^}\n]{1,40}\}\}/g, ambiguous: false },
  { re: /<[A-Z][A-Z0-9 _-]{2,30}>/g, ambiguous: false },
  { re: /\b(TODO|TBD|FIXME|XXX)\b/g, ambiguous: false },
  // TK is a copy-editing "to come" marker. Uppercase + word boundary, so it
  // cannot fire on "tk" inside a word or as a lowercase typo.
  { re: /\bTK\b/g, ambiguous: false },
];

/** Words that mark a slot even in lowercase: "[insert date here]". */
const SLOT_KEYWORDS = new Set([
  "insert", "your", "name", "firstname", "lastname", "date", "company",
  "client", "amount", "price", "cost", "link", "url", "email", "phone",
  "address", "title", "placeholder", "todo", "tbd", "xxx",
]);

function wordCount(s: string): number {
  return s.split(/\s+/).filter(Boolean).length;
}

/**
 * Is a bracketed run a template slot, or is it just prose in brackets?
 *
 * Two signals, either sufficient. **Capitalisation** — real slots are written
 * `[Name]` / `[INSERT DATE]`, while bracketed asides are lowercase (`[as we
 * discussed]`, `[sic]`, `[emphasis mine]`). And a **keyword**, which rescues
 * the lowercase slots people do write, like `[insert date here]`.
 *
 * Length alone cannot separate these — `[as we discussed]` and `[First Name]`
 * are both short — which is why the word cap is a backstop rather than the
 * test.
 */
function looksLikeSlot(inner: string): boolean {
  if (wordCount(inner) > 3) return false;
  if (/^[A-Z]/.test(inner)) return true;
  return inner
    .toLowerCase()
    .split(/[\s_.-]+/)
    .some((w) => SLOT_KEYWORDS.has(w));
}

export function findPlaceholders(text: string): string[] {
  const hits: string[] = [];
  for (const { re, ambiguous } of PLACEHOLDER_PATTERNS) {
    // Fresh lastIndex per call — these are module-level /g regexes.
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      const raw = m[0];
      const inner = raw.replace(/^[[{<]+|[\]}>]+$/g, "").trim();
      if (ambiguous && !looksLikeSlot(inner)) continue;
      if (!hits.includes(raw)) hits.push(raw);
    }
  }
  return hits;
}

/* ── Link checks ────────────────────────────────────────────── */

const DEAD_HREFS = new Set(["", "#", "about:blank", "http://", "https://"]);

export interface DeadLink {
  href: string;
  text: string;
}

/**
 * Anchors whose href goes nowhere. Uses DOMParser rather than a regex because
 * the body is real HTML from TipTap and attribute order/quoting is not stable.
 * `example.com` counts as dead — it is the placeholder domain, never a real
 * destination in outgoing mail.
 */
export function findDeadLinks(html: string): DeadLink[] {
  if (!html.trim()) return [];
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(html, "text/html");
  } catch {
    return [];
  }
  const out: DeadLink[] = [];
  for (const a of Array.from(doc.querySelectorAll("a"))) {
    const href = (a.getAttribute("href") ?? "").trim();
    const text = (a.textContent ?? "").trim();
    const isDead =
      DEAD_HREFS.has(href.toLowerCase()) ||
      /^https?:\/\/(www\.)?example\.(com|org|net)\/?$/i.test(href);
    if (isDead) out.push({ href, text });
  }
  return out;
}

/* ── Greeting checks ────────────────────────────────────────── */

const GREETING_RE =
  /^(\s*)(hi|hey|hello|dear|good\s+(?:morning|afternoon|evening))\b([ \t]*)([^\n,!]{0,60})/i;

/**
 * Titles that carry a period and are followed by more of the name, so the
 * period is NOT the end of the greeting. Without this, "Bro. Ellis" — the
 * exact form a pinned rule exists to enforce — parses as "Bro".
 */
const HONORIFICS = new Set([
  "mr", "mrs", "ms", "dr", "prof", "rev", "fr", "st", "hon", "sr", "jr",
  "bro", "sis", "capt", "sgt", "lt", "col", "gen", "atty", "esq",
]);

/** Lowercase name particles: van Dijk, de la Cruz, bin Salman. */
const PARTICLES = new Set(["van", "von", "de", "del", "da", "di", "la", "le", "bin", "al"]);

const MAX_NAME_TOKENS = 4;

export interface Greeting {
  /** The greeting exactly as written, e.g. "Hi Bro. Ellis" (no trailing comma). */
  line: string;
  /** Just the addressed name, e.g. "Bro. Ellis". Empty when there is none. */
  name: string;
}

/**
 * Pull the greeting off the first non-empty line.
 *
 * The hard part is knowing where the name ends, because a period is both an
 * honorific separator ("Bro. Ellis") and a sentence terminator ("Hi Sam. I
 * wanted to ask…"). The walk below accepts name-shaped tokens and stops at the
 * first period that is NOT an honorific or an initial — which distinguishes
 * those two cases without a name dictionary.
 *
 * Lowercase words are rejected, so "Hi team," yields an empty name and is
 * treated as unaddressed rather than as a person called "team".
 */
export function parseGreeting(bodyText: string): Greeting | null {
  const firstLine = bodyText.split("\n").find((l) => l.trim().length > 0);
  if (!firstLine) return null;
  const m = GREETING_RE.exec(firstLine);
  if (!m) return null;

  const [, leading, greetWord, gap, rest] = m;
  const nameStart = leading.length + greetWord.length + gap.length;

  let nameEnd = 0;
  let taken = 0;
  const tokenRe = /\S+/g;
  let t: RegExpExecArray | null;
  while ((t = tokenRe.exec(rest)) !== null && taken < MAX_NAME_TOKENS) {
    const word = t[0];
    const bare = word.replace(/\.$/, "").toLowerCase();
    const isName = /^[A-Z][\p{L}'’-]*\.?$/u.test(word);
    const isParticle = PARTICLES.has(bare) && taken > 0;
    if (!isName && !isParticle) break;

    nameEnd = t.index + word.length;
    taken++;

    // A trailing period ends the greeting unless it's an honorific ("Bro.")
    // or an initial ("J."), both of which are followed by more of the name.
    if (word.endsWith(".") && !HONORIFICS.has(bare) && bare.length > 1) break;
  }

  const rawName = rest.slice(0, nameEnd).trim();
  // "Sam." → "Sam", but "Bro." keeps its period.
  const name = HONORIFICS.has(rawName.replace(/\.$/, "").toLowerCase())
    ? rawName
    : rawName.replace(/\.$/, "");

  const line = firstLine.slice(leading.length, nameStart + nameEnd).trimEnd();
  return { line, name };
}

/**
 * Does the greeting plausibly address one of the recipients?
 *
 * Generous on purpose — nicknames, honorifics and last-name forms all have to
 * pass, because a false "wrong name" alarm on every "Hi Bro. Ellis," would be
 * worse than missing a genuine mixup. It matches when any token of the
 * greeting matches any token of a recipient's display name, or the local part
 * of their address. An empty greeting ("Hi,") always passes.
 */
export function greetingMatchesRecipient(
  greetingName: string,
  recipients: { name: string | null; email: string }[],
): boolean {
  if (!greetingName) return true;
  if (recipients.length === 0) return true;

  const strip = (s: string) =>
    s
      .toLowerCase()
      .replace(/\b(mr|mrs|ms|dr|prof|bro|sis|sir|madam)\b\.?/g, " ")
      .replace(/[^a-z0-9\s]/g, " ")
      .split(/\s+/)
      .filter((t) => t.length > 1);

  const greetTokens = strip(greetingName);
  if (greetTokens.length === 0) return true;

  const recipientTokens = new Set<string>();
  for (const r of recipients) {
    for (const t of strip(r.name ?? "")) recipientTokens.add(t);
    const local = r.email.split("@")[0] ?? "";
    // first.last / first_last / first-last all split into their parts.
    for (const t of strip(local.replace(/[._-]+/g, " "))) recipientTokens.add(t);
  }

  return greetTokens.some((t) => recipientTokens.has(t));
}

/* ── Reply-prefix check ─────────────────────────────────────── */

const REPLY_PREFIX_RE = /^\s*(re|aw|antw|sv|vs|odp|res)\s*(\[\d+\])?\s*:/i;

export function hasReplyPrefix(subject: string): boolean {
  return REPLY_PREFIX_RE.test(subject);
}

/* ── Attachment mention ─────────────────────────────────────── */

/**
 * Kept byte-identical to `utils.detectAttachmentMention`, which drives the
 * existing pre-send warning. Duplicated rather than imported so this module
 * stays dependency-free and testable in isolation; the shared-constant test
 * pins them together.
 */
export const ATTACHMENT_MENTION_RE =
  /\b(?:i(?:'ve|'m|\s+have|\s+am)\s+attach(?:ed|ing)|(?:please\s+)?(?:find|see)\s+(?:the\s+)?attach(?:ed|ments?)|attach(?:ed|ments?|ing)|enclos(?:ed|ing|ure))\b/i;

/** The matched phrase, so the finding can quote the actual sentence. */
export function attachmentMentionSentence(bodyText: string): string | null {
  const m = ATTACHMENT_MENTION_RE.exec(bodyText);
  if (!m) return null;
  // Widen to the surrounding sentence so the quote is locatable in the draft.
  const idx = m.index;
  const start = Math.max(
    0,
    ...[". ", "! ", "? ", "\n"].map((p) => {
      const i = bodyText.lastIndexOf(p, idx);
      return i === -1 ? 0 : i + p.length;
    }),
  );
  const tail = bodyText.slice(idx);
  const endRel = tail.search(/[.!?](\s|$)|\n/);
  const end = endRel === -1 ? bodyText.length : idx + endRel + 1;
  return bodyText.slice(start, end).trim() || m[0];
}

/* ── The check runner ───────────────────────────────────────── */

/**
 * Every deterministic finding for a draft, ordered by severity then by
 * position in the message (recipients → subject → body → closing), which is
 * the order the overview nav renders them in.
 */
export function validateDraft(draft: DraftSnapshot): Finding[] {
  const findings: Finding[] = [];
  const all = [...draft.to, ...draft.cc, ...draft.bcc];
  const body = draft.bodyText.trim();

  /* Recipients */
  if (draft.to.length === 0) {
    findings.push({
      id: "no-recipients",
      severity: "error",
      source: "instant",
      where: "To",
      short: "No recipient",
      title: "No one is in the To field",
      detail: "This draft cannot be sent until it has at least one recipient.",
      action: null,
    });
  }

  const bad = all.filter((r) => !isPlausibleAddress(r.email));
  for (const r of bad) {
    findings.push({
      id: `bad-address:${r.email}`,
      severity: "error",
      source: "instant",
      where: "To",
      short: "Bad address",
      title: `"${r.email}" doesn't look like an address`,
      detail:
        "It's missing an @ or a domain suffix. Sending will fail, or the message will silently go nowhere.",
      quote: r.email,
      action: null,
    });
  }

  const seen = new Map<string, number>();
  for (const r of all) {
    const key = r.email.trim().toLowerCase();
    seen.set(key, (seen.get(key) ?? 0) + 1);
  }
  for (const [email, n] of seen) {
    if (n < 2) continue;
    findings.push({
      id: `duplicate:${email}`,
      severity: "warning",
      source: "instant",
      where: "To",
      short: "Duplicate",
      title: `${email} appears ${n} times`,
      detail:
        "The same address is in more than one field. They'll get one copy, but Cc/Bcc placement may not be what you intended.",
      quote: email,
      action: null,
    });
  }

  /* Subject */
  if (!draft.subject.trim()) {
    findings.push({
      id: "empty-subject",
      severity: "warning",
      source: "instant",
      where: "Subject",
      short: "No subject",
      title: "The subject line is empty",
      detail: "Empty subjects get deprioritised by both people and spam filters.",
      action: body
        ? { kind: "ask", prompt: "Suggest a subject line for this draft.", label: "Suggest one" }
        : null,
    });
  }

  if (hasReplyPrefix(draft.subject) && !draft.inReplyTo) {
    findings.push({
      id: "reply-no-target",
      severity: "info",
      source: "instant",
      where: "Subject",
      short: "Orphan reply",
      title: 'Subject starts "Re:" but this isn\'t threaded to anything',
      detail:
        "Without In-Reply-To the message starts a new thread in any client that doesn't fall back to subject matching. If this is a reply, open it from the original message instead.",
      quote: draft.subject,
      action: null,
    });
  }

  /* Body */
  if (!body) {
    findings.push({
      id: "empty-body",
      severity: "error",
      source: "instant",
      where: "Body",
      short: "Empty body",
      title: "The message body is empty",
      detail: "Nothing has been written yet, aside from any signature.",
      action: null,
    });
  }

  const greeting = parseGreeting(draft.bodyText);
  if (greeting && !greetingMatchesRecipient(greeting.name, all)) {
    const target = draft.to[0];
    const suggested = target?.name?.split(/\s+/)[0] ?? null;
    findings.push({
      id: "greeting-mismatch",
      severity: "warning",
      source: "instant",
      where: "Greeting",
      short: "Wrong name",
      title: `Greeting says "${greeting.name}" — that isn't the recipient`,
      detail: target
        ? `The only recipient is ${target.email}. This is the shape of a copy-paste from another thread.`
        : "The addressed name doesn't match anyone on this message.",
      quote: greeting.line,
      suggestion: suggested ? greeting.line.replace(greeting.name, suggested) : undefined,
      action: suggested
        ? {
            kind: "replace",
            find: greeting.line,
            with: greeting.line.replace(greeting.name, suggested),
            label: `Use "${suggested}"`,
          }
        : null,
    });
  }

  if (draft.attachmentCount === 0) {
    const sentence = attachmentMentionSentence(draft.bodyText);
    if (sentence) {
      findings.push({
        id: "attachment-mention",
        severity: "warning",
        source: "instant",
        where: "Body",
        short: "No attachment",
        title: "The body mentions an attachment — there isn't one",
        detail: "Attach the file, or drop the mention before sending.",
        quote: sentence,
        action: null,
      });
    }
  }

  for (const p of findPlaceholders(draft.bodyText)) {
    findings.push({
      id: `placeholder:${p}`,
      severity: "error",
      source: "instant",
      where: "Body",
      short: "Placeholder",
      title: `Unfilled placeholder ${p}`,
      detail: "This looks like a template slot that was never filled in.",
      quote: p,
      action: null,
    });
  }

  for (const link of findDeadLinks(draft.bodyHtml)) {
    findings.push({
      id: `dead-link:${link.text || link.href}`,
      severity: "warning",
      source: "instant",
      where: "Body",
      short: "Dead link",
      title: link.text ? `"${link.text}" links nowhere` : "A link has no destination",
      detail: `The href is ${link.href ? `"${link.href}"` : "empty"}. The URL was probably never pasted in.`,
      quote: link.text || undefined,
      action: null,
    });
  }

  return sortFindings(findings);
}

const SEVERITY_RANK: Record<FindingSeverity, number> = { error: 0, warning: 1, info: 2 };
const WHERE_RANK = ["To", "Subject", "Greeting", "Body", "Closing", "Thread"];

export function sortFindings(findings: Finding[]): Finding[] {
  return [...findings].sort((a, b) => {
    const s = SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity];
    if (s !== 0) return s;
    const wa = WHERE_RANK.indexOf(a.where);
    const wb = WHERE_RANK.indexOf(b.where);
    return (wa === -1 ? WHERE_RANK.length : wa) - (wb === -1 ? WHERE_RANK.length : wb);
  });
}

/* ── The AI wave ────────────────────────────────────────────── */

const CATEGORY_SHORT: Record<AiDraftFinding["category"], string> = {
  pinned_rule: "Pinned rule",
  unanswered_question: "Unanswered",
  missing_context: "Missing info",
  clarity: "Unclear",
  next_step: "No next step",
  consistency: "Contradiction",
  tone: "Tone",
};

const CATEGORY_BADGE: Partial<Record<AiDraftFinding["category"], string>> = {
  pinned_rule: "pinned rule",
  unanswered_question: "from the thread",
  consistency: "from the thread",
};

/**
 * Turn a backend finding into the shape the panel renders.
 *
 * The interesting part is choosing the action, and it's decided by what we
 * actually have rather than by category:
 *
 * - **quote + suggestion** → a mechanical `replace`. Safe because the Rust side
 *   has already verified the quote occurs verbatim in the body.
 * - **suggestion, no quote, at the end** → `append`. This is the "no next step"
 *   case: there's nothing to replace because the problem is an absence.
 * - **anything else** → `ask`, which hands the finding to the chat side with a
 *   seeded instruction. That's the hybrid doing its job: mechanical where we
 *   can be certain, conversational where judgement is required.
 */
export function fromAiFinding(f: AiDraftFinding, index: number): Finding {
  const isSubject = f.where.toLowerCase() === "subject";
  const atEnd = ["closing", "body"].includes(f.where.toLowerCase());

  let action: FindingAction;
  if (isSubject && f.suggestion) {
    action = { kind: "subject", with: f.suggestion, label: "Use this subject" };
  } else if (f.quote && f.suggestion) {
    action = { kind: "replace", find: f.quote, with: f.suggestion, label: "Apply" };
  } else if (f.suggestion && atEnd) {
    action = { kind: "append", text: f.suggestion, label: "Add it" };
  } else {
    action = {
      kind: "ask",
      prompt: `${f.title}. ${f.detail} Revise the draft to fix this, keeping my voice.`,
      label: "Fix in chat",
    };
  }

  return {
    id: `ai:${index}:${f.category}`,
    severity: f.severity,
    source: "ai",
    where: f.where,
    short: CATEGORY_SHORT[f.category] ?? "Review",
    title: f.title,
    detail: f.detail,
    quote: f.quote ?? undefined,
    suggestion: f.suggestion ?? undefined,
    badge: CATEGORY_BADGE[f.category],
    action,
  };
}

export function countBySeverity(findings: Finding[]): Record<FindingSeverity, number> {
  return {
    error: findings.filter((f) => f.severity === "error").length,
    warning: findings.filter((f) => f.severity === "warning").length,
    info: findings.filter((f) => f.severity === "info").length,
  };
}
