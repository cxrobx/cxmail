import { beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/tauri';
import { reviewSuggestedReply } from '../suggestedReply';
import { useWindowStore } from '@/stores/windowStore';
import { useMailStore } from '@/stores/mailStore';
import type { MessageDetail } from '@/types/email';
vi.mock('@/lib/tauri', () => ({ api: { intelligence: { suggestion: vi.fn() }, messages: { fetchBody: vi.fn() } } }));
const ref = { account_id: 'a', folder_name: 'INBOX', uid: 12 };
describe('Reviewing proactive replies', () => {
  beforeEach(() => {
    vi.clearAllMocks(); useWindowStore.setState({ windows: [] }); useMailStore.setState({ inlineComposeTarget: null });
    vi.mocked(api.messages.fetchBody).mockResolvedValue({ from_email: 'sam@peer.test', subject: 'Review', sanitized_html: '<p>Original</p>', to_list: [], cc_list: [], date: '', message_id: '<source@peer>', references: '<root@peer>' } as unknown as MessageDetail);
    vi.mocked(api.intelligence.suggestion).mockResolvedValue({ body: '<img onerror="bad">\n\nPlease confirm.', created_at: '', model: 'test' });
  });
  it('opens an editable reply with escaped text and the source threading headers', async () => {
    await reviewSuggestedReply(ref);
    const window = useWindowStore.getState().windows[0];
    expect(window.props.accountId).toBe('a'); expect(window.props.defaultTo).toBe('sam@peer.test');
    expect(window.props.defaultBody).toContain('&lt;img'); expect(window.props.defaultBody).not.toContain('<img');
    expect(window.props.referencesHeader).toBe('<root@peer> <source@peer>'); expect(window.props.inReplyTo).toBe('<source@peer>');
  });
  it('focuses an already edited reply even after autosave changes its window key', async () => {
    await reviewSuggestedReply(ref);
    const window = useWindowStore.getState().windows[0];
    useWindowStore.getState().setWindowKey(window.id, 'compose:a|Drafts|99');
    await reviewSuggestedReply(ref);
    expect(useWindowStore.getState().windows).toHaveLength(1);
    expect(api.intelligence.suggestion).toHaveBeenCalledTimes(1);
  });
  it('never opens a stale suggestion after the source thread changes', async () => {
    vi.mocked(api.intelligence.suggestion).mockResolvedValue(null);
    await expect(reviewSuggestedReply(ref)).rejects.toThrow('suggestion has changed');
    expect(useWindowStore.getState().windows).toHaveLength(0);
  });
});
