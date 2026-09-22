---
description: Find promotional/newsletter mail to bulk delete (requires explicit confirmation)
---

Find promotional/newsletter mail that's safe to bulk delete. This command is destructive — it MUST NOT delete without explicit user confirmation in the same turn.

Steps:
1. Call `mcp__cxmail__search_emails` for messages matching promo patterns: presence of `list-unsubscribe` header, sender domains typical of bulk mail (e.g. mailchimp, sendgrid, substack, marketing subdomains), age older than 30 days. Cap at 200.
2. Print:
   - **Total candidates:** <count>
   - **Top senders:** histogram table of the top 10 sending domains/addresses with per-sender counts
   - **Sample subjects:** 5 representative subject lines
3. Then STOP and ask the user explicitly:

   > Delete all <count> messages? Reply with "delete <count>" to confirm, or name senders to exclude.

4. Only if the user replies with a confirmation that matches the exact count (or a narrowed subset they specify), call `mcp__cxmail__bulk_delete_emails` with the selected ids.
5. Report what was deleted.

Rules:
- Never delete without a same-turn affirmative reply that includes the exact count or an explicit sender/subset selection. A prior blanket "yes you can clean up promos sometime" is NOT sufficient.
- If the user's reply is vague ("yeah go for it"), ask again with the exact count.
- Never expand scope beyond the candidate list shown.
