# Sync Engine

## Phase 1: Polling-Based Sync

### On App Launch
1. For each active account:
   a. Connect to IMAP with stored credentials (refresh token if needed)
   b. Sync all folders (LIST command → update folders table)
   c. Sync INBOX (incremental fetch of new messages)
2. Emit `sync-complete` event to frontend

### Incremental Sync Algorithm
```
1. SELECT folder
2. Read current UIDVALIDITY and UIDNEXT from IMAP server
3. Load stored UIDVALIDITY and last_uid from sync_state table

4. IF server UIDVALIDITY != stored UIDVALIDITY:
     → Full re-sync: delete all cached messages for this folder, re-fetch all
     → Update sync_state with new UIDVALIDITY

5. ELSE IF server UIDNEXT > stored last_uid + 1:
     → Fetch headers for UIDs from (last_uid + 1) to (UIDNEXT - 1)
     → Parse and store in messages table
     → Update sync_state.last_uid = highest fetched UID

6. Check for flag changes on recent messages (last 50):
     → FETCH <uid_range> (FLAGS)
     → Update is_read, is_flagged in messages table

7. Update folder unread_count:
     → SEARCH UNSEEN
     → Update folders.unread_count

8. Emit sync-complete event
```

### Manual Sync
- User clicks sync button or pulls to refresh
- Runs the same incremental sync for the currently selected folder
- Shows progress in StatusBar

### Background Sync
- Timer-based: sync every 5 minutes while app is open
- Runs on a tokio background task
- Does NOT block the UI

## Phase 2: IDLE Push Notifications

IMAP IDLE command for real-time new mail notification:

```
1. After initial sync, send IDLE command on INBOX
2. Server holds connection open
3. When new mail arrives, server sends "EXISTS" response
4. Client breaks IDLE, fetches new messages, re-enters IDLE
5. Refresh IDLE every 29 minutes (RFC 2177 recommendation)
```

Requires a persistent IMAP connection (separate from the sync connection).

## Offline Mode

- All fetched messages are cached in SQLite
- If IMAP connection fails:
  - Show cached data normally
  - StatusBar shows "Offline" indicator
  - Queue write operations (mark read, move) for replay on reconnect
  - Retry connection every 30 seconds
- On reconnect:
  - Replay queued operations
  - Run incremental sync
  - Clear offline indicator

## Conflict Resolution

- Server wins: if a message was deleted on server during offline, remove from local cache
- Local flag changes: apply to server on reconnect (STORE command)
- If message was moved on server: update local folder_name

## Error Handling

| Error | Action |
|-------|--------|
| Connection refused | Show offline, retry in 30s |
| Auth failed (401) | Try token refresh. If that fails, prompt re-auth |
| UIDVALIDITY changed | Full re-sync of affected folder, notify user |
| Timeout during fetch | Retry once, then skip and sync rest |
| Folder deleted on server | Remove from local cache, notify user |
