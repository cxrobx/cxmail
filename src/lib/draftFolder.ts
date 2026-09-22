import type { Folder } from "@/types/email";

/**
 * Is this folder the account's drafts folder — i.e. is a message sitting in it
 * an UNSENT draft rather than correspondence?
 *
 * One function because it was two, and they had already drifted: `MessageList`
 * consulted the synced folder list alone, while `MessageListItem` also
 * accepted the two well-known names. So the same row could open the composer
 * on double-click (the item's copy) and select as a read-only message on
 * single-click (the list's copy) whenever `foldersByAccount` had not loaded.
 * The thread card needs the same answer, and a third copy is how the next
 * divergence starts (gotcha #36).
 *
 * `folder_type` is the real answer — a generic IMAP account can call its
 * drafts mailbox anything, and the sync classifier has already resolved that.
 * The name check is a fallback for the window before the folder list arrives,
 * and is deliberately the *superset* of what the two copies did.
 *
 * Unknown → false, matching `folders::is_draft_sql` on the Rust side: an
 * unsynced folder reads as ordinary mail, which shows a message that might be
 * a draft rather than hiding one that is not.
 */
export function isDraftFolder(
  foldersByAccount: Record<string, Folder[]>,
  accountId: string | null | undefined,
  folderName: string | null | undefined,
): boolean {
  if (!accountId || !folderName) return false;
  const folders = foldersByAccount[accountId];
  if (folders?.some((f) => f.name === folderName && f.folder_type === "drafts")) return true;
  return folderName === "Drafts" || folderName === "[Gmail]/Drafts";
}
