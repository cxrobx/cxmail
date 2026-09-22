import { api } from "@/lib/tauri";
import type { MessageSummary } from "@/types/email";

/**
 * Resolve the UIDs that an action against `message` should affect, expanding
 * collapsed thread rows to every member in the SAME folder.
 *
 * Single-message rows return `[message.uid]` immediately — no backend
 * roundtrip. For multi-message threads, the backend's
 * `get_thread_uids_in_folder` is authoritative (it walks the full
 * `thread_root_id` index and only returns members within the displayed
 * folder).
 *
 * Cross-folder members (Sent copies, etc.) are intentionally excluded —
 * matches Gmail's behavior: archiving in Inbox does not also archive the
 * Sent copy.
 *
 * Falls back to `[message.uid]` if the IPC fails for any reason; this
 * keeps the action's blast radius bounded under network/IPC errors rather
 * than silently no-op'ing.
 */
export async function uidsForThreadAction(
  message: MessageSummary,
  fallbackAccountId: string,
  fallbackFolder: string,
): Promise<number[]> {
  if (!message.thread_root_id || message.thread_count <= 0) {
    return [message.uid];
  }
  const accountId = message.account_id ?? fallbackAccountId;
  const folder = message.folder_name ?? fallbackFolder;
  try {
    const uids = await api.messages.threadUidsInFolder(
      accountId,
      folder,
      message.thread_root_id,
    );
    return uids.length > 0 ? uids : [message.uid];
  } catch (e) {
    console.warn("threadUidsInFolder failed; falling back to single uid:", e);
    return [message.uid];
  }
}
