---
description: Search CXMail and summarize results
argument-hint: <query>
---

Search the inbox for: $ARGUMENTS

Steps:
1. If `$ARGUMENTS` is empty, stop and ask for a query.
2. Call `mcp__cxmail__search_emails` with the query. Cap at 20 results.
3. Print a markdown table: `#`, `from`, `subject`, `date`, one-line snippet.
4. End with: "Want me to read #N? (`/cxmail:summarize` for the full thread)".

Read-only. Do not open, archive, or modify anything.
