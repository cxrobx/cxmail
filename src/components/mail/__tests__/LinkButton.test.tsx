import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { Editor } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import Link from '@tiptap/extension-link';
import LinkButton from '@/components/mail/LinkButton';

// Regression: the toolbar used window.prompt, which Tauri's webview (wry) never
// answers — it returns null and the button did nothing. The control must not
// depend on it at all.
function makeEditor(html: string) {
  return new Editor({ extensions: [StarterKit, Link.configure({ openOnClick: false })], content: html });
}

function openAndSubmit(url: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Insert link' }));
  const input = screen.getByRole('textbox', { name: 'Link URL' });
  fireEvent.change(input, { target: { value: url } });
  fireEvent.keyDown(input, { key: 'Enter' });
}

let editor: Editor;
afterEach(() => {
  cleanup();
  editor?.destroy();
});

describe('LinkButton', () => {
  it('links the selected text without window.prompt', () => {
    const original = window.prompt;
    window.prompt = () => null; // what the Tauri webview does
    try {
      editor = makeEditor('<p>see the proposal here</p>');
      editor.commands.setTextSelection({ from: 9, to: 17 }); // "proposal"
      render(<LinkButton editor={editor} />);
      openAndSubmit('cxventures.io/proposals');
      expect(editor.getHTML()).toContain('<a target="_blank" rel="noopener noreferrer nofollow" href="https://cxventures.io/proposals">proposal</a>');
      expect(screen.queryByRole('dialog')).toBeNull();
    } finally {
      window.prompt = original;
    }
  });

  it('inserts the address as link text when nothing is selected', () => {
    editor = makeEditor('<p>hi </p>');
    editor.commands.setTextSelection(4);
    render(<LinkButton editor={editor} />);
    openAndSubmit('https://x.com');
    expect(editor.getHTML()).toMatch(/href="https:\/\/x\.com">https:\/\/x\.com<\/a>/);
  });

  it('refuses an unsafe URL and keeps the field open', () => {
    editor = makeEditor('<p>click</p>');
    editor.commands.setTextSelection({ from: 1, to: 6 });
    render(<LinkButton editor={editor} />);
    openAndSubmit('javascript:alert(1)');
    expect(editor.getHTML()).not.toContain('<a');
    expect(screen.getByRole('dialog')).toBeTruthy();
  });

  it('prefills an existing link and removes it', () => {
    editor = makeEditor('<p><a href="https://old.com">old</a></p>');
    editor.commands.setTextSelection(2);
    render(<LinkButton editor={editor} />);
    fireEvent.click(screen.getByRole('button', { name: 'Insert link' }));
    expect((screen.getByRole('textbox', { name: 'Link URL' }) as HTMLInputElement).value).toBe('https://old.com');
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    expect(editor.getHTML()).not.toContain('<a');
  });
});

describe('LinkButton open signal (⌘K)', () => {
  it('opens on a signal change but not on mount', () => {
    editor = makeEditor('<p>see the proposal here</p>');
    editor.commands.setTextSelection({ from: 9, to: 17 });
    const { rerender } = render(<LinkButton editor={editor} openSignal={0} />);
    expect(screen.queryByRole('dialog')).toBeNull();
    rerender(<LinkButton editor={editor} openSignal={1} />);
    const input = screen.getByRole('textbox', { name: 'Link URL' });
    fireEvent.change(input, { target: { value: 'x.com' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(editor.getHTML()).toContain('href="https://x.com">proposal</a>');
  });
});
