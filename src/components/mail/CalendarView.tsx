import { useEffect, useState, useMemo, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-shell";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useUIStore } from "@/stores/uiStore";
import { useWindowStore } from "@/stores/windowStore";
import { useAccountStore } from "@/stores/accountStore";
import EmptyState from "@/components/shared/EmptyState";
import LoadingSpinner from "@/components/shared/LoadingSpinner";
import type { AttendeeDelivery, CalendarEvent } from "@/types/email";
import {
  DELIVERY_PRESENTATION,
  countUnsent,
  responseDetail,
} from "@/lib/attendeeDelivery";
import EventComposeModal from "./EventComposeModal";
import ZoomSettings from "./ZoomSettings";
import { extractMeetingLink, descriptionForDisplay } from "@/lib/meetingLink";
import { isAllDay, parseEventDate } from "@/lib/eventTime";
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  MapPin,
  Clock,
  X,
  Maximize2,
  Minimize2,
  Video,
  Plus,
  RefreshCw,
  Link2,
  Send,
  Trash2,
  Pencil,
  AlertTriangle,
} from "lucide-react";
import { cn } from "@/lib/utils";

// ── Date helpers (native Date, no external deps) ──────────────────────

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function endOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth() + 1, 0);
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function isSameMonth(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();
}

function toDateKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function eventDateKey(dtstart: string): string {
  // dtstart can be "2026-04-05", "2026-04-05T14:00:00Z", etc.
  return dtstart.slice(0, 10);
}

/** Get all days to render in the month grid (includes padding from prev/next months). */
function getMonthGridDays(month: Date): Date[] {
  const first = startOfMonth(month);
  const last = endOfMonth(month);

  // Sunday = 0 in our grid (week starts on Sunday)
  const startDay = first.getDay();

  const days: Date[] = [];

  // Padding from previous month
  for (let i = startDay - 1; i >= 0; i--) {
    days.push(new Date(first.getFullYear(), first.getMonth(), -i));
  }

  // Current month days
  for (let d = 1; d <= last.getDate(); d++) {
    days.push(new Date(month.getFullYear(), month.getMonth(), d));
  }

  // Padding to complete the last week
  const remaining = 7 - (days.length % 7);
  if (remaining < 7) {
    for (let i = 1; i <= remaining; i++) {
      days.push(new Date(last.getFullYear(), last.getMonth() + 1, i));
    }
  }

  return days;
}

function formatMonthYear(d: Date): string {
  return d.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

function formatSelectedDate(d: Date): string {
  return d.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

function formatEventTime(dtstart: string, dtend?: string | null): string {
  // Same predicate the reading-pane card uses — see `@/lib/eventTime`.
  if (isAllDay(dtstart)) return "All day";

  try {
    const start = parseEventDate(dtstart);
    if (!start) return dtstart;
    const timeOpts: Intl.DateTimeFormatOptions = {
      hour: "numeric",
      minute: "2-digit",
    };
    let result = start.toLocaleTimeString(undefined, timeOpts);
    if (dtend) {
      const end = parseEventDate(dtend);
      if (end) result += ` – ${end.toLocaleTimeString(undefined, timeOpts)}`;
    }
    return result;
  } catch {
    return dtstart;
  }
}

// ── Guest delivery list ───────────────────────────────────────────────
// The whole point of this panel: on Google's own data, "invited, hasn't
// replied" and "never told, has no idea" are byte-identical — `responseStatus`
// reads `needsAction` from the moment an attendee is attached, and nothing on
// the event records whether an invitation was ever emailed. Every state below
// is resolved in Rust (`db::invite_notifications::derive`) and rendered as-is;
// re-deriving any of it here would give the app two answers to one question.

function GuestDeliveryList({ attendees }: { attendees: AttendeeDelivery[] }) {
  if (attendees.length === 0) return null;
  const neverTold = attendees.filter((a) => a.state === "unsent").length;

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-content-secondary">
          Guests ({attendees.length})
        </span>
        {neverTold > 0 && (
          <span className="text-[11px] font-medium text-warning">
            {neverTold} not notified
          </span>
        )}
      </div>
      <ul className="space-y-1">
        {attendees.map((attendee) => {
          const { Icon, className, summary, label } = DELIVERY_PRESENTATION[attendee.state];
          return (
            <li key={attendee.email} className="flex items-center gap-2 text-xs" title={label}>
              <Icon className={cn("h-3.5 w-3.5 shrink-0", className)} aria-hidden="true" />
              <span className="truncate font-mono text-[11px] text-content-secondary">
                {attendee.email}
                {attendee.is_self && " (you)"}
              </span>
              <span className={cn("ml-auto shrink-0 text-[11px]", className)}>
                {responseDetail(attendee) ?? summary}
              </span>
              <span className="sr-only">{label}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}


// ── Day-list event row ────────────────────────────────────────────────
// Split into its own component so the meeting-link extraction can be
// memoized per row (raw_ics can be large). The event body and its Join /
// Dismiss actions are sibling buttons so the accessibility tree never nests
// interactive controls.

function EventRow({
  event,
  onOpen,
  onDismiss,
}: {
  event: CalendarEvent;
  onOpen: (event: CalendarEvent) => void;
  onDismiss: (eventId: number) => void;
}) {
  const link = useMemo(() => extractMeetingLink(event), [event]);

  return (
    <div className="flex w-full items-start gap-2 border-b border-border-subtle px-3 py-2 text-left transition-colors hover:bg-surface">
      <div
        className={cn(
          "mt-1 w-0.5 self-stretch rounded-full",
          event.source === "gcal"
            ? "bg-success"
            : event.source === "ics"
              ? "bg-accent"
              : "bg-warning",
        )}
      />
      <button
        onClick={() => onOpen(event)}
        className="min-w-0 flex-1 cursor-pointer text-left"
      >
        <div className="truncate text-sm font-medium text-content">
          {event.summary || "Event"}
        </div>
        <div className="mt-0.5 flex items-center gap-1 text-xs text-content-secondary">
          <Clock className="h-3 w-3 shrink-0" />
          {formatEventTime(event.dtstart, event.dtend)}
        </div>
        {event.location && (
          <div className="mt-0.5 flex items-center gap-1 text-xs text-content-muted">
            <MapPin className="h-3 w-3 shrink-0" />
            <span className="truncate">{event.location}</span>
          </div>
        )}
        {(event.source === "detected" || event.source === "gcal") && (
          <div className="mt-1 flex items-center gap-2">
            {event.source === "detected" && (
              <span className="inline-block rounded px-1 py-0.5 text-[10px] bg-warning/10 text-warning">
                Detected
              </span>
            )}
            {event.source === "gcal" && (
              <span className="inline-block rounded bg-success/10 px-1 py-0.5 text-[10px] text-success">
                Google
              </span>
            )}
          </div>
        )}
      </button>
      {link && (
        <button
          onClick={() => void open(link.url).catch(console.error)}
          className="mt-1 flex shrink-0 items-center gap-1 rounded bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent transition-colors hover:bg-accent/25"
          title={`Join ${link.provider} meeting`}
        >
          <Video className="h-3 w-3" />
          Join
        </button>
      )}
      {event.source === "detected" && (
        <button
          onClick={() => onDismiss(event.id)}
          className="shrink-0 rounded p-1 text-content-muted hover:text-error"
          title="Dismiss"
        >
          <X className="h-3 w-3" />
        </button>
      )}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────

export default function CalendarView({ fullScreen = false }: { fullScreen?: boolean }) {
  const [currentMonth, setCurrentMonth] = useState(() => new Date());
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [selectedDate, setSelectedDate] = useState<Date | null>(
    () => new Date(),
  );
  const [isLoading, setIsLoading] = useState(true);
  const [refreshKey, setRefreshKey] = useState(0);
  const [showCompose, setShowCompose] = useState(false);
  const [showZoomSettings, setShowZoomSettings] = useState(false);
  const [editingEvent, setEditingEvent] = useState<CalendarEvent | null>(null);
  const [detailEvent, setDetailEvent] = useState<CalendarEvent | null>(null);
  // Any provider, not just Meet: a Zoom-backed event has hangout_link = NULL, so
  // gating the Join button on that field left it with no way to join at all.
  const detailMeetingLink = useMemo(
    () => (detailEvent ? extractMeetingLink(detailEvent) : null),
    [detailEvent],
  );
  const [connectedAccounts, setConnectedAccounts] = useState<Record<string, boolean>>({});
  const [isSyncing, setIsSyncing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // Resolved server-side (`db::invite_notifications::derive`) and rendered as
  // given. The old list here showed bare addresses, which could not distinguish
  // "waiting on a reply" from "never told" — the gap this panel now closes.
  // Deleting can put real mail in other people's inboxes, so it goes through a
  // confirmation the way every other outbound path in the app does. Keyed off
  // the panel rather than a modal because the user is already here and the
  // consequences (who gets emailed) belong beside the guest list.
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  useEffect(() => {
    setConfirmingDelete(false);
  }, [detailEvent?.id]);

  const detailDelivery = useMemo(
    () => detailEvent?.attendee_delivery ?? [],
    [detailEvent],
  );
  const detailUnsentCount = useMemo(
    () => countUnsent(detailDelivery),
    [detailDelivery],
  );
  // Who a cancellation actually reaches. Google notifies every attendee on the
  // event, not just the ones we have a ledger row for, so the confirmation lists
  // all of them minus yourself — naming a smaller set than Google will mail
  // would understate the blast radius, which is the one direction this must not
  // be wrong in.
  const detailNotifyRecipients = useMemo(
    () => detailDelivery.filter((a) => !a.is_self).map((a) => a.email),
    [detailDelivery],
  );
  const detailNotifyCount = detailNotifyRecipients.length;
  // `pending_notify` alone still counts: an event created before this ledger
  // existed has no per-attendee rows, but the flag is positive evidence.
  const detailNeedsNotifying =
    detailUnsentCount > 0 || Boolean(detailEvent?.pending_notify);
  const { openCalendarMessage, markMessageRead } = useMailStore();
  const accounts = useAccountStore((state) => state.accounts);
  const gmailAccounts = useMemo(
    () => accounts.filter((account) => account.provider === "gmail"),
    [accounts],
  );
  const toggleCalendarFullScreen = useUIStore((s) => s.toggleCalendarFullScreen);
  const openWindow = useWindowStore((s) => s.openWindow);

  // Compute date range for the displayed month grid
  const gridDays = useMemo(() => getMonthGridDays(currentMonth), [currentMonth]);
  const rangeStart = useMemo(() => toDateKey(gridDays[0]), [gridDays]);
  const rangeEnd = useMemo(() => {
    const last = gridDays[gridDays.length - 1];
    // Add one day to make it exclusive
    const next = new Date(last);
    next.setDate(next.getDate() + 1);
    return toDateKey(next);
  }, [gridDays]);

  // Fetch events for the visible range
  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setIsLoading(true);
      try {
        const result = await api.calendar.listByRange(rangeStart, rangeEnd);
        if (!cancelled) setEvents(result);
      } catch (e) {
        console.error("Failed to load calendar events:", e);
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    load();
    return () => {
      cancelled = true;
    };
  }, [rangeStart, rangeEnd, refreshKey]);

  useEffect(() => {
    let cancelled = false;
    Promise.all(
      gmailAccounts.map(async (account) => [
        account.id,
        await api.calendar.connectionStatus(account.id),
      ] as const),
    )
      .then((results) => {
        if (!cancelled) setConnectedAccounts(Object.fromEntries(results));
      })
      .catch((error) => console.error("Failed to load Calendar connections:", error));
    return () => {
      cancelled = true;
    };
  }, [gmailAccounts]);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    if ("__TAURI_INTERNALS__" in window) {
      void Promise.all([
        listen("calendar-sync-complete", () => setRefreshKey((value) => value + 1)),
        listen<string>("calendar-connected", (event) => {
          const account = gmailAccounts.find((item) => item.email === event.payload);
          if (account) {
            setConnectedAccounts((current) => ({ ...current, [account.id]: true }));
            void api.calendar.syncNow(account.id).finally(() => {
              setRefreshKey((value) => value + 1);
            });
          }
        }),
      ]).then((items) => unlisteners.push(...items));
    }
    const mcpHandler = (event: Event) => {
      const detail = (event as CustomEvent<{ kind?: string }>).detail;
      if (detail?.kind === "calendar") setRefreshKey((value) => value + 1);
    };
    window.addEventListener("cxmail:mcp-activity", mcpHandler);
    return () => {
      unlisteners.forEach((unlisten) => unlisten());
      window.removeEventListener("cxmail:mcp-activity", mcpHandler);
    };
  }, [gmailAccounts]);

  // Group events by date key
  const eventsByDate = useMemo(() => {
    const map = new Map<string, CalendarEvent[]>();
    for (const event of events) {
      const key = eventDateKey(event.dtstart);
      const arr = map.get(key) ?? [];
      arr.push(event);
      map.set(key, arr);
    }
    return map;
  }, [events]);

  const eventsForDate = useCallback(
    (d: Date) => eventsByDate.get(toDateKey(d)) ?? [],
    [eventsByDate],
  );

  const selectedDateEvents = useMemo(
    () => (selectedDate ? eventsForDate(selectedDate) : []),
    [selectedDate, eventsForDate],
  );

  const handlePrevMonth = () => {
    setCurrentMonth(
      (m) => new Date(m.getFullYear(), m.getMonth() - 1, 1),
    );
  };

  const handleNextMonth = () => {
    setCurrentMonth(
      (m) => new Date(m.getFullYear(), m.getMonth() + 1, 1),
    );
  };

  const handleToday = () => {
    const today = new Date();
    setCurrentMonth(new Date(today.getFullYear(), today.getMonth(), 1));
    setSelectedDate(today);
  };

  const handleEventClick = (event: CalendarEvent) => {
    if (event.source === "gcal") {
      setDetailEvent(event);
      return;
    }
    if (fullScreen) {
      // Full-screen mode: open the event as a floating popup instead of a
      // side reading pane (there is no reading pane in full-screen layout).
      openWindow({
        type: "email",
        title: event.summary || "Event",
        props: {
          accountId: event.account_id,
          folder: event.folder_name,
          uid: event.message_uid,
        },
      });
    } else {
      // Split mode: open the source email in the reading pane WITHOUT
      // clearing specialView, so the calendar stays in the center pane.
      openCalendarMessage(event.message_uid, event.account_id, event.folder_name);
      // ReadingPane's auto-mark-read no-ops here (messages[] is empty in
      // calendar), so mark the invite read the same way a normal open would.
      markMessageRead(event.message_uid, event.account_id);
      void api.messages
        .markRead(event.account_id, event.folder_name, [event.message_uid])
        .catch(console.error);
    }
  };

  const reload = () => setRefreshKey((value) => value + 1);

  const syncConnected = async () => {
    setIsSyncing(true);
    setActionError(null);
    try {
      await Promise.all(
        gmailAccounts
          .filter((account) => connectedAccounts[account.id])
          .map((account) => api.calendar.syncNow(account.id)),
      );
      reload();
    } catch (error) {
      setActionError(String(error));
    } finally {
      setIsSyncing(false);
    }
  };

  const sendInvites = async (event: CalendarEvent) => {
    setActionError(null);
    try {
      await api.calendar.sendInvites(event.id);
      setDetailEvent(null);
      reload();
    } catch (error) {
      setActionError(String(error));
    }
  };

  const deleteGoogleEvent = async (event: CalendarEvent, notify: boolean) => {
    setActionError(null);
    try {
      await api.calendar.delete(event.id, notify);
      setConfirmingDelete(false);
      setDetailEvent(null);
      reload();
    } catch (error) {
      setActionError(String(error));
    }
  };

  const resolveConflict = async (event: CalendarEvent, resolution: "mine" | "theirs") => {
    setActionError(null);
    try {
      await api.calendar.resolveConflict(event.id, resolution);
      setDetailEvent(null);
      reload();
    } catch (error) {
      setActionError(String(error));
    }
  };

  const handleDismiss = async (eventId: number) => {
    try {
      await api.calendar.dismiss(eventId);
      setEvents((prev) => prev.filter((e) => e.id !== eventId));
    } catch (e) {
      console.error("Failed to dismiss event:", e);
    }
  };

  const today = new Date();

  return (
    <div className="flex h-full flex-col">
      {/* Month navigation header */}
      <div className="flex items-center justify-between border-b border-border-subtle px-3 py-2">
        <div className="flex items-center gap-1">
          <button
            onClick={() => setShowCompose(true)}
            disabled={!Object.values(connectedAccounts).some(Boolean)}
            className="rounded p-1 text-content-secondary hover:bg-surface hover:text-content disabled:opacity-35"
            title="New event"
          >
            <Plus className="h-4 w-4" />
          </button>
          <button
            onClick={() => void syncConnected()}
            disabled={isSyncing || !Object.values(connectedAccounts).some(Boolean)}
            className="rounded p-1 text-content-secondary hover:bg-surface hover:text-content disabled:opacity-35"
            title="Sync Google Calendar"
          >
            <RefreshCw className={cn("h-4 w-4", isSyncing && "animate-spin")} />
          </button>
          <button
            onClick={handlePrevMonth}
            className="rounded p-1 text-content-secondary hover:bg-surface hover:text-content"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <button
            onClick={handleNextMonth}
            className="rounded p-1 text-content-secondary hover:bg-surface hover:text-content"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
        <span className="text-sm font-medium text-content">
          {formatMonthYear(currentMonth)}
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={handleToday}
            className="rounded px-2 py-0.5 text-xs text-content-secondary hover:bg-surface hover:text-content"
          >
            Today
          </button>
          <button
            onClick={() => setShowZoomSettings((open) => !open)}
            className={cn(
              "rounded p-1 hover:bg-surface hover:text-content",
              showZoomSettings ? "text-accent" : "text-content-secondary",
            )}
            title="Zoom meeting settings"
          >
            <Video className="h-4 w-4" />
          </button>
          <button
            onClick={toggleCalendarFullScreen}
            className="rounded p-1 text-content-secondary hover:bg-surface hover:text-content"
            title={fullScreen ? "Exit full screen" : "Full screen"}
          >
            {fullScreen ? (
              <Minimize2 className="h-4 w-4" />
            ) : (
              <Maximize2 className="h-4 w-4" />
            )}
          </button>
        </div>
      </div>

      {showZoomSettings && <ZoomSettings onClose={() => setShowZoomSettings(false)} />}

      {gmailAccounts.some((account) => !connectedAccounts[account.id]) && (
        <div className="flex flex-wrap items-center gap-2 border-b border-border-subtle bg-accent/5 px-3 py-2">
          <Link2 className="h-3.5 w-3.5 text-accent" />
          <span className="text-xs text-content-secondary">Connect Google Calendar:</span>
          {gmailAccounts
            .filter((account) => !connectedAccounts[account.id])
            .map((account) => (
              <button
                key={account.id}
                onClick={() => void api.calendar.connect(account.email)}
                className="rounded bg-accent/15 px-2 py-1 text-xs font-medium text-accent hover:bg-accent/25"
              >
                {account.display_name || account.email}
              </button>
            ))}
        </div>
      )}

      {actionError && (
        <div role="alert" className="border-b border-error/30 bg-error/10 px-3 py-2 text-xs text-error">
          {actionError}
        </div>
      )}

      {/* Day-of-week headers */}
      <div className="grid grid-cols-7 border-b border-border-subtle">
        {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((d) => (
          <div
            key={d}
            className="py-1.5 text-center text-[10px] font-medium uppercase tracking-wider text-content-muted"
          >
            {d}
          </div>
        ))}
      </div>

      {/* Calendar grid */}
      {isLoading ? (
        <LoadingSpinner className="flex-1" />
      ) : (
        <>
          <div className="grid grid-cols-7">
            {gridDays.map((day) => {
              const dayEvents = eventsForDate(day);
              const isCurrentMonth = isSameMonth(day, currentMonth);
              const isToday = isSameDay(day, today);
              const isSelected = selectedDate
                ? isSameDay(day, selectedDate)
                : false;
              const hasIcs = dayEvents.some((e) => e.source === "ics");
              const hasDetected = dayEvents.some(
                (e) => e.source === "detected",
              );
              const hasGoogle = dayEvents.some((e) => e.source === "gcal");

              return (
                <button
                  key={day.toISOString()}
                  onClick={() => setSelectedDate(day)}
                  onDoubleClick={() => {
                    setSelectedDate(day);
                    setShowCompose(true);
                  }}
                  className={cn(
                    "flex flex-col items-center gap-0.5 border-b border-r border-border-subtle py-1.5 text-xs transition-colors",
                    isSelected && "bg-accent/15",
                    !isSelected && "hover:bg-surface",
                  )}
                >
                  <span
                    className={cn(
                      "flex h-6 w-6 items-center justify-center rounded-full",
                      isToday && "bg-accent font-bold text-white",
                      !isToday && isCurrentMonth && "text-content",
                      !isToday && !isCurrentMonth && "text-content-muted/40",
                    )}
                  >
                    {day.getDate()}
                  </span>
                  <div className="flex gap-0.5">
                    {hasIcs && (
                      <div className="h-1 w-1 rounded-full bg-accent" />
                    )}
                    {hasDetected && (
                      <div className="h-1 w-1 rounded-full bg-warning" />
                    )}
                    {hasGoogle && (
                      <div className="h-1 w-1 rounded-full bg-success" />
                    )}
                  </div>
                </button>
              );
            })}
          </div>

          {/* Events for selected day */}
          <div className="flex-1 overflow-auto border-t border-border-subtle">
            {selectedDate && (
              <div className="px-3 py-2">
                <h3 className="text-xs font-medium uppercase tracking-wider text-content-muted">
                  {formatSelectedDate(selectedDate)}
                </h3>
              </div>
            )}

            {selectedDate && selectedDateEvents.length === 0 && (
              <div className="px-3 py-4 text-center text-xs text-content-muted">
                No events
              </div>
            )}

            {selectedDateEvents.map((event) => (
              <EventRow
                key={event.id}
                event={event}
                onOpen={handleEventClick}
                onDismiss={handleDismiss}
              />
            ))}

            {!selectedDate && events.length === 0 && (
              <EmptyState
                icon={CalendarDays}
                title="No events"
                description="Calendar invites and detected events will appear here"
              />
            )}
          </div>
        </>
      )}

      {(showCompose || editingEvent) && (
        <EventComposeModal
          initialDate={selectedDate || new Date()}
          event={editingEvent || undefined}
          onClose={() => {
            setShowCompose(false);
            setEditingEvent(null);
          }}
          onSaved={reload}
        />
      )}

      {detailEvent && (
        <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/60 p-4">
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="calendar-detail-title"
            className="w-full max-w-md rounded-xl border border-border bg-elevated shadow-2xl"
          >
            <div className="flex items-start justify-between border-b border-border-subtle p-4">
              <div>
                <div className="mb-1 text-[10px] font-medium uppercase tracking-wider text-success">
                  Google Calendar
                </div>
                <h2 id="calendar-detail-title" className="font-medium text-content">
                  {detailEvent.summary || "Event"}
                </h2>
                <p className="mt-1 text-xs text-content-secondary">
                  {formatEventTime(detailEvent.dtstart, detailEvent.dtend)}
                </p>
              </div>
              <button
                onClick={() => setDetailEvent(null)}
                className="rounded p-1 text-content-muted hover:bg-surface"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-3 p-4 text-sm text-content-secondary">
              {detailEvent.location && (
                <div className="flex gap-2">
                  <MapPin className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>{detailEvent.location}</span>
                </div>
              )}
              {descriptionForDisplay(detailEvent.description) && (
                <p className="whitespace-pre-wrap">
                  {descriptionForDisplay(detailEvent.description)}
                </p>
              )}
              {detailEvent.sync_state === "conflict" && (
                <div className="rounded-md border border-warning/30 bg-warning/10 p-3">
                  <div className="mb-2 flex items-center gap-2 text-warning">
                    <AlertTriangle className="h-4 w-4" />
                    This event changed both here and in Google Calendar.
                  </div>
                  <div className="flex gap-2">
                    <button
                      onClick={() => void resolveConflict(detailEvent, "mine")}
                      className="rounded bg-warning/15 px-2 py-1 text-xs text-warning"
                    >
                      Keep mine
                    </button>
                    <button
                      onClick={() => void resolveConflict(detailEvent, "theirs")}
                      className="rounded bg-surface px-2 py-1 text-xs text-content"
                    >
                      Use theirs
                    </button>
                  </div>
                </div>
              )}
              {detailEvent.recurring_event_id && (
                <div className="rounded-md border border-border bg-surface p-3 text-xs text-content-secondary">
                  Recurring instances are read-only in CXMail. Open this event in Google Calendar
                  to edit the series or one occurrence.
                </div>
              )}
              {detailMeetingLink && (
                <button
                  onClick={() => void open(detailMeetingLink.url)}
                  className="flex items-center gap-2 rounded-md bg-accent/15 px-3 py-2 font-medium text-accent"
                  title={`Join ${detailMeetingLink.provider} meeting`}
                >
                  <Video className="h-4 w-4" />
                  {detailMeetingLink.provider === "zoom"
                    ? "Join Zoom Meeting"
                    : detailMeetingLink.provider === "meet"
                      ? "Join Google Meet"
                      : "Join meeting"}
                </button>
              )}
              <GuestDeliveryList attendees={detailDelivery} />
              {detailNeedsNotifying && !detailEvent.recurring_event_id && (
                <div className="rounded-md border border-warning/30 bg-warning/10 p-3 text-xs">
                  {detailUnsentCount > 0
                    ? `${detailUnsentCount} guest${detailUnsentCount === 1 ? " has" : "s have"} not been notified.`
                    : "Attendees have not been notified yet."}
                  <button
                    onClick={() => void sendInvites(detailEvent)}
                    className="mt-2 flex items-center gap-1 rounded bg-warning px-2 py-1 font-medium text-black"
                  >
                    <Send className="h-3.5 w-3.5" />
                    Send invites
                  </button>
                </div>
              )}
            </div>
            {confirmingDelete && (
              <div
                role="alertdialog"
                aria-label="Confirm deleting this event"
                className="border-t border-warning/30 bg-warning/10 px-4 py-3 text-xs"
              >
                {detailEvent.cancellation_notifies ? (
                  <>
                    <div className="font-medium text-content-primary">
                      Cancel this meeting and email {detailNotifyCount}{" "}
                      {detailNotifyCount === 1 ? "guest" : "guests"}?
                    </div>
                    <ul className="mt-1.5 space-y-0.5">
                      {detailNotifyRecipients.map((email) => (
                        <li key={email} className="font-mono text-[11px] text-content-secondary">
                          {email}
                        </li>
                      ))}
                    </ul>
                    <p className="mt-2 text-[11px] text-content-secondary">
                      Google sends the cancellation notice. Guests we have no record of
                      telling are included, because we cannot rule out that they know.
                    </p>
                  </>
                ) : (
                  <div className="font-medium text-content-primary">
                    Delete this event? No one will be emailed
                    {detailDelivery.length > 0
                      ? " — no guest here was ever sent an invitation."
                      : "."}
                  </div>
                )}
                <div className="mt-2.5 flex flex-wrap gap-2">
                  <button
                    onClick={() => setConfirmingDelete(false)}
                    className="rounded px-2 py-1 text-xs text-content-secondary hover:bg-surface"
                  >
                    Keep event
                  </button>
                  {detailEvent.cancellation_notifies ? (
                    <>
                      <button
                        onClick={() => void deleteGoogleEvent(detailEvent, true)}
                        className="flex items-center gap-1 rounded bg-error px-2 py-1 text-xs font-medium text-white"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                        Delete and notify
                      </button>
                      <button
                        onClick={() => void deleteGoogleEvent(detailEvent, false)}
                        className="rounded px-2 py-1 text-xs text-error hover:bg-error/10"
                      >
                        Delete without notifying
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={() => void deleteGoogleEvent(detailEvent, false)}
                      className="flex items-center gap-1 rounded bg-error px-2 py-1 text-xs font-medium text-white"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                      Delete event
                    </button>
                  )}
                </div>
              </div>
            )}
            <div className="flex justify-between border-t border-border-subtle px-4 py-3">
              {detailEvent.recurring_event_id ? <span /> : (
                <button
                  onClick={() => setConfirmingDelete(true)}
                  disabled={confirmingDelete}
                  className="flex items-center gap-1 rounded px-2 py-1.5 text-xs text-error hover:bg-error/10 disabled:opacity-40"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  Delete
                </button>
              )}
              <div className="flex gap-2">
                {detailEvent.html_link && (
                  <button
                    onClick={() => void open(detailEvent.html_link!)}
                    className="rounded px-2 py-1.5 text-xs text-content-secondary hover:bg-surface"
                  >
                    Open in Google
                  </button>
                )}
                {!detailEvent.recurring_event_id && (
                  <button
                    onClick={() => {
                      setEditingEvent(detailEvent);
                      setDetailEvent(null);
                    }}
                    className="flex items-center gap-1 rounded bg-accent px-2 py-1.5 text-xs font-medium text-white"
                  >
                    <Pencil className="h-3.5 w-3.5" />
                    Edit
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
