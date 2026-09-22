import { useEffect } from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore, cacheKey, groupCacheKey } from "@/stores/mailStore";
import { api } from "@/lib/tauri";
import AccountSetup from "@/components/accounts/AccountSetup";
import AppLayout from "@/components/layout/AppLayout";
import ErrorBoundary from "@/components/shared/ErrorBoundary";
import LicenseGate from "@/components/accounts/LicenseGate";

function App() {
  const { isSetupComplete, setAccounts } = useAccountStore();
  const cacheMessages = useMailStore((s) => s.cacheMessages);

  useEffect(() => {
    const loadAccounts = async () => {
      try {
        const result = await api.accounts.list();
        setAccounts(result);
        // Prefetch first page of INBOX for each account (from SQLite, instant)
        for (const account of result) {
          try {
            const page = await api.messages.fetch(account.id, "INBOX", 0, 50, null);
            cacheMessages(cacheKey(account.id, "INBOX", null), page.messages);
          } catch (e) {
            console.error(`Failed to prefetch inbox for ${account.id}:`, e);
          }
        }
        // Prefetch unified inbox (all categories + primary default view)
        try {
          const [unified, unifiedPrimary] = await Promise.all([
            api.messages.fetchUnifiedInbox(0, 50, null),
            api.messages.fetchUnifiedInbox(0, 50, "primary"),
          ]);
          cacheMessages(cacheKey(null, null, null), unified.messages);
          cacheMessages(cacheKey(null, null, "primary"), unifiedPrimary.messages);
        } catch (e) {
          console.error("Failed to prefetch unified inbox:", e);
        }
        // Prefetch inbox group messages (from SQLite, instant)
        try {
          const groups = await api.inboxGroups.list();
          useMailStore.getState().setInboxGroups(groups);
          await Promise.all(
            groups.map(async (group) => {
              try {
                const page = await api.inboxGroups.fetchMessages(group.id, 0, 50);
                cacheMessages(groupCacheKey(group.id), page.messages);
              } catch (e) {
                console.error(`Failed to prefetch group ${group.name}:`, e);
              }
            })
          );
        } catch (e) {
          console.error("Failed to prefetch inbox groups:", e);
        }
      } catch (e) {
        console.error("Failed to load accounts:", e);
      }
    };
    loadAccounts();
  }, [setAccounts, cacheMessages]);

  return (
    <ErrorBoundary>
      <LicenseGate>
        {!isSetupComplete ? (
          // AccountSetup is layout-neutral so it can also render inside
          // AppLayout's fixed-width modal; first-run owns the full-screen
          // centering (and scrolls, since the IMAP form can be tall).
          <div className="flex min-h-screen items-center justify-center overflow-y-auto bg-base">
            <AccountSetup />
          </div>
        ) : (
          <Tooltip.Provider delayDuration={500} skipDelayDuration={300}>
            <AppLayout />
          </Tooltip.Provider>
        )}
      </LicenseGate>
    </ErrorBoundary>
  );
}

export default App;
