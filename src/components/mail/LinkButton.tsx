import { useEffect, useRef, useState } from "react";
import type { Editor } from "@tiptap/react";
import { Link as LinkIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { captureCaret, isEditingHtmlBlock } from "@/lib/composeNodes";
import { normalizeLinkHref } from "@/lib/composeLink";

/**
 * The compose toolbar's link control.
 *
 * It is an inline field and not `window.prompt` on purpose: wry does not
 * implement WKUIDelegate's text-input panel, so in the Tauri webview
 * `window.prompt` returns null immediately and the button did nothing at all.
 */
export default function LinkButton({ editor, openSignal = 0 }: { editor: Editor; openSignal?: number }) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const [invalid, setInvalid] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // Focusing the field takes the caret out of the editor. Inside a styled-HTML
  // block the caret IS the selection (composeNodes.ts), so it is captured at
  // open time and put back before the native command runs.
  const inBlockRef = useRef(false);
  const restoreRef = useRef<(() => void) | null>(null);

  const linkActive = editor.isActive("link");

  const close = () => {
    setOpen(false);
    setInvalid(false);
  };

  const openField = () => {
    inBlockRef.current = isEditingHtmlBlock();
    restoreRef.current = inBlockRef.current ? captureCaret() : null;
    const existing = inBlockRef.current ? "" : (editor.getAttributes("link").href as string | undefined) ?? "";
    setValue(existing);
    setInvalid(false);
    setOpen(true);
  };

  // ⌘K in the editor. Skips the initial value so mounting never opens it.
  const seenSignal = useRef(openSignal);
  useEffect(() => {
    if (openSignal === seenSignal.current) return;
    seenSignal.current = openSignal;
    openField();
    // openField reads the live selection; it is not a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openSignal]);

  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
    inputRef.current?.select();
    // Capture phase: a drag region swallows bubbling mousedowns (gotcha #51).
    const onDown = (e: MouseEvent) => {
      if (!wrapperRef.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown, true);
    return () => document.removeEventListener("mousedown", onDown, true);
  }, [open]);

  const removeLink = () => {
    if (inBlockRef.current) {
      restoreRef.current?.();
      try {
        document.execCommand("unlink");
      } catch (e) {
        console.error("Native unlink failed inside styled block:", e);
      }
    } else {
      editor.chain().focus().extendMarkRange("link").unsetLink().run();
    }
    close();
  };

  const apply = () => {
    if (!value.trim()) {
      if (linkActive) removeLink();
      else close();
      return;
    }
    const href = normalizeLinkHref(value);
    if (!href) {
      setInvalid(true);
      return;
    }

    if (inBlockRef.current) {
      restoreRef.current?.();
      try {
        // With a collapsed caret WebKit inserts the URL itself as the link text.
        document.execCommand("createLink", false, href);
      } catch (e) {
        console.error("Native createLink failed inside styled block:", e);
      }
    } else if (editor.state.selection.empty && !linkActive) {
      // setLink on an empty selection marks nothing, so there would be no link
      // to see. Insert the address as the link text instead.
      editor
        .chain()
        .focus()
        .insertContent({ type: "text", text: value.trim(), marks: [{ type: "link", attrs: { href } }] })
        .unsetMark("link")
        .run();
    } else {
      editor.chain().focus().extendMarkRange("link").setLink({ href }).run();
    }
    close();
  };

  return (
    <div ref={wrapperRef} className="relative">
      <button
        type="button"
        aria-label="Insert link"
        // Keep the caret where it is until the field takes focus deliberately.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => (open ? close() : openField())}
        className={cn(
          "rounded p-1.5 transition-colors",
          linkActive || open ? "bg-elevated text-content" : "text-content-muted hover:text-content"
        )}
      >
        <LinkIcon className="h-4 w-4" />
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Link"
          className="absolute left-0 top-full z-50 mt-1 flex w-80 items-center gap-1 rounded-md border border-border bg-elevated p-1.5 shadow-lg"
        >
          <input
            ref={inputRef}
            type="text"
            aria-label="Link URL"
            placeholder="https://example.com"
            value={value}
            spellCheck={false}
            onChange={(e) => {
              setValue(e.target.value);
              setInvalid(false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                apply();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                close();
                if (inBlockRef.current) restoreRef.current?.();
                else editor.commands.focus();
              }
            }}
            className={cn(
              "min-w-0 flex-1 rounded border bg-input px-2 py-1 text-xs text-content outline-none",
              invalid ? "border-error" : "border-border-subtle focus:border-accent"
            )}
          />
          <button
            type="button"
            onClick={apply}
            className="rounded bg-accent px-2 py-1 text-xs font-medium text-white hover:bg-accent-hover"
          >
            Apply
          </button>
          {linkActive && !inBlockRef.current && (
            <button
              type="button"
              onClick={removeLink}
              className="rounded px-2 py-1 text-xs text-content-muted hover:text-content"
            >
              Remove
            </button>
          )}
        </div>
      )}
    </div>
  );
}
