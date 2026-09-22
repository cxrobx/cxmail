import { describe, it, expect, afterEach } from "vitest";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { SignatureBlock, QuotedBlock } from "@/lib/composeNodes";
import {
  appendParagraph,
  bodyEndPos,
  findTextRange,
  replaceText,
} from "@/lib/draftEdits";

// jsdom can build and mutate a real ProseMirror document faithfully; what it
// cannot model is focus and selection (gotcha #33). Everything asserted here is
// document state, never caret position.
const editors: Editor[] = [];

function makeEditor(content: string): Editor {
  const e = new Editor({
    element: document.createElement("div"),
    extensions: [StarterKit, SignatureBlock, QuotedBlock],
    content,
  });
  editors.push(e);
  return e;
}

afterEach(() => {
  while (editors.length) editors.pop()?.destroy();
});

const BODY =
  "<p>Hi Sam,</p><p>Both CRM asks are live. It comes to $20 a month, flat.</p>" +
  "<p>Here is the link to set it up.</p>";

describe("findTextRange", () => {
  it("finds a phrase inside one text node", () => {
    const e = makeEditor(BODY);
    const r = findTextRange(e, "Hi Sam,")!;
    expect(r).not.toBeNull();
    expect(e.state.doc.textBetween(r.from, r.to)).toBe("Hi Sam,");
  });

  it("returns null for text that isn't there", () => {
    expect(findTextRange(makeEditor(BODY), "Dear Sarah")).toBeNull();
  });

  it("returns null for an empty needle rather than matching everything", () => {
    expect(findTextRange(makeEditor(BODY), "   ")).toBeNull();
  });

  /**
   * A quote spanning a mark boundary is the case the naive single-node search
   * misses — any bold or link inside the phrase splits the text run.
   */
  it("falls back to the block when marks split the phrase", () => {
    const e = makeEditor("<p>It comes to <strong>$20 a month</strong>, flat.</p>");
    const r = findTextRange(e, "It comes to $20 a month, flat.")!;
    expect(r).not.toBeNull();
    expect(e.state.doc.textBetween(r.from, r.to)).toBe("It comes to $20 a month, flat.");
  });

  it("matches across different whitespace than the model echoed", () => {
    const e = makeEditor(BODY);
    expect(findTextRange(e, "Both CRM asks   are\nlive.")).not.toBeNull();
  });

  it("resolves to a leaf block, never the whole document", () => {
    const e = makeEditor(BODY);
    const r = findTextRange(e, "Here is  the link to set it up.")!;
    expect(e.state.doc.textBetween(r.from, r.to)).toBe("Here is the link to set it up.");
  });
});

describe("replaceText", () => {
  it("swaps the phrase and leaves the rest alone", () => {
    const e = makeEditor(BODY);
    expect(replaceText(e, "Hi Sam,", "Bro. Ellis,")).toBe(true);
    expect(e.getText()).toContain("Bro. Ellis,");
    expect(e.getText()).not.toContain("Hi Sam,");
    expect(e.getText()).toContain("Here is the link to set it up.");
  });

  /**
   * The user may have edited the draft between the review running and clicking
   * Apply. Returning false is what lets the panel say so instead of showing a
   * button that silently did nothing.
   */
  it("reports failure when the text is gone", () => {
    expect(replaceText(makeEditor(BODY), "Hi Sarah,", "Bro. Ellis,")).toBe(false);
  });

  it("leaves the document untouched on a failed replace", () => {
    const e = makeEditor(BODY);
    const before = e.getHTML();
    replaceText(e, "not present", "x");
    expect(e.getHTML()).toBe(before);
  });

  it("preserves surrounding formatting when replacing inside a block", () => {
    const e = makeEditor("<p>Costs <strong>$20</strong> a month, flat.</p>");
    expect(replaceText(e, "flat.", "flat, billed monthly.")).toBe(true);
    expect(e.getHTML()).toContain("<strong>$20</strong>");
    expect(e.getText()).toContain("billed monthly.");
  });
});

describe("bodyEndPos / appendParagraph", () => {
  // The real signature shape — `data-cx-signature` + `email-signature`, per
  // SignatureBlock's marker/className in composeNodes.
  const SIG =
    '<div data-cx-signature="1" class="email-signature"><p>— Chris</p></div>';

  it("is the document end when there is no signature", () => {
    const e = makeEditor(BODY);
    expect(bodyEndPos(e)).toBe(e.state.doc.content.size);
  });

  /**
   * The whole reason this function exists: appending at `doc.content.size`
   * would put the new line BELOW the sign-off.
   */
  it("stops before a signature block", () => {
    const e = makeEditor(BODY + SIG);
    expect(bodyEndPos(e)).toBeLessThan(e.state.doc.content.size);
  });

  it("appends above the signature, not after it", () => {
    const e = makeEditor(BODY + SIG);
    appendParagraph(e, "Let me know if the 1st works.");

    // Asserted on node order, not on getText(): the signature is an ATOM whose
    // content lives in an attribute, so its text never appears in getText() at
    // all (pinned by the next test).
    const types: string[] = [];
    let added = -1;
    e.state.doc.content.forEach((node, _offset, i) => {
      types.push(node.type.name);
      if (node.textContent.includes("Let me know if the 1st works.")) added = i;
    });
    const sig = types.indexOf("signatureBlock");
    expect(sig).toBeGreaterThan(-1);
    expect(added).toBeGreaterThan(-1);
    expect(added).toBeLessThan(sig);
  });

  /**
   * `getDraftSnapshot` feeds `editor.getText()` to the validator, so this is a
   * load-bearing contract: a placeholder or a stale name inside the signature
   * or the quoted history must NOT be reported as a finding — that would fire
   * on every draft the user ever writes.
   */
  it("getText() excludes signature and quoted-history content", () => {
    const e = makeEditor(
      BODY +
        SIG +
        '<blockquote data-cx-quote="1" class="cx-quote"><p>Hi [Name], older thread</p></blockquote>',
    );
    const text = e.getText();
    expect(text).toContain("Both CRM asks are live.");
    expect(text).not.toContain("— Chris");
    expect(text).not.toContain("[Name]");
  });

  it("appends to the end when there is no signature", () => {
    const e = makeEditor(BODY);
    appendParagraph(e, "Let me know if the 1st works.");
    expect(e.getText().trimEnd().endsWith("Let me know if the 1st works.")).toBe(true);
  });
});
