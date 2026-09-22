---
description: Summarize unread inbox and suggest actions (read-only)
---

Triage the inbox. No destructive actions — output a table, wait for instructions.

Steps:
1. Call `mcp__cxmail__search_emails` for unread messages in the inbox from the last 7 days. Cap results at 30.
2. Bucket each message into: `action` (needs a reply or decision), `fyi` (informational, can be archived), or `promo` (newsletters, marketing, list-unsubscribe headers).
3. Print a compact markdown table with columns: `#`, `from`, `subject`, `date`, `bucket`, `suggested` (one of: reply / archive / flag / delete / ignore).
4. After the table, print a summary line: counts per bucket.
5. End with: "Tell me which rows to act on (e.g. 'archive 2,5,7' or 'reply to 3'). For the promo block, run `/cxmail:cleanup`."

Do NOT call archive, flag, delete, bulk_delete, or send. This command is read-only.
