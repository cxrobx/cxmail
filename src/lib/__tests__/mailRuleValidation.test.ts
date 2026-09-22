import { describe, it, expect } from "vitest";
import {
  mailRuleWarnings,
  validateMailRule,
} from "@/lib/mailRuleValidation";
import type { MailRule } from "@/types/email";

function rule(overrides: Partial<MailRule> = {}): MailRule {
  return {
    id: null,
    account_id: null,
    name: "Block crypto spam",
    is_active: true,
    priority: 0,
    conditions: [{ field: "from", operator: "contains", value: "crypto" }],
    actions: [{ action_type: "set_category", value: "junk" }],
    ...overrides,
  };
}

describe("validateMailRule", () => {
  it("accepts a well-formed rule", () => {
    expect(validateMailRule(rule())).toEqual([]);
  });

  it("accepts a `to` condition — T17 made it live", () => {
    expect(
      validateMailRule(
        rule({ conditions: [{ field: "to", operator: "equals", value: "me@x.io" }] })
      )
    ).toEqual([]);
  });

  it("refuses a blank condition value, naming the whole-mailbox consequence", () => {
    const errors = validateMailRule(
      rule({ conditions: [{ field: "from", operator: "contains", value: "" }] })
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/matches every message/i);
    expect(errors[0]).toMatch(/whole mailbox/i);
  });

  it("treats a whitespace-only value as blank", () => {
    expect(
      validateMailRule(
        rule({ conditions: [{ field: "subject", operator: "contains", value: "   " }] })
      )
    ).not.toEqual([]);
  });

  it("refuses a rule with zero conditions", () => {
    const errors = validateMailRule(rule({ conditions: [] }));
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/every message/i);
  });

  it("refuses a body condition and says why it can never match", () => {
    const errors = validateMailRule(
      rule({ conditions: [{ field: "body", operator: "contains", value: "invoice" }] })
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/headers/i);
    expect(errors[0]).toMatch(/subject/i);
  });

  it("refuses an unknown field or operator", () => {
    expect(
      validateMailRule(
        rule({ conditions: [{ field: "cc", operator: "contains", value: "x" }] })
      )
    ).toHaveLength(1);
    expect(
      validateMailRule(
        rule({ conditions: [{ field: "from", operator: "matches", value: "x" }] })
      )
    ).toHaveLength(1);
  });

  it("refuses move/delete actions — stored but never executed", () => {
    const errors = validateMailRule(
      rule({ actions: [{ action_type: "move", value: "Archive" }] })
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/never executed/i);
  });

  it("refuses zero actions and an unknown category", () => {
    expect(validateMailRule(rule({ actions: [] }))).toHaveLength(1);
    expect(
      validateMailRule(
        rule({ actions: [{ action_type: "set_category", value: "newsletters" }] })
      )
    ).toHaveLength(1);
  });

  it("requires a name", () => {
    expect(validateMailRule(rule({ name: "  " }))).toHaveLength(1);
  });

  it("reports every problem at once, not just the first", () => {
    expect(
      validateMailRule(
        rule({
          name: "",
          conditions: [{ field: "body", operator: "contains", value: "" }],
          actions: [{ action_type: "delete", value: null }],
        })
      ).length
    ).toBeGreaterThan(3);
  });
});

describe("mailRuleWarnings", () => {
  it("is silent on a healthy rule", () => {
    expect(mailRuleWarnings(rule())).toEqual([]);
  });

  it("flags a stored body condition", () => {
    expect(
      mailRuleWarnings(
        rule({ conditions: [{ field: "body", operator: "contains", value: "x" }] })
      )[0]
    ).toMatch(/never match/i);
  });

  it("flags a conditionless rule as matching everything", () => {
    expect(mailRuleWarnings(rule({ conditions: [] }))[0]).toMatch(/EVERY message/);
  });

  it("flags a blank stored value", () => {
    expect(
      mailRuleWarnings(
        rule({ conditions: [{ field: "from", operator: "contains", value: "" }] })
      )[0]
    ).toMatch(/every message/i);
  });

  it("flags dead move/delete actions", () => {
    expect(
      mailRuleWarnings(rule({ actions: [{ action_type: "delete", value: null }] }))[0]
    ).toMatch(/never executed/i);
  });
});
