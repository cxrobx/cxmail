import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import EmailFrame from "./EmailFrame";
import SelectionFeedback from "@/components/shared/SelectionFeedback";
import { MAX_SELECTION_LENGTH, wantsFeedback, type FeedbackSelection } from "@/lib/selectionFeedback";
import { useChatStore } from "@/stores/chatStore";
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
      wouldReplaceChat: () => useChatStore.getState().wouldReplaceChat(seed),
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
    const text = domSelection.toString();
    if (!wantsFeedback(text)) { setSelection(null); return; }
    setSelection({ text: text.slice(0, MAX_SELECTION_LENGTH), anchor: range.getBoundingClientRect() });
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
      onInteract={() => { frozen.current = true; }} onAsk={feedback.onAsk} wouldReplaceChat={feedback.wouldReplaceChat} />}
  </>;
}
