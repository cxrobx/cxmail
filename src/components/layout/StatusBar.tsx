import { useMailStore } from "@/stores/mailStore";
import { useAccountStore } from "@/stores/accountStore";
import { Wifi, WifiOff, Loader2, RefreshCw } from "lucide-react";
import { api } from "@/lib/tauri";

function formatRelativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export default function StatusBar() {
  const { isSyncing, selectedFolder, selectedAccountId, lastSyncedAt, syncError, accountsNeedingReauth, messages } = useMailStore();
  const accounts = useAccountStore((s) => s.accounts);
  const setShowingAddAccount = useAccountStore((s) => s.setShowingAddAccount);
  const account = accounts.find((a) => a.id === selectedAccountId);
  const unreadCount = messages.filter((m) => !m.is_read).length;

  const handleForceSync = async () => {
    const { setSyncing, setSyncCompleted, setSyncError } = useMailStore.getState();
    setSyncing(true);
    try {
      await Promise.race([
        api.messages.forceFullSync(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error("Force sync timed out after 4 minutes")), 240_000)
        ),
      ]);
      setSyncCompleted();
      window.dispatchEvent(new CustomEvent("cxmail:refresh-messages"));
    } catch (e) {
      setSyncError(e instanceof Error ? e.message : "Force sync failed");
    }
  };

  return (
    <div className="flex h-6 items-center justify-between border-t border-border-subtle bg-base px-3">
      <div className="flex items-center gap-2 text-xs text-content-muted">
        {isSyncing ? (
          <>
            <Loader2 className="h-3 w-3 animate-spin text-accent" />
            <span>Syncing...</span>
          </>
        ) : accountsNeedingReauth.length > 0 ? (
          <>
            <WifiOff className="h-3 w-3 text-red-400" />
            <span className="max-w-[240px] truncate text-red-400">
              {accountsNeedingReauth.length === 1
                ? `${accountsNeedingReauth[0]} needs reconnecting`
                : `${accountsNeedingReauth.length} accounts need reconnecting`}
            </span>
            <button
              onClick={() => setShowingAddAccount(true)}
              className="ml-1 flex items-center gap-1 rounded px-1.5 py-0.5 text-content-secondary transition-colors hover:bg-surface hover:text-content"
              title="Reconnect this account"
            >
              <RefreshCw className="h-3 w-3" />
              <span>Reconnect</span>
            </button>
          </>
        ) : syncError ? (
          <>
            <WifiOff className="h-3 w-3 text-red-400" />
            <span className="max-w-[240px] truncate text-red-400" title={syncError}>
              {syncError}
            </span>
            <button
              onClick={handleForceSync}
              className="ml-1 flex items-center gap-1 rounded px-1.5 py-0.5 text-content-secondary transition-colors hover:bg-surface hover:text-content"
              title="Force full sync (resets checkpoints)"
            >
              <RefreshCw className="h-3 w-3" />
              <span>Retry</span>
            </button>
          </>
        ) : (
          <>
            <Wifi className="h-3 w-3 text-green-500" />
            <span>Connected</span>
            {lastSyncedAt && (
              <span className="text-content-faint">· {formatRelativeTime(lastSyncedAt)}</span>
            )}
          </>
        )}
      </div>
      <div className="flex items-center gap-3 text-xs text-content-muted">
        {unreadCount > 0 && (
          <span>{unreadCount} unread</span>
        )}
        {account && <span>{account.email}</span>}
        {selectedFolder && <span>{selectedFolder}</span>}
      </div>
    </div>
  );
}
