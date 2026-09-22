import { useState, useEffect } from "react";
import { listen, emit } from "@tauri-apps/api/event";
import { Shield, X, Check, Pencil } from "lucide-react";

interface ApprovalRequest {
  id: string;
  action: string;
  description: string;
  details: Record<string, string>;
}

// This card used to run a 60-second countdown and auto-deny at zero, mirroring a
// deadline the MCP was blocking on. Both are gone: the MCP returns immediately
// and approving here is what performs the delivery. The countdown was the actual
// bug — the card renders inside the main window, so a backgrounded CXMail ran the
// clock out against nobody and reported a denial the user never made. It now
// waits indefinitely; a request left unanswered for 24h is cleared at next launch
// by `db::gcal::sweep_stale_invite_approvals`.
export default function McpApprovalModal() {
  const [request, setRequest] = useState<ApprovalRequest | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    const unlisten = listen<ApprovalRequest>("mcp-approval-request", (event) => {
      setRequest(event.payload);
      setSubmitting(false);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleResponse = async (decision: "approved" | "denied" | "edit") => {
    if (!request || submitting) return;
    // Approving triggers a real Google Calendar round trip on the Rust side, so
    // guard against a double-click racing two decisions for one approval id.
    setSubmitting(true);
    await emit("mcp-approval-response", {
      id: request.id,
      decision,
    });
    setRequest(null);
  };

  if (!request) return null;

  return (
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-overlay">
      <div className="w-[440px] overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl">
        {/* Header */}
        <div className="flex items-center gap-3 border-b border-border-subtle bg-surface px-4 py-3">
          <Shield className="h-5 w-5 text-warning" />
          <div className="flex-1">
            <p className="text-sm font-medium text-content">
              Claude wants to {request.action}
            </p>
            <p className="text-xs text-content-secondary">
              Approval required &middot; nothing is sent until you approve
            </p>
          </div>
        </div>

        {/* Details */}
        <div className="space-y-2 px-4 py-3">
          <p className="text-sm text-content-secondary">{request.description}</p>
          <div className="rounded-md bg-surface p-3">
            {Object.entries(request.details).map(([key, value]) => (
              <div key={key} className="flex gap-2 py-0.5 text-sm">
                <span className="shrink-0 text-content-muted">{key}:</span>
                <span className="text-content-secondary">{value}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <button
            onClick={() => handleResponse("denied")}
            disabled={submitting}
            className="flex items-center gap-1.5 rounded-md bg-elevated px-4 py-2 text-sm text-content-secondary hover:bg-border disabled:opacity-50"
          >
            <X className="h-4 w-4" />
            Deny
          </button>
          <button
            onClick={() => handleResponse("edit")}
            disabled={submitting}
            className="flex items-center gap-1.5 rounded-md bg-elevated px-4 py-2 text-sm text-content-secondary hover:bg-border disabled:opacity-50"
          >
            <Pencil className="h-4 w-4" />
            Edit
          </button>
          <button
            onClick={() => handleResponse("approved")}
            disabled={submitting}
            className="flex items-center gap-1.5 rounded-md bg-accent px-4 py-2 text-sm font-medium text-content transition-colors hover:bg-accent-hover disabled:opacity-50"
          >
            <Check className="h-4 w-4" />
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}
