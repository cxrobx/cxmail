import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import ComposeModal from '@/components/mail/ComposeModal';
import { clampDockOffset } from '@/hooks/useDockDrag';

// Every IPC call resolves to an empty list: the composer mounts, fetches its
// signatures/templates/tracking config, and finds nothing. Only the chrome is
// under test here.
vi.mock('@/lib/tauri', () => {
  const fn = () => Promise.resolve([]);
  const ns = new Proxy({}, { get: () => fn });
  return { api: new Proxy({}, { get: () => ns }) };
});

const BOX = { w: 560, h: 500 };
let rectSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  // jsdom has no layout; give the compose box a real docked size.
  rectSpy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    width: BOX.w, height: BOX.h, top: 0, left: 0, right: BOX.w, bottom: BOX.h, x: 0, y: 0,
    toJSON: () => ({}),
  } as DOMRect);
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: 900 });
});
afterEach(() => rectSpy.mockRestore());

function mount() {
  render(<ComposeModal onClose={() => {}} />);
  const header = screen.getByText('New Message').parentElement as HTMLElement;
  const box = header.parentElement as HTMLElement;
  return { header, box };
}

describe('docked composer drag', () => {
  it('moves the box when its header is dragged — in the page, not the OS window', () => {
    const { header, box } = mount();
    // gotcha #51: this attribute would slide CXMail across the desktop.
    expect(header.hasAttribute('data-tauri-drag-region')).toBe(false);
    expect(box.style.right).toBe('');
    expect(box.className).toContain('rounded-t-lg');

    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0 });
    fireEvent.mouseMove(document, { clientX: 700, clientY: 580 });
    // Docked at right 24 / bottom 0; moved 300 left and 300 up.
    expect(box.style.right).toBe('324px');
    expect(box.style.bottom).toBe('300px');
    // Off the bottom edge it rounds every corner.
    expect(box.className).toContain('rounded-lg');
    expect(box.className).not.toContain('rounded-t-lg');

    // Released: further movement must not keep dragging it.
    fireEvent.mouseUp(document);
    fireEvent.mouseMove(document, { clientX: 100, clientY: 100 });
    expect(box.style.right).toBe('324px');
  });

  it('covers the page from MOUSEDOWN, before any move, so the email iframe cannot swallow the drag', () => {
    // Raised on the first move instead, a first step that clears the box's
    // edge lands on the reading pane's iframe, which eats every mousemove and
    // the mouseup: the box never moves (seen in Chromium).
    const { header } = mount();
    expect(screen.queryByTestId('compose-drag-overlay')).toBeNull();
    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0, detail: 1 });
    expect(screen.getByTestId('compose-drag-overlay')).toBeTruthy();
    fireEvent.mouseUp(document);
    expect(screen.queryByTestId('compose-drag-overlay')).toBeNull();
  });

  it('cannot be dragged off screen', () => {
    const { header, box } = mount();
    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0 });
    fireEvent.mouseMove(document, { clientX: -5000, clientY: -5000 });
    // Pinned to the top-left: right = 1440 - 560, bottom = 900 - 500.
    expect(box.style.right).toBe('880px');
    expect(box.style.bottom).toBe('400px');
    fireEvent.mouseMove(document, { clientX: 9000, clientY: 9000 });
    expect(box.style.right).toBe('0px');
    expect(box.style.bottom).toBe('0px');
    fireEvent.mouseUp(document);
  });

  it('re-clamps when the window shrinks under a moved box', () => {
    const { header, box } = mount();
    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0 });
    fireEvent.mouseMove(document, { clientX: 300, clientY: 480 });
    fireEvent.mouseUp(document);
    expect(box.style.right).toBe('724px');
    act(() => {
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1000 });
      window.dispatchEvent(new Event('resize'));
    });
    expect(box.style.right).toBe('440px'); // 1000 - 560
  });

  it('double-clicking the header puts it back in the dock', () => {
    // Read off the second mousedown's click count: the overlay takes each
    // click's mouseup, so a real `dblclick` fires on the box, never the header.
    const { header, box } = mount();
    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0, detail: 1 });
    fireEvent.mouseMove(document, { clientX: 700, clientY: 580 });
    fireEvent.mouseUp(document);
    fireEvent.mouseDown(header, { clientX: 700, clientY: 580, button: 0, detail: 1 });
    fireEvent.mouseUp(document);
    fireEvent.mouseDown(header, { clientX: 700, clientY: 580, button: 0, detail: 2 });
    fireEvent.mouseUp(document);
    expect(box.style.right).toBe('');
    expect(box.style.bottom).toBe('');
    expect(box.className).toContain('rounded-t-lg');
  });

  it('does not start a drag from the header buttons', () => {
    const { header, box } = mount();
    const minimize = header.querySelectorAll('button')[0] as HTMLElement;
    fireEvent.mouseDown(minimize, { clientX: 1000, clientY: 880, button: 0 });
    fireEvent.mouseMove(document, { clientX: 200, clientY: 200 });
    expect(box.style.right).toBe('');
    expect(screen.queryByTestId('compose-drag-overlay')).toBeNull();
  });

  it('keeps its place across minimize and restore', () => {
    const { header } = mount();
    fireEvent.mouseDown(header, { clientX: 1000, clientY: 880, button: 0 });
    fireEvent.mouseMove(document, { clientX: 700, clientY: 580 });
    fireEvent.mouseUp(document);
    // Minimize is the last button before close when there is no pop-out.
    const buttons = header.querySelectorAll('button');
    fireEvent.click(buttons[buttons.length - 2]);
    // The minimized bar docks, as it always has.
    const bar = screen.getByText('New Message').closest('div.fixed') as HTMLElement;
    expect(bar.className).toContain('w-[320px]');
    expect(bar.style.right).toBe('');
    expect(screen.queryByPlaceholderText('Subject')).toBeNull();

    fireEvent.click(screen.getByText('New Message'));
    expect(screen.getByPlaceholderText('Subject')).toBeTruthy();
    const restored = screen.getByText('New Message').parentElement!.parentElement as HTMLElement;
    expect(restored.style.right).toBe('324px');
    expect(restored.style.bottom).toBe('300px');
  });
});

describe('clampDockOffset', () => {
  it('keeps the whole box inside the viewport', () => {
    const vp = { w: 1000, h: 800 };
    expect(clampDockOffset({ right: -10, bottom: -10 }, BOX, vp)).toEqual({ right: 0, bottom: 0 });
    expect(clampDockOffset({ right: 900, bottom: 900 }, BOX, vp)).toEqual({ right: 440, bottom: 300 });
    expect(clampDockOffset({ right: 100, bottom: 50 }, BOX, vp)).toEqual({ right: 100, bottom: 50 });
  });

  it('pins a box bigger than the viewport to the bottom-right instead of going negative', () => {
    expect(clampDockOffset({ right: 50, bottom: 50 }, BOX, { w: 400, h: 300 })).toEqual({ right: 0, bottom: 0 });
  });
});
