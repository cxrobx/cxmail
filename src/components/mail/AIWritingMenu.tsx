import { useState, useRef, useEffect } from "react";
import { api } from "@/lib/tauri";
import type { RecipientProfileStatusKind } from "@/types/email";
import { plainToHtmlParagraphs } from "@/lib/utils";

// Module-scoped memo so the lazy LLM extraction fires at most once per
// (account, recipient) per app session — not on every menu open.
const recipientStatusCache = new Map<string, RecipientProfileStatusKind>();
const inFlightStatus = new Map<string, Promise<RecipientProfileStatusKind>>();
const statusCacheKey = (a: string, r: string) => `${a}::${r.toLowerCase()}`;
import {
  Sparkles,
  Loader2,
  Wand2,
  Briefcase,
  Coffee,
  Heart,
  Scissors,
  AlignLeft,
  PenLine,
  SpellCheck,
  ChevronRight,
  ShieldCheck,
  MessageSquarePlus,
} from "lucide-react";
import type { Editor } from "@tiptap/react";
import { setEditorContent } from "@/lib/utils";
import type { AssistLaunch } from "./AIAssistPanel";

interface AIWritingMenuProps {
  editor: Editor;
  /** Pass these to enable "Draft reply" — requires the original email context */
  replyContext?: {
    accountId: string;
    folder: string;
    uid: number;
  };
  /** Active account for compose-mode drafting (no-op when undefined). */
  accountId?: string;
  /** First recipient address from the To field. Used to load the recipient voice profile. */
  firstRecipientEmail?: string;
  /** "compose" enables the "Draft for me" action; "reply" hides it. */
  composeMode?: "reply" | "compose";
  /**
   * Hand off to the floating AI Assist panel.
   *
   * Everything that takes an instruction or produces something to read now
   * lives there instead of in this dropdown's one-line inputs. The menu keeps
   * only the one-shot actions — tone presets and proofread — which apply
   * straight to the editor and have nothing to show.
   */
  onOpenAssist: (launch: AssistLaunch) => void;
  /** Findings from the last review, for the Validate badge. */
  reviewCount?: number;
}

const TONE_OPTIONS = [
  { label: "Professional", value: "professional", icon: Briefcase },
  { label: "Casual", value: "casual", icon: Coffee },
  { label: "Friendly", value: "friendly and warm", icon: Heart },
  { label: "Concise", value: "concise and direct — shorten significantly while keeping the key points", icon: Scissors },
  { label: "Detailed", value: "detailed and thorough — expand on the points", icon: AlignLeft },
] as const;

export default function AIWritingMenu({
  editor,
  replyContext,
  accountId,
  firstRecipientEmail,
  composeMode = "reply",
  onOpenAssist,
  reviewCount,
}: AIWritingMenuProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [showToneSubmenu, setShowToneSubmenu] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<RecipientProfileStatusKind | "loading" | null>(
    null,
  );
  const menuRef = useRef<HTMLDivElement>(null);

  const showDraftForMe = composeMode === "compose" && !!accountId && !!firstRecipientEmail;

  const launch = (l: AssistLaunch) => {
    closeMenu();
    onOpenAssist(l);
  };

  // Close on outside click
  useEffect(() => {
    if (!isOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        closeMenu();
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [isOpen]);

  // When the menu opens in compose mode with a recipient, fetch the voice
  // profile status. Memoized per (account, recipient) for the app session — a
  // recipient cache hit returns instantly; a miss runs at most one backend call
  // even if the user opens/closes the menu repeatedly. Backend has its own
  // server-side cache + 30s extraction timeout.
  useEffect(() => {
    if (!isOpen || !accountId || !firstRecipientEmail) {
      setVoiceStatus(null);
      return;
    }
    const key = statusCacheKey(accountId, firstRecipientEmail);
    const cached = recipientStatusCache.get(key);
    if (cached) {
      setVoiceStatus(cached);
      return;
    }
    let cancelled = false;
    setVoiceStatus("loading");
    let pending = inFlightStatus.get(key);
    if (!pending) {
      pending = api.ai
        .getRecipientProfile(accountId, firstRecipientEmail)
        .then((res) => {
          recipientStatusCache.set(key, res.status);
          inFlightStatus.delete(key);
          return res.status;
        })
        .catch(() => {
          inFlightStatus.delete(key);
          recipientStatusCache.set(key, "missing");
          return "missing" as const;
        });
      inFlightStatus.set(key, pending);
    }
    pending.then((status) => {
      if (!cancelled) setVoiceStatus(status);
    });
    return () => {
      cancelled = true;
    };
  }, [isOpen, accountId, firstRecipientEmail]);

  const closeMenu = () => {
    setIsOpen(false);
    setShowToneSubmenu(false);
  };

  const getSelectedOrFullText = (): string => {
    const { from, to } = editor.state.selection;
    if (from !== to) {
      return editor.state.doc.textBetween(from, to, " ");
    }
    return editor.getText();
  };

  const replaceContent = (html: string) => {
    const { from, to } = editor.state.selection;
    if (from !== to) {
      // Replace selection
      editor.chain().focus().deleteSelection().insertContent(html).run();
    } else {
      // Replace full content
      setEditorContent(editor, html);
    }
  };

  const handleToneAdjust = async (tone: string) => {
    const text = getSelectedOrFullText();
    if (!text.trim()) return;
    setIsLoading(true);
    try {
      const result = await api.ai.adjustTone(text, tone);
      replaceContent(plainToHtmlParagraphs(result));
      closeMenu();
    } catch (e) {
      console.error("Tone adjustment failed:", e);
    } finally {
      setIsLoading(false);
    }
  };

  const handleProofread = async () => {
    const text = getSelectedOrFullText();
    if (!text.trim()) return;
    setIsLoading(true);
    try {
      const result = await api.ai.proofread(text);
      replaceContent(plainToHtmlParagraphs(result));
      closeMenu();
    } catch (e) {
      console.error("Proofread failed:", e);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="relative" ref={menuRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        disabled={isLoading}
        className="flex items-center gap-1 rounded p-1.5 text-ai hover:bg-surface hover:text-ai transition-colors disabled:opacity-50"
        title="AI Writing Assist"
      >
        {isLoading ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Sparkles className="h-4 w-4" />
        )}
      </button>

      {isOpen && (
        <div className="absolute left-0 top-full z-50 mt-1 w-56 rounded-lg border border-border bg-base-solid p-1 shadow-xl">
          <div className="px-2 py-1.5 text-xs font-medium text-content-muted">
            AI Writing
          </div>

          {/* Voice status badge — only in compose mode with a recipient */}
          {composeMode === "compose" && firstRecipientEmail && voiceStatus && (
            <div className="px-2 pb-1.5 text-[11px] text-content-faint">
              {voiceStatus === "loading" && (
                <span className="flex items-center gap-1">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  Building voice profile…
                </span>
              )}
              {voiceStatus === "fresh" && (
                <span title={firstRecipientEmail}>
                  Voice: matched to {firstRecipientEmail}
                </span>
              )}
              {voiceStatus === "stale" && (
                <span title={firstRecipientEmail}>
                  Voice: matched (last refresh failed)
                </span>
              )}
              {(voiceStatus === "missing" || voiceStatus === "insufficient_samples") && (
                <span>Voice: account default</span>
              )}
            </div>
          )}

          {/* Validate — first, because checking is cheaper than rewriting */}
          <button
            onClick={() => launch({ mode: "review" })}
            disabled={isLoading}
            className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content hover:bg-surface disabled:opacity-50"
          >
            <ShieldCheck className="h-4 w-4 text-ai" />
            Validate
            {reviewCount !== undefined && reviewCount > 0 && (
              <span className="ml-auto flex h-4 min-w-4 items-center justify-center rounded-full bg-warning/20 px-1 text-[10px] font-semibold text-warning">
                {reviewCount}
              </span>
            )}
          </button>

          <div className="my-1 border-t border-border-subtle" />

          {/* Draft for me — fresh compose path */}
          {showDraftForMe && (
            <button
              onClick={() =>
                launch({
                  mode: "chat",
                  chat: { kind: "draft", instruction: "", autoRun: false },
                })
              }
              disabled={isLoading}
              className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
            >
              <MessageSquarePlus className="h-4 w-4 text-content-muted" />
              Draft for me
            </button>
          )}

          {/* Draft Reply — only when replying */}
          {replyContext && (
            <button
              onClick={() =>
                launch({
                  mode: "chat",
                  chat: { kind: "reply", instruction: "", autoRun: true },
                })
              }
              disabled={isLoading}
              className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
            >
              <MessageSquarePlus className="h-4 w-4 text-content-muted" />
              Draft Reply
            </button>
          )}

          {/* Tone Adjust submenu */}
          <div
            className="relative"
            onMouseEnter={() => setShowToneSubmenu(true)}
            onMouseLeave={() => setShowToneSubmenu(false)}
          >
            <button disabled={isLoading} className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50">
              <Wand2 className="h-4 w-4 text-content-muted" />
              Adjust Tone
              <ChevronRight className="ml-auto h-3.5 w-3.5 text-content-faint" />
            </button>
            {showToneSubmenu && (
              <div className="absolute left-full top-0 z-50 ml-1 w-48 rounded-lg border border-border bg-base-solid p-1 shadow-xl">
                {TONE_OPTIONS.map(({ label, value, icon: Icon }) => (
                  <button
                    key={value}
                    onClick={() => handleToneAdjust(value)}
                    disabled={isLoading}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
                  >
                    <Icon className="h-4 w-4 text-content-muted" />
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Custom Rewrite — opens the panel so the result can be read and refined */}
          <button
            onClick={() =>
              launch({
                mode: "chat",
                chat: { kind: "rewrite", instruction: "", autoRun: false },
              })
            }
            disabled={isLoading}
            className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
          >
            <PenLine className="h-4 w-4 text-content-muted" />
            Rewrite with Instructions
          </button>

          {/* Fix Spelling & Grammar — one-shot proofread of selection or full draft */}
          <button
            onClick={handleProofread}
            disabled={isLoading}
            className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-content-secondary hover:bg-surface hover:text-content disabled:opacity-50"
          >
            <SpellCheck className="h-4 w-4 text-content-muted" />
            Fix Spelling & Grammar
          </button>
        </div>
      )}
    </div>
  );
}
