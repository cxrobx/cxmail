import { beforeEach, expect, it, vi } from 'vitest';
import { api } from '@/lib/tauri';
import { useMailStore } from '@/stores/mailStore';
import { runServerSearch } from '../searchFilters';
import type { SearchResult } from '@/types/email';
vi.mock('@/lib/tauri', () => ({ api: { messages: { serverSearch: vi.fn() } } }));
beforeEach(() => { vi.clearAllMocks(); useMailStore.getState().clearSearch(); });
it('does not append a late server response after the same query moves to another account', async () => {
  let resolve!: (value: SearchResult[]) => void;
  vi.mocked(api.messages.serverSearch).mockReturnValue(new Promise((r) => { resolve = r; }));
  useMailStore.getState().setSearchResults('budget', [], [], { scopeIds: ['a'], scopedToGroup: true });
  const request = runServerSearch('budget', {}, ['a']);
  useMailStore.getState().setSearchResults('budget', [], [], { scopeIds: ['b'], scopedToGroup: true });
  resolve([{ account_id: 'a', folder_name: 'INBOX', uid: 1, subject: 'Private budget' } as SearchResult]);
  await request;
  expect(useMailStore.getState().searchResults).toEqual([]);
  expect(useMailStore.getState().searchScopeIds).toEqual(['b']);
});
