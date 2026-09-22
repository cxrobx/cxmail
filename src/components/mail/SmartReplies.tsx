import { useState, useCallback, useEffect } from "react";
import { api } from "@/lib/tauri";
import { MessageSquare, Loader2, RefreshCw } from "lucide-react";

const replyCache = new Map<string, string[]>();

interface SmartRepliesProps {
  accountId: string;
  folder: string;
  uid: number;
  onSelectReply: (text: string) => void;
}

export default function SmartReplies({ accountId, folder, uid, onSelectReply }: SmartRepliesProps) {
  const cacheKey = `${accountId}:${folder}:${uid}`;
  const [replies, setReplies] = useState<string[]>(() => replyCache.get(cacheKey) ?? []);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  // Restore from cache on message change
  useEffect(() => {
    setReplies(replyCache.get(cacheKey) ?? []);
    setIsLoading(false);
    setError(null);
    setDismissed(false);
  }, [cacheKey]);

  const fetchReplies = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await api.ai.smartReplies(accountId, folder, uid);
      setReplies(result.replies);
      replyCache.set(cacheKey, result.replies);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("API key not configured")) {
        setError("AI features need a configured provider. Use AI settings beside Summarize.");
      } else {
        setError(msg);
      }
    } finally {
      setIsLoading(false);
    }
  }, [accountId, folder, uid, cacheKey]);

  if (dismissed) return null;

  // Show suggestion chips
  if (replies.length > 0) {
    return (
      <div className="border-t border-border-subtle px-4 py-2.5">
        <div className="flex items-center gap-2 mb-2">
          <MessageSquare className="h-3.5 w-3.5 text-ai" />
          <span className="text-xs font-medium text-ai">Smart Replies</span>
          <button
            onClick={fetchReplies}
            className="ml-auto text-content-faint hover:text-content-secondary transition-colors"
            title="Regenerate"
          >
            <RefreshCw className="h-3 w-3" />
          </button>
        </div>
        <div className="flex flex-wrap gap-2">
          {replies.map((reply, i) => (
            <button
              key={i}
              onClick={() => onSelectReply(reply)}
              className="rounded-full border border-border bg-ai-surface px-3 py-1.5 text-xs text-content-secondary hover:border-accent/50 hover:bg-ai-surface hover:text-content transition-colors text-left max-w-[280px]"
            >
              {reply}
            </button>
          ))}
        </div>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="border-t border-border-subtle px-4 py-2">
        <span className="text-xs text-red-400">{error}</span>
      </div>
    );
  }

  // Default: generate button
  return (
    <div className="border-t border-border-subtle px-4 py-2">
      <button
        onClick={fetchReplies}
        disabled={isLoading}
        className="flex items-center gap-1.5 rounded px-2 py-1 text-xs text-content-muted hover:bg-ai-surface hover:text-ai transition-colors disabled:opacity-50"
      >
        {isLoading ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <MessageSquare className="h-3.5 w-3.5" />
        )}
        {isLoading ? "Generating replies..." : "Suggest Replies"}
      </button>
    </div>
  );
}
