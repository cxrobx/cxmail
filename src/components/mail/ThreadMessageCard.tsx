import { useMemo, useState } from "react";
import { Pencil } from "lucide-react";
import EmailFrame from "./EmailFrame";
import AttachmentList from "./AttachmentList";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import { cn, formatRelativeDate, getInitials } from "@/lib/utils";
import { splitTrailingQuote } from "@/lib/quoteToggle";
import type { MessageDetail, MessageSummary } from "@/types/email";

interface ThreadMessageCardProps {
  member: MessageSummary;
  /** This member is an UNSENT draft (it lives in the account's drafts
   * folder). It renders as a draft and is never expandable — see the card
   * doc. */
  isDraft?: boolean;
  isExpanded: boolean;
  detail: MessageDetail | null;
  isLoading: boolean;
  error: string | null;
  onToggle: () => void;
  onRetry?: () => void;
  /** Account + folder used for AttachmentList downloads when the member's
   * own folder_name is null (rare; happens for unified-inbox synthesized
   * rows). */
  fallbackAccountId?: string | null;
  fallbackFolder?: string | null;
}

/**
 * One thread member card. Three visual modes:
 *
 *  • Collapsed: avatar + sender + date + 1-line snippet, click to expand.
 *  • Expanded: From/To/Cc + body iframe + attachments, with an optional
 *    "•••" toggle that hides any trailing quoted-reply block.
 *  • Draft: an unsent reply. It reads as a draft and CANNOT expand.
 *
 * The draft mode exists because a thread's members come from every folder the
 * account has — the same query that pulls your Sent replies pulls the reply
 * you started and never sent, and it used to render identically to the
 * others. A message you never sent, printed in a thread beside messages you
 * did, is not a cosmetic problem: it is a false record of what the other
 * person has been told.
 *
 * Expanding is deliberately not offered. A read-only body is the one view of
 * a draft that is never useful — the thing you want with an unfinished reply
 * is to finish it — and it is also what made it look sent. Clicking hands off
 * to `openDraftForEdit`, which is idempotent under repeated clicks (#55).
 *
 * The toggle is rendered as a React button OUTSIDE the iframe so the
 * iframe sandbox can stay no-script (matches the security spec). Clicking
 * it changes the composed HTML and the iframe's `useMemo` rebuilds srcDoc.
 */
export default function ThreadMessageCard({
  member,
  isDraft = false,
  isExpanded,
  detail,
  isLoading,
  error,
  onToggle,
  onRetry,
  fallbackAccountId,
  fallbackFolder,
}: ThreadMessageCardProps) {
  const fromDisplay = member.from_name || member.from_email || "Unknown";
  const initials = getInitials(member.from_name, member.from_email);
  const resolvedAccountId = member.account_id ?? fallbackAccountId ?? null;
  const resolvedFolder = member.folder_name ?? fallbackFolder ?? null;

  if (isDraft) {
    return (
      <button
        type="button"
        onClick={onToggle}
        aria-label={`Continue editing draft${member.subject ? `: ${member.subject}` : ""}`}
        title="Continue editing this draft"
        className="flex w-full items-center gap-3 rounded-md border border-dashed border-warning/50 bg-surface px-3 py-2 text-left transition-colors hover:bg-elevated"
      >
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-elevated text-content-muted">
          <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <span className="flex min-w-0 items-center gap-2">
              <span className="shrink-0 rounded-full bg-warning/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase leading-none tracking-wide text-warning">
                Draft
              </span>
              <span className="truncate text-sm text-content-muted">Not sent</span>
            </span>
            <span className="shrink-0 text-xs text-content-muted">
              {formatRelativeDate(member.date)}
            </span>
          </div>
          {member.snippet && (
            <p className="truncate text-xs text-content-muted">{member.snippet}</p>
          )}
        </div>
      </button>
    );
  }

  if (!isExpanded) {
    return (
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-3 rounded-md border border-border-subtle bg-surface px-3 py-2 text-left transition-colors hover:bg-elevated"
      >
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-elevated text-xs font-semibold text-content-secondary">
          {initials}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <span className="truncate text-sm font-medium text-content">
              {fromDisplay}
            </span>
            <span className="shrink-0 text-xs text-content-muted">
              {formatRelativeDate(member.date)}
            </span>
          </div>
          {member.snippet && (
            <p className="truncate text-xs text-content-muted">
              {member.snippet}
            </p>
          )}
        </div>
      </button>
    );
  }

  // Expanded: header + body
  const headerBlock = (
    <div className="flex items-start gap-3">
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-elevated text-xs font-semibold text-content-secondary">
        {initials}
      </div>
      <div className="min-w-0 flex-1 text-sm">
        <p className="text-content">
          <span className="font-medium">{fromDisplay}</span>
          {member.from_name && member.from_email && (
            <span className="ml-1 text-content-muted">
              &lt;{member.from_email}&gt;
            </span>
          )}
        </p>
        <p className="text-xs text-content-muted">
          {new Date(member.date).toLocaleString()}
        </p>
      </div>
    </div>
  );

  const bodyBlock = (() => {
    if (isLoading) {
      return (
        <div className="flex items-center justify-center py-6">
          <LoadingSpinner />
        </div>
      );
    }
    if (error) {
      return (
        <div className="px-3 py-4 text-center">
          <p className="text-sm text-red-400">Failed to load message</p>
          <p className="mt-1 text-xs text-content-muted">{error}</p>
          {onRetry && (
            <button
              onClick={onRetry}
              className="mt-2 rounded-md bg-elevated px-3 py-1 text-xs text-content-secondary hover:bg-surface hover:text-content"
            >
              Retry
            </button>
          )}
        </div>
      );
    }
    if (!detail) return null;
    return (
      <ExpandedBody
        detail={detail}
        accountId={resolvedAccountId}
        folder={resolvedFolder}
      />
    );
  })();

  return (
    <div className="rounded-md border border-border-subtle bg-surface">
      <div
        className="cursor-pointer px-3 py-2"
        onClick={onToggle}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
      >
        {headerBlock}
      </div>
      <div className="border-t border-border-subtle">{bodyBlock}</div>
    </div>
  );
}

interface ExpandedBodyProps {
  detail: MessageDetail;
  accountId: string | null;
  folder: string | null;
}

function ExpandedBody({ detail, accountId, folder }: ExpandedBodyProps) {
  const split = useMemo(() => {
    const html = detail.sanitized_html ?? "";
    return splitTrailingQuote(html);
  }, [detail.sanitized_html]);

  const [showQuote, setShowQuote] = useState(false);

  const composedHtml = split.quotedHtml && !showQuote
    ? split.mainHtml
    : split.mainHtml + (split.quotedHtml ?? "");

  const hasHtml = !!detail.sanitized_html;
  const plainText = detail.plain_text;

  return (
    // The thread's copy of the email dial's veil: surface-coloured, because it
    // lies on this card rather than on the pane (emailVeilAlphas). The card does
    // not clip, so the veil rounds its own bottom corners to stay inside it.
    <div className={cn("rounded-b-md bg-email-veil-card px-1 py-2")}>
      {hasHtml ? (
        <EmailFrame html={composedHtml} senderEmail={detail.from_email} />
      ) : plainText ? (
        <pre className="whitespace-pre-wrap p-4 text-sm text-content-secondary">
          {plainText}
        </pre>
      ) : (
        <p className="px-4 py-3 text-sm text-content-muted">No content.</p>
      )}
      {split.quotedHtml && (
        <div className="px-3 pb-2">
          <button
            type="button"
            onClick={() => setShowQuote((v) => !v)}
            className="rounded bg-elevated px-2 py-0.5 text-xs text-content-muted hover:bg-surface hover:text-content"
            aria-label={showQuote ? "Hide trimmed content" : "Show trimmed content"}
          >
            •••
          </button>
        </div>
      )}
      {detail.attachments.length > 0 && accountId && folder && (
        <AttachmentList
          accountId={accountId}
          folder={folder}
          uid={detail.uid}
          attachments={detail.attachments}
        />
      )}
    </div>
  );
}
