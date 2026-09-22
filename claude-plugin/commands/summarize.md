---
description: Summarize an email thread
argument-hint: <thread-id or search query>
---

Summarize the thread matching: $ARGUMENTS

Steps:
1. If `$ARGUMENTS` is empty, stop and ask.
2. Resolve to a thread id:
   - If it looks like an id, use it.
   - Otherwise `mcp__cxmail__search_emails` with the query. If more than one plausible match, list top 3 and ask.
3. Call `mcp__cxmail__read_thread` on the resolved id.
4. Produce:
   - **Participants:** comma-separated list
   - **3-bullet summary** of what happened in the thread (not per-message; synthesized)
   - **Open questions** (if any)
   - **Next action** (one line, or "none — thread resolved")

Read-only. Do not draft, reply, or modify.
