import { describe, expect, it } from "vitest";
import { parseDraftLink } from "@/lib/draftLink";

describe("parseDraftLink", () => {
  it("reads the draft_id form", () => {
    expect(parseDraftLink("cxmail://draft/0278e6ec-4dd4-4661-bd7e-b1cf34edb637")).toEqual({
      draftId: "0278e6ec-4dd4-4661-bd7e-b1cf34edb637",
    });
  });

  it("reads the account + uid form", () => {
    expect(parseDraftLink("cxmail://draft/acct-1/1067")).toEqual({ accountId: "acct-1", uid: 1067 });
  });

  it("tolerates a trailing slash, a query and an upper-case scheme", () => {
    expect(parseDraftLink("CXMAIL://draft/abc/")).toEqual({ draftId: "abc" });
    expect(parseDraftLink("cxmail://draft/abc?x=1")).toEqual({ draftId: "abc" });
  });

  it("decodes percent-encoding", () => {
    expect(parseDraftLink("cxmail://draft/a%20b")).toEqual({ draftId: "a b" });
  });

  it("rejects anything else", () => {
    for (const bad of [
      "mailto:a@b.example",
      "cxmail://message/abc",
      "cxmail://draft/",
      "cxmail://draft/acct/0",
      "cxmail://draft/acct/12x",
      "cxmail://draft/acct/99999999999",
      "cxmail://draft/a/1/2",
      "cxmail://draft/%E0%A4%A",
      `cxmail://draft/${"a".repeat(129)}`,
    ]) {
      expect(parseDraftLink(bad), bad).toBeNull();
    }
  });
});
