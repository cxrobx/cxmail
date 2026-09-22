/**
 * Put back the structure another mail client stripped from a CXMail draft.
 *
 * CXMail keeps the signature and the quoted history out of TipTap's hands by
 * marking them: `div.email-signature` and `blockquote.cx-quote` parse into
 * atomic nodes that store their HTML verbatim. Gmail's editor keeps neither
 * class, so a draft saved there comes back as a `<div dir="ltr">` body and a bare
 * `<blockquote>`. Opened as-is, TipTap (which has no table node) flattened the
 * signature table into paragraphs, the quote lost its protection, and the
 * signature step found no marker and appended a second signature after the
 * quote (T186).
 *
 * This runs on the HTML of a reopened draft, before TipTap parses it:
 *  - a trailing top-level quote becomes a `cx-quote` block again;
 *  - every table outside a protected block is wrapped in a `cx-html-block`, so
 *    it survives as a table (the signature step then recognises the one that
 *    is the signature — see `claimExistingSignature`).
 *
 * A draft that still carries CXMail's markers is returned untouched: that is
 * the CXMail-to-CXMail round trip, which already works, and gotcha #34's
 * behaviour for designed HTML without `layout` is deliberately not changed here.
 */
import { splitTrailingQuote } from "@/lib/quoteToggle";

const PROTECTED = "div.email-signature, div[data-cx-signature], blockquote.cx-quote, blockquote[data-cx-quote], div.cx-html-block";

export function hasCxMarkers(html: string): boolean {
  return /\bemail-signature\b|\bcx-quote\b|data-cx-signature|data-cx-quote/.test(html);
}

export function recoverForeignDraftHtml(html: string): string {
  if (!html || hasCxMarkers(html)) return html;
  try {
    const { mainHtml, quotedHtml } = splitTrailingQuote(html);
    const body = wrapLooseTables(mainHtml);
    if (!quotedHtml) return body;
    return `${body}<blockquote data-cx-quote="1" class="cx-quote">${unwrapSingleBlockquote(quotedHtml)}</blockquote>`;
  } catch {
    return html;
  }
}

function wrapLooseTables(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  // Outermost tables only: a table nested in a table travels with its parent.
  const tables = Array.from(doc.body.querySelectorAll("table")).filter(
    (t) => !t.parentElement?.closest("table") && !t.closest(PROTECTED),
  );
  for (const table of tables) {
    const block = doc.createElement("div");
    block.className = "cx-html-block";
    table.replaceWith(block);
    block.appendChild(table);
  }
  return doc.body.innerHTML;
}

/** `<blockquote>X</blockquote>` → `X`, so the recovered quote is not nested twice. */
function unwrapSingleBlockquote(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  const kids = Array.from(doc.body.childNodes).filter(
    (n) => !(n.nodeType === Node.TEXT_NODE && !(n.textContent || "").trim()),
  );
  if (kids.length === 1 && (kids[0] as Element).tagName === "BLOCKQUOTE") {
    return (kids[0] as Element).innerHTML;
  }
  return html;
}

// Whitespace is dropped entirely, not collapsed: `<p>CX</p><p>Ventures</p>` and
// `<div>CX</div>\n<div>Ventures</div>` are the same signature, but textContent
// gives one "CXVentures" and the other "CX\nVentures".
function normalizedText(el: { textContent: string | null }): string {
  return (el.textContent || "").replace(/\s+/g, "");
}

/**
 * Whether the editor's HTML already contains the account's signature, and if so
 * the HTML with that copy re-marked as the signature block.
 *
 * - A `cx-html-block` whose text IS the signature's text becomes the signature
 *   block again (its own markup kept, so nothing the user sees changes).
 * - Otherwise, if the signature's text appears anywhere — e.g. already
 *   flattened into paragraphs by an earlier broken save — it counts as present:
 *   appending would put a second signature in the draft.
 *
 * Returns `{ present: false }` when the signature is nowhere, so the caller
 * appends one as before.
 */
export function claimExistingSignature(
  editorHtml: string,
  signatureHtml: string,
): { present: boolean; html?: string } {
  if (!signatureHtml) return { present: false };
  const sigDoc = new DOMParser().parseFromString(`<body>${signatureHtml}</body>`, "text/html");
  const sigText = normalizedText(sigDoc.body);
  if (!sigText) return { present: false };

  const doc = new DOMParser().parseFromString(`<body>${editorHtml}</body>`, "text/html");
  const outsideQuote = (el: Element) => !el.closest("blockquote");
  const match = Array.from(doc.body.querySelectorAll("div.cx-html-block"))
    .filter(outsideQuote)
    .find((el) => normalizedText(el) === sigText);
  if (match) {
    const sig = doc.createElement("div");
    sig.setAttribute("data-cx-signature", "1");
    sig.className = "email-signature";
    sig.innerHTML = match.innerHTML;
    match.replaceWith(sig);
    return { present: true, html: doc.body.innerHTML };
  }

  // Text check against the body only: a quoted older message signed by the
  // same person carries the same signature and must not count.
  const bodyOnly = doc.body.cloneNode(true) as HTMLElement;
  bodyOnly.querySelectorAll("blockquote").forEach((q) => q.remove());
  return { present: normalizedText(bodyOnly).includes(sigText) };
}

/** Insert the signature block before the quoted history, not after it. */
export function insertSignatureBeforeQuote(editorHtml: string, sigBlock: string): string {
  const doc = new DOMParser().parseFromString(`<body>${editorHtml}</body>`, "text/html");
  const quote = Array.from(doc.body.children).find(
    (el) => el.matches("blockquote.cx-quote, blockquote[data-cx-quote]"),
  );
  if (!quote) return `${editorHtml}${sigBlock}`;
  const tmp = doc.createElement("div");
  tmp.innerHTML = sigBlock;
  for (const n of Array.from(tmp.childNodes)) quote.before(n);
  return doc.body.innerHTML;
}

export function isEmptyEditorHtml(html: string): boolean {
  return !html || html === "<p></p>" || html.trim() === "";
}

/**
 * The compose window's one-time signature step, as a pure function: the HTML
 * to load into the editor, or null to leave it alone.
 *
 * - already marked → null (a CXMail draft reopened in CXMail)
 * - a reopened draft that still holds the signature → re-mark it, or leave it
 *   (T186: appending here is what produced two signatures)
 * - otherwise append the signature, above the quoted history
 */
export function placeSignature({
  currentHtml,
  signatureHtml,
  quotedHtml,
  isDraft,
}: {
  currentHtml: string;
  signatureHtml: string | null;
  quotedHtml: string | null;
  isDraft: boolean;
}): string | null {
  if (
    currentHtml.includes('data-cx-signature="1"') ||
    /class="[^"]*\bemail-signature\b[^"]*"/.test(currentHtml)
  ) {
    return null;
  }
  if (isDraft && signatureHtml) {
    const claimed = claimExistingSignature(currentHtml, signatureHtml);
    if (claimed.present) return claimed.html ?? null;
  }
  const sigBlock = signatureHtml
    ? `<div data-cx-signature="1" class="email-signature">${signatureHtml}</div>`
    : "";
  const quotedBlock = quotedHtml
    ? `<blockquote data-cx-quote="1" class="cx-quote">${quotedHtml}</blockquote>`
    : "";
  if (!quotedBlock && !sigBlock) return null;
  // Treat a TipTap "empty" doc as no body so we don't double up the leading paragraph.
  const bodyPart = isEmptyEditorHtml(currentHtml) ? "<p></p>" : currentHtml;
  // Without a quotedHtml prop the quote (if any) is already in the body — a
  // reopened draft — and the signature belongs above it, not below.
  return quotedBlock
    ? `${bodyPart}${sigBlock}${quotedBlock}`
    : insertSignatureBeforeQuote(bodyPart, sigBlock);
}
