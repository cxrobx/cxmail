import { cn } from "@/lib/utils";
import type { Nudge } from "@/types/email";

interface NudgeBadgeProps {
  nudge: Nudge;
  onDismiss: (nudge: Nudge) => void;
  className?: string;
}

function label(nudge: Nudge): string {
  const days = nudge.days_ago;
  const ago = days === 1 ? "1 day ago" : `${days} days ago`;
  return nudge.kind === "follow_up"
    ? `Sent ${ago}. Follow up?`
    : `Received ${ago}. Reply?`;
}

/**
 * The stalled-conversation prompt, rendered in place of the row's date.
 *
 * Replacing the date rather than sitting beside it is deliberate: the message
 * list is virtualized, and `measureElement` caches row heights. A badge that
 * adds a line — or wraps on a narrow window — makes rows overlap, which is the
 * long-standing failure mode of this list. Everything here is `shrink-0` and
 * single-line for the same reason. The date it displaces is not lost
 * information: the age *is* the message, stated in the units that matter.
 */
export default function NudgeBadge({ nudge, onDismiss, className }: NudgeBadgeProps) {
  return (
    <span
      className={cn(
        "group/nudge flex shrink-0 items-center gap-1 whitespace-nowrap text-warning",
        className,
      )}
      title={
        nudge.kind === "follow_up"
          ? `You wrote to ${nudge.counterpart_email} and haven't heard back`
          : `${nudge.counterpart_email} is waiting on a reply`
      }
    >
      {label(nudge)}
      {/* Not a <button>: the row itself is one, and nesting buttons is invalid
          HTML that browsers resolve by dropping the inner element. */}
      <span
        role="button"
        tabIndex={-1}
        aria-label="Dismiss nudge"
        className="cursor-default px-0.5 opacity-0 transition-opacity hover:text-content group-hover/nudge:opacity-70"
        onClick={(e) => {
          // Without this the row's own onClick opens the message, so the nudge
          // is dismissed AND the thread opens — two outcomes from one click.
          e.stopPropagation();
          e.preventDefault();
          onDismiss(nudge);
        }}
      >
        ×
      </span>
    </span>
  );
}
