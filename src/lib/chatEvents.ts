/**
 * The chat panel's reducer: `chat-event`s in, transcript items out. Pure, so
 * the ordering rules below are pinned by `chatEvents.test.ts`.
 *
 * With `--include-partial-messages` the CLI streams a text block as
 * `text_delta`s and THEN sends the whole block as `assistant_text`. The
 * complete block is authoritative — it replaces what the deltas built, so a
 * dropped delta can never leave a garbled message on screen.
 */
import type { ChatDraftRef, ChatEvent, ChatItem, PermissionContext } from "@/types/chat";

export interface ChatTranscript {
  items: ChatItem[];
  busy: boolean;
  alive: boolean;
}

let seq = 0;
const nextId = (prefix: string) => `${prefix}-${Date.now()}-${++seq}`;

function finishStreaming(items: ChatItem[]): ChatItem[] {
  const last = items[items.length - 1];
  if (last?.kind === "assistant" && last.streaming) {
    return [...items.slice(0, -1), { ...last, streaming: false }];
  }
  return items;
}

export function applyChatEvent(t: ChatTranscript, ev: ChatEvent): ChatTranscript {
  const items = t.items;
  const last = items[items.length - 1];
  switch (ev.type) {
    case "ready":
      return { ...t, alive: true };
    case "text_delta":
      if (last?.kind === "assistant" && last.streaming) {
        return { ...t, items: [...items.slice(0, -1), { ...last, text: last.text + ev.text }] };
      }
      return {
        ...t,
        items: [...items, { kind: "assistant", id: nextId("a"), text: ev.text, streaming: true }],
      };
    case "assistant_text":
      if (last?.kind === "assistant" && last.streaming) {
        return { ...t, items: [...items.slice(0, -1), { ...last, text: ev.text, streaming: false }] };
      }
      return {
        ...t,
        items: [...items, { kind: "assistant", id: nextId("a"), text: ev.text, streaming: false }],
      };
    case "tool_use": {
      const base = finishStreaming(items);
      if (base.some((i) => i.kind === "tool" && i.id === ev.id)) return { ...t, items: base };
      return {
        ...t,
        items: [
          ...base,
          { kind: "tool", id: ev.id, name: ev.name, input: ev.input, status: "running", result: null, draft: null },
        ],
      };
    }
    case "tool_result":
      return {
        ...t,
        items: items.map((i) =>
          i.kind === "tool" && i.id === ev.tool_use_id
            ? { ...i, status: ev.is_error ? "error" : "done", result: ev.text, draft: ev.draft }
            : i,
        ),
      };
    case "permission_request":
      return {
        ...t,
        items: [
          ...finishStreaming(items),
          {
            kind: "permission",
            id: ev.request_id,
            toolName: ev.tool_name,
            displayName: ev.display_name,
            input: ev.input,
            context: ev.context ?? null,
            state: "pending",
          },
        ],
      };
    case "turn_done": {
      const base = finishStreaming(items);
      const failed =
        ev.is_error && ev.subtype !== "success"
          ? [{ kind: "notice" as const, id: nextId("n"), text: turnErrorText(ev.subtype), tone: "error" as const }]
          : [];
      return { ...t, busy: false, items: [...base, ...failed] };
    }
    case "notice":
      return { ...t, items: [...items, { kind: "notice", id: nextId("n"), text: ev.text, tone: "info" }] };
    case "exited": {
      // A question nobody can answer any more must not keep its buttons.
      const base = finishStreaming(items).map((i) =>
        i.kind === "permission" && i.state === "pending" ? { ...i, state: "expired" as const } : i,
      );
      const crashed = t.alive && ev.code !== 0 && ev.code !== null;
      const tail = ev.stderr_tail.trim().split("\n").slice(-3).join("\n");
      return {
        ...t,
        busy: false,
        alive: false,
        items: crashed
          ? [
              ...base,
              {
                kind: "notice",
                id: nextId("n"),
                text: `Claude stopped unexpectedly (exit ${ev.code}).${tail ? `\n${tail}` : ""}`,
                tone: "error",
              },
            ]
          : base,
      };
    }
  }
}

function turnErrorText(subtype: string): string {
  switch (subtype) {
    case "error_max_turns":
      return "Claude hit its turn limit for this message.";
    case "error_during_execution":
      return "Claude hit an error mid-turn. The conversation is still open.";
    default:
      return `The turn ended with an error (${subtype}).`;
  }
}

/** Resolve a draft's account: the id when the call named one, else by address. */
export function draftAccountId(
  draft: ChatDraftRef,
  accounts: { id: string; email: string }[],
): string | null {
  if (draft.account_id) return draft.account_id;
  const email = draft.account_email?.toLowerCase();
  return accounts.find((a) => a.email.toLowerCase() === email)?.id ?? null;
}

const str = (input: unknown, key: string): string | null => {
  if (input && typeof input === "object" && key in input) {
    const v = (input as Record<string, unknown>)[key];
    return typeof v === "string" && v.trim() ? v.trim() : null;
  }
  return null;
};

const basename = (p: string) => p.split("/").filter(Boolean).pop() ?? p;

/** One line naming what a tool call did, in words rather than tool ids. */
export function describeTool(name: string, input: unknown): string {
  const short = name.startsWith("mcp__") ? name.split("__").slice(2).join("__") : name;
  const q = (s: string | null) => (s ? ` “${s.length > 48 ? `${s.slice(0, 47)}…` : s}”` : "");
  switch (short) {
    case "search_emails":
      return `Searched mail${q(str(input, "query"))}`;
    case "read_email":
      return "Read an email";
    case "read_thread":
      return "Read the thread";
    case "read_email_source":
      return "Read the raw headers";
    case "preview_email_html":
      return "Previewed a draft";
    case "resolve_project_repo":
      return "Looked up the project";
    case "list_voice_rules":
    case "get_voice_profile":
    case "extract_recipient_profile":
      return "Checked your writing voice";
    case "compose_draft":
      return `Drafted${q(str(input, "subject"))}`;
    case "edit_draft":
      return `Revised the draft${q(str(input, "subject"))}`;
    case "list_calendar_events":
      return "Checked the calendar";
    case "search_vault":
    case "related_notes":
      return `Searched notes${q(str(input, "query"))}`;
    case "Read": {
      const p = str(input, "file_path");
      return p ? `Read ${basename(p)}` : "Read a file";
    }
    case "Grep":
      return `Searched files${q(str(input, "pattern"))}`;
    case "Glob":
      return `Listed files${q(str(input, "pattern"))}`;
    case "Skill":
      return `Loaded a skill${q(str(input, "skill") ?? str(input, "command"))}`;
    default:
      return short.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
  }
}

/**
 * The permission card's question, in words: "Archive this email?", not
 * "archive_email {uid: 830}". The ids are what the model sent; the card has
 * to be something a person can check before clicking Allow.
 */
export function permissionTitle(toolName: string, input: unknown, ctx: PermissionContext | null): string {
  const short = toolName.startsWith("mcp__") ? toolName.split("__").slice(2).join("__") : toolName;
  const n = ctx?.message_count ?? 0;
  const these = n > 1 ? `these ${n} emails` : "this email";
  const bool = (k: string) => (input as Record<string, unknown> | null)?.[k];
  switch (short) {
    case "archive_email":
      return `Archive ${these}?`;
    case "delete_email":
      return bool("permanent") === true ? `Permanently delete ${these}?` : `Move ${these} to Trash?`;
    case "bulk_delete_emails":
      return `Delete ${n || "several"} email${n === 1 ? "" : "s"}?`;
    case "move_email":
      return `Move ${these} to ${str(input, "to_folder") ?? "another folder"}?`;
    case "flag_email": {
      const flag = str(input, "flag");
      const verb: Record<string, string> = {
        starred: "Star",
        unstarred: "Unstar",
        read: "Mark as read:",
        unread: "Mark as unread:",
      };
      return `${verb[flag ?? ""] ?? "Change the flag on"} ${these}?`;
    }
    case "download_attachment":
      return "Save an attachment to disk?";
    case "set_open_tracking": {
      const on = bool("enabled");
      return on === true ? "Turn open tracking on?" : on === false ? "Turn open tracking off?" : "Check the open-tracking setting?";
    }
    case "create_mail_rule":
      return "Create a mail rule?";
    case "update_mail_rule":
      return "Change a mail rule?";
    case "delete_mail_rule":
      return "Delete a mail rule?";
    case "apply_mail_rule":
      return "Apply a mail rule to existing mail?";
    case "create_group":
      return "Create an inbox group?";
    case "update_group":
      return "Change an inbox group?";
    case "delete_group":
      return "Delete an inbox group?";
    case "create_calendar_event":
      return `Add “${str(input, "summary") ?? "an event"}” to your calendar?`;
    case "update_calendar_event":
      return "Change a calendar event?";
    case "set_voice_rule":
      return "Pin a writing rule?";
    case "delete_voice_rule":
      return "Remove a pinned writing rule?";
    case "dismiss_nudge":
      return "Dismiss a nudge?";
    case "send_email":
      return "Send an email?";
    default:
      return `Allow: ${describeTool(toolName, input)}?`;
  }
}

/** Input keys the card already says in words (via the context), or that
 *  mean nothing to a person. They stay visible under "Details". */
const HIDDEN_FIELDS = new Set(["account_id", "folder", "from_folder", "uid", "uids", "confirmed"]);

/** The rest of the input as readable label/value lines. */
export function permissionFields(input: unknown): [string, string][] {
  if (!input || typeof input !== "object") return [];
  return Object.entries(input as Record<string, unknown>)
    .filter(([k, v]) => !HIDDEN_FIELDS.has(k) && !/(^id$|_id$|_ids$)/.test(k) && v !== null && v !== undefined && v !== "")
    .slice(0, 6)
    .map(([k, v]) => {
      const label = k.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
      let value: string;
      if (typeof v === "boolean") value = v ? "Yes" : "No";
      else if (typeof v === "string") value = v;
      else if (Array.isArray(v) && v.every((x) => typeof x === "string")) value = v.join(", ");
      else value = JSON.stringify(v);
      return [label, value.length > 140 ? `${value.slice(0, 139)}…` : value];
    });
}
