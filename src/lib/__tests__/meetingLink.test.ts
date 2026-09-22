import { describe, it, expect } from "vitest";
import { extractMeetingLink } from "@/lib/meetingLink";
import type { CalendarEvent } from "@/types/email";

// Minimal CalendarEvent factory — only the fields extractMeetingLink reads
// (location / description / raw_ics) matter; the rest are filler.
function makeEvent(over: Partial<CalendarEvent>): CalendarEvent {
  return {
    id: 1,
    account_id: "acct",
    folder_name: "INBOX",
    message_uid: 100,
    event_uid: null,
    summary: "Meeting",
    description: null,
    location: null,
    dtstart: "2026-05-28T15:00:00Z",
    dtend: null,
    organizer_name: null,
    organizer_email: null,
    status: null,
    method: "REQUEST",
    rsvp_status: "needs-action",
    raw_ics: null,
    source: "ics",
    confidence: null,
    dismissed: false,
    created_at: "2026-05-28T00:00:00Z",
    ...over,
  };
}

describe("extractMeetingLink", () => {
  it("detects a Zoom /j/ link in location", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "https://us02web.zoom.us/j/85123456789?pwd=abc" }),
    );
    expect(link).toEqual({
      url: "https://us02web.zoom.us/j/85123456789?pwd=abc",
      provider: "zoom",
    });
  });

  it("detects a Zoom /my/ personal-room link", () => {
    const link = extractMeetingLink(
      makeEvent({ description: "Join: https://zoom.us/my/chris" }),
    );
    expect(link?.provider).toBe("zoom");
    expect(link?.url).toBe("https://zoom.us/my/chris");
  });

  it("detects a Google Meet link", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "https://meet.google.com/abc-defg-hij" }),
    );
    expect(link).toEqual({
      url: "https://meet.google.com/abc-defg-hij",
      provider: "meet",
    });
  });

  it("detects a Teams meetup-join link in description", () => {
    const link = extractMeetingLink(
      makeEvent({
        description:
          "Click here to join the meeting https://teams.microsoft.com/l/meetup-join/19%3ameeting_X/0?context=Y",
      }),
    );
    expect(link?.provider).toBe("teams");
  });

  it("detects a Webex link", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "https://acme.webex.com/meet/jdoe" }),
    );
    expect(link?.provider).toBe("webex");
  });

  it("detects a Jitsi link", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "https://meet.jit.si/StandupRoom" }),
    );
    expect(link?.provider).toBe("jitsi");
  });

  it("falls back to generic for a bare URL alone in location", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "  https://example.com/room/42  " }),
    );
    expect(link).toEqual({
      url: "https://example.com/room/42",
      provider: "generic",
    });
  });

  it("does not treat a free-text location as a generic link", () => {
    const link = extractMeetingLink(
      makeEvent({ location: "Conference Room B, 3rd floor" }),
    );
    expect(link).toBeNull();
  });

  it("returns null when no link is present anywhere", () => {
    const link = extractMeetingLink(
      makeEvent({
        location: "TBD",
        description: "We'll figure out the details later.",
        raw_ics: "BEGIN:VEVENT\nSUMMARY:Sync\nEND:VEVENT",
      }),
    );
    expect(link).toBeNull();
  });

  it("scans raw_ics when location and description have no link", () => {
    const link = extractMeetingLink(
      makeEvent({
        location: "Online",
        description: "See invite",
        raw_ics:
          "BEGIN:VEVENT\nURL:https://us02web.zoom.us/j/999888777\nEND:VEVENT",
      }),
    );
    expect(link?.provider).toBe("zoom");
  });

  it("prefers location over description (priority order)", () => {
    const link = extractMeetingLink(
      makeEvent({
        location: "https://meet.google.com/aaa-bbbb-ccc",
        description: "backup https://zoom.us/j/123",
      }),
    );
    expect(link?.provider).toBe("meet");
  });
});
