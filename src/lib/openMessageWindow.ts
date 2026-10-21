import { emailWindowKey, useWindowStore } from "@/stores/windowStore";
import { useMailStore } from "@/stores/mailStore";
import { openDraftForEdit } from "@/lib/draftCompose";
import { isDraftFolder } from "@/lib/draftFolder";

/**
 * Pop a message out into its own floating window — or, when it sits in the
 * account's drafts folder, open it in the composer instead (a read-only view of
 * an unsent reply is the one view nobody wants; see gotcha #61).
 *
 * One function because the message list had it twice (Option-click and
 * double-click in `MessageListItem`) and search results had it not at all.
 * Both branches are idempotent: the email window is keyed, so a repeat opens
 * nothing new, and `openDraftForEdit` joins an open already in flight (#55).
 */
export function openMessageWindow(
  accountId: string,
  folder: string,
  uid: number,
  subject: string | null | undefined,
): void {
  if (isDraftFolder(useMailStore.getState().foldersByAccount, accountId, folder)) {
    void openDraftForEdit(accountId, folder, uid);
    return;
  }
  useWindowStore.getState().openWindow({
    type: "email",
    title: subject || "(no subject)",
    props: { accountId, folder, uid },
    key: emailWindowKey(accountId, folder, uid),
  });
}
