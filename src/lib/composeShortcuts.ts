import { Extension } from "@tiptap/core";

/**
 * Formatting shortcuts people bring from other mail clients, layered on top of
 * the ones StarterKit already binds (⌘B/⌘I/⌘U, ⇧⌘S strike, ⇧⌘7/⇧⌘8 lists,
 * ⇧⌘B quote, ⌘Z/⇧⌘Z). ⌘K (link) is not here: it has to open React UI and work
 * inside a styled HTML block, which ProseMirror never sees keys from — the
 * compose editor's wrapper handles it (ComposeModal).
 *
 * Mirrored for styled HTML blocks by `nativeCommandForShortcut` in
 * composeNodes.ts; change the two together.
 */
export const ComposeShortcuts = Extension.create({
  name: "composeShortcuts",
  addKeyboardShortcuts() {
    return {
      // Gmail's strikethrough.
      "Mod-Shift-x": () => this.editor.commands.toggleStrike(),
      // Gmail's quote.
      "Mod-Shift-9": () => this.editor.commands.toggleBlockquote(),
      // Gmail's "remove formatting".
      "Mod-\\": () => this.editor.chain().focus().unsetAllMarks().run(),
    };
  },
});

/** ⌘K / Ctrl+K with no other modifier — the universal "insert link". */
export function isLinkShortcut(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): boolean {
  return (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k";
}
