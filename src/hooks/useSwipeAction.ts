import { useCallback, useEffect, useRef, useState } from "react";

const MIN_ABS_DELTA_X = 2;
const DIRECTION_LOCK_PX = 4;
const H_OVER_V_RATIO = 1.5;
// 300ms — was 150ms but caused mid-gesture snap-backs when the user's finger
// speed naturally dipped between strokes. Trackpad wheel events from a real
// two-finger swipe can space out further than spec suggests, especially on
// slow drags. 300ms is still fast enough to feel responsive at release.
const END_IDLE_MS = 300;
// 0.5px / run of 6 — was 1.5px / run of 3 but triggered false "decay" mid-gesture
// when the user slowed to a controlled pace (a deliberate, low-velocity drag
// has many sub-1.5px events). 0.5px = essentially "the finger truly stopped
// emitting motion," and 6 consecutive events of that is unambiguous lift-off.
const MOMENTUM_DECAY_PX = 0.5;
const MOMENTUM_DECAY_RUN = 6;
const THRESHOLD_FRAC = 0.25;
const THRESHOLD_MIN_PX = 80;
const THRESHOLD_MAX_PX = 140;
const SNAP_BACK_MS = 200;
const COMMIT_GLIDE_MS = 180;

export type SwipePhase = "idle" | "tracking" | "committing" | "snapping";
export type RevealedAction = "archive" | "read" | null;

export interface UseSwipeActionOptions {
  isRead: boolean;
  canArchive: boolean;
  messageId: string;
  onArchive: () => void;
  onToggleRead: () => void;
}

export interface UseSwipeActionResult {
  offset: number;
  revealed: RevealedAction;
  phase: SwipePhase;
  bind: { ref: (el: HTMLDivElement | null) => void };
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

interface VisualState {
  offset: number;
  revealed: RevealedAction;
}

const INITIAL_VISUAL: VisualState = { offset: 0, revealed: null };

function revealedFor(offset: number): RevealedAction {
  // Swipe direction reveals the OPPOSITE side's panel (iOS Mail convention):
  // right swipe (offset > 0) exposes the read/unread panel on the LEFT,
  // left swipe (offset < 0) exposes the archive panel on the RIGHT.
  if (offset > 0) return "read";
  if (offset < 0) return "archive";
  return null;
}

export function useSwipeAction(opts: UseSwipeActionOptions): UseSwipeActionResult {
  // offset+revealed in one state object so each wheel event triggers exactly
  // one re-render, not two. (At 120Hz trackpad polling, separate setState
  // calls compound into visible jank.)
  const [visual, setVisual] = useState<VisualState>(INITIAL_VISUAL);
  const [phase, setPhase] = useState<SwipePhase>("idle");
  const { offset, revealed } = visual;

  // Latest callbacks held in a ref so the wheel listener doesn't reattach
  // every time a parent re-renders.
  const optsRef = useRef(opts);
  useEffect(() => {
    optsRef.current = opts;
  }, [opts]);

  // Per-gesture mutable state (refs so updates don't trigger re-renders or
  // wheel-listener reattaches).
  const nodeRef = useRef<HTMLDivElement | null>(null);
  const phaseRef = useRef<SwipePhase>("idle");
  const offsetRef = useRef(0);
  const accumDeltaXRef = useRef(0);
  const directionLockedRef = useRef<0 | 1 | -1>(0);
  const rowWidthRef = useRef(0);
  const thresholdRef = useRef(0);
  const verticalRejectedRef = useRef(false);
  const decayRunRef = useRef(0);
  const offsetAtDecayStartRef = useRef(0);
  const idleTimerRef = useRef<number | null>(null);
  const animTimerRef = useRef<number | null>(null);

  const messageIdRef = useRef(opts.messageId);

  const setPhaseBoth = useCallback((p: SwipePhase) => {
    phaseRef.current = p;
    setPhase(p);
  }, []);

  const setOffsetBoth = useCallback((value: number) => {
    offsetRef.current = value;
    setVisual((prev) => {
      const nextRevealed = revealedFor(value);
      if (prev.offset === value && prev.revealed === nextRevealed) return prev;
      return { offset: value, revealed: nextRevealed };
    });
  }, []);

  const clearRevealed = useCallback(() => {
    setVisual((prev) => (prev.revealed === null ? prev : { ...prev, revealed: null }));
  }, []);

  const clearTimers = useCallback(() => {
    if (idleTimerRef.current !== null) {
      window.clearTimeout(idleTimerRef.current);
      idleTimerRef.current = null;
    }
    if (animTimerRef.current !== null) {
      window.clearTimeout(animTimerRef.current);
      animTimerRef.current = null;
    }
  }, []);

  const resetGestureState = useCallback(() => {
    accumDeltaXRef.current = 0;
    directionLockedRef.current = 0;
    rowWidthRef.current = 0;
    thresholdRef.current = 0;
    verticalRejectedRef.current = false;
    decayRunRef.current = 0;
    offsetAtDecayStartRef.current = 0;
  }, []);

  const resetAll = useCallback(() => {
    clearTimers();
    resetGestureState();
    offsetRef.current = 0;
    setVisual(INITIAL_VISUAL);
    setPhaseBoth("idle");
  }, [clearTimers, resetGestureState, setPhaseBoth]);

  // Reset when the row is reused for a different message (virtualization).
  useEffect(() => {
    if (messageIdRef.current !== opts.messageId) {
      messageIdRef.current = opts.messageId;
      resetAll();
    }
  }, [opts.messageId, resetAll]);

  const endGestureWithOffset = useCallback(
    (endOffset: number) => {
      clearTimers();
      const threshold = thresholdRef.current;
      const reduced = prefersReducedMotion();

      if (threshold > 0 && Math.abs(endOffset) >= threshold) {
        // Commit: snap to ±rowWidth and fire callback once.
        // Direction mapping: negative offset (left swipe) → archive,
        // positive offset (right swipe) → toggle read/unread.
        const direction = endOffset < 0 ? -1 : 1;
        const target = direction * (rowWidthRef.current || Math.abs(endOffset));
        if (direction < 0) optsRef.current.onArchive();
        else optsRef.current.onToggleRead();
        setPhaseBoth("committing");
        setOffsetBoth(target);
        const dur = reduced ? 0 : COMMIT_GLIDE_MS;
        animTimerRef.current = window.setTimeout(() => {
          animTimerRef.current = null;
          // Reset visual state. The row may have been removed from the list
          // already (archive); if it's still mounted (read/unread), this
          // hides the panel and recenters the content.
          resetAll();
        }, dur);
      } else {
        // Snap back to 0.
        setPhaseBoth("snapping");
        setOffsetBoth(0);
        clearRevealed();
        const dur = reduced ? 0 : SNAP_BACK_MS;
        animTimerRef.current = window.setTimeout(() => {
          animTimerRef.current = null;
          resetGestureState();
          setPhaseBoth("idle");
        }, dur);
      }
    },
    [clearTimers, resetAll, resetGestureState, setOffsetBoth, setPhaseBoth],
  );

  const handleWheel = useCallback(
    (event: WheelEvent) => {
      // Ignore while animating; the gesture is committed/snapping.
      if (phaseRef.current === "committing" || phaseRef.current === "snapping") {
        return;
      }

      // Pinch-zoom / shift-wheel-as-horizontal: leave alone.
      if (event.ctrlKey || event.shiftKey) return;
      // Only pixel-delta wheels.
      if (event.deltaMode !== 0) return;

      const dx = event.deltaX;
      const dy = event.deltaY;

      // Vertical-dominant rejection (with sticky latch for the rest of this burst).
      if (verticalRejectedRef.current) return;
      if (
        phaseRef.current === "idle" &&
        Math.abs(dy) > Math.abs(dx) * H_OVER_V_RATIO
      ) {
        verticalRejectedRef.current = true;
        return;
      }
      // Below noise floor for horizontal — but only reject if we're not yet tracking.
      if (phaseRef.current === "idle" && Math.abs(dx) < MIN_ABS_DELTA_X) return;

      // From here on, this is a horizontal-dominant event we want to consume.
      // Prevent WKWebView swipe-back and horizontal rubber-band immediately.
      event.preventDefault();

      // Measure row width and compute threshold once per gesture.
      if (rowWidthRef.current === 0) {
        const node = nodeRef.current;
        rowWidthRef.current = node?.offsetWidth ?? 0;
        thresholdRef.current = clamp(
          rowWidthRef.current * THRESHOLD_FRAC,
          THRESHOLD_MIN_PX,
          THRESHOLD_MAX_PX,
        );
      }

      // Accumulate and possibly enter tracking.
      accumDeltaXRef.current += dx;

      if (phaseRef.current === "idle") {
        if (Math.abs(accumDeltaXRef.current) < DIRECTION_LOCK_PX) {
          // Not yet enough to commit to a direction.
          return;
        }
        setPhaseBoth("tracking");
        // Seed offset to the accumulated delta so the row starts where the
        // user's fingers have already pulled it (no visual jump back to 0).
        // Sign convention: in this WKWebView setup, dx is OPPOSITE the
        // user's finger direction, so we negate. Offset then matches finger
        // direction: right swipe → positive offset → row slides right →
        // read/unread panel reveals on the left. Left swipe → negative
        // offset → row slides left → archive panel reveals on the right.
        let initialOffset = -accumDeltaXRef.current;
        if (!optsRef.current.canArchive && initialOffset < 0) initialOffset = 0;
        directionLockedRef.current = initialOffset < 0 ? -1 : 1;
        setOffsetBoth(initialOffset);
      } else {
        // tracking: update offset. Same negated-sign convention as above.
        let next = offsetRef.current - dx;
        // Clamp to direction sign; ignore mid-gesture flips.
        if (directionLockedRef.current === -1 && next > 0) next = 0;
        if (directionLockedRef.current === 1 && next < 0) next = 0;
        // canArchive=false → never let offset go negative (left swipe is archive).
        if (!optsRef.current.canArchive && next < 0) next = 0;
        setOffsetBoth(next);
      }

      // Momentum-decay bail: if the user lifted their fingers, the trailing
      // events have small |dx|. Once we see MOMENTUM_DECAY_RUN consecutive
      // small events, end the gesture using the offset captured at the start
      // of the decay run — not the inflated current offset.
      if (Math.abs(dx) < MOMENTUM_DECAY_PX) {
        if (decayRunRef.current === 0) {
          offsetAtDecayStartRef.current = offsetRef.current;
        }
        decayRunRef.current += 1;
        if (decayRunRef.current >= MOMENTUM_DECAY_RUN && phaseRef.current === "tracking") {
          endGestureWithOffset(offsetAtDecayStartRef.current);
          return;
        }
      } else {
        decayRunRef.current = 0;
      }

      // Reset the idle timer on every horizontally-dominant event.
      if (idleTimerRef.current !== null) {
        window.clearTimeout(idleTimerRef.current);
      }
      idleTimerRef.current = window.setTimeout(() => {
        idleTimerRef.current = null;
        verticalRejectedRef.current = false;
        if (phaseRef.current === "tracking") {
          endGestureWithOffset(offsetRef.current);
        } else {
          // Was still in idle (never crossed direction lock). Just reset accumulators.
          resetGestureState();
        }
      }, END_IDLE_MS);
    },
    [endGestureWithOffset, resetGestureState, setOffsetBoth, setPhaseBoth],
  );

  const bindRef = useCallback(
    (el: HTMLDivElement | null) => {
      nodeRef.current = el;
    },
    [],
  );

  // Attach listener once per DOM node. Using a separate effect that depends
  // only on nodeRef.current via a re-attach on remount; we capture the node
  // by reading nodeRef inside the effect on mount.
  useEffect(() => {
    const node = nodeRef.current;
    if (!node) return;
    const listener = (e: WheelEvent) => handleWheel(e);
    node.addEventListener("wheel", listener, { passive: false, capture: true });
    return () => {
      node.removeEventListener("wheel", listener, { capture: true } as EventListenerOptions);
    };
    // handleWheel is stable (useCallback over refs); nodeRef.current is set
    // by the ref callback before this effect mounts because React guarantees
    // ref callbacks fire before effects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handleWheel, opts.messageId]);

  // Cleanup on unmount.
  useEffect(() => {
    return () => {
      clearTimers();
    };
  }, [clearTimers]);

  return {
    offset,
    revealed,
    phase,
    bind: { ref: bindRef },
  };
}
