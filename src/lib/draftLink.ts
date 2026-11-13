import { api } from "@/lib/tauri";
import { openDraftForEdit } from "@/lib/draftCompose";

/**
 * What a `cxmail://draft/…` link names. Two forms:
 *
 *  - `cxmail://draft/<draft_id>` — the stable id (survives every re-save, and
 *    means the same draft on every Mac, whose account ids differ).
 *  - `cxmail://draft/<account_id>/<uid>` — one revision, for a draft with no
 *    CXMail identity.
 *
 * Agents hand these out (the MCP's compose_draft / edit_draft / open_draft
 * results); the MCP's own `open_draft` normally goes over the socket bridge
 * instead and only falls back to the link when the app isn't running.
 */
export type DraftLink = { draftId: string } | { accountId: string; uid: number };

const DRAFT_LINK = /^cxmail:\/\/draft\/([^/?#]+)(?:\/(\d+))?\/?(?:[?#].*)?$/i;

export function parseDraftLink(url: string): DraftLink | null {
  const m = DRAFT_LINK.exec(url.trim());
  if (!m) return null;
  let first: string;
  try {
    first = decodeURIComponent(m[1]);
  } catch {
    return null;
  }
  if (!first || first.length > 128) return null;
  if (m[2] === undefined) return { draftId: first };
  const uid = Number(m[2]);
  if (!Number.isSafeInteger(uid) || uid <= 0 || uid > 0xffffffff) return null;
  return { accountId: first, uid };
}

/** Attempts × interval for a draft that has not reached the local DB yet. */
const ATTEMPTS = 4;
const RETRY_MS = 750;

/**
 * Resolve a draft link against this Mac's DB and open its compose window.
 * Resolves to the window id, or null when the draft isn't known here.
 */
export async function openDraftLink(link: DraftLink): Promise<string | null> {
  for (let attempt = 0; attempt < ATTEMPTS; attempt++) {
    if (attempt > 0) await new Promise((r) => setTimeout(r, RETRY_MS));
    const loc =
      "draftId" in link
        ? await api.compose.resolveDraftLink({ draftId: link.draftId })
        : await api.compose.resolveDraftLink({ accountId: link.accountId, uid: link.uid });
    if (loc) return openDraftForEdit(loc.account_id, loc.folder, loc.uid);
  }
  return null;
}
