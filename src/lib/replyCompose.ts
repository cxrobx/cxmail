import { buildQuotedEmailHtml, plainToHtmlParagraphs } from "@/lib/utils";
import type { MessageDetail } from "@/types/email";

export type ReplyComposeMode = "new" | "reply" | "reply-all" | "forward";

export interface ReplyTarget {
  accountId: string | null;
  folder: string | null;
  uid: number | null;
  /** The receiving account's address. Reply-all never Cc's it. */
  accountEmail?: string | null;
  /** Send-as aliases, excluded from reply-all like the account address. */
  aliasEmails?: (string | null | undefined)[];
}

/**
 * Compose props for a new message answering `detail`. The one builder for
 * reply, reply-all and forward: the reading pane's inline reply and pop-out,
 * and selection feedback's "Use in reply", all go through it, so a threading
 * field cannot be remembered in one copy and forgotten in another (#39).
 */
export function buildReplyComposeProps(
  detail: MessageDetail,
  mode: ReplyComposeMode,
  target: ReplyTarget,
  body?: string | null,
): Record<string, unknown> {
  const subjectPrefix = mode === "forward" ? "Fwd:" : "Re:";
  const defaultSubject = detail.subject?.startsWith(subjectPrefix)
    ? detail.subject
    : `${subjectPrefix} ${detail.subject || ""}`;

  const props: Record<string, unknown> = {
    mode,
    accountId: target.accountId || undefined,
    defaultSubject,
    quotedHtml: buildQuotedEmailHtml(detail),
  };

  if (mode === "reply" || mode === "reply-all") {
    props.defaultTo = detail.from_email;
    if (body) props.defaultBody = plainToHtmlParagraphs(body);
    props.inReplyTo = detail.message_id || undefined;
    props.referencesHeader = detail.message_id
      ? [detail.references, detail.message_id].filter(Boolean).join(" ")
      : undefined;
    props.replyContext = target.accountId && target.folder && target.uid
      ? { accountId: target.accountId, folder: target.folder, uid: target.uid }
      : undefined;
    if (mode === "reply-all") {
      // Self-set = the receiving account's address + its send-as aliases.
      // Normalize with the codebase's lowercase+trim convention (no shared
      // helper exists). Reply-all should never Cc the user, and should dedupe
      // the sender (already To) out of Cc.
      const norm = (e: string) => e.toLowerCase().trim();
      const self = new Set(
        [target.accountEmail, ...(target.aliasEmails ?? [])].filter(Boolean).map((e) => norm(e!)),
      );
      const fromIsSelf = self.has(norm(detail.from_email || ""));
      const dedupeExclude = (emails: string[], exclude: Set<string>) => {
        const seen = new Set<string>();
        return emails.filter((em) => {
          const k = norm(em);
          if (!k || exclude.has(k) || seen.has(k)) return false;
          seen.add(k);
          return true;
        });
      };
      if (fromIsSelf) {
        // Reply-all to your OWN sent message → reply to the original
        // recipients, not yourself.
        const toEmails = dedupeExclude(detail.to_list.map((a) => a.email), self);
        props.defaultTo = toEmails.length ? toEmails : detail.from_email;
        props.defaultCc = dedupeExclude(
          detail.cc_list.map((a) => a.email),
          new Set([...self, ...toEmails.map(norm)]),
        );
      } else {
        props.defaultTo = detail.from_email; // unchanged for the common case
        props.defaultCc = dedupeExclude(
          detail.to_list.concat(detail.cc_list).map((a) => a.email),
          new Set([...self, norm(detail.from_email || "")]),
        );
      }
    }
  }
  return props;
}
