import { describe, it, expect } from "vitest";
import { splitTrailingQuote } from "@/lib/quoteToggle";

describe("splitTrailingQuote", () => {
  it("returns input unchanged when there is no trailing quote", () => {
    const html = "<p>Hi there.</p><p>How are you?</p>";
    const result = splitTrailingQuote(html);
    expect(result.quotedHtml).toBeNull();
    expect(result.mainHtml).toBe(html);
  });

  it("splits a trailing blockquote", () => {
    const html =
      "<p>Thanks for the intro, David.</p>" +
      "<p>Nice to meet you, Chris.</p>" +
      "<blockquote>Original message from David</blockquote>";
    const result = splitTrailingQuote(html);
    expect(result.quotedHtml).toContain("<blockquote>");
    expect(result.mainHtml).toContain("Thanks for the intro");
    expect(result.mainHtml).not.toContain("blockquote");
  });

  it("splits a Gmail-style quoted block", () => {
    const html =
      '<p>Reply body</p><div class="gmail_quote"><blockquote>quoted</blockquote></div>';
    const result = splitTrailingQuote(html);
    expect(result.quotedHtml).toContain("gmail_quote");
    expect(result.mainHtml).toBe("<p>Reply body</p>");
  });

  it("splits an Apple Mail / Outlook 'On … wrote:' lead-in", () => {
    const html =
      "<p>My reply.</p>" +
      "<div>On Wed, Apr 30, 2026, David Condo wrote:</div>" +
      "<blockquote>Original</blockquote>";
    const result = splitTrailingQuote(html);
    expect(result.quotedHtml).toContain("David Condo wrote");
    expect(result.quotedHtml).toContain("blockquote");
    expect(result.mainHtml).toBe("<p>My reply.</p>");
  });

  it("does not split when stripping the quote leaves the body empty", () => {
    // A message that is ONLY a quoted block (forwarded headers, etc.) —
    // the user expects to see it as-is, not behind a "•••" toggle.
    const html = "<blockquote>Only the quote, nothing else.</blockquote>";
    const result = splitTrailingQuote(html);
    expect(result.quotedHtml).toBeNull();
    expect(result.mainHtml).toBe(html);
  });

  it("returns null on empty input", () => {
    const result = splitTrailingQuote("");
    expect(result.quotedHtml).toBeNull();
    expect(result.mainHtml).toBe("");
  });

  it("captures everything from the lead-in onward, including following siblings", () => {
    const html =
      "<p>Reply.</p>" +
      "<div>On Mon, May 4, 2026, John wrote:</div>" +
      "<p>quoted line one</p>" +
      "<p>quoted line two</p>";
    const result = splitTrailingQuote(html);
    expect(result.mainHtml).toBe("<p>Reply.</p>");
    expect(result.quotedHtml).toContain("John wrote");
    expect(result.quotedHtml).toContain("quoted line one");
    expect(result.quotedHtml).toContain("quoted line two");
  });
});
