import { describe, expect, it } from "vitest";
import {
  DELIVERY_PRESENTATION,
  countUnsent,
  responseDetail,
} from "@/lib/attendeeDelivery";
import type { AttendeeDelivery, AttendeeDeliveryState } from "@/types/email";

function attendee(overrides: Partial<AttendeeDelivery> = {}): AttendeeDelivery {
  return {
    email: "dana@northwind.example",
    display_name: null,
    response_status: "needsAction",
    is_self: false,
    state: "unsent",
    ...overrides,
  };
}

const ALL_STATES: AttendeeDeliveryState[] = [
  "responded",
  "sent",
  "unsent",
  "unknown",
];

describe("attendee delivery presentation", () => {
  it("covers every state the backend can emit", () => {
    // A missing key is an undefined destructure at render time — a crash in the
    // event detail panel, not a visual glitch.
    for (const state of ALL_STATES) {
      expect(DELIVERY_PRESENTATION[state]).toBeDefined();
    }
    expect(Object.keys(DELIVERY_PRESENTATION).sort()).toEqual(
      [...ALL_STATES].sort(),
    );
  });

  it("never claims an unconfirmed invitation was RECEIVED", () => {
    // The entire point of the feature. We know Google accepted the send; we do
    // not know it survived a spam filter. Wording this as "received" rebuilds
    // the same false confidence `responseStatus: needsAction` already had.
    for (const state of ["sent", "unsent", "unknown"] as const) {
      const { summary, label } = DELIVERY_PRESENTATION[state];
      expect(`${summary} ${label}`.toLowerCase()).not.toContain("received");
    }
    // `responded` may say it, because a reply cannot exist without one.
    expect(DELIVERY_PRESENTATION.responded.label.toLowerCase()).toContain(
      "received",
    );
  });

  it("gives each state a visually distinct icon and colour", () => {
    // Four states that render identically would leave the panel exactly as
    // uninformative as the calendar it is meant to improve on.
    const icons = new Set(ALL_STATES.map((s) => DELIVERY_PRESENTATION[s].Icon));
    const summaries = new Set(
      ALL_STATES.map((s) => DELIVERY_PRESENTATION[s].summary),
    );
    expect(icons.size).toBe(ALL_STATES.length);
    expect(summaries.size).toBe(ALL_STATES.length);
    // Only the two states that need action or celebration carry a colour.
    expect(DELIVERY_PRESENTATION.unsent.className).toContain("warning");
    expect(DELIVERY_PRESENTATION.responded.className).toContain("success");
  });

  it("treats needsAction as no information, not as a status", () => {
    // `needsAction` is stamped on every attendee at attach time, so surfacing it
    // as a response is the original bug wearing a label.
    expect(responseDetail(attendee({ response_status: "needsAction" }))).toBeNull();
    expect(responseDetail(attendee({ response_status: null }))).toBeNull();
    expect(responseDetail(attendee({ response_status: "accepted" }))).toBe(
      "accepted",
    );
    expect(responseDetail(attendee({ response_status: "DECLINED" }))).toBe(
      "declined",
    );
    expect(responseDetail(attendee({ response_status: "tentative" }))).toBe(
      "maybe",
    );
  });

  it("counts only the guests we have positive reason to think were never told", () => {
    // `unknown` must NOT be counted: an event created in Google Calendar's web
    // UI has no ledger, and reporting those as "not notified" would cry wolf on
    // most of the calendar and train the badge to be ignored.
    const guests = [
      attendee({ email: "a@x.com", state: "unsent" }),
      attendee({ email: "b@x.com", state: "unknown" }),
      attendee({ email: "c@x.com", state: "sent" }),
      attendee({ email: "d@x.com", state: "responded" }),
      attendee({ email: "e@x.com", state: "unsent" }),
    ];
    expect(countUnsent(guests)).toBe(2);
    expect(countUnsent([])).toBe(0);
  });
});
