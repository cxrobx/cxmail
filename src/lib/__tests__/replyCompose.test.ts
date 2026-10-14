import { buildReplyComposeProps } from "@/lib/replyCompose";
import type { MessageDetail } from "@/types/email";

const detail = {
  uid: 7, subject: "Plans", from_email: "dana@example.com", from_name: "Dana",
  to_list: [{ name: null, email: "me@example.com" }, { name: null, email: "sam@example.com" }],
  cc_list: [{ name: null, email: "ME@example.com " }, { name: null, email: "lee@example.com" }],
  message_id: "<m2@example.com>", references: "<m1@example.com>",
  sanitized_html: "<p>Hi</p>", plain_text: "Hi", date: "2026-10-13T00:00:00Z",
} as unknown as MessageDetail;
const target = { accountId: "acct", folder: "INBOX", uid: 7, accountEmail: "me@example.com", aliasEmails: [] };

it("carries every threading field a reply needs (#39)", () => {
  const props = buildReplyComposeProps(detail, "reply", target, "Shorter version");
  expect(props).toMatchObject({
    mode: "reply", accountId: "acct", defaultSubject: "Re: Plans", defaultTo: "dana@example.com",
    inReplyTo: "<m2@example.com>", referencesHeader: "<m1@example.com> <m2@example.com>",
    replyContext: { accountId: "acct", folder: "INBOX", uid: 7 },
  });
  expect(props.defaultBody).toContain("Shorter version");
  expect(props.quotedHtml).toBeTruthy();
});

it("reply-all leaves the user's own addresses and the sender out of Cc", () => {
  expect(buildReplyComposeProps(detail, "reply-all", target).defaultCc).toEqual(["sam@example.com", "lee@example.com"]);
});

it("does not double a Re: prefix and omits replyContext without full coordinates", () => {
  const props = buildReplyComposeProps({ ...detail, subject: "Re: Plans" }, "reply", { accountId: "acct", folder: null, uid: 7 });
  expect(props.defaultSubject).toBe("Re: Plans");
  expect(props.replyContext).toBeUndefined();
});
