import { api } from "@/lib/tauri";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import { uidsForThreadAction } from "@/lib/threadActions";

/**
 * Archive/delete for the command palette.
 *
 * This deliberately COPIES the optimistic-removal pattern from
 * useKeyboardShortcuts.ts (⌘E / # branches) rather than refactoring the hook:
 * optimistic removeMessages → advance selection → thread-expanded IMAP call.
 * If you change the semantics there, change them here too — the two surfaces
 * must stay in lockstep.
 */
export interface MessageActionContext {
  /** Resolved account for the IMAP call (selectedMessageAccountId ?? selectedAccountId). */
  accountId: string;
  /** Resolved folder for the IMAP call. */
  folder: string;
  /** UID of the target message. */
  uid: number;
  /** Raw selectedMessageAccountId — null in single-account views where rows
   * may not carry account_id; used only to locate the row in `messages`. */
  selectedMessageAccountId: string | null;
}

async function removeThenAct(
  ctx: MessageActionContext,
  act: (accountId: string, folder: string, uids: number[]) => Promise<void>,
  verb: string,
): Promise<void> {
  const { messages, removeMessages, setSelectedMessage } = useMailStore.getState();
  const idx = messages.findIndex(
    (m) =>
      m.uid === ctx.uid
      && (!ctx.selectedMessageAccountId || m.account_id === ctx.selectedMessageAccountId),
  );
  const target = messages[idx];
  const next = messages[idx + 1] || messages[idx - 1];
  if (target) {
    removeMessages(
      new Set([
        messageSelectionKey(
          target.account_id ?? ctx.accountId,
          target.uid,
          target.folder_name ?? ctx.folder,
        ),
      ]),
    );
  }
  setSelectedMessage(next?.uid ?? null, next?.account_id);
  try {
    const uids = target
      ? await uidsForThreadAction(target, ctx.accountId, ctx.folder)
      : [ctx.uid];
    await act(ctx.accountId, ctx.folder, uids);
  } catch (e) {
    console.error(`Failed to ${verb}:`, e);
    useUIStore.getState().addToast({
      message: `Failed to ${verb}: ${e instanceof Error ? e.message : String(e)}`,
      type: "error",
    });
  }
}

export function archiveMessage(ctx: MessageActionContext): Promise<void> {
  return removeThenAct(ctx, api.messages.archive, "archive");
}

export function deleteMessage(ctx: MessageActionContext): Promise<void> {
  return removeThenAct(ctx, api.messages.delete, "delete");
}
