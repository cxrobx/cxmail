import { useState } from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { ChevronDown, ChevronUp } from "lucide-react";
import type { EmailAddress } from "@/types/email";

/** A display name that differs from the address — only then is there anything to reveal. */
function hasHiddenAddress(a: EmailAddress): boolean {
  return !!a.name && a.name.trim().toLowerCase() !== a.email.toLowerCase();
}

/**
 * Hovering a name shows its address. Without it, two recipients with the same
 * name ("Glendora Robinson, Glendora Robinson") look identical.
 */
export function AddressName({
  name,
  email,
  className,
  onClick,
}: {
  name: string | null | undefined;
  email: string | null | undefined;
  className?: string;
  onClick?: () => void;
}) {
  const label = name || email || "Unknown";
  if (!email || !name || name.trim().toLowerCase() === email.toLowerCase()) {
    return <span className={className}>{label}</span>;
  }
  return (
    <Tooltip.Root delayDuration={150}>
      <Tooltip.Trigger asChild>
        <span className={className} onClick={onClick} data-address={email}>
          {label}
        </span>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side="bottom"
          sideOffset={4}
          className="z-[200] select-text rounded-md border border-border-subtle bg-elevated px-2 py-1 text-xs text-content shadow-lg"
        >
          {email}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function AddressRow({
  label,
  addresses,
  expanded,
  onToggle,
}: {
  label: string;
  addresses: EmailAddress[];
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <p>
      <span className="text-content-muted">{label}:</span>{" "}
      {addresses.map((a, i) => (
        <span key={`${a.email}-${i}`}>
          {i > 0 && ", "}
          {expanded ? (
            <span className="select-text">
              {a.name && hasHiddenAddress(a) ? (
                <>
                  {a.name} <span className="text-content-muted">&lt;{a.email}&gt;</span>
                </>
              ) : (
                a.email
              )}
            </span>
          ) : (
            <AddressName
              name={a.name}
              email={a.email}
              onClick={onToggle}
              className={hasHiddenAddress(a) ? "cursor-pointer decoration-dotted underline-offset-2 hover:underline hover:text-content" : undefined}
            />
          )}
        </span>
      ))}
    </p>
  );
}

/**
 * The To / Cc lines of a message header. Names only by default; hover a name
 * for its address, or click a name (or the chevron) to show every address
 * inline as selectable text. Callers key this on the message so the expanded
 * state does not leak into the next one.
 */
export default function RecipientLines({ to, cc = [] }: { to: EmailAddress[]; cc?: EmailAddress[] }) {
  const [expanded, setExpanded] = useState(false);
  const toggle = () => setExpanded((v) => !v);
  const revealable = [...to, ...cc].some(hasHiddenAddress);
  if (to.length === 0 && cc.length === 0) return null;

  const chevron = revealable && (
    <button
      type="button"
      onClick={toggle}
      className="ml-1 inline-flex items-center rounded px-1 align-middle text-content-muted hover:bg-surface hover:text-content"
      aria-label={expanded ? "Hide addresses" : "Show addresses"}
      title={expanded ? "Hide addresses" : "Show addresses"}
    >
      {expanded ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
    </button>
  );

  return (
    <>
      {to.length > 0 && (
        <div className="flex items-start">
          <AddressRow label="To" addresses={to} expanded={expanded} onToggle={toggle} />
          {chevron}
        </div>
      )}
      {cc.length > 0 && (
        <div className="flex items-start">
          <AddressRow label="Cc" addresses={cc} expanded={expanded} onToggle={toggle} />
          {to.length === 0 && chevron}
        </div>
      )}
    </>
  );
}
