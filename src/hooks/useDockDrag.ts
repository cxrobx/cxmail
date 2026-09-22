import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

/**
 * Drag the docked composer around the window by its header.
 *
 * Position is carried as `right`/`bottom` offsets, NOT as a transform. The
 * composer contains `fixed inset-0` layers (the attachment warning), and a
 * transform makes an element the containing block for its fixed descendants —
 * that overlay would shrink to the compose box instead of covering the window
 * (gotcha #37's `position: fixed` lesson). Offsets also keep the box anchored
 * at its bottom edge, so it grows upward when Cc/Bcc opens, as it does docked.
 *
 * Dragging happens in the PAGE, never through `data-tauri-drag-region`, which
 * moves the OS window instead (gotcha #51).
 */

export interface DockOffset {
  right: number;
  bottom: number;
}

/** Where the composer sits when docked — Tailwind's `right-6 bottom-0`. */
export const DOCKED: DockOffset = { right: 24, bottom: 0 };

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/**
 * Keep the whole box inside the viewport. A box larger than the viewport pins
 * to the bottom-right rather than going negative, so the header can't be
 * pushed off the top of the window.
 */
export function clampDockOffset(
  offset: DockOffset,
  box: { w: number; h: number },
  viewport: { w: number; h: number },
): DockOffset {
  return {
    right: clamp(offset.right, 0, Math.max(0, viewport.w - box.w)),
    bottom: clamp(offset.bottom, 0, Math.max(0, viewport.h - box.h)),
  };
}

const same = (a: DockOffset | null, b: DockOffset | null) =>
  a === b || (!!a && !!b && a.right === b.right && a.bottom === b.bottom);

export function useDockDrag(boxRef: RefObject<HTMLElement | null>, enabled: boolean) {
  // null = docked. Held per compose, not persisted: a new message opens in the
  // dock, the same call SettingsDialog makes about re-centring on each open.
  const [offset, setOffset] = useState<DockOffset | null>(null);
  const [dragging, setDragging] = useState(false);
  const offsetRef = useRef(offset);
  offsetRef.current = offset;

  const measure = useCallback(() => {
    const rect = boxRef.current?.getBoundingClientRect();
    return {
      box: { w: rect?.width ?? 0, h: rect?.height ?? 0 },
      viewport: { w: window.innerWidth, h: window.innerHeight },
    };
  }, [boxRef]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (!enabled || e.button !== 0) return;
      // The header's buttons are buttons first.
      if ((e.target as HTMLElement).closest("button")) return;
      // Stops text selection, and keeps the caret in the editor while moving.
      e.preventDefault();
      // Double-click docks it. Read off the second mousedown's click count, NOT
      // a `dblclick` handler: the overlay below takes each click's mouseup, so
      // the browser fires `dblclick` on the box (the common ancestor of the
      // header and the overlay) and a header handler never hears it.
      if (e.detail >= 2) {
        setOffset(null);
        return;
      }
      const start = { x: e.clientX, y: e.clientY, ...(offsetRef.current ?? DOCKED) };
      const { box } = measure();
      // Raise the overlay NOW, not on the first move. Before it is up, a move
      // that leaves the box lands on the reading pane's email iframe, which
      // swallows every mousemove AND the mouseup — the box never moves and the
      // drag never ends (seen in Chromium whenever the first step cleared the
      // box's edge).
      setDragging(true);

      const onMove = (ev: MouseEvent) => {
        const next = clampDockOffset(
          { right: start.right - (ev.clientX - start.x), bottom: start.bottom - (ev.clientY - start.y) },
          box,
          { w: window.innerWidth, h: window.innerHeight },
        );
        setOffset((prev) => (same(prev, next) ? prev : next));
      };
      const onUp = () => {
        setDragging(false);
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [enabled, measure],
  );

  // Stay on screen when the window shrinks or the box grows (Cc/Bcc,
  // attachments) — both can push a moved box past an edge after the drag.
  // `enabled` goes false while minimized, which unmounts the box; re-running on
  // restore observes the NEW element and catches a resize that happened while
  // it was down.
  useEffect(() => {
    if (!enabled || !offset) return;
    const reclamp = () => {
      const current = offsetRef.current;
      if (!current) return;
      const { box, viewport } = measure();
      const next = clampDockOffset(current, box, viewport);
      if (!same(current, next)) setOffset(next);
    };
    reclamp();
    window.addEventListener("resize", reclamp);
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(reclamp) : null;
    if (ro && boxRef.current) ro.observe(boxRef.current);
    return () => {
      window.removeEventListener("resize", reclamp);
      ro?.disconnect();
    };
    // Re-subscribe only when docking or undocking, not on every move.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, offset === null, measure, boxRef]);

  return { offset, dragging, onMouseDown };
}
