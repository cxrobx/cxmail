import { cn } from "@/lib/utils";

interface KbdProps {
  children: React.ReactNode;
  className?: string;
}

/** Keycap chip shared by the shortcut sheet and command palette hints. */
export default function Kbd({ children, className }: KbdProps) {
  return (
    <kbd
      className={cn(
        "inline-flex min-w-[24px] items-center justify-center rounded border border-border bg-surface px-1.5 py-0.5 text-xs font-medium text-content-secondary",
        className,
      )}
    >
      {children}
    </kbd>
  );
}
