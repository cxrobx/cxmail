import { describe, it, expect } from "vitest";
import {
  validateDraft,
  isPlausibleAddress,
  findPlaceholders,
  findDeadLinks,
  parseGreeting,
  greetingMatchesRecipient,
  hasReplyPrefix,
  attachmentMentionSentence,
  countBySeverity,
  ATTACHMENT_MENTION_RE,
  type DraftSnapshot,
} from "@/lib/draftValidation";
import { detectAttachmentMention } from "@/lib/utils";

function snap(overrides: Partial<DraftSnapshot> = {}): DraftSnapshot {
  return {
    subject: "CRM updates are live",
    to: [{ name: "Sam Ellis", email: "sam@harborline.example" }],
    cc: [],
    bcc: [],
    bodyText: "Hi Sam,\n\nBoth of Skyla's CRM asks are live.",
    bodyHtml: "<p>Hi Sam,</p><p>Both of Skyla's CRM asks are live.</p>",
    attachmentCount: 0,
    inReplyTo: null,
    ...overrides,
  };
}

const ids = (d: DraftSnapshot) => validateDraft(d).map((f) => f.id);

describe("a clean draft", () => {
  it("produces no findings", () => {
    expect(validateDraft(snap())).toEqual([]);
  });
});

describe("addresses", () => {
  it.each([
    ["sam@harborline.example", true],
    ["first.last+tag@sub.domain.co.uk", true],
    ["sam@harborline", false],
    ["ericharborline.example", false],
    ["sam @harborline.example", false],
    ["", false],
  ])("%s → %s", (email, ok) => {
    expect(isPlausibleAddress(email)).toBe(ok);
  });

  it("flags an empty To field", () => {
    expect(ids(snap({ to: [] }))).toContain("no-recipients");
  });

  it("flags a malformed address", () => {
    const found = validateDraft(snap({ to: [{ name: null, email: "sam@harborline" }] }));
    expect(found.map((f) => f.id)).toContain("bad-address:sam@harborline");
    expect(found[0].severity).toBe("error");
  });

  it("flags the same address in To and Cc", () => {
    const d = snap({
      to: [{ name: null, email: "sam@harborline.example" }],
      cc: [{ name: null, email: "Sam@Harborline.example" }],
    });
    expect(ids(d)).toContain("duplicate:sam@harborline.example");
  });

  it("does not flag two different addresses", () => {
    const d = snap({
      to: [{ name: null, email: "sam@harborline.example" }],
      cc: [{ name: null, email: "dana@northwind.example" }],
    });
    expect(ids(d).filter((i) => i.startsWith("duplicate:"))).toEqual([]);
  });
});

describe("subject", () => {
  it("flags an empty subject and offers to suggest one", () => {
    const f = validateDraft(snap({ subject: "  " })).find((x) => x.id === "empty-subject")!;
    expect(f).toBeDefined();
    expect(f.action?.kind).toBe("ask");
  });

  it("offers no suggestion when there is no body to summarize", () => {
    const f = validateDraft(snap({ subject: "", bodyText: "", bodyHtml: "" })).find(
      (x) => x.id === "empty-subject",
    )!;
    expect(f.action).toBeNull();
  });

  it.each([
    ["Re: budget", true],
    ["RE: budget", true],
    ["re[2]: budget", true],
    ["AW: budget", true],
    ["Retro notes", false],
    ["Rebuilding the CRM", false],
  ])("reply prefix %s → %s", (subject, expected) => {
    expect(hasReplyPrefix(subject)).toBe(expected);
  });

  it("flags a Re: subject with no threading target", () => {
    expect(ids(snap({ subject: "Re: hosting fee" }))).toContain("reply-no-target");
  });

  it("stays quiet when the reply IS threaded", () => {
    const d = snap({ subject: "Re: hosting fee", inReplyTo: "<parent@x>" });
    expect(ids(d)).not.toContain("reply-no-target");
  });
});

describe("placeholders", () => {
  it.each([
    ["Hi [Name], welcome", ["[Name]"]],
    ["Hi [First Name], welcome", ["[First Name]"]],
    ["Due {{date}} please", ["{{date}}"]],
    ["Send by <INSERT DATE> ok", ["<INSERT DATE>"]],
    ["TODO: follow up", ["TODO"]],
    ["Ship TK before Friday", ["TK"]],
    // Lowercase, but a keyword rescues it.
    ["Send by [insert date here] please", ["[insert date here]"]],
    // Capitalised with NO keyword — only the capitalisation signal catches
    // these, so they're what pins that half of looksLikeSlot.
    ["Contract with [Vendor] pending", ["[Vendor]"]],
    ["Rolling out in [Region] next", ["[Region]"]],
  ])("finds a placeholder in %s", (text, expected) => {
    expect(findPlaceholders(text)).toEqual(expected);
  });

  it.each([
    // Short enough to reach the word-count guard — this is what proves the
    // guard works, rather than the length cap silently doing the job.
    "The invoice [see the note above] is attached",
    "Pricing [as we discussed] holds",
    "The quote [sic] was theirs",
    "It reads better [emphasis mine] that way",
    "The invoice [see the breakdown I sent last week] is attached",
    "we talked about it (tk was the old spelling)",
    "the stock ticker fell",
    "a > b and c < d",
  ])("does not fire on prose: %s", (text) => {
    expect(findPlaceholders(text)).toEqual([]);
  });

  it("reports each distinct placeholder once", () => {
    expect(findPlaceholders("Hi [Name], thanks [Name]")).toEqual(["[Name]"]);
  });

  it("is stateless across calls despite module-level /g regexes", () => {
    const text = "Hi [Name]";
    expect(findPlaceholders(text)).toEqual(findPlaceholders(text));
  });
});

describe("links", () => {
  it.each([
    ['<a href="#">Here is the link</a>', "Here is the link"],
    ['<a href="">click</a>', "click"],
    ['<a href="https://example.com">demo</a>', "demo"],
    ["<a>bare anchor</a>", "bare anchor"],
  ])("flags %s", (html, text) => {
    expect(findDeadLinks(html)).toEqual([expect.objectContaining({ text })]);
  });

  it("leaves real links alone", () => {
    expect(findDeadLinks('<a href="https://stripe.com/pay/abc">set it up</a>')).toEqual([]);
  });

  it("surfaces the anchor text in the finding", () => {
    const d = snap({ bodyHtml: '<p><a href="#">Here is the link to set it up</a></p>' });
    const f = validateDraft(d).find((x) => x.id.startsWith("dead-link:"))!;
    expect(f.quote).toBe("Here is the link to set it up");
  });
});

describe("greeting", () => {
  it.each([
    ["Hi Sam,\n\nbody", "Sam"],
    ["Hey Bro. Ellis,\n\nbody", "Bro. Ellis"],
    ["Dear Dr. Watson,\n\nbody", "Dr. Watson"],
    ["Good morning Sarah,\n\nbody", "Sarah"],
    ["Dear J. R. Ewing,\n\nbody", "J. R. Ewing"],
    ["Hi Ana de la Cruz,\n\nbody", "Ana de la Cruz"],
    ["Hi,\n\nbody", ""],
    // Lowercase collective — not a person, must not be read as a name.
    ["Hi team,\n\nbody", ""],
  ])("parses %s", (body, name) => {
    expect(parseGreeting(body)?.name).toBe(name);
  });

  it("stops at a sentence period rather than eating the message", () => {
    // The counterpart to "Bro. Ellis": here the period IS the end.
    const g = parseGreeting("Hi Sam. Thanks for the quick turnaround.")!;
    expect(g.name).toBe("Sam");
    expect(g.line).toBe("Hi Sam.");
  });

  it("keeps the greeting line verbatim so the fix can find it", () => {
    expect(parseGreeting("Hey Bro. Ellis,\n\nbody")!.line).toBe("Hey Bro. Ellis");
  });

  it("returns null when there is no greeting", () => {
    expect(parseGreeting("Following up on the invoice.")).toBeNull();
  });

  const sam = [{ name: "Sam Ellis", email: "sam@harborline.example" }];

  it.each([
    ["Sam", true],
    ["Bro. Ellis", true],
    ["Ellis", true],
    ["Mr. Ellis", true],
    ["", true],
    ["Sarah", false],
    ["Dana", false],
  ])("greeting %s matches Sam Ellis → %s", (greet, expected) => {
    expect(greetingMatchesRecipient(greet, sam)).toBe(expected);
  });

  it("matches a first.last local part when there is no display name", () => {
    const r = [{ name: null, email: "sam.ellis@harborline.example" }];
    expect(greetingMatchesRecipient("Sam", r)).toBe(true);
    expect(greetingMatchesRecipient("Sarah", r)).toBe(false);
  });

  it("flags the mixup and offers the right name", () => {
    const f = validateDraft(snap({ bodyText: "Hi Sarah,\n\nBoth asks are live." })).find(
      (x) => x.id === "greeting-mismatch",
    )!;
    expect(f.quote).toBe("Hi Sarah");
    expect(f.action).toEqual(
      expect.objectContaining({ kind: "replace", find: "Hi Sarah", with: "Hi Sam" }),
    );
  });

  it("stays quiet with no recipient rather than guessing", () => {
    const d = snap({ to: [], bodyText: "Hi Sarah,\n\nhello" });
    expect(ids(d)).not.toContain("greeting-mismatch");
  });
});

describe("attachment mention", () => {
  it("fires when the body promises a file and none is attached", () => {
    const d = snap({ bodyText: "Hi Sam,\n\nI've attached the invoice breakdown. Thanks." });
    const f = validateDraft(d).find((x) => x.id === "attachment-mention")!;
    expect(f.quote).toBe("I've attached the invoice breakdown.");
  });

  it("stays quiet when a file IS attached", () => {
    const d = snap({
      bodyText: "I've attached the invoice breakdown.",
      attachmentCount: 1,
    });
    expect(ids(d)).not.toContain("attachment-mention");
  });

  it("quotes the sentence, not just the matched word", () => {
    const s = attachmentMentionSentence("First line. Please see the attached deck. Third.");
    expect(s).toBe("Please see the attached deck.");
  });

  it("stays byte-identical to the pre-send warning's matcher", () => {
    // Both must agree, or the panel and the send-time dialog disagree about
    // the same draft. This is the tripwire for that drift.
    const cases = [
      "I've attached the file",
      "please find attached",
      "see the enclosure",
      "the attachment is late",
      "we discussed the roof",
    ];
    for (const c of cases) {
      expect(ATTACHMENT_MENTION_RE.test(c)).toBe(detectAttachmentMention(c));
    }
  });
});

describe("ordering and counts", () => {
  it("puts errors before warnings before info", () => {
    const d = snap({
      to: [],
      subject: "Re: x",
      inReplyTo: null,
      bodyText: "Hi Sarah,\n\nI've attached it.",
    });
    const sev = validateDraft(d).map((f) => f.severity);
    expect(sev).toEqual([...sev].sort((a, b) => "ewi".indexOf(a[0]) - "ewi".indexOf(b[0])));
  });

  it("counts by severity", () => {
    const d = snap({ to: [], bodyText: "" });
    const c = countBySeverity(validateDraft(d));
    expect(c.error).toBeGreaterThanOrEqual(2);
  });
});
