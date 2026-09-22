/**
 * Editor operations behind the AI Assist panel's Apply buttons.
 *
 * Split out of the panel so the document-level logic is testable without a
 * browser: TipTap builds and mutates a real ProseMirror doc under jsdom
 * perfectly well. What jsdom CANNOT model is focus and selection (gotcha #33),
 * so nothing here depends on either — every function addresses the document by
 * position, and the panel does focusing separately.
 */
import type { Editor } from "@tiptap/react";

export interface TextRange {
  from: number;
  to: number;
}

/** Node types that mark the end of the user's own prose. */
const TRAILING_BLOCKS = new Set(["signatureBlock", "quotedBlock"]);

/**
 * The position just before the signature / quoted history, i.e. the end of the
 * body the user actually wrote.
 *
 * Appending at `doc.content.size` would land AFTER the signature, which reads
 * as text accidentally typed below the sign-off.
 */
export function bodyEndPos(editor: Editor): number {
  const { doc } = editor.state;
  let end = doc.content.size;
  doc.descendants((node, pos) => {
    if (TRAILING_BLOCKS.has(node.type.name)) {
      end = Math.min(end, pos);
      return false;
    }
    return true;
  });
  return end;
}

function normalize(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

/**
 * Locate `needle` in the document.
 *
 * Two passes, because a quote is not guaranteed to sit inside one text node —
 * any bold or link mark inside it splits the run.
 *
 * 1. **Exact, within a single text node.** Gives a tight range covering just
 *    the phrase, which is what makes an Apply feel surgical.
 * 2. **Whitespace-normalized, per block.** Catches a quote split across marks
 *    or wrapped differently from how the model echoed it, at the cost of
 *    resolving to the whole block.
 *
 * Returns null rather than guessing when neither finds it — the caller reports
 * that honestly instead of silently editing the wrong text.
 */
export function findTextRange(editor: Editor, needle: string): TextRange | null {
  const target = needle.trim();
  if (!target) return null;
  const { doc } = editor.state;

  let hit: TextRange | null = null;

  doc.descendants((node, pos) => {
    if (hit) return false;
    if (node.isText && node.text) {
      const i = node.text.indexOf(target);
      if (i !== -1) {
        hit = { from: pos + i, to: pos + i + target.length };
        return false;
      }
    }
    return true;
  });
  if (hit) return hit;

  const wanted = normalize(target);
  if (!wanted) return null;

  doc.descendants((node, pos) => {
    if (hit) return false;
    // Leaf blocks only — a match on an ancestor would select the whole doc.
    if (node.isTextblock && normalize(node.textContent).includes(wanted)) {
      hit = { from: pos + 1, to: pos + 1 + node.content.size };
      return false;
    }
    return true;
  });

  return hit;
}

/**
 * Replace the first occurrence of `needle` with `replacement`.
 *
 * Returns false when the text can no longer be found — the user may have
 * edited it since the review ran, and an Apply that quietly does nothing is
 * the failure mode this return value exists to make visible.
 */
export function replaceText(editor: Editor, needle: string, replacement: string): boolean {
  const range = findTextRange(editor, needle);
  if (!range) return false;
  editor
    .chain()
    .focus()
    .insertContentAt({ from: range.from, to: range.to }, replacement)
    .run();
  return true;
}

/** Add a paragraph at the end of the body, before any signature. */
export function appendParagraph(editor: Editor, text: string): void {
  const at = bodyEndPos(editor);
  editor
    .chain()
    .focus()
    .insertContentAt(at, { type: "paragraph", content: [{ type: "text", text }] })
    .run();
}

/**
 * Select `needle` and scroll it into view — how a chip click answers "where in
 * my draft is this?".
 *
 * Selection rather than a ProseMirror decoration plugin: a decoration set would
 * have to be remapped through every transaction to stay anchored while the user
 * types, and re-running this cheap search on click is both simpler and always
 * current.
 */
export function locateText(editor: Editor, needle: string): boolean {
  const range = findTextRange(editor, needle);
  if (!range) return false;
  editor.chain().focus().setTextSelection(range).scrollIntoView().run();
  return true;
}
