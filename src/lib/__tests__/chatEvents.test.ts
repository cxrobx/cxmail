import { describe, expect, it } from "vitest";
import { applyChatEvent, describeTool, draftAccountId, permissionFields, permissionTitle, type ChatTranscript } from "../chatEvents";
import type { ChatEvent } from "@/types/chat";

const empty: ChatTranscript = { items: [], busy: true, alive: true };
const run = (events: ChatEvent[], start = empty) => events.reduce(applyChatEvent, start);

describe("applyChatEvent", () => {
  it("streams text into one message, then lets the complete block replace it", () => {
    const t = run([
      { type: "text_delta", text: "Hel" },
      { type: "text_delta", text: "lo" },
      { type: "assistant_text", text: "Hello there." },
    ]);
    expect(t.items).toHaveLength(1);
    expect(t.items[0]).toMatchObject({ kind: "assistant", text: "Hello there.", streaming: false });
  });

  it("a text block after a tool call is a new message, not an append", () => {
    const t = run([
      { type: "text_delta", text: "Looking." },
      { type: "assistant_text", text: "Looking." },
      { type: "tool_use", id: "t1", name: "mcp__cxmail__search_emails", input: { query: "nick" } },
      { type: "tool_result", tool_use_id: "t1", is_error: false, text: "3 results", draft: null },
      { type: "text_delta", text: "Found him." },
    ]);
    expect(t.items.map((i) => i.kind)).toEqual(["assistant", "tool", "assistant"]);
    expect(t.items[2]).toMatchObject({ text: "Found him.", streaming: true });
  });

  it("a tool call ends a stream that never got its complete block", () => {
    const t = run([
      { type: "text_delta", text: "Partial" },
      { type: "tool_use", id: "t1", name: "Read", input: {} },
    ]);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: false });
  });

  it("joins a result to its call and carries the draft", () => {
    const draft = { account_id: "a1", account_email: null, folder: "Drafts", uid: 9 };
    const t = run([
      { type: "tool_use", id: "t1", name: "mcp__cxmail__compose_draft", input: { subject: "Re: scope" } },
      { type: "tool_result", tool_use_id: "t1", is_error: false, text: "Draft saved", draft },
    ]);
    expect(t.items[0]).toMatchObject({ kind: "tool", status: "done", draft });
  });

  it("a duplicate tool_use does not add a second row", () => {
    const ev: ChatEvent = { type: "tool_use", id: "t1", name: "Read", input: {} };
    expect(run([ev, ev]).items).toHaveLength(1);
  });

  it("turn_done clears busy, and only a real error adds a notice", () => {
    const ok = run([{ type: "turn_done", is_error: false, subtype: "success", cost_usd: 0.1, duration_ms: 5 }]);
    expect(ok.busy).toBe(false);
    expect(ok.items).toHaveLength(0);
    const bad = run([{ type: "turn_done", is_error: true, subtype: "error_max_turns", cost_usd: null, duration_ms: null }]);
    expect(bad.items[0]).toMatchObject({ kind: "notice", tone: "error" });
  });

  it("exit expires unanswered permission questions so their buttons go away", () => {
    const t = run([
      { type: "permission_request", request_id: "r1", tool_name: "mcp__cxmail__archive_email", display_name: "Archive Email", input: {}, context: null },
      { type: "exited", code: 0, stderr_tail: "" },
    ]);
    expect(t.items[0]).toMatchObject({ kind: "permission", state: "expired" });
    expect(t.alive).toBe(false);
    expect(t.busy).toBe(false);
  });

  it("a crash says so; a clean stop is silent", () => {
    const crash = run([{ type: "exited", code: 1, stderr_tail: "boom\nError: not logged in" }]);
    expect(crash.items[0]).toMatchObject({ kind: "notice", tone: "error" });
    expect((crash.items[0] as { text: string }).text).toContain("not logged in");
    const clean = run([{ type: "exited", code: 0, stderr_tail: "" }]);
    expect(clean.items).toHaveLength(0);
    const killed = run([{ type: "exited", code: null, stderr_tail: "" }]);
    expect(killed.items).toHaveLength(0);
  });
});

describe("draftAccountId", () => {
  const accounts = [{ id: "a1", email: "Me@Example.com" }];
  it("prefers the id, falls back to the address, case-insensitively", () => {
    expect(draftAccountId({ account_id: "x", account_email: null, folder: "D", uid: 1 }, accounts)).toBe("x");
    expect(draftAccountId({ account_id: null, account_email: "me@example.com", folder: "D", uid: 1 }, accounts)).toBe("a1");
    expect(draftAccountId({ account_id: null, account_email: "other@example.com", folder: "D", uid: 1 }, accounts)).toBeNull();
  });
});

describe("describeTool", () => {
  it("names what happened, not the tool id", () => {
    expect(describeTool("mcp__cxmail__search_emails", { query: "nick" })).toBe("Searched mail “nick”");
    expect(describeTool("mcp__cxmail__compose_draft", { subject: "Re: scope" })).toBe("Drafted “Re: scope”");
    expect(describeTool("Read", { file_path: "/Users/x/Projects/a/CLAUDE.md" })).toBe("Read CLAUDE.md");
    expect(describeTool("mcp__cxmail__archive_email", {})).toBe("Archive email");
  });
});

describe("permission card wording", () => {
  const one = { account: "me@example.com", folder: "INBOX", messages: [{ subject: "Re: scope", from: "Dana" }], message_count: 1 };
  it("asks a plain question, counting the emails", () => {
    expect(permissionTitle("mcp__cxmail__archive_email", { uid: 1 }, one)).toBe("Archive this email?");
    expect(permissionTitle("mcp__cxmail__archive_email", {}, { ...one, message_count: 3 })).toBe("Archive these 3 emails?");
    expect(permissionTitle("mcp__cxmail__delete_email", { permanent: true }, one)).toBe("Permanently delete this email?");
    expect(permissionTitle("mcp__cxmail__delete_email", {}, one)).toBe("Move this email to Trash?");
    expect(permissionTitle("mcp__cxmail__move_email", { to_folder: "Clients" }, one)).toBe("Move this email to Clients?");
    expect(permissionTitle("mcp__cxmail__flag_email", { flag: "starred" }, one)).toBe("Star this email?");
    expect(permissionTitle("mcp__cxmail__set_open_tracking", { enabled: false }, null)).toBe("Turn open tracking off?");
  });

  it("an unknown tool still gets a readable question", () => {
    expect(permissionTitle("mcp__cxmail__brand_new_tool", {}, null)).toBe("Allow: Brand new tool?");
  });

  it("hides ids the context already explains, and words the rest", () => {
    expect(
      permissionFields({ account_id: "6ed9…", folder: "INBOX", uid: 830, confirmed: true, permanent: false, to_folder: "Clients", rule_id: 4 }),
    ).toEqual([
      ["Permanent", "No"],
      ["To folder", "Clients"],
    ]);
  });
});
