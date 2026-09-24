import { useEffect, useState, useRef, useCallback, useMemo } from "react";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { useWindowStore } from "@/stores/windowStore";
import { useAccountStore } from "@/stores/accountStore";
import { api } from "@/lib/tauri";
import EmailFrame from "./EmailFrame";
import AttachmentList from "./AttachmentList";
import ComposeModal from "./ComposeModal";
import type { ComposeMode } from "./ComposeModal";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import ThreadMessageCard from "./ThreadMessageCard";
import type { MessageDetail, MessageSummary, OutgoingEmail, SavedDraftRef } from "@/types/email";
import { Mail, Reply, ReplyAll, Forward, Clock, MailMinus, MailX, CheckSquare, Trash2 } from "lucide-react";
import SnoozePopover from "./SnoozePopover";
import AISummary from "./AISummary";
import SmartReplies from "./SmartReplies";
import CalendarEventCard from "./CalendarEventCard";
import TrackingBadge from "./TrackingBadge";
import { useUnsubscribe } from "@/hooks/useUnsubscribe";
import { buildQuotedEmailHtml } from "@/lib/utils";
import { uidsForThreadAction } from "@/lib/threadActions";
import { isDraftFolder } from "@/lib/draftFolder";
import { openDraftForEdit } from "@/lib/draftCompose";

type BodyKey = `${string}:${string}:${number}`;
const bodyKey = (accountId: string, folder: string, uid: number): BodyKey =>
  `${accountId}:${folder}:${uid}`;
const BODY_CACHE_LIMIT = 30;

export default function ReadingPane() {
  const {
    selectedAccountId,
    selectedFolder,
    selectedMessageUid,
    selectedMessageAccountId,
    selectedMessageFolder,
    selectedMessageUids,
    isUnifiedInbox,
    selectedGroupId,
    messages,
    markMessageRead,
    setSelectedMessage,
    unsubscribedSenders,
    removeMessages,
    inlineComposeTarget,
    inlineComposeProps,
    setInlineCompose,
    clearInlineCompose,
    foldersByAccount,
  } = useMailStore();
  const openWindow = useWindowStore((s) => s.openWindow);
  const accounts = useAccountStore((s) => s.accounts);
  const { handleUnsubscribeResult, addToast } = useUnsubscribe();
  const [detail, setDetail] = useState<MessageDetail | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Send-as alias addresses for the receiving account, used (alongside the
  // account's own address) to keep the user out of a reply-all Cc.
  const [aliasEmails, setAliasEmails] = useState<string[]>([]);
  const [composeMode, setComposeMode] = useState<ComposeMode | null>(null);
  const [unsubscribing, setUnsubscribing] = useState(false);
  const markReadTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Thread stack state. `threadMembers` is null until getThread resolves;
  // single-message threads (thread_count === 0 OR getThread returned ≤ 1)
  // collapse back to today's single-message view via `effectiveMembers`
  // below. `bodyCache` is FIFO-evicted so a long session doesn't grow
  // unbounded.
  const [threadMembers, setThreadMembers] = useState<MessageSummary[] | null>(null);
  const [bodyCache, setBodyCache] = useState<Map<BodyKey, MessageDetail>>(new Map());
  const [expandedKeys, setExpandedKeys] = useState<Set<BodyKey>>(new Set());
  const [loadingKeys, setLoadingKeys] = useState<Set<BodyKey>>(new Set());
  const [errorKeys, setErrorKeys] = useState<Map<BodyKey, string>>(new Map());
  const getThreadDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  // A selection that carries its own account_id wins, always. Every caller of
  // setSelectedMessage passes the account_id off the message row, and views
  // that are neither the unified inbox nor a group — Needs You, Snoozed — left
  // selectedAccountId pointing at whatever account was picked last, so the
  // reader fetched the right UID from the WRONG mailbox and reported
  // "Message UID N not found" on a message it had cached all along.
  const needsMessageLookup = isUnifiedInbox || selectedGroupId !== null;
  const resolvedAccountId =
    selectedMessageAccountId ?? (needsMessageLookup ? null : selectedAccountId);
  // A selection that carries its own folder (e.g. a search result, which can
  // live in Sent/Drafts/etc.) wins over the view-derived fallback. Without this
  // a non-INBOX search hit gets fetched from INBOX → "Message UID N not found".
  const resolvedFolder = selectedMessageFolder ?? (needsMessageLookup ? "INBOX" : selectedFolder);

  // The receiving account's own address — the primary self-filter for reply-all
  // Cc. May be undefined briefly before accounts load; guarded at use.
  const accountEmail = accounts.find((a) => a.id === resolvedAccountId)?.email;

  // Load the account's send-as aliases once per account (cheap local DB read).
  // These extend the self-filter so an alias the user sends-as also can't land
  // in their own reply-all Cc. On error/null → [] (account address still filters).
  useEffect(() => {
    if (!resolvedAccountId) {
      setAliasEmails([]);
      return;
    }
    let cancelled = false;
    api.identities
      .list(resolvedAccountId)
      .then((list) => {
        if (!cancelled) setAliasEmails((list ?? []).map((i) => i.email));
      })
      .catch(() => {
        if (!cancelled) setAliasEmails([]);
      });
    return () => {
      cancelled = true;
    };
  }, [resolvedAccountId]);

  // Reset thread-stack state on selection change so old members never
  // bleed into the next read.
  useEffect(() => {
    setThreadMembers(null);
    setExpandedKeys(new Set());
    setLoadingKeys(new Set());
    setErrorKeys(new Map());
  }, [resolvedAccountId, resolvedFolder, selectedMessageUid]);

  useEffect(() => {
    if (!resolvedAccountId || !resolvedFolder || !selectedMessageUid) {
      setDetail(null);
      return;
    }

    let cancelled = false;
    const accountId = resolvedAccountId;
    const folder = resolvedFolder;
    const uid = selectedMessageUid;
    const key = bodyKey(accountId, folder, uid);

    const loadBody = async () => {
      // Cache hit: skip the network round-trip. Clear any global loading/
      // error left over from a prior selection so the cached card renders
      // immediately instead of inheriting a spinner or error screen.
      const cached = bodyCache.get(key);
      if (cached) {
        setDetail(cached);
        setIsLoading(false);
        setError(null);
        setExpandedKeys((prev) => {
          if (prev.has(key)) return prev;
          const next = new Set(prev);
          next.add(key);
          return next;
        });
        return;
      }
      setIsLoading(true);
      setError(null);
      try {
        const result = await api.messages.fetchBody(accountId, folder, uid);
        if (cancelled) return;
        setDetail(result);
        cacheBody(key, result);
        setExpandedKeys((prev) => {
          const next = new Set(prev);
          next.add(key);
          return next;
        });

        // Update UI immediately so the message appears read
        const msg = messages.find(
          (m) => m.uid === uid && (!accountId || m.account_id === accountId),
        );
        if (msg && !msg.is_read) {
          markMessageRead(uid, accountId);

          // Sync to backend/IMAP after a short debounce. Thread fan-out is
          // handled by a separate effect once `threadMembers` resolves, so
          // this call only needs to mark the selected representative.
          if (markReadTimer.current) clearTimeout(markReadTimer.current);
          markReadTimer.current = setTimeout(async () => {
            try {
              await api.messages.markRead(accountId, folder, [uid]);
            } catch (e) {
              console.error("Failed to mark as read:", e);
            }
          }, 500);
        }
      } catch (e) {
        if (cancelled) return;
        console.error("Failed to fetch message body:", e);
        setError(String(e));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };

    loadBody();

    return () => {
      cancelled = true;
      if (markReadTimer.current) clearTimeout(markReadTimer.current);
    };
  // bodyCache intentionally omitted: we don't want a cache mutation to
  // re-fire fetchBody. cacheBody is stable via useCallback below.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedAccountId, resolvedFolder, selectedMessageUid]);

  // FIFO-evict the body cache to keep memory bounded across long sessions.
  const cacheBody = useCallback((key: BodyKey, value: MessageDetail) => {
    setBodyCache((prev) => {
      if (prev.has(key)) {
        const next = new Map(prev);
        next.set(key, value);
        return next;
      }
      const next = new Map(prev);
      next.set(key, value);
      while (next.size > BODY_CACHE_LIMIT) {
        const firstKey = next.keys().next().value as BodyKey | undefined;
        if (firstKey === undefined) break;
        next.delete(firstKey);
      }
      return next;
    });
  }, []);

  // Selected row metadata — provides thread_root_id so we can fire
  // getThread in PARALLEL with fetchBody (no chained dependency on
  // detail.message_id arriving first).
  const selectedRow = useMemo(() => {
    if (!resolvedAccountId || !selectedMessageUid) return null;
    return (
      messages.find(
        (m) =>
          m.uid === selectedMessageUid && m.account_id === resolvedAccountId,
      ) ?? null
    );
  }, [messages, resolvedAccountId, selectedMessageUid]);

  // Load thread members when the selected row has siblings. 120 ms debounce
  // so rapid j/k navigation doesn't fire a getThread per keypress.
  useEffect(() => {
    if (!resolvedAccountId || !selectedRow) return;
    // `thread_count` counts messages EXCHANGED; an unsent draft is reported
    // separately so the badge stays honest. Both open the stack — a thread
    // that is one message plus the reply you started is exactly where the
    // draft card needs to appear.
    if (!selectedRow.thread_root_id) return;
    if (selectedRow.thread_count <= 0 && selectedRow.thread_draft_count <= 0) return;

    if (getThreadDebounce.current) clearTimeout(getThreadDebounce.current);
    let cancelled = false;
    const accountId = resolvedAccountId;
    const rootId = selectedRow.thread_root_id;

    getThreadDebounce.current = setTimeout(async () => {
      try {
        const rows = await api.messages.getThread(accountId, rootId);
        if (cancelled) return;
        // ≤ 1 row: stale thread_count or single-message conversation.
        if (rows.length <= 1) {
          setThreadMembers(null);
          return;
        }
        setThreadMembers(rows);
      } catch (e) {
        if (cancelled) return;
        console.warn("getThread failed; falling back to single-message view:", e);
        setThreadMembers(null);
      }
    }, 120);

    return () => {
      cancelled = true;
      if (getThreadDebounce.current) clearTimeout(getThreadDebounce.current);
    };
  }, [resolvedAccountId, selectedRow]);

  // Mark-read fan-out: when a multi-message thread loads, mark every
  // sibling in the displayed folder as read on the server, and clear
  // the representative row's `thread_has_unread` locally.
  //
  // The local update is unconditional (not gated on `!is_read` like
  // loadBody) because the bug we're fixing is "representative is already
  // read but an older sibling is unread → the row stays bolded until the
  // next sync." Calling markMessageRead for an already-read row is a
  // harmless no-op for `is_read` and clears the stale thread_has_unread.
  useEffect(() => {
    if (!threadMembers || !resolvedAccountId || !resolvedFolder) return;
    const folder = resolvedFolder;
    const accountId = resolvedAccountId;
    const hasUnreadInFolder = threadMembers.some(
      (m) =>
        m.account_id === accountId
        && (m.folder_name ?? folder) === folder
        && !m.is_read,
    );
    if (hasUnreadInFolder && selectedMessageUid) {
      markMessageRead(selectedMessageUid, accountId);
    }

    const siblingUids = threadMembers
      .filter(
        (m) =>
          m.account_id === accountId
          && (m.folder_name ?? folder) === folder
          && m.uid !== selectedMessageUid
          && !m.is_read,
      )
      .map((m) => m.uid);
    if (siblingUids.length === 0) return;

    api.messages
      .markRead(accountId, folder, siblingUids)
      .catch((e) => console.error("Failed to mark thread siblings read:", e));
  }, [threadMembers, resolvedAccountId, resolvedFolder, selectedMessageUid, markMessageRead]);

  // Is this member an unsent draft? Threads span folders, so a member can be
  // one even when the folder being browsed is not the drafts folder.
  const memberIsDraft = useCallback(
    (member: MessageSummary) =>
      isDraftFolder(
        foldersByAccount,
        member.account_id ?? resolvedAccountId,
        member.folder_name ?? resolvedFolder,
      ),
    [foldersByAccount, resolvedAccountId, resolvedFolder],
  );

  // Click handler: open a draft in the composer, or lazy-load a collapsed
  // card's body / collapse it.
  const handleToggleMember = useCallback(
    async (member: MessageSummary) => {
      const accountId = member.account_id ?? resolvedAccountId;
      const folder = member.folder_name ?? resolvedFolder;
      if (!accountId || !folder) return;

      // A draft is finished, not read. `openDraftForEdit` focuses an already
      // open window and joins an open fetch rather than stacking a second
      // composer (gotcha #55), so repeated clicks during the IMAP round trip
      // are safe.
      if (memberIsDraft(member)) {
        void openDraftForEdit(accountId, folder, member.uid);
        return;
      }

      const key = bodyKey(accountId, folder, member.uid);

      // Toggle off if already expanded.
      if (expandedKeys.has(key)) {
        setExpandedKeys((prev) => {
          const next = new Set(prev);
          next.delete(key);
          return next;
        });
        return;
      }

      // Cache hit → expand instantly.
      if (bodyCache.has(key)) {
        setExpandedKeys((prev) => {
          const next = new Set(prev);
          next.add(key);
          return next;
        });
        return;
      }

      setLoadingKeys((prev) => new Set(prev).add(key));
      setErrorKeys((prev) => {
        const next = new Map(prev);
        next.delete(key);
        return next;
      });
      try {
        const result = await api.messages.fetchBody(accountId, folder, member.uid);
        cacheBody(key, result);
        setExpandedKeys((prev) => {
          const next = new Set(prev);
          next.add(key);
          return next;
        });
      } catch (e) {
        console.error("Failed to load thread member body:", e);
        setErrorKeys((prev) => {
          const next = new Map(prev);
          next.set(key, String(e));
          return next;
        });
      } finally {
        setLoadingKeys((prev) => {
          const next = new Set(prev);
          next.delete(key);
          return next;
        });
      }
    },
    [bodyCache, cacheBody, expandedKeys, memberIsDraft, resolvedAccountId, resolvedFolder],
  );

  // Listen for keyboard shortcut reply event
  useEffect(() => {
    const handleReply = () => setComposeMode("reply");
    window.addEventListener("cxmail:compose-reply", handleReply);
    return () => window.removeEventListener("cxmail:compose-reply", handleReply);
  }, []);

  // Delete the currently-displayed thread. Fans out to every member in the
  // displayed folder (Gmail behavior). The list update + selection move
  // happen synchronously for a snappy UI; the delete IPC runs after the
  // thread-uid lookup resolves.
  const deleteSelectedThread = useCallback(async () => {
    if (!resolvedAccountId || !resolvedFolder || !selectedMessageUid) return;
    const acct = resolvedAccountId;
    const fld = resolvedFolder;
    const uid = selectedMessageUid;

    const idx = messages.findIndex(
      (m) =>
        m.uid === uid
        && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId)
        && (m.folder_name ?? fld) === fld,
    );
    const target = messages[idx];
    const next = messages[idx + 1] || messages[idx - 1];

    // Snap the list/selection forward immediately.
    if (target) {
      removeMessages(
        new Set([
          messageSelectionKey(target.account_id ?? acct, target.uid, target.folder_name ?? fld),
        ]),
      );
    }
    setSelectedMessage(next?.uid ?? null, next?.account_id);

    try {
      const uids = target
        ? await uidsForThreadAction(target, acct, fld)
        : [uid];
      await api.messages.delete(acct, fld, uids);
    } catch (e) {
      console.error("Failed to delete thread:", e);
    }
  }, [
    resolvedAccountId,
    resolvedFolder,
    selectedMessageUid,
    selectedMessageAccountId,
    messages,
    removeMessages,
    setSelectedMessage,
  ]);

  // Build the props payload for a new compose targeting the current thread.
  // Used for both inline reply and pop-out floating handoff.
  const buildComposeProps = useCallback(
    (mode: ComposeMode, body?: string | null): Record<string, unknown> | null => {
      if (!detail) return null;
      const quotedBody = buildQuotedEmailHtml(detail);
      const subjectPrefix = mode === "forward" ? "Fwd:" : "Re:";
      const defaultSubject = detail.subject?.startsWith(subjectPrefix)
        ? detail.subject
        : `${subjectPrefix} ${detail.subject || ""}`;

      const props: Record<string, unknown> = {
        mode,
        accountId: resolvedAccountId || undefined,
        defaultSubject,
        quotedHtml: quotedBody,
      };

      if (mode === "reply" || mode === "reply-all") {
        props.defaultTo = detail.from_email;
        if (body) props.defaultBody = `<p>${body}</p>`;
        props.inReplyTo = detail.message_id || undefined;
        props.referencesHeader = detail.message_id
          ? [detail.references, detail.message_id].filter(Boolean).join(" ")
          : undefined;
        props.replyContext = resolvedAccountId && resolvedFolder && selectedMessageUid
          ? { accountId: resolvedAccountId, folder: resolvedFolder, uid: selectedMessageUid }
          : undefined;
        if (mode === "reply-all") {
          // Self-set = the receiving account's address + its send-as aliases.
          // Normalize with the codebase's lowercase+trim convention (no shared
          // helper exists). Reply-all should never Cc the user, and should dedupe
          // the sender (already To) out of Cc.
          const norm = (e: string) => e.toLowerCase().trim();
          const self = new Set(
            [accountEmail, ...aliasEmails].filter(Boolean).map((e) => norm(e!)),
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
    },
    [detail, resolvedAccountId, resolvedFolder, selectedMessageUid, accountEmail, aliasEmails],
  );

  const openInlineCompose = useCallback(
    (mode: "reply" | "reply-all" | "forward", body?: string | null) => {
      if (!resolvedAccountId || !resolvedFolder || !selectedMessageUid) return;
      const props = buildComposeProps(mode, body);
      if (!props) return;
      setInlineCompose(
        { accountId: resolvedAccountId, folder: resolvedFolder, uid: selectedMessageUid },
        {
          mode,
          // defaultTo may be a single address (common reply) or an array
          // (reply-all to your own sent message → the original recipients).
          defaultTo: Array.isArray(props.defaultTo)
            ? (props.defaultTo as string[]).map((e) => ({ name: null, email: e }))
            : typeof props.defaultTo === "string"
              ? [{ name: null, email: props.defaultTo as string }]
              : undefined,
          defaultCc: Array.isArray(props.defaultCc)
            ? (props.defaultCc as string[]).map((e) => ({ name: null, email: e }))
            : undefined,
          defaultSubject: props.defaultSubject as string | undefined,
          defaultBody: props.defaultBody as string | undefined,
          quotedHtml: props.quotedHtml as string | undefined,
          inReplyTo: props.inReplyTo as string | undefined,
          referencesHeader: props.referencesHeader as string | undefined,
          replyContext: props.replyContext as { accountId: string; folder: string; uid: number } | undefined,
        },
      );
    },
    [buildComposeProps, resolvedAccountId, resolvedFolder, selectedMessageUid, setInlineCompose],
  );

  // Handle keyboard shortcut compose mode trigger (always "reply" — see listener above).
  useEffect(() => {
    if (composeMode && composeMode !== "new" && detail) {
      openInlineCompose(composeMode);
      setComposeMode(null);
    }
  }, [composeMode, detail, openInlineCompose]);

  const handleInlinePopOut = useCallback(
    (snapshot: OutgoingEmail, draftRef: SavedDraftRef | null, composeAccountId?: string) => {
      if (!resolvedAccountId || !inlineComposeProps) return;
      // The modal's currently selected From account — may differ from the
      // receiving account when the user changed the From dropdown.
      const popAccountId = composeAccountId ?? resolvedAccountId;
      const subjectFallback = snapshot.subject || inlineComposeProps.defaultSubject || "";
      // Convert recipient lists to the string-form props ComposeModal accepts.
      const popProps: Record<string, unknown> = {
        mode: inlineComposeProps.mode,
        accountId: popAccountId,
        // Carry the From ADDRESS too, not just its account: an alias the user
        // picked (or that the reply default chose) is otherwise re-derived in
        // the new window, which loses an explicit choice.
        fromAddress: snapshot.from_email || undefined,
        // ALL To recipients, not just the first — taking to[0] silently dropped
        // every other recipient when popping out a multi-recipient inline compose.
        defaultTo: snapshot.to.map((r) => r.email),
        defaultCc: snapshot.cc.map((r) => r.email),
        // Carry Bcc across the inline→floating handoff (was dropped before).
        defaultBcc: snapshot.bcc.map((r) => r.email),
        defaultSubject: subjectFallback,
        defaultBody: snapshot.html_body,
        defaultAttachments: snapshot.attachments,
        inReplyTo: snapshot.in_reply_to ?? inlineComposeProps.inReplyTo,
        referencesHeader: snapshot.references ?? inlineComposeProps.referencesHeader,
        replyContext: inlineComposeProps.replyContext,
        draftContext: draftRef
          ? { accountId: popAccountId, folder: draftRef.folder, uid: draftRef.uid }
          : undefined,
      };
      openWindow({ type: "compose", title: subjectFallback || "Reply", props: popProps });
      clearInlineCompose();
    },
    [resolvedAccountId, inlineComposeProps, openWindow, clearInlineCompose],
  );

  // === All hooks above, early returns below ===

  if (selectedMessageUids.size > 1) {
    return (
      <EmptyState
        icon={CheckSquare}
        title={`${selectedMessageUids.size} messages selected`}
        description="Right-click to apply actions to all selected messages"
      />
    );
  }

  if (!selectedMessageUid) {
    return (
      <EmptyState
        icon={Mail}
        title="Select a message"
        description="Choose a message to read"
      />
    );
  }

  if (isLoading) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3">
        <LoadingSpinner />
        <p className="text-xs text-content-muted">Loading message...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center">
          <p className="text-sm text-red-400">Failed to load message</p>
          <p className="mt-1 text-xs text-content-muted">{error}</p>
          <button
            onClick={() => {
              setError(null);
              setDetail(null);
              // Re-trigger the effect by toggling a dependency
              const uid = selectedMessageUid;
              if (uid !== null) {
                setIsLoading(true);
                api.messages.fetchBody(resolvedAccountId!, resolvedFolder!, uid)
                  .then((result) => { setDetail(result); setIsLoading(false); })
                  .catch((e) => { setError(String(e)); setIsLoading(false); });
              }
            }}
            className="mt-3 rounded-md bg-surface px-3 py-1.5 text-xs text-content-secondary hover:bg-elevated hover:text-content"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!detail) return null;

  const toStr = detail.to_list.map((a) => a.name || a.email).join(", ");
  const ccStr = detail.cc_list.map((a) => a.name || a.email).join(", ");

  return (
    <div className="flex h-full flex-col overflow-auto">
      {/* Header */}
      <div className="shrink-0 border-b border-border-subtle px-4 py-3">
        <div className="flex items-start justify-between">
          <div className="flex-1">
            <h2 className="text-lg font-semibold text-content">
              {detail.subject || "(no subject)"}
            </h2>
            <div className="mt-1 space-y-0.5 text-sm text-content-secondary">
              <p>
                <span className="text-content-muted">From:</span>{" "}
                <span className="text-content-secondary">
                  {detail.from_name || detail.from_email}
                </span>{" "}
                {detail.from_name && (
                  <span className="text-content-muted">&lt;{detail.from_email}&gt;</span>
                )}
                {detail.from_email && unsubscribedSenders.has(detail.from_email.toLowerCase()) && (
                  <span className="ml-2 inline-flex items-center gap-1 rounded-full bg-red-500/15 px-2 py-0.5 text-xs font-medium text-red-400">
                    <MailX className="h-3 w-3" />
                    Unsubscribed
                  </span>
                )}
              </p>
              {toStr && (
                <p>
                  <span className="text-content-muted">To:</span> {toStr}
                </p>
              )}
              {ccStr && (
                <p>
                  <span className="text-content-muted">Cc:</span> {ccStr}
                </p>
              )}
              {detail.date && (
                <p className="flex items-center gap-2 text-xs text-content-muted">
                  {new Date(detail.date).toLocaleString()}
                  <TrackingBadge messageId={detail.message_id} />
                </p>
              )}
            </div>
          </div>
          {/* Action buttons */}
          <div className="flex items-center gap-1">
            <button onClick={() => openInlineCompose("reply")} className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content" title="Reply">
              <Reply className="h-4 w-4" />
            </button>
            <button onClick={() => openInlineCompose("reply-all")} className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content" title="Reply All">
              <ReplyAll className="h-4 w-4" />
            </button>
            <button onClick={() => openInlineCompose("forward")} className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content" title="Forward">
              <Forward className="h-4 w-4" />
            </button>
            {resolvedAccountId && resolvedFolder && selectedMessageUid && (
              <SnoozePopover
                onSnooze={async (wakeAt) => {
                  try {
                    await api.snooze.snooze(resolvedAccountId, resolvedFolder, selectedMessageUid, wakeAt);
                    setSelectedMessage(null);
                  } catch (e) {
                    console.error("Failed to snooze:", e);
                  }
                }}
              >
                <button className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content" title="Snooze">
                  <Clock className="h-4 w-4" />
                </button>
              </SnoozePopover>
            )}
            {resolvedAccountId && resolvedFolder && (
              <button
                onClick={async () => {
                  if (!resolvedAccountId || !resolvedFolder || !selectedMessageUid) return;
                  setUnsubscribing(true);
                  try {
                    const result = await api.messages.unsubscribe(resolvedAccountId, resolvedFolder, selectedMessageUid);
                    await handleUnsubscribeResult(result, {
                      postAction: {
                        label: "Move to Trash",
                        onClick: () => {
                          void deleteSelectedThread();
                        },
                      },
                      accountId: resolvedAccountId,
                      senderEmail: detail.from_email,
                    });
                  } catch {
                    addToast({ message: "Unsubscribe failed", type: "error" });
                  } finally {
                    setUnsubscribing(false);
                  }
                }}
                disabled={unsubscribing}
                className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
                title="Unsubscribe"
              >
                <MailMinus className="h-4 w-4" />
              </button>
            )}
            {resolvedAccountId && resolvedFolder && selectedMessageUid && (
              <button
                onClick={() => void deleteSelectedThread()}
                className="rounded p-1.5 text-content-secondary hover:bg-surface hover:text-content"
                title="Move to Trash"
              >
                <Trash2 className="h-4 w-4" />
              </button>
            )}
          </div>
        </div>
      </div>

      {/* AI Summary */}
      {resolvedAccountId && resolvedFolder && (
        <AISummary
          accountId={resolvedAccountId}
          folder={resolvedFolder}
          uid={selectedMessageUid}
        />
      )}

      {/* Smart Replies */}
      {resolvedAccountId && resolvedFolder && (
        <SmartReplies
          accountId={resolvedAccountId}
          folder={resolvedFolder}
          uid={selectedMessageUid}
          onSelectReply={(text) => {
            openInlineCompose("reply", text);
          }}
        />
      )}

      {/* Calendar Events */}
      {resolvedAccountId && resolvedFolder && (
        <CalendarEventCard
          accountId={resolvedAccountId}
          folder={resolvedFolder}
          uid={selectedMessageUid}
        />
      )}

      {/* Body */}
      {threadMembers && threadMembers.length > 1 ? (
        <div className="flex-1 space-y-2 overflow-auto px-3 py-3">
          {threadMembers.map((m) => {
            const acct = m.account_id ?? resolvedAccountId ?? "";
            const fld = m.folder_name ?? resolvedFolder ?? "";
            const k = bodyKey(acct, fld, m.uid);
            const isSelected =
              m.uid === selectedMessageUid && m.account_id === resolvedAccountId;
            const isExpanded = expandedKeys.has(k);
            // The selected member's body lives in `detail` (loaded by the
            // top-level effect). Other members come from `bodyCache`.
            const memberDetail = isSelected ? detail : (bodyCache.get(k) ?? null);
            return (
              <ThreadMessageCard
                key={k}
                member={m}
                isDraft={memberIsDraft(m)}
                isExpanded={isExpanded}
                detail={memberDetail}
                isLoading={isSelected ? isLoading : loadingKeys.has(k)}
                error={isSelected ? error : (errorKeys.get(k) ?? null)}
                onToggle={() => handleToggleMember(m)}
                fallbackAccountId={resolvedAccountId}
                fallbackFolder={resolvedFolder}
              />
            );
          })}
        </div>
      ) : (
        <>
          {/* The email dial's veil: the body only, never the header rows above
              it — the text here sits right on the pane's glass. */}
          <div className="flex-1 overflow-auto bg-email-veil">
            {detail.sanitized_html ? (
              <EmailFrame html={detail.sanitized_html} senderEmail={detail.from_email} />
            ) : detail.plain_text ? (
              <pre className="whitespace-pre-wrap p-4 text-sm text-content-secondary">
                {detail.plain_text}
              </pre>
            ) : (
              <EmptyState icon={Mail} title="No content" />
            )}
          </div>

          {detail.attachments.length > 0 && resolvedAccountId && resolvedFolder && (
            <AttachmentList
              accountId={resolvedAccountId}
              folder={resolvedFolder}
              uid={detail.uid}
              attachments={detail.attachments}
            />
          )}
        </>
      )}

      {/* Inline composer (Gmail-style reply box) */}
      {inlineComposeTarget &&
        inlineComposeProps &&
        resolvedAccountId &&
        resolvedFolder &&
        selectedMessageUid &&
        inlineComposeTarget.accountId === resolvedAccountId &&
        inlineComposeTarget.folder === resolvedFolder &&
        inlineComposeTarget.uid === selectedMessageUid && (
          <div className="border-t border-border-subtle px-4 py-3">
            <ComposeModal
              key={`inline-${inlineComposeTarget.accountId}-${inlineComposeTarget.uid}-${inlineComposeProps.mode}`}
              composeSurface="inline"
              onClose={clearInlineCompose}
              onPopOut={handleInlinePopOut}
              mode={inlineComposeProps.mode}
              accountId={resolvedAccountId}
              defaultTo={inlineComposeProps.defaultTo?.map((r) => r.email)}
              defaultCc={inlineComposeProps.defaultCc?.map((r) => r.email)}
              defaultSubject={inlineComposeProps.defaultSubject}
              defaultBody={inlineComposeProps.defaultBody}
              quotedHtml={inlineComposeProps.quotedHtml}
              inReplyTo={inlineComposeProps.inReplyTo}
              referencesHeader={inlineComposeProps.referencesHeader}
              replyContext={inlineComposeProps.replyContext}
            />
          </div>
        )}
    </div>
  );
}
