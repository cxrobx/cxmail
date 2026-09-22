import { describe, it, expect } from "vitest";
import { buildDraftComposeProps } from "@/lib/draftCompose";
import type { MessageDetail } from "@/types/email";

function draft(overrides: Partial<MessageDetail> = {}): MessageDetail {
  return {
    uid: 619,
    subject: "Re: Northwind Company / CX Ventures/ Catalyst Partners Meeting Follow Up",
    from_name: "CX Ventures",
    from_email: "chris@cxventures.io",
    to_list: [{ name: null, email: "dana@northwind.example" }],
    cc_list: [{ name: null, email: "riley@northwind.example" }],
    bcc_list: [{ name: null, email: "sam@harborline.example" }],
    date: "2026-07-31T20:41:51+00:00",
    plain_text: "Hi Dana,",
    sanitized_html: "<p>Hi Dana,</p>",
    attachments: [],
    is_read: true,
    is_flagged: false,
    list_unsubscribe: null,
    list_unsubscribe_post: null,
    message_id: "<58927677-cfc5-4094-8983-d3b8e48cbdbf@cxmail.app>",
    references: "<root@x> <parent@y>",
    in_reply_to: "<parent@y>",
    ...overrides,
  };
}

describe("buildDraftComposeProps", () => {
  // THE regression. Reopening a draft rebuilds its MIME from these props, so a
  // missing In-Reply-To here is not a display bug — it is a reply that goes out
  // starting a brand new thread. Gmail's subject-based threading hid this for
  // three months; CXMail's own thread view did not (gotcha #39).
  it("round-trips the draft's own threading headers", () => {
    const props = buildDraftComposeProps(draft(), "acct-1", "[Gmail]/Drafts", 619, []);
    expect(props.inReplyTo).toBe("<parent@y>");
    expect(props.referencesHeader).toBe("<root@x> <parent@y>");
  });

  // A draft that genuinely isn't a reply must not invent headers — an
  // In-Reply-To pointing at nothing is worse than none at all.
  it("passes undefined when the draft carries no threading headers", () => {
    const props = buildDraftComposeProps(
      draft({ in_reply_to: null, references: null }),
      "acct-1",
      "[Gmail]/Drafts",
      619,
      [],
    );
    expect(props.inReplyTo).toBeUndefined();
    expect(props.referencesHeader).toBeUndefined();
  });

  // The other three fields that were each forgotten once before. Same failure
  // mode every time: absent from the props means deleted from the draft.
  it("carries every recipient field, not just the first To", () => {
    const props = buildDraftComposeProps(
      draft({
        to_list: [
          { name: null, email: "dana@northwind.example" },
          { name: null, email: "morgan@catalystpartners.example" },
        ],
      }),
      "acct-1",
      "[Gmail]/Drafts",
      619,
      [],
    );
    expect(props.defaultTo).toEqual([
      "dana@northwind.example",
      "morgan@catalystpartners.example",
    ]);
    expect(props.defaultCc).toEqual(["riley@northwind.example"]);
    expect(props.defaultBcc).toEqual(["sam@harborline.example"]);
  });

  it("preserves attachments and draft coordinates", () => {
    const attachments = [
      { filename: "proposal.pdf", content_type: "application/pdf", data_base64: "AAA=" },
    ];
    const props = buildDraftComposeProps(draft(), "acct-1", "[Gmail]/Drafts", 619, attachments);
    expect(props.defaultAttachments).toBe(attachments);
    expect(props.draftContext).toEqual({
      accountId: "acct-1",
      folder: "[Gmail]/Drafts",
      uid: 619,
    });
  });
});
