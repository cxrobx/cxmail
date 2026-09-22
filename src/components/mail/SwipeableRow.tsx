import { Archive, Mail, MailOpen } from "lucide-react";
import { cn } from "@/lib/utils";
import { useSwipeAction } from "@/hooks/useSwipeAction";

interface SwipeableRowProps {
  messageId: string;
  isRead: boolean;
  canArchive: boolean;
  onArchive: () => void;
  onToggleRead: () => void;
  children: React.ReactNode;
}

const SNAP_BACK_MS = 200;
const COMMIT_GLIDE_MS = 180;

export default function SwipeableRow({
  messageId,
  isRead,
  canArchive,
  onArchive,
  onToggleRead,
  children,
}: SwipeableRowProps) {
  const { offset, revealed, phase, bind } = useSwipeAction({
    messageId,
    isRead,
    canArchive,
    onArchive,
    onToggleRead,
  });

  const isSwiping = phase !== "idle";
  const transitionMs =
    phase === "committing" ? COMMIT_GLIDE_MS : phase === "snapping" ? SNAP_BACK_MS : 0;
  const transition =
    phase === "tracking" ? "none" : `transform ${transitionMs}ms ease-out`;

  return (
    <div
      ref={bind.ref}
      className="relative h-full overflow-hidden"
      data-swiping={isSwiping ? "true" : undefined}
    >
      {/* Left panel — read/unread. Reveals on RIGHT swipe (offset > 0) as
          the row content slides right and exposes what's behind on the left.
          Only rendered when offset > 0 so it can't leak visually behind a
          translucent row background at rest. */}
      {offset > 0 && (
        <div
          aria-hidden
          className={cn(
            "absolute inset-y-0 left-0 flex items-center px-4 bg-[#3d3d3d] text-white overflow-hidden transition-opacity",
            revealed === "read" ? "opacity-100" : "opacity-50",
          )}
          style={{ width: offset }}
        >
          {isRead ? (
            <MailOpen className="h-4 w-4 mr-2 shrink-0" />
          ) : (
            <Mail className="h-4 w-4 mr-2 shrink-0" />
          )}
          <span className="text-sm font-medium whitespace-nowrap">
            {isRead ? "Mark unread" : "Mark read"}
          </span>
        </div>
      )}

      {/* Right panel — archive. Reveals on LEFT swipe (offset < 0) as the
          row content slides left and exposes what's behind on the right. */}
      {offset < 0 && (
        <div
          aria-hidden
          className={cn(
            "absolute inset-y-0 right-0 flex items-center justify-end px-4 bg-[#ff9f0a] text-white overflow-hidden transition-opacity",
            revealed === "archive" ? "opacity-100" : "opacity-50",
          )}
          style={{ width: -offset }}
        >
          <span className="text-sm font-medium whitespace-nowrap mr-2">Archive</span>
          <Archive className="h-4 w-4 shrink-0" />
        </div>
      )}

      {/* Row content. `relative` lifts it above the panels in stacking order;
          MessageListItem provides its own opaque background, so when offset=0
          the panels are visually invisible even if widths are non-zero. */}
      <div
        className="relative h-full"
        style={{
          transform: `translateX(${offset}px)`,
          transition,
        }}
      >
        {children}
      </div>
    </div>
  );
}
