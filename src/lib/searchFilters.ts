import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import type { AppliedFilter, SearchFilters } from "@/types/email";

/** Rebuild backend SearchFilters from the chip row (inverse of the backend's
 * applied_filters_from). Used when a chip is removed so structured search
 * re-runs without another LLM call. */
export function filtersFromChips(chips: AppliedFilter[]): SearchFilters {
  const f: SearchFilters = {};
  for (const c of chips) {
    switch (c.kind) {
      case "from":
        f.from = c.value;
        break;
      case "to":
        f.to = c.value;
        break;
      case "cc":
        f.cc = c.value;
        break;
      case "subject":
        f.subject_contains = c.value;
        break;
      case "filename":
        f.filename = c.value;
        break;
      case "after":
        f.date_after = c.value;
        break;
      case "before":
        // The chip shows the bare date; the backend bound is inclusive
        // end-of-day (a bare date normalizes to midnight = exclusive).
        f.date_before = c.value.length === 10 ? `${c.value}T23:59:59` : c.value;
        break;
      case "in":
        f.folder = c.value;
        break;
      case "has":
        if (c.value === "attachment") f.has_attachments = true;
        break;
      case "is":
        if (c.value === "starred") f.is_starred = true;
        else if (c.value === "unread") f.is_unread = true;
        else if (c.value === "read") f.is_unread = false;
        break;
      case "larger":
        f.larger_bytes = Number(c.value) || null;
        break;
      case "smaller":
        f.smaller_bytes = Number(c.value) || null;
        break;
      case "keywords":
        f.keywords = c.value;
        break;
    }
  }
  return f;
}

/**
 * Commit a global (unscoped) search — the command palette's "Search mail for"
 * fallthrough. Mirrors SearchBar.commitSearch minus group scoping: LLM parse
 * first, plain FTS on failure, thin results kick the IMAP server fallback.
 */
export async function commitGlobalSearch(q: string): Promise<void> {
  // setSearchResults does NOT clear specialView, and MessageList renders
  // special views before checking searchQuery — without this the results
  // would be committed but invisible.
  if (useMailStore.getState().specialView) {
    useMailStore.getState().setUnifiedInbox();
  }
  useUIStore.getState().addRecentSearch(q);
  try {
    const resp = await api.messages.aiSearch(q);
    const store = useMailStore.getState();
    store.setSearchResults(q, resp.results, resp.applied_filters, {
      aiFallback: resp.ai_failed,
    });
    if (resp.results.length < 10) {
      void runServerSearch(q, filtersFromChips(resp.applied_filters));
    }
  } catch {
    try {
      const results = await api.messages.search(q);
      const chips: AppliedFilter[] = [{ kind: "keywords", value: q }];
      useMailStore.getState().setSearchResults(q, results, chips, { aiFallback: true });
      if (results.length < 10) {
        void runServerSearch(q, filtersFromChips(chips));
      }
    } catch {
      useMailStore.getState().setSearchResults(q, []);
    }
  }
}

/** Kick off the IMAP server-side search fallback for the CURRENT committed
 * query. Results merge into the store tagged with a cloud badge. Stale
 * completions (query changed while searching) are dropped. Failures are
 * non-fatal — the indicator just hides. */
export async function runServerSearch(
  query: string,
  filters: SearchFilters,
  accountIds?: string[],
): Promise<void> {
  const store = useMailStore.getState();
  if (store.serverSearchStatus === "searching") return;
  store.setServerSearchStatus("searching");
  try {
    const results = await api.messages.serverSearch(filters, accountIds);
    const current = useMailStore.getState();
    if (current.searchQuery !== query) return; // user moved on
    current.appendSearchResults(results, true);
    current.setServerSearchStatus("done");
  } catch (e) {
    console.error("Server search failed:", e);
    const current = useMailStore.getState();
    if (current.searchQuery === query) current.setServerSearchStatus("error");
  }
}
