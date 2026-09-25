/**
 * The chat panel — a docked pane on the right edge of the main row.
 *
 * A pane, not a floating surface: it tiles the window like the reading pane,
 * so it paints the alpha-aware `bg-base` (gotcha #52). Its cards sit ON the
 * pane and use `bg-surface`.
 *
 * The component stays mounted while the panel is closed — it returns nothing
 * but keeps its `chat-event` listener — so a turn still running when you close
 * the panel is all there when you reopen it.
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  ArrowUp,
  Check,
  ChevronRight,
  FolderGit2,
  Loader2,
  Mail,
  ShieldAlert,
  Sparkles,
  Square,
  SquarePen,
  Terminal,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useChatStore } from "@/stores/chatStore";
import { useAccountStore } from "@/stores/accountStore";
import { useUIStore } from "@/stores/uiStore";
import { describeTool, draftAccountId, permissionFields, permissionTitle } from "@/lib/chatEvents";
import { openDraftForEdit } from "@/lib/draftCompose";
import type { ChatEnvelope, ChatItem } from "@/types/chat";

const MODELS: { value: string; label: string }[] = [
  { value: "", label: "Default model" },
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
];

const SUGGESTIONS = [
  "What needs a reply from me today?",
  "Write a follow-up to …",
  "Summarize my latest thread with …",
];

const basename = (p: string) => p.split("/").filter(Boolean).pop() ?? p;

export default function ChatPanel() {
  useEffect(() => {
    const un = listen<ChatEnvelope>("chat-event", (e) => useChatStore.getState().handleEnvelope(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const open = useChatStore((s) => s.open);
  if (!open) return null;
  return <ChatPane />;
}

function ChatPane() {
  const { items, busy, alive, starting, started, sessionModel, model } = useChatStore();
  const { setOpen, setModel, startNew, send, interrupt, continueInTerminal } = useChatStore();
  const [draft, setDraft] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Follow the stream unless the user has scrolled up to read something.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [items]);

  const submit = () => {
    const text = draft.trim();
    if (!text || busy || starting) return;
    setDraft("");
    stickToBottom.current = true;
    void send(text);
  };

  return (
    <aside
      aria-label="Chat with Claude"
      className="flex w-[380px] shrink-0 flex-col border-l border-border-subtle bg-base"
    >
      {/* Header */}
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border-subtle px-3">
        <Sparkles className="h-4 w-4 text-ai" />
        <span className="text-sm font-medium text-content">Claude</span>
        <span
          className={cn("h-1.5 w-1.5 rounded-full", alive ? "bg-success" : "bg-content-faint")}
          title={alive ? `Running${sessionModel ? ` · ${sessionModel}` : ""}` : "Not running"}
        />
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className="ml-auto rounded-md border border-border-subtle bg-input px-1.5 py-0.5 text-xs text-content-secondary outline-none"
          title="Model for the next new chat"
          aria-label="Model"
        >
          {MODELS.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
        </select>
        <IconButton label="New chat" onClick={() => void startNew()} disabled={starting}>
          <SquarePen className="h-3.5 w-3.5" />
        </IconButton>
        <IconButton
          label="Continue in terminal"
          onClick={() => void continueInTerminal()}
          disabled={!alive || busy || items.length === 0}
        >
          <Terminal className="h-3.5 w-3.5" />
        </IconButton>
        <IconButton label="Close (⌘L)" onClick={() => setOpen(false)}>
          <X className="h-3.5 w-3.5" />
        </IconButton>
      </div>

      {/* Where this conversation stands */}
      {started && (started.seed_subject || started.repo || started.missing_repo_path) && (
        <div className="flex shrink-0 flex-wrap gap-1.5 border-b border-border-subtle px-3 py-2">
          {started.seed_subject && (
            <Chip icon={<Mail className="h-3 w-3" />} title="The email this chat was opened from">
              {started.seed_subject}
            </Chip>
          )}
          {started.repo && (
            <Chip
              icon={<FolderGit2 className="h-3 w-3" />}
              title={`${started.repo.repo_path} (${started.repo.scope} ${started.repo.source})`}
            >
              {basename(started.repo.repo_path)}
            </Chip>
          )}
          {started.missing_repo_path && (
            <Chip icon={<AlertTriangle className="h-3 w-3 text-warning" />} title={started.missing_repo_path}>
              {basename(started.missing_repo_path)} isn&apos;t there
            </Chip>
          )}
        </div>
      )}

      {/* Transcript */}
      <div
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className="flex-1 space-y-3 overflow-y-auto overflow-x-hidden px-3 py-3"
      >
        {items.length === 0 && !starting && (
          <EmptyState
            readable={started?.readable_repos.length ?? null}
            onPick={(s) => {
              setDraft(s.endsWith("…") ? s.slice(0, -1) : s);
              inputRef.current?.focus();
            }}
          />
        )}
        {starting && (
          <div className="flex items-center gap-2 text-xs text-content-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin" /> Starting Claude…
          </div>
        )}
        {items.map((item) => (
          <ChatItemView key={item.id} item={item} />
        ))}
        {busy && !items.some((i) => i.kind === "permission" && i.state === "pending") && (
          <div className="flex items-center gap-2 text-xs text-content-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin" /> Working…
          </div>
        )}
      </div>

      {/* Composer */}
      <div className="shrink-0 border-t border-border-subtle p-2">
        <div className="flex items-end gap-2 rounded-lg border border-border-subtle bg-input px-2 py-1.5">
          <textarea
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                submit();
              }
            }}
            rows={Math.min(6, Math.max(1, draft.split("\n").length))}
            placeholder="Ask Claude about your mail…"
            className="max-h-40 min-h-[22px] flex-1 resize-none overflow-y-auto bg-transparent text-sm text-content outline-none [scrollbar-width:none] placeholder:text-content-faint"
          />
          {busy ? (
            <button
              onClick={() => void interrupt()}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-surface text-content-secondary hover:text-content"
              title="Stop this turn"
              aria-label="Stop"
            >
              <Square className="h-3 w-3 fill-current" />
            </button>
          ) : (
            <button
              onClick={submit}
              disabled={!draft.trim() || starting}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-accent text-white disabled:opacity-40"
              title="Send (Enter)"
              aria-label="Send"
            >
              <ArrowUp className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}

function IconButton({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className="rounded-md p-1 text-content-secondary transition-colors hover:bg-surface hover:text-content disabled:pointer-events-none disabled:opacity-40"
    >
      {children}
    </button>
  );
}

function Chip({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <span
      title={title}
      className="flex max-w-full items-center gap-1 truncate rounded-md bg-surface px-1.5 py-0.5 text-xs text-content-secondary"
    >
      {icon}
      <span className="truncate">{children}</span>
    </span>
  );
}

function EmptyState({ readable, onPick }: { readable: number | null; onPick: (s: string) => void }) {
  return (
    <div className="space-y-3 pt-6 text-center">
      <Sparkles className="mx-auto h-6 w-6 text-ai" />
      <p className="text-sm text-content-secondary">
        Claude can search your mail, read your project folders, and write drafts. Anything that
        changes mail asks you first, and nothing is ever sent.
      </p>
      {readable !== null && (
        <p className="text-xs text-content-muted">
          {readable === 0
            ? "No project folders linked yet — Settings → Claude repos."
            : `${readable} project folder${readable === 1 ? "" : "s"} readable.`}
        </p>
      )}
      <div className="flex flex-col items-center gap-1.5">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            onClick={() => onPick(s)}
            className="rounded-md bg-surface px-2.5 py-1 text-xs text-content-secondary hover:text-content"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}

function ChatItemView({ item }: { item: ChatItem }) {
  switch (item.kind) {
    case "user":
      return (
        <div className="flex justify-end">
          <div className="max-w-[85%] whitespace-pre-wrap rounded-lg bg-surface px-2.5 py-1.5 text-sm text-content">
            {item.text}
          </div>
        </div>
      );
    case "assistant":
      return (
        <div className="min-w-0 text-sm leading-relaxed text-content [overflow-wrap:anywhere]">
          {renderMarkdown(item.text)}
        </div>
      );
    case "tool":
      return <ToolRow item={item} />;
    case "permission":
      return <PermissionCard item={item} />;
    case "notice":
      return (
        <div
          className={cn(
            "whitespace-pre-wrap rounded-md px-2.5 py-1.5 text-xs",
            item.tone === "error" ? "bg-surface text-error" : "text-content-muted",
          )}
        >
          {item.text}
        </div>
      );
  }
}

function ToolRow({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const [expanded, setExpanded] = useState(false);
  const accounts = useAccountStore((s) => s.accounts);
  const addToast = useUIStore((s) => s.addToast);
  const icon =
    item.status === "running" ? (
      <Loader2 className="h-3 w-3 animate-spin" />
    ) : item.status === "error" ? (
      <AlertTriangle className="h-3 w-3 text-warning" />
    ) : (
      <Check className="h-3 w-3 text-success" />
    );

  const openDraft = async () => {
    if (!item.draft) return;
    const accountId = draftAccountId(item.draft, accounts);
    if (!accountId) {
      addToast({ message: "Couldn't tell which account that draft is in — find it in Drafts.", type: "error" });
      return;
    }
    await openDraftForEdit(accountId, item.draft.folder, item.draft.uid);
  };

  return (
    <div className="text-xs">
      <button
        onClick={() => setExpanded((v) => !v)}
        disabled={!item.result}
        className="flex w-full items-center gap-1.5 text-left text-content-secondary hover:text-content disabled:hover:text-content-secondary"
      >
        {icon}
        <span className="truncate">{describeTool(item.name, item.input)}</span>
        {item.result && (
          <ChevronRight className={cn("ml-auto h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")} />
        )}
      </button>
      {item.draft && (
        <button
          onClick={() => void openDraft()}
          className="mt-1.5 flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface px-2.5 py-1.5 text-xs text-content hover:border-accent"
        >
          <SquarePen className="h-3.5 w-3.5 text-accent" /> Open draft
        </button>
      )}
      {expanded && item.result && (
        <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface p-2 font-mono text-[11px] text-content-secondary">
          {item.result}
        </pre>
      )}
    </div>
  );
}

function PermissionCard({ item }: { item: Extract<ChatItem, { kind: "permission" }> }) {
  const answer = useChatStore((s) => s.answer);
  const ctx = item.context;
  const fields = permissionFields(item.input);
  const more = ctx ? ctx.message_count - ctx.messages.length : 0;
  const where = [ctx?.account, ctx?.folder && folderLabel(ctx.folder)].filter(Boolean).join(" · ");
  const verdict: Record<Exclude<typeof item.state, "pending">, string> = {
    allowed: "Allowed",
    allowed_always: "Allowed for the rest of this chat",
    denied: "Declined",
    expired: "No longer waiting",
  };
  return (
    <div className="rounded-lg border border-warning/40 bg-surface p-2.5 text-xs">
      <div className="flex items-start gap-1.5 text-[13px] font-medium text-content">
        <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
        <span>{permissionTitle(item.toolName, item.input, ctx)}</span>
      </div>

      {ctx && ctx.messages.length > 0 && (
        <ul className="mt-2 space-y-1.5 border-l-2 border-border pl-2.5">
          {ctx.messages.map((m, i) => (
            <li key={i} className="min-w-0">
              <div className="truncate text-content" title={m.subject}>
                {m.subject}
              </div>
              {m.from && <div className="truncate text-content-muted">from {m.from}</div>}
            </li>
          ))}
          {more > 0 && <li className="text-content-muted">and {more} more</li>}
        </ul>
      )}
      {ctx && ctx.messages.length === 0 && ctx.message_count > 0 && (
        <div className="mt-1.5 text-content-secondary">
          {ctx.message_count === 1 ? "An email" : `${ctx.message_count} emails`} not cached locally yet
        </div>
      )}
      {where && <div className="mt-1.5 truncate text-content-muted">in {where}</div>}

      {fields.length > 0 && (
        <dl className="mt-1.5 space-y-0.5 text-content-secondary">
          {fields.map(([k, v]) => (
            <div key={k} className="flex gap-1.5">
              <dt className="shrink-0 text-content-muted">{k}:</dt>
              <dd className="truncate" title={v}>
                {v}
              </dd>
            </div>
          ))}
        </dl>
      )}

      {item.state === "pending" ? (
        <div className="mt-2.5 flex gap-1.5">
          <button
            onClick={() => void answer(item.id, true, false)}
            className="rounded-md bg-accent px-2.5 py-1 text-white"
          >
            Allow
          </button>
          <button
            onClick={() => void answer(item.id, true, true)}
            className="rounded-md border border-border-subtle px-2.5 py-1 text-content-secondary hover:text-content"
            title={`Don't ask again for ${item.displayName} in this chat`}
          >
            Allow for this chat
          </button>
          <button
            onClick={() => void answer(item.id, false, false)}
            className="ml-auto rounded-md px-2.5 py-1 text-content-secondary hover:text-content"
          >
            Deny
          </button>
        </div>
      ) : (
        <div className="mt-1.5 text-content-muted">{verdict[item.state]}</div>
      )}

      {/* The exact call, for when the words above aren't enough. */}
      <details className="mt-1.5 text-content-faint">
        <summary className="cursor-pointer select-none hover:text-content-muted">Details</summary>
        <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-base p-1.5 font-mono text-[11px] text-content-muted">
          {item.toolName}
          {"\n"}
          {JSON.stringify(item.input, null, 2)}
        </pre>
      </details>
    </div>
  );
}

/** "INBOX" and "[Gmail]/All Mail" as a person names them. */
function folderLabel(folder: string): string {
  if (folder.toUpperCase() === "INBOX") return "Inbox";
  return folder.replace(/^\[Gmail\]\//, "");
}

/**
 * Just enough Markdown for chat replies — paragraphs, bullet and numbered
 * lists, **bold**, `code` — built as React nodes, never as HTML, so model
 * output that quotes an email can't inject markup.
 */
function renderMarkdown(text: string): ReactNode {
  const blocks = text.split(/\n{2,}/);
  return blocks.map((block, bi) => {
    const lines = block.split("\n");
    const isList = lines.every((l) => /^\s*([-*•]|\d+[.)])\s+/.test(l));
    if (isList) {
      const ordered = /^\s*\d/.test(lines[0]);
      const Tag = ordered ? "ol" : "ul";
      return (
        <Tag key={bi} className={cn("my-1.5 space-y-0.5 pl-5", ordered ? "list-decimal" : "list-disc")}>
          {lines.map((l, li) => (
            <li key={li}>{inline(l.replace(/^\s*([-*•]|\d+[.)])\s+/, ""))}</li>
          ))}
        </Tag>
      );
    }
    return (
      <p key={bi} className="my-1.5 whitespace-pre-wrap first:mt-0 last:mb-0">
        {inline(block.replace(/^#{1,6}\s+/gm, ""))}
      </p>
    );
  });
}

function inline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).map((part, i) => {
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      return <strong key={i}>{part.slice(2, -2)}</strong>;
    }
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      return (
        <code key={i} className="break-all rounded bg-surface px-1 font-mono text-[12px]">
          {part.slice(1, -1)}
        </code>
      );
    }
    return part;
  });
}
