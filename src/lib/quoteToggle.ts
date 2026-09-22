/**
 * Split sanitized HTML into a "main" portion and a trailing "quoted" portion
 * (everything from the first detected quote marker onward, inclusive).
 *
 * The detection runs over top-level nodes only — it does not recurse into
 * arbitrary nesting — and prefers the earliest quote-start so we capture
 * the full "On <date> wrote: ..." trailer.
 *
 * Returns `{ mainHtml: html, quotedHtml: null }` when no quote is detected
 * or when stripping the quote would leave the main body empty (a message
 * that's nothing but a quote isn't really a quoted reply — show it as-is).
 */
export interface QuoteSplit {
  mainHtml: string;
  quotedHtml: string | null;
}

export function splitTrailingQuote(html: string): QuoteSplit {
  if (!html) return { mainHtml: html, quotedHtml: null };

  const doc = new DOMParser().parseFromString(
    `<div id="cx-root">${html}</div>`,
    "text/html",
  );
  const root = doc.getElementById("cx-root");
  if (!root) return { mainHtml: html, quotedHtml: null };

  const nodes = Array.from(root.childNodes);
  if (nodes.length === 0) return { mainHtml: html, quotedHtml: null };

  // Find the EARLIEST top-level node that's a quote-start — once we see
  // "On <date> wrote:" followed by a blockquote, we want everything from
  // the lead-in onward in the quoted blob.
  let quoteStartIdx: number | null = null;
  for (let i = nodes.length - 1; i >= 0; i--) {
    if (isQuoteStart(nodes[i])) {
      quoteStartIdx = i;
    }
  }

  if (quoteStartIdx === null) {
    return { mainHtml: html, quotedHtml: null };
  }

  const mainHtml = serializeSlice(doc, nodes.slice(0, quoteStartIdx));
  // If stripping the quote leaves nothing visible, treat the message as
  // having no separable trailing quote — the user wants to see it.
  if (!mainHtml.trim()) {
    return { mainHtml: html, quotedHtml: null };
  }

  const quotedHtml = serializeSlice(doc, nodes.slice(quoteStartIdx));
  return { mainHtml, quotedHtml: quotedHtml || null };
}

function isQuoteStart(node: Node): boolean {
  if (node.nodeType !== Node.ELEMENT_NODE) return false;
  const el = node as HTMLElement;
  const tag = el.tagName.toLowerCase();

  if (tag === "blockquote") return true;
  if (tag === "div") {
    if (el.classList.contains("gmail_quote")) return true;
    if (el.getAttribute("data-cx-quote") === "1") return true;
    // "On <date>, <name> wrote:" — Apple Mail / Outlook plaintext-converted
    // replies. Multi-line via `m` flag because some clients break the
    // attribution across lines.
    const text = (el.textContent || "").trim();
    if (
      /^On\s.+\s(?:wrote|escribió|écrit|написал)\s*:?\s*$/im.test(text)
    ) {
      return true;
    }
  }
  return false;
}

function serializeSlice(doc: Document, slice: Node[]): string {
  const tmp = doc.createElement("div");
  for (const n of slice) tmp.appendChild(n.cloneNode(true));
  return tmp.innerHTML;
}
