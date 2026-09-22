import { create } from "zustand";
import type { Account } from "@/types/email";

interface AccountState {
  accounts: Account[];
  isSetupComplete: boolean;
  showingAddAccount: boolean;
  setAccounts: (accounts: Account[]) => void;
  addAccount: (account: Account) => void;
  removeAccount: (id: string) => void;
  setShowingAddAccount: (show: boolean) => void;
}

export const useAccountStore = create<AccountState>((set) => ({
  accounts: [],
  isSetupComplete: false,
  showingAddAccount: false,
  setAccounts: (accounts) =>
    set({ accounts, isSetupComplete: accounts.length > 0 }),
  addAccount: (account) =>
    set((state) => {
      // A re-auth of an already-connected email arrives here with the SAME
      // effective id (see db::accounts::upsert) — update that row in place
      // instead of appending a duplicate. Email fallback covers the case
      // where the local list hasn't refreshed yet.
      const idx = state.accounts.findIndex(
        (a) => a.id === account.id || a.email === account.email,
      );
      const accounts =
        idx === -1
          ? [...state.accounts, account]
          : state.accounts.map((a, i) => (i === idx ? account : a));
      return {
        accounts,
        isSetupComplete: true,
        showingAddAccount: false,
      };
    }),
  removeAccount: (id) =>
    set((state) => {
      const accounts = state.accounts.filter((a) => a.id !== id);
      return { accounts, isSetupComplete: accounts.length > 0 };
    }),
  setShowingAddAccount: (show) => set({ showingAddAccount: show }),
}));
