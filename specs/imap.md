# IMAP Client

Wrapper around `async-imap` crate in `src-tauri/crates/cxmail-email/src/email/imap.rs`.

## Connection

- Host: `imap.gmail.com` (Gmail), `imap.mail.me.com` (iCloud), `outlook.office365.com` (Outlook)
- Port: 993 (IMAPS — TLS from the start)
- TLS: async-native-tls (uses macOS SecureTransport)
- Auth: XOAUTH2 SASL for Gmail/Outlook, plain LOGIN for iCloud (app-specific passwords)

## XOAUTH2 SASL Authentication

```rust
// Build the SASL XOAUTH2 token
fn build_xoauth2_token(email: &str, access_token: &str) -> String {
    let auth_string = format!(
        "user={}\x01auth=Bearer {}\x01\x01",
        email, access_token
    );
    base64::engine::general_purpose::STANDARD.encode(auth_string)
}

// Usage with async-imap:
// session = client.authenticate("XOAUTH2", |_| Ok(xoauth2_token)).await?;
```

Before each IMAP session, check if the OAuth2 access token is expired (or within 5-minute buffer). If so, refresh it first via the oauth2 module, then connect.

## Operations

### Folder Listing
```
LIST "" "*"
```
Parse response to extract folder name, delimiter, and flags. Map Gmail special folders:
- `[Gmail]/Sent Mail` → folder_type: "sent"
- `[Gmail]/Drafts` → folder_type: "drafts"
- `[Gmail]/Trash` → folder_type: "trash"
- `[Gmail]/Spam` → folder_type: "spam"
- `[Gmail]/All Mail` → folder_type: "archive"
- `INBOX` → folder_type: "inbox"

### Message Header Fetching
```
SELECT "INBOX"
FETCH <uid_range> (UID ENVELOPE FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID IN-REPLY-TO REFERENCES)])
```
- Use UID ranges for pagination: fetch UIDs from `uidnext - page_size` to `uidnext`
- BODY.PEEK avoids setting \Seen flag
- Extract: subject, from, to, cc, date, message-id, in-reply-to, references, flags, size

### Message Body Fetching
```
FETCH <uid> (BODY[])
```
- Fetched on demand (when user clicks a message)
- Full RFC822 message passed to mail-parser for MIME parsing
- Result stored in `message_bodies` table

### Flag Updates
```
STORE <uid> +FLAGS (\Seen)    -- mark read
STORE <uid> -FLAGS (\Seen)    -- mark unread
STORE <uid> +FLAGS (\Flagged) -- star
STORE <uid> -FLAGS (\Flagged) -- unstar
```

### Incremental Sync

1. SELECT folder → get current UIDVALIDITY and UIDNEXT
2. Compare UIDVALIDITY with stored value in `sync_state`
   - If different: full re-sync (delete cached messages, re-fetch all)
   - If same: fetch only UIDs > `sync_state.last_uid`
3. Fetch new message headers
4. Update `sync_state.last_uid` and `sync_state.last_synced`

### Folder Operations (Phase 2)
```
MOVE <uid> "destination"      -- move message
COPY <uid> "destination"      -- copy message
STORE <uid> +FLAGS (\Deleted) -- mark for deletion
EXPUNGE                       -- permanently delete marked messages
```

## Error Handling

- Connection refused → show "Unable to connect" in StatusBar
- Auth failed → prompt to re-authenticate (token may be revoked)
- UIDVALIDITY changed → trigger full re-sync, notify user
- Timeout → retry once, then show offline indicator
- All IMAP errors wrapped in custom `ImapError` enum via thiserror

## Connection Lifecycle

- Create new connection per sync operation (don't hold persistent connections in Phase 1)
- Phase 2: IDLE command for push notifications (persistent connection)
- Always disconnect cleanly (LOGOUT command)
