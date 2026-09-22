import type { MessageSummary } from "@/types/email";

export type ListItem =
  | { type: "header"; label: string; count: number; key: string }
  | { type: "message"; message: MessageSummary; key: string };

type DateBucket = "Pinned" | "Today" | "Yesterday" | "This Week" | "Last Week" | "This Month" | "Older";

function getDateBucket(ts: number, todayStart: number, yesterdayStart: number, thisWeekStart: number, lastWeekStart: number, thisMonthStart: number): DateBucket {
  if (ts >= todayStart) return "Today";
  if (ts >= yesterdayStart) return "Yesterday";
  if (ts >= thisWeekStart) return "This Week";
  if (ts >= lastWeekStart) return "Last Week";
  if (ts >= thisMonthStart) return "This Month";
  return "Older";
}

const BUCKET_ORDER: DateBucket[] = ["Today", "Yesterday", "This Week", "Last Week", "This Month", "Older"];

export function groupMessagesByDate(messages: MessageSummary[]): ListItem[] {
  if (messages.length === 0) return [];

  const now = new Date();
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const yesterdayStart = todayStart - 86400000;
  const dayOfWeek = now.getDay(); // 0=Sun, 1=Mon, ...
  const thisWeekStart = todayStart - dayOfWeek * 86400000;
  const lastWeekStart = thisWeekStart - 7 * 86400000;
  const thisMonthStart = new Date(now.getFullYear(), now.getMonth(), 1).getTime();

  const items: ListItem[] = [];

  // Pinned messages come first
  const pinnedMessages = messages.filter((m) => m.is_pinned);
  const regularMessages = messages.filter((m) => !m.is_pinned);

  if (pinnedMessages.length > 0) {
    items.push({ type: "header", label: "Pinned", count: pinnedMessages.length, key: "header-pinned" });
    for (const msg of pinnedMessages) {
      items.push({ type: "message", message: msg, key: `msg-${msg.account_id}-${msg.uid}` });
    }
  }

  // Collect messages into buckets, then emit in guaranteed order
  const buckets = new Map<DateBucket, MessageSummary[]>();
  for (const bucket of BUCKET_ORDER) {
    buckets.set(bucket, []);
  }

  for (const msg of regularMessages) {
    const ts = new Date(msg.date).getTime();
    const bucket = getDateBucket(ts, todayStart, yesterdayStart, thisWeekStart, lastWeekStart, thisMonthStart);
    buckets.get(bucket)!.push(msg);
  }

  for (const bucket of BUCKET_ORDER) {
    const msgs = buckets.get(bucket)!;
    if (msgs.length === 0) continue;
    // Sort within each bucket newest-first
    msgs.sort((a, b) => new Date(b.date).getTime() - new Date(a.date).getTime());
    items.push({ type: "header", label: bucket, count: msgs.length, key: `header-${bucket}` });
    for (const msg of msgs) {
      items.push({ type: "message", message: msg, key: `msg-${msg.account_id}-${msg.uid}` });
    }
  }

  return items;
}
