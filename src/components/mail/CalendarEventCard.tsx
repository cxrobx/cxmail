import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import { api } from "@/lib/tauri";
import type { CalendarEvent } from "@/types/email";
import { extractMeetingLink } from "@/lib/meetingLink";
import { isAllDay, parseEventDate, formatEventDay } from "@/lib/eventTime";
import { Calendar, MapPin, Clock, User, Check, HelpCircle, XCircle, Video } from "lucide-react";
import { cn } from "@/lib/utils";

interface CalendarEventCardProps {
  accountId: string;
  folder: string;
  uid: number;
}

export default function CalendarEventCard({ accountId, folder, uid }: CalendarEventCardProps) {
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [rsvpLoading, setRsvpLoading] = useState<number | null>(null);

  useEffect(() => {
    api.calendar.getEvents(accountId, folder, uid).then(setEvents).catch((e) => console.error("Failed to load calendar events:", e));
  }, [accountId, folder, uid]);

  if (events.length === 0) return null;

  const handleRsvp = async (eventId: number, response: string) => {
    setRsvpLoading(eventId);
    try {
      await api.calendar.rsvp(eventId, accountId, response);
      setEvents((prev) =>
        prev.map((e) => (e.id === eventId ? { ...e, rsvp_status: response } : e))
      );
    } catch (e) {
      console.error("RSVP failed:", e);
    } finally {
      setRsvpLoading(null);
    }
  };

  const formatDateTime = (dt: string, dtend?: string | null) => {
    try {
      // All-day is the third arm of the dtstart contract — see `@/lib/eventTime`
      // for why it must not go through `new Date(dt)`.
      if (isAllDay(dt)) {
        const day = formatEventDay(dt);
        return day ? `${day} · All day` : dt;
      }
      const start = parseEventDate(dt);
      if (!start) return dt;
      const opts: Intl.DateTimeFormatOptions = {
        weekday: "short",
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      };
      let result = start.toLocaleString(undefined, opts);
      if (dtend) {
        const end = parseEventDate(dtend);
        // An unparseable end appends nothing rather than "Invalid Date".
        if (end) {
          result +=
            start.toDateString() === end.toDateString()
              ? ` - ${end.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`
              : ` - ${end.toLocaleString(undefined, opts)}`;
        }
      }
      return result;
    } catch {
      return dt;
    }
  };

  return (
    <div className="border-b border-border-subtle">
      {events.map((event) => {
        const link = extractMeetingLink(event);
        return (
        <div key={event.id} className="mx-4 my-3 rounded-lg border border-border bg-sidebar p-3">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 rounded-md bg-accent/10 p-2">
              <Calendar className="h-5 w-5 text-accent" />
            </div>
            <div className="flex-1 min-w-0">
              <h4 className="text-sm font-medium text-content">
                {event.summary || "Event"}
              </h4>
              <div className="mt-1.5 space-y-1">
                <div className="flex items-center gap-1.5 text-xs text-content-secondary">
                  <Clock className="h-3.5 w-3.5 shrink-0" />
                  {formatDateTime(event.dtstart, event.dtend)}
                </div>
                {event.location && (
                  <div className="flex items-center gap-1.5 text-xs text-content-secondary">
                    <MapPin className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">{event.location}</span>
                  </div>
                )}
                {event.organizer_email && (
                  <div className="flex items-center gap-1.5 text-xs text-content-secondary">
                    <User className="h-3.5 w-3.5 shrink-0" />
                    {event.organizer_name || event.organizer_email}
                  </div>
                )}
              </div>

              {link && (
                <button
                  onClick={() => void open(link.url).catch(console.error)}
                  className="mt-3 flex items-center gap-1.5 rounded-md bg-accent/15 px-3 py-1.5 text-xs font-medium text-accent transition-colors hover:bg-accent/25"
                  title={`Join ${link.provider} meeting`}
                >
                  <Video className="h-3.5 w-3.5" />
                  Join meeting
                </button>
              )}

              {/* RSVP buttons */}
              {event.method === "REQUEST" && (
                <div className="mt-3 flex items-center gap-2">
                  <button
                    onClick={() => handleRsvp(event.id, "accepted")}
                    disabled={rsvpLoading === event.id}
                    className={cn(
                      "flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                      event.rsvp_status === "accepted"
                        ? "bg-success/20 text-success"
                        : "bg-elevated text-content-secondary hover:bg-success/20 hover:text-success"
                    )}
                  >
                    <Check className="h-3.5 w-3.5" />
                    Accept
                  </button>
                  <button
                    onClick={() => handleRsvp(event.id, "tentative")}
                    disabled={rsvpLoading === event.id}
                    className={cn(
                      "flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                      event.rsvp_status === "tentative"
                        ? "bg-warning/20 text-warning"
                        : "bg-elevated text-content-secondary hover:bg-warning/20 hover:text-warning"
                    )}
                  >
                    <HelpCircle className="h-3.5 w-3.5" />
                    Maybe
                  </button>
                  <button
                    onClick={() => handleRsvp(event.id, "declined")}
                    disabled={rsvpLoading === event.id}
                    className={cn(
                      "flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                      event.rsvp_status === "declined"
                        ? "bg-error/20 text-error"
                        : "bg-elevated text-content-secondary hover:bg-error/20 hover:text-error"
                    )}
                  >
                    <XCircle className="h-3.5 w-3.5" />
                    Decline
                  </button>
                </div>
              )}
              {event.rsvp_status !== "needs-action" && event.method !== "REQUEST" && (
                <div className="mt-2 text-xs text-content-muted">
                  RSVP: {event.rsvp_status}
                </div>
              )}
            </div>
          </div>
        </div>
        );
      })}
    </div>
  );
}
