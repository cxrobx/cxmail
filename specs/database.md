# Database Schema

SQLite via rusqlite (bundled). Location: `~/Library/Application Support/com.cxmail.app/cxmail.db`

## Tables

```sql
-- Accounts (minimal — credentials in Keychain, not here)
CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,              -- UUID v4
    email       TEXT NOT NULL UNIQUE,
    display_name TEXT,
    provider    TEXT NOT NULL,                 -- 'gmail', 'icloud', 'outlook'
    imap_host   TEXT NOT NULL,
    imap_port   INTEGER NOT NULL DEFAULT 993,
    smtp_host   TEXT NOT NULL,
    smtp_port   INTEGER NOT NULL DEFAULT 587,
    color       TEXT,                          -- Account accent color hex
    is_active   INTEGER NOT NULL DEFAULT 1,
    -- Later migrations add: imap_security/smtp_security/imap_username/
    -- smtp_username (v50), sort_order, group_name (account folder),
    -- notify_enabled (v25), track_opens_enabled (v41), and
    hidden_from_aggregates INTEGER NOT NULL DEFAULT 0, -- v58: kept out of every cross-account view;
                                               -- enforced by db::accounts::visible_in_aggregates_sql
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Folders (cached IMAP mailbox list)
CREATE TABLE folders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                 -- e.g., 'INBOX', '[Gmail]/Sent Mail'
    display_name TEXT,                         -- User-friendly name
    folder_type TEXT,                          -- 'inbox', 'sent', 'drafts', 'trash', 'spam', 'archive', 'other'
    delimiter   TEXT,                          -- IMAP hierarchy delimiter
    flags       TEXT,                          -- JSON array of IMAP flags
    total_count INTEGER DEFAULT 0,
    unread_count INTEGER DEFAULT 0,
    uidvalidity INTEGER,                      -- IMAP UIDVALIDITY for cache invalidation
    uidnext     INTEGER,                      -- IMAP UIDNEXT for incremental sync
    last_synced TEXT,
    UNIQUE(account_id, name)
);

-- Messages (cached email headers + metadata only)
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_name TEXT NOT NULL,
    uid         INTEGER NOT NULL,              -- IMAP UID within folder
    message_id  TEXT,                          -- RFC 2822 Message-ID header
    in_reply_to TEXT,                          -- For threading
    references  TEXT,                          -- JSON array of Message-IDs
    subject     TEXT,
    from_name   TEXT,
    from_email  TEXT,
    to_list     TEXT,                          -- JSON array of {name, email}
    cc_list     TEXT,                          -- JSON array of {name, email}
    date        TEXT NOT NULL,                 -- Stored as ISO 8601
    snippet     TEXT,                          -- First ~200 chars of plain text body
    flags       TEXT,                          -- JSON array: ["\\Seen", "\\Flagged", etc.]
    is_read     INTEGER NOT NULL DEFAULT 0,
    is_flagged  INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    size_bytes  INTEGER DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, folder_name, uid)
);

-- Message bodies (stored separately — fetched on demand)
CREATE TABLE message_bodies (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL,
    folder_name TEXT NOT NULL,
    uid         INTEGER NOT NULL,
    plain_text  TEXT,                          -- Plain text body
    html_body   TEXT,                          -- Raw HTML body (before sanitization)
    sanitized_html TEXT,                       -- ammonia-sanitized HTML (safe to render)
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, folder_name, uid),
    FOREIGN KEY (account_id, folder_name, uid)
        REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
);

-- Attachments metadata (files fetched on demand)
CREATE TABLE attachments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL,
    folder_name TEXT NOT NULL,
    message_uid INTEGER NOT NULL,
    filename    TEXT,
    content_type TEXT,
    size_bytes  INTEGER,
    content_id  TEXT,                          -- For inline images (cid: references)
    is_inline   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (account_id, folder_name, message_uid)
        REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
);

-- Sync state (track incremental sync per folder)
CREATE TABLE sync_state (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_name TEXT NOT NULL,
    last_uid    INTEGER DEFAULT 0,             -- Highest UID synced
    uidvalidity INTEGER,                       -- Detect mailbox recreations
    last_synced TEXT,
    UNIQUE(account_id, folder_name)
);

-- Schema version tracking
CREATE TABLE schema_version (
    version     INTEGER NOT NULL
);
```

## Indexes

```sql
CREATE INDEX idx_messages_account_folder ON messages(account_id, folder_name);
CREATE INDEX idx_messages_date ON messages(account_id, folder_name, date DESC);
CREATE INDEX idx_messages_message_id ON messages(message_id);
CREATE INDEX idx_messages_is_read ON messages(account_id, folder_name, is_read);
CREATE INDEX idx_folders_account ON folders(account_id);
```

## Key Design Decisions

1. **Bodies separate from headers**: The `messages` table has only metadata for fast list queries. Bodies are in `message_bodies`, fetched on demand when user clicks a message. This keeps the message list query fast even with thousands of emails.

2. **Pre-sanitized HTML cached**: `sanitized_html` is computed once by ammonia and stored alongside raw HTML. No re-sanitizing on every view.

3. **UIDVALIDITY tracking**: If IMAP server resets UIDs (mailbox recreated), `sync_state.uidvalidity` will mismatch. This triggers a full re-sync of that folder — never serve stale data.

4. **No credentials here**: All OAuth tokens and passwords go to macOS Keychain via keyring crate. The `accounts` table only stores server hostnames, ports, and display metadata.

5. **CASCADE deletes**: Removing an account cascades to all its folders, messages, bodies, and attachments.

6. **Migration pattern**: Simple version number in `schema_version` table. On launch, check version and run any pending migrations sequentially.
