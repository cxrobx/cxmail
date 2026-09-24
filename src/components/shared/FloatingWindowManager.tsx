import { draftWindowKey, useWindowStore } from "@/stores/windowStore";
import FloatingWindow from "./FloatingWindow";
import FloatingEmailView from "@/components/mail/FloatingEmailView";
import ComposeModal from "@/components/mail/ComposeModal";
import type { ComposeMode } from "@/components/mail/ComposeModal";
import type { OutgoingAttachment, SavedDraftRef } from "@/types/email";

export default function FloatingWindowManager() {
  const windows = useWindowStore((s) => s.windows);
  const restoreWindow = useWindowStore((s) => s.restoreWindow);
  const closeWindow = useWindowStore((s) => s.closeWindow);
  const setWindowKey = useWindowStore((s) => s.setWindowKey);

  const activeWindows = windows.filter((w) => !w.isMinimized);
  const minimizedWindows = windows.filter((w) => w.isMinimized);

  return (
    <>
      {/* Active floating windows */}
      {activeWindows.map((w) => (
        <FloatingWindow
          key={w.id}
          id={w.id}
          title={w.title}
          position={w.position}
          size={w.size}
          zIndex={w.zIndex}
        >
          {w.type === "email" && (
            <FloatingEmailView
              accountId={w.props.accountId as string}
              folder={w.props.folder as string}
              uid={w.props.uid as number}
            />
          )}
          {w.type === "compose" && (
            <ComposeModal
              onClose={() => closeWindow(w.id)}
              mode={(w.props.mode as ComposeMode) || "new"}
              defaultTo={w.props.defaultTo as string | string[] | undefined}
              defaultCc={w.props.defaultCc as string[] | undefined}
              defaultBcc={w.props.defaultBcc as string[] | undefined}
              defaultSubject={w.props.defaultSubject as string | undefined}
              defaultBody={w.props.defaultBody as string | undefined}
              defaultAttachments={w.props.defaultAttachments as OutgoingAttachment[] | undefined}
              quotedHtml={w.props.quotedHtml as string | undefined}
              inReplyTo={w.props.inReplyTo as string | undefined}
              referencesHeader={w.props.referencesHeader as string | undefined}
              accountId={w.props.accountId as string | undefined}
              fromAddress={w.props.fromAddress as string | undefined}
              replyContext={
                w.props.replyContext as { accountId: string; folder: string; uid: number } | undefined
              }
              draftContext={
                (w.props.draftContext as
                  | (SavedDraftRef & { accountId: string })
                  | { accountId: string; folder: string; uid: number }
                  | undefined)
              }
              composeSurface="windowed"
              onDraftRefChange={(ref, aid) =>
                setWindowKey(w.id, ref && aid ? draftWindowKey(aid, ref.folder, ref.uid) : null)
              }
            />
          )}
        </FloatingWindow>
      ))}

      {/* Minimized windows bar */}
      {minimizedWindows.length > 0 && (
        <div className="fixed bottom-8 left-1/2 z-[200] flex -translate-x-1/2 items-center gap-1 rounded-lg border border-border-subtle bg-elevated p-1 shadow-xl">
          {minimizedWindows.map((w) => (
            <button
              key={w.id}
              onClick={() => restoreWindow(w.id)}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs text-content-secondary hover:bg-surface hover:text-content"
            >
              <span className="max-w-[120px] truncate">{w.title}</span>
            </button>
          ))}
        </div>
      )}
    </>
  );
}
