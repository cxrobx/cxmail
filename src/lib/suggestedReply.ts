import { api } from "@/lib/tauri";
import { buildQuotedEmailHtml, plainToHtmlParagraphs } from "@/lib/utils";
import { useWindowStore } from "@/stores/windowStore";
import { useMailStore } from "@/stores/mailStore";
import type { NeedsYouRef } from "@/types/email";

/** Fetch again on review: an answered or changed thread has no valid suggestion. */
export async function reviewSuggestedReply(ref: NeedsYouRef): Promise<void> {
  const key = `suggestion:${ref.account_id}|${ref.folder_name}|${ref.uid}`;
  const focusExisting = () => {
    const windows = useWindowStore.getState();
    const existing = windows.windows.find((w) => {
      const context = w.props.replyContext as { accountId?: string; folder?: string; uid?: number } | undefined;
      return w.type === "compose" && context?.accountId === ref.account_id && context.folder === ref.folder_name && context.uid === ref.uid;
    });
    if (existing) { windows.restoreWindow(existing.id); return true; }
    const mail = useMailStore.getState();
    const target = mail.inlineComposeTarget;
    if (target?.accountId === ref.account_id && target.folder === ref.folder_name && target.uid === ref.uid) {
      mail.setSelectedMessage(ref.uid, ref.account_id, ref.folder_name); return true;
    }
    return false;
  };
  if (focusExisting()) return;
  const detail = await api.messages.fetchBody(ref.account_id, ref.folder_name, ref.uid);
  const suggestion = await api.intelligence.suggestion(ref.account_id, ref.folder_name, ref.uid);
  if (!suggestion) throw new Error("This suggestion has changed. Refresh Needs You.");
  if (focusExisting()) return;
  useWindowStore.getState().openWindow({
    key, type: "compose", title: "Review suggested reply",
    props: {
      mode: "reply", accountId: ref.account_id, defaultTo: detail.from_email,
      defaultSubject: /^re:/i.test(detail.subject ?? "") ? detail.subject : `Re: ${detail.subject ?? ""}`,
      defaultBody: plainToHtmlParagraphs(suggestion.body), quotedHtml: buildQuotedEmailHtml(detail),
      suggestedReplyText: suggestion.body,
      inReplyTo: detail.message_id ?? undefined,
      referencesHeader: [detail.references, detail.message_id].filter(Boolean).join(" ") || undefined,
      replyContext: { accountId: ref.account_id, folder: ref.folder_name, uid: ref.uid },
    },
  });
}
