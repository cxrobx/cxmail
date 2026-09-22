import { useState, useEffect, useRef } from "react";
import { cn, formatRelativeDate, getInitials, getGravatarUrl } from "@/lib/utils";
import { getCategoryBadge } from "@/lib/categories";
import { Paperclip, Star, Pin, MailX } from "lucide-react";
import * as HoverCard from "@radix-ui/react-hover-card";
import type { MessageDetail, MessageSummary, Nudge } from "@/types/email";
import NudgeBadge from "./NudgeBadge";
import { emailWindowKey, useWindowStore } from "@/stores/windowStore";
import { useUIStore } from "@/stores/uiStore";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { api } from "@/lib/tauri";
import { openDraftForEdit } from "@/lib/draftCompose";
import { isDraftFolder as isDraftFolderShared } from "@/lib/draftFolder";
import SwipeableRow from "./SwipeableRow";

interface MessageListItemProps {
  message: MessageSummary;
  isSelected: boolean;
  isMultiSelected?: boolean;
  /** Transient flash highlight (e.g. Claude just changed this row via MCP). */
  flash?: boolean;
  onClick: (e: React.MouseEvent) => void;
  accountId?: string | null;
  folder?: string | null;
  onSwipeArchive?: (message: MessageSummary) => void;
  onSwipeToggleRead?: (message: MessageSummary) => void;
  /** Stalled-conversation prompt for this row, if any. */
  nudge?: Nudge;
  onDismissNudge?: (nudge: Nudge) => void;
}

// Module-level coalescing for the hover-tooltip body fetch. The list is
// virtualized, so a single MessageListItem instance can receive many different
// `message` props as the user scrolls. Caching by `bodyKey` keeps repeated
// hovers of the same row off the IPC bus and short-circuits stale-prop renders.
const tooltipInFlight = new Map<string, Promise<MessageDetail | null>>();
const tooltipResults = new Map<string, MessageDetail | null>();
const TOOLTIP_RESULT_CAP = 100;

function rememberTooltipResult(key: string, detail: MessageDetail | null) {
  if (tooltipResults.has(key)) {
    tooltipResults.delete(key);
  }
  tooltipResults.set(key, detail);
  while (tooltipResults.size > TOOLTIP_RESULT_CAP) {
    const oldest = tooltipResults.keys().next().value;
    if (oldest === undefined) break;
    tooltipResults.delete(oldest);
  }
}

function stripTagsToPlainText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  return (doc.body.textContent ?? "").replace(/\s+/g, " ").trim();
}

interface TooltipBody {
  html: string | null;
  text: string;
}

function detailToTooltipBody(detail: MessageDetail | null): TooltipBody {
  if (!detail) return { html: null, text: "" };
  const html = detail.sanitized_html && detail.sanitized_html.trim().length > 0
    ? detail.sanitized_html
    : null;
  let text = "";
  if (detail.plain_text && detail.plain_text.trim().length > 0) {
    text = detail.plain_text;
  } else if (detail.sanitized_html) {
    text = stripTagsToPlainText(detail.sanitized_html);
  }
  return { html, text };
}

// Build a self-contained HTML doc for the tooltip iframe. Mirrors EmailFrame
// styling minus the height-reporting script + image-trust UI we don't need
// for an ephemeral preview. ammonia has already stripped <script>; the iframe
// is sandboxed with no allow flags as defense in depth.
function buildTooltipDoc(html: string, isDark: boolean): string {
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <style>
      html, body {
        margin: 0;
        padding: 0;
        background: transparent;
        font-family: -apple-system, BlinkMacSystemFont, sans-serif;
        font-size: 14px;
        line-height: 1.45;
        color: ${isDark ? "rgb(232, 220, 200)" : "rgb(30, 25, 15)"};
        word-wrap: break-word;
        overflow-x: hidden;
      }
      /* WebKit zoom shrinks the rendered layout — the right tool for
         fixed-width marketing-email tables inside a 296px tooltip. Unlike
         transform: scale, zoom recalculates layout at the smaller size, so
         wrapping, scrollbars, and dimensions all stay correct. */
      body {
        zoom: 0.5;
        padding: 0;
      }
      /* Force every email element to respect the iframe width. Email HTML
         frequently has hard pixel widths (700+ px wrappers, fixed-width
         tables, spacer divs); without these overrides the content overflows
         the right edge and overflow-x hidden simply clips it. */
      body, body * {
        max-width: 100% !important;
        box-sizing: border-box !important;
      }
      table {
        table-layout: fixed !important;
        width: 100% !important;
      }
      td, th { word-break: break-word; overflow-wrap: anywhere; }
      img { height: auto !important; }
      a:not([style*="background"]) { color: rgb(10, 132, 255); }
      a { pointer-events: none; }
      img { max-width: 100%; height: auto; }
      img[data-original-src] { display: none !important; }
      blockquote {
        border-left: 3px solid ${isDark ? "rgb(74, 68, 57)" : "rgb(200, 195, 185)"};
        margin: 6px 0;
        padding-left: 10px;
        color: ${isDark ? "rgb(160, 144, 120)" : "rgb(100, 90, 75)"};
      }
      hr { border-color: ${isDark ? "rgb(74, 68, 57)" : "rgb(220, 215, 205)"}; }
      ${isDark ? `
      div:not([style*="background"]),
      p, span:not([style*="background"]),
      td:not([style*="background"]), th, li,
      h1, h2, h3, h4, h5, h6, font {
        color: rgb(232, 220, 200) !important;
      }
      a:not([style*="background"]) { color: rgb(10, 132, 255) !important; }
      table:not([style*="background"]),
      tr:not([style*="background"]),
      td:not([style*="background"]),
      th:not([style*="background"]),
      div:not([style*="background"]) {
        background-color: transparent !important;
      }
      table, tr, td, th { border-color: rgb(74, 68, 57) !important; }
      ` : ""}
    </style>
  </head>
  <body><div class="email-body">${html}</div></body>
</html>`;
}

const NON_ARCHIVABLE_FOLDER_NAMES = new Set([
  "Archive",
  "[Gmail]/All Mail",
  "Drafts",
  "[Gmail]/Drafts",
]);

export default function MessageListItem({ message, isSelected, isMultiSelected, flash, onClick, accountId, folder, onSwipeArchive, onSwipeToggleRead, nudge, onDismissNudge }: MessageListItemProps) {
  const initials = getInitials(message.from_name, message.from_email);
  const fromDisplay = message.from_name || message.from_email || "Unknown";
  const [avatarUrl, setAvatarUrl] = useState<string | null>(null);
  const [avatarError, setAvatarError] = useState(false);
  const categoryBadge = getCategoryBadge(message.category);
  const openWindow = useWindowStore((s) => s.openWindow);
  const density = useUIStore((s) => s.density);
  const unsubscribedSenders = useMailStore((s) => s.unsubscribedSenders);
  const foldersByAccount = useMailStore((s) => s.foldersByAccount);

  const isDraftFolder = (aid: string | null | undefined, fld: string | null | undefined) =>
    isDraftFolderShared(foldersByAccount, aid, fld);

  const isUltraCompact = density === "ultra-compact";
  const isUnsubscribed = message.from_email
    ? unsubscribedSenders.has(message.from_email.toLowerCase())
    : false;
  // Treat the row as unread if any thread member is unread (Gmail behavior),
  // not just the displayed representative.
  const isUnread = !message.is_read || message.thread_has_unread;

  // bodyKey identifies the (account, folder, uid) that this row currently
  // points at. Because the virtualized list reuses MessageListItem instances,
  // `fullBody` must be scoped to this identity — a stale body from the row's
  // previous occupant would render under the wrong header.
  const aid = accountId ?? message.account_id ?? null;
  const fld = message.folder_name ?? folder ?? "INBOX";
  const bodyKey = aid ? `${aid}|${fld}|${message.uid}` : null;

  const [fullBody, setFullBody] = useState<{ key: string; html: string | null; text: string } | null>(null);
  const [bodyLoading, setBodyLoading] = useState(false);
  const tooltipTokenRef = useRef(0);
  const theme = useResolvedTheme();

  useEffect(() => {
    if (message.from_email) {
      setAvatarError(false);
      getGravatarUrl(message.from_email, 32).then(setAvatarUrl);
    }
  }, [message.from_email]);

  // Stale-key reset: when the virtualized row is reused for a different
  // message, drop any body tied to the previous occupant and invalidate any
  // in-flight fetch so its result is ignored.
  useEffect(() => {
    tooltipTokenRef.current += 1;
    setFullBody(null);
    setBodyLoading(false);
  }, [bodyKey]);

  const handleTooltipOpenChange = (open: boolean) => {
    if (!open) {
      tooltipTokenRef.current += 1;
      setBodyLoading(false);
      return;
    }
    if (!bodyKey) return;

    if (tooltipResults.has(bodyKey)) {
      const cached = tooltipResults.get(bodyKey) ?? null;
      const body = detailToTooltipBody(cached);
      if (body.html || body.text) {
        setFullBody({ key: bodyKey, html: body.html, text: body.text });
      }
      return;
    }

    const token = ++tooltipTokenRef.current;
    setBodyLoading(true);

    let pending = tooltipInFlight.get(bodyKey);
    if (!pending) {
      pending = api.messages
        .fetchCachedBody(aid!, fld, message.uid)
        .then((detail) => {
          // Cache hit with a usable body — done, no network.
          const body = detailToTooltipBody(detail);
          if (body.html || body.text) return detail;
          // Cache miss: fetch the full body from IMAP. This also persists it,
          // so later hovers (and opening the email) are instant. Guarded by the
          // 500ms hover open-delay + single-flight (tooltipInFlight) so passive
          // hovers don't storm the server.
          return api.messages.fetchBody(aid!, fld, message.uid).catch(() => detail);
        })
        .catch((err) => {
          console.warn("tooltip body fetch failed:", err);
          return null;
        })
        .then((detail) => {
          rememberTooltipResult(bodyKey, detail);
          tooltipInFlight.delete(bodyKey);
          return detail;
        });
      tooltipInFlight.set(bodyKey, pending);
    }

    void pending.then((detail) => {
      if (tooltipTokenRef.current !== token) return;
      const body = detailToTooltipBody(detail);
      if (body.html || body.text) {
        setFullBody({ key: bodyKey, html: body.html, text: body.text });
      }
      setBodyLoading(false);
    });
  };

  const handleClick = (e: React.MouseEvent) => {
    const resolvedAccountId = accountId || message.account_id;
    if (e.altKey && resolvedAccountId && folder) {
      e.preventDefault();
      if (isDraftFolder(resolvedAccountId, folder)) {
        void openDraftForEdit(resolvedAccountId, folder, message.uid);
      } else {
        openWindow({
          type: "email",
          title: message.subject || "(no subject)",
          props: {
            accountId: resolvedAccountId,
            folder,
            uid: message.uid,
          },
          key: emailWindowKey(resolvedAccountId, folder, message.uid),
        });
      }
    } else {
      onClick(e);
    }
  };

  const handleDoubleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    const resolvedAccountId = accountId || message.account_id;
    const resolvedFolder = folder || "INBOX";
    if (!resolvedAccountId) return;
    if (isDraftFolder(resolvedAccountId, resolvedFolder)) {
      void openDraftForEdit(resolvedAccountId, resolvedFolder, message.uid);
      return;
    }
    openWindow({
      type: "email",
      title: message.subject || "(no subject)",
      props: { accountId: resolvedAccountId, folder: resolvedFolder, uid: message.uid },
      key: emailWindowKey(resolvedAccountId, resolvedFolder, message.uid),
    });
  };

  const button = isUltraCompact ? (
    <button
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      className={cn(
        "message-list-item flex h-full w-full items-center gap-2 border-b border-border-subtle px-3 text-left transition-colors",
        isSelected ? "bg-accent/10" : isMultiSelected ? "bg-accent/15" : isUnread ? "bg-surface hover:bg-sidebar" : "hover:bg-sidebar",
        message.is_muted && "opacity-50",
        flash && "cx-row-flash"
      )}
    >
      {/* Unread indicator */}
      {isUnread ? (
        <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
      ) : (
        <div className="h-1.5 w-1.5 shrink-0" />
      )}

      {/* Sender */}
      <span
        className={cn(
          "w-[140px] shrink-0 truncate text-xs",
          isUnread ? "font-semibold text-content" : "text-content-secondary"
        )}
      >
        {fromDisplay}
      </span>

      {/* Subject + category */}
      <span
        className={cn(
          "min-w-0 flex-1 truncate text-xs",
          isUnread ? "font-medium text-content" : "text-content-secondary"
        )}
      >
        {message.subject || "(no subject)"}
      </span>
      {categoryBadge && (
        <span
          className="shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-medium leading-none"
          style={{ background: categoryBadge.bgColor, color: categoryBadge.textColor }}
        >
          {categoryBadge.label}
        </span>
      )}

      {/* Indicators */}
      {isUnsubscribed && <MailX className="h-3 w-3 shrink-0 text-red-400/70" aria-label="Unsubscribed" />}
      {message.is_pinned && <Pin className="h-3 w-3 shrink-0 fill-accent text-accent" />}
      {message.is_flagged && <Star className="h-3 w-3 shrink-0 fill-yellow-500 text-yellow-500" />}
      {message.has_attachments && <Paperclip className="h-3 w-3 shrink-0 text-content-muted" />}

      {/* Date — or the nudge, which replaces it rather than joining it (see
          NudgeBadge: this list is virtualized and row height is cached). */}
      {nudge && onDismissNudge ? (
        <NudgeBadge nudge={nudge} onDismiss={onDismissNudge} className="text-[11px]" />
      ) : (
        <span className="shrink-0 text-[11px] text-content-muted">
          {formatRelativeDate(message.date)}
        </span>
      )}
    </button>
  ) : (
    <button
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      className={cn(
        "message-list-item flex h-full w-full gap-3 border-b border-border-subtle px-4 py-3 text-left transition-colors",
        isSelected ? "bg-accent/10" : isMultiSelected ? "bg-accent/15" : isUnread ? "bg-surface hover:bg-sidebar" : "hover:bg-sidebar",
        message.is_muted && "opacity-50",
        flash && "cx-row-flash"
      )}
    >
      {/* Unread indicator */}
      <div className="flex shrink-0 items-start pt-1">
        {isUnread ? (
          <div className="h-2 w-2 rounded-full bg-accent" />
        ) : (
          <div className="h-2 w-2" />
        )}
      </div>

      {/* Avatar */}
      {avatarUrl && !avatarError ? (
        <img
          src={avatarUrl}
          alt=""
          className="msg-avatar h-8 w-8 shrink-0 rounded-full"
          onError={() => setAvatarError(true)}
          loading="lazy"
        />
      ) : (
        <div className="msg-avatar flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-elevated text-xs font-medium text-content-secondary">
          {initials}
        </div>
      )}

      {/* Content */}
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between">
          <span
            className={cn(
              "truncate text-sm",
              isUnread ? "font-semibold text-content" : "text-content-secondary"
            )}
          >
            {fromDisplay}
          </span>
          <div className="ml-2 flex shrink-0 items-center gap-1.5">
            {message.thread_count > 0 && (
              <span className="inline-flex items-center justify-center min-w-[18px] h-[18px] rounded-full bg-elevated px-1 text-[10px] font-semibold text-content-secondary">
                {message.thread_count + 1}
              </span>
            )}
            {nudge && onDismissNudge ? (
              <NudgeBadge nudge={nudge} onDismiss={onDismissNudge} className="text-xs" />
            ) : (
              <span className="text-xs text-content-muted">
                {formatRelativeDate(message.date)}
              </span>
            )}
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          <p
            className={cn(
              "truncate text-sm",
              isUnread ? "font-medium text-content" : "text-content-secondary"
            )}
          >
            {message.subject || "(no subject)"}
          </p>
          {categoryBadge && (
            <span
              className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium leading-none"
              style={{ background: categoryBadge.bgColor, color: categoryBadge.textColor }}
            >
              {categoryBadge.label}
            </span>
          )}
        </div>
        {message.snippet && (
          <p className="msg-snippet truncate text-xs text-content-muted">{message.snippet}</p>
        )}
      </div>

      {/* Indicators */}
      <div className="flex shrink-0 items-start gap-1 pt-1">
        {isUnsubscribed && <MailX className="h-3.5 w-3.5 text-red-400/70" aria-label="Unsubscribed" />}
        {message.is_pinned && <Pin className="h-3.5 w-3.5 fill-accent text-accent" />}
        {message.is_flagged && <Star className="h-3.5 w-3.5 fill-yellow-500 text-yellow-500" />}
        {message.has_attachments && <Paperclip className="h-3.5 w-3.5 text-content-muted" />}
      </div>
    </button>
  );

  const showFullBody = fullBody && fullBody.key === bodyKey && (fullBody.html || fullBody.text);

  const inner = isSelected ? (
    button
  ) : (
    <HoverCard.Root openDelay={500} closeDelay={0} onOpenChange={handleTooltipOpenChange}>
      <HoverCard.Trigger asChild>{button}</HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content
          side="top"
          sideOffset={0}
          align="end"
          className="tooltip-bubble z-[100] w-[320px] rounded-lg border border-border-subtle bg-elevated p-3 shadow-xl animate-in fade-in-0 zoom-in-95"
          avoidCollisions
        >
          <div className="space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-content">{fromDisplay}</span>
              {message.from_email && message.from_name && (
                <span className="text-xs text-content-muted truncate">{message.from_email}</span>
              )}
            </div>
            <p className="text-sm font-medium text-content">{message.subject || "(no subject)"}</p>
            <p className="text-xs text-content-muted">{formatRelativeDate(message.date)}</p>
            {showFullBody ? (
              fullBody!.html ? (
                <iframe
                  title="Email preview"
                  sandbox=""
                  srcDoc={buildTooltipDoc(fullBody!.html, theme === "dark")}
                  className="block w-full border-0"
                  style={{ height: 280, background: "transparent" }}
                />
              ) : (
                <div className="max-h-[280px] overflow-y-auto overscroll-contain whitespace-pre-wrap text-xs text-content-secondary leading-relaxed pr-1">
                  {fullBody!.text}
                </div>
              )
            ) : (
              <>
                {message.snippet && (
                  <p className="max-h-[120px] overflow-y-auto overscroll-contain text-xs text-content-secondary leading-relaxed">
                    {message.snippet}
                  </p>
                )}
                {bodyLoading && (
                  <p className="text-[10px] text-content-muted">Loading preview…</p>
                )}
              </>
            )}
          </div>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
  );

  if (!onSwipeArchive || !onSwipeToggleRead) {
    return inner;
  }

  // canArchive: false in folders where archive is a no-op. Checks the folder's
  // declared type (preferred) and falls back to common provider folder names
  // for Drafts/Archive synonyms.
  const folders = aid ? foldersByAccount[aid] : undefined;
  const folderMeta = folders?.find((f) => f.name === fld);
  const folderType = folderMeta?.folder_type ?? null;
  const canArchive =
    folderType !== "archive" &&
    folderType !== "drafts" &&
    !NON_ARCHIVABLE_FOLDER_NAMES.has(fld);

  const swipeKey = messageSelectionKey(aid, message.uid, fld);

  return (
    <SwipeableRow
      key={swipeKey}
      messageId={swipeKey}
      isRead={!isUnread}
      canArchive={canArchive}
      onArchive={() => onSwipeArchive(message)}
      onToggleRead={() => onSwipeToggleRead(message)}
    >
      {inner}
    </SwipeableRow>
  );
}
