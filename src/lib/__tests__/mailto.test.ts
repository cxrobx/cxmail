import { describe, it, expect } from "vitest";
import { parseMailto, plainBodyToComposeHtml } from "@/lib/mailto";

describe("parseMailto (RFC 6068)", () => {
  it("parses a single path recipient", () => {
    expect(parseMailto("mailto:alice@example.com").to).toEqual(["alice@example.com"]);
  });

  it("comma-splits multiple path recipients", () => {
    expect(parseMailto("mailto:a@x.com,b@y.com").to).toEqual(["a@x.com", "b@y.com"]);
  });

  it("parses to/cc/bcc/subject/body query params with %-decoding", () => {
    const r = parseMailto(
      "mailto:a@b.com?cc=c@d.com,e@f.com&bcc=s@t.com&subject=Hi%20there&body=Line1%0ALine2",
    );
    expect(r.to).toEqual(["a@b.com"]);
    expect(r.cc).toEqual(["c@d.com", "e@f.com"]);
    expect(r.bcc).toEqual(["s@t.com"]);
    expect(r.subject).toBe("Hi there");
    expect(r.body).toBe("Line1\nLine2");
  });

  it("merges path + ?to= recipients", () => {
    expect(parseMailto("mailto:a@b.com?to=c@d.com").to).toEqual(["a@b.com", "c@d.com"]);
  });

  it("treats '+' as a literal plus (not a space) per RFC 6068", () => {
    expect(parseMailto("mailto:a@b.com?subject=one+two").subject).toBe("one+two");
  });

  it("decodes encoded reserved chars inside body", () => {
    expect(parseMailto("mailto:a@b.com?body=a%20%26%20b%20%3D%20c").body).toBe("a & b = c");
  });

  it("handles an empty mailto and a bare scheme", () => {
    expect(parseMailto("mailto:")).toEqual({ to: [], cc: [], bcc: [] });
    expect(parseMailto("")).toEqual({ to: [], cc: [], bcc: [] });
  });

  it("is case-insensitive on the scheme", () => {
    expect(parseMailto("MAILTO:a@b.com").to).toEqual(["a@b.com"]);
  });

  it("falls back to the raw value on malformed percent-encoding", () => {
    // decodeURIComponent throws on a lone %ZZ — parser must not throw.
    expect(() => parseMailto("mailto:a@b.com?subject=%ZZ")).not.toThrow();
  });
});

describe("plainBodyToComposeHtml (XSS hardening)", () => {
  it("escapes HTML metacharacters so markup is rendered as literal text", () => {
    const out = plainBodyToComposeHtml('<div class="cx-html-block"><img src=x onerror=alert(1)></div>');
    // No live tags survive — every angle bracket is entity-escaped.
    expect(out).not.toMatch(/<img/i);
    expect(out).not.toMatch(/<div/i);
    expect(out).toContain("&lt;img");
    expect(out).toContain("&lt;div");
  });

  it("escapes ampersands before other entities (no double-encoding artifacts)", () => {
    expect(plainBodyToComposeHtml("a & b")).toBe("<p>a &amp; b</p>");
  });

  it("preserves newlines as <br> and normalizes CRLF", () => {
    expect(plainBodyToComposeHtml("Line1\r\nLine2")).toBe("<p>Line1<br>Line2</p>");
  });

  it("wraps a plain single-line body in a paragraph", () => {
    expect(plainBodyToComposeHtml("hello")).toBe("<p>hello</p>");
  });

  it("end-to-end: a hostile mailto body cannot inject a node into the editor HTML", () => {
    const parsed = parseMailto(
      "mailto:v@x.com?body=%3Cdiv%20class%3D%22cx-html-block%22%3E%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E%3C%2Fdiv%3E",
    );
    // parseMailto returns the decoded *plain text* (this is the dangerous string)...
    expect(parsed.body).toContain("<img");
    // ...but the value handed to ComposeModal's defaultBody is escaped.
    const html = plainBodyToComposeHtml(parsed.body!);
    expect(html).not.toMatch(/<img/i);
    expect(html).toContain("&lt;img");
  });
});
