---
description: Compose a new email via CXMail (draft only, does not send)
argument-hint: <to> — <notes/bullets>
---

Draft an email based on the notes in: $ARGUMENTS

Steps:
1. Read `~/.claude/cxmail/voice.md` with the Read tool. If it exists, use it to shape tone, greeting, signoff, and signature. If it does not exist, proceed without and infer tone from the request.
2. Parse `$ARGUMENTS` for the recipient. If no recipient is present, stop and ask before drafting.
3. If multiple sending accounts might apply and it's not obvious from context, call `mcp__cxmail__list_accounts` once to disambiguate; otherwise skip.
4. **Call `mcp__cxmail__list_voice_rules` with that recipient.** Whatever it returns is **absolute** — the user stated these explicitly ("always address him as Bro. Ellis", "never open with Hope you're well"). They outrank voice.md, the derived voice profile, and how the user's past mail to this person actually reads; a mixed history is exactly why the rule exists. Obey them literally in step 5.
5. Write subject and body from the notes. Keep it tight — no filler, no throat-clearing.
6. Call `mcp__cxmail__compose_draft` with `to`, `subject`, and `body`. If its result line carries a **⚠ PINNED RULES** note, check the draft you just wrote against each rule and fix any breach with `mcp__cxmail__edit_draft` before showing it to the user.
7. Print the draft back to the user (to / subject / body), noting the UID from the result line, and stop.

**If the user states a new standing preference** — anything phrased as *always*, *never*, *from now on*, or a correction to how you addressed someone — record it with `mcp__cxmail__set_voice_rule` (pass `recipient_email` for one person, omit it for every message from that account) so the next session honors it too. Do not put it in voice.md; a pinned rule is what survives a voice-profile rebuild.

## Revisions

If the user asks for a change to a draft you already created — a reworded line, an added paragraph, a different subject — **revise that draft; never start a new one.** A second `compose_draft` leaves the first draft sitting in the folder as a duplicate.

1. Re-read the draft **as it stands now**: `mcp__cxmail__preview_email_html` for designed HTML, `mcp__cxmail__read_email` for ordinary prose. Do this even when you remember what you wrote — the user may have edited it in the compose window in the meantime, and rebuilding from your own last body throws their edits away without a trace.
2. Apply the requested change to *that* text.
3. Call `mcp__cxmail__edit_draft` with the draft's current UID.
4. Note the **new** UID from the result line (`Draft UID N replaced with UID M`) — it changes on every edit.

**If the UID is not found**, it went stale: `edit_draft` deletes and re-appends, and the user saving in compose does the same. Recover with `mcp__cxmail__search_emails` on the subject, take the newest Drafts UID, and confirm it with `preview_email_html` — a stale local search row fails that fetch, which is how you tell a live draft from an expunged one.

Do NOT call `mcp__cxmail__send_email`. Sending is a separate step via `/cxmail:send`.
