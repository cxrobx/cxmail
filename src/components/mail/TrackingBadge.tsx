import { useState, useEffect } from "react";
import { Eye } from "lucide-react";
import { api } from "@/lib/tauri";
import type { TrackingPixel } from "@/types/email";

interface TrackingBadgeProps {
  messageId: string | null;
}

export default function TrackingBadge({ messageId }: TrackingBadgeProps) {
  const [pixel, setPixel] = useState<TrackingPixel | null>(null);

  useEffect(() => {
    if (!messageId) return;
    api.tracking.getPixelForMessage(messageId).then(setPixel).catch((e) => console.error("Failed to load tracking pixel:", e));
  }, [messageId]);

  if (!pixel) return null;

  const hasOpens = pixel.open_count > 0;
  const timeAgo = pixel.last_open_at ? formatTimeAgo(pixel.last_open_at) : null;

  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs ${
        hasOpens
          ? "bg-success/10 text-success"
          : "bg-neutral-500/10 text-content-muted"
      }`}
      title={
        hasOpens
          ? `Opened ${pixel.open_count}x — first: ${pixel.first_open_at}, last: ${pixel.last_open_at}`
          : "Not opened yet"
      }
    >
      <Eye className="h-3 w-3" />
      {hasOpens ? `Opened ${pixel.open_count}x${timeAgo ? `, ${timeAgo}` : ""}` : "Not opened"}
    </span>
  );
}

function formatTimeAgo(isoDate: string): string {
  const diff = Date.now() - new Date(isoDate).getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
