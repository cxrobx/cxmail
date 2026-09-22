import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useKeyboardShortcuts } from '@/hooks/useKeyboardShortcuts';
import { useMailStore } from '@/stores/mailStore';
import { api } from '@/lib/tauri';

vi.mock('@/lib/tauri', () => ({
  api: { messages: { toggleStar: vi.fn(() => Promise.resolve()), toggleMute: vi.fn(() => Promise.resolve()), togglePin: vi.fn(() => Promise.resolve()), delete: vi.fn(() => Promise.resolve()) } },
}));

const msg = { uid: 7, account_id: 'a', folder_name: 'INBOX', is_flagged: false, is_pinned: false };

let editable: HTMLDivElement;
beforeEach(() => {
  useMailStore.setState({
    selectedAccountId: 'a', selectedFolder: 'INBOX', selectedMessageUid: 7,
    selectedMessageAccountId: 'a', messages: [msg, { ...msg, uid: 8 }] as never,
  });
  editable = document.createElement('div');
  editable.contentEditable = 'true';
  // jsdom does not implement isContentEditable.
  Object.defineProperty(editable, 'isContentEditable', { value: true });
  document.body.appendChild(editable);
});
afterEach(() => {
  editable.remove();
  vi.clearAllMocks();
});

function press(target: EventTarget, init: KeyboardEventInit, preventFirst = false) {
  const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  // Stand in for a closer handler (the compose editor) claiming the key.
  if (preventFirst) target.addEventListener('keydown', (e) => e.preventDefault(), { once: true });
  target.dispatchEvent(evt);
  return evt;
}

function setup() {
  const onCommandPalette = vi.fn();
  renderHook(() => useKeyboardShortcuts({ onCommandPalette, onShortcutSheet: vi.fn() }));
  return { onCommandPalette };
}

describe('useKeyboardShortcuts while typing', () => {
  it('does not run message actions on held-⌘ letters', () => {
    setup();
    press(editable, { key: 's', metaKey: true });
    press(editable, { key: 'm', metaKey: true });
    press(editable, { key: 'j', metaKey: true });
    expect(api.messages.toggleStar).not.toHaveBeenCalled();
    expect(api.messages.toggleMute).not.toHaveBeenCalled();
    expect(useMailStore.getState().selectedMessageUid).toBe(7);
  });

  it('leaves ⌘K to the editor once it has claimed it, and opens the palette otherwise', () => {
    const { onCommandPalette } = setup();
    press(editable, { key: 'k', metaKey: true }, true);
    expect(onCommandPalette).not.toHaveBeenCalled();
    press(editable, { key: 'k', metaKey: true });
    expect(onCommandPalette).toHaveBeenCalledTimes(1);
  });

  it('still runs bare-key message actions outside an editable', () => {
    setup();
    press(document.body, { key: 's' });
    expect(api.messages.toggleStar).toHaveBeenCalledTimes(1);
  });
});
