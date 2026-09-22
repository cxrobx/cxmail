import type { Account } from "@/types/email";

/** The slice of mail-store state that decides what a search is scoped to. */
export interface SearchScopeView {
  isUnifiedInbox: boolean;
  specialView: string | null;
  selectedGroupId: number | null;
  selectedAccountId: string | null;
  selectedAccountGroup: string | null;
}

/**
 * Which accounts a search typed into the search bar should cover.
 *
 * - An account folder is selected → its members **minus any hidden from
 *   aggregates**. A folder whose members are all hidden returns `[]`, never
 *   `undefined`: the backend reads `[]` as "nothing" and `undefined` as "every
 *   visible account", so collapsing the two would flip an empty folder into
 *   All Inboxes.
 * - A single account is on screen (the same four-term test the sidebar uses,
 *   gotcha #53) **and it is hidden** → `[thatAccount]`. Naming it is the one
 *   way its mail is reachable by search.
 * - Otherwise `undefined` — today's behaviour, which the backend now reads as
 *   every *visible* account.
 */
export function searchScopeIds(
  accounts: Account[],
  view: SearchScopeView,
): string[] | undefined {
  if (view.selectedAccountGroup) {
    return accounts
      .filter((a) => a.group_name === view.selectedAccountGroup && !a.hidden_from_aggregates)
      .map((a) => a.id);
  }
  const isAccountScoped =
    !view.isUnifiedInbox && view.specialView === null && view.selectedGroupId === null;
  if (isAccountScoped && view.selectedAccountId) {
    const account = accounts.find((a) => a.id === view.selectedAccountId);
    if (account?.hidden_from_aggregates) return [account.id];
  }
  return undefined;
}
