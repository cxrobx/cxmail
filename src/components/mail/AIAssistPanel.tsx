/**
 * The floating AI Assist panel — one surface with two modes.
 *
 *   Review — deterministic findings, then the LLM pass, over a chip-strip
 *            overview nav (every flagged section visible without a click).
 *   Chat   — a conversation thread for Draft / Rewrite / follow-ups.
 *
 * The two are connected: a finding whose fix needs judgement hands off to Chat
 * with a seeded instruction, and Chat results apply back to the draft.
 *
 * Portaled to `document.body` on purpose. The compose window renders both
 * docked inside the reading pane and popped out inside a FloatingWindow; a
 * portal makes the panel behave identically in both and keeps it clear of the
 * compose modal's stacking context (gotcha #37's lesson).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Editor } from "@tiptap/react";
import {
  AlertOctagon,
  AlertTriangle,
  ArrowUp,
  Check,
  CornerDownLeft,
  GripVertical,
  Info,
  Loader2,
  MessageSquare,
  Pin,
  RotateCw,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { api } from "@/lib/tauri";
import { cn, plainToHtmlParagraphs } from "@/lib/utils";
import {
  countBySeverity,
  fromAiFinding,
  sortFindings,
  validateDraft,
  type DraftSnapshot,
  type Finding,
  type FindingSeverity,
} from "@/lib/draftValidation";
import { appendParagraph, locateText, replaceText } from "@/lib/draftEdits";

export type AssistMode = "review" | "chat";

/** What the menu asked for when it opened the panel. */
export interface AssistLaunch {
  mode: AssistMode;
  /** Chat only: the kind of generation to run, and how to label the turn. */
  chat?: {
    kind: "draft" | "reply" | "rewrite";
    /** Prefilled instruction. Empty means "wait for the user to type". */
    instruction: string;
    /** Run immediately on open rather than waiting for a send. */
    autoRun: boolean;
  };
}

interface ChatTurn {
  id: number;
  role: "user" | "assistant";
  text: string;
  /** Assistant turns carry a result that can be applied to the draft. */
  applicable?: boolean;
}

interface AIAssistPanelProps {
  open: boolean;
  onClose: () => void;
  launch: AssistLaunch;
  editor: Editor;
  /** Rebuilt on demand so validation always sees the live draft. */
  getDraft: () => DraftSnapshot;
  accountId?: string;
  recipientEmail?: string;
  subject: string;
  onSubjectChange: (subject: string) => void;
  replyContext?: { accountId: string; folder: string; uid: number };
  /** Selection at launch — lost once focus moves to the panel. */
  selection: { from: number; to: number; text: string } | null;
  /** Reports the finding count back, so the menu can badge Validate. */
  onReviewCount?: (n: number | undefined) => void;
  onAIDraftGenerated?: (plain: string) => void;
}

const SEVERITY: Record<
  FindingSeverity,
  { icon: typeof Info; text: string; bg: string; ring: string; dot: string; label: string }
> = {
  error: {
    icon: AlertOctagon,
    text: "text-error",
    bg: "bg-error/10",
    ring: "ring-error/30",
    dot: "bg-error",
    label: "Blocker",
  },
  warning: {
    icon: AlertTriangle,
    text: "text-warning",
    bg: "bg-warning/10",
    ring: "ring-warning/30",
    dot: "bg-warning",
    label: "Check",
  },
  info: {
    icon: Info,
    text: "text-content-muted",
    bg: "bg-surface",
    ring: "ring-border",
    dot: "bg-content-faint",
    label: "Note",
  },
};

const PANEL_W = 440;
const PANEL_H = 560;
const MARGIN = 16;

export default function AIAssistPanel({
  open,
  onClose,
  launch,
  editor,
  getDraft,
  accountId,
  recipientEmail,
  subject,
  onSubjectChange,
  replyContext,
  selection,
  onReviewCount,
  onAIDraftGenerated,
}: AIAssistPanelProps) {
  const [mode, setMode] = useState<AssistMode>(launch.mode);
  const [pos, setPos] = useState(() => ({
    x: Math.max(MARGIN, window.innerWidth - PANEL_W - MARGIN * 2),
    y: 96,
  }));
  const [size, setSize] = useState({ w: PANEL_W, h: PANEL_H });

  /* Review state */
  const [instant, setInstant] = useState<Finding[]>([]);
  const [aiFindings, setAiFindings] = useState<Finding[]>([]);
  const [aiState, setAiState] = useState<"idle" | "loading" | "done" | "error">("idle");
  const [aiError, setAiError] = useState<string | null>(null);
  const [index, setIndex] = useState(0);
  const [applied, setApplied] = useState<Set<string>>(new Set());
  const [applyNote, setApplyNote] = useState<string | null>(null);

  /* Chat state */
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const turnId = useRef(0);
  const threadRef = useRef<HTMLDivElement>(null);
  const autoRunDone = useRef(false);

  const findings = useMemo(() => [...instant, ...aiFindings], [instant, aiFindings]);
  const current = findings[Math.min(index, Math.max(0, findings.length - 1))];
  // Severity tallies describe what's LEFT, matching the badges. Counting
  // applied findings would leave "2 blockers" on screen after both are fixed.
  const counts = useMemo(
    () => countBySeverity(findings.filter((f) => !applied.has(f.id))),
    [findings, applied],
  );

  // Counts what is still OUTSTANDING, not what was found — an applied finding
  // shouldn't keep nagging. Same number in the panel header and the menu
  // badge; two different counts for one review reads as a bug.
  const outstanding = useMemo(
    () => findings.filter((f) => !applied.has(f.id)).length,
    [findings, applied],
  );

  useEffect(() => {
    if (aiState === "idle") return;
    onReviewCount?.(outstanding);
  }, [outstanding, aiState, onReviewCount]);

  /* ── Review: the two waves ────────────────────────────────── */

  const runReview = useCallback(async () => {
    const draft = getDraft();
    // Wave 1 is synchronous and free — it must be on screen before the second
    // wave is even requested, or Validate feels like it does nothing for a
    // few seconds.
    setInstant(sortFindings(validateDraft(draft)));
    setIndex(0);
    setApplied(new Set());
    setApplyNote(null);

    if (!accountId || !draft.bodyText.trim()) {
      setAiFindings([]);
      setAiState("done");
      return;
    }

    setAiState("loading");
    setAiError(null);
    setAiFindings([]);
    try {
      const raw = await api.ai.validateDraft({
        accountId,
        recipientEmail: recipientEmail ?? null,
        subject: draft.subject,
        bodyText: draft.bodyText,
        replyFolder: replyContext?.folder ?? null,
        replyUid: replyContext?.uid ?? null,
      });
      setAiFindings(sortFindings(raw.map(fromAiFinding)));
      setAiState("done");
    } catch (e) {
      // The instant findings stay on screen — a failed AI pass degrades the
      // review, it doesn't cancel it.
      setAiState("error");
      setAiError(String(e));
      void api.system.logClientError("ai-validate", String(e));
    }
  }, [accountId, getDraft, recipientEmail, replyContext]);

  useEffect(() => {
    if (!open) return;
    setMode(launch.mode);
    if (launch.mode === "review") void runReview();
    // Intentionally keyed on `open` alone: re-running on every getDraft
    // identity change would re-validate (and re-bill) on every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open) {
      autoRunDone.current = false;
      setTurns([]);
      setInput("");
      setChatError(null);
    }
  }, [open]);

  /* ── Chat ─────────────────────────────────────────────────── */

  const pushTurn = (role: ChatTurn["role"], text: string, applicable = false) => {
    turnId.current += 1;
    setTurns((t) => [...t, { id: turnId.current, role, text, applicable }]);
  };

  const generate = useCallback(
    async (instruction: string, kind: "draft" | "reply" | "rewrite") => {
      setBusy(true);
      setChatError(null);
      try {
        let result: string;
        if (kind === "reply" && replyContext) {
          result = await api.ai.generateReply(
            replyContext.accountId,
            replyContext.folder,
            replyContext.uid,
            instruction,
          );
        } else if (kind === "draft" && accountId && recipientEmail) {
          result = await api.ai.generateCompose(
            accountId,
            recipientEmail,
            subject || null,
            instruction || null,
          );
        } else {
          // Rewrite works on the selection when there was one, else the whole
          // body — the same rule the old inline menu used.
          const base = selection?.text?.trim() || editor.getText();
          if (!base.trim()) throw new Error("There is nothing to rewrite yet.");
          result = await api.ai.rewriteText(base, instruction);
        }
        onAIDraftGenerated?.(result);
        pushTurn("assistant", result, true);
      } catch (e) {
        setChatError(String(e));
        void api.system.logClientError("ai-assist-chat", String(e));
      } finally {
        setBusy(false);
      }
    },
    [accountId, editor, onAIDraftGenerated, recipientEmail, replyContext, selection, subject],
  );

  // Auto-run the launch instruction (e.g. "Draft Reply" with no context).
  useEffect(() => {
    if (!open || mode !== "chat" || autoRunDone.current) return;
    const c = launch.chat;
    if (!c?.autoRun) return;
    autoRunDone.current = true;
    if (c.instruction) pushTurn("user", c.instruction);
    void generate(c.instruction, c.kind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, mode, launch]);

  useEffect(() => {
    threadRef.current?.scrollTo({ top: threadRef.current.scrollHeight });
  }, [turns, busy]);

  const send = () => {
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    pushTurn("user", text);
    void generate(text, launch.chat?.kind === "reply" ? "reply" : "rewrite");
  };

  /* ── Applying ─────────────────────────────────────────────── */

  const applyResult = (text: string, replaceAll: boolean) => {
    const html = plainToHtmlParagraphs(text);
    if (!replaceAll && selection && selection.from !== selection.to) {
      editor
        .chain()
        .focus()
        .insertContentAt({ from: selection.from, to: selection.to }, html)
        .run();
    } else {
      editor.chain().focus().insertContentAt(bodyRange(editor), html).run();
    }
  };

  const applyFinding = (f: Finding) => {
    const a = f.action;
    if (!a) return;
    if (a.kind === "ask") {
      setMode("chat");
      setInput(a.prompt);
      return;
    }
    let ok = true;
    if (a.kind === "replace") ok = replaceText(editor, a.find, a.with);
    else if (a.kind === "append") appendParagraph(editor, a.text);
    else if (a.kind === "subject") onSubjectChange(a.with);

    if (!ok) {
      // The draft changed under the review. Saying so beats a button that
      // looks like it worked.
      setApplyNote("Couldn't find that text any more — re-run the review.");
      return;
    }
    setApplyNote(null);
    setApplied((prev) => new Set(prev).add(f.id));
    const next = findings.findIndex((x, i) => i > index && !applied.has(x.id));
    if (next !== -1) setIndex(next);
  };

  const locate = (f: Finding) => {
    if (f.quote) locateText(editor, f.quote);
  };

  /* ── Dragging ─────────────────────────────────────────────── */

  const dragRef = useRef<{ dx: number; dy: number } | null>(null);
  const onDragStart = (e: React.MouseEvent) => {
    dragRef.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
    const move = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      setPos({
        x: Math.max(0, Math.min(window.innerWidth - 120, ev.clientX - dragRef.current.dx)),
        y: Math.max(0, Math.min(window.innerHeight - 60, ev.clientY - dragRef.current.dy)),
      });
    };
    const up = () => {
      dragRef.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div
      className="fixed z-[60] flex flex-col overflow-hidden rounded-xl border border-border bg-base-solid shadow-macos-lg"
      style={{ left: pos.x, top: pos.y, width: size.w, height: size.h }}
      role="dialog"
      aria-label="AI Assist"
    >
      {/* Header */}
      <div
        onMouseDown={onDragStart}
        className="flex cursor-grab items-center gap-2 border-b border-border-subtle px-3 py-2.5 active:cursor-grabbing"
      >
        <GripVertical className="h-4 w-4 shrink-0 text-content-faint" />
        <div className="flex items-center gap-0.5 rounded-lg bg-surface p-0.5">
          <button
            onClick={() => {
              setMode("review");
              if (aiState === "idle" && instant.length === 0) void runReview();
            }}
            className={cn(
              "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              mode === "review" ? "bg-base-solid text-content shadow-macos" : "text-content-muted",
            )}
          >
            <ShieldCheck className="h-3.5 w-3.5" />
            Review
            {outstanding > 0 && (
              <span
                className={cn(
                  "rounded-full px-1.5 text-[10px] font-bold",
                  counts.error > 0
                    ? "bg-error/20 text-error"
                    : "bg-warning/20 text-warning",
                )}
              >
                {outstanding}
              </span>
            )}
          </button>
          <button
            onClick={() => setMode("chat")}
            className={cn(
              "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              mode === "chat" ? "bg-base-solid text-content shadow-macos" : "text-content-muted",
            )}
          >
            <MessageSquare className="h-3.5 w-3.5" />
            Chat
          </button>
        </div>
        {mode === "review" && (
          <button
            onClick={() => void runReview()}
            disabled={aiState === "loading"}
            title="Re-run the review"
            className="ml-auto text-content-faint hover:text-content disabled:opacity-40"
          >
            <RotateCw className={cn("h-3.5 w-3.5", aiState === "loading" && "animate-spin")} />
          </button>
        )}
        <button
          onClick={onClose}
          aria-label="Close AI Assist"
          className={cn("shrink-0 text-content-muted hover:text-content", mode !== "review" && "ml-auto")}
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {mode === "review" ? (
        <ReviewMode
          findings={findings}
          current={current}
          index={index}
          setIndex={setIndex}
          counts={counts}
          aiState={aiState}
          aiError={aiError}
          applied={applied}
          applyNote={applyNote}
          outstanding={outstanding}
          onApply={applyFinding}
          onLocate={locate}
          onRerun={() => void runReview()}
        />
      ) : (
        <ChatModeView
          turns={turns}
          busy={busy}
          error={chatError}
          input={input}
          setInput={setInput}
          onSend={send}
          onApply={applyResult}
          hasSelection={!!selection && selection.from !== selection.to}
          threadRef={threadRef}
        />
      )}

      {/* Resize grip */}
      <div
        onMouseDown={(e) => {
          e.preventDefault();
          const start = { x: e.clientX, y: e.clientY, w: size.w, h: size.h };
          const move = (ev: MouseEvent) => {
            setSize({
              w: Math.max(340, start.w + (ev.clientX - start.x)),
              h: Math.max(280, start.h + (ev.clientY - start.y)),
            });
          };
          const up = () => {
            window.removeEventListener("mousemove", move);
            window.removeEventListener("mouseup", up);
          };
          window.addEventListener("mousemove", move);
          window.addEventListener("mouseup", up);
        }}
        className="absolute bottom-0 right-0 h-3.5 w-3.5 cursor-nwse-resize"
      />
    </div>,
    document.body,
  );
}

/** Whole-body range, stopping before the signature. */
function bodyRange(editor: Editor) {
  const from = 0;
  let to = editor.state.doc.content.size;
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name === "signatureBlock" || node.type.name === "quotedBlock") {
      to = Math.min(to, pos);
      return false;
    }
    return true;
  });
  return { from, to };
}

/* ────────────────────────────────────────────────────────────
   Review
   ──────────────────────────────────────────────────────────── */

function ReviewMode({
  findings,
  current,
  index,
  setIndex,
  counts,
  aiState,
  aiError,
  applied,
  applyNote,
  outstanding,
  onApply,
  onLocate,
  onRerun,
}: {
  findings: Finding[];
  current: Finding | undefined;
  index: number;
  setIndex: (i: number) => void;
  counts: Record<FindingSeverity, number>;
  aiState: "idle" | "loading" | "done" | "error";
  aiError: string | null;
  applied: Set<string>;
  applyNote: string | null;
  outstanding: number;
  onApply: (f: Finding) => void;
  onLocate: (f: Finding) => void;
  onRerun: () => void;
}) {
  if (findings.length === 0 && aiState !== "loading") {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
        <div className="rounded-full bg-success/15 p-2.5">
          <Check className="h-5 w-5 text-success" />
        </div>
        <div className="text-sm font-medium text-content">Nothing to flag</div>
        <p className="max-w-[240px] text-xs leading-relaxed text-content-muted">
          {aiState === "error"
            ? "The mechanical checks passed. The AI review couldn't run — see the note below."
            : "The draft passed every check."}
        </p>
        {aiState === "error" && aiError && (
          <p className="max-w-[260px] truncate text-[11px] text-error" title={aiError}>
            {aiError}
          </p>
        )}
        <button
          onClick={onRerun}
          className="mt-1 rounded-md border border-border px-2.5 py-1 text-xs text-content-secondary hover:bg-surface"
        >
          Re-run
        </button>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Nav A — chip strip */}
      <div className="border-b border-border-subtle px-2.5 py-2">
        <div className="mb-1.5 flex items-center gap-2 px-0.5">
          <span className="text-[10px] font-bold uppercase tracking-[0.08em] text-content-faint">
            {outstanding} flagged
          </span>
          {counts.error > 0 && (
            <span className="text-[10px] text-error">{counts.error} blocker</span>
          )}
          {counts.warning > 0 && (
            <span className="text-[10px] text-warning">{counts.warning} checks</span>
          )}
          {aiState === "loading" && (
            <span className="ml-auto flex items-center gap-1 text-[10px] text-ai">
              <Loader2 className="h-3 w-3 animate-spin" />
              AI review…
            </span>
          )}
          {/* A failed second wave must SAY so. Otherwise the instant findings
              alone look like a complete review, and the checks that need the
              model — pinned rules, unanswered questions — appear to have
              passed when they never ran. */}
          {aiState === "error" && (
            <span
              className="ml-auto flex items-center gap-1 text-[10px] text-error"
              title={aiError ?? undefined}
            >
              <AlertTriangle className="h-3 w-3" />
              AI review failed
            </span>
          )}
        </div>
        <div className="flex flex-wrap gap-1">
          {findings.map((f, i) => {
            const s = SEVERITY[f.severity];
            const active = i === index;
            const done = applied.has(f.id);
            return (
              <button
                key={f.id}
                onClick={() => {
                  setIndex(i);
                  onLocate(f);
                }}
                title={f.title}
                className={cn(
                  "flex items-center gap-1.5 rounded-full border px-2 py-1 text-[11px] transition-colors",
                  active
                    ? cn(s.bg, "border-transparent text-content ring-1", s.ring)
                    : "border-border-subtle text-content-muted hover:bg-surface",
                  done && "opacity-50",
                )}
              >
                {done ? (
                  <Check className="h-2.5 w-2.5 text-success" />
                ) : (
                  <span className={cn("h-1.5 w-1.5 rounded-full", s.dot)} />
                )}
                <span className="font-medium">{f.where}</span>
                <span className={active ? "text-content-secondary" : "text-content-faint"}>
                  {f.short}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Detail */}
      {current && (
        <FindingDetail
          f={current}
          applied={applied.has(current.id)}
          note={applyNote}
          onApply={() => onApply(current)}
          onLocate={() => onLocate(current)}
        />
      )}
    </div>
  );
}

function FindingDetail({
  f,
  applied,
  note,
  onApply,
  onLocate,
}: {
  f: Finding;
  applied: boolean;
  note: string | null;
  onApply: () => void;
  onLocate: () => void;
}) {
  const s = SEVERITY[f.severity];
  const Icon = s.icon;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto p-4">
        <div className="flex flex-wrap items-center gap-1.5">
          <div className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5", s.bg)}>
            <Icon className={cn("h-3 w-3", s.text)} />
            <span className={cn("text-[10px] font-bold uppercase tracking-wide", s.text)}>
              {s.label}
            </span>
          </div>
          <span className="rounded-full bg-surface px-2 py-0.5 text-[10px] font-medium text-content-muted">
            {f.where}
          </span>
          {f.badge && (
            <span className="inline-flex items-center gap-1 rounded-full bg-ai-surface px-2 py-0.5 text-[10px] font-semibold text-ai">
              <Pin className="h-2.5 w-2.5" />
              {f.badge}
            </span>
          )}
          {f.source === "instant" && (
            <span className="text-[10px] text-content-faint">instant</span>
          )}
        </div>

        <h3 className="mt-3 text-sm font-medium leading-snug text-content">{f.title}</h3>
        <p className="mt-2 text-[13px] leading-relaxed text-content-secondary">{f.detail}</p>

        {f.quote && (
          <div className="mt-4">
            <div className="mb-1 flex items-center gap-2">
              <span className="text-[10px] font-bold uppercase tracking-[0.08em] text-content-faint">
                In your draft
              </span>
              <button
                onClick={onLocate}
                className="text-[10px] text-accent hover:underline"
              >
                show me
              </button>
            </div>
            <div className={cn("rounded-lg px-3 py-2 text-[13px] text-content-secondary", s.bg)}>
              {f.quote}
            </div>
          </div>
        )}

        {/* Shown for every action kind, including `ask` — a finding we can't
            apply mechanically still has the most useful sentence in it. */}
        {f.suggestion && (
          <Suggested
            text={f.suggestion}
            label={
              f.action?.kind === "subject"
                ? "Suggested subject"
                : f.action?.kind === "append"
                  ? "Suggested addition"
                  : "Suggested"
            }
          />
        )}

        {note && <p className="mt-3 text-xs text-error">{note}</p>}
      </div>

      <div className="flex items-center gap-1.5 border-t border-border-subtle p-2.5">
        {f.action ? (
          <button
            onClick={onApply}
            disabled={applied}
            className={cn(
              "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium",
              applied
                ? "bg-success/15 text-success"
                : "bg-accent text-white hover:bg-accent-hover",
            )}
          >
            {applied ? <Check className="h-3.5 w-3.5" /> : null}
            {applied ? "Applied" : f.action.label}
          </button>
        ) : (
          <span className="px-1 text-xs text-content-faint">Needs a manual fix</span>
        )}
      </div>
    </div>
  );
}

function Suggested({ text, label = "Suggested" }: { text: string; label?: string }) {
  return (
    <div className="mt-3">
      <div className="mb-1 text-[10px] font-bold uppercase tracking-[0.08em] text-content-faint">
        {label}
      </div>
      <div className="rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-[13px] text-content-secondary">
        {text}
      </div>
    </div>
  );
}

/* ────────────────────────────────────────────────────────────
   Chat
   ──────────────────────────────────────────────────────────── */

function ChatModeView({
  turns,
  busy,
  error,
  input,
  setInput,
  onSend,
  onApply,
  hasSelection,
  threadRef,
}: {
  turns: ChatTurn[];
  busy: boolean;
  error: string | null;
  input: string;
  setInput: (s: string) => void;
  onSend: () => void;
  onApply: (text: string, replaceAll: boolean) => void;
  hasSelection: boolean;
  threadRef: React.RefObject<HTMLDivElement | null>;
}) {
  const [appliedTurn, setAppliedTurn] = useState<number | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Grow to fit. A seeded prompt handed over from a finding is a full
  // sentence or two, and a fixed single row would show the user a truncated
  // instruction they can't read before sending.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 96)}px`;
  }, [input]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div ref={threadRef} className="min-h-0 flex-1 space-y-3 overflow-auto p-3">
        {turns.length === 0 && !busy && (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
            <Sparkles className="h-5 w-5 text-ai" />
            <p className="max-w-[240px] text-xs leading-relaxed text-content-muted">
              Describe the change you want. {hasSelection ? "Your selection is the target." : "The whole draft is the target."}
            </p>
          </div>
        )}

        {turns.map((t) =>
          t.role === "user" ? (
            <div key={t.id} className="flex justify-end">
              <div className="max-w-[85%] whitespace-pre-wrap rounded-lg rounded-br-sm bg-elevated px-3 py-2 text-sm text-content">
                {t.text}
              </div>
            </div>
          ) : (
            <div key={t.id} className="rounded-lg border border-border bg-surface p-3">
              <div className="whitespace-pre-wrap text-sm leading-relaxed text-content-secondary">
                {t.text}
              </div>
              <div className="mt-3 flex items-center gap-1.5 border-t border-border-subtle pt-2.5">
                <button
                  onClick={() => {
                    onApply(t.text, false);
                    setAppliedTurn(t.id);
                  }}
                  className="rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-white hover:bg-accent-hover"
                >
                  {hasSelection ? "Replace selection" : "Replace draft"}
                </button>
                {hasSelection && (
                  <button
                    onClick={() => {
                      onApply(t.text, true);
                      setAppliedTurn(t.id);
                    }}
                    className="rounded-md border border-border px-2.5 py-1 text-xs text-content-secondary hover:bg-elevated"
                  >
                    Replace whole draft
                  </button>
                )}
                {appliedTurn === t.id && (
                  <span className="ml-auto flex items-center gap-1 text-xs text-success">
                    <Check className="h-3 w-3" />
                    Applied
                  </span>
                )}
              </div>
            </div>
          ),
        )}

        {busy && (
          <div className="flex items-center gap-2 text-sm text-content-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin text-ai" />
            Working…
          </div>
        )}
        {error && (
          <div className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">
            {error}
          </div>
        )}
      </div>

      <div className="border-t border-border-subtle p-2.5">
        <div className="flex items-end gap-2 rounded-lg border border-border bg-input px-2.5 py-2 focus-within:border-accent">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                onSend();
              }
            }}
            rows={1}
            placeholder="Describe the change…"
            spellCheck
            className="max-h-24 flex-1 resize-none bg-transparent py-0.5 text-sm text-content outline-none placeholder:text-content-faint"
          />
          <button
            onClick={onSend}
            disabled={!input.trim() || busy}
            aria-label="Send"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-accent text-white disabled:opacity-40"
          >
            <ArrowUp className="h-4 w-4" />
          </button>
        </div>
        <div className="mt-1.5 flex items-center gap-1 px-1 text-[10px] text-content-faint">
          <CornerDownLeft className="h-3 w-3" />
          <span>⌘↵ to send · ↵ for a new line</span>
        </div>
      </div>
    </div>
  );
}
