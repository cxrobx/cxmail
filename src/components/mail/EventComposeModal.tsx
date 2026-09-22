import { useMemo, useState } from "react";
import { CalendarDays, Loader2, MapPin, Video, X } from "lucide-react";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import type { CalendarEvent, CalendarEventInput } from "@/types/email";
import { toLocalDateTimeInput as localInputValue } from "@/lib/dateInput";

function attendeesFromEvent(event?: CalendarEvent): string {
  if (!event?.attendees_json) return "";
  try {
    const attendees = JSON.parse(event.attendees_json) as Array<{ email?: string }>;
    return attendees.map((attendee) => attendee.email).filter(Boolean).join(", ");
  } catch {
    return "";
  }
}

interface EventComposeModalProps {
  initialDate?: Date;
  event?: CalendarEvent;
  onClose: () => void;
  onSaved: () => void;
}

export default function EventComposeModal({
  initialDate = new Date(),
  event,
  onClose,
  onSaved,
}: EventComposeModalProps) {
  const allAccounts = useAccountStore((state) => state.accounts);
  const accounts = useMemo(
    () => allAccounts.filter((account) => account.provider === "gmail"),
    [allAccounts],
  );
  const defaultStart = useMemo(() => {
    if (event) return localInputValue(new Date(event.dtstart));
    const date = new Date(initialDate);
    date.setHours(Math.max(new Date().getHours() + 1, 9), 0, 0, 0);
    return localInputValue(date);
  }, [event, initialDate]);
  const defaultEnd = useMemo(() => {
    if (event?.dtend) return localInputValue(new Date(event.dtend));
    const date = new Date(defaultStart);
    date.setHours(date.getHours() + 1);
    return localInputValue(date);
  }, [event, defaultStart]);

  const [accountId, setAccountId] = useState(event?.account_id || accounts[0]?.id || "");
  const [summary, setSummary] = useState(event?.summary || "");
  const [start, setStart] = useState(defaultStart);
  const [end, setEnd] = useState(defaultEnd);
  const [location, setLocation] = useState(event?.location || "");
  const [description, setDescription] = useState(event?.description || "");
  const [attendees, setAttendees] = useState(attendeesFromEvent(event));
  const [addMeet, setAddMeet] = useState(!event);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timeZone =
    event?.start_tz || Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

  const save = async () => {
    setError(null);
    if (!summary.trim()) {
      setError("Add an event title.");
      return;
    }
    if (!accountId) {
      setError("Connect a Gmail account first.");
      return;
    }
    const attendeeList = attendees
      .split(/[,\n;]/)
      .map((value) => value.trim())
      .filter(Boolean);
    setSaving(true);
    try {
      if (event) {
        await api.calendar.update(event.id, {
          summary: summary.trim(),
          description: description.trim() || null,
          location: location.trim() || null,
          start,
          end,
          time_zone: timeZone,
          attendees: attendeeList,
        });
      } else {
        const input: CalendarEventInput = {
          account_id: accountId,
          summary: summary.trim(),
          description: description.trim() || null,
          location: location.trim() || null,
          start,
          end,
          time_zone: timeZone,
          attendees: attendeeList,
          add_meet: addMeet,
        };
        await api.calendar.create(input);
      }
      onSaved();
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="event-compose-title"
        className="w-full max-w-lg rounded-xl border border-border bg-elevated shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <div className="flex items-center gap-2">
            <CalendarDays className="h-4 w-4 text-accent" />
            <h2 id="event-compose-title" className="font-medium text-content">
              {event ? "Edit event" : "New event"}
            </h2>
          </div>
          <button onClick={onClose} className="rounded p-1 text-content-muted hover:bg-surface">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-3 p-4">
          {!event && (
            <label className="block text-xs text-content-secondary">
              Calendar account
              <select
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                className="mt-1 w-full rounded-md border border-border bg-surface px-3 py-2 text-sm text-content"
              >
                {accounts.map((account) => (
                  <option key={account.id} value={account.id}>
                    {account.display_name || account.email}
                  </option>
                ))}
              </select>
            </label>
          )}
          <input
            autoFocus
            value={summary}
            onChange={(e) => setSummary(e.target.value)}
            placeholder="Event title"
            className="w-full rounded-md border border-border bg-surface px-3 py-2 text-sm text-content outline-none focus:border-accent"
          />
          <div className="grid grid-cols-2 gap-3">
            <label className="text-xs text-content-secondary">
              Starts
              <input
                type="datetime-local"
                value={start}
                onChange={(e) => setStart(e.target.value)}
                className="mt-1 w-full rounded-md border border-border bg-surface px-2 py-2 text-sm text-content"
              />
            </label>
            <label className="text-xs text-content-secondary">
              Ends
              <input
                type="datetime-local"
                value={end}
                onChange={(e) => setEnd(e.target.value)}
                className="mt-1 w-full rounded-md border border-border bg-surface px-2 py-2 text-sm text-content"
              />
            </label>
          </div>
          <div className="text-[11px] text-content-muted">{timeZone}</div>
          <label className="flex items-center gap-2 rounded-md border border-border bg-surface px-3 py-2">
            <MapPin className="h-4 w-4 text-content-muted" />
            <input
              value={location}
              onChange={(e) => setLocation(e.target.value)}
              placeholder="Location"
              className="min-w-0 flex-1 bg-transparent text-sm text-content outline-none"
            />
          </label>
          <input
            value={attendees}
            onChange={(e) => setAttendees(e.target.value)}
            placeholder="Attendees (comma-separated emails)"
            className="w-full rounded-md border border-border bg-surface px-3 py-2 text-sm text-content outline-none focus:border-accent"
          />
          {!event && (
            <label className="flex items-center gap-2 text-sm text-content-secondary">
              <input
                type="checkbox"
                checked={addMeet}
                onChange={(e) => setAddMeet(e.target.checked)}
                className="accent-accent"
              />
              <Video className="h-4 w-4" />
              Add Google Meet
            </label>
          )}
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Description"
            rows={4}
            className="w-full resize-none rounded-md border border-border bg-surface px-3 py-2 text-sm text-content outline-none focus:border-accent"
          />
          {error && (
            <div role="alert" className="rounded-md bg-error/10 px-3 py-2 text-xs text-error">
              {error}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t border-border-subtle px-4 py-3">
          <button
            onClick={onClose}
            className="rounded-md px-3 py-1.5 text-sm text-content-secondary hover:bg-surface"
          >
            Cancel
          </button>
          <button
            onClick={() => void save()}
            disabled={saving}
            className="flex items-center gap-2 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
          >
            {saving && <Loader2 className="h-4 w-4 animate-spin" />}
            {event ? "Save changes" : "Create event"}
          </button>
        </div>
      </div>
    </div>
  );
}
