import { describe, expect, it } from "vitest";
import { searchScopeIds, type SearchScopeView } from "@/lib/searchScope";
import type { Account } from "@/types/email";

function account(id: string, group: string | null, hidden = false): Account {
  return {
    id,
    email: `${id}@example.com`,
    display_name: null,
    provider: "gmail",
    imap_host: "imap.gmail.com",
    imap_port: 993,
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    imap_security: "implicit",
    smtp_security: "starttls",
    imap_username: null,
    smtp_username: null,
    color: null,
    is_active: true,
    sort_order: 0,
    group_name: group,
    notify_enabled: true,
    track_opens_enabled: false,
    hidden_from_aggregates: hidden,
    triage_enabled: false,
  };
}

const unified: SearchScopeView = {
  isUnifiedInbox: true,
  specialView: null,
  selectedGroupId: null,
  selectedAccountId: null,
  selectedAccountGroup: null,
};

const accounts = [
  account("acc-a", "BUSINESS"),
  account("acc-hidden", "BUSINESS", true),
  account("acc-c", null),
  account("acc-only-hidden", "WARMUP", true),
];

describe("searchScopeIds", () => {
  it("is unscoped (every visible account) in All Inboxes, special views and inbox groups", () => {
    expect(searchScopeIds(accounts, unified)).toBeUndefined();
    expect(
      searchScopeIds(accounts, { ...unified, isUnifiedInbox: false, specialView: "needs_you", selectedAccountId: "acc-hidden" }),
    ).toBeUndefined();
    expect(
      searchScopeIds(accounts, { ...unified, isUnifiedInbox: false, selectedGroupId: 3, selectedAccountId: "acc-hidden" }),
    ).toBeUndefined();
    // A visible single account keeps today's behaviour too.
    expect(
      searchScopeIds(accounts, { ...unified, isUnifiedInbox: false, selectedAccountId: "acc-a" }),
    ).toBeUndefined();
  });

  it("scopes an account folder to its visible members only", () => {
    expect(searchScopeIds(accounts, { ...unified, selectedAccountGroup: "BUSINESS" })).toEqual(["acc-a"]);
  });

  it("returns [] — not undefined — for a folder whose members are all hidden", () => {
    // `undefined` would mean every account; an empty folder must search nothing.
    expect(searchScopeIds(accounts, { ...unified, selectedAccountGroup: "WARMUP" })).toEqual([]);
  });

  it("names the hidden account when it is the one on screen — the only way its mail is searchable", () => {
    expect(
      searchScopeIds(accounts, { ...unified, isUnifiedInbox: false, selectedAccountId: "acc-hidden" }),
    ).toEqual(["acc-hidden"]);
  });
});
