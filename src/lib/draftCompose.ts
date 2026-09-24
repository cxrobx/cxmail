import type { MessageDetail, OutgoingAttachment } from "@/types/email";
import { api } from "@/lib/tauri";
import { draftWindowKey, useWindowStore } from "@/stores/windowStore";

/**
 * Build the compose-window props for reopening an existing draft.
 *
 * This exists as one function because it used to exist as two: `MessageList`
 * and `MessageListItem` each carried their own copy of the prop object, and
 * every field that had to be threaded through — recipients, Bcc, attachments,
 * and finally the reply headers — had to be remembered twice. Each one was
 * forgotten at least once. A draft's threading headers were the last to be
 * missed, and that miss unthreaded every reply CXMail sent for three months
 * (gotcha #39).
 *
 * The rule the two copies kept violating: a draft's MIME is REBUILT from these
 * props on every save and send. Anything not passed here is not merely hidden
 * in the UI — it is deleted from the draft.
 */
export function buildDraftComposeProps(
  detail: MessageDetail,
  accountId: string,
  folder: string,
  uid: number,
  defaultAttachments: OutgoingAttachment[],
): Record<string, unknown> {
  return {
    mode: "new",
    accountId,
    // ALL To recipients, not just the first — taking to_list[0] silently
    // dropped every other recipient when reopening a multi-recipient draft.
    defaultTo: detail.to_list.map((a) => a.email),
    defaultCc: detail.cc_list.map((a) => a.email),
    // Bcc round-trips on draft reopen (gotcha #25 sibling fix) — without it,
    // re-saving the draft rebuilds the MIME and drops the Bcc.
    defaultBcc: detail.bcc_list.map((a) => a.email),
    defaultSubject: detail.subject || "",
    defaultBody: detail.sanitized_html || "",
    // The draft's OWN threading headers, passed through verbatim — NOT
    // recomputed from the parent the way ReadingPane's reply path does it.
    // The draft already carries the correct values; recomputing here would
    // need a parent this code does not have.
    inReplyTo: detail.in_reply_to || undefined,
    referencesHeader: detail.references || undefined,
    draftContext: { accountId, folder, uid },
    defaultAttachments,
    // The draft's own From, so a draft saved from an alias reopens FROM the
    // alias. Without it the composer falls back to the primary and the next
    // save silently rewrites the From. `resolveFrom` ignores an address the
    // account cannot send as, so a stale one degrades to the primary.
    fromAddress: detail.from_email || undefined,
  };
}

/** Draft opens in flight, by window key — see `openDraftForEdit`. */
const opening = new Map<string, Promise<string | null>>();

/**
 * Open the compose window for an existing draft. Resolves to the window id, or
 * null if the draft could not be loaded.
 *
 * Idempotent, in three layers, because the open is SLOW — it fetches the body
 * and (when the draft carries real attachments) the attachment bytes over
 * IMAP, and a user who sees nothing happen clicks again:
 *
 *  1. Already open → focus that window. No fetch.
 *  2. Already opening → join the pending open. One fetch, one window, however
 *     many clicks land while it runs. A double-click is three of them (two
 *     clicks and the dblclick), so this is the common case, not the edge.
 *  3. The window itself is opened with the draft's key, so the store refuses to
 *     stack a second one even if a caller bypasses this function.
 *
 * Like `buildDraftComposeProps`, this is one function because it used to be
 * two (`MessageList` and `MessageListItem`), and a guard that lives in one copy
 * is a guard that half the clicks skip.
 */
export function openDraftForEdit(accountId: string, folder: string, uid: number): Promise<string | null> {
  const key = draftWindowKey(accountId, folder, uid);
  const already = useWindowStore.getState().focusWindowByKey(key);
  if (already) return Promise.resolve(already);
  const pending = opening.get(key);
  if (pending) return pending;

  const task = (async (): Promise<string | null> => {
    try {
      const detail = await api.messages.fetchBody(accountId, folder, uid);
      // Reload the draft's real attachments so they're visible in compose AND
      // preserved on re-save/send: the draft MIME carries only metadata, and
      // without the bytes, editing the draft silently drops the files (gotcha
      // #25). Skip the IMAP round-trip when there are only inline (cid:) images.
      const defaultAttachments = detail.attachments.some((a) => !a.is_inline)
        ? await api.compose.fetchOutgoingAttachments(accountId, folder, uid).catch((e) => {
            console.error("Failed to load draft attachments (opening without them):", e);
            return [] as OutgoingAttachment[];
          })
        : [];
      return useWindowStore.getState().openWindow({
        type: "compose",
        title: detail.subject || "Draft",
        props: buildDraftComposeProps(detail, accountId, folder, uid, defaultAttachments),
        key,
      });
    } catch (e) {
      // "The draft wouldn't open" is a bug report with no detail unless this
      // reaches CXMail.log — the console is invisible in release builds.
      console.error("Failed to open draft for editing:", e);
      void api.system.logClientError("draft-open", `${folder}/${uid}: ${String(e)}`);
      return null;
    } finally {
      opening.delete(key);
    }
  })();
  opening.set(key, task);
  return task;
}
