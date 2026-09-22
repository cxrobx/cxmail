import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import NudgeBadge from "@/components/mail/NudgeBadge";
import type { Nudge } from "@/types/email";

function nudge(overrides: Partial<Nudge> = {}): Nudge {
  return {
    kind: "follow_up",
    account_id: "acct-1",
    folder_name: "INBOX",
    uid: 225,
    thread_key: "<thread@x>",
    subject: "Northwind Company / CX Ventures Meeting Follow Up",
    counterpart_email: "dana@northwind.example",
    counterpart_name: "Dana Robertson",
    date: "2026-07-31T20:41:51+00:00",
    days_ago: 5,
    ...overrides,
  };
}

describe("NudgeBadge", () => {
  it("matches the phrasing for each lane", () => {
    const { rerender } = render(<NudgeBadge nudge={nudge()} onDismiss={vi.fn()} />);
    expect(screen.getByText(/Sent 5 days ago\. Follow up\?/)).toBeTruthy();

    rerender(<NudgeBadge nudge={nudge({ kind: "reply", days_ago: 4 })} onDismiss={vi.fn()} />);
    expect(screen.getByText(/Received 4 days ago\. Reply\?/)).toBeTruthy();
  });

  it("says 1 day, not 1 days", () => {
    render(<NudgeBadge nudge={nudge({ days_ago: 1 })} onDismiss={vi.fn()} />);
    expect(screen.getByText(/Sent 1 day ago\./)).toBeTruthy();
  });

  // The badge lives inside the row's own button. Without stopPropagation the
  // dismiss click also opens the message — two outcomes from one click.
  it("dismisses without letting the click reach the row", () => {
    const onDismiss = vi.fn();
    const onRowClick = vi.fn();
    render(
      <button onClick={onRowClick}>
        <NudgeBadge nudge={nudge()} onDismiss={onDismiss} />
      </button>,
    );

    fireEvent.click(screen.getByLabelText("Dismiss nudge"));
    expect(onDismiss).toHaveBeenCalledWith(expect.objectContaining({ kind: "follow_up" }));
    expect(onRowClick).not.toHaveBeenCalled();
  });

  // Row height is cached by the virtualizer; a badge that wraps makes rows
  // overlap. Everything in it must stay on one line.
  it("never wraps", () => {
    const { container } = render(
      <NudgeBadge
        nudge={nudge({ days_ago: 28, counterpart_email: "a-very-long-address@example.com" })}
        onDismiss={vi.fn()}
      />,
    );
    const badge = container.firstElementChild as HTMLElement;
    expect(badge.className).toContain("whitespace-nowrap");
    expect(badge.className).toContain("shrink-0");
  });
});
