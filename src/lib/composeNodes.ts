import { Node } from "@tiptap/core";

// All three nodes are atomic — they appear as a single block in the editor that
// the user can position around or delete entirely. The raw HTML is stashed in an
// `html` attribute and rendered verbatim via innerHTML.
//
// This is the only way to preserve arbitrary email HTML (tables, <span>/<font>
// styling, inline styles, <sup>/<sub>, class hooks) through the round-trip:
//   defaultBody → setContent → editor → getHTML → IMAP append → fetch → display
//
// Routing the inner HTML through TipTap's normal parse/serialize would silently
// drop anything its registered extensions don't recognize — which is most
// real-world email content.
//
// Signature and quoted blocks are sealed: the user can't edit inside them (Gmail
// and Apple Mail behave the same way). Customizing the signature is done in
// identity settings; replacing it per-message means deleting the block and
// retyping. HtmlBlock is the exception — see `editable` below.

export const HTML_BLOCK_CLASS = "cx-html-block";

// ─── Editable-block plumbing ────────────────────────────────────────────────
//
// Inside an editable block ProseMirror never sees the keystrokes: `stopEvent`
// and `ignoreMutation` hand the whole subtree to the browser's native
// contentEditable, which is exactly what preserves the surrounding inline styles
// while the user retypes a word. The cost is that the node's `html` attribute —
// the thing `getHTML()` serializes and the thing the plain-text serializer reads
// — goes stale the moment the user types.
//
// Each mounted editable block registers a flush here. Anything that serializes
// the editor must call `flushHtmlBlockEdits()` first, or it persists pre-edit
// HTML. `buildOutgoingEmail()` in ComposeModal is that choke point: autosave,
// send, schedule-send, and pop-out all route through it.
const pendingBlockFlushes = new Set<() => void>();

/** Push every pending in-block edit into the ProseMirror document, synchronously. */
export function flushHtmlBlockEdits(): void {
  // Iterate a copy: a flush dispatches a transaction, whose "update" listeners
  // can re-enter this function. Each flush is a no-op when nothing is dirty, so
  // the re-entrant pass terminates immediately.
  for (const flush of Array.from(pendingBlockFlushes)) flush();
}

/**
 * True when the caret currently sits inside an editable styled-HTML block.
 *
 * Formatting commands must branch on this: ProseMirror's selection is stale
 * while the user edits in a block (we stop the events that would update it), so
 * `editor.chain().focus().toggleBold()` would apply the mark at whatever
 * position ProseMirror last knew about — somewhere else in the draft.
 */
export function isEditingHtmlBlock(): boolean {
  if (typeof document === "undefined") return false;
  const sel = document.getSelection();
  const anchor = sel?.anchorNode;
  if (!anchor) return false;
  const el = anchor.nodeType === 1 ? (anchor as Element) : anchor.parentElement;
  return !!el?.closest(`div.${HTML_BLOCK_CLASS}`);
}

/**
 * Snapshot the caret so it can be put back after something steals focus — a
 * `window.prompt`, a toolbar button. Returns null when there is nothing to
 * restore. The restore refocuses the editing host too, because
 * `document.execCommand` acts on the focused editable and silently does nothing
 * without one.
 */
export function captureCaret(): (() => void) | null {
  if (typeof document === "undefined") return null;
  const sel = document.getSelection();
  if (!sel || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0).cloneRange();
  const anchor = range.commonAncestorContainer;
  const el = anchor.nodeType === 1 ? (anchor as Element) : anchor.parentElement;
  const host = el?.closest('[contenteditable="true"]') as HTMLElement | null;
  return () => {
    host?.focus();
    const after = document.getSelection();
    if (!after) return;
    after.removeAllRanges();
    after.addRange(range);
  };
}

/** Insert text at the caret without importing any foreign markup. */
function insertPlainTextAtCaret(text: string): void {
  // execCommand keeps the insertion in the browser's native undo stack, which is
  // what Cmd+Z uses inside a block. It's absent in jsdom, hence the fallback.
  try {
    if (typeof document.execCommand === "function" && document.execCommand("insertText", false, text)) {
      return;
    }
  } catch {
    // fall through
  }
  const sel = document.getSelection();
  if (!sel || sel.rangeCount === 0) return;
  const range = sel.getRangeAt(0);
  range.deleteContents();
  const node = document.createTextNode(text);
  range.insertNode(node);
  range.setStartAfter(node);
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

/**
 * The formatting shortcuts a text editor is expected to have, mapped to the
 * contentEditable command that performs them.
 *
 * They have to be bound by hand inside an editable block, because BOTH of the
 * layers that normally supply them are absent there:
 *
 *  - ProseMirror never sees the keystroke (`stopEvent` below), so TipTap's own
 *    `Mod-b` / `Mod-i` / `Mod-u` keymap — the thing that makes Cmd+B work
 *    everywhere else in the composer — never runs.
 *  - macOS does not supply them either. Cocoa has no Cmd+B key binding; bold in
 *    a native editing host comes from a Format menu item, and a WKWebView app
 *    with no such menu (CXMail has none) leaves the combo unclaimed. Safari's
 *    contenteditable responds to Cmd+B because *Safari* has that menu.
 *
 * Without this, the toolbar buttons are the only way to bold a word inside a
 * designed email — which is exactly how it looked: type freely, format never.
 */
function nativeCommandForShortcut(e: KeyboardEvent): string | null {
  // Cmd on macOS, Ctrl elsewhere. Alt combinations belong to other commands.
  // ⌘K is deliberately absent: it opens the link field, which the compose
  // editor's wrapper handles for plain text and blocks alike.
  if (!(e.metaKey || e.ctrlKey) || e.altKey) return null;
  if (e.shiftKey) {
    // e.code, not e.key: Shift turns "7" into "&" on a US layout.
    switch (e.code) {
      case "KeyS":
      case "KeyX":
        return "strikeThrough";
      case "Digit7":
        return "insertOrderedList";
      case "Digit8":
        return "insertUnorderedList";
      default:
        return null;
    }
  }
  switch (e.key.toLowerCase()) {
    case "b":
      return "bold";
    case "i":
      return "italic";
    case "u":
      return "underline";
    case "\\":
      return "removeFormat";
    default:
      return null;
  }
}

interface AtomicBlockOptions {
  name: string;
  tagName: "div" | "blockquote";
  marker: string; // e.g., "data-cx-signature"
  className: string;
  /**
   * Let the user type inside the block. The subtree becomes its own native
   * editing host and ProseMirror is told to ignore everything that happens in
   * it; edits are synced back into the `html` attribute by the node view.
   */
  editable?: boolean;
}

function createAtomicBlock({ name, tagName, marker, className, editable = false }: AtomicBlockOptions) {
  return Node.create({
    name,
    group: "block",
    atom: true,
    selectable: true,
    defining: true,
    // StarterKit's Blockquote parses `blockquote` at the default priority of 100
    // and would otherwise win the parse rule for QuotedBlock's `blockquote[data-cx-quote]`,
    // flattening tables/styles inside the quote. A higher priority forces our
    // attribute-bearing rule to match first. Same precaution for SignatureBlock
    // (no extension claims `div.email-signature` today, but cheap insurance).
    priority: 1000,

    addAttributes() {
      return {
        html: {
          default: "",
          parseHTML: (el: HTMLElement) => el.innerHTML,
          // Don't surface `html` as a DOM attribute — it's content, not metadata.
          renderHTML: () => ({}),
        },
      };
    },

    parseHTML() {
      return [
        { tag: `${tagName}[${marker}="1"]` },
        // Legacy selector covers content saved before this refactor.
        { tag: `${tagName}.${className}` },
      ];
    },

    // Returning a real DOM element (not a JSON DOMOutputSpec) lets us inject
    // the raw inner HTML so prosemirror's DOMSerializer captures it verbatim
    // when the editor produces getHTML() output.
    renderHTML({ node, HTMLAttributes }) {
      const dom = document.createElement(tagName);
      dom.setAttribute(marker, "1");
      dom.className = className;
      for (const [key, value] of Object.entries(HTMLAttributes)) {
        if (key === "html" || value == null) continue;
        dom.setAttribute(key, String(value));
      }
      const raw = (node.attrs as { html?: string }).html ?? "";
      dom.innerHTML = raw;
      return { dom };
    },

    addNodeView() {
      return ({ node, editor, getPos }) => {
        const dom = document.createElement(tagName);
        dom.setAttribute(marker, "1");
        dom.className = className;
        const html = (node.attrs as { html?: string }).html ?? "";

        if (!editable) {
          // Block edits inside the node so arbitrary email HTML doesn't get
          // mangled by ProseMirror selection / input handling.
          dom.innerHTML = html;
          dom.setAttribute("contenteditable", "false");
          return { dom };
        }

        // The editable content lives in a CHILD, and the node view's own root
        // stays uneditable. That is not cosmetic: a contenteditable="true"
        // nested directly inside another contenteditable="true" is not a
        // separate editing host — the browser treats the whole subtree as one,
        // so focus stays on the ProseMirror root and every keyboard event
        // targets it. `stopEvent` is dispatched by event target, so it would
        // never be consulted, and ProseMirror would apply the keystroke to its
        // own NodeSelection — replacing the entire styled card with the single
        // character just typed. Verified in Chromium: card gone on keystroke one.
        //
        // An editable child of an UNeditable parent is a genuine editing host.
        // It takes focus, so keydown/beforeinput target it, `stopEvent` fires,
        // and ProseMirror keeps its hands off. The wrapper is node-view-only —
        // `renderHTML` builds the serialized output, so it never reaches the
        // draft, the sent message, or the database.
        //
        // setAttribute rather than the `.contentEditable` property throughout:
        // ProseMirror only skips stamping its own contenteditable="false" when
        // the ATTRIBUTE is already present, and jsdom's property doesn't
        // reflect to the attribute.
        dom.setAttribute("contenteditable", "false");

        const host = document.createElement("div");
        host.setAttribute("contenteditable", "true");
        host.setAttribute("data-cx-html-body", "1");
        host.spellcheck = true;
        host.innerHTML = html;
        dom.appendChild(host);

        // The last `html` value this view and the document agree on. It's how
        // update() tells our own echo (leave the DOM and the caret alone) from a
        // genuine external change like an MCP live-reload (adopt it).
        let syncedHtml = html;
        let timer: ReturnType<typeof setTimeout> | null = null;
        let dirty = false;

        const push = () => {
          if (timer) {
            clearTimeout(timer);
            timer = null;
          }
          if (!dirty) return;
          dirty = false;

          const edited = host.innerHTML;
          if (edited === syncedHtml) return;

          const pos = typeof getPos === "function" ? getPos() : undefined;
          if (typeof pos !== "number") return;
          const { view } = editor;
          const target = view.state.doc.nodeAt(pos);
          if (!target || target.type.name !== name) return;

          // Dispatching re-runs ProseMirror's selection sync, which would write
          // its own (stale — we stop the events that would have updated it)
          // selection into the DOM and yank the caret out of the block. The DOM
          // range survives because update() leaves our nodes in place on an echo.
          const sel = document.getSelection();
          const savedRange =
            sel && sel.rangeCount > 0 && host.contains(sel.anchorNode)
              ? sel.getRangeAt(0).cloneRange()
              : null;

          syncedHtml = edited;
          view.dispatch(view.state.tr.setNodeAttribute(pos, "html", edited));

          if (savedRange) {
            const after = document.getSelection();
            if (after) {
              after.removeAllRanges();
              after.addRange(savedRange);
            }
          }
        };

        const markDirty = () => {
          dirty = true;
          if (timer) clearTimeout(timer);
          timer = setTimeout(push, 250);
        };

        // `dom` is a union of element types, so addEventListener resolves to the
        // untyped overload — hence the Event parameter and the cast.
        const onPaste = (e: Event) => {
          // Styled paste would drop foreign CSS (Word/Gmail wrappers) into a
          // design that's meant to stay exactly as authored.
          e.preventDefault();
          const text = (e as ClipboardEvent).clipboardData?.getData("text/plain") ?? "";
          if (text) insertPlainTextAtCaret(text);
          markDirty();
        };

        const onDrop = (e: Event) => e.preventDefault();

        const onKeyDown = (e: Event) => {
          const ev = e as KeyboardEvent;
          const command = nativeCommandForShortcut(ev);
          if (!command) return;
          // Claim the key even if the command throws: a half-handled shortcut
          // that also falls through to a native binding would toggle twice.
          ev.preventDefault();
          try {
            document.execCommand(command, false);
          } catch (err) {
            console.error(`Native ${command} failed inside styled block:`, err);
            return;
          }
          // execCommand fires `input` in a real browser, but not in every
          // environment — and a formatting edit that never reaches the `html`
          // attribute is a formatting edit that never gets sent.
          markDirty();
        };

        host.addEventListener("input", markDirty);
        host.addEventListener("keydown", onKeyDown);
        host.addEventListener("blur", push, true);
        host.addEventListener("paste", onPaste);
        host.addEventListener("drop", onDrop);
        pendingBlockFlushes.add(push);

        return {
          dom,
          // `contentDOM` stays undefined on purpose: naming the host would hand
          // it back to ProseMirror to parse, which is the flattening this whole
          // node exists to avoid.

          update: (updatedNode) => {
            if (updatedNode.type.name !== name) return false;
            const incoming = (updatedNode.attrs as { html?: string }).html ?? "";
            // Our own edit coming back around. Rewriting innerHTML here would
            // destroy the text nodes the caret lives in, so don't.
            if (incoming === syncedHtml) return true;
            syncedHtml = incoming;
            host.innerHTML = incoming;
            return true;
          },

          // ProseMirror keeps a NodeSelection on the block (its own mousedown
          // handling is stopped, so nothing moves it) and would otherwise paint
          // the whole card as selected while the user types inside it.
          selectNode: () => {},
          deselectNode: () => {},

          // Everything inside the block is the browser's business, not
          // ProseMirror's — including selection changes, which is what keeps it
          // from re-parsing a subtree its schema can't represent.
          stopEvent: () => true,
          ignoreMutation: () => true,

          destroy: () => {
            if (timer) clearTimeout(timer);
            host.removeEventListener("input", markDirty);
            host.removeEventListener("keydown", onKeyDown);
            host.removeEventListener("blur", push, true);
            host.removeEventListener("paste", onPaste);
            host.removeEventListener("drop", onDrop);
            pendingBlockFlushes.delete(push);
          },
        };
      };
    },
  });
}

export const SignatureBlock = createAtomicBlock({
  name: "signatureBlock",
  tagName: "div",
  marker: "data-cx-signature",
  className: "email-signature",
});

export const QuotedBlock = createAtomicBlock({
  name: "quotedBlock",
  tagName: "blockquote",
  marker: "data-cx-quote",
  className: "cx-quote",
});

// Generic "rich HTML" block. The cxmail MCP's `layout: "card" | "rich"` drafts
// wrap their styled markup (table layout, inline-styled <div>s, etc.) in a
// `div.cx-html-block`. Like the signature/quote blocks, this is the only way to
// carry arbitrary email HTML through the compose round-trip — StarterKit would
// otherwise flatten tables and custom-styled divs the moment the draft is opened
// to send. Survival hinges on the `class` selector: ammonia strips the
// `data-cx-html` marker (not in its attribute allowlist) but keeps `class`, so
// `div.cx-html-block` is what re-claims the block as atomic on reopen.
//
// Unlike the other two this block is editable, so a typo in a designed email is
// a click-and-retype instead of a round-trip through Claude. Native
// contentEditable does the editing, which is precisely why the surrounding
// inline styles survive a word swap untouched — structural changes (add a table
// row, restyle a button) still belong to whoever authored the HTML.
export const HtmlBlock = createAtomicBlock({
  name: "htmlBlock",
  tagName: "div",
  marker: "data-cx-html",
  className: HTML_BLOCK_CLASS,
  editable: true,
});
