import { describe, it, expect } from "vitest";
import { extractMeetingLink, descriptionForDisplay } from "@/lib/meetingLink";
import type { CalendarEvent } from "@/types/email";

/**
 * The interop contract between `email::zoom_sync::zoom_block` (Rust) and
 * `extractMeetingLink` (TypeScript).
 *
 * `meetingLink.test.ts` is deliberately NOT edited — its staying green with no
 * changes is the proof the Zoom feature needed no calendar work. This file adds
 * the one thing that file cannot state: that the *exact* description block the
 * backend writes is what lights the "Join meeting" button.
 *
 * The literal below is a copy of `zoom_block`'s output. If it drifts, this test
 * fails and the button silently stops appearing on Zoom events — which is
 * otherwise indistinguishable from "the meeting was never created".
 */
const ZOOM_BLOCK = [
  "[cxmail:zoom]",
  "Join Zoom Meeting",
  "https://us02web.zoom.us/j/86903742305",
  "",
  "Meeting ID: 869 0374 2305",
  "[/cxmail:zoom]",
].join("\n");

function makeEvent(over: Partial<CalendarEvent>): CalendarEvent {
  return {
    id: 1,
    account_id: "acct",
    folder_name: "",
    message_uid: 0,
    event_uid: null,
    summary: "Design review",
    description: null,
    location: null,
    dtstart: "2026-08-12T15:00:00Z",
    dtend: "2026-08-12T15:45:00Z",
    organizer_name: null,
    organizer_email: null,
    status: "confirmed",
    method: null,
    rsvp_status: "needs-action",
    raw_ics: null,
    source: "gcal",
    confidence: null,
    dismissed: false,
    created_at: "2026-08-10T00:00:00Z",
    ...over,
  };
}

describe("the Zoom description block CXMail writes", () => {
  it("resolves to a zoom join link with no frontend changes", () => {
    expect(extractMeetingLink(makeEvent({ description: ZOOM_BLOCK }))).toEqual({
      url: "https://us02web.zoom.us/j/86903742305",
      provider: "zoom",
    });
  });

  it("is still found when the user's own prose sits above it", () => {
    const description = `Agenda:\n- roadmap\n- hiring\n\n${ZOOM_BLOCK}`;
    expect(extractMeetingLink(makeEvent({ description }))?.provider).toBe("zoom");
  });

  it("leaves location empty, so no raw-URL MapPin line is rendered (gotcha #21)", () => {
    const event = makeEvent({ description: ZOOM_BLOCK });
    expect(event.location).toBeNull();
    // The scan order is location → description → raw_ics; description alone is
    // enough, and is the field that is NOT displayed.
    expect(extractMeetingLink(event)?.provider).toBe("zoom");
  });

  it("is never SHOWN to a person — the fence markers are machine plumbing", () => {
    // Caught live: the calendar detail modal renders `description` verbatim, so
    // the raw `[cxmail:zoom]` markers leaked into the UI as junk. gotcha #21 says
    // description is "scanned, not displayed" — true of the event CARD, not of
    // this modal.
    const description = `Throwaway event verifying CXMail's Zoom integration.\n\n${ZOOM_BLOCK}`;
    const shown = descriptionForDisplay(description);
    expect(shown).toBe("Throwaway event verifying CXMail's Zoom integration.");
    expect(shown).not.toContain("[cxmail:zoom]");
    expect(shown).not.toContain("[/cxmail:zoom]");
    expect(shown).not.toContain("Meeting ID");
  });

  it("returns null when the block was the only content, so no empty paragraph renders", () => {
    expect(descriptionForDisplay(ZOOM_BLOCK)).toBeNull();
    expect(descriptionForDisplay(null)).toBeNull();
    expect(descriptionForDisplay(undefined)).toBeNull();
    expect(descriptionForDisplay("   ")).toBeNull();
  });

  it("leaves a description that merely mentions a zoom URL completely alone", () => {
    const prose = "Dana will send a https://zoom.us/j/12345 link before we start.";
    expect(descriptionForDisplay(prose)).toBe(prose);
  });

  it("keeps prose on BOTH sides of the block", () => {
    const description = `Before.\n\n${ZOOM_BLOCK}\n\nAfter.`;
    expect(descriptionForDisplay(description)).toBe("Before.\n\nAfter.");
  });

  it("stripping is display-only and must not be what gets stored", () => {
    // If this value were ever written back to the event, the join link would be
    // gone and `extractMeetingLink` would find nothing.
    const description = `Agenda\n\n${ZOOM_BLOCK}`;
    const shown = descriptionForDisplay(description)!;
    expect(extractMeetingLink(makeEvent({ description }))?.provider).toBe("zoom");
    expect(extractMeetingLink(makeEvent({ description: shown }))).toBeNull();
  });

  it("does not misreport Zoom when Google also provisioned a Meet link", () => {
    // hangout_link short-circuits ahead of every scan, so a Meet-backed event is
    // never mistaken for a Zoom one — which is why the two are mutually
    // exclusive at the create boundary rather than merely discouraged.
    const event = makeEvent({ description: ZOOM_BLOCK });
    (event as CalendarEvent & { hangout_link?: string }).hangout_link =
      "https://meet.google.com/abc-defg-hij";
    expect(extractMeetingLink(event)).toEqual({
      url: "https://meet.google.com/abc-defg-hij",
      provider: "meet",
    });
  });
});
