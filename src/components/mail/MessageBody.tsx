import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import EmailFrame from "./EmailFrame";
import SelectionFeedback from "@/components/shared/SelectionFeedback";
import { MAX_SELECTION_LENGTH, type FeedbackSelection } from "@/lib/selectionFeedback";
import { useChatStore } from "@/stores/chatStore";
import { useWindowStore } from "@/stores/windowStore";
import { buildQuotedEmailHtml, plainToHtmlParagraphs } from "@/lib/utils";
import type { MessageDetail } from "@/types/email";

interface Props {
  detail: MessageDetail;
  accountId: string | null;
  folder: string | null;
  /** Thread cards can hide trimmed history without changing email context. */
  html?: string;
}

/** HTML and plain-text mail share the same feedback actions and email identity. */
export default function MessageBody({ detail, accountId, folder, html }: Props) {
  const plainRoot = useRef<HTMLPreElement>(null);
  const [selection, setSelection] = useState<FeedbackSelection | null>(null);
  const frozen = useRef(false);
  const close = useCallback(() => { frozen.current = false; setSelection(null); }, []);
  useEffect(close, [detail, close]);
  const feedback = useMemo(() => {
    if (!accountId || !folder) return undefined;
    const seed = { account_id: accountId, folder, uid: detail.uid };
    return {
      onAsk: (quote: string, note: string) => useChatStore.getState().askAboutPassage(seed, quote, note),
      onUseInReply: (text: string) => {
        // A new draft preserves any reply the user already has open.
        useWindowStore.getState().openWindow({
          type: "compose", title: `Re: ${detail.subject || "(no subject)"}`,
          props: {
            mode: "reply", accountId, defaultTo: detail.from_email,
            defaultSubject: /^re:/i.test(detail.subject ?? "") ? detail.subject : `Re: ${detail.subject || ""}`,
            defaultBody: plainToHtmlParagraphs(text), quotedHtml: buildQuotedEmailHtml(detail),
            inReplyTo: detail.message_id || undefined,
            referencesHeader: [detail.references, detail.message_id].filter(Boolean).join(" ") || undefined,
            replyContext: { accountId, folder, uid: detail.uid },
          },
        });
      },
    };
  }, [detail, accountId, folder]);

  const updateSelection = () => {
    if (frozen.current || !feedback) return;
    const domSelection = document.getSelection();
    const range = domSelection?.rangeCount === 1 ? domSelection.getRangeAt(0) : null;
    if (!domSelection || domSelection.isCollapsed || !range ||
      !plainRoot.current?.contains(range.startContainer) || !plainRoot.current.contains(range.endContainer)) {
      setSelection(null); return;
    }
    setSelection({ text: domSelection.toString().slice(0, MAX_SELECTION_LENGTH), anchor: range.getBoundingClientRect() });
  };
  useEffect(() => {
    const cleared = () => { if (!frozen.current && document.getSelection()?.isCollapsed) setSelection(null); };
    document.addEventListener("selectionchange", cleared);
    return () => document.removeEventListener("selectionchange", cleared);
  }, []);

  const bodyHtml = html ?? detail.sanitized_html;
  if (bodyHtml) return <EmailFrame key={`${accountId}|${folder}|${detail.uid}`} html={bodyHtml} senderEmail={detail.from_email} feedback={feedback} />;
  return <>
    <pre ref={plainRoot} onMouseUp={updateSelection} onKeyUp={(event) => { if (event.key !== "Escape") updateSelection(); }}
      className="whitespace-pre-wrap p-4 text-sm text-content-secondary">{detail.plain_text}</pre>
    {selection && feedback && <SelectionFeedback mode="readonly" selection={selection} onClose={close}
      onInteract={() => { frozen.current = true; }} onAsk={feedback.onAsk} onUseInReply={feedback.onUseInReply} />}
  </>;
}
