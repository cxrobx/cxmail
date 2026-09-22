import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import {
  SignatureBlock,
  QuotedBlock,
  HtmlBlock,
  flushHtmlBlockEdits,
  isEditingHtmlBlock,
} from "@/lib/composeNodes";

// The cxmail MCP's layout="card"/"rich" feature hinges on a single frontend
// property: a styled email loaded into the compose editor must survive the
// TipTap round-trip (setContent -> getHTML) verbatim, instead of being flattened
// by StarterKit when the user opens the draft to send. HtmlBlock is the unlock;
// the `div.cx-html-block` class is the load-bearing selector (the data-cx-html
// marker is stripped by the backend sanitize, so class is all that survives to
// re-claim the block as atomic). These tests assert that round-trip directly.

// The shape a reopened MCP card draft actually has AFTER the backend pipeline:
// sanitize drops data-cx-html but keeps class; apply_inline_font_styles injects a
// font style on the wrapper div. The inner card chrome carries the card's inline
// styles. (No data-cx-html marker present — class is the only hook.)
const CARD_DRAFT_BODY =
  `<div class="cx-html-block" style="font-family:Arial,Helvetica,sans-serif;color:#222222">` +
  `<div style="background-color:#f4f5f7;padding:40px 24px;">` +
  `<div style="max-width:600px;margin:0 auto;background-color:#ffffff;border-radius:8px;` +
  `box-shadow:0 2px 8px rgba(0,0,0,0.08);padding:32px;">` +
  `<h1 style="margin:0">PEAK6</h1><p>Welcome aboard.</p>` +
  `</div></div></div>`;

function makeEditor(extensions: NonNullable<ConstructorParameters<typeof Editor>[0]>["extensions"]) {
  return new Editor({
    element: document.createElement("div"),
    extensions,
    content: CARD_DRAFT_BODY,
  });
}

let editor: Editor | null = null;
afterEach(() => {
  editor?.destroy();
  editor = null;
});

describe("HtmlBlock round-trip (the layout=card/rich unlock)", () => {
  it("preserves the styled card verbatim through setContent -> getHTML", () => {
    editor = makeEditor([StarterKit, Image, SignatureBlock, QuotedBlock, HtmlBlock]);
    const html = editor.getHTML();

    // The load-bearing class re-claims the block.
    expect(html).toContain('class="cx-html-block"');
    // Card chrome survives verbatim — not flattened by StarterKit.
    expect(html).toContain("max-width:600px");
    expect(html).toContain("background-color:#ffffff");
    expect(html).toContain("border-radius:8px");
    // Inner content survives.
    expect(html).toContain("PEAK6");
    expect(html).toContain("Welcome aboard.");
  });

  it("NEGATIVE CONTROL: without HtmlBlock, StarterKit flattens the card (proves the block is the unlock)", () => {
    editor = makeEditor([StarterKit, Image, SignatureBlock, QuotedBlock]);
    const html = editor.getHTML();

    // StarterKit has no div node, so the nested styled divs and their inline
    // styles are dropped — the card chrome does NOT survive.
    expect(html).not.toContain("max-width:600px");
    expect(html).not.toContain("background-color:#ffffff");
    // (Text content may survive as bare paragraphs; the STYLING is what's lost.)
  });

  it("POSITIVE CONTROL: StarterKit alone preserves simple markup, so plain drafts need no layout", () => {
    // The mirror of the negative control above, and the reason the rewritten
    // `layout` docs steer callers to OMIT it for ordinary mail. The 2026-07-30
    // Northwind draft was blamed on the missing layout param, but its bullets were
    // hand-built table rows; a real <ul> would have come through untouched.
    // Warning on every HTML draft — or reaching for layout="rich" on prose —
    // would seal an editable draft into a structure-locked block for nothing.
    editor = new Editor({
      element: document.createElement("div"),
      extensions: [StarterKit, Image, SignatureBlock, QuotedBlock],
      content:
        `<p>Quick update on the audit.</p>` +
        `<ul><li>Baseline is <strong>done</strong></li><li>Crawl runs Friday</li></ul>` +
        `<h2>Next steps</h2>` +
        `<p>See the <a href="https://example.test/report">report</a>.</p>`,
    });
    const html = editor.getHTML();

    // Lists keep their structure — bullets stay bullets, not run-on text.
    expect(html).toContain("<ul>");
    expect(html).toContain("<li>");
    expect(html).toContain("Baseline is");
    expect(html).toContain("Crawl runs Friday");
    // Bold, headings, and links all round-trip.
    expect(html).toMatch(/<strong>done<\/strong>/);
    expect(html).toContain("Next steps");
    expect(html).toContain('href="https://example.test/report"');
    // No preservation block needed for any of it.
    expect(html).not.toContain("cx-html-block");
  });

  it("preserves both a card block and the appended signature in one draft", () => {
    // The realistic MCP draft shape: card + signature div appended below it.
    const body =
      CARD_DRAFT_BODY +
      `<div data-cx-signature="1" class="email-signature" style="margin-top:16px">` +
      `<p>Christopher Robinson</p></div>`;
    editor = new Editor({
      element: document.createElement("div"),
      extensions: [StarterKit, Image, SignatureBlock, QuotedBlock, HtmlBlock],
      content: body,
    });
    const html = editor.getHTML();

    expect(html).toContain('class="cx-html-block"');
    expect(html).toContain("max-width:600px");
    expect(html).toContain('class="email-signature"');
    expect(html).toContain("Christopher Robinson");
  });
});

// In-place editing of a styled block. ProseMirror never sees these keystrokes —
// the node view stops the events and ignores the mutations, so the browser's
// native contentEditable does the editing and the surrounding inline styles come
// through a word swap untouched. The cost is that the node's `html` attribute
// (what getHTML serializes) is stale until something flushes it.

const ALL_BLOCKS = [StarterKit, Image, SignatureBlock, QuotedBlock, HtmlBlock];

/** The node view's outer element — a ProseMirror leaf, deliberately uneditable. */
function blockEl(ed: Editor): HTMLElement {
  const el = ed.view.dom.querySelector("div.cx-html-block");
  if (!el) throw new Error("cx-html-block node view did not render");
  return el as HTMLElement;
}

/**
 * The nested editing host that actually holds the email markup. It has to be a
 * child of an uneditable parent to be a separate editing host at all — see the
 * comment in composeNodes.ts; without that, keystrokes reach ProseMirror and it
 * replaces the whole card.
 */
function hostEl(ed: Editor): HTMLElement {
  const el = blockEl(ed).querySelector('[data-cx-html-body="1"]');
  if (!el) throw new Error("editable host did not render inside the block");
  return el as HTMLElement;
}

/** Retype a word inside the card the way a user would, then signal the DOM change. */
function typeInBlock(ed: Editor, mutate: (host: HTMLElement) => void): void {
  const host = hostEl(ed);
  mutate(host);
  host.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("HtmlBlock in-place editing", () => {
  it("is editable, while the signature and quote blocks stay sealed", () => {
    editor = new Editor({
      element: document.createElement("div"),
      extensions: ALL_BLOCKS,
      content:
        CARD_DRAFT_BODY +
        `<div data-cx-signature="1" class="email-signature"><p>Chris</p></div>` +
        `<blockquote data-cx-quote="1" class="cx-quote"><p>Original</p></blockquote>`,
    });
    const dom = editor.view.dom;

    // The editable host must be a child of an UNEDITABLE node-view root. Nesting
    // contenteditable="true" directly inside the editor's own contenteditable
    // makes one editing host, not two: focus stays on the ProseMirror root, key
    // events target it instead of the block, `stopEvent` is never consulted, and
    // the first keystroke replaces the whole card. Both halves of this assertion
    // are load-bearing.
    expect(blockEl(editor).getAttribute("contenteditable")).toBe("false");
    expect(hostEl(editor).getAttribute("contenteditable")).toBe("true");

    expect(dom.querySelector("div.email-signature")?.getAttribute("contenteditable")).toBe("false");
    expect(dom.querySelector("blockquote.cx-quote")?.getAttribute("contenteditable")).toBe("false");
    // Sealed blocks get no editing host at all.
    expect(dom.querySelector('div.email-signature [data-cx-html-body="1"]')).toBeNull();
  });

  it("carries a typed edit into getHTML() with the card's styling intact", () => {
    editor = makeEditor(ALL_BLOCKS);
    typeInBlock(editor, (block) => {
      const h1 = block.querySelector("h1");
      if (h1) h1.textContent = "PEAK6 Capital";
    });

    flushHtmlBlockEdits();
    const html = editor.getHTML();

    expect(html).toContain("PEAK6 Capital");
    expect(html).not.toContain(">PEAK6<");
    // The whole point: editing text does not disturb the design around it.
    expect(html).toContain("max-width:600px");
    expect(html).toContain("background-color:#ffffff");
    expect(html).toContain("border-radius:8px");
    expect(html).toContain('class="cx-html-block"');
  });

  it("NEGATIVE CONTROL: without the flush the edit is invisible to getHTML (proves the flush is load-bearing)", () => {
    editor = makeEditor(ALL_BLOCKS);
    typeInBlock(editor, (block) => {
      const h1 = block.querySelector("h1");
      if (h1) h1.textContent = "PEAK6 Capital";
    });

    // No flush, and the node view's own debounce hasn't fired: ProseMirror still
    // holds the pre-edit markup. This is the state buildOutgoingEmail() would
    // serialize and send if it stopped calling flushHtmlBlockEdits().
    expect(editor.getHTML()).not.toContain("PEAK6 Capital");
  });

  it("survives repeated edit-then-flush cycles", () => {
    editor = makeEditor(ALL_BLOCKS);
    for (const text of ["First pass", "Second pass", "Third pass"]) {
      typeInBlock(editor, (block) => {
        const h1 = block.querySelector("h1");
        if (h1) h1.textContent = text;
      });
      flushHtmlBlockEdits();
    }
    const html = editor.getHTML();
    expect(html).toContain("Third pass");
    expect(html).not.toContain("First pass");
    expect(html).toContain("max-width:600px");
  });

  it("flushing with nothing typed leaves the document untouched", () => {
    editor = makeEditor(ALL_BLOCKS);
    const before = editor.getHTML();
    flushHtmlBlockEdits();
    flushHtmlBlockEdits();
    expect(editor.getHTML()).toBe(before);
  });

  it("adopts an external content change into the block's DOM (MCP live-reload path)", () => {
    editor = makeEditor(ALL_BLOCKS);
    const replacement = `<h1 style="margin:0">Rewritten by Claude</h1><p>New body.</p>`;

    // What an MCP edit_draft reload does: replace the node's html attribute from
    // outside. The node view must re-render rather than keep showing stale text.
    const { view } = editor;
    view.dispatch(view.state.tr.setNodeAttribute(0, "html", replacement));

    expect(blockEl(editor).innerHTML).toContain("Rewritten by Claude");
    expect(editor.getHTML()).toContain("Rewritten by Claude");
  });

  it("refuses pasted markup so foreign CSS can't corrupt the design", () => {
    editor = makeEditor(ALL_BLOCKS);
    const evt = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(evt, "clipboardData", {
      value: {
        getData: (type: string) =>
          type === "text/plain" ? "pasted words" : `<div style="color:red">pasted words</div>`,
      },
    });

    hostEl(editor).dispatchEvent(evt);

    // Default prevented means the browser never inserts the text/html flavour;
    // only the plain-text one goes in, at the caret.
    expect(evt.defaultPrevented).toBe(true);
    expect(editor.getHTML()).not.toContain("color:red");
  });

  it("unregisters its flush when the block goes away", () => {
    editor = makeEditor(ALL_BLOCKS);
    typeInBlock(editor, (block) => {
      const h1 = block.querySelector("h1");
      if (h1) h1.textContent = "Doomed edit";
    });

    // Replacing the document destroys the node view. A flush afterwards must not
    // reach into the torn-down view and dispatch against a stale position.
    editor.commands.setContent("<p>Something else entirely</p>");
    expect(() => flushHtmlBlockEdits()).not.toThrow();
    expect(editor.getHTML()).toContain("Something else entirely");
  });
});


// Cmd+B / Cmd+I / Cmd+U inside a styled block. Two layers that normally provide
// them are both absent here: ProseMirror never sees the keystroke (`stopEvent`),
// so TipTap's Mod-b keymap can't run, and a WKWebView with no Format menu gets
// no binding from macOS either. Without the node view's own handler the toolbar
// buttons are the only way to bold a word in a designed email.
describe("HtmlBlock formatting shortcuts", () => {
  let exec: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    // jsdom ships no execCommand at all.
    exec = vi.fn(() => true);
    (document as unknown as { execCommand: unknown }).execCommand = exec;
  });

  afterEach(() => {
    delete (document as unknown as { execCommand?: unknown }).execCommand;
  });

  function press(ed: Editor, init: KeyboardEventInit): KeyboardEvent {
    const evt = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    hostEl(ed).dispatchEvent(evt);
    return evt;
  }

  it("runs the native bold command and carries the result into getHTML", () => {
    editor = makeEditor(ALL_BLOCKS);
    // Stand in for what a browser's bold command does to the DOM, so the
    // assertion below can follow the edit all the way into getHTML().
    const host = hostEl(editor);
    exec.mockImplementation((command: string) => {
      if (command === "bold") {
        const h1 = host.querySelector("h1");
        if (h1) h1.innerHTML = `<b>${h1.innerHTML}</b>`;
      }
      return true;
    });

    const evt = press(editor, { key: "b", metaKey: true });

    expect(exec).toHaveBeenCalledWith("bold", false);
    // Claiming the key matters: an unclaimed combo can also reach a native
    // binding and toggle the formatting straight back off.
    expect(evt.defaultPrevented).toBe(true);

    // The shortcut must mark the block dirty, or the formatting lives in the DOM
    // and never reaches the draft that gets sent.
    flushHtmlBlockEdits();
    expect(editor.getHTML()).toContain("<b>");
  });

  it("maps italic and underline too, on Ctrl as well as Cmd", () => {
    editor = makeEditor(ALL_BLOCKS);
    press(editor, { key: "i", metaKey: true });
    press(editor, { key: "u", ctrlKey: true });
    expect(exec).toHaveBeenCalledWith("italic", false);
    expect(exec).toHaveBeenCalledWith("underline", false);
  });

  it("maps strikethrough, lists and remove-formatting like the plain editor", () => {
    editor = makeEditor(ALL_BLOCKS);
    // Shift changes e.key ("7" → "&"), so digits must be matched by e.code.
    press(editor, { key: "&", code: "Digit7", metaKey: true, shiftKey: true });
    press(editor, { key: "*", code: "Digit8", metaKey: true, shiftKey: true });
    press(editor, { key: "x", code: "KeyX", metaKey: true, shiftKey: true });
    press(editor, { key: "s", code: "KeyS", ctrlKey: true, shiftKey: true });
    press(editor, { key: "\\", code: "Backslash", metaKey: true });
    expect(exec.mock.calls.map((c) => c[0])).toEqual([
      "insertOrderedList",
      "insertUnorderedList",
      "strikeThrough",
      "strikeThrough",
      "removeFormat",
    ]);
  });

  it("leaves plain typing alone", () => {
    editor = makeEditor(ALL_BLOCKS);
    const evt = press(editor, { key: "b" });
    expect(exec).not.toHaveBeenCalled();
    // Preventing this would stop the letter from being typed at all.
    expect(evt.defaultPrevented).toBe(false);
  });

  it("ignores combos that belong to someone else", () => {
    editor = makeEditor(ALL_BLOCKS);
    // Cmd+K opens the link field (the editor wrapper claims it, not the block);
    // Cmd+Shift+B is nobody's bold.
    const palette = press(editor, { key: "k", metaKey: true });
    const shifted = press(editor, { key: "B", metaKey: true, shiftKey: true });
    expect(exec).not.toHaveBeenCalled();
    expect(palette.defaultPrevented).toBe(false);
    expect(shifted.defaultPrevented).toBe(false);
  });

  it("does not reach the sealed signature block", () => {
    editor = new Editor({
      element: document.createElement("div"),
      extensions: ALL_BLOCKS,
      content: `<div data-cx-signature="1" class="email-signature"><p>Chris</p></div>`,
    });
    const sig = editor.view.dom.querySelector("div.email-signature") as HTMLElement;
    sig.dispatchEvent(new KeyboardEvent("keydown", { key: "b", metaKey: true, bubbles: true, cancelable: true }));
    expect(exec).not.toHaveBeenCalled();
  });
});

describe("isEditingHtmlBlock", () => {
  it("is false when the caret is nowhere near a styled block", () => {
    editor = makeEditor(ALL_BLOCKS);
    document.getSelection()?.removeAllRanges();
    expect(isEditingHtmlBlock()).toBe(false);
  });

  it("is true when the caret sits inside one", () => {
    editor = makeEditor(ALL_BLOCKS);
    // The block has to be in the document for a Selection to resolve into it.
    document.body.appendChild(editor.view.dom);
    const heading = blockEl(editor).querySelector("h1");
    expect(heading).not.toBeNull();

    const range = document.createRange();
    range.selectNodeContents(heading as Element);
    const sel = document.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);

    expect(isEditingHtmlBlock()).toBe(true);

    sel?.removeAllRanges();
    editor.view.dom.remove();
  });
});
