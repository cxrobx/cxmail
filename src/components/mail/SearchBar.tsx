import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useAccountStore } from "@/stores/accountStore";
import { useUIStore } from "@/stores/uiStore";
import { filtersFromChips, runServerSearch } from "@/lib/searchFilters";
import { searchScopeIds } from "@/lib/searchScope";
import type { AppliedFilter, ContactResult, SearchResult } from "@/types/email";
import { X, Loader2, Sparkles, Clock, User } from "lucide-react";

/** Trailing `from:partial` / `to:partial` token, for contact autocomplete. */
function trailingContactToken(input: string): { op: string; partial: string } | null {
  const m = /(?:^|\s)(from|to):([^\s"]*)$/i.exec(input);
  if (!m) return null;
  return { op: m[1].toLowerCase(), partial: m[2] };
}

type DropdownItem =
  | { type: "recent"; query: string }
  | { type: "contact"; contact: ContactResult; op: string }
  | { type: "result"; result: SearchResult };

export default function SearchBar() {
  const [query, setQuery] = useState("");
  const [dropdownResults, setDropdownResults] = useState<SearchResult[]>([]);
  const [contactSuggestions, setContactSuggestions] = useState<ContactResult[]>([]);
  const [contactOp, setContactOp] = useState("from");
  const [isSearching, setIsSearching] = useState(false);
  const [showDropdown, setShowDropdown] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const { setSearchResults, clearSearch, setSelectedMessage, searchQuery } = useMailStore();
  const selectedAccountGroup = useMailStore((s) => s.selectedAccountGroup);
  const isUnifiedInbox = useMailStore((s) => s.isUnifiedInbox);
  const specialView = useMailStore((s) => s.specialView);
  const selectedGroupId = useMailStore((s) => s.selectedGroupId);
  const selectedAccountId = useMailStore((s) => s.selectedAccountId);
  const accounts = useAccountStore((s) => s.accounts);
  const recentSearches = useUIStore((s) => s.recentSearches);
  const addRecentSearch = useUIStore((s) => s.addRecentSearch);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Scope dropdown + committed searches: the selected account folder's
  // visible members, the hidden account on screen, or undefined (= every
  // visible account). See `searchScopeIds` for why `[]` stays `[]`.
  const scopeIds = useMemo(
    () =>
      searchScopeIds(accounts, {
        isUnifiedInbox,
        specialView,
        selectedGroupId,
        selectedAccountId,
        selectedAccountGroup,
      }),
    [accounts, isUnifiedInbox, specialView, selectedGroupId, selectedAccountId, selectedAccountGroup],
  );
  const scoped = scopeIds !== undefined;

  // Clear local input when store search is cleared (e.g. by sidebar navigation)
  useEffect(() => {
    if (!searchQuery && query) {
      setQuery("");
      setDropdownResults([]);
      setContactSuggestions([]);
      setShowDropdown(false);
      setActiveIndex(-1);
    }
  }, [searchQuery]);

  const items: DropdownItem[] = useMemo(() => {
    if (query.length < 2) {
      return recentSearches.map((q) => ({ type: "recent" as const, query: q }));
    }
    return [
      ...contactSuggestions.map((contact) => ({ type: "contact" as const, contact, op: contactOp })),
      ...dropdownResults.slice(0, 8).map((result) => ({ type: "result" as const, result })),
    ];
  }, [query, recentSearches, contactSuggestions, contactOp, dropdownResults]);

  // Debounced LOCAL search for the dropdown preview — cheap FTS with prefix
  // matching; the AI parse only runs on Enter.
  const handleInputChange = (value: string) => {
    setQuery(value);
    setActiveIndex(-1);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (value.length < 2) {
      setDropdownResults([]);
      setContactSuggestions([]);
      setShowDropdown(value.length === 0 && recentSearches.length > 0);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      setIsSearching(true);
      const token = trailingContactToken(value);
      const [results, contacts] = await Promise.all([
        api.messages
          .search(value, { prefix: true, accountIds: scopeIds })
          .catch(() => [] as SearchResult[]),
        token && token.partial.length >= 1
          ? api.messages.searchContacts(token.partial).catch(() => [] as ContactResult[])
          : Promise.resolve([] as ContactResult[]),
      ]);
      setDropdownResults(results);
      setContactSuggestions(contacts.slice(0, 4));
      if (token) setContactOp(token.op);
      setShowDropdown(true);
      setIsSearching(false);
    }, 250);
  };

  // Enter commits: deterministic operator parse + optional LLM on the backend,
  // chips come back as applied_filters. Thin local results kick off the IMAP
  // server-side fallback in the background.
  const commitSearch = useCallback(
    async (q: string) => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      setShowDropdown(false);
      setActiveIndex(-1);
      setIsSearching(true);
      addRecentSearch(q);
      let committedFilters: AppliedFilter[] | null = null;
      try {
        const resp = await api.messages.aiSearch(q, scopeIds);
        committedFilters = resp.applied_filters;
        setSearchResults(q, resp.results, resp.applied_filters, {
          aiFallback: resp.ai_failed,
          scopedToGroup: scoped,
          scopeIds,
        });
        if (resp.results.length < 10) {
          void runServerSearch(q, filtersFromChips(resp.applied_filters), scopeIds);
        }
      } catch {
        try {
          const res = await api.messages.search(q, { accountIds: scopeIds });
          committedFilters = [{ kind: "keywords", value: q }];
          setSearchResults(q, res, committedFilters, {
            aiFallback: true,
            scopedToGroup: scoped,
            scopeIds,
          });
          if (res.length < 10) {
            void runServerSearch(q, filtersFromChips(committedFilters), scopeIds);
          }
        } catch {
          setSearchResults(q, []);
        }
      }
      setIsSearching(false);
    },
    [addRecentSearch, scopeIds, scoped, setSearchResults],
  );

  const openResult = (result: SearchResult) => {
    // Promote the dropdown preview to the full search-results view so the
    // left column filters down to the matches and highlights the clicked row.
    setSearchResults(query, dropdownResults, [{ kind: "keywords", value: query }], {
      scopedToGroup: scoped,
      scopeIds,
    });
    setSelectedMessage(result.uid, result.account_id, result.folder_name);
    setShowDropdown(false);
  };

  const activateItem = (item: DropdownItem) => {
    if (item.type === "recent") {
      setQuery(item.query);
      void commitSearch(item.query);
    } else if (item.type === "contact") {
      // Replace the trailing from:/to: token with the picked contact.
      const replaced = query.replace(/(from|to):[^\s"]*$/i, `${item.op}:${item.contact.email} `);
      setQuery(replaced);
      setContactSuggestions([]);
      setActiveIndex(-1);
      inputRef.current?.focus();
      handleInputChange(replaced);
    } else {
      openResult(item.result);
    }
  };

  const handleKeyDown = async (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" && items.length > 0) {
      e.preventDefault();
      setShowDropdown(true);
      setActiveIndex((i) => (i + 1) % items.length);
      return;
    }
    if (e.key === "ArrowUp" && items.length > 0) {
      e.preventDefault();
      setActiveIndex((i) => (i <= 0 ? items.length - 1 : i - 1));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (showDropdown && activeIndex >= 0 && activeIndex < items.length) {
        activateItem(items[activeIndex]);
        return;
      }
      if (query.length >= 2) {
        await commitSearch(query);
      }
      return;
    }
    if (e.key === "Escape") {
      if (showDropdown) {
        setShowDropdown(false);
        setActiveIndex(-1);
      } else {
        handleClear();
        inputRef.current?.blur();
      }
    }
  };

  const handleClear = () => {
    setQuery("");
    setDropdownResults([]);
    setContactSuggestions([]);
    setShowDropdown(false);
    setActiveIndex(-1);
    clearSearch();
  };

  const showRecent = query.length < 2 && recentSearches.length > 0;

  return (
    <div className="relative border-b border-border-subtle px-3 py-2">
      <div className="flex items-center gap-2 rounded-md bg-surface px-2 py-1.5">
        {isSearching ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-content-muted" />
        ) : (
          <Sparkles className="h-3.5 w-3.5 shrink-0 text-accent" />
        )}
        <input
          ref={inputRef}
          type="text"
          data-search-input
          value={query}
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={() => {
            if (dropdownResults.length > 0 && !searchQuery) setShowDropdown(true);
            else if (showRecent && !searchQuery) setShowDropdown(true);
          }}
          onBlur={() => setTimeout(() => setShowDropdown(false), 200)}
          placeholder="Search emails... press Enter for full results"
          className="flex-1 bg-transparent text-sm text-content placeholder-content-muted outline-none"
        />
        {query && (
          <button onClick={handleClear} className="text-content-muted hover:text-content">
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {showDropdown && items.length > 0 && (
        <div className="absolute left-3 right-3 top-full z-50 mt-1 max-h-[300px] overflow-auto rounded-md border border-border bg-surface-solid py-1 shadow-lg">
          <div className="px-3 py-1.5 text-xs text-content-muted">
            {showRecent
              ? "Recent searches"
              : `Press Enter for full results · ${dropdownResults.length} found`}
          </div>
          {items.map((item, i) => {
            const isActive = i === activeIndex;
            const rowClass = `flex w-full flex-col gap-0.5 px-3 py-2 text-left hover:bg-elevated ${
              isActive ? "bg-elevated" : ""
            }`;
            if (item.type === "recent") {
              return (
                <button
                  key={`recent-${item.query}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    activateItem(item);
                  }}
                  className={`${rowClass} flex-row items-center gap-2`}
                >
                  <Clock className="h-3 w-3 shrink-0 text-content-muted" />
                  <span className="truncate text-sm text-content">{item.query}</span>
                </button>
              );
            }
            if (item.type === "contact") {
              return (
                <button
                  key={`contact-${item.contact.email}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    activateItem(item);
                  }}
                  className={`${rowClass} flex-row items-center gap-2`}
                >
                  <User className="h-3 w-3 shrink-0 text-accent" />
                  <span className="truncate text-sm text-content">
                    {item.op}:{item.contact.email}
                  </span>
                  {item.contact.name && (
                    <span className="truncate text-xs text-content-muted">{item.contact.name}</span>
                  )}
                </button>
              );
            }
            const r = item.result;
            return (
              <button
                key={`${r.account_id}-${r.folder_name}-${r.uid}`}
                onMouseDown={(e) => {
                  e.preventDefault();
                  activateItem(item);
                }}
                className={rowClass}
              >
                <span className="truncate text-sm text-content">
                  {r.subject || "(no subject)"}
                </span>
                <span className="text-xs text-content-muted">
                  {r.from_name || r.from_email} &middot; {r.folder_name}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
