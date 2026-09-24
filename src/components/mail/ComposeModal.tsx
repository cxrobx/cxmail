import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import { useEditor, EditorContent } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Image from "@tiptap/extension-image";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import type { ContactResult, OutgoingEmail, OutgoingAttachment, EmailTemplate, SavedDraftRef, McpActivity, SendAsAddress } from "@/types/email";
import {
  fromOptions,
  primaryOnly,
  resolveFrom,
  sameAddress,
  sendAsLabel,
  shouldShowFromPicker,
} from "@/lib/sendAs";
import {
  X,
  Bold,
  Italic,
  Underline as UnderlineIcon,
  List,
  ListOrdered,
  Minus,
  ChevronUp,
  ChevronDown,
  Loader2,
  Save,
  Clock,
  Sun,
  CalendarDays,
  Eye,
  EyeOff,
  AlertTriangle,
  BellRing,
  Paperclip,
  FileText,
  Sparkles,
  Maximize2,
  Check,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn, detectAttachmentMention, formatBytes, setEditorContent } from "@/lib/utils";
import {
  SignatureBlock,
  QuotedBlock,
  HtmlBlock,
  flushHtmlBlockEdits,
  isEditingHtmlBlock,
} from "@/lib/composeNodes";
import { useUIStore } from "@/stores/uiStore";
import { buildSchedulePresets, type SchedulePreset } from "@/lib/schedulePresets";
import { toLocalDateTimeInput } from "@/lib/dateInput";
import AIWritingMenu from "./AIWritingMenu";
import LinkButton from "./LinkButton";
import { ComposeShortcuts, isLinkShortcut } from "@/lib/composeShortcuts";
import { isEmptyEditorHtml, placeSignature, recoverForeignDraftHtml } from "@/lib/draftRecovery";
import AIAssistPanel, { type AssistLaunch } from "./AIAssistPanel";
import type { DraftSnapshot } from "@/lib/draftValidation";
import FollowupPopover from "./FollowupPopover";
import { useDockDrag } from "@/hooks/useDockDrag";
import { formatPresetDate } from "@/components/shared/DateTimePickerMenu";

export type ComposeMode = "new" | "reply" | "reply-all" | "forward";
export type ComposeSurface = "docked" | "windowed" | "inline";

const SCHEDULE_PRESET_ICONS: Record<SchedulePreset["key"], LucideIcon> = {
  tomorrow: Sun,
  "next-week": CalendarDays,
};

interface ComposeModalProps {
  onClose: () => void;
  mode?: ComposeMode;
  /** A single address (back-compat) or many — mailto deep-links pass an array
   * so every `to` recipient lands in the To field. */
  defaultTo?: string | string[];
  defaultCc?: string[];
  defaultBcc?: string[];
  defaultSubject?: string;
  defaultBody?: string;
  defaultAttachments?: OutgoingAttachment[];
  quotedHtml?: string;
  inReplyTo?: string;
  referencesHeader?: string;
  accountId?: string;
  /** Seeds the From ADDRESS (as opposed to `accountId`, the From account).
   * Set when a compose is continued somewhere else — the inline→floating
   * pop-out — so an alias the user picked survives the handoff. Ignored when
   * the account cannot send as it, which is what makes it safe to pass a
   * snapshot's `from_email` blindly. */
  fromAddress?: string;
  replyContext?: { accountId: string; folder: string; uid: number };
  draftContext?: { accountId: string; folder: string; uid: number };
  /** Visual surface for the modal. "docked" = bottom-right card (default),
   * "windowed" = body chrome only (FloatingWindow wraps it),
   * "inline" = bordered card embedded in another component (e.g., ReadingPane). */
  composeSurface?: ComposeSurface;
  /** Called when the inline composer's pop-out icon is clicked. Receives the
   * current draft snapshot, the latest SavedDraftRef (or null if no save
   * has landed yet), and the modal's currently selected From account id —
   * which may differ from the receiving account when the user changed the
   * From dropdown — so the floating window continues with the same draft
   * AND the same sender. */
  onPopOut?: (snapshot: OutgoingEmail, draftRef: SavedDraftRef | null, accountId?: string) => void;
  /** Called with the draft this modal currently edits (null before the first
   * save lands, and again whenever autosave moves it to a new UID) plus the
   * From account. The floating-window host uses it to keep the window keyed to
   * its draft, so a click on that draft's row focuses this window instead of
   * opening a second one. */
  onDraftRefChange?: (ref: SavedDraftRef | null, accountId: string | undefined) => void;
}

interface Recipient {
  name: string | null;
  email: string;
}

// SignatureBlock and QuotedBlock store their content in an `html` attribute
// rather than as editor children, so editor.getText() walks past them and
// produces a plain-text body that's missing the signature and the quoted
// thread. We compensate by registering text serializers that read the `html`
// attribute and convert it to text via the DOM. Callers should always go
// through this helper instead of editor.getText() for outgoing plain bodies.
function htmlToText(html: string): string {
  if (!html) return "";
  const tmp = document.createElement("div");
  tmp.innerHTML = html;
  return tmp.textContent || "";
}

function getPlainBody(editor: ReturnType<typeof useEditor> | null): string {
  if (!editor) return "";
  return editor.getText({
    blockSeparator: "\n\n",
    textSerializers: {
      signatureBlock: ({ node }) => htmlToText((node.attrs as { html?: string }).html ?? ""),
      quotedBlock: ({ node }) => htmlToText((node.attrs as { html?: string }).html ?? ""),
      htmlBlock: ({ node }) => htmlToText((node.attrs as { html?: string }).html ?? ""),
    },
  });
}

/** Approximate decoded byte length of a base64 string, for displaying attachment sizes. */
function base64ByteLength(b64: string): number {
  if (!b64) return 0;
  const padding = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((b64.length * 3) / 4) - padding);
}

export default function ComposeModal({
  onClose,
  mode = "new",
  defaultTo,
  defaultCc,
  defaultBcc,
  defaultSubject,
  defaultBody,
  defaultAttachments,
  quotedHtml,
  inReplyTo,
  referencesHeader,
  accountId: propAccountId,
  fromAddress: propFromAddress,
  replyContext,
  draftContext,
  composeSurface = "docked",
  onPopOut,
  onDraftRefChange,
}: ComposeModalProps) {
  const isFloating = composeSurface === "windowed";
  const isInline = composeSurface === "inline";
  const { accounts } = useAccountStore();
  const { selectedAccountId } = useMailStore();
  const { undoSendDelaySeconds, setActiveSend, lastFromAccountId } = useUIStore();
  // Default From: explicit prop (replies/drafts) > sidebar-selected account >
  // last-used From on a fresh compose (unified inbox has no account context) >
  // first account. lastFromAccountId is validated against the live account
  // list so a deleted account can't be resolved.
  const defaultAccountId =
    propAccountId ||
    selectedAccountId ||
    (accounts.some((a) => a.id === lastFromAccountId) ? lastFromAccountId! : undefined) ||
    accounts[0]?.id;
  const [fromOverrideId, setFromOverrideId] = useState<string | null>(null);
  const resolvedAccountId = fromOverrideId ?? defaultAccountId;
  const fromAccount = accounts.find((a) => a.id === resolvedAccountId);

  // ── Send-as: the From picker selects an ADDRESS, not an account ─────────
  // An account can own aliases, and a reply to mail that arrived at one should
  // go out from that alias. `sendAsByAccount` is filled per account from
  // `list_send_as` (primary first, aliases after); an account still loading
  // contributes its own address, so the picker never briefly loses a row.
  const [sendAsByAccount, setSendAsByAccount] = useState<Record<string, SendAsAddress[]>>({});
  // The address the user picked. Separate from the reply default below so a
  // late-arriving default can never overwrite a choice already made.
  const [fromAddressOverride, setFromAddressOverride] = useState<string | null>(
    propFromAddress ?? null,
  );
  // The address the message being replied to was addressed to, resolved in
  // Rust (`reply_from_for_message`) so compose and the MCP cannot disagree.
  const [replyDefaultFrom, setReplyDefaultFrom] = useState<string | null>(null);
  const fromAddressOptions = useMemo(
    () => fromOptions(accounts, sendAsByAccount),
    [accounts, sendAsByAccount],
  );
  const fromAddress = resolveFrom(
    fromAddressOptions,
    resolvedAccountId,
    fromAddressOverride,
    replyDefaultFrom,
  );

  const [to, setTo] = useState<Recipient[]>(() => {
    if (!defaultTo) return [];
    const list = Array.isArray(defaultTo) ? defaultTo : [defaultTo];
    return list.filter(Boolean).map((email) => ({ name: null, email }));
  });
  const [cc, setCc] = useState<Recipient[]>(
    defaultCc ? defaultCc.map((e) => ({ name: null, email: e })) : []
  );
  const [bcc, setBcc] = useState<Recipient[]>(
    defaultBcc ? defaultBcc.filter(Boolean).map((email) => ({ name: null, email })) : []
  );
  const [subject, setSubject] = useState(defaultSubject || "");
  const [showCcBcc, setShowCcBcc] = useState(
    (defaultCc?.length ?? 0) > 0 || (defaultBcc?.length ?? 0) > 0
  );
  const [isMinimized, setIsMinimized] = useState(false);
  // The docked composer moves by its header. Inline and windowed surfaces
  // already have a place (the reading pane, a FloatingWindow).
  const dockBoxRef = useRef<HTMLDivElement>(null);
  const isDocked = !isFloating && !isInline;
  const dock = useDockDrag(dockBoxRef, isDocked && !isMinimized);
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [showScheduleMenu, setShowScheduleMenu] = useState(false);
  const [showCustomSchedule, setShowCustomSchedule] = useState(false);
  const [customScheduleDate, setCustomScheduleDate] = useState("");
  const [trackOpens, setTrackOpens] = useState(false);
  const [trackingServiceConfigured, setTrackingServiceConfigured] = useState(false);
  // Effective gate for the toggle UI: service configured AND the From account
  // has tracking enabled. Recomputes when the From selector changes mid-compose.
  const trackingConfigured = trackingServiceConfigured && !!fromAccount?.track_opens_enabled;
  const [showAttachmentWarning, setShowAttachmentWarning] = useState(false);
  const [followupRemindAt, setFollowupRemindAt] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<OutgoingAttachment[]>(defaultAttachments ?? []);
  const [draftContextState, setDraftContextState] = useState<SavedDraftRef | null>(
    draftContext ? { folder: draftContext.folder, uid: draftContext.uid } : null,
  );
  // Set when Claude edits this draft while the user has unsaved local changes:
  // holds the pending envelope so the conflict banner can offer Reload / Keep mine.
  const [mcpUpdate, setMcpUpdate] = useState<McpActivity | null>(null);
  // Bumped by applyMcpReload once it has applied a Claude edit. The reseed effect
  // (declared right after the snapshot effect) then rebases lastSavedHashRef onto
  // the settled snapshot so the debounced autosave the reload scheduled skips.
  const [reloadSettleTick, setReloadSettleTick] = useState(0);
  // AI Assist panel. `assistLaunch` is non-null exactly while the panel is
  // open; `assistSelection` freezes the editor selection at launch, because
  // focusing the panel destroys it and Rewrite needs to know what to act on.
  const [assistLaunch, setAssistLaunch] = useState<AssistLaunch | null>(null);
  const [assistSelection, setAssistSelection] = useState<{
    from: number;
    to: number;
    text: string;
  } | null>(null);
  const [reviewCount, setReviewCount] = useState<number | undefined>(undefined);
  const [templates, setTemplates] = useState<EmailTemplate[]>([]);
  const [showTemplateMenu, setShowTemplateMenu] = useState(false);
  const [showSaveTemplate, setShowSaveTemplate] = useState(false);
  const [templateName, setTemplateName] = useState("");
  const [isSuggestingSubject, setIsSuggestingSubject] = useState(false);
  const [signatureHtml, setSignatureHtml] = useState<string | null>(null);
  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [lastSavedAt, setLastSavedAt] = useState<Date | null>(null);
  const [showFromMenu, setShowFromMenu] = useState(false);

  const latestDraftRef = useRef<{ outgoing: OutgoingEmail; editorText: string } | null>(null);
  const initialDraftSnapshotRef = useRef<string | null>(null);
  const finalizedRef = useRef(false);
  // signatureLoaded must be state so the insertion effect re-runs even when no signature exists.
  const [signatureLoaded, setSignatureLoaded] = useState(false);
  const signatureInsertedRef = useRef(false);
  const didInitRef = useRef(false);
  // Captures the original AI-generated reply so we can log (draft, sent) pairs for edit-learning.
  const aiDraftRef = useRef<string | null>(null);
  // Autosave: serial promise queue. saveOrEdit() chains a new closure that uses
  // latestDraftRef at execution time. The queue serializes IMAP operations.
  const saveQueueRef = useRef<Promise<unknown>>(Promise.resolve());
  const lastSavedHashRef = useRef<string | null>(null);
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // True while a conflict banner is shown (Claude edited a draft the user has
  // unsaved changes to). Suppresses autosave so the modal's stale content can't
  // silently overwrite Claude's edit on the new UID before the user decides.
  const mcpReloadPendingRef = useRef(false);
  // Always-current ref so unmount + online handlers can read latest values
  const draftContextStateRef = useRef<SavedDraftRef | null>(draftContextState);
  draftContextStateRef.current = draftContextState;
  const resolvedAccountIdRef = useRef<string | undefined>(resolvedAccountId);
  resolvedAccountIdRef.current = resolvedAccountId;
  // Read by the send-as load effect, which depends on the account id alone:
  // `replyContext` is a fresh object on every parent render, and re-running
  // that effect would re-resolve the reply default over a choice already made.
  const replyContextRef = useRef(replyContext);
  replyContextRef.current = replyContext;
  const fromAddressOverrideRef = useRef<string | null>(fromAddressOverride);
  fromAddressOverrideRef.current = fromAddressOverride;
  // Tell the host which draft this modal edits now. Read through a ref: the
  // window manager passes an inline callback whose identity changes on every
  // window-store render (each drag frame), and this must fire on ref changes
  // only — not re-fire, and re-set the store, on every one of those renders.
  const onDraftRefChangeRef = useRef(onDraftRefChange);
  onDraftRefChangeRef.current = onDraftRefChange;
  useEffect(() => {
    onDraftRefChangeRef.current?.(draftContextState, resolvedAccountId);
  }, [draftContextState, resolvedAccountId]);

  useEffect(() => {
    api.tracking.getConfig().then((config) => {
      setTrackingServiceConfigured(!!config.service_url);
    }).catch((e) => console.error("Failed to load tracking config:", e));
  }, []);

  // Reset the per-message toggle to the From-account's default whenever the
  // account or its tracking flag changes (including mid-compose From switches).
  // The backend gate is authoritative even if a stale trackOpens slips through.
  useEffect(() => {
    setTrackOpens(trackingServiceConfigured && !!fromAccount?.track_opens_enabled);
  }, [resolvedAccountId, trackingServiceConfigured, fromAccount?.track_opens_enabled]);

  // Load the account's send-as addresses, the reply default, and the signature
  // that belongs to the resulting From — in ONE effect, and all of it before
  // `signatureLoaded` flips.
  //
  // The ordering is the point: the signature is inserted exactly once
  // (`signatureInsertedRef`), so an alias's signature that arrives after that
  // insertion never reaches the editor. Resolving the From first is what lets
  // a reply to mail that came in on an alias open already signed as the alias.
  useEffect(() => {
    if (!resolvedAccountId) return;
    const accountId = resolvedAccountId;
    let cancelled = false;
    (async () => {
      try {
        const [sendAs, identities, replyFrom] = await Promise.all([
          Promise.resolve(api.identities.listSendAs(accountId)).catch(() => [] as SendAsAddress[]),
          api.identities.list(accountId).catch(() => []),
          // Only for the account the original actually arrived on: another
          // account's aliases say nothing about who this reply is from.
          replyContextRef.current && replyContextRef.current.accountId === accountId
            ? api.identities
                .replyFrom(accountId, replyContextRef.current.folder, replyContextRef.current.uid)
                .catch(() => null)
            : Promise.resolve(null),
        ]);
        if (cancelled) return;

        const addresses = Array.isArray(sendAs) ? sendAs : [];
        if (addresses.length > 0) {
          setSendAsByAccount((m) => ({ ...m, [accountId]: addresses }));
        }
        // Defensive shape check: an older backend (or a test double) can
        // answer with something that is not an address, and a bad `from_email`
        // is refused at the write boundary rather than silently corrected.
        const replyEmail =
          replyFrom && !Array.isArray(replyFrom) && typeof replyFrom.email === "string"
            ? replyFrom.email
            : null;
        if (replyEmail) setReplyDefaultFrom(replyEmail);

        const account = accounts.find((a) => a.id === accountId);
        const pool = addresses.length > 0 ? addresses : account ? primaryOnly(account) : [];
        const effective = resolveFrom(pool, accountId, fromAddressOverrideRef.current, replyEmail);
        // An alias with no signature of its own inherits the account's rather
        // than sending unsigned — mirrors `signature_for_send_as` in the MCP.
        const accountDefault = Array.isArray(identities)
          ? identities.find((i) => i.is_default) ?? identities[0]
          : undefined;
        const sig = effective?.signature_html?.trim()
          ? effective.signature_html
          : accountDefault?.signature_html;
        if (sig) setSignatureHtml(sig);
      } catch (e) {
        console.error("Failed to load send-as addresses:", e);
      } finally {
        // Must flip even on failure: the insertion effect is gated on it, and
        // a composer that never signs is worse than one signed as the primary.
        if (!cancelled) setSignatureLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedAccountId]);

  // Every OTHER account's send-as list, loaded once so the From menu can offer
  // their aliases too. Separate from the effect above because that one gates
  // the signature and must not wait on eight other accounts' round trips.
  useEffect(() => {
    let cancelled = false;
    Promise.all(
      accounts.map(async (a) => {
        const list = await Promise.resolve(api.identities.listSendAs(a.id)).catch(() => [] as SendAsAddress[]);
        return [a.id, Array.isArray(list) ? list : []] as const;
      }),
    ).then((pairs) => {
      if (cancelled) return;
      const next: Record<string, SendAsAddress[]> = {};
      for (const [id, list] of pairs) if (list.length > 0) next[id] = list;
      if (Object.keys(next).length > 0) setSendAsByAccount((m) => ({ ...next, ...m }));
    });
    return () => {
      cancelled = true;
    };
    // Account identity, not the array identity — the store hands back a new
    // array on every sync tick and this would refetch nine lists each time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accounts.map((a) => a.id).join(",")]);

  // The signature insertion effect handles new compose (insert fresh
  // signatureBlock) and edit-existing-draft (already contains marker — skip). A
  // reopened draft that another client saved has lost those markers, so its
  // structure is recovered first or TipTap flattens it (T186).
  const initialContent = useMemo(
    () => (draftContext ? recoverForeignDraftHtml(defaultBody || "") : defaultBody || ""),
    // draftContext is fixed for the life of the window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [defaultBody],
  );

  // Bumped by ⌘K in the editor; LinkButton opens its field on each change.
  const [linkOpenSignal, setLinkOpenSignal] = useState(0);
  const editor = useEditor({
    extensions: [
      // StarterKit v3 already includes Link and Underline; registering them a
      // second time made TipTap warn about duplicate extensions.
      StarterKit.configure({ link: { openOnClick: false } }),
      ComposeShortcuts,
      Placeholder.configure({ placeholder: "Write your message..." }),
      // Required for signatures that include logo/image — without this,
      // TipTap's content parser silently drops <img> tags during setContent.
      Image.configure({ inline: true, allowBase64: true }),
      SignatureBlock,
      QuotedBlock,
      HtmlBlock,
    ],
    content: initialContent,
    editorProps: {
      attributes: {
        class: "prose prose-invert prose-sm max-w-none focus:outline-none min-h-[80px] px-4 py-3",
        spellcheck: "true",
      },
    },
  });

  const buildOutgoingEmail = (): OutgoingEmail => {
    // Edits typed inside a styled-HTML block bypass ProseMirror entirely, so the
    // document still holds pre-edit markup until this runs. Every serializing
    // path — autosave snapshot, send, schedule-send, pop-out — funnels through
    // here, which is why this one call is enough. The dispatch is synchronous,
    // so getHTML() below already sees the edit.
    flushHtmlBlockEdits();
    return {
      // The resolved send-as address, not the account's — an alias reply must
      // put the alias on the wire. `resolve_from` in the backend validates it
      // and refuses anything this account cannot send as, so a stale override
      // is caught before SMTP, not after.
      from_email: fromAddress?.email || fromAccount?.email || "",
      from_name: fromAddress?.display_name ?? fromAccount?.display_name ?? null,
      to: to.map((r) => ({ name: r.name, email: r.email })),
      cc: cc.map((r) => ({ name: r.name, email: r.email })),
      bcc: bcc.map((r) => ({ name: r.name, email: r.email })),
      subject,
      // Signature and quoted block live inside the TipTap editor as custom nodes,
      // so editor.getHTML() is the single source of truth for the body.
      html_body: editor?.getHTML() || "",
      plain_body: getPlainBody(editor) || null,
      in_reply_to: inReplyTo || null,
      references: referencesHeader || null,
      track_opens: trackOpens,
      attachments,
    };
  };

  /**
   * The live draft, as the validator sees it.
   *
   * Rebuilt on every call rather than memoized — Validate must judge what is
   * on screen right now, and a stale snapshot would report findings the user
   * has already fixed. `getText()` excludes the signature and quoted-history
   * nodes' inner text, which is what we want: neither is the user's prose for
   * this message, and flagging a signature's placeholder or an old quoted
   * "[Name]" would be noise on every single draft.
   */
  const getDraftSnapshot = useCallback((): DraftSnapshot => {
    flushHtmlBlockEdits();
    return {
      subject,
      to: to.map((r) => ({ name: r.name, email: r.email })),
      cc: cc.map((r) => ({ name: r.name, email: r.email })),
      bcc: bcc.map((r) => ({ name: r.name, email: r.email })),
      bodyText: editor?.getText() ?? "",
      bodyHtml: editor?.getHTML() ?? "",
      attachmentCount: attachments.length,
      inReplyTo: inReplyTo || null,
    };
  }, [subject, to, cc, bcc, editor, attachments, inReplyTo]);

  const openAssist = useCallback(
    (launch: AssistLaunch) => {
      // Freeze the selection before focus moves to the panel.
      const sel = editor?.state.selection;
      setAssistSelection(
        sel && sel.from !== sel.to
          ? {
              from: sel.from,
              to: sel.to,
              text: editor!.state.doc.textBetween(sel.from, sel.to, " "),
            }
          : null,
      );
      setAssistLaunch(launch);
    },
    [editor],
  );

  const hashDraft = (o: OutgoingEmail): string =>
    JSON.stringify({
      to: o.to.map((r) => r.email),
      cc: o.cc.map((r) => r.email),
      bcc: o.bcc.map((r) => r.email),
      subject: o.subject,
      html: o.html_body,
      attachments: (o.attachments ?? []).map((a) => a.filename),
    });

  const hasMeaningfulContent = (snap: { outgoing: OutgoingEmail; editorText: string }): boolean => {
    const { outgoing, editorText } = snap;
    if (outgoing.to.length + outgoing.cc.length + outgoing.bcc.length > 0) return true;
    if (outgoing.subject.trim().length > 0) return true;
    if (editorText.trim().length > 0) return true;
    if ((outgoing.attachments ?? []).length > 0) return true;
    return false;
  };

  // Insert the user's signature (and quoted block when forwarding/replying) into
  // the editor once the identity has loaded. Triggers on `quotedHtml` (covers
  // reply, reply-all, AND forward — none of which is implied by `replyContext`
  // alone) and preserves any defaultBody content (e.g., from SmartReplies).
  // Skip if the editor already contains a signature marker (edit-existing-draft).
  useEffect(() => {
    if (!editor) return;
    if (!signatureLoaded) return;
    if (signatureInsertedRef.current) return;
    const currentHtml = editor.getHTML();
    const placed = placeSignature({
      currentHtml,
      signatureHtml,
      quotedHtml: quotedHtml ?? null,
      isDraft: !!draftContext,
    });
    if (placed !== null) {
      editor.commands.setContent(placed);
      // For replies/forwards or new compose, place caret in the leading paragraph
      // (only meaningful when the body was empty — preserves caret position when
      // SmartReplies seeded a body).
      if (isEmptyEditorHtml(currentHtml)) editor.commands.focus("start");
    }
    signatureInsertedRef.current = true;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, signatureHtml, signatureLoaded, quotedHtml]);

  // Snapshot effect: keep latestDraftRef current on every editor update / form change.
  useEffect(() => {
    if (!editor) return;
    const snapshot = () => {
      latestDraftRef.current = { outgoing: buildOutgoingEmail(), editorText: editor.getText() };
      didInitRef.current = true;
    };
    snapshot();
    editor.on("update", snapshot);
    return () => { editor.off("update", snapshot); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, to, cc, bcc, subject, attachments, trackOpens, inReplyTo, referencesHeader, fromAccount, fromAddress?.email]);

  // Rebase the saved-hash baseline after an MCP reload settles. Declared AFTER the
  // snapshot effect so React runs it later in the SAME commit — by which point
  // latestDraftRef already holds the final buildOutgoingEmail() for the reloaded
  // content (the snapshot effect / editor "update" refreshed it). Seeding the
  // baseline from that exact object is what makes the pending debounced autosave
  // (which reads the same latestDraftRef) see hash equality and skip — with zero
  // dependence on byte-parity between two OutgoingEmail construction paths, the
  // fragility that let the reload re-save and re-mint the UID. A genuine user edit
  // after the reload mutates latestDraftRef, so the hashes diverge and it still saves.
  useEffect(() => {
    if (reloadSettleTick === 0) return;
    if (!latestDraftRef.current) return;
    lastSavedHashRef.current = hashDraft(latestDraftRef.current.outgoing);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reloadSettleTick]);

  // Capture the initial draft hash once the signature has been inserted, so we can
  // tell whether the user has actually changed anything since open.
  //
  // lastSavedHashRef must ONLY be seeded when an IMAP draft already exists at this
  // hash (i.e., we're editing an existing draft, draftContext is set). For new
  // compose / reply / forward / smart-reply seeded bodies, no IMAP save has
  // happened yet — seeding lastSavedHashRef would make saveOrEdit() see "current
  // == lastSaved" and silently skip the first save, so handleSaveDraft would
  // close the modal without anything reaching IMAP.
  useEffect(() => {
    if (!editor) return;
    if (!signatureInsertedRef.current) return;
    if (initialDraftSnapshotRef.current !== null) return;
    initialDraftSnapshotRef.current = hashDraft(buildOutgoingEmail());
    if (draftContext) {
      lastSavedHashRef.current = initialDraftSnapshotRef.current;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, signatureHtml, signatureLoaded]);

  // Serial save queue. Each call chains a closure that runs after all earlier saves
  // complete. The closure snapshots `latestDraftRef` at execution time and skips
  // when nothing has changed (hash match) — so multiple back-to-back enqueues
  // collapse to at most one network round-trip.
  //
  // - P1.2: a finalized check AFTER the await deletes the just-created draft if we
  //   were finalized while waiting (prevents send/discard/pop-out orphans).
  // - P1.3: returns a Promise that resolves when this save attempt completes, so
  //   manual Save Draft / pop-out can `await saveOrEdit()` to flush before closing.
  // - P2: synchronously updates `draftContextStateRef.current` BEFORE queuing the
  //   React state update, so the next chained save sees the new UID and uses
  //   `editDraft` instead of duplicating with another `saveDraft`.
  // `throwOnError: true` lets manual callers (Save Draft button) distinguish
  // a real failure from a no-op skip — autosave callers keep the default of
  // toast-and-swallow so a single network blip doesn't blow up the queue.
  const saveOrEdit = useCallback(
    async (opts?: { throwOnError?: boolean }): Promise<SavedDraftRef | null> => {
      const throwOnError = opts?.throwOnError ?? false;
      let result: SavedDraftRef | null = null;
      let savedError: unknown = null;
      const next = saveQueueRef.current.then(async () => {
        const accountId = resolvedAccountIdRef.current;
        if (!accountId) { console.debug("saveOrEdit: skip — no account"); return; }
        // Don't bail on finalizedRef here — manual Save Draft sets finalizedRef
        // immediately after awaiting, and pop-out/send drain the queue first.
        // The post-await finalizedRef check below handles the in-flight race.
        const snap = latestDraftRef.current;
        if (!snap) { console.debug("saveOrEdit: skip — no snapshot"); return; }
        if (!hasMeaningfulContent(snap)) { console.debug("saveOrEdit: skip — empty content"); return; }
        const currentHash = hashDraft(snap.outgoing);
        if (currentHash === lastSavedHashRef.current) { console.debug("saveOrEdit: skip — no changes"); return; }

        setSaveStatus("saving");
        try {
          const ref = draftContextStateRef.current
            ? await api.compose.editDraft(
                accountId,
                draftContextStateRef.current.folder,
                draftContextStateRef.current.uid,
                snap.outgoing,
              )
            : await api.compose.saveDraft(accountId, snap.outgoing);

          // If the user finalized (sent / discarded / popped out) while this
          // save was awaiting IMAP, delete the just-created draft to avoid an orphan.
          if (finalizedRef.current) {
            api.messages
              .delete(accountId, ref.folder, [ref.uid])
              .catch((e) => console.error("Failed to clean up orphan draft:", e));
            return;
          }

          // Write the ref synchronously so a chained save sees the new UID.
          draftContextStateRef.current = ref;
          setDraftContextState(ref);
          lastSavedHashRef.current = currentHash;
          result = ref;

          // save_draft / edit_draft only do IMAP work — they don't update the local
          // messages table. Without this sync the Drafts list keeps showing the old
          // (now-EXPUNGED) UID, so reopening the draft fetches a stale body.
          try {
            await api.messages.sync(accountId, ref.folder);
          } catch (e) {
            console.error("Drafts folder sync after save failed:", e);
          }
          window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
          setSaveStatus("saved");
          setLastSavedAt(new Date());
        } catch (e) {
          savedError = e;
          setSaveStatus("error");
          if (!throwOnError) {
            console.error("Autosave draft failed:", e);
            useUIStore.getState().addToast({
              type: "error",
              message: "Autosave failed",
              duration: 4000,
            });
            // Leave lastSavedHashRef unchanged so the next change naturally retries.
          }
        }
      });
      // Swallow rejections on the queue so one failure doesn't poison subsequent saves.
      saveQueueRef.current = next.catch(() => {});
      // Wait for the closure to finish; the inner try/catch already handled the error.
      await next.catch(() => {});
      if (throwOnError && savedError) throw savedError;
      return result;
    },
    [],
  );

  // 2s debounced autosave. Triggered by any change to form fields or editor content.
  // Each fire calls saveOrEdit() which enqueues at most one save per debounce window.
  const scheduleAutosave = useCallback(() => {
    if (finalizedRef.current) return;
    if (!signatureInsertedRef.current) return;
    // Suppressed while a Claude-conflict banner is awaiting the user's choice.
    if (mcpReloadPendingRef.current) return;
    if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    autosaveTimerRef.current = setTimeout(() => {
      // saveOrEdit dispatches cxmail:refresh-messages internally on success.
      saveOrEdit();
    }, 2000);
  }, [saveOrEdit]);

  useEffect(() => {
    scheduleAutosave();
    return () => {
      if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    };
  }, [to, cc, bcc, subject, attachments, signatureHtml, trackOpens, scheduleAutosave]);

  // Editor content changes don't show up in deps; hook a separate listener.
  useEffect(() => {
    if (!editor) return;
    const onUpdate = () => scheduleAutosave();
    editor.on("update", onUpdate);
    return () => { editor.off("update", onUpdate); };
  }, [editor, scheduleAutosave]);

  // A review describes the draft it ran against. Once the body changes the
  // count is no longer true, and a stale number on the Validate badge is worse
  // than none — it invites trusting a review of text that no longer exists.
  // Cleared only while the panel is closed; an Apply from the panel edits the
  // body too, and that count is already correct.
  useEffect(() => {
    if (!editor || assistLaunch) return;
    const clear = () => setReviewCount(undefined);
    editor.on("update", clear);
    return () => { editor.off("update", clear); };
  }, [editor, assistLaunch]);

  // Reconnect-without-typing: when the browser comes back online, kick a save if
  // the latest hash diverges from the last saved one.
  useEffect(() => {
    const onOnline = () => {
      if (finalizedRef.current) return;
      const snap = latestDraftRef.current;
      if (!snap) return;
      if (hashDraft(snap.outgoing) === lastSavedHashRef.current) return;
      saveOrEdit().catch((e) => console.error("Reconnect autosave failed:", e));
    };
    window.addEventListener("online", onOnline);
    return () => window.removeEventListener("online", onOnline);
  }, [saveOrEdit]);

  // Unmount fallback: fire a final save if there's unsaved meaningful content.
  useEffect(() => {
    return () => {
      if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
      if (!didInitRef.current) return;
      if (finalizedRef.current) return;
      const snap = latestDraftRef.current;
      if (!snap) return;
      if (!hasMeaningfulContent(snap)) return;
      if (hashDraft(snap.outgoing) === lastSavedHashRef.current) return;
      // Fire-and-forget; component is unmounting anyway.
      saveOrEdit().catch((e) => console.error("Unmount autosave failed:", e));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Fetch the freshly-edited draft from the new UID and apply it in place. The
  // field values come from `fresh` (the fetchBody result) and the body from the
  // editor's canonical post-parse HTML — NOT from the event envelope, which
  // carries only identifiers. After applying, it bumps reloadSettleTick; the
  // reseed effect then rebases the saved-hash onto the settled snapshot so the
  // debounced autosave these state changes schedule sees "no change" and skips,
  // instead of re-saving (which would re-mint the UID and clobber).
  const applyMcpReload = useCallback(async (env: McpActivity) => {
    if (!editor) return;
    try {
      const fresh = await api.messages.fetchBody(env.account_id, env.folder, env.uid);
      const freshTo = fresh.to_list.map((a) => ({ name: a.name, email: a.email }));
      const freshCc = fresh.cc_list.map((a) => ({ name: a.name, email: a.email }));
      setSubject(fresh.subject ?? "");
      setTo(freshTo);
      setCc(freshCc);
      if (freshCc.length > 0) setShowCcBcc(true);
      editor.commands.setContent(fresh.sanitized_html ?? "");
      // The fetched HTML already carries the data-cx-signature marker, so block
      // the signature-insertion effect from appending a second signature.
      signatureInsertedRef.current = true;

      // Adopt the reloaded content as the saved baseline so the debounced autosave
      // these setState/setContent calls just scheduled treats it as unchanged and
      // skips (a spurious save here re-mints the UID and re-opens the edit race).
      // Do NOT seed lastSavedHashRef from a hand-built OutgoingEmail — the autosave
      // compares against latestDraftRef (rebuilt by the snapshot effect via
      // buildOutgoingEmail), so seed from that same source. Bump the tick; the
      // reseed effect above sets the baseline once the snapshot has settled.
      setLastSavedAt(new Date());
      setReloadSettleTick((t) => t + 1);
    } catch (e) {
      console.error("Failed to reload draft after MCP edit:", e);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor]);

  // Listen for Claude edits to the draft this modal is editing. Match on
  // account + the expunged old UID equalling our current UID.
  useEffect(() => {
    const onDraftUpdated = (e: Event) => {
      const env = (e as CustomEvent<McpActivity>).detail;
      if (!env) return;
      const ctx = draftContextStateRef.current;
      if (!ctx) return; // new compose with no saved draft yet — nothing to match
      if (env.account_id !== resolvedAccountIdRef.current) return;
      // Only draft-updated carries old_uid; draft-created has none (guards
      // against a brand-new draft matching a not-yet-saved compose modal).
      if (typeof env.old_uid !== "number") return;
      if (env.old_uid !== ctx.uid) return;

      // Step 1 — ALWAYS adopt the new UID so the next autosave targets the live
      // draft (edit_draft expunged the old UID; saving to it would 404).
      const newCtx: SavedDraftRef = { folder: env.folder, uid: env.uid };
      draftContextStateRef.current = newCtx;
      setDraftContextState(newCtx);

      const snap = latestDraftRef.current;
      const isDirty = !!snap && hashDraft(snap.outgoing) !== lastSavedHashRef.current;
      if (isDirty) {
        // Don't clobber unsaved edits. Suppress autosave + surface a banner.
        if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
        mcpReloadPendingRef.current = true;
        setMcpUpdate(env);
      } else {
        void applyMcpReload(env);
      }
    };
    window.addEventListener("cxmail:draft-updated", onDraftUpdated);
    return () => window.removeEventListener("cxmail:draft-updated", onDraftUpdated);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applyMcpReload]);

  const handleMcpReload = useCallback(async () => {
    const env = mcpUpdate;
    setMcpUpdate(null);
    mcpReloadPendingRef.current = false;
    if (env) await applyMcpReload(env);
  }, [mcpUpdate, applyMcpReload]);

  const handleMcpKeepMine = useCallback(() => {
    setMcpUpdate(null);
    mcpReloadPendingRef.current = false;
    // Persist the user's local edits onto the new (live) UID — draftContextStateRef
    // already points at it — so Claude's server-side copy is overwritten with what
    // the user kept. The content is dirty by definition here, so this writes.
    saveOrEdit().catch((e) => console.error("Keep-mine autosave failed:", e));
  }, [saveOrEdit]);

  const hasAttachments = attachments.length > 0;

  const handleAddAttachment = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ multiple: true }) as string | string[] | null;
      if (!selected) return;
      const paths: string[] = Array.isArray(selected) ? selected : [selected];
      for (const path of paths) {
        const [filename, contentType, dataBase64] = await api.compose.readFile(path);
        setAttachments((prev) => [...prev, { filename, content_type: contentType, data_base64: dataBase64 }]);
      }
    } catch (e) {
      console.error("Failed to add attachment:", e);
    }
  };

  const removeAttachment = (index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  };

  const handleSaveAsTemplate = async () => {
    if (!templateName.trim()) return;
    try {
      await api.templates.create(
        resolvedAccountId || null,
        templateName.trim(),
        subject,
        editor?.getHTML() || "",
        editor?.getText() || null,
      );
      setShowSaveTemplate(false);
      setTemplateName("");
    } catch (e) {
      console.error("Failed to save template:", e);
    }
  };

  const handleLoadTemplates = async () => {
    try {
      const list = await api.templates.list(resolvedAccountId || null);
      setTemplates(list);
      setShowTemplateMenu(true);
    } catch (e) {
      console.error("Failed to load templates:", e);
    }
  };

  const applyTemplate = (template: EmailTemplate) => {
    setSubject(template.subject);
    setEditorContent(editor, template.html_body);
    setShowTemplateMenu(false);
  };

  // Swap the signature in place — a surgical DOM edit of the existing
  // [data-cx-signature] node. Do NOT re-run the insertion effect: it appends
  // quotedBlock unconditionally and would duplicate the quote on replies.
  // Shared by both From handlers below (the account switch and the plain
  // address switch), because both change which signature belongs to the draft.
  const swapSignature = useCallback(
    (newSig: string | null) => {
      setSignatureHtml(newSig);
      if (!editor) return;
      const doc = new DOMParser().parseFromString(editor.getHTML(), "text/html");
      const sigNode = doc.querySelector("[data-cx-signature]");
      if (sigNode) {
        if (newSig) sigNode.innerHTML = newSig;
        else sigNode.remove();
      } else if (newSig) {
        const block = doc.createElement("div");
        block.setAttribute("data-cx-signature", "1");
        block.className = "email-signature";
        block.innerHTML = newSig;
        const quote = doc.querySelector("[data-cx-quote]");
        if (quote) quote.before(block);
        else doc.body.appendChild(block);
      }
      // Only touch the editor when the signature actually changed —
      // setContent resets the caret.
      if (sigNode || newSig) editor.commands.setContent(doc.body.innerHTML);
    },
    [editor],
  );

  /** The signature that belongs to one send-as address: its own, else the
   * account's default identity signature. Mirrors the MCP's
   * `signature_for_send_as` — an alias without one inherits rather than
   * sending unsigned. */
  const signatureForAddress = useCallback(
    async (accountId: string, address: SendAsAddress | undefined): Promise<string | null> => {
      if (address?.signature_html?.trim()) return address.signature_html;
      const identities = await api.identities.list(accountId).catch(() => []);
      const fallback = Array.isArray(identities)
        ? identities.find((i) => i.is_default) ?? identities[0]
        : undefined;
      return fallback?.signature_html || null;
    },
    [],
  );

  // Switch the From account: swap the signature in place and migrate any
  // autosaved draft to the new account's Drafts folder.
  const handleFromChange = useCallback(
    async (newId: string, addressEmail?: string) => {
      setShowFromMenu(false);
      const oldAccountId = resolvedAccountId;
      if (!newId || newId === oldAccountId) return;
      setFromOverrideId(newId);
      // The previous account's alias must not carry over. `resolveFrom` already
      // ignores an address belonging to another account, but leaving it set
      // would resurrect it if the user switched back.
      setFromAddressOverride(addressEmail ?? null);
      fromAddressOverrideRef.current = addressEmail ?? null;
      // The reply default belongs to the account the original arrived on.
      setReplyDefaultFrom(null);
      // Flip the ref synchronously so any already-queued autosave that executes
      // before the re-render targets the new account, not the old one.
      resolvedAccountIdRef.current = newId;

      try {
        const sendAs = await Promise.resolve(api.identities.listSendAs(newId))
          .catch(() => [] as SendAsAddress[]);
        const addresses = Array.isArray(sendAs) ? sendAs : [];
        if (addresses.length > 0) {
          setSendAsByAccount((m) => ({ ...m, [newId]: addresses }));
        }
        const chosen = addressEmail
          ? addresses.find((a) => sameAddress(a.email, addressEmail))
          : addresses.find((a) => a.is_primary);
        swapSignature(await signatureForAddress(newId, chosen));
      } catch (e) {
        console.error("Failed to swap signature for From change:", e);
      }

      // Draft migration — chained on the serial save queue so it can't race an
      // in-flight autosave. Delete the old account's autosaved draft, then clear
      // the draft context + saved hash so the next autosave creates the draft
      // fresh in the new account's Drafts folder.
      const migrate = saveQueueRef.current.then(async () => {
        const ref = draftContextStateRef.current;
        if (!ref) return;
        if (oldAccountId) {
          try {
            await api.messages.delete(oldAccountId, ref.folder, [ref.uid]);
            window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
          } catch (e) {
            console.error("Failed to remove draft from previous account:", e);
            useUIStore.getState().addToast({
              type: "error",
              message: "Couldn't remove the draft from the previous account",
              duration: 4000,
            });
          }
        }
        draftContextStateRef.current = null;
        setDraftContextState(null);
        lastSavedHashRef.current = null;
      });
      saveQueueRef.current = migrate.catch(() => {});
      await migrate.catch(() => {});
      // Guarantee a follow-up save even when nothing else changes (e.g. neither
      // account has a signature), so the migrated draft reappears promptly.
      scheduleAutosave();
    },
    [resolvedAccountId, scheduleAutosave, signatureForAddress, swapSignature],
  );

  /**
   * Pick a From ADDRESS from the menu.
   *
   * Same account — the ordinary alias switch — is cheap: the draft stays where
   * it is and only the signature moves, so it deliberately does NOT go through
   * `handleFromChange`, which deletes the autosaved draft and re-creates it in
   * another account's Drafts folder. A different account still needs all of
   * that, and carries the chosen address through it.
   */
  const handleFromAddressChange = useCallback(
    async (address: SendAsAddress) => {
      setShowFromMenu(false);
      if (address.account_id !== resolvedAccountId) {
        await handleFromChange(address.account_id, address.email);
        return;
      }
      if (sameAddress(address.email, fromAddress?.email)) return;
      setFromAddressOverride(address.email);
      fromAddressOverrideRef.current = address.email;
      try {
        swapSignature(await signatureForAddress(address.account_id, address));
      } catch (e) {
        console.error("Failed to swap signature for From address change:", e);
      }
      scheduleAutosave();
    },
    [
      fromAddress?.email,
      handleFromChange,
      resolvedAccountId,
      scheduleAutosave,
      signatureForAddress,
      swapSignature,
    ],
  );

  const handleSend = () => {
    if (!resolvedAccountId || to.length === 0) return;
    const plainText = editor?.getText() || "";
    if (!hasAttachments && detectAttachmentMention(plainText)) {
      setShowAttachmentWarning(true);
      return;
    }
    executeSend();
  };

  const executeSend = async () => {
    if (!resolvedAccountId || to.length === 0) return;
    // Set early so any in-flight autosave's completion bails and deletes its
    // freshly-created draft (avoiding orphans).
    finalizedRef.current = true;
    if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    // Drain any in-flight save so cleanup uses the right UID.
    await saveQueueRef.current;
    setIsSending(true);
    setSendError(null);
    try {
      const result = await api.compose.send(resolvedAccountId, buildOutgoingEmail(), undoSendDelaySeconds);

      // Remember the chosen From for the next fresh compose. Replies that kept
      // their natural account (propAccountId set, dropdown untouched) don't
      // pollute the preference.
      if (!propAccountId || fromOverrideId !== null) {
        useUIStore.getState().setLastFromAccountId(resolvedAccountId);
      }

      // Capture AI-draft → sent diff for voice learning. For delayed sends we defer the
      // log until "send-completed" fires (so cancelled sends don't pollute the dataset).
      // Logged for both reply (replyContext) and fresh-compose (mode === "new" with a To address)
      // — recipientEmail is recorded so the per-recipient learning column gets populated.
      let voiceLearning:
        | {
            accountId: string;
            aiDraft: string;
            sentBody: string;
            recipientEmail?: string | null;
          }
        | undefined;
      if (aiDraftRef.current && (replyContext || to.length > 0)) {
        const sentBody = editor?.getText()?.trim() ?? "";
        if (sentBody.length > 0) {
          voiceLearning = {
            accountId: resolvedAccountId,
            aiDraft: aiDraftRef.current,
            sentBody,
            recipientEmail: to[0]?.email ?? null,
          };
        }
      }
      aiDraftRef.current = null;

      if (result.send_id) {
        // Delayed send: SMTP hasn't run yet. Defer draft cleanup until send-completed
        // fires so a failed send leaves the original draft intact for recovery.
        // Use draftContextStateRef so autosaved drafts get cleaned up too.
        const cleanupRef = draftContextStateRef.current
          ? { accountId: resolvedAccountId, folder: draftContextStateRef.current.folder, uid: draftContextStateRef.current.uid }
          : draftContext
          ? { accountId: draftContext.accountId, folder: draftContext.folder, uid: draftContext.uid }
          : undefined;
        setActiveSend({
          sendId: result.send_id,
          subject: subject || "(no subject)",
          startedAt: Date.now(),
          delaySeconds: undoSendDelaySeconds,
          draftCleanup: cleanupRef,
          voiceLearning,
        });
      } else if (voiceLearning) {
        // Immediate send already succeeded — log the edit now (no cancel possible)
        api.ai
          .logReplyEdit(
            voiceLearning.accountId,
            voiceLearning.aiDraft,
            voiceLearning.sentBody,
            voiceLearning.recipientEmail,
          )
          .catch(() => {});
      }

      // Create follow-up reminder if one was set
      if (followupRemindAt && result.message_id && to.length > 0) {
        const senderAccount = accounts.find((a) => a.id === resolvedAccountId);
        try {
          await api.followup.create(
            resolvedAccountId,
            result.message_id,
            senderAccount?.email || "",
            to[0]?.email || "",
            subject || null,
            followupRemindAt,
          );
        } catch (e) {
          console.error("Failed to create follow-up reminder:", e);
        }
      }

      // Immediate (non-delayed) send: SMTP already succeeded since `send` would
      // have thrown otherwise — safe to clean up the draft now. Prefer the
      // autosaved UID over the original draftContext (autosave may have replaced it).
      const cleanupCtx = draftContextStateRef.current
        ? { accountId: resolvedAccountId, folder: draftContextStateRef.current.folder, uid: draftContextStateRef.current.uid }
        : draftContext;
      if (!result.send_id && cleanupCtx) {
        try {
          await api.messages.delete(cleanupCtx.accountId, cleanupCtx.folder, [cleanupCtx.uid]);
        } catch (e) {
          console.error("Failed to clean up sent draft:", e);
        }
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      }

      onClose();
    } catch (e) {
      // Send failed — un-finalize so autosave can resume.
      finalizedRef.current = false;
      setSendError(String(e));
      setIsSending(false);
    }
  };

  const handleScheduleSend = async (sendAt: Date) => {
    if (!resolvedAccountId || to.length === 0) return;
    setIsSending(true);
    setSendError(null);
    try {
      const res = await api.schedule.schedule(resolvedAccountId, buildOutgoingEmail(), sendAt.toISOString());
      // Same last-used rule as executeSend.
      if (!propAccountId || fromOverrideId !== null) {
        useUIStore.getState().setLastFromAccountId(resolvedAccountId);
      }
      // Make the scheduled state + exact time unmistakable, and surface when a
      // re-schedule replaced an earlier one (so "the latest wins" is visible).
      const when = sendAt.toLocaleDateString(undefined, {
        weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
      });
      useUIStore.getState().addToast({
        type: "success",
        message:
          res.superseded > 0
            ? `Scheduled to send ${when} — replaced ${res.superseded} earlier scheduled send${res.superseded > 1 ? "s" : ""} to this recipient`
            : `Scheduled to send ${when}`,
        duration: 6000,
      });
      window.dispatchEvent(new CustomEvent("cxmail:scheduled-changed"));
      finalizedRef.current = true;
      onClose();
    } catch (e) {
      setSendError(String(e));
      setIsSending(false);
    }
  };

  const handleSaveDraft = async () => {
    if (!resolvedAccountId) return;
    if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    try {
      // Enqueue + await. The serial queue ensures any in-flight save plus this
      // explicit save both complete before we close. throwOnError makes a real
      // failure visible — without it we'd close the modal on a failed save and
      // the user would lose their work with no warning. saveOrEdit dispatches
      // cxmail:refresh-messages internally after the post-save folder sync.
      await saveOrEdit({ throwOnError: true });
      useUIStore.getState().addToast({
        type: "success",
        message: "Draft saved",
        duration: 2000,
      });
      finalizedRef.current = true;
      onClose();
    } catch (e) {
      // Keep modal open so the user can retry or copy their content out.
      setSendError(`Failed to save draft: ${String(e)}`);
    }
  };

  const handleDiscard = async () => {
    finalizedRef.current = true;
    if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    // Drain any in-flight autosave first — its post-await finalized check will
    // delete the just-created draft (so we don't double-delete or miss it).
    await saveQueueRef.current;
    // Prefer the autosaved UID over the original draftContext if autosave landed
    // before finalization. After draining, draftContextStateRef holds the latest.
    const ctx = draftContextStateRef.current
      ? { accountId: resolvedAccountId!, folder: draftContextStateRef.current.folder, uid: draftContextStateRef.current.uid }
      : draftContext;
    if (ctx) {
      try {
        await api.messages.delete(ctx.accountId, ctx.folder, [ctx.uid]);
        window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
      } catch (e) {
        console.error("Failed to discard draft:", e);
      }
    }
    onClose();
  };

  const handlePopOut = async () => {
    if (!onPopOut) return;
    if (autosaveTimerRef.current) clearTimeout(autosaveTimerRef.current);
    // Force a flush of any unsaved typing — `await saveQueueRef.current` only
    // drains saves that were already enqueued by the 2s debounce, so it can
    // miss the last 0–2s of typing. saveOrEdit() chains onto the queue and
    // hashes-out a no-op when nothing changed, so this is safe even when
    // there's nothing to save. Without this, the floating window would mount
    // pointing at a stale draftContext UID and could leave IMAP out of sync.
    await saveOrEdit();
    finalizedRef.current = true;
    onPopOut(buildOutgoingEmail(), draftContextStateRef.current, resolvedAccountId);

  };

  if (isMinimized && !isFloating && !isInline) {
    return (
      <div className="fixed bottom-0 right-6 z-50 w-[320px] overflow-hidden rounded-t-lg border border-border bg-surface-solid shadow-xl">
        <div
          className="flex cursor-pointer items-center justify-between px-4 py-2"
          onClick={() => setIsMinimized(false)}
        >
          <span className="truncate text-sm font-medium text-content">
            {subject || "New Message"}
          </span>
          <div className="flex items-center gap-1">
            <button
              onClick={(e) => { e.stopPropagation(); setIsMinimized(false); }}
              className="rounded p-0.5 text-content-secondary hover:text-content"
            >
              <ChevronUp className="h-4 w-4" />
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); onClose(); }}
              className="rounded p-0.5 text-content-secondary hover:text-content"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>
      </div>
    );
  }

  // Lifted off the bottom edge, the box rounds all four corners; sitting on it,
  // only the top two, as it always has.
  const isLifted = !!dock.offset && dock.offset.bottom > 0;
  const wrapperClass = isFloating
    ? "flex h-full flex-col overflow-hidden bg-base-solid"
    : isInline
    ? "flex flex-col overflow-hidden rounded-lg border border-border bg-base-solid shadow-md"
    : cn(
        "fixed bottom-0 right-6 z-50 flex w-[560px] flex-col overflow-hidden border border-border bg-base-solid shadow-2xl",
        isLifted ? "rounded-lg" : "rounded-t-lg",
      );

  const headerLabel = draftContext
    ? "Draft"
    : mode === "reply"
    ? "Reply"
    : mode === "reply-all"
    ? "Reply All"
    : mode === "forward"
    ? "Forward"
    : "New Message";

  return (
    <div
      ref={isDocked ? dockBoxRef : undefined}
      className={wrapperClass}
      style={isDocked && dock.offset ? { right: dock.offset.right, bottom: dock.offset.bottom } : undefined}
    >
      {/* Header - hidden when inside FloatingWindow (which provides its own chrome) */}
      {!isFloating && (
        <div
          onMouseDown={isDocked ? dock.onMouseDown : undefined}
          className={cn(
            "flex items-center justify-between border-b border-border bg-surface px-4 py-2",
            isDocked && "cursor-grab select-none active:cursor-grabbing",
          )}
        >
          <span className="text-sm font-medium text-content">{headerLabel}</span>
          <div className="flex items-center gap-1">
            {/* Pop out — available on both the inline reply box and the docked
                composer. The windowed surface hides this whole header, so a
                floating window can never show a redundant pop-out button. */}
            {onPopOut && (
              <button
                onClick={handlePopOut}
                className="rounded p-1 text-content-secondary hover:text-content"
                title="Pop out"
              >
                <Maximize2 className="h-4 w-4" />
              </button>
            )}
            {!isInline && (
              <button
                onClick={() => setIsMinimized(true)}
                className="rounded p-1 text-content-secondary hover:text-content"
              >
                <Minus className="h-4 w-4" />
              </button>
            )}
            <button
              onClick={onClose}
              className="rounded p-1 text-content-secondary hover:text-content"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}

      {/* Recipients */}
      <div className="border-b border-border-subtle">
        {/* From selector — an ADDRESS picker, not an account picker: an account
            can own aliases and a reply to mail that came in on one goes out from
            it. Hidden when there is exactly one address to choose from, which is
            the single-account no-alias case and keeps today's chrome. */}
        {shouldShowFromPicker(fromAddressOptions) && (
          <div className="relative flex items-center gap-2 px-4 py-1.5">
            <span className="shrink-0 text-xs text-content-muted">From:</span>
            <button
              onClick={() => setShowFromMenu((v) => !v)}
              aria-label="From address"
              className="flex min-w-0 items-center gap-1.5 rounded px-1 py-0.5 text-sm text-content hover:bg-surface"
            >
              <span
                className="h-2 w-2 shrink-0 rounded-full"
                style={{ backgroundColor: fromAccount?.color || "#0a84ff" }}
              />
              <span className="truncate">
                {sendAsLabel(fromAddress) || "Select address"}
              </span>
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-content-muted" />
            </button>
            {showFromMenu && (
              <div className="absolute left-12 top-full z-50 mt-1 w-80 rounded-lg border border-border bg-base-solid p-1 shadow-xl">
                {fromAddressOptions.map((option) => {
                  const optionAccount = accounts.find((a) => a.id === option.account_id);
                  return (
                    <button
                      key={`${option.account_id}:${option.email}`}
                      onClick={() => handleFromAddressChange(option)}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content"
                    >
                      <span
                        className="h-2 w-2 shrink-0 rounded-full"
                        style={{
                          backgroundColor: optionAccount?.color || "#0a84ff",
                          // An alias is drawn as a hollow dot in its account's
                          // colour: same mailbox, different address, and the
                          // relationship has to be readable at a glance in a
                          // flat list.
                          opacity: option.is_primary ? 1 : 0.45,
                        }}
                      />
                      <span className="min-w-0 flex-1 truncate text-left">{option.email}</span>
                      {!option.is_primary && (
                        <span className="shrink-0 text-[10px] uppercase tracking-wide text-content-faint">
                          alias
                        </span>
                      )}
                      {option.account_id === resolvedAccountId &&
                        sameAddress(option.email, fromAddress?.email) && (
                          <Check className="h-3.5 w-3.5 shrink-0 text-accent" />
                        )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        )}
        <RecipientField label="To" recipients={to} onChange={setTo}>
          {!showCcBcc && (
            <button
              onClick={() => setShowCcBcc(true)}
              className="shrink-0 text-xs text-content-muted hover:text-accent"
            >
              Cc/Bcc
            </button>
          )}
        </RecipientField>
        {showCcBcc && (
          <>
            <RecipientField label="Cc" recipients={cc} onChange={setCc} />
            <RecipientField label="Bcc" recipients={bcc} onChange={setBcc} />
          </>
        )}
      </div>

      {/* Subject */}
      <div className="flex items-center gap-1.5 border-b border-border-subtle px-4 py-2">
        <input
          type="text"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          placeholder="Subject"
          spellCheck={true}
          className="min-w-0 flex-1 bg-transparent text-sm text-content placeholder-content-muted outline-none"
        />
        <button
          onClick={async () => {
            const body = editor?.getText();
            if (!body?.trim()) return;
            setIsSuggestingSubject(true);
            try {
              const suggestion = await api.ai.suggestSubject(body);
              setSubject(suggestion);
            } catch (e) {
              console.error("Failed to suggest subject:", e);
            } finally {
              setIsSuggestingSubject(false);
            }
          }}
          disabled={isSuggestingSubject}
          className="shrink-0 rounded p-1 text-content-muted transition-colors hover:text-accent disabled:opacity-50"
          title="AI suggest subject"
        >
          {isSuggestingSubject ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
        </button>
      </div>

      {/* Claude-conflict banner: shown when Claude edited this draft while the
          user has unsaved local changes. Non-destructive — the user chooses. */}
      {mcpUpdate && (
        <div className="flex items-center justify-between gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2">
          <div className="flex min-w-0 items-center gap-2 text-xs text-warning">
            <Sparkles className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">Claude updated this draft — you have unsaved edits.</span>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <button
              onClick={handleMcpReload}
              className="rounded-md bg-warning/20 px-2.5 py-1 text-xs font-medium text-warning hover:bg-warning/30"
            >
              Reload
            </button>
            <button
              onClick={handleMcpKeepMine}
              className="rounded-md px-2.5 py-1 text-xs text-content-secondary hover:text-content"
            >
              Keep mine
            </button>
          </div>
        </div>
      )}

      {/* Toolbar */}
      {editor && (
        <EditorToolbar
          editor={editor}
          replyContext={replyContext}
          accountId={resolvedAccountId}
          firstRecipientEmail={to[0]?.email}
          composeMode={mode === "new" ? "compose" : "reply"}
          onOpenAssist={openAssist}
          reviewCount={reviewCount}
          linkOpenSignal={linkOpenSignal}
        />
      )}

      {editor && assistLaunch && (
        <AIAssistPanel
          open
          launch={assistLaunch}
          onClose={() => setAssistLaunch(null)}
          editor={editor}
          getDraft={getDraftSnapshot}
          accountId={resolvedAccountId}
          recipientEmail={to[0]?.email}
          subject={subject}
          onSubjectChange={setSubject}
          replyContext={replyContext}
          selection={assistSelection}
          onReviewCount={setReviewCount}
          onAIDraftGenerated={(draft) => {
            aiDraftRef.current = draft;
          }}
        />
      )}

      {/* Editor — signature and quoted block live inside the editor as custom nodes. */}
      <div
        className={`flex-1 overflow-auto ${isFloating ? '' : 'max-h-[300px]'}`}
        onKeyDown={(e) => {
          // ⌘K opens the link field. Claimed here rather than in a ProseMirror
          // keymap so it also works inside a styled HTML block, and
          // preventDefault is what tells the app-wide handler not to open the
          // command palette.
          if (isLinkShortcut(e)) {
            e.preventDefault();
            setLinkOpenSignal((n) => n + 1);
          }
        }}
      >
        <EditorContent editor={editor} />
      </div>

      {/* Attachments */}
      {attachments.length > 0 && (
        <div className="border-t border-border-subtle px-4 py-2">
          <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-content-secondary">
            <Paperclip className="h-3 w-3 text-content-muted" />
            <span>
              {attachments.length}{" "}
              {attachments.length === 1 ? "attachment" : "attachments"}
            </span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {attachments.map((att, i) => (
              <span
                key={i}
                className="flex items-center gap-1.5 rounded border border-border-subtle bg-surface px-2 py-1 text-xs text-content-secondary"
              >
                <Paperclip className="h-3 w-3 shrink-0 text-content-muted" />
                <span className="max-w-[220px] truncate" title={att.filename}>
                  {att.filename}
                </span>
                <span className="shrink-0 text-content-muted">
                  {formatBytes(base64ByteLength(att.data_base64))}
                </span>
                <button
                  onClick={() => removeAttachment(i)}
                  className="shrink-0 text-content-muted hover:text-content"
                  title="Remove attachment"
                  aria-label={`Remove ${att.filename}`}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Error */}
      {sendError && (
        <div className="border-t border-border bg-error/10 px-4 py-2 text-xs text-error">
          {sendError}
        </div>
      )}

      {/* Footer */}
      <div className="flex items-center justify-between border-t border-border px-4 py-2">
        <div className="flex items-center gap-2">
          {/* Send + Schedule split button */}
          <div className="relative flex">
            <button
              onClick={handleSend}
              disabled={to.length === 0 || isSending}
              className="flex items-center gap-1.5 rounded-l-md bg-accent px-4 py-1.5 text-sm font-medium text-content transition-colors hover:bg-accent-hover disabled:opacity-50"
            >
              {isSending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {isSending ? "Sending..." : "Send"}
            </button>
            <button
              onClick={() => setShowScheduleMenu(!showScheduleMenu)}
              disabled={to.length === 0 || isSending}
              className="flex items-center rounded-r-md border-l border-accent/30 bg-accent px-1.5 py-1.5 text-content transition-colors hover:bg-accent-hover disabled:opacity-50"
            >
              <ChevronDown className="h-3.5 w-3.5" />
            </button>

            {/* Schedule dropdown */}
            {showScheduleMenu && (
              <div className="absolute bottom-full left-0 z-50 mb-1 w-60 rounded-lg border border-border bg-base-solid p-1 shadow-xl">
                <div className="px-2 py-1.5 text-xs font-medium text-content-muted">
                  Schedule send
                </div>
                {buildSchedulePresets().map((preset) => {
                  const Icon = SCHEDULE_PRESET_ICONS[preset.key];
                  return (
                    <button
                      key={preset.key}
                      onClick={() => {
                        handleScheduleSend(preset.date);
                        setShowScheduleMenu(false);
                      }}
                      className="flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-left text-sm text-content-secondary hover:bg-surface hover:text-content"
                    >
                      <Icon className="h-4 w-4 shrink-0 text-content-muted" />
                      <div className="flex-1">
                        <div>{preset.label}</div>
                        <div className="text-xs text-content-muted">{formatPresetDate(preset.date)}</div>
                      </div>
                    </button>
                  );
                })}
                <div className="my-1 border-t border-border" />
                {showCustomSchedule ? (
                  <div className="px-2 py-1.5">
                    <input
                      type="datetime-local"
                      value={customScheduleDate}
                      onChange={(e) => setCustomScheduleDate(e.target.value)}
                      className="w-full rounded border border-border bg-sidebar px-2 py-1.5 text-sm text-content-secondary outline-none focus:border-accent"
                      min={toLocalDateTimeInput()}
                    />
                    <div className="mt-1.5 flex justify-end gap-1.5">
                      <button
                        onClick={() => { setShowCustomSchedule(false); setCustomScheduleDate(""); }}
                        className="rounded px-2 py-1 text-xs text-content-secondary hover:text-content"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={() => {
                          if (customScheduleDate) {
                            handleScheduleSend(new Date(customScheduleDate));
                            setShowScheduleMenu(false);
                            setShowCustomSchedule(false);
                            setCustomScheduleDate("");
                          }
                        }}
                        disabled={!customScheduleDate}
                        className="rounded bg-accent px-2 py-1 text-xs text-content hover:bg-accent-hover disabled:opacity-40"
                      >
                        Schedule
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    onClick={() => setShowCustomSchedule(true)}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content"
                  >
                    <Clock className="h-4 w-4 text-content-muted" />
                    Pick date & time
                  </button>
                )}
              </div>
            )}
          </div>
          <button
            onClick={handleSaveDraft}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm text-content-secondary hover:text-content"
            title="Save draft"
          >
            {saveStatus === "saving" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : saveStatus === "error" ? (
              <AlertTriangle className="h-3.5 w-3.5 text-amber-400" />
            ) : (
              <Save className="h-3.5 w-3.5" />
            )}
            {saveStatus === "saving" ? (
              <span className="text-xs text-content-muted">Saving…</span>
            ) : saveStatus === "error" ? (
              <span className="text-xs text-amber-400">Save failed</span>
            ) : lastSavedAt ? (
              <span className="text-xs text-content-muted">
                Saved {new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(lastSavedAt)}
              </span>
            ) : null}
          </button>
          {trackingConfigured && (
            <button
              onClick={() => setTrackOpens(!trackOpens)}
              className={cn(
                "flex items-center gap-1 rounded-md px-2 py-1.5 text-sm transition-colors",
                trackOpens ? "text-accent" : "text-content-muted hover:text-content-secondary"
              )}
              title={trackOpens ? "Open tracking enabled" : "Open tracking disabled"}
            >
              {trackOpens ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
            </button>
          )}
          <FollowupPopover onSetReminder={(remindAt) => setFollowupRemindAt(remindAt)}>
            <button
              className={cn(
                "flex items-center gap-1 rounded-md px-2 py-1.5 text-sm transition-colors",
                followupRemindAt ? "text-warning" : "text-content-muted hover:text-content-secondary"
              )}
              title={followupRemindAt ? "Follow-up reminder set" : "Set follow-up reminder"}
            >
              <BellRing className="h-3.5 w-3.5" />
              {followupRemindAt && (
                <span className="text-[10px]">
                  {new Date(followupRemindAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                </span>
              )}
            </button>
          </FollowupPopover>
          <button
            onClick={handleAddAttachment}
            className="flex items-center gap-1 rounded-md px-2 py-1.5 text-sm text-content-muted hover:text-content-secondary"
            title="Attach files"
          >
            <Paperclip className="h-3.5 w-3.5" />
          </button>
          <div className="relative">
            <button
              onClick={handleLoadTemplates}
              className="flex items-center gap-1 rounded-md px-2 py-1.5 text-sm text-content-muted hover:text-content-secondary"
              title="Templates"
            >
              <FileText className="h-3.5 w-3.5" />
            </button>
            {showTemplateMenu && (
              <div className="absolute bottom-full left-0 z-50 mb-1 w-56 rounded-lg border border-border bg-base-solid p-1 shadow-xl">
                <div className="px-2 py-1.5 text-xs font-medium text-content-muted">
                  Templates
                </div>
                {templates.length === 0 ? (
                  <div className="px-2 py-2 text-xs text-content-muted">No templates yet</div>
                ) : (
                  templates.map((t) => (
                    <button
                      key={t.id}
                      onClick={() => applyTemplate(t)}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content"
                    >
                      <FileText className="h-3.5 w-3.5 text-content-muted" />
                      <span className="truncate">{t.name}</span>
                    </button>
                  ))
                )}
                <div className="my-1 border-t border-border" />
                {showSaveTemplate ? (
                  <div className="px-2 py-1.5">
                    <input
                      type="text"
                      value={templateName}
                      onChange={(e) => setTemplateName(e.target.value)}
                      placeholder="Template name"
                      className="w-full rounded border border-border bg-sidebar px-2 py-1.5 text-sm text-content-secondary outline-none focus:border-accent"
                      onKeyDown={(e) => { if (e.key === "Enter") handleSaveAsTemplate(); }}
                      autoFocus
                    />
                    <div className="mt-1.5 flex justify-end gap-1.5">
                      <button
                        onClick={() => { setShowSaveTemplate(false); setTemplateName(""); }}
                        className="rounded px-2 py-1 text-xs text-content-secondary hover:text-content"
                      >
                        Cancel
                      </button>
                      <button
                        onClick={handleSaveAsTemplate}
                        disabled={!templateName.trim()}
                        className="rounded bg-accent px-2 py-1 text-xs text-content hover:bg-accent-hover disabled:opacity-40"
                      >
                        Save
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    onClick={() => setShowSaveTemplate(true)}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content"
                  >
                    <Save className="h-3.5 w-3.5 text-content-muted" />
                    Save as template
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
        <button
          onClick={handleDiscard}
          className="rounded-md px-3 py-1.5 text-sm text-content-secondary hover:text-content"
        >
          Discard
        </button>
      </div>

      {/* While dragging, cover the window: the reading pane's email iframe
          would otherwise swallow mousemove and drop the box mid-drag (the
          same overlay FloatingWindow uses). */}
      {dock.dragging && (
        <div
          data-testid="compose-drag-overlay"
          className="fixed inset-0 cursor-grabbing"
          style={{ zIndex: 2147483000 }}
        />
      )}

      {showAttachmentWarning && (
        <div
          className="fixed inset-0 z-[100] flex items-center justify-center bg-overlay"
          onKeyDown={(e) => { if (e.key === "Escape") setShowAttachmentWarning(false); }}
          onClick={() => setShowAttachmentWarning(false)}
          tabIndex={-1}
        >
          <div
            className="w-[400px] overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-3 border-b border-border-subtle bg-surface px-4 py-3">
              <AlertTriangle className="h-5 w-5 text-warning" />
              <p className="text-sm font-medium text-content">No attachment found</p>
            </div>
            <div className="px-4 py-4">
              <p className="text-sm text-content-secondary">
                Your message mentions an attachment, but no files are attached. Send anyway?
              </p>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-border-subtle px-4 py-3">
              <button
                onClick={() => setShowAttachmentWarning(false)}
                className="rounded-md bg-elevated px-4 py-2 text-sm text-content-secondary hover:bg-elevated"
              >
                Go Back
              </button>
              <button
                onClick={() => { setShowAttachmentWarning(false); executeSend(); }}
                className="rounded-md bg-accent px-4 py-2 text-sm font-medium text-content hover:bg-accent-hover"
              >
                Send Anyway
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function EditorToolbar({
  editor,
  replyContext,
  accountId,
  firstRecipientEmail,
  composeMode,
  onOpenAssist,
  reviewCount,
  linkOpenSignal,
}: {
  editor: ReturnType<typeof useEditor>;
  replyContext?: { accountId: string; folder: string; uid: number };
  accountId?: string;
  firstRecipientEmail?: string;
  composeMode?: "reply" | "compose";
  onOpenAssist: (launch: AssistLaunch) => void;
  reviewCount?: number;
  linkOpenSignal: number;
}) {
  if (!editor) return null;

  // Inside a styled-HTML block the caret is the browser's, not ProseMirror's —
  // the block stops the events that would keep ProseMirror's selection current
  // (see composeNodes.ts). Running a TipTap command there would apply formatting
  // at whatever position ProseMirror last knew about, somewhere else in the
  // draft. The equivalent contentEditable command targets the real caret and
  // leaves the block's surrounding inline styles alone.
  const format = (nativeCommand: string, tiptapCommand: () => void, value?: string) => {
    if (isEditingHtmlBlock()) {
      try {
        document.execCommand(nativeCommand, false, value);
      } catch (e) {
        console.error(`Native ${nativeCommand} failed inside styled block:`, e);
      }
      return;
    }
    tiptapCommand();
  };

  const buttons = [
    {
      icon: Bold,
      action: () => format("bold", () => editor.chain().focus().toggleBold().run()),
      active: editor.isActive("bold"),
    },
    {
      icon: Italic,
      action: () => format("italic", () => editor.chain().focus().toggleItalic().run()),
      active: editor.isActive("italic"),
    },
    {
      icon: UnderlineIcon,
      action: () => format("underline", () => editor.chain().focus().toggleUnderline().run()),
      active: editor.isActive("underline"),
    },
    {
      icon: List,
      action: () =>
        format("insertUnorderedList", () => editor.chain().focus().toggleBulletList().run()),
      active: editor.isActive("bulletList"),
    },
    {
      icon: ListOrdered,
      action: () =>
        format("insertOrderedList", () => editor.chain().focus().toggleOrderedList().run()),
      active: editor.isActive("orderedList"),
    },
  ];

  return (
    <div className="flex items-center gap-0.5 border-b border-border-subtle px-3 py-1">
      {buttons.map(({ icon: Icon, action, active }, i) => (
        <button
          key={i}
          // Keep the click from moving focus: inside a styled-HTML block the
          // caret IS the selection these commands act on, and blurring would
          // discard it before onClick runs.
          onMouseDown={(e) => e.preventDefault()}
          onClick={action}
          className={cn(
            "rounded p-1.5 transition-colors",
            active ? "bg-elevated text-content" : "text-content-muted hover:text-content"
          )}
        >
          <Icon className="h-4 w-4" />
        </button>
      ))}
      <LinkButton editor={editor} openSignal={linkOpenSignal} />
      <div className="mx-1 h-4 w-px bg-border" />
      <AIWritingMenu
        editor={editor}
        replyContext={replyContext}
        accountId={accountId}
        firstRecipientEmail={firstRecipientEmail}
        composeMode={composeMode}
        onOpenAssist={onOpenAssist}
        reviewCount={reviewCount}
      />
    </div>
  );
}

interface RecipientFieldProps {
  label: string;
  recipients: Recipient[];
  onChange: (recipients: Recipient[]) => void;
  children?: React.ReactNode;
}

function RecipientField({ label, recipients, onChange, children }: RecipientFieldProps) {
  const [input, setInput] = useState("");
  const [suggestions, setSuggestions] = useState<ContactResult[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const searchContacts = useCallback(async (query: string) => {
    if (query.length < 2) {
      setSuggestions([]);
      return;
    }
    try {
      const results = await api.messages.searchContacts(query);
      // Filter out already-added recipients
      const existing = new Set(recipients.map((r) => r.email));
      setSuggestions(results.filter((c) => !existing.has(c.email)));
    } catch {
      setSuggestions([]);
    }
  }, [recipients]);

  const handleInputChange = (value: string) => {
    setInput(value);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => searchContacts(value), 200);
    setShowSuggestions(true);
  };

  const addRecipient = (recipient: Recipient) => {
    onChange([...recipients, recipient]);
    setInput("");
    setSuggestions([]);
    setShowSuggestions(false);
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" || e.key === "Tab" || e.key === ",") {
      e.preventDefault();
      const trimmed = input.trim().replace(/,$/, "");
      if (trimmed && trimmed.includes("@")) {
        addRecipient({ name: null, email: trimmed });
      } else if (suggestions.length > 0) {
        addRecipient({ name: suggestions[0].name, email: suggestions[0].email });
      }
    } else if (e.key === "Backspace" && !input && recipients.length > 0) {
      onChange(recipients.slice(0, -1));
    }
  };

  const removeRecipient = (index: number) => {
    onChange(recipients.filter((_, i) => i !== index));
  };

  return (
    <div className="relative flex items-start gap-2 px-4 py-1.5">
      <span className="shrink-0 pt-0.5 text-xs text-content-muted">{label}:</span>
      <div className="flex min-h-[24px] flex-1 flex-wrap items-center gap-1">
        {recipients.map((r, i) => (
          <span
            key={i}
            className="flex items-center gap-1 rounded bg-surface px-2 py-0.5 text-xs text-content-secondary"
          >
            {r.name || r.email}
            <button onClick={() => removeRecipient(i)} className="text-content-muted hover:text-content">
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={() => setTimeout(() => setShowSuggestions(false), 200)}
          onFocus={() => input.length >= 2 && setShowSuggestions(true)}
          className="min-w-[120px] flex-1 bg-transparent text-sm text-content outline-none placeholder:text-content-faint"
          placeholder={recipients.length === 0 ? "Type an email..." : ""}
        />
      </div>
      {children}

      {/* Suggestions dropdown */}
      {showSuggestions && suggestions.length > 0 && (
        <div className="absolute left-14 top-full z-50 mt-1 w-[calc(100%-70px)] rounded-md border border-border bg-surface-solid py-1 shadow-lg">
          {suggestions.map((s, i) => (
            <button
              key={i}
              onMouseDown={(e) => { e.preventDefault(); addRecipient({ name: s.name, email: s.email }); }}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-elevated"
            >
              <span className="text-content">{s.name || s.email}</span>
              {s.name && <span className="text-xs text-content-muted">&lt;{s.email}&gt;</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
