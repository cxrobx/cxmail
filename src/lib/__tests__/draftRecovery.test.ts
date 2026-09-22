import { describe, it, expect, afterEach } from "vitest";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { SignatureBlock, QuotedBlock, HtmlBlock, flushHtmlBlockEdits } from "@/lib/composeNodes";
import { ComposeShortcuts } from "@/lib/composeShortcuts";
import { placeSignature, recoverForeignDraftHtml } from "@/lib/draftRecovery";

// T186. A reply draft reopened in compose after Gmail's editor had saved it lost
// its signature table (flattened to paragraphs) and gained a second signature
// below the quote. The artefact that matters is the HTML that gets STORED, so
// every test here drives the real open → signature step → edit → save loop and
// inspects what a save would write, three saves deep.

const SIGNATURE =
  '<div><table cellpadding="0" cellspacing="0" border="0"><tbody><tr>' +
  '<td style="padding-right:12px"><div><strong>CX</strong></div><div>Ventures</div></td>' +
  '<td><div><strong>Christopher Robinson</strong></div><div>CX Ventures</div>' +
  '<div><a href="tel:+1">+1 (713) 555-0100</a><span> | </span><a href="mailto:c@x.io">c@x.io</a></div></td>' +
  "</tr></tbody></table></div>";

// An older message in the quote, signed with the same table.
const OLDER = `<div><p>Earlier note from Chris.</p><div>${SIGNATURE}</div></div>`;

/** What CXMail itself writes for a reply draft. */
const CXMAIL_DRAFT =
  `<p>Hi Dana,</p><p>Here is the walkthrough.</p>` +
  `<div class="email-signature" style="margin-top:16px;">${SIGNATURE}</div>` +
  `<blockquote class="cx-quote"><br><br><div><p><strong>Dana</strong> wrote on 9/16:</p><p>Looks good.</p>${OLDER}</div></blockquote>`;

/** The same draft after Gmail's editor saved it (UID 988's shape): markers gone. */
const GMAIL_SAVED =
  `<div dir="ltr"><p>Hi Dana,</p><p>Here is the walkthrough.</p><div><div>${SIGNATURE}</div></div></div>` +
  `<blockquote><br><br><div><p><strong>Dana</strong> wrote on 9/16:</p><p>Looks good.</p>${OLDER}</div></blockquote>`;

const EXTENSIONS = [
  StarterKit.configure({ link: { openOnClick: false } }),
  ComposeShortcuts,
  Image.configure({ inline: true, allowBase64: true }),
  SignatureBlock,
  QuotedBlock,
  HtmlBlock,
];

let editor: Editor | null = null;
afterEach(() => {
  editor?.destroy();
  editor = null;
});

/** Storage keeps `class` and drops `data-*` (ammonia), so a reopen sees that. */
function store(html: string): string {
  return html.replace(/\sdata-[a-z-]+="[^"]*"/g, "");
}

/** One compose session on a reopened draft, the way ComposeModal runs it. */
function openEditSave(stored: string, signatureHtml: string, edit: string): string {
  editor = new Editor({
    element: document.createElement("div"),
    extensions: EXTENSIONS,
    content: recoverForeignDraftHtml(stored),
  });
  const placed = placeSignature({
    currentHtml: editor.getHTML(),
    signatureHtml,
    quotedHtml: null,
    isDraft: true,
  });
  if (placed !== null) editor.commands.setContent(placed);
  editor.commands.insertContentAt(1, edit);
  flushHtmlBlockEdits();
  const html = editor.getHTML();
  editor.destroy();
  editor = null;
  return store(html);
}

function shape(html: string) {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  const top = Array.from(doc.body.children);
  const sigs = doc.body.querySelectorAll("div.email-signature");
  const quoteIdx = top.findIndex((el) => el.matches("blockquote.cx-quote"));
  const sigIdx = top.findIndex((el) => el.matches("div.email-signature"));
  const quote = top[quoteIdx];
  return {
    signatures: sigs.length,
    signatureIsTable: !!sigs[0]?.querySelector("table td"),
    signatureAboveQuote: sigIdx >= 0 && quoteIdx >= 0 && sigIdx < quoteIdx,
    quotes: top.filter((el) => el.matches("blockquote")).length,
    tablesInQuote: quote ? quote.querySelectorAll("table").length : 0,
    flattenedSignatureParagraphs: Array.from(doc.body.querySelectorAll("p")).filter(
      (p) => !p.closest("blockquote") && p.textContent?.trim() === "Christopher Robinson",
    ).length,
    // Every copy of the signature outside the quote, marked or not — a table
    // left in a plain html block beside a fresh signature is still two.
    signatureCopiesInBody: (() => {
      const body = doc.body.cloneNode(true) as HTMLElement;
      body.querySelectorAll("blockquote").forEach((q) => q.remove());
      return (body.textContent || "").split("Christopher Robinson").length - 1;
    })(),
    text: doc.body.textContent || "",
  };
}

function expectHealthy(html: string) {
  const s = shape(html);
  expect(s.signatures).toBe(1);
  expect(s.signatureCopiesInBody).toBe(1);
  expect(s.signatureIsTable).toBe(true);
  expect(s.signatureAboveQuote).toBe(true);
  expect(s.quotes).toBe(1);
  expect(s.tablesInQuote).toBe(1);
  expect(s.flattenedSignatureParagraphs).toBe(0);
}

describe("reopening a draft in compose (T186)", () => {
  it("survives three saves after Gmail's editor stripped the markers", () => {
    let stored = GMAIL_SAVED;
    for (const edit of ["First. ", "Second. ", "Third. "]) {
      stored = openEditSave(stored, SIGNATURE, edit);
      expectHealthy(stored);
    }
    expect(shape(stored).text).toContain("Third. Second. First. Hi Dana");
  });

  it("survives three saves of a draft CXMail wrote itself", () => {
    let stored = store(CXMAIL_DRAFT);
    for (const edit of ["One. ", "Two. ", "Three. "]) {
      stored = openEditSave(stored, SIGNATURE, edit);
      expectHealthy(stored);
    }
  });

  it("appends a changed signature once, above the quote", () => {
    const newSig = "<div><table><tbody><tr><td>Chris — new title</td></tr></tbody></table></div>";
    let stored = GMAIL_SAVED;
    for (const edit of ["a ", "b ", "c "]) {
      stored = openEditSave(stored, newSig, edit);
      const s = shape(stored);
      expect(s.signatures).toBe(1);
      expect(s.signatureAboveQuote).toBe(true);
      expect(s.quotes).toBe(1);
    }
  });

  it("does not add a second signature to a draft whose first one was already flattened", () => {
    const damaged =
      `<p>Hi Dana,</p><p><strong>CX</strong></p><p>Ventures</p><p><strong>Christopher Robinson</strong></p>` +
      `<p>CX Ventures</p><p><a href="tel:+1">+1 (713) 555-0100</a> | <a href="mailto:c@x.io">c@x.io</a></p>` +
      `<blockquote><p>Looks good.</p></blockquote>`;
    const stored = openEditSave(damaged, SIGNATURE, "x ");
    expect(shape(stored).signatures).toBe(0);
    expect(shape(stored).quotes).toBe(1);
  });

  it("does not mistake the signature inside the quoted history for the draft's own", () => {
    const noOwnSig = `<div dir="ltr"><p>Hi Dana,</p></div><blockquote><div><p>Looks good.</p>${OLDER}</div></blockquote>`;
    const stored = openEditSave(noOwnSig, SIGNATURE, "x ");
    expectHealthy(stored);
  });
});
