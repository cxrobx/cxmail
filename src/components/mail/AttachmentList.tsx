import { useState } from "react";
import { Paperclip, Download, File, FileText, Image as ImageIcon, FileArchive, FileAudio, FileVideo } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import type { AttachmentMeta } from "@/types/email";

interface AttachmentListProps {
  accountId: string;
  folder: string;
  uid: number;
  attachments: AttachmentMeta[];
}

function iconFor(contentType: string): LucideIcon {
  const ct = contentType.toLowerCase();
  if (ct.startsWith("image/")) return ImageIcon;
  if (ct.startsWith("audio/")) return FileAudio;
  if (ct.startsWith("video/")) return FileVideo;
  if (ct === "application/pdf") return FileText;
  if (ct.includes("wordprocessingml") || ct === "application/msword") return FileText;
  if (ct.includes("spreadsheetml") || ct === "application/vnd.ms-excel") return FileText;
  if (ct.includes("presentationml") || ct === "application/vnd.ms-powerpoint") return FileText;
  if (ct.startsWith("text/")) return FileText;
  if (ct === "application/zip" || ct === "application/x-tar" || ct === "application/gzip" || ct === "application/x-7z-compressed") return FileArchive;
  return File;
}

export default function AttachmentList({ accountId, folder, uid, attachments }: AttachmentListProps) {
  const visible = attachments
    .map((att, index) => ({ att, index }))
    .filter(({ att }) => !(att.is_inline && att.content_id));

  const [pendingIndex, setPendingIndex] = useState<number | null>(null);
  const [status, setStatus] = useState<{ kind: "ok" | "err"; text: string } | null>(null);

  if (visible.length === 0) return null;

  const handleDownload = async (index: number, filename: string) => {
    setPendingIndex(index);
    setStatus(null);

    let savePath: string | null = null;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      savePath = await save({ defaultPath: filename });
    } catch (e) {
      console.error("Save dialog failed:", e);
      setStatus({ kind: "err", text: `Could not open save dialog: ${String(e)}` });
      setPendingIndex(null);
      return;
    }

    if (!savePath) {
      setPendingIndex(null);
      return;
    }

    try {
      console.info("download_attachment: invoking", { accountId, folder, uid, index, savePath });
      await api.compose.downloadAttachment(accountId, folder, uid, index, savePath);
      setStatus({ kind: "ok", text: `Saved ${filename}` });
    } catch (e) {
      console.error("Download attachment failed:", e);
      setStatus({ kind: "err", text: `Failed to save ${filename}: ${String(e)}` });
    } finally {
      setPendingIndex(null);
    }
  };

  return (
    <div className="shrink-0 border-t border-border-subtle px-4 py-3">
      <div className="mb-2 flex items-center gap-1.5 text-xs text-content-muted">
        <Paperclip className="h-3.5 w-3.5" />
        <span>
          {visible.length} attachment{visible.length === 1 ? "" : "s"}
        </span>
      </div>
      <div className="flex flex-wrap gap-2">
        {visible.map(({ att, index }) => {
          const Icon = iconFor(att.content_type);
          const displayName = att.filename || `attachment-${index + 1}`;
          const isPending = pendingIndex === index;
          return (
            <div
              key={index}
              className="flex items-center gap-2 rounded-md border border-border-subtle bg-surface px-3 py-2 text-xs"
            >
              <Icon className="h-4 w-4 shrink-0 text-content-muted" />
              <div className="flex min-w-0 flex-col">
                <span className="truncate text-content" title={displayName}>
                  {displayName}
                </span>
                <span className="text-content-muted">{formatBytes(att.size_bytes)}</span>
              </div>
              <button
                onClick={() => handleDownload(index, displayName)}
                disabled={isPending}
                className="ml-1 rounded p-1 text-content-secondary hover:bg-elevated hover:text-content disabled:opacity-50"
                title={isPending ? "Downloading..." : "Download"}
              >
                <Download className="h-3.5 w-3.5" />
              </button>
            </div>
          );
        })}
      </div>
      {status && (
        <p className={`mt-2 text-xs ${status.kind === "err" ? "text-red-400" : "text-content-muted"}`}>
          {status.text}
        </p>
      )}
    </div>
  );
}
