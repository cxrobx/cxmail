# MCP Send-Approval Bridge

**Status:** Proposed (spec only — no implementation)
**Related:** `specs/mcp.md` (permission tiers), gotcha #26 (bridge socket), recipient-integrity work

## Problem

The MCP `send_email` tool is currently **disabled by design**. The embedded server (running inside the Tauri app) could in principle drive the existing **Approve-tier** UI (`McpApprovalModal`, 60 s timeout, default reject — see `specs/mcp.md`). But the **standalone** `cxmail-mcp` binary — the one registered in `~/.mcp.json` and launched by an external Claude client with no CXMail window attached — has no UI surface, so it cannot obtain human approval. Rather than ship a tool that silently delivers mail from an unattended process, `send_email` returns guidance and never transmits:

> `send_email is disabled by design: the MCP never delivers mail directly. Use compose_draft … then have the user review it in CXMail's Drafts folder and click Send.`

That is the correct default (the whole point of a recipient-integrity effort is that no automated path delivers mail without a human seeing the exact recipients). This spec proposes a **safe way to re-enable send** for the standalone MCP: an approval bridge that requires an explicit human decision — made against the full recipient set — before any message leaves the machine.

## Current behavior (baseline to preserve)

- `compose_draft` / `edit_draft` create/replace a draft in the Drafts folder (Confirm tier). No transmission.
- `send_email` (`src-tauri/crates/cxmail-mcp/src/mcp/server.rs`) returns `invalid_params` guidance and does nothing.
- A Unix-domain-socket bridge already exists between the standalone MCP and a running app instance (`src-tauri/crates/cxmail-core/src/bridge.rs`, `notify` / `Envelope`; commit "Live MCP → app sync over a Unix-domain socket"). It is currently one-way (MCP → app cache-sync notifications).

## Goals

1. `send_email` can transmit **only** after an explicit, per-message human approval.
2. The human approves against the **full recipient set** — every To, Cc, and Bcc address — never a truncated or summarized view. (This is the direct tie-in to recipient integrity: the approval payload is the last checkpoint where a dropped recipient is visible.)
3. Default-deny everywhere: timeout, no bridge, malformed response, or ambiguous state all resolve to "not sent".
4. Works when the app is running (preferred: reuse `McpApprovalModal`), and degrades to a **hook-based** approval when it is not.

## Non-goals

- Re-enabling `send_email` unconditionally, or any "auto-approve" / "trusted sender" mode.
- Changing the Confirm-tier draft flow.
- Scheduled send (covered by `specs/scheduled-send-reliability.md`).

## Proposed design

### Path A — app-mediated approval (preferred, app running)

Extend the existing bridge from one-way notify into a **request/response** round-trip:

1. MCP `send_email` builds the outgoing message and an `ApprovalRequest` envelope (see payload below) and writes it to the bridge socket.
2. The app receives it, renders the existing `McpApprovalModal` populated with **From, full To/Cc/Bcc, Subject, and a body preview**, and a content hash.
3. The user picks Approve / Edit / Deny. The app replies with an `ApprovalResponse` envelope carrying the decision (and, for Edit, the amended draft handle).
4. On **Approve**, the app performs the SMTP send using its own credentials/session (it already owns the send path in `commands/compose.rs`) and returns the result to the MCP. The MCP never holds send credentials itself under Path A.
5. 60 s timeout, default **reject** (matches the Approve-tier contract in `specs/mcp.md`).

Having the **app** do the actual send (not the MCP) is deliberate: it keeps SMTP credentials and the single-writer DB in one process and means the approval and the send are the same action, not two hops that could diverge.

### Path B — hook-based approval (app not running)

When no app instance answers the bridge within a short probe window, fall back to a configurable **pre-send hook**:

- A user-configured command (e.g. `~/.config/cxmail/hooks/pre-send`) receives the approval payload as JSON on stdin.
- Exit `0` = approved; any non-zero exit or no hook configured = denied.
- The hook is responsible for surfacing the recipients to a human (terminal prompt, notification, etc.). CXMail ships no default auto-approving hook.
- Under Path B the standalone MCP performs the SMTP send itself only after a `0` exit, which requires it to have credential access; if it cannot read credentials, it denies.

Path B is opt-in. With no app and no configured hook, `send_email` behaves exactly as today (guidance + no send).

## Approval payload (contract)

```jsonc
{
  "type": "approval_request",
  "request_id": "uuid",           // single-use; response must echo it
  "account_id": "…",
  "from": "me@example.com",
  "to":  [{ "name": "Dana", "email": "dana@example.com" }, { "email": "sam@example.com" }],
  "cc":  [{ "email": "morgan@example.com" }],
  "bcc": [{ "email": "secret@example.com" }],
  "subject": "…",
  "body_sha256": "…",            // approver sees preview; hash pins what gets sent
  "expires_at": "ISO-8601 UTC"
}
```

- `to` / `cc` / `bcc` are **complete** address lists — the same `Vec<EmailAddress>` shape used across the codebase — so the modal/hook can display every recipient. Truncating them here would reintroduce the exact class of bug this effort exists to kill.
- The response must echo `request_id` (non-replayable) and carry `approve: bool`. A response for an unknown/expired `request_id` is ignored (treated as deny).

## Failure & security semantics

| Condition | Result |
|-----------|--------|
| App bridge answers → user Approves | Send |
| App bridge answers → Deny / Edit-cancel / 60 s timeout | No send |
| No app; hook exits 0 | Send (Path B) |
| No app; hook exits non-zero / missing / errors | No send (today's behavior) |
| Malformed or mismatched `request_id` response | No send |
| Body hash mismatch at send time | No send |

Invariants: per-message approval only; no persistent/blanket approval; default-deny on every ambiguous path; the approver always sees the full recipient set and the exact body (hash-pinned).

## Testing

- Unit: approval payload carries all To/Cc/Bcc with no truncation; response validation rejects wrong/expired `request_id`; timeout resolves to reject.
- Integration: app-running Approve → message sent; Deny/timeout → not sent; app-absent with 0-exit hook → sent; app-absent no hook → guidance/no send.
- Regression: a two-To + Cc + Bcc message surfaces all four addresses in the approval payload.

## Open questions

- Should Path B be gated behind an explicit config flag (default off) so a stray hook can never enable unattended sending by accident? (Recommended: yes.)
- Edit flow: does "Edit" hand back to `compose_draft`/`edit_draft` and require a fresh approval round-trip? (Recommended: yes — edited content invalidates the prior hash.)
