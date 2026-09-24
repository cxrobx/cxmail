import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import EmailFrame from "./EmailFrame";
import AttachmentList from "./AttachmentList";
import CalendarEventCard from "./CalendarEventCard";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { MessageDetail } from "@/types/email";
import { Reply, Forward, Mail } from "lucide-react";
import { useWindowStore } from "@/stores/windowStore";
import { buildQuotedEmailHtml } from "@/lib/utils";

interface FloatingEmailViewProps {
  accountId: string;
  folder: string;
  uid: number;
}

export default function FloatingEmailView({ accountId, folder, uid }: FloatingEmailViewProps) {
  const [detail, setDetail] = useState<MessageDetail | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const openWindow = useWindowStore((s) => s.openWindow);

  useEffect(() => {
    const load = async () => {
      setIsLoading(true);
      setError(null);
      try {
        const result = await api.messages.fetchBody(accountId, folder, uid);
        setDetail(result);
      } catch (e) {
        setError(String(e));
      } finally {
        setIsLoading(false);
      }
    };
    load();
  }, [accountId, folder, uid]);

  if (isLoading) return <LoadingSpinner className="h-full" />;

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-red-400">Failed to load message</p>
      </div>
    );
  }

  if (!detail) return null;

  const toStr = detail.to_list.map((a) => a.name || a.email).join(", ");

  const handleReply = () => {
    const quotedBody = buildQuotedEmailHtml(detail);

    openWindow({
      type: "compose",
      title: `Re: ${detail.subject || "(no subject)"}`,
      props: {
        mode: "reply",
        defaultTo: detail.from_email,
        defaultSubject: detail.subject?.startsWith("Re:") ? detail.subject : `Re: ${detail.subject || ""}`,
        quotedHtml: quotedBody,
        inReplyTo: detail.message_id || undefined,
        referencesHeader: detail.message_id
          ? [detail.references, detail.message_id].filter(Boolean).join(" ")
          : undefined,
        accountId,
        // Lets the composer default From to the alias this message was sent to.
        replyContext: { accountId, folder, uid },
      },
    });
  };

  const handleForward = () => {
    const quotedBody = buildQuotedEmailHtml(detail);

    openWindow({
      type: "compose",
      title: `Fwd: ${detail.subject || "(no subject)"}`,
      props: {
        mode: "forward",
        defaultSubject: detail.subject?.startsWith("Fwd:") ? detail.subject : `Fwd: ${detail.subject || ""}`,
        quotedHtml: quotedBody,
        accountId,
      },
    });
  };

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="shrink-0 border-b border-border-subtle px-4 py-3">
        <h3 className="text-sm font-semibold text-content">{detail.subject || "(no subject)"}</h3>
        <div className="mt-1 space-y-0.5 text-xs text-content-secondary">
          <p>
            <span className="text-content-muted">From:</span>{" "}
            {detail.from_name || detail.from_email}
            {detail.from_name && <span className="text-content-muted"> &lt;{detail.from_email}&gt;</span>}
          </p>
          {toStr && <p><span className="text-content-muted">To:</span> {toStr}</p>}
          {detail.date && (
            <p className="text-content-muted">{new Date(detail.date).toLocaleString()}</p>
          )}
        </div>
        <div className="mt-2 flex items-center gap-1">
          <button onClick={handleReply} className="flex items-center gap-1 rounded px-2 py-1 text-xs text-content-secondary hover:bg-surface hover:text-content">
            <Reply className="h-3.5 w-3.5" /> Reply
          </button>
          <button onClick={handleForward} className="flex items-center gap-1 rounded px-2 py-1 text-xs text-content-secondary hover:bg-surface hover:text-content">
            <Forward className="h-3.5 w-3.5" /> Forward
          </button>
        </div>
      </div>

      {/* Event details (Join + RSVP) — renders nothing for non-invite mail */}
      <CalendarEventCard accountId={accountId} folder={folder} uid={uid} />

      {/* Body */}
      <div className="flex-1 overflow-auto">
        {detail.sanitized_html ? (
          <EmailFrame html={detail.sanitized_html} senderEmail={detail.from_email} />
        ) : detail.plain_text ? (
          <pre className="whitespace-pre-wrap p-4 text-sm text-content-secondary">{detail.plain_text}</pre>
        ) : (
          <div className="flex h-full items-center justify-center">
            <Mail className="h-8 w-8 text-content-muted" />
          </div>
        )}
      </div>

      {detail.attachments.length > 0 && (
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
