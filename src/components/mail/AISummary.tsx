import { useState, useEffect, useCallback } from "react";
import { api } from "@/lib/tauri";
import { Sparkles, Loader2, AlertCircle, ChevronDown, ChevronRight, Settings } from "lucide-react";
import AIProviderSettings from "./AIProviderSettings";

interface AISummaryProps {
  accountId: string;
  folder: string;
  uid: number;
}

export default function AISummary({ accountId, folder, uid }: AISummaryProps) {
  const [summary, setSummary] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Reset state when message changes
  useEffect(() => {
    setSummary(null);
    setIsLoading(false);
    setError(null);
    setCollapsed(false);
    setShowSettings(false);
  }, [accountId, folder, uid]);

  const handleSummarize = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    setShowSettings(false);
    try {
      const result = await api.ai.summarize(accountId, folder, uid);
      setSummary(result);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("API key not configured")) {
        setShowSettings(true);
      } else {
        setError(msg);
      }
    } finally {
      setIsLoading(false);
    }
  }, [accountId, folder, uid]);

  if (showSettings) {
    return <AIProviderSettings onClose={() => setShowSettings(false)} onReady={handleSummarize} />;
  }

  // Summary displayed
  if (summary) {
    return (
      <div className="border-t border-border-subtle bg-ai-surface px-4 py-2.5">
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="flex w-full items-center gap-2 text-left"
        >
          <Sparkles className="h-3.5 w-3.5 shrink-0 text-ai" />
          <span className="text-xs font-medium text-ai">AI Summary</span>
          {collapsed ? (
            <ChevronRight className="h-3 w-3 text-content-muted" />
          ) : (
            <ChevronDown className="h-3 w-3 text-content-muted" />
          )}
          <span
            role="button"
            tabIndex={0}
            aria-label="AI provider settings"
            onClick={(event) => { event.stopPropagation(); setShowSettings(true); }}
            onKeyDown={(event) => event.key === "Enter" && setShowSettings(true)}
            className="ml-auto rounded p-1 text-content-muted hover:text-content"
          >
            <Settings className="h-3.5 w-3.5" />
          </span>
        </button>
        {!collapsed && (
          <p className="mt-1.5 pl-[22px] text-sm leading-relaxed text-content-secondary">
            {summary}
          </p>
        )}
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="border-t border-border-subtle bg-ai-surface px-4 py-2.5">
        <div className="flex items-center gap-2">
          <AlertCircle className="h-3.5 w-3.5 text-red-400" />
          <span className="text-xs text-red-400">{error}</span>
          <button
            onClick={handleSummarize}
            className="ml-auto text-xs text-content-muted hover:text-content-secondary"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  // Default: summarize button
  return (
    <div className="border-t border-border-subtle px-4 py-2">
      <div className="flex items-center justify-between">
        <button
          onClick={handleSummarize}
          disabled={isLoading}
          className="flex items-center gap-1.5 rounded px-2 py-1 text-xs text-content-muted hover:bg-ai-surface hover:text-ai transition-colors disabled:opacity-50"
        >
          {isLoading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
          {isLoading ? "Summarizing..." : "Summarize"}
        </button>
        <button
          onClick={() => setShowSettings(true)}
          aria-label="AI provider settings"
          className="rounded p-1.5 text-content-muted hover:bg-ai-surface hover:text-content"
        >
          <Settings className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
