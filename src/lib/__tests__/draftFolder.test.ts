import { describe, it, expect } from 'vitest';
import { isDraftFolder } from '@/lib/draftFolder';
import type { Folder } from '@/types/email';

const folder = (name: string, folder_type: string | null): Folder => ({
  id: 1, account_id: 'a', name, display_name: null, folder_type,
  delimiter: '/', total_count: 0, unread_count: 0, uidvalidity: 1, uidnext: 2,
});

const SYNCED = {
  a: [
    folder('INBOX', 'inbox'),
    folder('Entwürfe', 'drafts'),
    folder('Drafts', 'custom'),
  ],
};

describe('isDraftFolder', () => {
  /** `folder_type` is the answer, not the name: a generic IMAP account can
   * call its drafts mailbox anything, and the sync classifier has already
   * worked that out. */
  it('reads the synced folder type, whatever the folder is called', () => {
    expect(isDraftFolder(SYNCED, 'a', 'Entwürfe')).toBe(true);
    expect(isDraftFolder(SYNCED, 'a', 'INBOX')).toBe(false);
  });

  /** The two copies this replaced disagreed exactly here: MessageList
   * consulted the folder list alone, so before it loaded a Drafts row selected
   * as a read-only message while the same row opened the composer on
   * double-click. */
  it('still recognises the two well-known names before the folder list loads', () => {
    expect(isDraftFolder({}, 'a', 'Drafts')).toBe(true);
    expect(isDraftFolder({}, 'a', '[Gmail]/Drafts')).toBe(true);
    expect(isDraftFolder({}, 'a', 'INBOX')).toBe(false);
  });

  /** Unknown reads as ordinary mail, matching `folders::is_draft_sql` on the
   * Rust side — the direction that shows a message rather than hiding one
   * behind a label that may be wrong. */
  it('treats an unknown folder, and a missing account, as ordinary mail', () => {
    expect(isDraftFolder(SYNCED, 'a', 'Some/Label')).toBe(false);
    expect(isDraftFolder(SYNCED, 'other', 'Entwürfe')).toBe(false);
    expect(isDraftFolder(SYNCED, null, 'Entwürfe')).toBe(false);
    expect(isDraftFolder(SYNCED, 'a', null)).toBe(false);
  });
});
