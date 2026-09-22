import { useCallback, useRef, useState, type ReactNode } from "react";
import { Minus, X } from "lucide-react";
import { useWindowStore } from "@/stores/windowStore";

interface FloatingWindowProps {
  id: string;
  title: string;
  position: { x: number; y: number };
  size: { w: number; h: number };
  zIndex: number;
  children: ReactNode;
}

const MIN_W = 400;
const MIN_H = 300;

const EDGE_CURSOR: Record<string, string> = {
  n: "n-resize",
  s: "s-resize",
  e: "e-resize",
  w: "w-resize",
  ne: "ne-resize",
  nw: "nw-resize",
  se: "se-resize",
  sw: "sw-resize",
};

export default function FloatingWindow({ id, title, position, size, zIndex, children }: FloatingWindowProps) {
  const { closeWindow, updatePosition, updateSize, focusWindow, minimizeWindow } = useWindowStore();
  const [isResizing, setIsResizing] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [activeEdge, setActiveEdge] = useState("");
  const dragRef = useRef<{ startX: number; startY: number; startPosX: number; startPosY: number }>({
    startX: 0, startY: 0, startPosX: 0, startPosY: 0,
  });
  const resizeRef = useRef<{ startX: number; startY: number; startW: number; startH: number; startPosX: number; startPosY: number; edge: string }>({
    startX: 0, startY: 0, startW: 0, startH: 0, startPosX: 0, startPosY: 0, edge: "",
  });

  const handleDragStart = useCallback((e: React.MouseEvent) => {
    // Don't drag if clicking buttons
    if ((e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    setIsDragging(true);
    dragRef.current = { startX: e.clientX, startY: e.clientY, startPosX: position.x, startPosY: position.y };

    const onMouseMove = (ev: MouseEvent) => {
      const dx = ev.clientX - dragRef.current.startX;
      const dy = ev.clientY - dragRef.current.startY;
      updatePosition(id, {
        x: dragRef.current.startPosX + dx,
        y: Math.max(0, dragRef.current.startPosY + dy),
      });
    };

    const onMouseUp = () => {
      setIsDragging(false);
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }, [id, position, updatePosition]);

  const handleResizeStart = useCallback((e: React.MouseEvent, edge: string) => {
    e.preventDefault();
    e.stopPropagation();
    setIsResizing(true);
    setActiveEdge(edge);
    resizeRef.current = { startX: e.clientX, startY: e.clientY, startW: size.w, startH: size.h, startPosX: position.x, startPosY: position.y, edge };

    const onMouseMove = (ev: MouseEvent) => {
      const dx = ev.clientX - resizeRef.current.startX;
      const dy = ev.clientY - resizeRef.current.startY;
      const { startW, startH, startPosX, startPosY, edge: e } = resizeRef.current;

      let newW = startW;
      let newH = startH;
      let newX = startPosX;
      let newY = startPosY;

      if (e.includes("e")) newW = Math.max(MIN_W, startW + dx);
      if (e.includes("s")) newH = Math.max(MIN_H, startH + dy);
      if (e.includes("w")) {
        newW = Math.max(MIN_W, startW - dx);
        if (newW > MIN_W) newX = startPosX + dx;
      }
      if (e.includes("n")) {
        newH = Math.max(MIN_H, startH - dy);
        if (newH > MIN_H) newY = startPosY + dy;
      }

      updateSize(id, { w: newW, h: newH });
      updatePosition(id, { x: newX, y: newY });
    };

    const onMouseUp = () => {
      setIsResizing(false);
      setActiveEdge("");
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }, [id, size, position, updateSize, updatePosition]);

  return (
    <div
      style={{
        position: "fixed",
        left: position.x,
        top: position.y,
        width: size.w,
        height: size.h,
        zIndex,
      }}
      onMouseDown={() => focusWindow(id)}
      className="flex flex-col overflow-hidden rounded-xl border border-border-subtle bg-base-solid shadow-2xl"
    >
      {/* Title bar - drag handle */}
      <div
        onMouseDown={handleDragStart}
        className={`flex h-9 shrink-0 items-center justify-between border-b border-border-subtle bg-elevated px-3 ${isDragging ? "cursor-grabbing" : "cursor-grab"}`}
      >
        <span className="truncate text-xs font-medium text-content-secondary">{title}</span>
        <div className="flex items-center gap-1">
          <button
            onClick={(e) => { e.stopPropagation(); minimizeWindow(id); }}
            className="rounded p-1 text-content-muted hover:bg-surface hover:text-content"
          >
            <Minus className="h-3 w-3" />
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); closeWindow(id); }}
            className="rounded p-1 text-content-muted hover:bg-red-500/20 hover:text-red-400"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      </div>

      {/* Content */}
      <div className={`flex-1 overflow-auto ${isResizing || isDragging ? "select-none" : ""}`}>
        {children}
      </div>

      {/* Drag/resize overlay — covers iframes so mouseup always reaches the document */}
      {(isResizing || isDragging) && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            zIndex: 2147483000,
            cursor: isDragging ? "grabbing" : EDGE_CURSOR[activeEdge] || "default",
            background: "transparent",
          }}
        />
      )}

      {/* Resize handles */}
      <div className="absolute inset-0 pointer-events-none">
        {/* Edges */}
        <div className="absolute top-0 left-2 right-2 h-1 cursor-n-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "n")} />
        <div className="absolute bottom-0 left-2 right-2 h-1 cursor-s-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "s")} />
        <div className="absolute left-0 top-2 bottom-2 w-1 cursor-w-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "w")} />
        <div className="absolute right-0 top-2 bottom-2 w-1 cursor-e-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "e")} />
        {/* Corners */}
        <div className="absolute top-0 left-0 h-2 w-2 cursor-nw-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "nw")} />
        <div className="absolute top-0 right-0 h-2 w-2 cursor-ne-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "ne")} />
        <div className="absolute bottom-0 left-0 h-2 w-2 cursor-sw-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "sw")} />
        <div className="absolute bottom-0 right-0 h-2 w-2 cursor-se-resize pointer-events-auto" onMouseDown={(e) => handleResizeStart(e, "se")} />
      </div>
    </div>
  );
}
