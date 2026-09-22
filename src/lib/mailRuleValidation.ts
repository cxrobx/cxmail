/**
 * Mail rule validation — the UI half of the guard added at the MCP boundary
 * on 2026-08-03 (see gotcha #36).
 *
 * `db::rules::MailRule` is wider than the engine that runs it: `evaluate()`
 * resolves an unknown field to `""` and an unknown operator to `false`, so bad
 * input degrades to "never matches" rather than to an error. Two of those
 * traps are reachable from this UI, one of them destructive:
 *
 *  - `body` conditions can never match — rules run at classification time,
 *    against headers only.
 *  - a blank condition value, or a rule with no conditions at all, matches
 *    EVERY message (`"".contains("")` is true; `.all()` is vacuously true on an
 *    empty list) and applies its actions to the whole mailbox.
 *
 * These predicates deliberately mirror `validate_rule_conditions` /
 * `validate_rule_actions` / `mail_rule_warnings` in
 * `src-tauri/src/mcp/server.rs`. Keep the two boundaries in step — if the
 * allowed fields/operators/actions change there, change them here.
 */

import type { MailRule, RuleAction, RuleCondition } from "@/types/email";

export const RULE_FIELDS = ["from", "to", "subject"] as const;
export const RULE_OPERATORS = [
  "contains",
  "equals",
  "starts_with",
  "ends_with",
] as const;
export const RULE_CATEGORIES = [
  "primary",
  "updates",
  "social",
  "promotions",
  "junk",
] as const;
export const RULE_ACTIONS = [
  "set_category",
  "mark_read",
  "mark_flagged",
] as const;

const norm = (s: string | null | undefined) => (s ?? "").trim().toLowerCase();
const isIn = (haystack: readonly string[], v: string) => haystack.includes(v);

/**
 * Reasons a rule cannot be saved. Empty array = safe to save.
 * Messages name the consequence and the remedy, matching the MCP's tone.
 */
export function validateMailRule(rule: MailRule): string[] {
  const errors: string[] = [];

  if (!rule.name.trim()) {
    errors.push("Give the rule a name.");
  }

  errors.push(...validateRuleConditions(rule.conditions));
  errors.push(...validateRuleActions(rule.actions));

  return errors;
}

export function validateRuleConditions(conditions: RuleCondition[]): string[] {
  const errors: string[] = [];

  // `conditions.iter().all(..)` is vacuously true on an empty list, so a rule
  // with no conditions matches EVERY message. Never allow one.
  if (conditions.length === 0) {
    errors.push(
      "Add at least one condition. A rule with no conditions matches every message and would apply its actions to your whole mailbox."
    );
    return errors;
  }

  conditions.forEach((c, i) => {
    const at = `Condition ${i + 1}`;
    const field = norm(c.field);

    if (field === "body") {
      errors.push(
        `${at}: “body” can never match. Rules are evaluated at classification time, when only message headers are available — the rules engine is handed an empty body. Match on “subject” or “from” instead.`
      );
    } else if (!isIn(RULE_FIELDS, field)) {
      errors.push(
        `${at}: unknown field “${c.field}” never matches. Use From, To or Subject.`
      );
    }

    if (!isIn(RULE_OPERATORS, norm(c.operator))) {
      errors.push(
        `${at}: unknown operator “${c.operator}” never matches. Use contains, equals, starts with or ends with.`
      );
    }

    // An empty value makes contains / starts_with / ends_with match
    // everything — the same whole-mailbox hazard as an empty condition list.
    if (!c.value.trim()) {
      errors.push(
        `${at}: the value is empty, which matches every message — this rule would apply its actions to your whole mailbox. Type what to match on, or remove the condition.`
      );
    }
  });

  return errors;
}

export function validateRuleActions(actions: RuleAction[]): string[] {
  const errors: string[] = [];

  if (actions.length === 0) {
    errors.push("Add at least one action — a rule with none does nothing.");
    return errors;
  }

  actions.forEach((a, i) => {
    const at = `Action ${i + 1}`;
    const type = norm(a.action_type);

    if (type === "move" || type === "delete") {
      errors.push(
        `${at}: “${a.action_type}” is stored but NEVER executed — the classifier deliberately skips move/delete because they are destructive IMAP side-effects that don't belong on the sync path. Use “Move to Junk” (set_category) to route the mail out of the inbox instead.`
      );
      return;
    }

    if (!isIn(RULE_ACTIONS, type)) {
      errors.push(
        `${at}: unknown action “${a.action_type}” is ignored. Use set category, mark as read or star.`
      );
      return;
    }

    if (type === "set_category" && !isIn(RULE_CATEGORIES, norm(a.value))) {
      errors.push(
        `${at}: “${a.value}” is not a known category, so the message would be filed somewhere the UI never shows. Use primary, updates, social, promotions or junk.`
      );
    }
  });

  return errors;
}

/**
 * Non-blocking annotations for rules ALREADY stored — created before this
 * guard, or through some other writer. Mirrors `mail_rule_warnings()` in
 * `src-tauri/src/mcp/server.rs`, which does the same job for
 * `list_mail_rules` over the MCP.
 */
export function mailRuleWarnings(rule: MailRule): string[] {
  const warnings: string[] = [];

  if (rule.conditions.length === 0) {
    warnings.push(
      "Has no conditions — matches EVERY message and applies its actions to all mail."
    );
  }

  for (const c of rule.conditions) {
    const field = norm(c.field);
    if (field === "body") {
      warnings.push(
        `Condition on “body” (${c.operator} “${c.value}”) can never match: rules are evaluated against headers only.`
      );
    } else if (!isIn(RULE_FIELDS, field)) {
      warnings.push(`Unknown condition field “${c.field}” never matches.`);
    }

    if (!isIn(RULE_OPERATORS, norm(c.operator))) {
      warnings.push(`Unknown condition operator “${c.operator}” never matches.`);
    }

    if (!c.value.trim()) {
      warnings.push(
        `Condition on “${c.field}” has an empty value — it matches every message.`
      );
    }
  }

  for (const a of rule.actions) {
    const type = norm(a.action_type);
    if (type === "move" || type === "delete") {
      warnings.push(
        `Action “${a.action_type}” is never executed by the classifier — this part of the rule is dead.`
      );
    } else if (!isIn(RULE_ACTIONS, type)) {
      warnings.push(`Unknown action “${a.action_type}” is ignored.`);
    } else if (type === "set_category" && !isIn(RULE_CATEGORIES, norm(a.value))) {
      warnings.push(
        `set_category value “${a.value}” is not a known category; the message would be filed under a category the UI does not show.`
      );
    }
  }

  return warnings;
}
