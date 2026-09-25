/**
 * The in-app chat's wire types — mirrors of `email::chat_agent::ChatEvent` and
 * `commands::chat::ChatStarted`. The Rust side is the source of truth; these
 * are serde's output, field for field.
 */
import type { ResolvedClaudeRepo } from "@/lib/tauri";

export interface ChatSeed {
  account_id: string;
  folder: string;
  uid: number;
}

export interface ChatStarted {
  gen: number;
  cwd: string;
  repo: ResolvedClaudeRepo | null;
  missing_repo_path: string | null;
  readable_repos: string[];
  mcp_servers: string[];
  seed_subject: string | null;
}

/** Where a draft the chat wrote lives. `account_id` is null when the call
 *  used the default account; `account_email` resolves it then. */
export interface ChatDraftRef {
  account_id: string | null;
  account_email: string | null;
  folder: string;
  uid: number;
}

/** What a permission question's ids point at, looked up by the app. */
export interface PermissionContext {
  account: string | null;
  folder: string | null;
  messages: { subject: string; from: string }[];
  message_count: number;
}

export type ChatEvent =
  | { type: "ready"; session_id: string; model: string; mcp_servers: { name: string; status: string }[] }
  | { type: "text_delta"; text: string }
  | { type: "assistant_text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; is_error: boolean; text: string; draft: ChatDraftRef | null }
  | {
      type: "permission_request";
      request_id: string;
      tool_name: string;
      display_name: string;
      input: unknown;
      context: PermissionContext | null;
    }
  | { type: "turn_done"; is_error: boolean; subtype: string; cost_usd: number | null; duration_ms: number | null }
  | { type: "notice"; text: string }
  | { type: "exited"; code: number | null; stderr_tail: string };

export interface ChatEnvelope {
  gen: number;
  event: ChatEvent;
}

export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string; streaming: boolean }
  | {
      kind: "tool";
      id: string;
      name: string;
      input: unknown;
      status: "running" | "done" | "error";
      result: string | null;
      draft: ChatDraftRef | null;
    }
  | {
      kind: "permission";
      id: string;
      toolName: string;
      displayName: string;
      input: unknown;
      context: PermissionContext | null;
      state: "pending" | "allowed" | "allowed_always" | "denied" | "expired";
    }
  | { kind: "notice"; id: string; text: string; tone: "info" | "error" };
