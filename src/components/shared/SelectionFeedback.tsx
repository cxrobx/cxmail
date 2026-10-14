import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Editor } from "@tiptap/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ArrowUp, Loader2, X } from "lucide-react";
import { api } from "@/lib/tauri";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";
import { useUIStore } from "@/stores/uiStore";
import { cn } from "@/lib/utils";
import {
  captureEditableSelection, feedbackPosition, HUMANIZE_INSTRUCTION,
  replaceFeedbackSelection, SOFTEN_TONE,
  type EditableFeedbackSelection, type FeedbackSelection,
} from "@/lib/selectionFeedback";

type Props = {
  selection: FeedbackSelection;
  onClose: () => void;
  /** Freeze the source range once the user interacts, without stealing focus on selection. */
  onInteract?: () => void;
} & ({
  mode: "editable";
  onApply: (text: string) => boolean;
} | {
  mode: "readonly";
  onAsk: (quote: string, note: string) => Promise<void>;
  onUseInReply: (text: string) => void;
  /** True when sending would end a chat the user is in the middle of. */
  wouldReplaceChat?: () => boolean;
});

/** Portalled because fixed elements inside floating/transformed windows move (#37). */
export default function SelectionFeedback(props: Props) {
  const { selection, onClose } = props;
  const theme = useResolvedTheme();
  const preference = useUIStore((state) => state.theme);
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [stale, setStale] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Opens as one compact row; grows when the user reaches for the text field
  // or there is something to show, so a selection made only to copy or delete
  // covers as little of the text around it as possible.
  const [focused, setFocused] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [size, setSize] = useState({ width: 360, height: 120 });
  const root = useRef<HTMLDivElement>(null);
  const active = useRef(true);
  const inFlight = useRef(false);

  useEffect(() => {
    active.current = true;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) onClose();
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onClose();
      }
    };
    const frameEvent = (event: MessageEvent) => {
      if (!["cxmail-iframe-pointerdown", "cxmail-iframe-wheel", "cxmail-iframe-escape"].includes(event.data?.type)) return;
      if (Array.from(document.querySelectorAll("iframe")).some((frame) => frame.contentWindow === event.source)) onClose();
    };
    // Focus moving into an email iframe also emits parent blur; do not use it
    // as dismissal. Its pointer relay handles the click reliably instead.
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", keydown, true);
    window.addEventListener("message", frameEvent);
    window.addEventListener("resize", onClose);
    const scroll = (event: Event) => {
      if (!root.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("scroll", scroll, true);
    return () => {
      active.current = false;
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", keydown, true);
      window.removeEventListener("message", frameEvent);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [onClose]);

  useLayoutEffect(() => {
    const measure = () => {
      const rect = root.current?.getBoundingClientRect();
      if (rect) setSize({ width: rect.width, height: rect.height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (root.current) observer?.observe(root.current);
    return () => observer?.disconnect();
  }, [result, error, busy, focused, confirming]);

  async function run(action: "shorten" | "soften" | "humanize" | "custom", replaceConfirmed = false) {
    const note = instruction.trim();
    if (inFlight.current || (action === "custom" && !note)) return;
    props.onInteract?.();
    if (action === "custom" && props.mode === "readonly" && !replaceConfirmed && props.wouldReplaceChat?.()) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    inFlight.current = true;
    setBusy(true);
    setError(null);
    setCopied(false);
    try {
      if (action === "custom" && props.mode === "readonly") {
        await props.onAsk(selection.text, note);
        if (active.current) onClose();
        return;
      }
      const rewritten = action === "shorten" || action === "soften"
        ? await api.ai.adjustTone(selection.text, action === "shorten" ? "concise" : SOFTEN_TONE)
        : await api.ai.rewriteText(selection.text, action === "humanize" ? HUMANIZE_INSTRUCTION : note);
      if (!active.current) return;
      if (!rewritten.trim()) throw new Error("Claude returned no text. Try again.");
      if (props.mode === "editable") {
        if (props.onApply(rewritten)) { onClose(); return; }
        setStale(true);
      }
      setResult(rewritten);
    } catch (e) {
      if (active.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      inFlight.current = false;
      if (active.current) setBusy(false);
    }
  }

  const position = feedbackPosition(selection.anchor, size.width, size.height);
  const keepSelection = (event: React.MouseEvent) => {
    props.onInteract?.();
    event.preventDefault();
  };
  const expanded = focused || busy || confirming || error !== null || result !== null;
  // Free text on someone's email goes to the Claude chat; everything else runs
  // on the configured AI provider, which need not be Claude.
  const asksClaude = props.mode === "readonly";
  const fieldLabel = asksClaude ? "Ask Claude about this passage" : "Describe the change";
  const placeholder = expanded
    ? (asksClaude ? "Ask Claude about this passage…" : "Change it how? e.g. more direct")
    : (asksClaude ? "Ask Claude…" : "Change…");
  const chipClass = "rounded-full border border-border-subtle bg-surface px-2.5 py-0.5 text-xs text-content-secondary hover:bg-elevated hover:text-content disabled:opacity-50";

  return createPortal(
    <div ref={root} style={position} data-theme={preference === "vault" ? "vault" : theme}
      className="fixed z-[10000] max-w-[calc(100vw-16px)] text-content"
      onFocusCapture={() => props.onInteract?.()}
      onPointerDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}>
        <div role="dialog" aria-label="Selection feedback"
          className={cn(
            "max-w-[calc(100vw-16px)] max-h-[calc(100vh-16px)] overflow-auto border border-border-subtle bg-base-solid shadow-2xl",
            expanded ? "w-[360px] rounded-xl p-3" : "rounded-full py-1 pl-1.5 pr-1",
          )}>
          <form onSubmit={(event) => { event.preventDefault(); void run("custom"); }}
            className={cn("flex gap-1.5", expanded ? "flex-col" : "items-center")}>
            <div className="flex items-center gap-1">
              {(["shorten", "soften", "humanize"] as const).map((action) => (
                <button key={action} type="button" onMouseDown={keepSelection} disabled={busy}
                  onClick={() => void run(action)} className={chipClass}>
                  {action[0].toUpperCase() + action.slice(1)}
                </button>
              ))}
              {expanded && (
                <button type="button" aria-label="Close feedback" onMouseDown={keepSelection} onClick={onClose}
                  className="ml-auto rounded p-1 text-content-muted hover:text-content"><X className="h-3.5 w-3.5" /></button>
              )}
            </div>
            <div className={cn("flex items-end gap-1", expanded
              ? "rounded-lg border border-border-subtle bg-surface p-2"
              : "rounded-full bg-surface py-0.5 pl-2.5 pr-0.5")}>
              <textarea aria-label={fieldLabel} placeholder={placeholder}
                rows={expanded ? 2 : 1} value={instruction}
                onFocus={() => setFocused(true)}
                onChange={(event) => { setInstruction(event.target.value); setConfirming(false); }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault(); void run("custom");
                  }
                }}
                className={cn("min-w-0 resize-none bg-transparent text-xs leading-5 text-content outline-none placeholder:text-content-muted",
                  expanded ? "flex-1" : "w-24")} />
              <button type="submit" disabled={busy || !instruction.trim()} aria-label="Send feedback"
                className="rounded-md p-1 text-accent hover:bg-elevated disabled:opacity-40"><ArrowUp className="h-3.5 w-3.5" /></button>
            </div>
          </form>
          {confirming && (
            <div role="alert" className="mt-2 text-xs text-content-secondary">
              <p>This starts a new chat about this email and ends your current one.</p>
              <div className="mt-2 flex gap-2">
                <button type="button" className={chipClass} onClick={() => void run("custom", true)}>Start new chat</button>
                <button type="button" className={chipClass} onClick={() => setConfirming(false)}>Cancel</button>
              </div>
            </div>
          )}
          {busy && <p role="status" className="mt-2 flex items-center gap-2 text-xs text-content-muted"><Loader2 className="h-3 w-3 animate-spin" />{props.mode === "readonly" ? "Working…" : "Rewriting…"}</p>}
          {error && <p role="alert" className="mt-2 text-xs text-red-400">{error}</p>}
          {result !== null && (
            <div className="mt-3 border-t border-border-subtle pt-2">
              {stale && <p role="status" className="mb-2 text-xs text-content-muted">Your draft changed. Copy this version to use it.</p>}
              <p className="max-h-52 overflow-auto whitespace-pre-wrap text-sm">{result}</p>
              <div className="mt-2 flex gap-2">
                <button type="button" disabled={busy} className={chipClass} onClick={async () => {
                  try { await writeText(result); if (active.current) setCopied(true); }
                  catch (e) { if (active.current) setError(String(e)); }
                }}>{copied ? "Copied" : "Copy"}</button>
                {props.mode === "readonly" && <button type="button" disabled={busy} className={chipClass}
                  onClick={() => { props.onUseInReply(result); onClose(); }}>Use in reply</button>}
              </div>
            </div>
          )}
        </div>
    </div>, document.body,
  );
}

/** The saved range survives focus moving to the feedback input. */
export function ComposerSelectionFeedback({ editor }: { editor: Editor }) {
  const [selection, setSelection] = useState<EditableFeedbackSelection | null>(null);
  const frozen = useRef(false);
  const close = useCallback(() => { frozen.current = false; setSelection(null); }, []);
  useEffect(() => {
    let dom: HTMLElement | null = null;
    const update = () => {
      if (!editor.isDestroyed && !frozen.current) setSelection(captureEditableSelection(editor));
    };
    const cleared = () => {
      if (!frozen.current && document.getSelection()?.isCollapsed) setSelection(null);
    };
    const keyup = (event: KeyboardEvent) => { if (event.key !== "Escape") update(); };
    const attach = () => {
      if (editor.isDestroyed) return;
      dom?.removeEventListener("mouseup", update);
      dom?.removeEventListener("keyup", keyup);
      dom = editor.view.dom;
      dom.addEventListener("mouseup", update);
      dom.addEventListener("keyup", keyup);
    };
    // useEditor may return before EditorContent has mounted its view. Keep
    // the DOM reference for cleanup too: the editor can unmount first.
    editor.on("create", attach);
    attach();
    document.addEventListener("selectionchange", cleared);
    return () => {
      editor.off("create", attach);
      dom?.removeEventListener("mouseup", update);
      dom?.removeEventListener("keyup", keyup);
      document.removeEventListener("selectionchange", cleared);
    };
  }, [editor]);
  return selection && <SelectionFeedback mode="editable" selection={selection} onClose={close}
    onInteract={() => { frozen.current = true; }}
    onApply={(text) => replaceFeedbackSelection(editor, selection, text)} />;
}
