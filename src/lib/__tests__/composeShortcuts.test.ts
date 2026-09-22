import { describe, it, expect, afterEach } from 'vitest';
import { Editor } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { ComposeShortcuts, isLinkShortcut } from '@/lib/composeShortcuts';

// jsdom is not a Mac, so ProseMirror reads "Mod" as Ctrl here.
let editor: Editor;
afterEach(() => editor?.destroy());

function make(html: string, from: number, to: number) {
  editor = new Editor({
    element: document.createElement('div'),
    extensions: [StarterKit.configure({ link: { openOnClick: false } }), ComposeShortcuts],
    content: html,
  });
  editor.commands.setTextSelection({ from, to });
  return editor;
}

function press(init: KeyboardEventInit) {
  const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  editor.view.dom.dispatchEvent(evt);
  return evt;
}

describe('compose formatting shortcuts', () => {
  it('keeps the StarterKit ones: bold, italic, underline, lists', () => {
    make('<p>hello world</p>', 1, 6);
    press({ key: 'b', ctrlKey: true });
    press({ key: 'i', ctrlKey: true });
    press({ key: 'u', ctrlKey: true });
    expect(editor.getHTML()).toContain('<strong><em><u>hello</u></em></strong>');
    press({ key: '8', ctrlKey: true, shiftKey: true });
    expect(editor.getHTML()).toMatch(/^<ul>/);
  });

  it('adds strikethrough on Shift+X, quote on Shift+9, and clears marks on Mod-\\', () => {
    make('<p>hello world</p>', 1, 6);
    press({ key: 'x', ctrlKey: true, shiftKey: true });
    expect(editor.getHTML()).toContain('<s>hello</s>');
    press({ key: 'b', ctrlKey: true });
    const clear = press({ key: '\\', ctrlKey: true });
    expect(clear.defaultPrevented).toBe(true); // tells the app-wide ⌘\ to stand down
    expect(editor.getHTML()).toBe('<p>hello world</p>');
    press({ key: '9', ctrlKey: true, shiftKey: true });
    expect(editor.getHTML()).toMatch(/^<blockquote>/);
  });

  it('registers Link and Underline once', () => {
    make('<p>x</p>', 1, 1);
    const names = editor.extensionManager.extensions.map((e) => e.name);
    expect(names.filter((n) => n === 'link')).toHaveLength(1);
    expect(names.filter((n) => n === 'underline')).toHaveLength(1);
  });
});

describe('isLinkShortcut', () => {
  const k = (o: Partial<KeyboardEvent>) => ({ key: 'k', metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...o });
  it('is Cmd+K or Ctrl+K and nothing else', () => {
    expect(isLinkShortcut(k({ metaKey: true }))).toBe(true);
    expect(isLinkShortcut(k({ ctrlKey: true, key: 'K' }))).toBe(true);
    expect(isLinkShortcut(k({}))).toBe(false);
    expect(isLinkShortcut(k({ metaKey: true, shiftKey: true }))).toBe(false);
    expect(isLinkShortcut(k({ metaKey: true, key: 'j' }))).toBe(false);
  });
});
