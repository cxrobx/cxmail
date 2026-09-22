import type { CalendarEvent } from "@/types/email";

export type MeetingProvider =
  | "zoom"
  | "meet"
  | "teams"
  | "webex"
  | "jitsi"
  | "generic";

export interface MeetingLink {
  url: string;
  provider: MeetingProvider;
}

// Provider-specific URL patterns, scanned in priority order. Each regex is
// anchored on the provider host so a bare mention of "zoom" in prose won't
// match — only a real join URL does.
const PROVIDER_PATTERNS: { provider: MeetingProvider; regex: RegExp }[] = [
  {
    provider: "zoom",
    // zoom.us/j/<id>, /my/<name>, /w/<id> (+ vanity subdomains like company.zoom.us)
    regex: /https?:\/\/(?:[\w-]+\.)?zoom\.us\/(?:j|my|w)\/[^\s"'<>)]+/i,
  },
  {
    provider: "meet",
    regex: /https?:\/\/meet\.google\.com\/[^\s"'<>)]+/i,
  },
  {
    provider: "teams",
    // Standard meetup-join links plus the consumer teams.live.com variant.
    regex: /https?:\/\/teams\.microsoft\.com\/l\/meetup-join\/[^\s"'<>)]+|https?:\/\/teams\.live\.com\/[^\s"'<>)]+/i,
  },
  {
    provider: "webex",
    regex: /https?:\/\/(?:[\w-]+\.)?webex\.com\/[^\s"'<>)]+/i,
  },
  {
    provider: "jitsi",
    regex: /https?:\/\/meet\.jit\.si\/[^\s"'<>)]+/i,
  },
];

// A `location` that is itself a bare URL (some clients put the join link there).
const BARE_URL = /^\s*(https?:\/\/[^\s"'<>)]+)\s*$/i;

function scan(text: string | null | undefined): MeetingLink | null {
  if (!text) return null;
  for (const { provider, regex } of PROVIDER_PATTERNS) {
    const match = text.match(regex);
    if (match) return { url: match[0], provider };
  }
  return null;
}

/**
 * The fenced block CXMail writes into an event description when it attaches a
 * Zoom meeting (`email::zoom_sync::zoom_block`). Fenced so the backend can find,
 * replace and remove it exactly — but the fence markers are machine plumbing and
 * must never be shown to a person.
 *
 * Non-greedy, so two blocks (which should not happen, but might after a manual
 * edit) are each removed rather than everything between the first and last.
 * Also swallows the blank line the backend inserts before it, so removing the
 * block does not leave a gap in the middle of the user's prose.
 */
const MANAGED_BLOCK = /\n{0,2}\[cxmail:zoom\][\s\S]*?\[\/cxmail:zoom\]\n{0,2}/g;

/**
 * A description with CXMail's managed join block removed, for display.
 *
 * The block is redundant wherever a Join button is rendered from the same URL,
 * and its `[cxmail:zoom]` markers read as junk. Returns `null` when nothing
 * printable is left, so callers can skip the paragraph entirely instead of
 * rendering an empty one.
 *
 * Display-only — never write this back to the event, or the join link is gone.
 */
export function descriptionForDisplay(
  description: string | null | undefined,
): string | null {
  if (!description) return null;
  const stripped = description.replace(MANAGED_BLOCK, "\n\n").trim();
  return stripped.length > 0 ? stripped : null;
}

/**
 * Extract the first recognizable meeting-join URL from an event, scanning
 * `location` → `description` → `raw_ics` in that order. Falls back to a bare
 * URL sitting alone in `location`. Returns `null` if nothing is found.
 *
 * Pure function — unit-testable, no side effects.
 */
export function extractMeetingLink(event: CalendarEvent): MeetingLink | null {
  if (event.hangout_link) {
    return { url: event.hangout_link, provider: "meet" };
  }
  for (const field of [event.location, event.description, event.raw_ics]) {
    const found = scan(field);
    if (found) return found;
  }

  // Generic fallback: location is itself a bare https:// URL.
  const bare = event.location?.match(BARE_URL);
  if (bare) return { url: bare[1], provider: "generic" };

  return null;
}
