---
description: Draft a reply to an existing thread (does not send)
argument-hint: <thread-id or search query> [— <notes>]
---

Draft a reply. Input: $ARGUMENTS

Steps:
1. Split `$ARGUMENTS` on ` — ` (em dash surrounded by spaces) into target and optional notes. If no em dash, treat the whole string as the target.
2. Resolve the target to a thread:
   - If it looks like an id (short, no spaces, alphanumeric), use it directly.
   - Otherwise call `mcp__cxmail__search_emails` with the target as the query. If more than one plausible match, list the top 3 and ask which one.
3. Call `mcp__cxmail__read_thread` on the resolved thread id to load full context. Write down the `account_id` and — for the **latest** message in the thread — its `Folder:` and `UID:` exactly as printed. **These two are the reply target you pass in step 7.** (`read_thread`, `read_email`, and `search_emails` all print Folder and UID together.)
4. **Recipient integrity — do this before drafting. This is the step that prevents silently dropping a recipient (a real reply once went out missing a second To recipient).** `compose_draft` does NOT infer recipients from the thread — you must pass them explicitly, so build the set yourself:
   - Call `mcp__cxmail__read_email` on the latest message and read its `From`, `To`, and `Cc` lines (read_email now lists every recipient, not just the first).
   - **Reply-all (default whenever the original had more than one recipient):** To = the original `From` plus every address on the original `To`; Cc = every address on the original `Cc`. Then remove the user's own account address and its send-as aliases (get them from `mcp__cxmail__list_accounts`) from both, dedupe case-insensitively, and drop from Cc anyone already in To.
   - **Reply to sender only:** To = the original `From`, no Cc. Use this only when the notes say to reply to just the sender, or the original had a single recipient.
   - If you deliberately leave any address that was on the original To/Cc off the reply, call it out explicitly in step 8.
5. Read `~/.claude/cxmail/voice.md` if it exists, then **call `mcp__cxmail__list_voice_rules` with the person you are replying to** (the original `From`). Whatever it returns is **absolute** — the user stated these explicitly ("always address him as Bro. Ellis", "never open with Hope you're well"), and they outrank voice.md, the derived voice profile, and how past replies in this very thread read. A mixed history is exactly why the rule exists.
6. Draft a reply body that addresses the latest message and incorporates the notes (if any). Match the tone of voice.md, subject to the pinned rules from step 5. The body must contain **only the new reply prose** — `compose_draft` automatically appends the quoted original message *below the signature* (Gmail-style), so never paste thread history into the body yourself. Pasting it puts the user's signature **underneath** the quote, which is wrong.
7. Call `mcp__cxmail__compose_draft` with:
   - `reply_to_folder` and `reply_to_uid` = the Folder and UID you noted in step 3. **This is the reply target.** Pass `reply_to_message_id` only if you somehow have an ID but no Folder/UID; it is the fallback path.
   - the explicit `to` / `cc` (and `bcc` if the user asked) arrays you built in step 4. Never rely on the tool to fill in recipients.
   - A successful reply's result line ends with `+ quoted original`. If it doesn't, the quote wasn't attached — go to step 7a.
   - If the result line carries a **⚠ PINNED RULES** note, check your reply against each rule and fix any breach with `mcp__cxmail__edit_draft` before step 8.
7a. **If `compose_draft` returns an error starting `QUOTED HISTORY UNAVAILABLE`**, no draft was created and nothing was destroyed. Do exactly what the error says, in this order — and **never** work around it by pasting the quote into `body`:
   - *"not in the local cache"* / *"is not a folder on this account"* → the target is stale or misspelled. Re-run `mcp__cxmail__search_emails` (or `mcp__cxmail__list_folders` for exact folder names) to get current `Folder:` / `UID:`, then retry step 7 with those. UIDs are folder-scoped and change when a message is moved.
   - *IMAP connect / SELECT / fetch failed* → transient or the message moved. Retry once; if it fails again, report the error text to the user and stop without creating a draft.
   - *"no renderable text or HTML body to quote"* → the original genuinely has nothing to quote. Retry with `quote_original=false` and say so in step 8.
8. Print the draft back, and include a **Recipients** line listing the full To / Cc / Bcc you passed plus any original recipient you intentionally excluded. If you had to fall back to `quote_original=false`, say so. Then stop.

## Revisions

If the user asks for a change to the reply you just drafted, **revise that draft — do not draft a second one.** Calling `compose_draft` again leaves the first reply in the folder as a duplicate, and it re-runs the whole recipient build in step 4, which is where a recipient gets dropped.

1. Re-read the draft as it stands now (`mcp__cxmail__preview_email_html` for designed HTML, `mcp__cxmail__read_email` otherwise) and build the change on top of that text — the user may have edited it in the compose window since you wrote it.
2. Call `mcp__cxmail__edit_draft` with the draft's current UID, passing the **full** `to` / `cc` / `bcc` arrays from step 4 again (edit_draft replaces the draft, so an omitted list is a dropped recipient) and the same `reply_to_folder` / `reply_to_uid` so the threading and quote survive.
3. Note the new UID from the result line — it changes on every edit. If a UID is not found it went stale; recover it via `search_emails` on the subject, newest Drafts UID, confirmed with `preview_email_html`.

**If the user states a new standing preference** — anything phrased as *always*, *never*, *from now on*, or a correction to how you addressed someone — record it with `mcp__cxmail__set_voice_rule` (pass `recipient_email` for one person, omit it for every message from that account) so the next session honors it too. Do not put it in voice.md; a pinned rule is what survives a voice-profile rebuild.

Do NOT send. Sending is `/cxmail:send`.
