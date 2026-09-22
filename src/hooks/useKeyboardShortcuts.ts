import { useEffect, useCallback } from "react";
import { messageSelectionKey, useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import { api } from "@/lib/tauri";
import { uidsForThreadAction } from "@/lib/threadActions";

interface ShortcutHandlers {
  onCommandPalette: () => void;
  onShortcutSheet: () => void;
}

export function useKeyboardShortcuts({ onCommandPalette, onShortcutSheet }: ShortcutHandlers) {
  const {
    selectedAccountId,
    selectedFolder,
    selectedMessageUid,
    selectedMessageAccountId,
    messages,
    setSelectedMessage,
    setComposing,
    toggleMessagePin,
    toggleMessageMute,
    removeMessages,
  } = useMailStore();
  const { toggleSidebar } = useUIStore();

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const selectedMessage = messages.find(
        (message) =>
          message.uid === selectedMessageUid
          && (!selectedMessageAccountId || message.account_id === selectedMessageAccountId),
      );
      const actionAccountId = selectedMessageAccountId ?? selectedAccountId;
      const actionFolder =
        selectedFolder ?? (selectedMessageAccountId ? "INBOX" : null);

      // Don't trigger in input/textarea/contenteditable
      const target = e.target as HTMLElement;
      const inEditable =
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable;
      if (inEditable) {
        // Only handle Cmd/Ctrl combos in inputs
        if (!e.metaKey && !e.ctrlKey) return;
        // The field already acted on it — ⌘K opening the compose link field,
        // ⌘\ clearing formatting. Don't also run the app-wide binding.
        if (e.defaultPrevented) return;
      }

      // Cmd+K / Cmd+P — command palette
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "p")) {
        e.preventDefault();
        onCommandPalette();
        return;
      }

      // Cmd+N — compose
      if ((e.metaKey || e.ctrlKey) && e.key === "n") {
        e.preventDefault();
        setComposing(true);
        return;
      }

      // Cmd+\ — toggle sidebar
      if ((e.metaKey || e.ctrlKey) && e.key === "\\") {
        e.preventDefault();
        toggleSidebar();
        return;
      }

      // Past here every binding is a message action. While typing, a held ⌘
      // must not reach them: ⌘S starred the selected message, ⌘M muted it and
      // jumped to the next, ⌘J changed the selection under a reply.
      if (inEditable) return;

      // / — focus search (stable data attribute, not the placeholder text —
      // matching on placeholder broke silently when the copy changed)
      if (e.key === "/" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        const searchInput = document.querySelector<HTMLInputElement>(
          "[data-search-input]"
        );
        searchInput?.focus();
        return;
      }

      // c — compose
      if (e.key === "c" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        setComposing(true);
        return;
      }

      // j/k — navigate messages
      if (e.key === "j" || e.key === "k") {
        e.preventDefault();
        if (messages.length === 0) return;
        const currentIdx = messages.findIndex(
          (m) =>
            m.uid === selectedMessageUid
            && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
        );
        if (e.key === "j") {
          const next = Math.min(currentIdx + 1, messages.length - 1);
          setSelectedMessage(messages[next].uid, messages[next].account_id);
        } else {
          const prev = Math.max(currentIdx - 1, 0);
          setSelectedMessage(messages[prev].uid, messages[prev].account_id);
        }
        return;
      }

      // ⌘E — archive (whole thread within this folder). Deliberately NOT a
      // bare key: a stray "e" press used to silently archive mail. Excluded
      // from inputs so ⌘E while typing in search can't archive the selection.
      // NOTE: src/lib/messageActions.ts copies this optimistic-removal
      // pattern for the command palette — keep the two in lockstep.
      if (
        e.key === "e"
        && (e.metaKey || e.ctrlKey)
        && !inEditable
        && actionAccountId && actionFolder && selectedMessageUid
      ) {
        e.preventDefault();
        const idx = messages.findIndex(
          (m) =>
            m.uid === selectedMessageUid
            && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
        );
        const target = messages[idx];
        const next = messages[idx + 1] || messages[idx - 1];
        if (target) {
          removeMessages(
            new Set([
              messageSelectionKey(
                target.account_id ?? actionAccountId,
                target.uid,
                target.folder_name ?? actionFolder,
              ),
            ]),
          );
        }
        setSelectedMessage(next?.uid ?? null, next?.account_id);
        const uidSrc = target ?? null;
        if (uidSrc) {
          uidsForThreadAction(uidSrc, actionAccountId, actionFolder)
            .then((uids) =>
              api.messages.archive(actionAccountId, actionFolder, uids),
            )
            .catch(console.error);
        } else {
          api.messages
            .archive(actionAccountId, actionFolder, [selectedMessageUid])
            .catch(console.error);
        }
        return;
      }

      // # — delete (whole thread within this folder)
      if (e.key === "#" && actionAccountId && actionFolder && selectedMessageUid) {
        e.preventDefault();
        const idx = messages.findIndex(
          (m) =>
            m.uid === selectedMessageUid
            && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
        );
        const target = messages[idx];
        const next = messages[idx + 1] || messages[idx - 1];
        if (target) {
          removeMessages(new Set([messageSelectionKey(target.account_id ?? actionAccountId, target.uid, target.folder_name ?? actionFolder)]));
        }
        setSelectedMessage(next?.uid ?? null, next?.account_id);
        const uidSrc = target ?? null;
        if (uidSrc) {
          uidsForThreadAction(uidSrc, actionAccountId, actionFolder)
            .then((uids) =>
              api.messages.delete(actionAccountId, actionFolder, uids),
            )
            .catch(console.error);
        } else {
          api.messages
            .delete(actionAccountId, actionFolder, [selectedMessageUid])
            .catch(console.error);
        }
        return;
      }

      // r — reply to message
      if (e.key === "r" && !e.metaKey && !e.ctrlKey && selectedMessageUid) {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("cxmail:compose-reply"));
        return;
      }

      // s — toggle star
      if (e.key === "s" && actionAccountId && actionFolder && selectedMessageUid) {
        e.preventDefault();
        if (selectedMessage) {
          api.messages
            .toggleStar(
              actionAccountId,
              actionFolder,
              selectedMessageUid,
              !selectedMessage.is_flagged,
            )
            .catch(console.error);
        }
        return;
      }

      // p — toggle pin. Bare key only: ⌘P belongs to the palette branch above
      // (which returns first), and this guard keeps that contract explicit.
      if (e.key === "p" && !e.metaKey && !e.ctrlKey && actionAccountId && actionFolder && selectedMessageUid) {
        e.preventDefault();
        if (selectedMessage) {
          api.messages
            .togglePin(
              actionAccountId,
              actionFolder,
              selectedMessageUid,
              !selectedMessage.is_pinned,
            )
            .catch(console.error);
          toggleMessagePin(selectedMessageUid, actionAccountId);
        }
        return;
      }

      // m — toggle mute
      if (e.key === "m" && actionAccountId && actionFolder && selectedMessageUid) {
        e.preventDefault();
        api.messages
          .toggleMute(actionAccountId, actionFolder, selectedMessageUid, true)
          .catch(console.error);
        toggleMessageMute(selectedMessageUid, actionAccountId);
        const idx = messages.findIndex(
          (m) =>
            m.uid === selectedMessageUid
            && (!selectedMessageAccountId || m.account_id === selectedMessageAccountId),
        );
        const next = messages[idx + 1] || messages[idx - 1];
        setSelectedMessage(next?.uid ?? null, next?.account_id);
        return;
      }

      // ? — shortcut cheatsheet
      if (e.key === "?" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        onShortcutSheet();
        return;
      }

      // Escape — deselect message
      if (e.key === "Escape") {
        setSelectedMessage(null);
        return;
      }
    },
    [
      selectedAccountId,
      selectedFolder,
      selectedMessageUid,
      selectedMessageAccountId,
      messages,
      setSelectedMessage,
      setComposing,
      toggleSidebar,
      toggleMessagePin,
      toggleMessageMute,
      removeMessages,
      onCommandPalette,
      onShortcutSheet,
    ]
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
