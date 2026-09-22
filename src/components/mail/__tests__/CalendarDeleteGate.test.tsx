/**
 * Deleting a calendar event can put real mail in other people's inboxes, so it
 * goes through a confirmation the way every other outbound path in this app
 * does. Before this gate existed, clicking Delete fired `SendUpdates::All`
 * immediately — one click, cancellation notices to a client's whole team, no
 * way back. These tests pin the two halves that matter: nothing is sent before
 * a human confirms, and the choice they make is the choice that reaches the API.
 *
 * See gotcha #50.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import CalendarView from "@/components/mail/CalendarView";
import { api } from "@/lib/tauri";
import type { CalendarEvent } from "@/types/email";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));
vi.mock("@/components/mail/EventComposeModal", () => ({ default: () => null }));
vi.mock("@/components/mail/ZoomSettings", () => ({ default: () => null }));

vi.mock("@/lib/tauri", () => ({
  api: {
    calendar: {
      listByRange: vi.fn(),
      connectionStatus: vi.fn(),
      connect: vi.fn(),
      syncNow: vi.fn(),
      dismiss: vi.fn(),
      delete: vi.fn(),
      sendInvites: vi.fn(),
      resolveConflict: vi.fn(),
    },
  },
}));

vi.mock("@/stores/mailStore", () => ({
  useMailStore: () => ({ openCalendarMessage: vi.fn(), markMessageRead: vi.fn() }),
}));
vi.mock("@/stores/accountStore", () => ({
  useAccountStore: (selector: (s: unknown) => unknown) => selector({ accounts: [] }),
}));
vi.mock("@/stores/uiStore", () => ({
  useUIStore: (selector: (s: unknown) => unknown) =>
    selector({ toggleCalendarFullScreen: vi.fn() }),
}));
vi.mock("@/stores/windowStore", () => ({
  useWindowStore: (selector: (s: unknown) => unknown) => selector({ openWindow: vi.fn() }),
}));

const listByRange = api.calendar.listByRange as ReturnType<typeof vi.fn>;
const deleteEvent = api.calendar.delete as ReturnType<typeof vi.fn>;

/** Today at noon, so the event lands in the month the view opens on. */
function todayAt(hour: number): string {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour).toISOString();
}

function event(overrides: Partial<CalendarEvent> = {}): CalendarEvent {
  return {
    id: 1,
    kind: "gcal",
    account_id: "acct",
    folder_name: "",
    message_uid: 0,
    event_uid: "uid-1",
    summary: "Northwind quarterly review",
    description: null,
    location: null,
    dtstart: todayAt(10),
    dtend: todayAt(11),
    organizer_name: null,
    organizer_email: "chris@cxventures.io",
    status: "confirmed",
    method: null,
    rsvp_status: "accepted",
    raw_ics: null,
    source: "gcal",
    confidence: null,
    dismissed: false,
    created_at: todayAt(9),
    gcal_calendar_id: "primary",
    gcal_event_id: "evt-1",
    attendees_json: null,
    html_link: null,
    hangout_link: null,
    is_all_day: false,
    start_tz: null,
    sync_state: "synced",
    pending_notify: false,
    attendee_delivery: [
      {
        email: "dana@northwind.example",
        display_name: null,
        response_status: "needsAction",
        is_self: false,
        state: "sent",
      },
    ],
    cancellation_notifies: true,
    ...overrides,
  };
}

async function openDetail(evt: CalendarEvent) {
  listByRange.mockResolvedValue([evt]);
  const user = userEvent.setup();
  render(<CalendarView />);
  const row = await screen.findByText(evt.summary as string);
  await user.click(row);
  return user;
}

beforeEach(() => {
  vi.clearAllMocks();
  (api.calendar.connectionStatus as ReturnType<typeof vi.fn>).mockResolvedValue([]);
  deleteEvent.mockResolvedValue(undefined);
});

describe("calendar delete confirmation gate", () => {
  it("sends nothing when Delete is clicked — it only arms the confirmation", async () => {
    // The regression this exists for: Delete used to hit the API instantly, and
    // the API now defaults to notifying every guest.
    const user = await openDetail(event());
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));

    expect(deleteEvent).not.toHaveBeenCalled();
    expect(
      await screen.findByText(/Cancel this meeting and email 1 guest\?/i),
    ).toBeTruthy();
  });

  it("names the people who will actually be emailed", async () => {
    // A gate that does not say who it mails is not meaningfully a gate.
    const user = await openDetail(event());
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));
    // The address also appears in the guest list above, so scope to the dialog.
    const gate = within(screen.getByRole("alertdialog"));
    expect(gate.getByText("dana@northwind.example")).toBeTruthy();
  });

  it("backing out leaves the event alone", async () => {
    const user = await openDetail(event());
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));
    await user.click(screen.getByRole("button", { name: /Keep event/i }));

    expect(deleteEvent).not.toHaveBeenCalled();
    expect(screen.queryByText(/Cancel this meeting and email/i)).toBeNull();
  });

  it("passes the human's choice through verbatim", async () => {
    const user = await openDetail(event());
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));
    await user.click(screen.getByRole("button", { name: /Delete and notify/i }));
    expect(deleteEvent).toHaveBeenCalledWith(1, true);
  });

  it("offers an explicit silent delete, and it is never the default action", async () => {
    // Deleting a duplicate or a test event should not mail anyone — but that has
    // to be a deliberate second choice, not what happens if you click through.
    const user = await openDetail(event());
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));
    await user.click(screen.getByRole("button", { name: /Delete without notifying/i }));
    expect(deleteEvent).toHaveBeenCalledWith(1, false);
  });

  it("does not promise to email anyone when no mail will be sent", async () => {
    // `cancellation_notifies` is decided in Rust. When it is false the gate still
    // confirms the destructive act, but must not claim guests are being told —
    // that would be the same false confidence this feature set out to remove.
    const user = await openDetail(event({ cancellation_notifies: false }));
    await user.click(screen.getByRole("button", { name: /^Delete$/i }));

    expect(screen.queryByText(/Cancel this meeting and email/i)).toBeNull();
    expect(screen.getByText(/No one will be emailed/i)).toBeTruthy();

    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: /^Delete event$/i,
      }),
    );
    expect(deleteEvent).toHaveBeenCalledWith(1, false);
  });
});
