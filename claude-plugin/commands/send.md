---
description: Send an existing CXMail draft
argument-hint: <draft-id>
---

Send the draft identified by: $ARGUMENTS

Steps:
1. If `$ARGUMENTS` is empty, stop and ask which draft to send. Do NOT guess.
2. If `$ARGUMENTS` is ambiguous (looks like a query, not an id), stop and ask for the id.
3. **Verify recipients against the local DB before anything transmits — mandatory. This is the last line of defense against a recipient that was silently dropped while the draft was built.** The database is the source of truth for what will actually be sent. Read the draft's real recipient set from it. If you know the draft's `account_id`, Drafts folder, and UID:
   ```bash
   sqlite3 "$HOME/Library/Application Support/com.cxmail.app/cxmail.db" \
     "SELECT to_json, cc_json, bcc_json FROM message_bodies
      WHERE account_id='<account>' AND folder_name='<drafts-folder>' AND uid=<uid>;"
   ```
   (`<drafts-folder>` is `[Gmail]/Drafts` for Gmail accounts, `Drafts` otherwise.) If you don't have those identifiers, list recent drafts and their recipients so you can pick the right one:
   ```bash
   sqlite3 "$HOME/Library/Application Support/com.cxmail.app/cxmail.db" \
     "SELECT m.account_id, m.folder_name, m.uid, m.subject, b.to_json, b.cc_json, b.bcc_json
      FROM messages m JOIN message_bodies b USING (account_id, folder_name, uid)
      WHERE m.folder_name LIKE '%Drafts%' ORDER BY datetime(m.date) DESC LIMIT 10;"
   ```
   Parse the JSON arrays and echo the full **To / Cc / Bcc** back to the user. If the row is missing or the To list is empty, STOP and report it — never send a draft whose recipients you can't verify.
4. Ask the user to confirm the recipient set is complete and correct. Proceed only on an explicit "yes".
5. Send: call `mcp__cxmail__send_email` with the draft id. The MCP send tool is disabled by design and returns guidance instead of transmitting — if so, tell the user to open the draft in CXMail's Drafts folder and click Send. That in-app send is the only path that actually delivers mail.
6. Report the result (sent / directed to in-app send / error) back to the user and stop.

This command is the only one in the cxmail plugin concerned with transmitting mail. Treat it accordingly — verify recipients, one send, no chaining, no "while we're here" extras.
