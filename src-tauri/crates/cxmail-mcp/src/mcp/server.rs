// `bridge` lives in cxmail-core: it has no crate:: imports of its own and the
// APP consumes it too, so the app would otherwise depend on this whole crate
// for two symbols.
use cxmail_core::bridge::{notify, Envelope};
use crate::db;
use crate::email::{
    gcal::{Conferencing, EventPatch, GcalClient, NewEvent, SendUpdates},
    imap, loose_text, parser, smtp, zoom_sync,
};
use futures::StreamExt;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, InitializeRequestParams, InitializeResult, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::ErrorData as McpError;
use rmcp::{schemars, tool, tool_handler, tool_router, RoleServer, ServerHandler};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

// ─── Parameter structs ────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchEmailsParams {
    #[schemars(description = "Search query string")]
    pub query: String,
    #[schemars(description = "Maximum results to return (default 20)")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Optional list of account IDs to restrict the search. Omit or leave empty for all accounts — which skips any account hidden from aggregated views (list_accounts shows 'Hidden from aggregates: yes'); name such an account here to search it. Use list_accounts to get IDs."
    )]
    pub account_ids: Option<Vec<String>>,
    #[schemars(
        description = "ISO 8601 date or timestamp (e.g. '2026-04-01' or '2026-04-01T00:00:00Z'). Only return messages with date >= this value (inclusive)."
    )]
    pub since: Option<String>,
    #[schemars(
        description = "ISO 8601 date or timestamp (e.g. '2026-04-09'). Only return messages with date < this value (exclusive)."
    )]
    pub until: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadEmailParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResolveProjectRepoParams {
    #[schemars(description = "Account ID of a message involving the person or project")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadEmailSourceParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
    #[schemars(
        description = "Optional case-insensitive header-name filter, e.g. [\"authentication-results\", \"received-spf\", \"dkim-signature\"]. Returns only matching header lines (with their folded continuations) instead of the whole block. Prefer this when you know which header you want — a full block is 1–8 KB of routing detail."
    )]
    pub headers: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewEmailHtmlParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX or [Gmail]/Drafts)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadThreadParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Message-ID header value")]
    pub message_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFoldersParams {
    #[schemars(description = "Account ID (omit for all accounts)")]
    pub account_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ComposeDraftParams {
    #[schemars(description = "Account ID to send from")]
    pub account_id: String,
    #[schemars(description = "Optional send-as address for the `From:` header. Omit and CXMail picks it: for a reply, the address the ORIGINAL was addressed to (matched against the account's send-as addresses over the original's To, Cc and delivery headers — Delivered-To, iCloud's Original-recipient); otherwise the account's own address. An account's send-as addresses are the ones configured in the app PLUS any address found on its own Sent mail. Pass one only to override that — `list_accounts` prints each account's send-as addresses. An address that is neither is REFUSED and nothing is written; add it in the app under the account's Send-as addresses first. SMTP still authenticates as the account, so the provider must also recognise the alias for a send to succeed.")]
    pub from: Option<String>,
    #[schemars(description = "Recipient email addresses")]
    pub to: Vec<String>,
    #[schemars(description = "Optional CC (carbon-copy) recipient email addresses")]
    pub cc: Option<Vec<String>>,
    #[schemars(
        description = "Optional BCC (blind carbon-copy) recipient email addresses — not visible to other recipients"
    )]
    pub bcc: Option<Vec<String>>,
    #[schemars(description = "Email subject")]
    pub subject: String,
    #[schemars(
        description = "Email body — plain text or HTML. Use HTML tags for formatting (e.g. <strong>bold</strong>, <ul><li>item</li></ul>). Pass exactly one of `body` or `instruction`."
    )]
    pub body: Option<String>,
    #[schemars(
        description = "Have a local model write the body INSTEAD of authoring it yourself: a plain-language description of what the email should say. The body is generated on this machine via the Antigravity CLI (`agy`) in the user's voice — the stored voice profile and pinned rules ride in the generation prompt — and then flows through the normal draft pipeline (dash rule, signature, quoting). For edit_draft, the draft's current cached body is handed to the writer as the text to rewrite. Mutually exclusive with `body`; incompatible with `layout` and with is_html=true (generated output is plain prose). The generated text is echoed in the result — review it, and use edit_draft to fix anything."
    )]
    pub instruction: Option<String>,
    #[schemars(
        description = "Model for instruction-driven writing. Omit to use the default configured in the app's AI settings (falling back to gemini-3.7-flash-high). Any id printed by `agy models` works. Only valid together with `instruction`."
    )]
    pub writer_model: Option<String>,
    #[schemars(
        description = "Set true when `body` is HTML so it is rendered as formatted markup; set false to force plain text. Omit to auto-detect. Always set true when you wrote HTML tags — relying on auto-detect can mis-handle tags like <br> or code containing '</'."
    )]
    pub is_html: Option<bool>,
    #[schemars(
        description = "Optional styled-HTML layout — for genuinely DESIGNED email only. Most email is plain prose plus simple markup (<strong>, <em>, <a>, <h2>, <ul><li>), and that survives CXMail's compose editor untouched: omit `layout` for it and the draft stays fully editable. \"rich\" — REQUIRED for hand-authored designed HTML (tables, styled <div>s, inline styles); preserves your fragment verbatim through the compose round-trip. Its cost: the body becomes one preserved block, so the user can retype text in place but cannot restructure it (no new paragraphs or bullets, and paste inside the block is refused) — do not use it for ordinary correspondence. \"card\" — wraps `body` in built-in card chrome (centered white card on a light-grey page, padding, rounded corners, soft shadow); supply only the message content as `body` (plain text or HTML; with layout=card the is_html flag controls how that inner content is parsed). Reserve \"card\" for standalone designed/marketing mail — in normal correspondence Gmail folds a card behind \"Show trimmed content\" (an empty dark box in dark mode). Omitting `layout` on designed markup means the design is STRIPPED when the user opens the draft in compose: tables, <div>s and inline styles are dropped and only their text survives as bare paragraphs. So never omit it for a designed email — and prefer real <ul> bullets over table-row bullets, so that content degrades to a list rather than run-on text. Unknown values are rejected; \"card\" and \"rich\" are the only accepted values. CRITICAL: all CSS must be INLINE (style=\"...\") — <style> blocks and class selectors do NOT render in email clients. Use table layout and inline styles, as in marketing HTML email."
    )]
    pub layout: Option<String>,
    #[schemars(
        description = "PREFERRED reply target: folder of the message being replied to, exactly as printed by read_email / search_emails / read_thread (e.g. \"INBOX\"). Must be passed together with reply_to_uid."
    )]
    pub reply_to_folder: Option<String>,
    #[schemars(
        description = "PREFERRED reply target: UID of the message being replied to, as printed by read_email / search_emails / read_thread. Must be passed together with reply_to_folder. UIDs are folder-scoped."
    )]
    pub reply_to_uid: Option<u32>,
    #[schemars(
        description = "FALLBACK reply target, used only when reply_to_folder/reply_to_uid are omitted: Message-ID of the email being replied to. Bracketed `<id@host>` or bare `id@host` both work. Prefer the folder+uid pair — it is what the app itself keys off, and it cannot be ambiguous."
    )]
    pub reply_to_message_id: Option<String>,
    #[schemars(
        description = "When replying, whether to append the quoted original message below the signature, Gmail-style (default true). Set false only for designed one-off emails where quoted history is unwanted. Never paste thread history into `body` yourself — it is added automatically, and pasting it puts the user's signature BELOW the quote."
    )]
    pub quote_original: Option<bool>,
    #[schemars(
        description = "Optional absolute file paths to attach. Each file is read from disk and embedded in the draft. Per-file 25 MB cap. Paths must be absolute and stay inside your home directory."
    )]
    pub attachments: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EditDraftParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Optional send-as address for the `From:` header. Omit and CXMail picks it: for a reply, the address the ORIGINAL was addressed to (matched against the account's send-as addresses over the original's To, Cc and delivery headers — Delivered-To, iCloud's Original-recipient); otherwise the account's own address. An account's send-as addresses are the ones configured in the app PLUS any address found on its own Sent mail. Pass one only to override that — `list_accounts` prints each account's send-as addresses. An address that is neither is REFUSED and nothing is written; add it in the app under the account's Send-as addresses first. SMTP still authenticates as the account, so the provider must also recognise the alias for a send to succeed.")]
    pub from: Option<String>,
    #[schemars(
        description = "PREFERRED: the draft's stable `draft_id`, as returned by compose_draft / edit_draft. It names the logical draft no matter how many times it has been re-saved since — every save (the compose window's autosave included) mints a new UID. Pass either this or `uid`, never both."
    )]
    pub draft_id: Option<String>,
    #[schemars(
        description = "Optional revision check, only with `draft_id`: the UID you last read for this draft. If it has been re-saved since (its current UID differs), nothing is written and the reply names the current revision. Omit to replace whatever the current revision is."
    )]
    pub expected_uid: Option<u32>,
    #[schemars(
        description = "DEPRECATED — the UID of the existing draft to replace; kept for one release, prefer `draft_id`. A UID names one revision and goes stale on every save; the reply to a `uid` call carries the draft_id to use from then on."
    )]
    pub uid: Option<u32>,
    #[schemars(description = "Recipient email addresses")]
    pub to: Vec<String>,
    #[schemars(description = "Optional CC (carbon-copy) recipient email addresses")]
    pub cc: Option<Vec<String>>,
    #[schemars(
        description = "Optional BCC (blind carbon-copy) recipient email addresses — not visible to other recipients"
    )]
    pub bcc: Option<Vec<String>>,
    #[schemars(description = "Email subject")]
    pub subject: String,
    #[schemars(
        description = "Email body — plain text or HTML. Use HTML tags for formatting (e.g. <strong>bold</strong>, <ul><li>item</li></ul>). Pass exactly one of `body` or `instruction`."
    )]
    pub body: Option<String>,
    #[schemars(
        description = "Have a local model write the body INSTEAD of authoring it yourself: a plain-language description of what the email should say. The body is generated on this machine via the Antigravity CLI (`agy`) in the user's voice — the stored voice profile and pinned rules ride in the generation prompt — and then flows through the normal draft pipeline (dash rule, signature, quoting). For edit_draft, the draft's current cached body is handed to the writer as the text to rewrite. Mutually exclusive with `body`; incompatible with `layout` and with is_html=true (generated output is plain prose). The generated text is echoed in the result — review it, and use edit_draft to fix anything."
    )]
    pub instruction: Option<String>,
    #[schemars(
        description = "Model for instruction-driven writing. Omit to use the default configured in the app's AI settings (falling back to gemini-3.7-flash-high). Any id printed by `agy models` works. Only valid together with `instruction`."
    )]
    pub writer_model: Option<String>,
    #[schemars(
        description = "Set true when `body` is HTML so it is rendered as formatted markup; set false to force plain text. Omit to auto-detect. Always set true when you wrote HTML tags — relying on auto-detect can mis-handle tags like <br> or code containing '</'."
    )]
    pub is_html: Option<bool>,
    #[schemars(
        description = "Optional styled-HTML layout — for genuinely DESIGNED email only. Most email is plain prose plus simple markup (<strong>, <em>, <a>, <h2>, <ul><li>), and that survives CXMail's compose editor untouched: omit `layout` for it and the draft stays fully editable. \"rich\" — REQUIRED for hand-authored designed HTML (tables, styled <div>s, inline styles); preserves your fragment verbatim through the compose round-trip. Its cost: the body becomes one preserved block, so the user can retype text in place but cannot restructure it (no new paragraphs or bullets, and paste inside the block is refused) — do not use it for ordinary correspondence. \"card\" — wraps `body` in built-in card chrome (centered white card on a light-grey page, padding, rounded corners, soft shadow); supply only the message content as `body` (plain text or HTML; with layout=card the is_html flag controls how that inner content is parsed). Reserve \"card\" for standalone designed/marketing mail — in normal correspondence Gmail folds a card behind \"Show trimmed content\" (an empty dark box in dark mode). Omitting `layout` on designed markup means the design is STRIPPED when the user opens the draft in compose: tables, <div>s and inline styles are dropped and only their text survives as bare paragraphs. So never omit it for a designed email — and prefer real <ul> bullets over table-row bullets, so that content degrades to a list rather than run-on text. Unknown values are rejected; \"card\" and \"rich\" are the only accepted values. CRITICAL: all CSS must be INLINE (style=\"...\") — <style> blocks and class selectors do NOT render in email clients. Use table layout and inline styles, as in marketing HTML email."
    )]
    pub layout: Option<String>,
    #[schemars(
        description = "PREFERRED reply target: folder of the message being replied to, exactly as printed by read_email / search_emails / read_thread (e.g. \"INBOX\"). Must be passed together with reply_to_uid."
    )]
    pub reply_to_folder: Option<String>,
    #[schemars(
        description = "PREFERRED reply target: UID of the message being replied to, as printed by read_email / search_emails / read_thread. Must be passed together with reply_to_folder. UIDs are folder-scoped."
    )]
    pub reply_to_uid: Option<u32>,
    #[schemars(
        description = "FALLBACK reply target, used only when reply_to_folder/reply_to_uid are omitted: Message-ID of the email being replied to. Bracketed `<id@host>` or bare `id@host` both work. Prefer the folder+uid pair — it is what the app itself keys off, and it cannot be ambiguous."
    )]
    pub reply_to_message_id: Option<String>,
    #[schemars(
        description = "When replying, whether to append the quoted original message below the signature, Gmail-style (default true). Set false only for designed one-off emails where quoted history is unwanted. Never paste thread history into `body` yourself — it is added automatically, and pasting it puts the user's signature BELOW the quote."
    )]
    pub quote_original: Option<bool>,
    #[schemars(
        description = "Optional absolute file paths to attach. Each file is read from disk and embedded in the draft (replacing any attachments on the prior draft). Per-file 25 MB cap. Paths must be absolute and stay inside your home directory."
    )]
    pub attachments: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MoveEmailParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Source folder")]
    pub from_folder: String,
    #[schemars(description = "Destination folder")]
    pub to_folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FlagEmailParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
    #[schemars(description = "Flag action: starred, unstarred, read, unread")]
    pub flag: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetOpenTrackingParams {
    #[schemars(
        description = "Account ID. Omit to use the first configured account. Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
    #[schemars(
        description = "true to enable open tracking, false to disable. Omit to just read the current state."
    )]
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendEmailParams {
    #[schemars(description = "Account ID to send from")]
    pub account_id: String,
    #[schemars(description = "Recipient email addresses")]
    pub to: Vec<String>,
    #[schemars(description = "Email subject")]
    pub subject: String,
    #[schemars(description = "Email body (plain text or HTML)")]
    pub body: String,
    #[schemars(
        description = "Message-ID of the email being replied to (for threading). Get this from read_email output."
    )]
    pub reply_to_message_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteEmailParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
    #[schemars(description = "Permanently delete (true) or move to trash (false, default)")]
    pub permanent: Option<bool>,
    #[schemars(
        description = "Must be set to true to confirm deletion. Required safety gate for destructive operations."
    )]
    pub confirmed: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BulkDeleteParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name")]
    pub folder: String,
    #[schemars(description = "List of message UIDs to delete")]
    pub uids: Vec<u32>,
    #[schemars(description = "Permanently delete (true) or move to trash (false, default)")]
    pub permanent: Option<bool>,
    #[schemars(description = "Must be set to true to confirm deletion.")]
    pub confirmed: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListAttachmentsParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GroupRuleInput {
    #[schemars(description = "Field to match. One of: from_email, from_name, subject, to_list")]
    pub field: String,
    #[schemars(description = "Operator. One of: contains, equals, starts_with, ends_with")]
    pub operator: String,
    #[schemars(description = "Value to match against (case-sensitive)")]
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateGroupParams {
    #[schemars(description = "Group display name (e.g. \"Work\", \"Newsletters\")")]
    pub name: String,
    #[schemars(description = "Hex color, default \"#0a84ff\"")]
    pub color: Option<String>,
    #[schemars(
        description = "Icon name, default \"folder\". Examples: folder, briefcase, music, user, mail"
    )]
    pub icon: Option<String>,
    #[schemars(
        description = "Filter rules. A message belongs to the group if it is from any listed account OR matches any rule. Default empty."
    )]
    pub rules: Option<Vec<GroupRuleInput>>,
    #[schemars(
        description = "Account IDs to include. Use list_accounts to look up IDs. Default empty."
    )]
    pub account_ids: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateGroupParams {
    #[schemars(description = "Group ID (from list_groups)")]
    pub id: i64,
    #[schemars(description = "Group display name")]
    pub name: String,
    #[schemars(description = "Hex color")]
    pub color: String,
    #[schemars(description = "Icon name")]
    pub icon: String,
    #[schemars(description = "Filter rules — replaces ALL existing rules")]
    pub rules: Vec<GroupRuleInput>,
    #[schemars(description = "Account IDs — replaces ALL existing memberships")]
    pub account_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteGroupParams {
    #[schemars(description = "Group ID (from list_groups)")]
    pub id: i64,
    #[schemars(
        description = "Must be set to true to confirm deletion. Required safety gate for destructive operations."
    )]
    pub confirmed: Option<bool>,
}

// ─── Mail rules (the classification engine, not inbox groups) ─────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MailRuleConditionInput {
    #[schemars(
        description = "Field to match. One of: from, subject, to. (\"body\" exists in the stored data model but is REJECTED here: rules run at classification time, when only headers are loaded, so a body condition can never match.) \"to\" matches the full recipient list."
    )]
    pub field: String,
    #[schemars(
        description = "Operator. One of: contains, equals, starts_with, ends_with. Matching is case-insensitive."
    )]
    pub operator: String,
    #[schemars(description = "Value to match against. Case-insensitive; must be non-empty.")]
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MailRuleActionInput {
    #[schemars(
        description = "One of: set_category, mark_read, mark_flagged. \"move\" and \"delete\" exist in the stored data model but are REJECTED here — they are never executed by the classifier, so such a rule would be silently dead."
    )]
    pub action_type: String,
    #[schemars(
        description = "Required for set_category: one of primary, updates, social, promotions, junk. Ignored for mark_read / mark_flagged."
    )]
    pub value: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateMailRuleParams {
    #[schemars(description = "Rule display name, e.g. \"DMARC reports out of inbox\"")]
    pub name: String,
    #[schemars(
        description = "Restrict the rule to one account. Use list_accounts to look up IDs. Omit for a global rule that applies to every account."
    )]
    pub account_id: Option<String>,
    #[schemars(description = "Whether the rule is enabled. Default true.")]
    pub is_active: Option<bool>,
    #[schemars(
        description = "Evaluation priority; rules are listed highest-first. Default 0. Note that ALL matching rules apply — priority orders them, it does not stop at the first match."
    )]
    pub priority: Option<i32>,
    #[schemars(
        description = "Conditions, ALL of which must match (logical AND — there is no OR). At least one is required: a rule with no conditions would match every message."
    )]
    pub conditions: Vec<MailRuleConditionInput>,
    #[schemars(description = "Actions applied when every condition matches. At least one required.")]
    pub actions: Vec<MailRuleActionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateMailRuleParams {
    #[schemars(description = "Rule ID (from list_mail_rules)")]
    pub id: i64,
    #[schemars(description = "Rule display name")]
    pub name: String,
    #[schemars(
        description = "Restrict to one account, or omit for a global rule. This REPLACES the current value — omitting it makes a previously account-scoped rule global."
    )]
    pub account_id: Option<String>,
    #[schemars(description = "Whether the rule is enabled. Default true.")]
    pub is_active: Option<bool>,
    #[schemars(description = "Evaluation priority. Default 0.")]
    pub priority: Option<i32>,
    #[schemars(description = "Conditions, replaced in full. At least one required.")]
    pub conditions: Vec<MailRuleConditionInput>,
    #[schemars(description = "Actions, replaced in full. At least one required.")]
    pub actions: Vec<MailRuleActionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DismissNudgeParams {
    #[schemars(description = "Account ID, exactly as printed by list_nudges")]
    pub account_id: String,
    #[schemars(description = "Thread key, exactly as printed by list_nudges")]
    pub thread_key: String,
    #[schemars(
        description = "Which lane to silence: \"follow_up\" (we're waiting on them) or \"reply\" (they're waiting on us). Dismissing one does not affect the other."
    )]
    pub kind: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteMailRuleParams {
    #[schemars(description = "Rule ID (from list_mail_rules)")]
    pub id: i64,
    #[schemars(
        description = "Must be set to true to confirm deletion. Required safety gate for destructive operations."
    )]
    pub confirmed: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewMailRuleParams {
    #[schemars(
        description = "Preview an existing rule by ID. Mutually exclusive with `conditions` — supply exactly one of the two."
    )]
    pub rule_id: Option<i64>,
    #[schemars(
        description = "Preview a hypothetical rule's conditions without creating it. Mutually exclusive with `rule_id`."
    )]
    pub conditions: Option<Vec<MailRuleConditionInput>>,
    #[schemars(
        description = "Restrict the preview to one account. Omit to scan every account. Ignored when rule_id names an account-scoped rule."
    )]
    pub account_id: Option<String>,
    #[schemars(description = "Maximum sample messages to list back (default 20, max 200).")]
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyMailRuleParams {
    #[schemars(description = "ID of an existing rule to apply. Use list_mail_rules to look it up.")]
    pub id: i64,
    #[schemars(
        description = "Set false to actually write the changes. DEFAULTS TO TRUE (dry run) — a dry run reports exactly what a real run would change and modifies nothing."
    )]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "Restrict the run to one account. Omit to cover every account. Ignored when the rule is already account-scoped."
    )]
    pub account_id: Option<String>,
    #[schemars(description = "Maximum affected messages to list back (default 20, max 200).")]
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetVoiceProfileParams {
    #[schemars(
        description = "Account ID. Omit to use the first configured account. Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
    #[schemars(
        description = "Recipient email address. Returns the per-recipient voice profile if one is cached. Mutually exclusive with archetype_id."
    )]
    pub recipient_email: Option<String>,
    #[schemars(
        description = "Archetype ID (uuid) from list_archetypes. Mutually exclusive with recipient_email. If both omitted, returns the account-level profile."
    )]
    pub archetype_id: Option<String>,
    #[schemars(
        description = "Whether to include up to 3 plain-text excerpts from past sent mail (default true). Excerpts are signature-stripped, quote-stripped, and capped at 400 characters each."
    )]
    pub include_samples: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExtractRecipientProfileParams {
    #[schemars(
        description = "Account ID. Omit to use the first configured account. Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
    #[schemars(description = "Recipient email address (required).")]
    pub recipient_email: String,
    #[schemars(
        description = "If true, rebuild even when the cached profile is still fresh. Default false (skip the LLM call when a fresh cache hit exists)."
    )]
    pub force: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListArchetypesParams {
    #[schemars(
        description = "Account ID (omit for all accounts). Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetVoiceRuleParams {
    #[schemars(
        description = "Account ID. Omit to use the first configured account. Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
    #[schemars(
        description = "Recipient email address to pin this rule for. Omit for an account-wide rule that applies to every message sent from this account."
    )]
    pub recipient_email: Option<String>,
    #[schemars(
        description = "\"recipient\" or \"account\". Optional — inferred from recipient_email (present = recipient, absent = account). Pass it only to be explicit; a scope that contradicts recipient_email is rejected."
    )]
    pub scope: Option<String>,
    #[schemars(
        description = "The rule, written as a plain-English instruction the drafting agent must obey — e.g. 'Always address as \"Bro. Ellis\", never \"Sam\".' or 'Never open with \"Hope you're well\".' Max 500 characters. Write one instruction per rule rather than a paragraph of several."
    )]
    pub rule: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListVoiceRulesParams {
    #[schemars(
        description = "Account ID. Omit to use the first configured account. Use list_accounts to look up IDs."
    )]
    pub account_id: Option<String>,
    #[schemars(
        description = "Show the rules that apply when writing to this address: the account-wide rules plus that recipient's own. Omit to list every rule on the account."
    )]
    pub recipient_email: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteVoiceRuleParams {
    #[schemars(description = "Rule id, as printed by list_voice_rules.")]
    pub id: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DownloadAttachmentParams {
    #[schemars(description = "Account ID")]
    pub account_id: String,
    #[schemars(description = "Folder name (e.g. INBOX)")]
    pub folder: String,
    #[schemars(description = "Message UID")]
    pub uid: u32,
    #[schemars(
        description = "Attachment index from list_attachments. Provide this OR `filename`, not both."
    )]
    pub index: Option<usize>,
    #[schemars(
        description = "Attachment filename from list_attachments. Provide this OR `index`, not both. Case-insensitive fallback is applied."
    )]
    pub filename: Option<String>,
    #[schemars(
        description = "Where to save. Omit for ~/Downloads/<filename>. A trailing slash or existing directory is treated as a directory; otherwise the value is the full file path. Paths escaping the user's home directory are rejected."
    )]
    pub save_path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListCalendarEventsParams {
    #[schemars(description = "Account ID to filter by. Omit for all connected accounts.")]
    pub account_id: Option<String>,
    #[schemars(description = "Inclusive ISO-8601 range start.")]
    pub start: String,
    #[schemars(description = "Exclusive ISO-8601 range end.")]
    pub end: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateCalendarEventParams {
    #[schemars(description = "Gmail account ID that owns the event.")]
    pub account_id: String,
    #[schemars(description = "Event title.")]
    pub summary: String,
    #[schemars(description = "RFC-3339 date/time or local YYYY-MM-DDTHH:MM.")]
    pub start: String,
    #[schemars(description = "RFC-3339 date/time or local YYYY-MM-DDTHH:MM.")]
    pub end: String,
    #[schemars(description = "IANA time zone, for example America/New_York.")]
    pub time_zone: String,
    pub description: Option<String>,
    pub location: Option<String>,
    #[schemars(description = "Complete attendee email list. Attendees are saved but not notified.")]
    pub attendees: Option<Vec<String>>,
    #[schemars(
        description = "Conferencing provider: \"meet\" (default), \"zoom\", or \"none\". Use \"zoom\" only when the user needs computer audio in a screen share — Google Meet cannot share system audio from macOS. Requires Zoom credentials in CXMail's Zoom settings; the join link is written into the event description. Zoom is rejected for all-day and recurring events."
    )]
    pub conference: Option<String>,
    #[schemars(
        description = "DEPRECATED — use `conference` instead. Create a real Google Meet conference link. Default true. Passing this alongside a contradicting `conference` value is rejected."
    )]
    pub add_meet: Option<bool>,
    #[schemars(description = "Google calendar ID. Omit to use the primary calendar.")]
    pub calendar_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateCalendarEventParams {
    #[schemars(description = "Local Google Calendar event row ID from list_calendar_events.")]
    pub event_id: i64,
    pub summary: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub time_zone: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    #[schemars(description = "Complete replacement attendee list. Changes are not notified.")]
    pub attendees: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendCalendarInvitesParams {
    #[schemars(description = "Local Google Calendar event row ID.")]
    pub event_id: i64,
    #[schemars(description = "Must be true after the user reviews the full attendee list.")]
    pub confirmed: Option<bool>,
}

/// Look up the default identity's signature HTML for an account.
fn get_signature_html(conn: &rusqlite::Connection, account_id: &str) -> Option<String> {
    db::identities::list_by_account(conn, account_id)
        .ok()
        .and_then(|ids| ids.into_iter().find(|i| i.is_default))
        .and_then(|i| i.signature_html)
        .filter(|s| !s.is_empty())
}

/// The address a draft goes out FROM, and the signature that belongs to it.
///
/// Precedence: an explicit `from` param → the send-as the message being
/// replied to was addressed to → the account's own address. The middle rung is
/// the point of the feature: mail that arrived at an alias is answered from
/// that alias without the agent having to notice.
///
/// One matcher with the app (gotcha #36) — `db::identities::match_send_as` is
/// the same function `commands::settings::reply_from_for_message` calls, so
/// the MCP and the compose window cannot pick different addresses for the same
/// reply. Refusal of an unconfigured address is `invalid_params`: it is a
/// caller error, and it lands before any IMAP work (#30's ordering).
fn resolve_draft_from(
    conn: &rusqlite::Connection,
    account: &db::accounts::Account,
    requested: Option<&str>,
    target: Option<&ReplyTarget>,
    current: Option<&str>,
) -> Result<db::identities::SendAsAddress, McpError> {
    let addresses = db::identities::send_as_addresses(
        conn,
        &account.id,
        &account.email,
        account.display_name.as_deref(),
    )
    .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    if let Some(requested) = requested.map(str::trim).filter(|r| !r.is_empty()) {
        return db::identities::resolve_send_as(&addresses, Some(requested))
            .map_err(|e| McpError::invalid_params(format!("{e} Nothing was written."), None));
    }

    if let Some(target) = target {
        if let Some((folder, uid)) = reply_target_coordinates(conn, &account.id, target) {
            if let Ok(recipients) =
                db::messages::recipients_for_send_as_match(conn, &account.id, &folder, uid)
            {
                if let Some(hit) = db::identities::match_send_as(&addresses, &recipients) {
                    return Ok(hit.clone());
                }
            }
        }
    }

    // The revision being replaced already had a From, and an edit that does
    // not mention one is not a request to change it. Silently ignored when it
    // is no longer a configured send-as (the user removed the alias between
    // saves) — that is a fall back to the primary, not a refusal, because the
    // caller asked for nothing.
    if let Some(current) = current.map(str::trim).filter(|c| !c.is_empty()) {
        if let Ok(hit) = db::identities::resolve_send_as(&addresses, Some(current)) {
            return Ok(hit);
        }
    }

    db::identities::resolve_send_as(&addresses, None)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))
}

/// The `(folder, uid)` a reply target names, when the local cache knows it.
/// Read-only and best-effort: it feeds a DEFAULT, so an unknown target simply
/// leaves the account's own address in place.
fn reply_target_coordinates(
    conn: &rusqlite::Connection,
    account_id: &str,
    target: &ReplyTarget,
) -> Option<(String, u32)> {
    match target {
        ReplyTarget::Coordinates { folder, uid } => Some((folder.clone(), *uid)),
        ReplyTarget::MessageId(mid) => db::messages::get_reply_source(conn, account_id, mid)
            .ok()
            .flatten()
            .map(|src| (src.folder_name, src.uid)),
    }
}

/// The `From:` the revision being replaced already carried, read from the local
/// cache. Best-effort and read-only — it feeds a DEFAULT, so an uncached draft
/// (or one another client wrote) simply leaves the account's own address in
/// place. Resolved here rather than after the v60 claim so an explicit `from`
/// can still be refused before anything is claimed or written (#30/#57).
fn current_draft_from_address(
    conn: &rusqlite::Connection,
    account: &db::accounts::Account,
    target: &EditTarget,
) -> Option<String> {
    let (folder, uid) = match target {
        EditTarget::ById {
            draft_id,
            expected_uid,
        } => {
            let row = db::drafts::get(conn, draft_id).ok().flatten()?;
            (row.folder_name.clone(), expected_uid.unwrap_or(row.current_uid))
        }
        EditTarget::ByUid(uid) => (
            db::folders::folder_for_account(conn, &account.id, &account.provider, "drafts"),
            *uid,
        ),
    };
    db::messages::get_by_uid(conn, &account.id, &folder, uid)
        .ok()
        .flatten()
        .and_then(|row| row.from_email)
        .filter(|e| !e.trim().is_empty())
}

/// The signature to use for a draft sent from `from`: the alias's own when it
/// has one, otherwise the account's default identity signature. An alias
/// without a signature inherits rather than sending unsigned.
fn signature_for_send_as(
    conn: &rusqlite::Connection,
    account_id: &str,
    from: &db::identities::SendAsAddress,
) -> Option<String> {
    from.signature_html
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| get_signature_html(conn, account_id))
}

/// A one-line note naming the From, emitted only when it is NOT the account's
/// own address — silence for the ordinary case, so the note means something
/// when it appears (#34's reasoning).
fn send_as_note(from: &db::identities::SendAsAddress, explicit: bool) -> String {
    if from.is_primary {
        return String::new();
    }
    let why = if explicit {
        "as requested"
    } else {
        "the address the original was sent to"
    };
    format!("\n✉ From: {} ({why}).", from.email)
}

/// Append signature HTML to body, matching the frontend's pattern.
fn append_signature(html_body: &str, signature: &str) -> String {
    format!(
        r#"{}<div class="email-signature" style="margin-top:16px;">{}</div>"#,
        html_body, signature
    )
}

/// Which message a reply threads onto.
///
/// Every MCP read tool already hands the agent `Account | Folder | UID` —
/// the exact key the frontend quotes from. The Message-ID path re-derives
/// those coordinates from a string and is therefore the fallback, not the
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplyTarget {
    Coordinates { folder: String, uid: u32 },
    /// Always normalized to the canonical bracketed form at construction.
    MessageId(String),
}

/// Decide what the caller is replying to.
///
/// | folder | uid | message_id | result |
/// |---|---|---|---|
/// | Some | Some | any | `Coordinates` — wins, message_id ignored |
/// | Some | None | — | invalid_params |
/// | None | Some | — | invalid_params |
/// | None | None | Some | `MessageId` (normalized) |
/// | None | None | None | `None` — not a reply |
fn resolve_reply_target(
    folder: Option<&str>,
    uid: Option<u32>,
    message_id: Option<&str>,
) -> Result<Option<ReplyTarget>, McpError> {
    let folder = folder.map(str::trim).filter(|s| !s.is_empty());
    match (folder, uid) {
        (Some(f), Some(u)) => {
            return Ok(Some(ReplyTarget::Coordinates {
                folder: f.to_string(),
                uid: u,
            }))
        }
        (Some(_), None) => {
            return Err(McpError::invalid_params(
                "reply_to_folder requires reply_to_uid. Pass both (read_email / search_emails / read_thread print Folder and UID together) or neither.",
                None,
            ))
        }
        (None, Some(_)) => {
            return Err(McpError::invalid_params(
                "reply_to_uid requires reply_to_folder. IMAP UIDs are folder-scoped, so a UID on its own addresses a different message in every folder.",
                None,
            ))
        }
        (None, None) => {}
    }

    let Some(raw) = message_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let normalized = crate::email::message_id::normalize_message_id(raw).ok_or_else(|| {
        McpError::invalid_params(
            format!(
                "reply_to_message_id {:?} is not a Message-ID. Expected `<id@host>` (bare `id@host` is also accepted). Prefer reply_to_folder + reply_to_uid.",
                raw
            ),
            None,
        )
    })?;
    Ok(Some(ReplyTarget::MessageId(normalized)))
}

/// Resolve threading headers for a reply target.
/// Returns `(in_reply_to, references)` matching the frontend's ReadingPane logic.
///
/// The coordinates path reads headers off the very row being quoted, so the
/// two can't disagree. The Message-ID path keeps the historical
/// "not in DB → thread on the caller's ID" fallback, but normalized, so the
/// emitted headers are at least RFC-valid instead of a bare self-referential
/// chain.
fn resolve_threading(
    conn: &rusqlite::Connection,
    account_id: &str,
    target: &ReplyTarget,
) -> (Option<String>, Option<String>) {
    let (mid, ref_ids) = match target {
        ReplyTarget::Coordinates { folder, uid } => {
            match db::messages::get_message_headers(conn, account_id, folder, *uid) {
                // No local row → no headers. `build_reply_context` backfills
                // these from the IMAP-fetched message rather than shipping an
                // unthreaded reply.
                Ok((mid, refs, _in_reply_to)) => (mid, refs),
                Err(e) => {
                    log::warn!("MCP reply threading: header lookup failed for {folder}/{uid}: {e}");
                    (None, None)
                }
            }
        }
        ReplyTarget::MessageId(mid) => {
            match db::messages::get_threading_headers(conn, account_id, mid) {
                Ok(Some((db_mid, refs))) => (Some(db_mid), refs),
                _ => (Some(mid.clone()), None),
            }
        }
    };
    build_threading_headers(mid.as_deref(), ref_ids.as_deref())
}

/// `(In-Reply-To, References)` from a parent Message-ID and its chain, both
/// normalized to canonical bracketed form (gotcha #30).
fn build_threading_headers(
    parent_mid: Option<&str>,
    parent_refs: Option<&str>,
) -> (Option<String>, Option<String>) {
    use crate::email::message_id::{normalize_message_id, normalize_reference_chain};
    let Some(mid) = parent_mid.and_then(normalize_message_id) else {
        return (None, None);
    };
    let mut chain = parent_refs.map(normalize_reference_chain).unwrap_or_default();
    if !chain.is_empty() {
        chain.push(' ');
    }
    chain.push_str(&mid);
    (Some(mid), Some(chain))
}

/// Message-ID for display, always in canonical bracketed form, so the ID an
/// agent copies out of a read tool is spelled the same regardless of which
/// code path produced it (gotcha #30). Unparseable input passes through
/// verbatim rather than vanishing.
fn normalized_mid(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("");
    crate::email::message_id::normalize_message_id(raw).unwrap_or_else(|| raw.to_string())
}

/// Pull named headers out of a verbatim RFC 5322 header block.
///
/// Folding-aware, which is the whole difficulty: `Authentication-Results` and
/// `Received` are routinely wrapped across several lines, and a naive
/// line-prefix filter returns the first line and silently drops the verdict
/// that follows it. A continuation line is any line starting with SP or HTAB
/// (RFC 5322 §2.2.3) and belongs to whichever header it follows. Header names
/// are case-insensitive (§1.2.2). Duplicate headers — several `Received`, or two
/// `Authentication-Results` from different hops — are all returned, in order.
fn filter_header_lines(block: &str, wanted: &[String]) -> String {
    let wanted: Vec<String> = wanted
        .iter()
        .map(|w| w.trim().trim_end_matches(':').to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let mut out: Vec<&str> = Vec::new();
    let mut keeping = false;
    for line in block.lines() {
        let is_continuation = line.starts_with(' ') || line.starts_with('\t');
        if is_continuation {
            if keeping {
                out.push(line);
            }
            continue;
        }
        keeping = match line.split_once(':') {
            Some((name, _)) => {
                let name = name.trim().to_ascii_lowercase();
                wanted.iter().any(|w| *w == name)
            }
            // No colon and not a continuation: not a header line (the
            // `From ` mbox separator, or a malformed block). Never kept.
            None => false,
        };
        if keeping {
            out.push(line);
        }
    }
    out.join("\n")
}

/// True when the agent already pasted a quoted-history block into the body —
/// matches both `class="cx-quote"` and the `data-cx-quote` marker (the former
/// is a substring of the latter, so one check covers both).
fn body_already_quoted(body: &str) -> bool {
    body.contains("cx-quote")
}

/// Format the quoted-history block for an MCP-drafted reply.
///
/// The HTML mirrors the frontend contract exactly: `buildQuotedEmailHtml`
/// (`src/lib/utils.ts`) wrapped in ComposeModal's `<blockquote
/// data-cx-quote="1" class="cx-quote">`. On draft reopen, TipTap's QuotedBlock
/// node re-claims the block via `blockquote.cx-quote` (ammonia strips the
/// `data-cx-quote` marker on the round-trip but keeps `class`), and the
/// reading pane's `splitTrailingQuote` collapses it behind the ••• toggle.
///
/// Returns `(html, plain)`. `plain` is a `> `-prefixed text rendition for the
/// plain-text mirror of the draft; empty when the original had no text part.
fn format_quoted_history(
    from_name: Option<&str>,
    from_email: Option<&str>,
    date_raw: &str,
    sanitized_html: Option<&str>,
    plain_text: Option<&str>,
) -> (String, String) {
    let author = from_name
        .filter(|s| !s.trim().is_empty())
        .or(from_email)
        .unwrap_or("Unknown sender")
        .trim();
    // Local-time display like the frontend's `new Date(date).toLocaleString()`;
    // fall back to the raw stored string when it doesn't parse as RFC 3339.
    let date_display = chrono::DateTime::parse_from_rfc3339(date_raw)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%-m/%-d/%Y, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| date_raw.to_string());

    let quoted_body = match sanitized_html.filter(|h| !h.trim().is_empty()) {
        Some(html) => html.to_string(),
        None => format!("<pre>{}</pre>", html_escape(plain_text.unwrap_or(""))),
    };
    let html = format!(
        r#"<blockquote data-cx-quote="1" class="cx-quote"><br/><br/><div style="border-left:2px solid rgb(74, 68, 57);padding-left:12px;margin-left:4px;color:rgb(160, 144, 120);"><p><strong>{}</strong> wrote on {}:</p>{}</div></blockquote>"#,
        html_escape(author),
        html_escape(&date_display),
        quoted_body
    );

    let plain = match plain_text.filter(|t| !t.trim().is_empty()) {
        Some(text) => {
            let prefixed = text
                .replace("\r\n", "\n")
                .lines()
                .map(|l| format!("> {}", l))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\nOn {}, {} wrote:\n{}", date_display, author, prefixed)
        }
        None => String::new(),
    };
    (html, plain)
}

/// Why a requested quote could not be built.
///
/// These used to collapse into one static string —
/// `" (quoted history unavailable — original body not cached)"` — which was
/// factually false for the failure that motivated this code: the body *was*
/// cached, the Message-ID lookup simply missed on spelling. Each variant now
/// carries the concrete detail, and the MCP logs to stderr (discarded by the
/// host at session end), so the tool result is the only durable diagnostic
/// channel. See gotcha #30.
#[derive(Debug, Clone, PartialEq, Eq)]
enum QuoteSkip {
    NotInDb { message_id: String },
    NoLocalRow { folder: String, uid: u32 },
    ImapConnect { error: String },
    ImapSelect { folder: String, error: String },
    ImapFetch { folder: String, uid: u32, error: String },
    NoRenderableBody { folder: String, uid: u32 },
}

impl QuoteSkip {
    /// Reason plus remedy. Always names either a concrete error or the
    /// `reply_to_folder` / `reply_to_uid` escape hatch, and always forbids the
    /// paste workaround — pasting history into `body` puts the user's
    /// signature *below* the quote, which is the complaint that started this.
    fn message(&self) -> String {
        const NO_PASTE: &str = "Do NOT paste the quote into `body` — that puts the signature below the quote.";
        match self {
            QuoteSkip::NotInDb { message_id } => format!(
                "QUOTED HISTORY UNAVAILABLE: no message with Message-ID {} in the local cache for this account. \
                 Retry with `reply_to_folder` + `reply_to_uid` from read_email/search_emails/read_thread. {}",
                message_id, NO_PASTE
            ),
            QuoteSkip::NoLocalRow { folder, uid } => format!(
                "QUOTED HISTORY UNAVAILABLE: no message at folder {:?} UID {} for this account, and {:?} is not a folder on this account. \
                 Call list_folders for exact folder names, then retry with corrected `reply_to_folder` + `reply_to_uid` from search_emails or read_email (UIDs are folder-scoped). {}",
                folder, uid, folder, NO_PASTE
            ),
            QuoteSkip::ImapConnect { error } => format!(
                "QUOTED HISTORY UNAVAILABLE: the original body is not cached locally and the IMAP connection needed to fetch it failed: {}. \
                 Retry once the account is reachable. {}",
                error, NO_PASTE
            ),
            QuoteSkip::ImapSelect { folder, error } => format!(
                "QUOTED HISTORY UNAVAILABLE: could not SELECT folder {:?} to fetch the original: {}. \
                 Check the folder name with list_folders, then retry with corrected `reply_to_folder` + `reply_to_uid`. {}",
                folder, error, NO_PASTE
            ),
            QuoteSkip::ImapFetch { folder, uid, error } => format!(
                "QUOTED HISTORY UNAVAILABLE: IMAP fetch of UID {} in {:?} failed: {}. \
                 The message may have been moved or expunged — re-run search_emails for current `reply_to_folder` + `reply_to_uid`. {}",
                uid, folder, error, NO_PASTE
            ),
            QuoteSkip::NoRenderableBody { folder, uid } => format!(
                "QUOTED HISTORY UNAVAILABLE: the original (UID {} in {:?}) has no renderable text or HTML body to quote. \
                 Pass quote_original=false to send this reply without quoted history. {}",
                uid, folder, NO_PASTE
            ),
        }
    }

    /// Bad-input skips are the caller's to fix; the rest are environmental.
    fn to_error(&self) -> McpError {
        match self {
            QuoteSkip::NotInDb { .. } | QuoteSkip::NoLocalRow { .. } => {
                McpError::invalid_params(self.message(), None)
            }
            _ => McpError::internal_error(self.message(), None),
        }
    }
}

/// Coordinates plus whatever attribution metadata the local row could supply.
/// `None` fields fall back to the IMAP-parsed message, so a coordinates reply
/// works even for a message the local DB never indexed — a capability the
/// Message-ID path structurally cannot have.
#[derive(Debug, Clone)]
struct QuoteFetch {
    folder: String,
    uid: u32,
    from_name: Option<String>,
    from_email: Option<String>,
    date: Option<String>,
}

/// Body parts recovered from an IMAP fetch, for the cache write-back.
#[derive(Debug, Clone)]
struct FetchedBody {
    plain_text: Option<String>,
    html_body: Option<String>,
    sanitized_html: Option<String>,
}

/// Result of the synchronous DB phase. Carries only owned data so nothing
/// borrows the `Connection` across an await — `&Connection` is not `Send`
/// and the rmcp tool futures must be.
#[derive(Debug, Clone)]
enum QuotePlan {
    Ready { html: String, plain: String },
    Fetch(QuoteFetch),
    Skip(QuoteSkip),
}

#[derive(Debug, Clone, Default)]
struct QuoteOutcome {
    quote: Option<(String, String)>,
    skip: Option<QuoteSkip>,
    /// `(folder, uid, body)` to persist so the next reply skips the round-trip.
    writeback: Option<(String, u32, FetchedBody)>,
    /// `(message_id, references)` off the fetched message, used to backfill
    /// threading when the local row had none.
    fetched_headers: Option<(Option<String>, Option<String>)>,
}

/// True when a body row has something worth quoting.
fn body_has_content(sanitized_html: Option<&str>, plain_text: Option<&str>) -> bool {
    sanitized_html.is_some_and(|h| !h.trim().is_empty())
        || plain_text.is_some_and(|t| !t.trim().is_empty())
}

fn folder_is_known(conn: &rusqlite::Connection, account_id: &str, folder: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE account_id = ?1 AND name = ?2)",
        rusqlite::params![account_id, folder],
        |r| r.get::<_, i64>(0),
    )
    // A DB hiccup must not turn into a bogus "that folder doesn't exist".
    .map(|n| n != 0)
    .unwrap_or(true)
}

/// Sync phase: resolve the target to a ready-made quote, an IMAP fetch, or a
/// concrete skip reason. Replaces `resolve_quote_source`.
fn plan_quote(
    conn: &rusqlite::Connection,
    account_id: &str,
    target: &ReplyTarget,
) -> QuotePlan {
    let (folder, uid, from_name, from_email, date) = match target {
        ReplyTarget::Coordinates { folder, uid } => {
            let row = db::messages::get_by_uid(conn, account_id, folder, *uid)
                .ok()
                .flatten();
            if row.is_none() && !folder_is_known(conn, account_id, folder) {
                return QuotePlan::Skip(QuoteSkip::NoLocalRow {
                    folder: folder.clone(),
                    uid: *uid,
                });
            }
            let (from_name, from_email, date) = match row {
                Some(m) => (m.from_name, m.from_email, Some(m.date)),
                None => (None, None, None),
            };
            (folder.clone(), *uid, from_name, from_email, date)
        }
        ReplyTarget::MessageId(mid) => match db::messages::get_reply_source(conn, account_id, mid) {
            Ok(Some(src)) => (
                src.folder_name,
                src.uid,
                src.from_name,
                src.from_email,
                Some(src.date),
            ),
            Ok(None) => {
                return QuotePlan::Skip(QuoteSkip::NotInDb {
                    message_id: mid.clone(),
                })
            }
            Err(e) => {
                log::warn!("MCP quoted history: reply-source lookup failed for {mid}: {e}");
                return QuotePlan::Skip(QuoteSkip::NotInDb {
                    message_id: mid.clone(),
                });
            }
        },
    };

    if let Ok(Some(body)) = db::messages::get_body(conn, account_id, &folder, uid) {
        if body_has_content(body.sanitized_html.as_deref(), body.plain_text.as_deref()) {
            let (html, plain) = format_quoted_history(
                from_name.as_deref(),
                from_email.as_deref(),
                date.as_deref().unwrap_or_default(),
                body.sanitized_html.as_deref(),
                body.plain_text.as_deref(),
            );
            return QuotePlan::Ready { html, plain };
        }
    }

    QuotePlan::Fetch(QuoteFetch {
        folder,
        uid,
        from_name,
        from_email,
        date,
    })
}

/// Async phase: fetch the original body once from IMAP (mirroring
/// `read_email`'s path) and format the quote. Replaces `fetch_quote_via_imap`.
///
/// Takes the account row + owned data, so no DB borrow is live across the
/// await (`&Account` is a plain struct ref, not a `&Connection`). Every early
/// return disconnects first — the old select-failure path leaked the session.
async fn run_quote_plan(account: &db::accounts::Account, plan: QuotePlan) -> QuoteOutcome {
    let fetch = match plan {
        QuotePlan::Ready { html, plain } => {
            return QuoteOutcome {
                quote: Some((html, plain)),
                ..Default::default()
            }
        }
        QuotePlan::Skip(skip) => {
            return QuoteOutcome {
                skip: Some(skip),
                ..Default::default()
            }
        }
        QuotePlan::Fetch(f) => f,
    };

    let skip = |s: QuoteSkip| QuoteOutcome {
        skip: Some(s),
        ..Default::default()
    };

    let account_email = account.email.as_str();
    let mut session = match imap::connect_for_account(account).await {
        Ok(s) => s,
        Err(e) => {
            // Was `.ok()?` — silently swallowed, which is why the original
            // forensics dead-ended with nothing in any log.
            log::warn!("MCP quoted history: IMAP connect failed for {account_email}: {e}");
            return skip(QuoteSkip::ImapConnect {
                error: e.to_string(),
            });
        }
    };

    // Was `let _ = …` — a failed SELECT then fetched a UID from whatever
    // folder happened to be selected, or leaked the session on the way out.
    if let Err(e) = imap::select_folder(&mut session, &fetch.folder).await {
        log::warn!(
            "MCP quoted history: SELECT {} failed: {e}",
            fetch.folder
        );
        let _ = imap::disconnect(session, &account.email).await;
        return skip(QuoteSkip::ImapSelect {
            folder: fetch.folder,
            error: e.to_string(),
        });
    }

    let raw_body = match imap::fetch_body(&mut session, fetch.uid).await {
        Ok(raw) => raw,
        Err(e) => {
            let _ = imap::disconnect(session, &account.email).await;
            log::warn!(
                "MCP quoted history: IMAP fetch failed for UID {} in {}: {e}",
                fetch.uid,
                fetch.folder
            );
            return skip(QuoteSkip::ImapFetch {
                folder: fetch.folder,
                uid: fetch.uid,
                error: e.to_string(),
            });
        }
    };
    let _ = imap::disconnect(session, &account.email).await;

    // Gotcha #9: html5ever/ammonia panic on certain hostile HTML. The old
    // code called parse_message bare here.
    let parsed = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parser::parse_message(&raw_body)
    })) {
        Ok(p) => p,
        Err(_) => {
            log::warn!(
                "MCP quoted history: parse_message panicked on UID {} in {}",
                fetch.uid,
                fetch.folder
            );
            return skip(QuoteSkip::NoRenderableBody {
                folder: fetch.folder,
                uid: fetch.uid,
            });
        }
    };

    if !body_has_content(parsed.sanitized_html.as_deref(), parsed.plain_text.as_deref()) {
        return skip(QuoteSkip::NoRenderableBody {
            folder: fetch.folder,
            uid: fetch.uid,
        });
    }

    let from_name = fetch.from_name.or_else(|| parsed.from_name.clone());
    let from_email = fetch
        .from_email
        .or_else(|| Some(parsed.from_email.clone()))
        .filter(|e| !e.trim().is_empty());
    let date = fetch
        .date
        .or_else(|| parsed.date.clone())
        .unwrap_or_default();

    let quote = format_quoted_history(
        from_name.as_deref(),
        from_email.as_deref(),
        &date,
        parsed.sanitized_html.as_deref(),
        parsed.plain_text.as_deref(),
    );

    let fetched_headers = Some((
        parsed
            .message_id
            .as_deref()
            .and_then(crate::email::message_id::normalize_message_id),
        Some(crate::email::message_id::normalize_reference_chain(
            &parsed.references.join(" "),
        ))
        .filter(|c| !c.is_empty()),
    ));

    QuoteOutcome {
        quote: Some(quote),
        skip: None,
        writeback: Some((
            fetch.folder,
            fetch.uid,
            FetchedBody {
                plain_text: parsed.plain_text,
                html_body: parsed.html_body,
                // parse_message already inlined cid: images (gotcha #14).
                sanitized_html: parsed.sanitized_html,
            },
        )),
        fetched_headers,
    }
}

/// Final phase: fail loudly on a skip, otherwise persist the fetched body and
/// return the quote plus the result-line note.
fn finish_quote(
    conn: &rusqlite::Connection,
    account_id: &str,
    outcome: QuoteOutcome,
) -> Result<(Option<(String, String)>, String), McpError> {
    if let Some(skip) = outcome.skip {
        return Err(skip.to_error());
    }

    if let Some((folder, uid, body)) = outcome.writeback {
        // Best-effort: `message_bodies` has a composite FK to `messages`, and
        // the coordinates path can legitimately quote a message with no local
        // parent row. A failed cache write must never fail the tool.
        match db::messages::upsert_body_preserving_metadata(
            conn,
            account_id,
            &folder,
            uid,
            body.plain_text.as_deref(),
            body.html_body.as_deref(),
            body.sanitized_html.as_deref(),
            true,
        ) {
            Ok(true) => log::info!("MCP quoted history: cached body for {folder}/{uid}"),
            Ok(false) => log::warn!(
                "MCP quoted history: refused to overwrite the cached body for {folder}/{uid} with a blank one"
            ),
            Err(e) => log::warn!("MCP quoted history: body write-back for {folder}/{uid} failed: {e}"),
        }
    }

    let note = if outcome.quote.is_some() {
        " + quoted original".to_string()
    } else {
        String::new()
    };
    Ok((outcome.quote, note))
}

/// Everything a reply needs beyond the body: threading headers, the quoted
/// history block, and the note appended to the tool result.
#[derive(Debug, Clone, Default)]
struct ReplyContext {
    in_reply_to: Option<String>,
    references: Option<String>,
    quote: Option<(String, String)>,
    note: String,
}

/// Validate inbox-group rule inputs against the allowed enums and convert to
/// the tuple shape expected by `db::inbox_groups::set_rules`.
fn validate_group_rules(
    rules: &[GroupRuleInput],
) -> Result<Vec<(String, String, String)>, McpError> {
    const FIELDS: &[&str] = &["from_email", "from_name", "subject", "to_list"];
    const OPERATORS: &[&str] = &["contains", "equals", "starts_with", "ends_with"];
    let mut out = Vec::with_capacity(rules.len());
    for r in rules {
        if !FIELDS.contains(&r.field.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid rule field {:?}. Allowed: from_email, from_name, subject, to_list",
                    r.field
                ),
                None,
            ));
        }
        if !OPERATORS.contains(&r.operator.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid rule operator {:?}. Allowed: contains, equals, starts_with, ends_with",
                    r.operator
                ),
                None,
            ));
        }
        out.push((r.field.clone(), r.operator.clone(), r.value.clone()));
    }
    Ok(out)
}

// ─── Mail-rule validation ─────────────────────────────────────────────
//
// The stored `MailRule` struct is wider than the engine that runs it, in two
// ways that would produce a silently-dead rule if passed straight through:
//
//   1. `move` / `delete` actions are accepted by the struct but deliberately
//      never executed — see the comment in
//      `commands/messages.rs::classify_headers`. They are IMAP side-effects
//      that don't belong on the hot sync path. The RulesManager UI sidesteps
//      this by only offering the three safe actions; we reject explicitly.
//   2. A `body` condition can never match: rules are evaluated against
//      headers, and `evaluate()` is handed an empty body on both the sync and
//      the reclassify path. (`to` used to share this problem; it is now
//      really passed, so `to` conditions do fire.)
//
// Both are rejected at the boundary rather than documented-and-accepted, so an
// agent cannot create a rule that looks saved and does nothing.

const RULE_FIELDS: &[&str] = &["from", "to", "subject"];
const RULE_OPERATORS: &[&str] = &["contains", "equals", "starts_with", "ends_with"];
const RULE_CATEGORIES: &[&str] = &["primary", "updates", "social", "promotions", "junk"];
const RULE_ACTIONS: &[&str] = &["set_category", "mark_read", "mark_flagged"];

fn validate_rule_conditions(
    conditions: &[MailRuleConditionInput],
) -> Result<Vec<db::rules::RuleCondition>, McpError> {
    // `conditions.iter().all(..)` is vacuously true on an empty list, so a
    // rule with no conditions matches EVERY message. Never allow one.
    if conditions.is_empty() {
        return Err(McpError::invalid_params(
            "At least one condition is required. A rule with no conditions would match every message and apply its actions to the whole mailbox.",
            None,
        ));
    }
    let mut out = Vec::with_capacity(conditions.len());
    for c in conditions {
        let field = c.field.trim().to_lowercase();
        if field == "body" {
            return Err(McpError::invalid_params(
                "Condition field \"body\" is not supported. Mail rules are evaluated at classification time, when only message headers are available — the rules engine is handed an empty body, so a body condition can never match. Match on \"subject\" or \"from\" instead.",
                None,
            ));
        }
        if !RULE_FIELDS.contains(&field.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid condition field {:?}. Allowed: from, to, subject.",
                    c.field
                ),
                None,
            ));
        }
        let operator = c.operator.trim().to_lowercase();
        if !RULE_OPERATORS.contains(&operator.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid condition operator {:?}. Allowed: contains, equals, starts_with, ends_with.",
                    c.operator
                ),
                None,
            ));
        }
        // An empty value makes `contains` / `starts_with` / `ends_with` match
        // everything — same whole-mailbox hazard as an empty condition list.
        if c.value.trim().is_empty() {
            return Err(McpError::invalid_params(
                format!(
                    "Condition on {:?} has an empty value. An empty value matches every message.",
                    field
                ),
                None,
            ));
        }
        out.push(db::rules::RuleCondition {
            field,
            operator,
            value: c.value.clone(),
        });
    }
    Ok(out)
}

fn validate_rule_actions(
    actions: &[MailRuleActionInput],
) -> Result<Vec<db::rules::RuleAction>, McpError> {
    if actions.is_empty() {
        return Err(McpError::invalid_params(
            "At least one action is required.",
            None,
        ));
    }
    let mut out = Vec::with_capacity(actions.len());
    for a in actions {
        let action_type = a.action_type.trim().to_lowercase();
        if action_type == "move" || action_type == "delete" {
            return Err(McpError::invalid_params(
                format!(
                    "Action {:?} is stored by the data model but is NEVER executed: the classifier deliberately skips move/delete because they are destructive IMAP side-effects that don't belong on the sync path. Creating such a rule would appear to succeed and do nothing. Use set_category (e.g. \"junk\" or \"updates\") to route the mail out of the inbox, or move the messages directly with move_email.",
                    a.action_type
                ),
                None,
            ));
        }
        if !RULE_ACTIONS.contains(&action_type.as_str()) {
            return Err(McpError::invalid_params(
                format!(
                    "Invalid action_type {:?}. Allowed: set_category, mark_read, mark_flagged.",
                    a.action_type
                ),
                None,
            ));
        }
        let value = if action_type == "set_category" {
            let v = a
                .value
                .as_deref()
                .map(|s| s.trim().to_lowercase())
                .unwrap_or_default();
            if !RULE_CATEGORIES.contains(&v.as_str()) {
                return Err(McpError::invalid_params(
                    format!(
                        "set_category requires value to be one of: primary, updates, social, promotions, junk (got {:?}).",
                        a.value
                    ),
                    None,
                ));
            }
            Some(v)
        } else {
            None
        };
        out.push(db::rules::RuleAction { action_type, value });
    }
    Ok(out)
}

/// Flag stored rules that predate — or bypass — the validation above, so
/// `list_mail_rules` tells the caller *why* a rule isn't doing anything
/// instead of just showing it as active.
fn mail_rule_warnings(rule: &db::rules::MailRule) -> Vec<String> {
    let mut warnings = Vec::new();
    if rule.conditions.is_empty() {
        warnings.push(
            "Has no conditions — matches EVERY message and applies its actions to all mail."
                .to_string(),
        );
    }
    for c in &rule.conditions {
        if c.field.eq_ignore_ascii_case("body") {
            warnings.push(format!(
                "Condition on \"body\" ({} {:?}) can never match: rules are evaluated against headers only.",
                c.operator, c.value
            ));
        } else if !RULE_FIELDS.contains(&c.field.to_lowercase().as_str()) {
            warnings.push(format!(
                "Unknown condition field {:?} never matches.",
                c.field
            ));
        }
        if !RULE_OPERATORS.contains(&c.operator.to_lowercase().as_str()) {
            warnings.push(format!(
                "Unknown condition operator {:?} never matches.",
                c.operator
            ));
        }
    }
    for a in &rule.actions {
        let at = a.action_type.to_lowercase();
        if at == "move" || at == "delete" {
            warnings.push(format!(
                "Action {:?} is never executed by the classifier — this part of the rule is dead.",
                a.action_type
            ));
        } else if !RULE_ACTIONS.contains(&at.as_str()) {
            warnings.push(format!("Unknown action_type {:?} is ignored.", a.action_type));
        } else if at == "set_category"
            && !a
                .value
                .as_deref()
                .map(|v| RULE_CATEGORIES.contains(&v.to_lowercase().as_str()))
                .unwrap_or(false)
        {
            warnings.push(format!(
                "set_category value {:?} is not a known category; the message would be filed under a category the UI does not show.",
                a.value
            ));
        }
    }
    warnings
}

/// Every reason a stored rule must NOT be applied in bulk over an existing
/// mailbox. A strict superset of `mail_rule_warnings` — it adds the blank
/// condition `value` check, because `"".contains("")` is true, so a blank
/// value makes a condition match every message (gotcha #36 trap 3).
///
/// `list_mail_rules` only *annotates* legacy rows; `apply_mail_rule` has to
/// refuse them. A rule created through `create_mail_rule` is already
/// validated at that boundary, but the UI and pre-T17 rows are not, and an
/// apply tool must not become a way around the guards.
fn rule_apply_blockers(rule: &db::rules::MailRule) -> Vec<String> {
    let mut blockers = mail_rule_warnings(rule);
    for c in &rule.conditions {
        if c.value.trim().is_empty() {
            blockers.push(format!(
                "Condition {} {:?} has a blank value, which matches EVERY message.",
                c.field, c.operator
            ));
        }
    }
    blockers
}

/// One already-synced inbox message a rule matched, with the actions the
/// real engine returned for it.
struct RuleMatch<'a> {
    row: &'a db::messages::ReclassifyRow,
    actions: Vec<db::rules::RuleAction>,
}

/// Scan already-synced inbox messages against one rule. Returns
/// `(messages_scanned, matches)`.
///
/// This is the ONE matcher behind both `preview_mail_rule` and
/// `apply_mail_rule`, so a dry run can never disagree with what the apply
/// then does. It resolves through `db::rules::evaluate` — the same function
/// the sync path and the reclassify backfill call — so it also cannot drift
/// from production classification semantics. Do not write a second matcher.
///
/// `body` is passed as `""` exactly as both production call sites do: rules
/// run against headers only, which is why `body` conditions are rejected at
/// the boundary rather than silently never matching (gotcha #36 trap 2).
fn scan_rule_matches<'a>(
    rows: &'a [db::messages::ReclassifyRow],
    rule: &db::rules::MailRule,
    scope_account: Option<&str>,
) -> (usize, Vec<RuleMatch<'a>>) {
    let probe = std::slice::from_ref(rule);
    let mut scanned = 0usize;
    let mut matches = Vec::new();
    for row in rows {
        if let Some(a) = scope_account {
            if row.account_id != a {
                continue;
            }
        }
        scanned += 1;
        let actions = db::rules::evaluate(
            probe,
            row.from_email.as_deref().unwrap_or(""),
            &row.to_list,
            row.subject.as_deref().unwrap_or(""),
            "",
            &row.account_id,
        );
        if !actions.is_empty() {
            matches.push(RuleMatch { row, actions });
        }
    }
    (scanned, matches)
}

// One-or-more blank lines (CRLF or LF) separating paragraphs. Tolerates
// whitespace on the otherwise-blank line.
static PARA_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?:\r?\n)[ \t]*(?:\r?\n)+").unwrap());

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn sanitize_html_lossy(html: &str) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser::sanitize_html(html))) {
        Ok(clean) => clean,
        Err(_) => {
            log::error!("MCP body_to_html: sanitize_html panicked, falling back to escaped HTML");
            format!(
                "<pre style=\"white-space:pre-wrap\">{}</pre>",
                html_escape(html)
            )
        }
    }
}

/// Parse a `serde_json::to_string(&Vec<EmailAddress>)` blob back into addresses,
/// tolerating NULL / empty / corrupt JSON (older cached rows) by returning empty.
fn parse_addr_json(json: Option<&str>) -> Vec<parser::EmailAddress> {
    json.filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Vec<parser::EmailAddress>>(s).ok())
        .unwrap_or_default()
}

/// Render `To:` / `Cc:` / `Bcc:` header lines (each terminated by `\n`, empty
/// lists skipped) so an MCP agent reading an email sees the FULL recipient set
/// before it drafts a reply. Omitting these silently drops recipients when the
/// agent rebuilds a reply-all — the "Sam" incident, where a second To
/// recipient invisible to the reader was left off the reply. Each address
/// renders as `Name <email>` when a display name is present, else the bare
/// address.
fn fmt_recipient_lines(
    to: &[parser::EmailAddress],
    cc: &[parser::EmailAddress],
    bcc: &[parser::EmailAddress],
) -> String {
    fn one(addr: &parser::EmailAddress) -> String {
        match &addr.name {
            Some(n) if !n.trim().is_empty() => format!("{} <{}>", n.trim(), addr.email),
            _ => addr.email.clone(),
        }
    }
    fn line(label: &str, addrs: &[parser::EmailAddress]) -> String {
        if addrs.is_empty() {
            String::new()
        } else {
            format!(
                "{}: {}\n",
                label,
                addrs.iter().map(one).collect::<Vec<_>>().join(", ")
            )
        }
    }
    format!("{}{}{}", line("To", to), line("Cc", cc), line("Bcc", bcc))
}

/// Compact single-line recipient join (`a@x, b@y`) for the thread-list view.
fn fmt_addr_json_compact(json: Option<&str>) -> String {
    parse_addr_json(json)
        .iter()
        .map(|a| a.email.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Detect whether body is HTML or plain text and convert accordingly.
///
/// Plain text: blank-line-separated chunks become separate `<p>` blocks; intra-chunk
/// newlines become `<br/>`; all literal characters HTML-escaped before interpolation
/// (TipTap's prose render and most mail clients depend on real `<p>+<p>` boundaries
/// for paragraph spacing — `<br/>`s never produce inter-paragraph margin).
///
/// HTML: passed through `parser::sanitize_html` to enforce the project HTML-safety
/// invariant before the body lands in the Drafts folder.
fn body_to_html(body: &str) -> String {
    let looks_like_html = body.contains("</") || body.contains("/>");
    if looks_like_html {
        return sanitize_html_lossy(body);
    }
    plain_text_to_html(body)
}

/// Build paragraph HTML from plain text: blank-line-separated chunks become `<p>`
/// blocks, intra-chunk newlines become `<br/>`, and all characters are HTML-escaped.
/// Never interprets tags as markup — use for bodies the caller declared plain text.
fn plain_text_to_html(body: &str) -> String {
    let normalized = body.replace("\r\n", "\n");
    let trimmed = normalized.trim_matches('\n');
    if trimmed.is_empty() {
        return String::new();
    }

    PARA_RE
        .split(trimmed)
        .filter(|p| !p.trim().is_empty())
        .map(|chunk| {
            let escaped = html_escape(chunk);
            format!("<p>{}</p>", escaped.replace('\n', "<br/>"))
        })
        .collect::<String>()
}

/// Render an email body honoring an explicit HTML hint from the caller.
///
/// - `Some(true)`  — caller declared HTML; sanitize and pass through as markup.
/// - `Some(false)` — caller declared plain text; build escaped paragraphs (tags shown literally).
/// - `None`        — auto-detect via [`body_to_html`] (legacy heuristic; can misfire on `<br>` or `</` in text).
fn render_body(body: &str, is_html: Option<bool>) -> String {
    match is_html {
        Some(true) => sanitize_html_lossy(body),
        Some(false) => plain_text_to_html(body),
        None => body_to_html(body),
    }
}

/// Wrap LLM-supplied inner HTML in a subtle "card" layout: a centered white card
/// on a light-grey page background, with generous padding, rounded corners, and a
/// soft shadow. All styling is INLINE — email clients ignore `<style>` blocks and
/// CSS classes, so inline `style="..."` is the only thing that renders for the
/// recipient.
///
/// Two style-value constraints are load-bearing — do NOT "tidy" these:
///  1. The card element keeps `background-color` in its own `style`. CXMail's
///     dark-mode reading pane (`EmailFrame.tsx`) forces a transparent background
///     and light text on every block element whose style lacks the *substring*
///     `background`; `background-color:#ffffff` contains that substring, so the
///     white card survives dark mode instead of inverting.
///  2. No declaration here may *exactly* equal one that `apply_inline_font_styles`
///     injects or strips (`email::inline_styles::OUR_DECL_RE`) — that pass runs on
///     every draft after this one and would silently drop a colliding declaration.
///     `padding:32px`, `border-radius:8px`, and `background-color:#ffffff` are all
///     safe; `padding:8px`, `border-radius:4px`, and `background:#f5f5f5` are NOT.
fn wrap_card(inner: &str) -> String {
    format!(
        r#"<div style="background-color:#f4f5f7;padding:40px 24px;"><div style="max-width:600px;margin:0 auto;background-color:#ffffff;border-radius:8px;box-shadow:0 2px 8px rgba(0,0,0,0.08);padding:32px;line-height:1.6;">{}</div></div>"#,
        inner
    )
}

/// Wrap arbitrary HTML in the round-trip-preservation block. The `class` is what
/// actually survives the chain: ammonia drops `data-cx-html` (not in its attribute
/// allowlist) but keeps `class` (a generic attribute), and on reopen TipTap's
/// `div.cx-html-block` parse rule re-claims the block as an atomic node so it is
/// emitted verbatim by `editor.getHTML()` instead of being flattened by StarterKit.
/// The `data-cx-html` marker is kept for parity with the other atomic blocks even
/// though the subsequent sanitize removes it.
fn wrap_preserve(html: &str) -> String {
    format!(r#"<div class="cx-html-block" data-cx-html="1">{}</div>"#, html)
}

/// Render a styled-HTML body for the `layout` MCP parameter, marked for round-trip
/// survival through CXMail's TipTap compose editor.
///
/// - `"card"` — `body` is the card's inner content (parsed per `is_html`), wrapped
///   in the built-in card chrome, then in the preservation block.
/// - any other value (callers pass only `"rich"`) — `body` is treated as a
///   complete, already-styled HTML fragment and wrapped in the preservation block
///   as-is, with no card chrome added.
///
/// Both paths pass the author's own HTML through
/// [`loose_text::normalize_cell_text`] first, which gives every bare text node
/// inside a `<td>`/`<th>` an element to be styled by. It runs on the author's
/// fragment (and on the card's inner content) rather than on the assembled
/// output because the chrome this function adds has no table cells to fix, and
/// running it before `wrap_preserve` keeps the "sanitize exactly once, at the
/// end" invariant below intact.
///
/// The whole assembled structure is sanitized exactly once at the end — a single
/// `sanitize_html_lossy` pass over card chrome + inner — which enforces the
/// HTML-safety invariant and strips the `data-cx-html` marker while keeping
/// `class="cx-html-block"` and the inner inline styles. The caller still runs
/// `apply_inline_font_styles` afterwards (unchanged); it preserves `class` and the
/// card's inline declarations, so the load-bearing `class="cx-html-block"` and the
/// card's `background-color` both survive to the recipient.
fn render_styled(body: &str, is_html: Option<bool>, layout: &str) -> String {
    let wrapped = if layout == "card" {
        // Inner content is NOT sanitized here — the single pass below covers it.
        let inner = match is_html {
            Some(false) => plain_text_to_html(body),
            Some(true) => body.to_string(),
            None => {
                if body.contains("</") || body.contains("/>") {
                    body.to_string()
                } else {
                    plain_text_to_html(body)
                }
            }
        };
        wrap_preserve(&wrap_card(&loose_text::normalize_cell_text(&inner).html))
    } else {
        // "rich": body is already a complete styled HTML fragment.
        wrap_preserve(&loose_text::normalize_cell_text(body).html)
    };
    sanitize_html_lossy(&wrapped)
}

/// Reject a `layout` value that is neither `"card"` nor `"rich"`. `None` is
/// valid — it selects the ordinary editable-draft path, which is the right
/// choice for most mail.
///
/// `layout` arrives as a free-form `Option<String>` (schemars emits no enum for
/// it), so before this guard a typo like `"Card"` or an invented `"minimal"`
/// fell through the match arms in `compose_draft` / `edit_draft` onto the plain
/// path. The caller's designed HTML was then flattened when the user opened the
/// draft, and the tool reported plain success — the failure was completely
/// silent.
///
/// CALL SITE IS LOAD-BEARING: both handlers call this at the very top, before
/// any IMAP work. A rejection must land before the APPEND — since 2026-08-25
/// `edit_draft` appends the new revision before retiring the old one (gotcha
/// #57), so a failure past the APPEND no longer destroys the draft, but it does
/// leave a duplicate behind. Validating at the match site would put it there.
/// The conflict `edit_draft` returns when the UID it was handed is no longer in
/// the Drafts folder — returned strictly before the APPEND, so nothing is
/// written.
///
/// Why this exists: `UID STORE` on a nonexistent UID is silently ignored (RFC
/// 9051 §6.4.9), so a stale UID used to pass the delete, and the APPEND then
/// minted a *second* draft while the result read "UID N replaced with UID M".
/// The composer's live-reload listener matches on `old_uid == its current
/// uid`, so it was never told either (gotcha #57). The common way a UID goes
/// stale is the compose window's own autosave — every save mints a new UID.
///
/// The local-cache flag is advisory: the app's `edit_draft` evicts the old
/// row when it re-saves, so a missing row is the signature of that path, and
/// the message says so. The server is the authority on whether the draft is
/// gone. This is a preflight, not exclusion — a re-save can still land between
/// the check and the APPEND; the v60 drafts table (stable `draft_id`, CAS on
/// `current_uid`) is what closes that window.
fn stale_draft_error(uid: u32, folder: &str, email: &str, local_row_present: bool) -> McpError {
    let local = if local_row_present {
        "The local cache still lists it, so the server-side copy went away since the last sync."
    } else {
        "Its local cache row is gone too, which is what a re-save from the compose window does."
    };
    McpError::invalid_params(
        format!(
            "Draft UID {uid} is not in {folder} for {email} — nothing was changed. Every save mints a new UID (the compose window's autosave included), so this reference is stale, or the draft was deleted. {local} Find the current draft with search_emails (folder \"{folder}\") and retry edit_draft with its UID."
        ),
        None,
    )
}

/// How `edit_draft` was addressed — exactly one of `draft_id` / `uid`.
#[derive(Debug, PartialEq, Eq)]
enum EditTarget {
    ById {
        draft_id: String,
        expected_uid: Option<u32>,
    },
    ByUid(u32),
}

/// Who authors the draft's prose: the caller (`body`) or the external writer
/// (`instruction`, run through Antigravity's `agy` on the user's subscription).
#[derive(Debug)]
enum AuthoredSource {
    Explicit(String),
    Generated { instruction: String, model: String },
}

/// Resolve `body` XOR `instruction` before ANY other work — a caller error
/// must fail before it reaches IMAP (#30's ordering rule), and every arm of
/// this refusal says "nothing was written" and names the remedy (#57's
/// error-message convention).
///
/// `layout` and `is_html=true` are refused WITH `instruction` rather than
/// ignored: the writer emits plain prose, and silently dropping a layout the
/// caller asked for is how a designed email ships flattened (#34's lesson,
/// from the opposite direction).
/// `default_model` is what a call omitting `writer_model` gets — the caller
/// passes `external_writer::effective_default_model()`, i.e. the app-settings
/// value with the built-in as fallback, so the precedence is
/// per-call > configured > built-in. Kept as a parameter so this stays pure.
fn resolve_authored_source(
    body: Option<String>,
    instruction: Option<String>,
    writer_model: Option<String>,
    layout: Option<&str>,
    is_html: Option<bool>,
    default_model: &str,
) -> Result<AuthoredSource, McpError> {
    match (body, instruction) {
        (Some(_), Some(_)) => Err(McpError::invalid_params(
            "Pass exactly one of `body` or `instruction`, not both — nothing was written. \
             Use `body` for text you authored; use `instruction` to have the external \
             writer author it."
                .to_string(),
            None,
        )),
        (None, None) => Err(McpError::invalid_params(
            "Pass `body` (text you authored) or `instruction` (what the external writer \
             should say) — nothing was written."
                .to_string(),
            None,
        )),
        (Some(body), None) => {
            if writer_model.is_some() {
                return Err(McpError::invalid_params(
                    "`writer_model` only applies together with `instruction` — nothing was \
                     written. Drop it, or replace `body` with an `instruction`."
                        .to_string(),
                    None,
                ));
            }
            Ok(AuthoredSource::Explicit(body))
        }
        (None, Some(instruction)) => {
            if instruction.trim().is_empty() {
                return Err(McpError::invalid_params(
                    "`instruction` is empty — nothing was written.".to_string(),
                    None,
                ));
            }
            if layout.is_some() {
                return Err(McpError::invalid_params(
                    "`instruction` cannot be combined with `layout` — the external writer \
                     emits plain prose, not designed HTML. Nothing was written. For a \
                     designed email, author the HTML yourself and pass it as `body` with \
                     `layout`."
                        .to_string(),
                    None,
                ));
            }
            if is_html == Some(true) {
                return Err(McpError::invalid_params(
                    "`instruction` cannot be combined with is_html=true — the generated \
                     body is plain prose. Nothing was written; drop is_html."
                        .to_string(),
                    None,
                ));
            }
            let model = writer_model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| default_model.to_string());
            // Refuse a flag-shaped or junk id here, before any work — for a
            // per-call value AND for a corrupt configured default (the store
            // validates on save, but the store is writable by other tools).
            crate::email::external_writer::validate_model_id(&model)
                .map_err(|e| McpError::invalid_params(format!("{e} — nothing was written."), None))?;
            Ok(AuthoredSource::Generated { instruction, model })
        }
    }
}

/// Validate the three addressing parameters before anything else runs. Both
/// mistakes here fail loudly on purpose: `draft_id` + `uid` together is
/// ambiguous about which revision the caller believes it holds, and
/// `expected_uid` with a bare `uid` is a contradiction (the uid IS the
/// revision).
fn resolve_edit_target(
    draft_id: Option<&str>,
    expected_uid: Option<u32>,
    uid: Option<u32>,
) -> Result<EditTarget, McpError> {
    let draft_id = draft_id.map(str::trim).filter(|s| !s.is_empty());
    match (draft_id, uid) {
        (Some(id), None) => Ok(EditTarget::ById {
            draft_id: id.to_string(),
            expected_uid,
        }),
        (None, Some(uid)) => {
            if expected_uid.is_some() {
                return Err(McpError::invalid_params(
                    "`expected_uid` only applies with `draft_id` — with `uid`, the uid itself is the revision being replaced. Nothing was changed.",
                    None,
                ));
            }
            Ok(EditTarget::ByUid(uid))
        }
        (Some(_), Some(_)) => Err(McpError::invalid_params(
            "Pass either `draft_id` (preferred) or `uid`, not both. Nothing was changed.",
            None,
        )),
        (None, None) => Err(McpError::invalid_params(
            "edit_draft needs the draft to edit: pass `draft_id` (as returned by compose_draft / edit_draft) or, for a draft that has none, its `uid`. Nothing was changed.",
            None,
        )),
    }
}

fn unknown_draft_error(draft_id: &str) -> McpError {
    McpError::invalid_params(
        format!(
            "No draft with draft_id \"{draft_id}\" — nothing was changed. Either it was never saved by this CXMail (drafts saved before v60, or by another client, carry no identity), it was edited in another client (which drops the identity), or the id is wrong. Find the draft with search_emails and address it by `uid` once; that reply carries the draft_id to use afterwards."
        ),
        None,
    )
}

/// The conflict: the caller named a revision that is no longer current.
/// Carries what the current revision looks like, from the local cache when it
/// has been synced, so the caller can decide without another round trip.
fn draft_moved_error(
    conn: &rusqlite::Connection,
    row: &db::drafts::DraftRow,
    expected_uid: u32,
    current_uid: u32,
) -> McpError {
    let snap = db::drafts::snapshot(conn, &row.account_id, &row.folder_name, current_uid)
        .ok()
        .flatten();
    let described = match snap {
        Some(s) => format!(
            "subject {:?}, to {}",
            s.subject.unwrap_or_default(),
            s.to_list.unwrap_or_else(|| "(unknown)".to_string())
        ),
        None => "(not in the local cache yet — read_email fetches it)".to_string(),
    };
    McpError::invalid_params(
        format!(
            "Draft {} has moved on: you expected UID {expected_uid} but its current revision is UID {current_uid} in {} — it was re-saved (the compose window's autosave does this). Nothing was changed. Current revision: {described}. Read it with read_email (folder \"{}\", uid {current_uid}) and retry edit_draft with expected_uid={current_uid}, or omit expected_uid to replace the current revision regardless.",
            row.draft_id, row.folder_name, row.folder_name
        ),
        None,
    )
}

fn draft_busy_error(draft_id: &str, expires_at: &str) -> McpError {
    McpError::invalid_params(
        format!(
            "Draft {draft_id} is being saved by another writer right now (claim held until {expires_at}) — nothing was changed. Retry in a few seconds; if it is still busy after the claim expires, the other writer died mid-save and the retry will take over."
        ),
        None,
    )
}

/// Refuse a reply-shaped subject that carries no reply target.
///
/// A draft with a `Re:` subject and no parent produces a message with **no**
/// `In-Reply-To` and no `References` — a reply that starts a brand new thread.
/// This is not hypothetical: 16 of 17 reply-shaped messages CXMail sent between
/// 2026-05-07 and 2026-07-31 went out this way, including client correspondence.
///
/// It stayed invisible for three months because **Gmail hides it** — Gmail falls
/// back to subject+participant grouping when `References` is absent, so the
/// thread looks intact in the sender's and the recipient's Gmail while CXMail's
/// own thread view (which has no such fallback) fragments. Nothing in the send
/// path could detect it either, since an unthreaded message is perfectly valid
/// RFC 5322. The only place the mistake is still knowable is here, at the call
/// that omitted the parent.
///
/// Scoped to `Re:` deliberately. A `Fwd:` to a third party is legitimately a new
/// thread, and rejecting it would block ordinary forwards to buy nothing.
fn validate_reply_threading(subject: &str, target: &Option<ReplyTarget>) -> Result<(), McpError> {
    if target.is_some() {
        return Ok(());
    }
    let trimmed = subject.trim_start();
    let is_reply_shaped = trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("re:");
    if !is_reply_shaped {
        return Ok(());
    }
    Err(McpError::invalid_params(
        format!(
            "Subject {:?} is a reply, but no reply target was given, so the draft would be \
             sent with no In-Reply-To/References and would start a new thread. Pass \
             reply_to_folder + reply_to_uid (read_email / search_emails / read_thread print \
             Folder and UID together), or reply_to_message_id. If this genuinely is a new \
             conversation that merely begins with \"Re:\", change the subject.",
            subject.trim()
        ),
        None,
    ))
}

/// The instants behind a pair of event bounds, or `None` for an all-day bound
/// (which carries a date and no instant).
///
/// Extracted so `end > start` and the Zoom duration are computed from one parse
/// rather than two, and so an unparseable bound is an `invalid_params` naming the
/// field instead of a panic further down.
#[allow(clippy::type_complexity)]
fn timed_bounds(
    start: &crate::email::gcal::EventDateTime,
    end: &crate::email::gcal::EventDateTime,
) -> Result<
    (
        Option<chrono::DateTime<chrono::FixedOffset>>,
        Option<chrono::DateTime<chrono::FixedOffset>>,
    ),
    McpError,
> {
    fn one(
        value: &crate::email::gcal::EventDateTime,
        field: &str,
    ) -> Result<Option<chrono::DateTime<chrono::FixedOffset>>, McpError> {
        match value.date_time.as_deref() {
            None => Ok(None),
            Some(raw) => chrono::DateTime::parse_from_rfc3339(raw).map(Some).map_err(|e| {
                McpError::invalid_params(format!("Could not read {field} as a date/time: {e}"), None)
            }),
        }
    }
    Ok((one(start, "start")?, one(end, "end")?))
}

/// Resolve the conferencing choice at the MCP boundary.
///
/// Same posture as `validate_layout` and `validate_reply_threading`: reject what
/// cannot work, and say what would otherwise have happened silently. Every
/// failure here is invisible without it — a Zoom-less event that the caller
/// reports as having a Zoom link reads to the user as "the link is missing", with
/// nothing pointing at the call that omitted it.
///
/// `is_all_day` and `is_recurring` are passed in rather than inferred so the
/// refusals are testable and so a future recurrence parameter cannot bypass them.
/// `create_calendar_event` has no recurrence parameter today, so it passes
/// `false` — the check exists for the moment one is added.
fn validate_conference(
    conference: Option<&str>,
    add_meet: Option<bool>,
    is_all_day: bool,
    is_recurring: bool,
) -> Result<Conferencing, McpError> {
    let choice = match crate::email::gcal::parse_conferencing(conference, add_meet) {
        Ok(choice) => choice,
        Err(error) => {
            // Rewrite the terse AppError into the boundary's three-part shape:
            // what is wrong → what would silently happen → the exact fix.
            let raw = conference.unwrap_or("").trim().to_ascii_lowercase();
            let message = if Conferencing::VALID.contains(&raw.as_str()) {
                format!(
                    "{error} One of them has to go, because picking a winner silently would give \
                     the user an event with the wrong conferencing — or none at all — while this \
                     call reports success. `add_meet` is deprecated: drop it and pass only \
                     conference={raw:?}."
                )
            } else {
                format!(
                    "{error} An unrecognized value would otherwise fall through to the default and \
                     create a Google Meet event while you report something else. Pass \
                     conference=\"zoom\" for a Zoom meeting, \"meet\" for Google Meet, or \"none\" \
                     for no conferencing at all."
                )
            };
            return Err(McpError::invalid_params(message, None));
        }
    };

    if choice == Conferencing::Zoom && is_all_day {
        return Err(McpError::invalid_params(
            "conference=\"zoom\" cannot be used on an all-day event. A Zoom meeting needs a start \
             instant and a duration, so there is nothing to schedule — the event would be created \
             with no join link while this call reports one. Give `start` and `end` a time (for \
             example 2026-08-12T15:00), or pass conference=\"none\".",
            None,
        ));
    }
    if choice == Conferencing::Zoom && is_recurring {
        return Err(McpError::invalid_params(
            "conference=\"zoom\" cannot be used on a recurring event yet — only the first \
             occurrence would get a meeting, and every later one would show a link to a call that \
             has already happened. Create a single event with conference=\"zoom\", or use \
             conference=\"meet\" for the series.",
            None,
        ));
    }
    Ok(choice)
}

fn validate_layout(layout: Option<&str>) -> Result<(), McpError> {
    match layout {
        None | Some("card") | Some("rich") => Ok(()),
        Some(other) => Err(McpError::invalid_params(
            format!(
                "Unknown layout {:?}. The only accepted values are \"rich\" (preserve a \
                 hand-authored, inline-styled HTML fragment verbatim) and \"card\" (wrap the body \
                 in built-in card chrome). Omit `layout` entirely for an ordinary editable draft \
                 — that is the right choice for plain prose and simple markup such as <strong>, \
                 <ul> and links, which survive the compose editor untouched.",
                other
            ),
            None,
        )),
    }
}

/// A ⚠ note appended to an otherwise-successful compose/edit result when the
/// `layout` choice looks wrong for the body — in *either* direction. Returns an
/// empty string when the pairing is fine, so callers append unconditionally.
///
/// Both mismatches are silent without this, and both were made the same night
/// (2026-07-30):
///  - designed markup with `layout` omitted → the design is stripped when the
///    user opens the draft in CXMail's compose window (TipTap's StarterKit has
///    no `div` or `table` node), while the tool reports plain success;
///  - `"rich"` on trivial markup → the body is sealed into one preserved block,
///    so the user can retype words but cannot add a paragraph or a bullet
///    (gotcha #33).
///
/// Deliberately silent on *simple* markup (`<strong>`, `<ul>`, a bare `<div>`)
/// with `layout` omitted: StarterKit round-trips lists, bold, links and headings
/// untouched, so warning on every HTML draft would be noise — and noise is what
/// trains a caller to skip the note that matters. Also silent for `"card"` in
/// both directions: there the chrome *is* the design, so choosing it is always
/// deliberate.
/// The trailing line an unscoped `search_emails` appends when the DB rule
/// skipped one or more accounts hidden from aggregated views. `None` when
/// nothing was skipped, so the common case adds no noise. A free function so
/// it is unit-testable without a tool harness.
fn hidden_accounts_note(hidden: &[db::accounts::Account]) -> Option<String> {
    if hidden.is_empty() {
        return None;
    }
    let names = hidden
        .iter()
        .map(|a| format!("{} [ID: {}]", a.email, a.id))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "(Skipped {} account{} hidden from aggregated views: {}. Pass account_ids to search {}.)",
        hidden.len(),
        if hidden.len() == 1 { "" } else { "s" },
        names,
        if hidden.len() == 1 { "it" } else { "them" },
    ))
}

fn layout_advisory(layout: Option<&str>, body: &str) -> &'static str {
    let lower = body.to_lowercase();
    let designed = lower.contains("style=") || lower.contains("<table");
    match layout {
        None if designed => {
            " ⚠ body contains designed HTML but `layout` was omitted — tables and inline styles \
             will be stripped when the user opens this draft in compose. Re-issue with \
             layout=\"rich\"."
        }
        Some("rich") if !designed && !lower.contains("<div") => {
            " ⚠ body is plain/simple markup but layout=\"rich\" was set — this locks the draft's \
             structure in compose (text edits only, no restructuring). Omit `layout` unless the \
             email is genuinely designed."
        }
        // Bare text beside a block inside one table cell. `render_styled` has
        // already wrapped it (`email::loose_text`), so the draft is fine — but
        // the author wrote a styled element for one line and loose text for the
        // next, and only the element carries their `font-size`/`color`/`margin`.
        // The wrapped run inherits the cell's typography instead. Silent for a
        // cell that is nothing but text: there is no competing style there.
        Some("rich") | Some("card") if loose_text::has_mixed_cell_content(body) => {
            " ⚠ a table cell mixed a styled element with bare text — the bare run was wrapped in \
             <p style=\"margin:0\"> so it can't pick up default paragraph margins, and it inherits \
             the cell's typography. Author the <p> yourself if that line needs its own size, \
             colour or spacing."
        }
        _ => "",
    }
}

/// Validate a pinned voice rule at the boundary and return it normalized.
///
/// Same posture as `validate_rule_conditions` (gotcha #36): reject what cannot
/// work rather than describing the trap in a tool description an agent may
/// skip. Every failure here is silent otherwise — a rule pinned to a mistyped
/// address, or to no address at all, simply never fires, and "the rule didn't
/// apply" is indistinguishable from "the agent ignored it".
fn validate_pinned_rule(
    scope: Option<&str>,
    recipient_email: Option<&str>,
    rule: &str,
) -> Result<(db::voice_pinned_rules::Scope, String, String), McpError> {
    use db::voice_pinned_rules::{Scope, MAX_RULE_CHARS};

    let recipient = recipient_email
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());

    let scope = match scope.map(|s| s.trim().to_lowercase()).as_deref() {
        None | Some("") => {
            if recipient.is_some() {
                Scope::Recipient
            } else {
                Scope::Account
            }
        }
        Some("recipient") => Scope::Recipient,
        Some("account") => Scope::Account,
        Some(other) => {
            return Err(McpError::invalid_params(
                format!("Unknown scope {other:?}. Valid values: \"recipient\", \"account\"."),
                None,
            ))
        }
    };

    // A scope that disagrees with the address is always a mistake, and the two
    // mistakes fail in opposite directions: an account rule with an address
    // silently applies to *everyone*, a recipient rule without one applies to
    // no one.
    let recipient = match (scope, recipient) {
        (Scope::Recipient, Some(r)) => r,
        (Scope::Recipient, None) => {
            return Err(McpError::invalid_params(
                "scope=\"recipient\" needs recipient_email. Omit scope (or pass \"account\") for a rule that applies to every message.",
                None,
            ))
        }
        (Scope::Account, None) => String::new(),
        (Scope::Account, Some(r)) => {
            return Err(McpError::invalid_params(
                format!(
                    "scope=\"account\" applies to every message, but recipient_email={r:?} was also given. Drop recipient_email for an account-wide rule, or pass scope=\"recipient\" to pin it to that address only."
                ),
                None,
            ))
        }
    };

    if scope == Scope::Recipient && (!recipient.contains('@') || recipient.contains(char::is_whitespace))
    {
        return Err(McpError::invalid_params(
            format!(
                "recipient_email={recipient:?} is not an email address. A rule pinned to a name or a typo never fires and reports no error."
            ),
            None,
        ));
    }

    let rule = rule.trim();
    if rule.is_empty() {
        return Err(McpError::invalid_params(
            "rule is empty. Write the instruction the drafting agent must follow, e.g. 'Always address as \"Bro. Ellis\", never \"Sam\".'",
            None,
        ));
    }
    if rule.chars().count() > MAX_RULE_CHARS {
        return Err(McpError::invalid_params(
            format!(
                "rule is {} characters; the limit is {MAX_RULE_CHARS}. Pinned rules are injected into every draft's context — split this into separate one-instruction rules.",
                rule.chars().count()
            ),
            None,
        ));
    }

    Ok((scope, recipient, rule.to_string()))
}

/// Warn on a draft to someone who has pinned rules.
///
/// This is the load-bearing half of "pinned rules override derived style". The
/// skills tell an agent to look the rules up first, and `get_voice_profile`
/// returns them — but neither is on a path the agent is *obliged* to take,
/// whereas `compose_draft` always runs. Advisory rather than a refusal because
/// no check can decide whether prose honours "always address as Bro. Ellis";
/// what it can do is make sure the rule is never invisible.
fn pinned_rule_advisory(rules: &[db::voice_pinned_rules::PinnedRule]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let list = rules
        .iter()
        .map(|r| format!("\n  • {}", r.rule))
        .collect::<Vec<_>>()
        .join("");
    format!(
        "\n⚠ PINNED RULES apply to this message and OVERRIDE the recipient's derived writing style:{list}\nIf the draft you just wrote breaks one, fix it with edit_draft now."
    )
}

/// Enforce the house dash rule on an authored draft, rewriting it in place and
/// returning the note to append to the tool result.
///
/// **This is the guarantee; `pinned_rule_advisory` above is not.** That note
/// rides in the tool *result*, so the model sees it only after the draft is on
/// IMAP — it buys a follow-up `edit_draft`, not compliance. It was pinned, it
/// was surfaced, and em-dashes still went out, because the recipient's derived
/// profile described the user's real past habit ("Uses em-dashes, parentheses,
/// bullets") and `build_voice_context_full` feeds that in as the style to match.
/// A pinned rule states an absolute, and only a deterministic pass keeps one.
///
/// Placement matters twice over:
///
/// - **On the authored text only**, before `render_body`, the signature, and
///   the quoted original are attached. `format_quoted_history` output is a
///   byte-contract with the frontend (gotcha #30) and is someone else's prose;
///   the stored signature belongs to the user.
/// - **Above the IMAP work in both handlers** — before the APPEND, per gotcha
///   #30's rule that nothing fallible sits between the IMAP writes. (This one
///   cannot fail, but the ordering is the invariant, not the current
///   implementation's luck.)
///
/// Because it sits at the write boundary rather than in a prompt, it also
/// catches text the model never authored: a paste, or an external voice-rewrite
/// service handing prose back with the dashes reinstated. That is the case this
/// was built for.
fn enforce_dash_rule(subject: &mut String, body: &mut String) -> String {
    use crate::email::dashes::normalize_dashes;

    let fixed_subject = normalize_dashes(subject);
    let fixed_body = normalize_dashes(body);
    let total = fixed_subject.replaced + fixed_body.replaced;
    if total == 0 {
        return String::new();
    }
    *subject = fixed_subject.text;
    *body = fixed_body.text;

    // Body samples first: that is where prose lives and where a substitution is
    // most likely to want a human second look.
    let samples = fixed_body
        .samples
        .iter()
        .chain(fixed_subject.samples.iter())
        .take(3)
        .map(|w| format!("\n  • {w}"))
        .collect::<Vec<_>>()
        .join("");

    format!(
        "\n✎ Rewrote {total} long dash{plural} to house style before saving (pinned rule: never use em-dashes).{samples}\nThe draft on the server is the rewritten text. If a replacement reads wrong, call edit_draft with that sentence reworded — never reinstate the dash, and never substitute a hyphen.",
        plural = if total == 1 { "" } else { "es" }
    )
}

static DATA_URI_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"data:[a-zA-Z0-9/+.-]+;base64,[A-Za-z0-9+/=]+").unwrap());

/// Replaces inline `data:` base64 payloads (cid-resolved images, gotcha #14 —
/// up to 5MB total) with a size placeholder before an HTML preview is handed
/// back to an MCP client, so one draft with an embedded image can't dump
/// megabytes of base64 into the model's context.
fn redact_data_uris(html: &str) -> String {
    DATA_URI_RE
        .replace_all(html, |caps: &regex::Captures| {
            format!("data:[inline image omitted, {} bytes]", caps[0].len())
        })
        .into_owned()
}

/// Trim a sent-mail excerpt for safe inclusion in an MCP response.
/// Drops `>` quoted-reply lines, cuts at common signature markers, and caps
/// length at `max_chars`. Does not attempt to redact PII — if the user has
/// authorized the MCP client to read their sent mail, the text content of the
/// samples is the very thing that conveys their voice.
fn sanitize_voice_excerpt(text: &str, max_chars: usize) -> String {
    // Drop quoted-reply lines first
    let no_quotes: String = text
        .split('\n')
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");

    // Cut at signature markers (best-effort, no regex)
    let mut end = no_quotes.len();
    for marker in [
        "\n-- \n",
        "\n--\n",
        "\nSent from my ",
        "\nGet Outlook for ",
        "\n-----Original Message-----",
        "\n---------- Forwarded message",
    ] {
        if let Some(i) = no_quotes.find(marker) {
            if i < end {
                end = i;
            }
        }
    }
    let trimmed = no_quotes[..end].trim();

    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated.trim_end())
}

enum VoiceProfileSampleSource {
    Recipient(String),
    Archetype(String),
    Account,
}

/// The `resolve_project_repo` answer, in three states an agent must not
/// confuse: a usable repo, a mapping whose directory is gone, and no mapping.
///
/// The second must never read as the first — a stale mapping handed out as a
/// path sends the agent reading a directory that is not there — and the third
/// must never be papered over with a guess, since the whole point is that the
/// USER decided which project a correspondent belongs to.
pub(crate) fn project_repo_payload(
    resolved: Option<&db::claude_repos::ResolvedRepo>,
    is_dir: bool,
    has_claude_md: bool,
) -> serde_json::Value {
    match resolved {
        Some(r) if is_dir => serde_json::json!({
            "status": "mapped",
            "repo_path": r.repo_path,
            "chosen_by": format!("{} {}", r.scope.as_str(), r.source),
            "claude_md": if has_claude_md {
                serde_json::Value::String(format!("{}/CLAUDE.md", r.repo_path))
            } else {
                serde_json::Value::Null
            },
            "next_step": if has_claude_md {
                "Read its CLAUDE.md first — it says what the project is and where things live."
            } else {
                "No CLAUDE.md there; start from its README or top-level listing."
            },
        }),
        Some(r) => serde_json::json!({
            "status": "missing",
            "repo_path": r.repo_path,
            "chosen_by": format!("{} {}", r.scope.as_str(), r.source),
            "next_step": "This correspondent is mapped to that directory, but it is not there right now (moved, or on an unmounted volume). Tell the user; do not read elsewhere in its place.",
        }),
        None => serde_json::json!({
            "status": "unmapped",
            "next_step": "No project is linked to this correspondent. Ask the user which project it is — never guess a directory. They can link it in CXMail Settings → Claude repos.",
        }),
    }
}

// ─── MCP Server ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct CxMailMcp {
    tool_router: ToolRouter<Self>,
    db_path: String,
    /// Set to true once a real MCP client completes the `initialize` handshake.
    /// The orphan watchdog (see `bin/mcp.rs`) reaps processes that are never
    /// claimed within a grace window — e.g. abandoned pre-warmed spares.
    claimed: Arc<AtomicBool>,
}

impl CxMailMcp {
    fn open_db(&self) -> Result<rusqlite::Connection, McpError> {
        let conn = rusqlite::Connection::open(&self.db_path)
            .map_err(|e| McpError::internal_error(format!("DB open failed: {}", e), None))?;
        // PRAGMA foreign_keys is per-connection in SQLite. Schema declares
        // ON DELETE CASCADE for message_bodies / attachments / calendar_events,
        // but the cascade won't fire without this. edit_draft relies on it.
        if let Err(e) = conn.execute_batch("PRAGMA foreign_keys = ON;") {
            log::warn!("MCP open_db: PRAGMA foreign_keys = ON failed: {e}");
        }
        Ok(conn)
    }

    /// Clone the shared "claimed by a client" flag for the orphan watchdog.
    pub fn claimed_flag(&self) -> Arc<AtomicBool> {
        self.claimed.clone()
    }

    fn get_account(
        &self,
        conn: &rusqlite::Connection,
        account_id: &str,
    ) -> Result<db::accounts::Account, McpError> {
        db::accounts::get_by_id(conn, account_id)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Account {} not found", account_id), None)
            })
    }

    /// Resolve an `Option<&str>` account_id to a concrete account ID, falling
    /// back to the first configured account when the input is None or empty.
    /// Returns InvalidParams if no accounts are configured.
    fn resolve_account_id(
        &self,
        conn: &rusqlite::Connection,
        account_id: Option<&str>,
    ) -> Result<String, McpError> {
        if let Some(id) = account_id.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return Ok(id.to_string());
        }
        let accounts = db::accounts::list(conn)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        accounts
            .into_iter()
            .next()
            .map(|a| a.id)
            .ok_or_else(|| McpError::invalid_params("No accounts configured", None))
    }

    /// Every pinned rule that governs a draft: the account-wide ones plus those
    /// pinned for anyone in `to`, in that order, deduped.
    ///
    /// Scoped to `to` and not `cc` on purpose. These rules are overwhelmingly
    /// about how the message addresses its reader, and a note that fires for
    /// every bystander on a thread is noise — which is what trains a caller to
    /// stop reading the notes that matter (same reasoning as `layout_advisory`).
    fn pinned_rules_for_draft(
        &self,
        conn: &rusqlite::Connection,
        account_id: &str,
        to: &[String],
    ) -> Vec<db::voice_pinned_rules::PinnedRule> {
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<db::voice_pinned_rules::PinnedRule> = Vec::new();
        for rule in db::voice_pinned_rules::list_account_scoped(conn, account_id).unwrap_or_default()
        {
            if seen.insert(rule.id) {
                out.push(rule);
            }
        }
        for recipient in to {
            let recipient = recipient.trim().to_lowercase();
            if recipient.is_empty() {
                continue;
            }
            for rule in
                db::voice_pinned_rules::list_effective(conn, account_id, &recipient).unwrap_or_default()
            {
                if seen.insert(rule.id) {
                    out.push(rule);
                }
            }
        }
        out
    }

    /// Generate a draft body via the external writer (Antigravity `agy`).
    ///
    /// All DB reads happen first and the `Connection` is dropped before the
    /// subprocess await: the handler future must stay `Send` (#30/#57), the
    /// app shares this SQLite file (#12), and a generation can take tens of
    /// seconds. Reply context comes from the LOCAL cache only — the writer can
    /// do its job without it, and an IMAP round trip here would put a connect
    /// timeout on the critical path of every generated reply (#56's lesson).
    async fn generate_draft_body(
        &self,
        account_id: &str,
        to: &[String],
        subject: &str,
        instruction: &str,
        model: &str,
        reply_to_folder: Option<&str>,
        reply_to_uid: Option<u32>,
        rewrite_source: Option<String>,
    ) -> Result<String, McpError> {
        let (voice_ctx, reply_context) = {
            let conn = self.open_db()?;
            let recipient = to.first().map(String::as_str).unwrap_or("");
            // The SAME assembler the in-app AI drafting uses (one matcher,
            // #36): pinned rules ride first inside it, so on this path they
            // finally sit BEFORE the write instead of in the advisory after
            // it (#47).
            let voice_ctx =
                crate::email::ai::build_voice_ctx_for_recipient(&conn, account_id, recipient);
            let reply_context = match (reply_to_folder, reply_to_uid) {
                (Some(folder), Some(uid)) => {
                    db::messages::get_body(&conn, account_id, folder, uid)
                        .ok()
                        .flatten()
                        .and_then(|b| b.plain_text)
                        .filter(|t| !t.trim().is_empty())
                }
                _ => None,
            };
            (voice_ctx, reply_context)
        };
        let prompt = crate::email::external_writer::build_writer_prompt(
            voice_ctx.as_deref(),
            to,
            subject,
            instruction,
            reply_context.as_deref(),
            rewrite_source.as_deref(),
        );
        crate::email::external_writer::write_prose(model, &prompt)
            .await
            .map_err(|e| {
                McpError::internal_error(
                    format!("External writer failed — nothing was written to the mailbox: {e}"),
                    None,
                )
            })
    }

    /// Build threading headers + quoted history for a reply. One
    /// implementation for `compose_draft` and `edit_draft`, which carried
    /// verbatim clones of this logic.
    ///
    /// **Fails loudly**: when a reply target was supplied, `quote_original`
    /// isn't `Some(false)`, and the body isn't already quoted, a quote that
    /// can't be built returns an `McpError` naming the reason instead of
    /// silently producing a quote-less draft. Both callers invoke this
    /// strictly *before* the IMAP APPEND, so erroring here leaves no stray
    /// draft — and in `edit_draft`, which appends before it retires the old
    /// revision (gotcha #57), no duplicate either.
    ///
    /// `Send` note: `rusqlite::Connection` is `Send`, but `&Connection` is
    /// not. The short-lived `{ }` blocks below close around the await so no
    /// borrow is live across it. If `cargo check` ever reports "future cannot
    /// be sent between threads safely … `&rusqlite::Connection` is not
    /// `Send`", tighten those scopes — never `Box::pin` or spawn.
    async fn build_reply_context(
        &self,
        account_id: &str,
        account: &db::accounts::Account,
        target: Option<ReplyTarget>,
        quote_original: Option<bool>,
        body: &str,
    ) -> Result<ReplyContext, McpError> {
        let Some(target) = target else {
            return Ok(ReplyContext::default());
        };

        let want_quote = quote_original.unwrap_or(true) && !body_already_quoted(body);

        let (in_reply_to, references, plan) = {
            let conn = self.open_db()?;
            let (irt, refs) = resolve_threading(&conn, account_id, &target);
            let plan = want_quote.then(|| plan_quote(&conn, account_id, &target));
            (irt, refs, plan)
        }; // conn dropped here — nothing borrows it across the await below.

        let Some(plan) = plan else {
            return Ok(ReplyContext {
                in_reply_to,
                references,
                quote: None,
                note: String::new(),
            });
        };

        let outcome = run_quote_plan(account, plan).await;

        // A coordinates reply to a message the DB never indexed has no local
        // headers. The fetch already parsed the real ones — use them rather
        // than shipping an unthreaded reply.
        let (in_reply_to, references) = match (&in_reply_to, &outcome.fetched_headers) {
            (None, Some((mid, refs))) => {
                build_threading_headers(mid.as_deref(), refs.as_deref())
            }
            _ => (in_reply_to, references),
        };

        let conn = self.open_db()?;
        let (quote, note) = finish_quote(&conn, account_id, outcome)?;
        Ok(ReplyContext {
            in_reply_to,
            references,
            quote,
            note,
        })
    }
}

#[tool_router]
impl CxMailMcp {
    pub fn new(db_path: String) -> Self {
        Self {
            tool_router: Self::tool_router(),
            db_path,
            claimed: Arc::new(AtomicBool::new(false)),
        }
    }

    // ─── Open tier ─────────────────────────────────────────────

    #[tool(
        name = "search_emails",
        description = "Search emails by query across subject, sender, recipients, snippet, and body. Optional filters: account_ids (call list_accounts first to get IDs), since (ISO 8601, inclusive), until (ISO 8601, exclusive). Without account_ids, accounts hidden from aggregated views are skipped and the result says which; pass their IDs to search them."
    )]
    async fn search_emails(
        &self,
        Parameters(params): Parameters<SearchEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let limit = params.limit.unwrap_or(20) as u32;

        // An unscoped search skips hidden accounts (the DB rule); say so, so
        // an agent that finds nothing knows there is a mailbox it did not look
        // in. Computed once for both the FTS and the LIKE path.
        let scoped = params
            .account_ids
            .as_ref()
            .is_some_and(|ids| !ids.is_empty());
        let skipped_note = if scoped {
            None
        } else {
            hidden_accounts_note(&db::accounts::list_hidden(&conn).unwrap_or_default())
        };
        let with_note = |text: String| match &skipped_note {
            Some(note) => format!("{text}\n{note}"),
            None => text,
        };

        // Reject empty / whitespace-only queries: `%%` would match every message
        // and return the whole mailbox in date order (BUG-04).
        let query = params.query.trim();
        if query.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found. (empty search query)".to_string(),
            )]));
        }

        // Preferred path: the shared FTS5 engine (same ranking/AND semantics
        // as the UI). Guarded by table existence as a belt-and-braces check:
        // `bin/mcp.rs` DOES run `schema::initialize` at startup, but only when
        // the DB file already exists, and the migration can fail (logged, not
        // fatal) — a stale pre-v40 DB then falls through to the LIKE query.
        let has_fts: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='messages_fts')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if has_fts {
            let filters = db::search::SearchFilters {
                keywords: Some(query.to_string()),
                // `since` inclusive / `until` exclusive: datetime() of a bare
                // date is midnight, so <=/>= against it preserves both.
                date_after: params.since.clone(),
                date_before: params.until.clone(),
                account_ids: params
                    .account_ids
                    .clone()
                    .filter(|v| !v.is_empty()),
                ..Default::default()
            };
            match db::search::search(&conn, &filters, limit, 0, false) {
                Ok(results) => {
                    let text = if results.is_empty() {
                        "No results found.".to_string()
                    } else {
                        results
                            .iter()
                            .map(|r| {
                                // Strip the UI highlight markers from snippets.
                                let snippet: String = r
                                    .snippet
                                    .chars()
                                    .filter(|c| {
                                        *c != db::search::HIGHLIGHT_START
                                            && *c != db::search::HIGHLIGHT_END
                                    })
                                    .collect();
                                format!(
                                    "Account:{} | Folder:{} | UID:{} | {} | From: {} <{}> | {} | {}",
                                    r.account_id,
                                    r.folder_name,
                                    r.uid,
                                    r.subject.as_deref().unwrap_or_default(),
                                    r.from_name.as_deref().unwrap_or_default(),
                                    r.from_email.as_deref().unwrap_or_default(),
                                    r.date,
                                    snippet,
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    return Ok(CallToolResult::success(vec![Content::text(with_note(text))]));
                }
                Err(e) => {
                    // Fall through to the LIKE query rather than failing the tool.
                    tracing::warn!("search_emails: FTS path failed, using LIKE fallback: {e}");
                }
            }
        }

        let pattern = format!("%{}%", query);

        // Build the WHERE clause dynamically. The LIKE group is always present;
        // account_ids / since / until are appended as needed.
        //
        // Short queries (<= 4 chars) match only header/snippet fields and skip
        // the full body text: otherwise a short token like "Northwind" matches
        // invisible substrings inside tracking URLs in `plain_text`, surfacing
        // dozens of unrelated promo emails (BUG-04 precision). The snippet (the
        // human-visible preview) is still matched.
        let like_group = if query.chars().count() <= 4 {
            "(m.subject LIKE ?1 OR m.from_email LIKE ?1 OR m.from_name LIKE ?1 \
              OR m.to_list LIKE ?1 OR m.cc_list LIKE ?1 OR m.snippet LIKE ?1)"
        } else {
            "(m.subject LIKE ?1 OR m.from_email LIKE ?1 OR m.from_name LIKE ?1 \
              OR m.to_list LIKE ?1 OR m.cc_list LIKE ?1 OR m.snippet LIKE ?1 \
              OR mb.plain_text LIKE ?1)"
        };
        let mut conditions: Vec<String> = vec![like_group.to_string()];
        let mut sql_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];
        let mut idx: usize = 2;

        if let Some(ids) = params.account_ids.as_ref().filter(|v| !v.is_empty()) {
            let placeholders: Vec<String> =
                (0..ids.len()).map(|i| format!("?{}", idx + i)).collect();
            conditions.push(format!("m.account_id IN ({})", placeholders.join(", ")));
            for id in ids {
                sql_params.push(Box::new(id.clone()));
            }
            idx += ids.len();
        }

        if let Some(since) = params.since.as_ref() {
            conditions.push(format!("m.date >= ?{}", idx));
            sql_params.push(Box::new(since.clone()));
            idx += 1;
        }

        if let Some(until) = params.until.as_ref() {
            conditions.push(format!("m.date < ?{}", idx));
            sql_params.push(Box::new(until.clone()));
            idx += 1;
        }

        let sql = format!(
            "SELECT m.account_id, m.folder_name, m.uid, m.subject, m.from_name, m.from_email, m.date, m.snippet \
             FROM messages m \
             LEFT JOIN message_bodies mb ON m.account_id = mb.account_id AND m.folder_name = mb.folder_name AND m.uid = mb.uid \
             WHERE {} \
             ORDER BY m.date DESC LIMIT ?{}",
            conditions.join(" AND "),
            idx
        );
        sql_params.push(Box::new(limit));

        let bound: Vec<&dyn rusqlite::types::ToSql> =
            sql_params.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let results: Vec<String> = stmt
            .query_map(bound.as_slice(), |row| {
                Ok(format!(
                    "Account:{} | Folder:{} | UID:{} | {} | From: {} <{}> | {} | {}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                ))
            })
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?
            .filter_map(|r| r.ok())
            .collect();

        let text = if results.is_empty() {
            "No results found.".to_string()
        } else {
            results.join("\n")
        };
        Ok(CallToolResult::success(vec![Content::text(with_note(text))]))
    }

    #[tool(
        name = "resolve_project_repo",
        description = "Which project directory (code repo / client folder) a message belongs to, using the user's own mappings in CXMail Settings → Claude repos — the same resolver \"Open in Claude\" uses. Pass the coordinates of a message involving the person or project (from search_emails / read_email / read_thread). Returns the path, which mapping chose it, and whether it has a CLAUDE.md; read that CLAUDE.md before anything else in the repo. \"unmapped\" means the user has not linked this correspondent to any project: say so and ask which project it is — never guess a directory. Read-only."
    )]
    async fn resolve_project_repo(
        &self,
        Parameters(params): Parameters<ResolveProjectRepoParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let resolved = db::claude_repos::resolve_for_message(
            &conn,
            &params.account_id,
            &params.folder,
            params.uid,
        )
        .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        drop(conn);
        // Checked at use, like the handoff's `split_on_existence`: a repo on an
        // unmounted volume comes back without anybody editing the row.
        let (is_dir, has_claude_md) = match &resolved {
            Some(r) => {
                let dir = std::path::Path::new(&r.repo_path);
                (dir.is_dir(), dir.join("CLAUDE.md").is_file())
            }
            None => (false, false),
        };
        let payload = project_repo_payload(resolved.as_ref(), is_dir, has_claude_md);
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "read_email",
        description = "Read the full content of an email by account_id, folder, and UID."
    )]
    async fn read_email(
        &self,
        Parameters(params): Parameters<ReadEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        // Look up message_id from DB for threading
        let (db_message_id, _, _) = db::messages::get_message_headers(
            &conn,
            &params.account_id,
            &params.folder,
            params.uid,
        )
        .unwrap_or((None, None, None));

        // Try cached body first
        if let Ok(Some(body)) =
            db::messages::get_body(&conn, &params.account_id, &params.folder, params.uid)
        {
            // Query basic headers from DB for the cached path
            let msg =
                db::messages::get_by_uid(&conn, &params.account_id, &params.folder, params.uid)
                    .ok()
                    .flatten();
            // Recipient set for the cached path. Without these lines the agent
            // sees no To/Cc at all and cannot rebuild a reply-all without
            // dropping recipients (gotcha: recipient-integrity / the "Sam"
            // incident). message_bodies carries the full lists as JSON.
            let (to_json, cc_json, bcc_json) = db::messages::get_address_lists(
                &conn,
                &params.account_id,
                &params.folder,
                params.uid,
            )
            .unwrap_or((None, None, None));
            let recipients = fmt_recipient_lines(
                &parse_addr_json(to_json.as_deref()),
                &parse_addr_json(cc_json.as_deref()),
                &parse_addr_json(bcc_json.as_deref()),
            );
            // Folder + UID are the coordinates compose_draft/edit_draft want
            // (`reply_to_folder` / `reply_to_uid`); Message-ID is normalized so
            // the ID an agent copies is bracketed regardless of cache state.
            let coords = format!("Folder: {}\nUID: {}\n", params.folder, params.uid);
            let mid = normalized_mid(db_message_id.as_deref());
            let header = if let Some(m) = msg {
                format!(
                    "From: {}\n{}{}Subject: {}\nDate: {}\nMessage-ID: {}\n\n",
                    m.from_email.as_deref().unwrap_or(""),
                    recipients,
                    coords,
                    m.subject.as_deref().unwrap_or(""),
                    m.date,
                    mid,
                )
            } else {
                format!("{}{}Message-ID: {}\n\n", recipients, coords, mid)
            };
            let text = body
                .plain_text
                .unwrap_or_else(|| body.sanitized_html.unwrap_or_default());
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "{}{}",
                header, text
            ))]));
        }

        // Fetch from IMAP
        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.folder).await;
        let raw_body = imap::fetch_body(&mut session, params.uid)
            .await
            .map_err(|e| McpError::internal_error(format!("Fetch failed: {}", e), None))?;
        let _ = imap::disconnect(session, &account.email).await;

        let parsed = parser::parse_message(&raw_body);
        let text = parsed
            .plain_text
            .unwrap_or_else(|| parsed.sanitized_html.unwrap_or_default());
        // mail-parser strips the brackets, so this branch used to hand back a
        // BARE id while the cached branch above handed back a bracketed one —
        // the copied ID's format depended on cache state (gotcha #30).
        let mid = normalized_mid(parsed.message_id.as_deref().or(db_message_id.as_deref()));
        let recipients =
            fmt_recipient_lines(&parsed.to_list, &parsed.cc_list, &parsed.bcc_list);
        let header = format!(
            "From: {} <{}>\nFolder: {}\nUID: {}\n{}Subject: {}\nDate: {}\nMessage-ID: {}\n\n",
            parsed.from_name.unwrap_or_default(),
            parsed.from_email,
            params.folder,
            params.uid,
            recipients,
            parsed.subject.unwrap_or_default(),
            parsed.date.unwrap_or_default(),
            mid,
        );
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{}{}",
            header, text
        ))]))
    }

    #[tool(
        name = "read_thread",
        description = "Read all messages in a conversation thread by message_id. Returns Folder, UID and Message-ID for each message — pass the latest message's Folder + UID to compose_draft as reply_to_folder + reply_to_uid to thread and quote a reply."
    )]
    async fn read_thread(
        &self,
        Parameters(params): Parameters<ReadThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let thread =
            db::messages::get_thread_with_ids(&conn, &params.account_id, &params.message_id)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let text = thread
            .iter()
            .map(|m| {
                // Surface To/Cc per message so an agent drafting a reply-all
                // from thread context has the full recipient set (recipient
                // integrity — see fmt_recipient_lines). Cc omitted when empty.
                let to = fmt_addr_json_compact(m.to_list.as_deref());
                let cc = fmt_addr_json_compact(m.cc_list.as_deref());
                let recipients = if cc.is_empty() {
                    format!("To: {}", to)
                } else {
                    format!("To: {} | Cc: {}", to, cc)
                };
                // Folder is printed alongside UID because UIDs are
                // folder-scoped — without it an agent working from read_thread
                // can't use compose_draft's reply_to_folder/reply_to_uid path.
                format!(
                    "Folder:{} | UID:{} | From: {} | {} | {} | {} | Message-ID: {}",
                    m.folder_name,
                    m.uid,
                    m.from_email.as_deref().unwrap_or(""),
                    recipients,
                    m.subject.as_deref().unwrap_or(""),
                    m.date,
                    normalized_mid(m.message_id.as_deref()),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(CallToolResult::success(vec![Content::text(
            if text.is_empty() {
                "No messages in thread.".to_string()
            } else {
                text
            },
        )]))
    }

    #[tool(
        name = "read_email_source",
        description = "Return the VERBATIM RFC 5322 header block of a message — every header the server sent, exactly as sent. This is the only way to see headers CXMail does not model: Authentication-Results (SPF/DKIM/DMARC verdicts), the Received chain, DKIM-Signature, Return-Path, List-*, X-* anything. read_email and preview_email_html cannot show you these: read_email returns a curated From/To/Subject/Date summary plus body text, and preview_email_html returns the body markup. Pass `headers` with a list of header names to get just those lines (e.g. [\"authentication-results\"]) instead of the full 1–8 KB block. Body content is NOT included — use read_email or preview_email_html for that. Headers are cached after the first read; for mail synced before this feature existed the first call costs one small IMAP fetch. PRIVACY: this contains routing IPs and infrastructure detail — read it when asked to inspect delivery/authentication, don't dump it into summaries."
    )]
    async fn read_email_source(
        &self,
        Parameters(params): Parameters<ReadEmailSourceParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let cached =
            db::messages::get_raw_headers(&conn, &params.account_id, &params.folder, params.uid)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let (block, source) = match cached {
            Some(h) => (h, "cached"),
            None => {
                // Lazy fetch — headers only, one UID. Same shape as T26's flag
                // push: open a session, select, do the one thing, disconnect,
                // and write to the DB only after the socket is closed (a
                // blocking socket inside a write is gotcha #11).
                drop(conn);
                let mut session = imap::connect_for_account(&account)
                    .await
                    .map_err(|e| {
                        McpError::internal_error(
                            format!(
                                "IMAP connect failed, so the header block for Folder: {} UID: {} \
                                 could not be fetched (it was not cached): {}",
                                params.folder, params.uid, e
                            ),
                            None,
                        )
                    })?;
                if let Err(e) = imap::select_folder(&mut session, &params.folder).await {
                    let _ = imap::disconnect(session, &account.email).await;
                    return Err(McpError::internal_error(
                        format!("Could not select folder {}: {}", params.folder, e),
                        None,
                    ));
                }
                let fetched = imap::fetch_raw_headers(&mut session, params.uid).await;
                let _ = imap::disconnect(session, &account.email).await;
                let raw = fetched.map_err(|e| {
                    McpError::internal_error(format!("Header fetch failed: {}", e), None)
                })?;
                let block = parser::header_block_to_string(&raw).ok_or_else(|| {
                    McpError::internal_error(
                        "Server returned an empty or oversized header block".to_string(),
                        None,
                    )
                })?;
                // Cache it so a follow-up question costs nothing. Best-effort:
                // a write failure must not fail the read.
                let conn = self.open_db()?;
                if let Err(e) = db::messages::store_raw_headers(
                    &conn,
                    &params.account_id,
                    &params.folder,
                    params.uid,
                    &block,
                ) {
                    log::warn!("read_email_source: failed to cache headers: {e}");
                }
                (block, "fetched from IMAP")
            }
        };

        let (body, note) = match params.headers.as_ref() {
            Some(wanted) if !wanted.is_empty() => {
                let matched = filter_header_lines(&block, wanted);
                if matched.is_empty() {
                    (
                        String::new(),
                        format!(
                            "None of the requested headers ({}) are present on this message. \
                             Re-run without `headers` to see the full block.",
                            wanted.join(", ")
                        ),
                    )
                } else {
                    (matched, String::new())
                }
            }
            _ => (block, String::new()),
        };

        let text = if body.is_empty() {
            format!(
                "Raw headers for Folder: {} UID: {} ({}).\n\n{}",
                params.folder, params.uid, source, note
            )
        } else {
            format!(
                "Raw headers for Folder: {} UID: {} ({}) — verbatim, body not included:\n\n{}",
                params.folder, params.uid, source, body
            )
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "preview_email_html",
        description = "Return the literal HTML markup stored for a message or draft — the actual output of compose_draft/edit_draft's rendering pipeline (layout=\"card\"/\"rich\"/omitted), not the text you passed in. read_email echoes back plain text and will NOT show you this. Use it to verify structure before telling the user a draft looks right — e.g. that layout=\"rich\" preserved your tags instead of the default path flattening them into plain <p> paragraphs, or that a card's chrome survived sanitization. This is NOT a rendered preview: it is raw markup for structural inspection only — judge layout/spacing/color from the tags, don't assume how they'll paint. Embedded cid: images are replaced with a size placeholder so base64 doesn't fill your context."
    )]
    async fn preview_email_html(
        &self,
        Parameters(params): Parameters<PreviewEmailHtmlParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let html = if let Ok(Some(body)) =
            db::messages::get_body(&conn, &params.account_id, &params.folder, params.uid)
        {
            body.sanitized_html.unwrap_or_default()
        } else {
            let mut session = imap::connect_for_account(&account)
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("IMAP connect failed: {}", e), None)
                })?;
            let _ = imap::select_folder(&mut session, &params.folder).await;
            let raw_body = imap::fetch_body(&mut session, params.uid)
                .await
                .map_err(|e| McpError::internal_error(format!("Fetch failed: {}", e), None))?;
            let _ = imap::disconnect(session, &account.email).await;
            parser::parse_message(&raw_body).sanitized_html.unwrap_or_default()
        };

        if html.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "(no HTML body stored for Folder: {} UID: {} — it may be plain-text-only)",
                params.folder, params.uid
            ))]));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Raw stored HTML for Folder: {} UID: {} — structural inspection only, NOT a rendered preview:\n\n{}",
            params.folder,
            params.uid,
            redact_data_uris(&html)
        ))]))
    }

    #[tool(
        name = "list_folders",
        description = "List email folders with unread counts. Optionally filter by account_id."
    )]
    async fn list_folders(
        &self,
        Parameters(params): Parameters<ListFoldersParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let accounts = if let Some(id) = &params.account_id {
            vec![self.get_account(&conn, id)?]
        } else {
            db::accounts::list(&conn)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?
        };

        let mut lines = Vec::new();
        for account in &accounts {
            let folders = db::folders::list_by_account(&conn, &account.id)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
            for f in &folders {
                lines.push(format!(
                    "{} | {} | {} ({} unread)",
                    account.email,
                    f.name,
                    f.folder_type.as_deref().unwrap_or("other"),
                    f.unread_count
                ));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "list_accounts",
        description = "List all configured email accounts."
    )]
    async fn list_accounts(&self) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let accounts = db::accounts::list(&conn)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let text = accounts
            .iter()
            .map(|a| {
                // The alias list is printed only when there IS one: every
                // account would otherwise carry a `Send-as:` segment restating
                // its own address, which trains the reader to skip the line
                // (#34's reasoning). It is what makes `compose_draft`'s `from`
                // discoverable — the param description points here.
                let aliases = db::identities::send_as_addresses(
                    &conn,
                    &a.id,
                    &a.email,
                    a.display_name.as_deref(),
                )
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !s.is_primary)
                .map(|s| s.email)
                .collect::<Vec<_>>();
                let send_as = if aliases.is_empty() {
                    String::new()
                } else {
                    format!(" | Send-as: {}", aliases.join(", "))
                };
                format!(
                    "ID: {} | {} | Provider: {} | Active: {} | Open tracking: {} | Hidden from aggregates: {}{}",
                    a.id,
                    a.email,
                    a.provider,
                    a.is_active,
                    if a.track_opens_enabled { "on" } else { "off" },
                    if a.hidden_from_aggregates { "yes" } else { "no" },
                    send_as
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "list_archetypes",
        description = "List cached writing-style archetypes. Omit account_id to list archetypes for all accounts. Use archetype_id with get_voice_profile."
    )]
    async fn list_archetypes(
        &self,
        Parameters(params): Parameters<ListArchetypesParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let accounts = if let Some(id) = params.account_id.as_ref() {
            vec![self.get_account(&conn, id)?]
        } else {
            db::accounts::list(&conn)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?
        };

        let mut out = Vec::new();
        for account in &accounts {
            let rows = db::voice_archetypes::list_for_account(&conn, &account.id)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
            for row in rows {
                out.push(serde_json::json!({
                    "account_id": &account.id,
                    "account_email": &account.email,
                    "archetype_id": row.archetype_id,
                    "name": row.name,
                    "description": row.description,
                    "sample_count": row.sample_count,
                    "generated_at": row.generated_at,
                }));
            }
        }

        let text = serde_json::to_string_pretty(&out)
            .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "set_voice_rule",
        description = "Pin a PERMANENT writing rule the user has stated — an absolute instruction for how to write, such as 'Always address Sam as \"Bro. Ellis\", never \"Sam\"' or 'Never open with \"Hope you're well\"'. NOT a mail filter: to sort, categorise or flag incoming mail use create_mail_rule instead. Use this whenever the user says something should ALWAYS or NEVER happen in their email. Pass recipient_email to pin the rule to one correspondent, or omit it for an account-wide rule that governs every message sent from that account. Pinned rules are stored separately from the AI-derived voice profile, so extract_recipient_profile (even with force=true) cannot erase them, and they OVERRIDE anything the derived profile says — which is the point: a derived profile only describes what the user usually does, so a mixed history leaves an agent free to do the wrong thing. Re-pinning identical text is a no-op, not a duplicate."
    )]
    async fn set_voice_rule(
        &self,
        Parameters(params): Parameters<SetVoiceRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        use db::voice_pinned_rules::{Scope, MAX_RULES_PER_SCOPE};

        let (scope, recipient, rule) = validate_pinned_rule(
            params.scope.as_deref(),
            params.recipient_email.as_deref(),
            &params.rule,
        )?;

        let conn = self.open_db()?;
        let account_id = self.resolve_account_id(&conn, params.account_id.as_deref())?;
        let account = self.get_account(&conn, &account_id)?;

        let existing = db::voice_pinned_rules::count_for_scope(&conn, &account_id, scope, &recipient)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        if existing >= MAX_RULES_PER_SCOPE {
            return Err(McpError::invalid_params(
                format!(
                    "{} already has {existing} pinned rules (limit {MAX_RULES_PER_SCOPE}). Delete one with delete_voice_rule, or merge two into a single instruction.",
                    match scope {
                        Scope::Account => format!("Account {}", account.email),
                        Scope::Recipient => recipient.clone(),
                    }
                ),
                None,
            ));
        }

        let (id, created) =
            db::voice_pinned_rules::add(&conn, &account_id, scope, &recipient, &rule)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let target = match scope {
            Scope::Account => format!("every message from {}", account.email),
            Scope::Recipient => format!("mail to {} from {}", recipient, account.email),
        };
        let text = if created {
            format!("Pinned rule id={id} for {target}:\n  • {rule}\n\nIt applies to every future draft and survives extract_recipient_profile(force=true).")
        } else {
            format!("Rule id={id} was already pinned for {target}:\n  • {rule}\n\nNothing changed — no duplicate created.")
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "list_voice_rules",
        description = "List the user's PINNED writing rules — permanent instructions like 'always address as Bro. Ellis' (see set_voice_rule). Not mail filters; those are list_mail_rules. Pass recipient_email to see exactly what applies when writing to that person (account-wide rules first, then theirs), or omit it to audit every rule on the account. Call this before drafting on the user's behalf: these rules OVERRIDE the derived voice profile."
    )]
    async fn list_voice_rules(
        &self,
        Parameters(params): Parameters<ListVoiceRulesParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account_id = self.resolve_account_id(&conn, params.account_id.as_deref())?;

        let recipient = params
            .recipient_email
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());

        let rules = match recipient.as_deref() {
            Some(r) => db::voice_pinned_rules::list_effective(&conn, &account_id, r),
            None => db::voice_pinned_rules::list_all(&conn, &account_id),
        }
        .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        let payload = serde_json::json!({
            "account_id": account_id,
            "recipient_email": recipient,
            "count": rules.len(),
            "rules": rules
                .iter()
                .map(|r| serde_json::json!({
                    "id": r.id,
                    "scope": r.scope.as_str(),
                    "recipient_email": if r.recipient_email.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.recipient_email.clone()) },
                    "rule": r.rule,
                    "created_at": r.created_at,
                }))
                .collect::<Vec<_>>(),
            "note": if rules.is_empty() {
                "No pinned rules. These are absolute instructions from the user; a derived voice profile only describes habits."
            } else {
                "ABSOLUTE — follow these exactly. They override the derived voice profile, and where an account rule and a recipient rule disagree the recipient one wins (it is listed later)."
            },
        });
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "delete_voice_rule",
        description = "Remove a pinned writing rule by id (get ids from list_voice_rules). Only do this when the user asks for the rule to be dropped or changed — a pinned rule is something they explicitly told the system to always do. To reword one, delete it and set_voice_rule the new text."
    )]
    async fn delete_voice_rule(
        &self,
        Parameters(params): Parameters<DeleteVoiceRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let existing = db::voice_pinned_rules::get_by_id(&conn, params.id)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "No pinned rule with id {}. Call list_voice_rules for current ids.",
                        params.id
                    ),
                    None,
                )
            })?;

        db::voice_pinned_rules::delete(&conn, params.id)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted pinned rule id={} ({} scope{}):\n  • {}",
            existing.id,
            existing.scope.as_str(),
            if existing.recipient_email.is_empty() {
                String::new()
            } else {
                format!(", {}", existing.recipient_email)
            },
            existing.rule
        ))]))
    }

    #[tool(
        name = "get_voice_profile",
        description = "Return a structured voice profile (writing style) plus optional plain-text excerpts. Use recipient_email for per-recipient style, archetype_id for a writing-style cluster (call list_archetypes first), or omit both for the account-level profile. Use this to draft messages on the user's behalf without sounding like AI."
    )]
    async fn get_voice_profile(
        &self,
        Parameters(params): Parameters<GetVoiceProfileParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.recipient_email.is_some() && params.archetype_id.is_some() {
            return Err(McpError::invalid_params(
                "Provide recipient_email OR archetype_id, not both",
                None,
            ));
        }

        let conn = self.open_db()?;
        let account_id = match params.account_id.clone() {
            Some(id) => id,
            None => {
                let accounts = db::accounts::list(&conn)
                    .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
                accounts
                    .into_iter()
                    .next()
                    .map(|a| a.id)
                    .ok_or_else(|| McpError::invalid_params("No accounts configured", None))?
            }
        };

        let include_samples = params.include_samples.unwrap_or(true);

        // 0. Pinned rules — absolute instructions from the user, loaded before
        //    the derived profile because they can be the ONLY thing to return.
        let pinned = match params.recipient_email.as_ref() {
            Some(r) => {
                db::voice_pinned_rules::list_effective(&conn, &account_id, &r.trim().to_lowercase())
            }
            // No recipient in play, so only account-wide rules can apply.
            None => db::voice_pinned_rules::list_account_scoped(&conn, &account_id),
        }
        .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        // 1. Pick the source profile
        let (source, profile_json, sample_count, generated_at, sample_source) = if let Some(
            recipient_email,
        ) =
            params.recipient_email.as_ref()
        {
            let recipient = recipient_email.trim().to_lowercase();
            let row = db::voice_profiles_recipient::get(&conn, &account_id, &recipient)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
            match row {
                Some(row) => (
                    "recipient".to_string(),
                    Some(row.profile_json),
                    Some(row.sample_count),
                    Some(row.generated_at),
                    VoiceProfileSampleSource::Recipient(recipient),
                ),
                // A missing profile is not an error when the user has pinned
                // rules for this person: the rules are the part that MUST NOT be
                // withheld, and refusing here would hide them for exactly the
                // recipients nobody has extracted a profile for yet.
                None if !pinned.is_empty() => (
                    "pinned-rules-only".to_string(),
                    None,
                    None,
                    None,
                    VoiceProfileSampleSource::Recipient(recipient),
                ),
                None => {
                    return Err(McpError::invalid_params(
                        format!(
                            "No cached voice profile for {}. Have the user open compose to that recipient first, or call this with archetype_id / no recipient.",
                            recipient
                        ),
                        None,
                    ))
                }
            }
        } else if let Some(archetype_id) = params.archetype_id.as_ref() {
            let row = db::voice_archetypes::get(&conn, &account_id, archetype_id)
                .map_err(|e| McpError::internal_error(format!("{}", e), None))?
                .ok_or_else(|| {
                    McpError::invalid_params(format!("No archetype with id {}", archetype_id), None)
                })?;
            let row_archetype_id = row.archetype_id.clone();
            (
                "archetype".to_string(),
                Some(row.profile_json),
                Some(row.sample_count),
                Some(row.generated_at),
                VoiceProfileSampleSource::Archetype(row_archetype_id),
            )
        } else if db::voice_profiles::get_by_account(&conn, &account_id)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?
            .is_none()
            && !pinned.is_empty()
        {
            // Same reasoning as the recipient branch: account-wide pinned rules
            // must reach the caller even before anyone has extracted a profile.
            (
                "pinned-rules-only".to_string(),
                None,
                None,
                None,
                VoiceProfileSampleSource::Account,
            )
        } else {
            let row = db::voice_profiles::get_by_account(&conn, &account_id)
                    .map_err(|e| McpError::internal_error(format!("{}", e), None))?
                    .ok_or_else(|| {
                        McpError::invalid_params(
                            "No account-level voice profile yet. Run ai_extract_voice_profile from the app first.",
                            None,
                        )
                    })?;
            (
                "account".to_string(),
                Some(row.voice_profile_json),
                Some(row.sample_count),
                Some(row.generated_at),
                VoiceProfileSampleSource::Account,
            )
        };

        // 2. Optional samples (≤3 × ≤400 chars, signature/quote stripped)
        let samples: Vec<String> = if include_samples {
            let raw_samples = match &sample_source {
                VoiceProfileSampleSource::Recipient(recipient) => {
                    crate::email::voice::collect_diverse_samples_for_recipient(
                        &conn,
                        &account_id,
                        recipient,
                        3,
                    )
                    .unwrap_or_default()
                }
                VoiceProfileSampleSource::Archetype(archetype_id) => {
                    crate::email::voice::collect_samples_for_archetype(
                        &conn,
                        &account_id,
                        archetype_id,
                        3,
                    )
                    .unwrap_or_default()
                }
                VoiceProfileSampleSource::Account => {
                    crate::email::voice::collect_sent_samples(&conn, &account_id)
                        .unwrap_or_default()
                        .into_iter()
                        .take(3)
                        .collect()
                }
            };
            raw_samples
                .into_iter()
                .map(|s| sanitize_voice_excerpt(&s, 400))
                .filter(|s| !s.is_empty())
                .take(3)
                .collect()
        } else {
            Vec::new()
        };

        // `pinned_rules` is a TOP-LEVEL field, deliberately outside
        // `profile_json`. Everything inside that blob is a statistical
        // description the model may weigh against other evidence; these are
        // instructions it may not. Keeping them in separate fields is what
        // stops the distinction being lost on the way to the drafting agent.
        let payload = serde_json::json!({
            "source": source,
            "account_id": account_id,
            "profile_json": profile_json,
            "sample_count": sample_count,
            "generated_at": generated_at,
            "samples": samples,
            "pinned_rules": pinned
                .iter()
                .map(|r| serde_json::json!({
                    "id": r.id,
                    "scope": r.scope.as_str(),
                    "recipient_email": if r.recipient_email.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(r.recipient_email.clone()) },
                    "rule": r.rule,
                }))
                .collect::<Vec<_>>(),
            "pinned_rules_note": if pinned.is_empty() {
                "No pinned rules. profile_json below is a DESCRIPTION of past habits, not a set of instructions."
            } else {
                "ABSOLUTE instructions from the user. They OVERRIDE profile_json and the samples below, including where those describe the opposite habit. Where an account rule and a recipient rule disagree, the recipient one wins (listed later)."
            },
        });
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "extract_recipient_profile",
        description = "Build (or refresh) the per-recipient voice profile for a recipient by analyzing the user's prior sent mail to that address. This makes one call to the AI provider configured in CXMail and writes the result to the cache, after which get_voice_profile can return it. Use this once per recipient before drafting on the user's behalf. Returns the freshly built profile, or an error if fewer than 3 prior sent messages exist."
    )]
    async fn extract_recipient_profile(
        &self,
        Parameters(params): Parameters<ExtractRecipientProfileParams>,
    ) -> Result<CallToolResult, McpError> {
        let recipient = params.recipient_email.trim().to_lowercase();
        if recipient.is_empty() {
            return Err(McpError::invalid_params(
                "recipient_email is required",
                None,
            ));
        }

        let conn = self.open_db()?;
        let account_id = self.resolve_account_id(&conn, params.account_id.as_deref())?;
        let force = params.force.unwrap_or(false);
        let cached = db::voice_profiles_recipient::get(&conn, &account_id, &recipient)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;

        // Fresh cache hit + !force → no LLM call, return cached
        if !force {
            if let Some(row) = cached.as_ref() {
                let needs_refresh =
                    crate::email::voice::should_refresh_recipient(&conn, &account_id, &recipient)
                        .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
                if !needs_refresh {
                    let payload = serde_json::json!({
                        "status": "fresh",
                        "account_id": account_id,
                        "recipient_email": recipient,
                        "profile_json": row.profile_json,
                        "sample_count": row.sample_count,
                        "generated_at": row.generated_at,
                        "note": "Cache hit — no LLM call made. Pass force=true to rebuild.",
                    });
                    let text = serde_json::to_string_pretty(&payload)
                        .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
                    return Ok(CallToolResult::success(vec![Content::text(text)]));
                }
            }
        }

        // Incrementally group the available history, then select a balanced
        // set so older writing modes remain represented.
        let learning_set =
            crate::email::voice::build_recipient_learning_set(&conn, &account_id, &recipient)
                .map_err(|e| McpError::internal_error(format!("collect samples: {}", e), None))?;

        if learning_set.samples.len() < crate::email::voice::MIN_SAMPLES_REQUIRED {
            return Err(McpError::invalid_params(
                format!(
                    "Insufficient sent history for {}: found {} samples, need at least {}",
                    recipient,
                    learning_set.samples.len(),
                    crate::email::voice::MIN_SAMPLES_REQUIRED
                ),
                None,
            ));
        }

        let client = crate::email::inference::InferenceClient::load().map_err(|e| {
            McpError::invalid_params(
                format!("AI provider is not configured in CXMail: {e}"),
                None,
            )
        })?;

        // Extraction (no DB lock held — the MCP server opens a fresh Connection per call,
        // not the shared Mutex, so this isn't strictly necessary, but matches Tauri pattern)
        let outcome = crate::email::voice::extract_recipient_profile_from_learning_set(
            &client,
            &learning_set,
            cached.as_ref().map(|row| row.profile_json.as_str()),
        )
        .await
        .map_err(|e| McpError::internal_error(format!("extraction: {}", e), None))?;

        // Persist
        db::voice_profiles_recipient::upsert(
            &conn,
            &account_id,
            &recipient,
            &outcome.profile_json,
            &outcome.model_used,
            outcome.sample_count,
            &outcome.last_extracted_message_date,
        )
        .map_err(|e| McpError::internal_error(format!("upsert: {}", e), None))?;

        let row = db::voice_profiles_recipient::get(&conn, &account_id, &recipient)
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?
            .ok_or_else(|| {
                McpError::internal_error("Profile upsert succeeded but row not readable", None)
            })?;

        let payload = serde_json::json!({
            "status": "built",
            "account_id": account_id,
            "recipient_email": recipient,
            "profile_json": row.profile_json,
            "sample_count": row.sample_count,
            "style_bucket_count": learning_set.bucket_count,
            "generated_at": row.generated_at,
            "model_used": row.model_used,
        });
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| McpError::internal_error(format!("serialize: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // ─── Confirm tier ──────────────────────────────────────────

    #[tool(
        name = "compose_draft",
        description = "Create a draft email in the Drafts folder. Does NOT send — user reviews first. NOT for iteration: if you already created a draft for this email and the user asks for a change to it, call edit_draft on that draft instead — calling compose_draft a second time leaves the earlier draft behind as a duplicate and discards any edits the user made in the compose window. Supports an HTML body for formatting (set is_html=true whenever you write HTML tags) and optional file attachments via absolute paths inside the user's home directory. Optional `cc` and `bcc` recipient lists are supported (BCC recipients are hidden from other recipients). Plain prose plus simple markup (<strong>, <ul><li>, links, headings) is the norm — omit `layout` for it: it survives CXMail's compose editor fine and the draft stays fully editable. For a genuinely designed email you MUST set `layout`: \"rich\" for hand-built inline-styled HTML (the user can still retype text in place, but the structure is locked), \"card\" only for standalone marketing-style mail (in ordinary correspondence Gmail folds a card behind \"Show trimmed content\"). Omitting `layout` on designed HTML gets its tables, <div>s and inline styles FLATTENED when the user opens the draft in compose — only the text survives, as bare paragraphs — so build bullets as real <ul> lists, never as table rows. Unknown layout values are rejected. Styling MUST be inline (style attributes) — <style> blocks and CSS classes do not render in email. Verify the stored markup afterwards with preview_email_html. IMPORTANT: do NOT include a closing/sign-off (e.g., 'Best, Chris', 'Thanks,', 'Regards,') in the body — the user's saved signature is automatically appended below the styled block, and adding your own sign-off produces a visible duplicate. End the body with the last sentence of your message. REPLIES: pass `reply_to_folder` + `reply_to_uid` (both printed by read_email / search_emails / read_thread) — that is the default and most reliable way to thread and quote; `reply_to_message_id` is a fallback for when you only have the ID. The original is quoted automatically BELOW your signature, Gmail-style — write only the new reply prose and NEVER paste thread history into `body` (doing so puts the signature below the quote). quote_original=false opts out. If the quote cannot be built this tool returns an error naming the reason instead of silently creating a quote-less draft. WRITING DELEGATION: pass `instruction` instead of `body` to have a local model write the prose (Antigravity CLI, default gemini-3.7-flash-high) in the user's voice, on-device; the generated text is echoed back in the result for review."
    )]
    async fn compose_draft(
        &self,
        Parameters(mut params): Parameters<ComposeDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        // Before any IMAP work: an unknown layout is a caller error, not a
        // reason to silently produce a plain draft.
        validate_layout(params.layout.as_deref())?;
        // Exactly one author: the caller's `body`, or the external writer
        // driven by `instruction`. Resolved before anything else — a caller
        // error must not reach IMAP (#30's ordering rule), and a failed
        // generation writes nothing anywhere.
        let source = resolve_authored_source(
            params.body.take(),
            params.instruction.take(),
            params.writer_model.take(),
            params.layout.as_deref(),
            params.is_html,
            &crate::email::external_writer::effective_default_model(),
        )?;
        let (mut body, writer_note) = match source {
            AuthoredSource::Explicit(body) => (body, String::new()),
            AuthoredSource::Generated { instruction, model } => {
                // Generated output is plain prose by contract.
                params.is_html = Some(false);
                let generated = self
                    .generate_draft_body(
                        &params.account_id,
                        &params.to,
                        &params.subject,
                        &instruction,
                        &model,
                        params.reply_to_folder.as_deref(),
                        params.reply_to_uid,
                        None,
                    )
                    .await?;
                let note = format!(
                    "\n✎ Body written by {model} from your instruction — review it below \
                     (and in the draft) before any send; use edit_draft to fix anything:\n\
                     ---\n{generated}\n---"
                );
                (generated, note)
            }
        };
        // House dash rule, on the authored text only and before it is rendered,
        // signed, quoted, or appended anywhere — the external writer's output
        // included (#47: normalize at the boundary, whoever wrote the words).
        let dash_note = enforce_dash_rule(&mut params.subject, &mut body);
        let layout_note = layout_advisory(params.layout.as_deref(), &body);

        // `Connection` is `Send` (only `&Connection` isn't), so holding this
        // across the await below keeps the tool future `Send`.
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;
        let pinned_note = pinned_rule_advisory(&self.pinned_rules_for_draft(
            &conn,
            &params.account_id,
            &params.to,
        ));

        let target = resolve_reply_target(
            params.reply_to_folder.as_deref(),
            params.reply_to_uid,
            params.reply_to_message_id.as_deref(),
        )?;
        validate_reply_threading(&params.subject, &target)?;

        // The From, resolved before any IMAP work: an unconfigured address is
        // a caller error and must not leave a draft behind, and a reply
        // defaults to the address the original was addressed to.
        let from_addr = resolve_draft_from(
            &conn,
            &account,
            params.from.as_deref(),
            target.as_ref(),
            // A fresh draft has no previous revision to inherit a From from.
            None,
        )?;
        let from_note = send_as_note(&from_addr, params.from.is_some());
        let signature = signature_for_send_as(&conn, &params.account_id, &from_addr);

        // Threading + quoted history (ComposeModal order: body + signature +
        // quote). Errors here land before the APPEND below, so a failed quote
        // never leaves a stray draft.
        let reply = self
            .build_reply_context(
                &params.account_id,
                &account,
                target,
                params.quote_original,
                &body,
            )
            .await?;
        let ReplyContext {
            in_reply_to,
            references,
            quote: quoted,
            note: quote_note,
        } = reply;

        let mut html_body = match params.layout.as_deref() {
            Some("card") => render_styled(&body, params.is_html, "card"),
            Some("rich") => render_styled(&body, params.is_html, "rich"),
            // Omitted → ordinary editable draft. Anything else was already
            // rejected by `validate_layout` at the top of this handler.
            _ => render_body(&body, params.is_html),
        };
        if let Some(sig) = signature {
            html_body = append_signature(&html_body, &sig);
        }
        if let Some((ref quote_html, _)) = quoted {
            html_body.push_str(quote_html);
        }

        let attachment_paths = params.attachments.unwrap_or_default();
        let attachments_count = attachment_paths.len();
        let attachments = read_attachment_paths(&attachment_paths)?;

        // Plain email strings → recipient list (no display name from MCP params).
        let to_recipients = |addrs: &[String]| -> Vec<smtp::EmailRecipient> {
            addrs
                .iter()
                .map(|e| smtp::EmailRecipient {
                    name: None,
                    email: e.clone(),
                })
                .collect()
        };

        let email = smtp::OutgoingEmail {
            from_email: from_addr.email.clone(),
            from_name: from_addr.display_name.clone(),
            to: to_recipients(&params.to),
            cc: to_recipients(&params.cc.unwrap_or_default()),
            bcc: to_recipients(&params.bcc.unwrap_or_default()),
            subject: params.subject,
            html_body,
            plain_body: Some(match quoted {
                Some((_, ref plain_quote)) if !plain_quote.is_empty() => {
                    format!("{}{}", body, plain_quote)
                }
                _ => body.clone(),
            }),
            in_reply_to,
            references,
            track_opens: None,
            attachments: vec![],
        };

        // Synced folder list first (the only thing that knows a generic
        // server's layout), historical hardcoded name as the fallback for the
        // pre-folder-sync window. Resolved here, while `conn` is alive and
        // before any await.
        let drafts_folder = {
            let conn = self.open_db()?;
            db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts")
        };
        let drafts_folder = drafts_folder.as_str();

        // Generated Message-ID lets us locate the new UID after APPEND (UID
        // SEARCH HEADER Message-ID) so we can write a local cache row that
        // search_emails will find immediately.
        let message_id_bare = format!("{}@cxmail.app", uuid::Uuid::new_v4());
        let message_id_header = format!("<{}>", message_id_bare);
        // First save mints the draft's stable identity (v60, `db::drafts`).
        let draft_id = uuid::Uuid::new_v4().to_string();

        let mut message = mail_builder::MessageBuilder::new();
        message = message.message_id(message_id_bare.as_str());
        message = message.header(
            crate::email::draft_local::DRAFT_ID_HEADER,
            mail_builder::headers::raw::Raw::new(draft_id.as_str()),
        );
        // The display name rides along when the address has one — the app's
        // own draft path has always written it, and a draft whose From differs
        // from the message that will be sent is its own small lie.
        match email.from_name.as_deref() {
            Some(name) => message = message.from((name, email.from_email.as_str())),
            None => message = message.from(email.from_email.as_str()),
        }
        // One `.to()` / `.cc()` / `.bcc()` call per field — see `smtp::to_address_list`.
        // mail-builder 0.3.2 pushes a fresh header per call, so a per-recipient
        // loop would emit multiple `To:` lines.
        if !email.to.is_empty() {
            message = message.to(smtp::to_address_list(&email.to));
        }
        if !email.cc.is_empty() {
            message = message.cc(smtp::to_address_list(&email.cc));
        }
        if !email.bcc.is_empty() {
            message = message.bcc(smtp::to_address_list(&email.bcc));
        }
        message = message.subject(&email.subject);
        let styled_html = crate::email::inline_styles::apply_inline_font_styles(&email.html_body);
        message = message.html_body(&styled_html);
        if let Some(ref reply_to) = email.in_reply_to {
            // Pass the bare ID — mail-builder re-wraps in `<` `>`.
            message = message.in_reply_to(smtp::strip_message_id_brackets(reply_to));
        }
        if let Some(ref refs) = email.references {
            message = message.header(
                "References",
                mail_builder::headers::raw::Raw::new(refs.as_str()),
            );
        }
        for (filename, content_type, data) in &attachments {
            message = message.attachment(content_type.as_str(), filename.as_str(), data.clone());
        }

        let raw = message
            .write_to_string()
            .map_err(|e| McpError::internal_error(format!("Build draft failed: {}", e), None))?;

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;
        // The drafts row records the UIDVALIDITY the UID was issued under.
        let (uidvalidity, _) = imap::select_folder(&mut session, drafts_folder)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        // `\Seen \Draft` matches the Tauri save_draft path so drafts don't
        // show as unread and the RFC 3501 \Draft marker is set.
        session
            .append(
                drafts_folder,
                Some("(\\Seen \\Draft)"),
                None,
                raw.as_bytes(),
            )
            .await
            .map_err(|e| McpError::internal_error(format!("APPEND failed: {}", e), None))?;

        let uid = imap::find_uid_by_message_id(&mut session, drafts_folder, &message_id_header)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("UID lookup after APPEND failed: {}", e), None)
            })?;
        let _ = imap::disconnect(session, &account.email).await;

        // Persist locally so MCP search_emails finds the draft immediately.
        // Tantivy is not written here — the MCP server is a separate process
        // and can't safely open a second Tantivy writer. The running Tauri
        // app's next Drafts sync (or its startup drift-backfill) will index it.
        if let Err(e) = crate::email::draft_local::persist_local_draft(
            &conn,
            &params.account_id,
            drafts_folder,
            uid,
            raw.as_bytes(),
        ) {
            log::warn!(
                "MCP compose_draft: local cache write failed for UID {}: {}",
                uid, e
            );
        }
        if let Err(e) = db::drafts::link(
            &conn,
            &draft_id,
            &params.account_id,
            drafts_folder,
            uidvalidity,
            uid,
        ) {
            log::warn!(
                "MCP compose_draft: drafts row for {} not written: {}",
                draft_id, e
            );
        }

        // Live-notify the running app so the Drafts list refreshes without a
        // manual reload. Best-effort: a no-op when the app isn't running.
        notify(
            &Envelope::new("draft-created", &params.account_id, drafts_folder, uid)
                .with_tool("compose_draft"),
        )
        .await;

        let suffix = if attachments_count == 0 {
            String::new()
        } else if attachments_count == 1 {
            " with 1 attachment".to_string()
        } else {
            format!(" with {} attachments", attachments_count)
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Draft saved to {} (UID {}) for {}{} — draft_id {} (address later edits by draft_id; the UID changes on every save){}{}{}{}{}{}",
            drafts_folder, uid, account.email, suffix, draft_id, quote_note, layout_note,
            dash_note, pinned_note, writer_note, from_note
        ))]))
    }

    #[tool(
        name = "edit_draft",
        description = "ITERATING ON A DRAFT: always use this tool — never call compose_draft a second time for an email you already drafted, which leaves the earlier draft behind as a duplicate. Before writing the new body, re-read the draft as it stands NOW (preview_email_html for designed HTML, read_email for prose) and build your change on top of THAT text — the user may have edited it in the compose window since you wrote it, and rebuilding from your own previous body silently discards their edits. UIDs are NOT stable across edits: this tool deletes and re-appends (the result line prints the new UID), and the user saving in compose does the same, so a UID you are holding from an earlier call may already be gone — if it is not found, search the Drafts folder by subject, take the newest UID, and confirm it with preview_email_html (a stale local search row fails that fetch). Replace an existing draft with an updated version. Deletes the old draft and creates a new one. Supports an HTML body for formatting (set is_html=true whenever you write HTML tags) and optional file attachments (replaces any prior attachments). Optional `cc` and `bcc` recipient lists are supported (BCC recipients are hidden from other recipients); pass the full intended cc/bcc each call since this replaces the draft. Plain prose plus simple markup (<strong>, <ul><li>, links, headings) is the norm — omit `layout` for it: it survives CXMail's compose editor fine and the draft stays fully editable. For a genuinely designed email you MUST set `layout`: \"rich\" for hand-built inline-styled HTML (the user can still retype text in place, but the structure is locked), \"card\" only for standalone marketing-style mail (in ordinary correspondence Gmail folds a card behind \"Show trimmed content\"). Omitting `layout` on designed HTML gets its tables, <div>s and inline styles FLATTENED when the user opens the draft in compose — only the text survives, as bare paragraphs — so build bullets as real <ul> lists, never as table rows. Unknown layout values are rejected. Styling MUST be inline (style attributes) — <style> blocks and CSS classes do not render in email. Verify the stored markup afterwards with preview_email_html. IMPORTANT: do NOT include a closing/sign-off (e.g., 'Best, Chris', 'Thanks,', 'Regards,') in the body — the user's saved signature is automatically appended below the styled block. End the body with the last sentence of your message. REPLIES: pass `reply_to_folder` + `reply_to_uid` (both printed by read_email / search_emails / read_thread) — that is the default and most reliable way to thread and quote; `reply_to_message_id` is a fallback for when you only have the ID. The original is quoted automatically BELOW your signature, Gmail-style — write only the new reply prose and NEVER paste thread history into `body` (doing so puts the signature below the quote). quote_original=false opts out. If the quote cannot be built this tool returns an error naming the reason, leaving the existing draft untouched. WRITING DELEGATION: pass `instruction` instead of `body` to have a local model write the prose (Antigravity CLI, default gemini-3.7-flash-high) in the user's voice, on-device; the generated text is echoed back in the result for review."
    )]
    async fn edit_draft(
        &self,
        Parameters(mut params): Parameters<EditDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        // Addressing first: a malformed target is the cheapest thing to
        // refuse, and nothing below may run without knowing the revision.
        let edit_target =
            resolve_edit_target(params.draft_id.as_deref(), params.expected_uid, params.uid)?;
        // MUST stay above the APPEND below — an unknown layout has to fail
        // before anything reaches the server, not after (gotchas #30, #57).
        validate_layout(params.layout.as_deref())?;
        // Exactly one author (see compose_draft). For a generated rewrite, the
        // draft's CURRENT cached body is the text being rewritten, resolved
        // read-only here — the claim further down still arbitrates the actual
        // write, so a racing autosave costs the caller a `draft_moved_error`,
        // never a lost edit.
        let source = resolve_authored_source(
            params.body.take(),
            params.instruction.take(),
            params.writer_model.take(),
            params.layout.as_deref(),
            params.is_html,
            &crate::email::external_writer::effective_default_model(),
        )?;
        let (mut body, writer_note) = match source {
            AuthoredSource::Explicit(body) => (body, String::new()),
            AuthoredSource::Generated { instruction, model } => {
                params.is_html = Some(false);
                let rewrite_source = {
                    let conn = self.open_db()?;
                    let account = self.get_account(&conn, &params.account_id)?;
                    let drafts_folder = db::folders::folder_for_account(
                        &conn,
                        &account.id,
                        &account.provider,
                        "drafts",
                    );
                    let (folder, uid) = match &edit_target {
                        EditTarget::ById {
                            draft_id,
                            expected_uid,
                        } => {
                            let row = db::drafts::get(&conn, draft_id)
                                .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                                .ok_or_else(|| unknown_draft_error(draft_id))?;
                            (
                                row.folder_name.clone(),
                                expected_uid.unwrap_or(row.current_uid),
                            )
                        }
                        EditTarget::ByUid(uid) => (drafts_folder, *uid),
                    };
                    db::messages::get_body(&conn, &params.account_id, &folder, uid)
                        .ok()
                        .flatten()
                        .and_then(|b| b.plain_text)
                        .filter(|t| !t.trim().is_empty())
                };
                let Some(rewrite_source) = rewrite_source else {
                    return Err(McpError::invalid_params(
                        "This draft's current body is not in the local cache, so there is \
                         nothing for the external writer to rewrite — nothing was changed. \
                         Read the draft (read_email / preview_email_html) and pass an \
                         explicit `body` instead."
                            .to_string(),
                        None,
                    ));
                };
                let generated = self
                    .generate_draft_body(
                        &params.account_id,
                        &params.to,
                        &params.subject,
                        &instruction,
                        &model,
                        params.reply_to_folder.as_deref(),
                        params.reply_to_uid,
                        Some(rewrite_source),
                    )
                    .await?;
                let note = format!(
                    "\n✎ Body rewritten by {model} from your instruction — review it below \
                     (and in the draft) before any send:\n---\n{generated}\n---"
                );
                (generated, note)
            }
        };
        // Same rule: the authored text is normalized before any IMAP work —
        // the external writer's output included (#47).
        let dash_note = enforce_dash_rule(&mut params.subject, &mut body);
        let layout_note = layout_advisory(params.layout.as_deref(), &body);

        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;
        let pinned_note = pinned_rule_advisory(&self.pinned_rules_for_draft(
            &conn,
            &params.account_id,
            &params.to,
        ));

        let target = resolve_reply_target(
            params.reply_to_folder.as_deref(),
            params.reply_to_uid,
            params.reply_to_message_id.as_deref(),
        )?;
        // Same rule as validate_layout above: reject before any IMAP write.
        validate_reply_threading(&params.subject, &target)?;

        // The From, resolved before the v60 claim and every IMAP call: an
        // address the account cannot send as is a caller error and must not
        // cost the draft its claim, let alone reach the server. An edit that
        // names no `from` keeps the revision's existing one — a rewrite is not
        // a request to change the sender.
        let from_addr = resolve_draft_from(
            &conn,
            &account,
            params.from.as_deref(),
            target.as_ref(),
            current_draft_from_address(&conn, &account, &edit_target).as_deref(),
        )?;
        let from_note = send_as_note(&from_addr, params.from.is_some());

        // Threading + quoted history, built before the drafts-folder session
        // opens (the IMAP fallback runs its own short-lived connection).
        // ORDERING: this must stay above the APPEND below — a failure after
        // the new revision is on the server leaves a duplicate draft (and
        // before 2026-08-25, when the old one was expunged first, it lost the
        // draft outright — gotcha #57).
        let ReplyContext {
            in_reply_to,
            references,
            quote: quoted,
            note: quote_note,
        } = self
            .build_reply_context(
                &params.account_id,
                &account,
                target,
                params.quote_original,
                &body,
            )
            .await?;

        let attachment_paths = params.attachments.unwrap_or_default();
        let attachments_count = attachment_paths.len();
        let attachments = read_attachment_paths(&attachment_paths)?;

        // Synced folder list first (the only thing that knows a generic
        // server's layout), historical hardcoded name as the fallback for the
        // pre-folder-sync window. Resolved here, while `conn` is alive and
        // before any await.
        let drafts_folder = {
            let conn = self.open_db()?;
            db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts")
        };
        let drafts_folder = drafts_folder.as_str();

        // ── v60: identity + claim, before any IMAP work ─────────────────
        // Resolve the caller's target to the logical draft, check the revision
        // it named, and take the write claim (a compare-and-set on
        // `current_uid`, `db::drafts::claim`). Nothing below touches the
        // server unless this succeeded, and any failure past it releases the
        // claim. The preflight further down stays: the DB row says what WE
        // believe is current; the server says whether it is still there.
        let (row, expected_uid, addressed_by_uid) = match &edit_target {
            EditTarget::ById {
                draft_id,
                expected_uid,
            } => {
                let row = db::drafts::get(&conn, draft_id)
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                    .ok_or_else(|| unknown_draft_error(draft_id))?;
                if row.account_id != params.account_id {
                    return Err(McpError::invalid_params(
                        format!(
                            "Draft {} belongs to account {}, not {} — nothing was changed.",
                            draft_id, row.account_id, params.account_id
                        ),
                        None,
                    ));
                }
                let expected = expected_uid.unwrap_or(row.current_uid);
                (Some(row), expected, false)
            }
            EditTarget::ByUid(uid) => {
                let row = db::drafts::find_by_uid(&conn, &params.account_id, drafts_folder, *uid)
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
                (row, *uid, true)
            }
        };
        let old_uid = expected_uid;
        let claim_token = uuid::Uuid::new_v4().to_string();
        let (draft_id, old_folder, row_uidvalidity, claimed) = match row {
            Some(row) => {
                if expected_uid != row.current_uid {
                    return Err(draft_moved_error(&conn, &row, expected_uid, row.current_uid));
                }
                match db::drafts::claim(&conn, &row.draft_id, expected_uid, &claim_token)
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                {
                    db::drafts::ClaimOutcome::Claimed => (
                        row.draft_id.clone(),
                        row.folder_name.clone(),
                        Some(row.uidvalidity),
                        true,
                    ),
                    db::drafts::ClaimOutcome::Moved { current_uid } => {
                        return Err(draft_moved_error(&conn, &row, expected_uid, current_uid));
                    }
                    db::drafts::ClaimOutcome::Busy { expires_at } => {
                        return Err(draft_busy_error(&row.draft_id, &expires_at));
                    }
                    // Vanished between lookup and claim: proceed unclaimed, the
                    // way a pre-v60 draft does, and re-create the row on success.
                    db::drafts::ClaimOutcome::Missing => {
                        (row.draft_id.clone(), row.folder_name.clone(), None, false)
                    }
                }
            }
            // No row: a draft saved before v60, or by another client. The IMAP
            // preflight is the only check there is; it gets a row on success.
            None => (
                uuid::Uuid::new_v4().to_string(),
                drafts_folder.to_string(),
                None,
                false,
            ),
        };
        // Everything the IMAP section needs from the DB, read here: the
        // handler future must stay `Send`, and `&Connection` is not.
        let local_row_present =
            db::messages::get_by_uid(&conn, &params.account_id, &old_folder, old_uid)
                .ok()
                .flatten()
                .is_some();
        let signature = signature_for_send_as(&conn, &params.account_id, &from_addr);

        // ── IMAP — releases the claim on any failure ────────────────────
        let imap_outcome: Result<(u32, u32, Result<(), String>, String), McpError> = async {
            let mut session = imap::connect_for_account(&account)
                .await
                .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;

            // PREFLIGHT — strictly before the APPEND. `UID STORE` on a UID the
            // server no longer holds is silently ignored (RFC 9051 §6.4.9), so
            // without this a stale reference sailed through the delete and the
            // APPEND minted a SECOND draft (gotcha #57). The claim above is the
            // exclusion between CXMail's own writers; this is the check against
            // the world outside them — a delete in another client, a mailbox
            // that changed generation under us.
            let (old_generation, _) = imap::select_folder(&mut session, &old_folder)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            if let Some(recorded) = row_uidvalidity {
                if recorded != old_generation {
                    let _ = imap::disconnect(session, &account.email).await;
                    return Err(McpError::invalid_params(
                        format!(
                            "Draft {}: mailbox {} has a new UIDVALIDITY ({} → {}), so its recorded revision (UID {}) cannot be trusted — nothing was changed. Let the Drafts folder sync re-link it, then find it with search_emails and address it by uid once.",
                            draft_id, old_folder, recorded, old_generation, old_uid
                        ),
                        None,
                    ));
                }
            }
            let on_server = imap::uid_exists(&mut session, &old_folder, old_uid)
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("Draft preflight failed: {}", e), None)
                })?;
            if !on_server {
                let _ = imap::disconnect(session, &account.email).await;
                return Err(stale_draft_error(
                    old_uid,
                    &old_folder,
                    &account.email,
                    local_row_present,
                ));
            }

            // Build the new revision — nothing on the server has been touched yet.
            let mut html_body = match params.layout.as_deref() {
                Some("card") => render_styled(&body, params.is_html, "card"),
                Some("rich") => render_styled(&body, params.is_html, "rich"),
                // Omitted → ordinary editable draft. Anything else was already
                // rejected by `validate_layout` at the top of this handler.
                _ => render_body(&body, params.is_html),
            };
            if let Some(ref sig) = signature {
                html_body = append_signature(&html_body, sig);
            }
            if let Some((ref quote_html, _)) = quoted {
                html_body.push_str(quote_html);
            }
            let styled_html = crate::email::inline_styles::apply_inline_font_styles(&html_body);

            let message_id_bare = format!("{}@cxmail.app", uuid::Uuid::new_v4());
            let message_id_header = format!("<{}>", message_id_bare);

            // Plain email strings → recipient lists (no display name from MCP params).
            let to_recipients = |addrs: &[String]| -> Vec<smtp::EmailRecipient> {
                addrs
                    .iter()
                    .map(|e| smtp::EmailRecipient {
                        name: None,
                        email: e.clone(),
                    })
                    .collect()
            };
            let to_rcpts = to_recipients(&params.to);
            let cc_rcpts = to_recipients(&params.cc.clone().unwrap_or_default());
            let bcc_rcpts = to_recipients(&params.bcc.clone().unwrap_or_default());

            let mut message = mail_builder::MessageBuilder::new();
            message = message.message_id(message_id_bare.as_str());
            message = message.header(
                crate::email::draft_local::DRAFT_ID_HEADER,
                mail_builder::headers::raw::Raw::new(draft_id.as_str()),
            );
            // The display name rides along when the address has one, matching
            // the app's own draft path — a draft whose From differs from what
            // the send will put on the wire is its own small lie.
            match from_addr.display_name.as_deref() {
                Some(name) => message = message.from((name, from_addr.email.as_str())),
                None => message = message.from(from_addr.email.as_str()),
            }
            // One `.to()` / `.cc()` / `.bcc()` call per field — see `smtp::to_address_list`.
            if !to_rcpts.is_empty() {
                message = message.to(smtp::to_address_list(&to_rcpts));
            }
            if !cc_rcpts.is_empty() {
                message = message.cc(smtp::to_address_list(&cc_rcpts));
            }
            if !bcc_rcpts.is_empty() {
                message = message.bcc(smtp::to_address_list(&bcc_rcpts));
            }
            message = message.subject(&params.subject);
            message = message.html_body(&styled_html);
            if let Some(ref reply_to) = in_reply_to {
                // Pass the bare ID — mail-builder re-wraps in `<` `>`.
                message = message.in_reply_to(smtp::strip_message_id_brackets(reply_to));
            }
            if let Some(ref refs) = references {
                message = message.header(
                    "References",
                    mail_builder::headers::raw::Raw::new(refs.as_str()),
                );
            }
            for (filename, content_type, data) in &attachments {
                message = message.attachment(content_type.as_str(), filename.as_str(), data.clone());
            }

            let raw = message
                .write_to_string()
                .map_err(|e| McpError::internal_error(format!("Build draft failed: {}", e), None))?;


            // APPEND FIRST, delete after — the same order as the app's
            // `commands::compose::edit_draft`. Until 2026-08-25 this handler
            // expunged the old draft and only then built and APPENDed the new
            // one, so any failure in between destroyed the user's draft
            // outright. Now a failure past this point leaves a duplicate —
            // visible, recoverable, and reported in the tool result below.
            let (uidvalidity, _) = imap::select_folder(&mut session, drafts_folder)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            session
                .append(
                    drafts_folder,
                    Some("(\\Seen \\Draft)"),
                    None,
                    raw.as_bytes(),
                )
                .await
                .map_err(|e| McpError::internal_error(format!("APPEND failed: {}", e), None))?;

            let new_uid =
                imap::find_uid_by_message_id(&mut session, drafts_folder, &message_id_header)
                    .await
                    .map_err(|e| {
                        McpError::internal_error(
                            format!("UID lookup after APPEND failed: {}", e),
                            None,
                        )
                    })?;

            // Retire the old revision, in the folder it lived in. `UID EXPUNGE`
            // removes exactly this UID; a mailbox-wide EXPUNGE would also purge
            // anything another client had marked `\\Deleted` there.
            let old_removed: Result<(), String> =
                match imap::select_folder(&mut session, &old_folder).await {
                    Ok(_) => match imap::store_flags(&mut session, old_uid, true, "\\Deleted").await
                    {
                        Ok(()) => imap::uid_expunge(&mut session, old_uid)
                            .await
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => Err(e.to_string()),
                };
            let _ = imap::disconnect(session, &account.email).await;
            Ok((new_uid, uidvalidity, old_removed, raw))
        }
        .await;
        let (new_uid, uidvalidity, old_removed, raw) = match imap_outcome {
            Ok(v) => v,
            Err(e) => {
                if claimed {
                    if let Err(re) = db::drafts::release(&conn, &draft_id, &claim_token) {
                        log::warn!("MCP edit_draft: releasing claim on {} failed: {}", draft_id, re);
                    }
                }
                return Err(e);
            }
        };

        // Point the draft at its new revision (and drop our claim). Forward-only
        // within the generation, so a writer that outlived an expired claim
        // cannot drag the pointer backwards.
        let pointed = if claimed {
            db::drafts::advance(&conn, &draft_id, &claim_token, uidvalidity, new_uid)
        } else {
            db::drafts::link(
                &conn,
                &draft_id,
                &params.account_id,
                drafts_folder,
                uidvalidity,
                new_uid,
            )
        };
        if let Err(e) = pointed {
            log::warn!(
                "MCP edit_draft: drafts row for {} not advanced to UID {}: {}",
                draft_id, new_uid, e
            );
        }

        // Evict the old UID from the local cache (FK ON DELETE CASCADE clears
        // message_bodies/attachments) — but only if the server actually let it
        // go, so the cache never claims a draft is gone while it still exists.
        // The FTS triggers index the new row; nothing search-side to call.
        let dup_note = match &old_removed {
            Ok(()) => {
                if let Err(e) =
                    db::messages::delete_uids(&conn, &params.account_id, &old_folder, &[old_uid])
                {
                    log::warn!(
                        "MCP edit_draft: local DB delete of old UID {} failed: {}",
                        old_uid, e
                    );
                }
                String::new()
            }
            Err(reason) => {
                log::warn!(
                    "MCP edit_draft: new revision UID {} is on the server but the old UID {} was not removed: {}",
                    new_uid, old_uid, reason
                );
                format!(
                    "\n\n⚠ The previous revision (UID {}) could not be removed: {}. Both revisions are now in {} — once the new draft looks right, delete UID {} with delete_email.",
                    old_uid, reason, old_folder, old_uid
                )
            }
        };
        if let Err(e) = crate::email::draft_local::persist_local_draft(
            &conn,
            &params.account_id,
            drafts_folder,
            new_uid,
            raw.as_bytes(),
        ) {
            log::warn!(
                "MCP edit_draft: local cache write for new UID {} failed: {}",
                new_uid, e
            );
        }

        // Live-notify the app. `old_uid` lets an open compose modal — which only
        // knows its current UID — recognize that this is its draft and adopt the
        // fresh `new_uid` (edit_draft expunged the old one and minted a new one).
        notify(
            &Envelope::new("draft-updated", &params.account_id, drafts_folder, new_uid)
                .with_old_uid(old_uid)
                .with_tool("edit_draft"),
        )
        .await;

        let suffix = if attachments_count == 0 {
            String::new()
        } else if attachments_count == 1 {
            " with 1 attachment".to_string()
        } else {
            format!(" with {} attachments", attachments_count)
        };
        let deprecation_note = if addressed_by_uid {
            format!(
                "\n\nℹ `uid` is deprecated for edit_draft — address this draft as draft_id \"{}\" from now on. A UID names one revision and goes stale on every save; the draft_id does not.",
                draft_id
            )
        } else {
            String::new()
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Draft UID {} replaced with UID {} in {} for {}{} — draft_id {}{}{}{}{}{}{}{}{}",
            old_uid,
            new_uid,
            drafts_folder,
            account.email,
            suffix,
            draft_id,
            dup_note,
            deprecation_note,
            quote_note,
            layout_note,
            dash_note,
            pinned_note,
            writer_note,
            from_note
        ))]))
    }

    #[tool(
        name = "move_email",
        description = "Move an email from one folder to another."
    )]
    async fn move_email(
        &self,
        Parameters(params): Parameters<MoveEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.from_folder).await;
        imap::move_message(&mut session, params.uid, &params.to_folder)
            .await
            .map_err(|e| McpError::internal_error(format!("Move failed: {}", e), None))?;
        let _ = imap::disconnect(session, &account.email).await;

        // The row leaves `from_folder`; notify so any list showing it refreshes.
        notify(
            &Envelope::new("email-mutated", &params.account_id, &params.from_folder, params.uid)
                .with_tool("move_email"),
        )
        .await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Moved UID {} from {} to {}",
            params.uid, params.from_folder, params.to_folder
        ))]))
    }

    #[tool(
        name = "archive_email",
        description = "Archive an email (move to archive folder)."
    )]
    async fn archive_email(
        &self,
        Parameters(params): Parameters<ReadEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;
        let archive =
            db::folders::folder_for_account(&conn, &account.id, &account.provider, "archive");
        let archive = archive.as_str();

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.folder).await;
        imap::move_message(&mut session, params.uid, archive)
            .await
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        let _ = imap::disconnect(session, &account.email).await;

        notify(
            &Envelope::new("email-mutated", &params.account_id, &params.folder, params.uid)
                .with_tool("archive_email"),
        )
        .await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Archived UID {} from {}",
            params.uid, params.folder
        ))]))
    }

    #[tool(
        name = "flag_email",
        description = "Set or remove a flag on an email (starred/unstarred/read/unread)."
    )]
    async fn flag_email(
        &self,
        Parameters(params): Parameters<FlagEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let (add, flag) = match params.flag.as_str() {
            "starred" => (true, "\\Flagged"),
            "unstarred" => (false, "\\Flagged"),
            "read" => (true, "\\Seen"),
            "unread" => (false, "\\Seen"),
            _ => {
                return Err(McpError::invalid_params(
                    "Invalid flag. Use: starred, unstarred, read, unread",
                    None,
                ))
            }
        };

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.folder).await;
        imap::store_flags(&mut session, params.uid, add, flag)
            .await
            .map_err(|e| McpError::internal_error(format!("{}", e), None))?;
        let _ = imap::disconnect(session, &account.email).await;

        // The row persists (only its flag/read state changed), so the list
        // refresh + row flash both apply.
        notify(
            &Envelope::new("email-mutated", &params.account_id, &params.folder, params.uid)
                .with_tool("flag_email"),
        )
        .await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Set {} on UID {}",
            params.flag, params.uid
        ))]))
    }

    #[tool(
        name = "set_open_tracking",
        description = "Read or set per-account open tracking (pixel injection on sent mail). Omit `enabled` to read the current state. IMPORTANT: keep this OFF for accounts used for plain-text outreach — the pixel turns plain mail into image-only HTML and trips spam filters (SpamAssassin HTML_IMAGE_ONLY)."
    )]
    async fn set_open_tracking(
        &self,
        Parameters(params): Parameters<SetOpenTrackingParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;

        // Pre-v41 guard BEFORE any account read: this binary's startup migration
        // is warn-and-continue, so the DB may predate the column that
        // resolve_account_id/get_account now SELECT. Fail with a recovery hint.
        let has_column: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'track_opens_enabled')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        if !has_column {
            return Err(McpError::internal_error(
                "Database schema is older than this MCP build (accounts.track_opens_enabled missing). Launch CXMail once or restart the MCP session to run migrations, then retry.".to_string(),
                None,
            ));
        }

        let account_id = self.resolve_account_id(&conn, params.account_id.as_deref())?;
        let account = self.get_account(&conn, &account_id)?;

        if let Some(enabled) = params.enabled {
            db::accounts::set_track_opens_enabled(&conn, &account.id, enabled)
                .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Open tracking {} for {} ({})",
                if enabled { "ENABLED" } else { "DISABLED" },
                account.email,
                account.id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Open tracking is {} for {} ({})",
                if account.track_opens_enabled { "ENABLED" } else { "DISABLED" },
                account.email,
                account.id
            ))]))
        }
    }

    // ─── Google Calendar ───────────────────────────────────────

    #[tool(
        name = "list_calendar_events",
        description = "List CXMail's local unified calendar mirror for an ISO-8601 range. Google events include live RSVP state, Meet links, pending-notification state, and conflict state."
    )]
    async fn list_calendar_events(
        &self,
        Parameters(params): Parameters<ListCalendarEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let events = db::gcal::list_unified_by_range(
            &conn,
            params.account_id.as_deref(),
            &params.start,
            &params.end,
        )
        .map_err(|e| McpError::internal_error(format!("Calendar query failed: {e}"), None))?;
        let text = serde_json::to_string_pretty(&events)
            .map_err(|e| McpError::internal_error(format!("Serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "create_calendar_event",
        description = "Create a real Google Calendar event, optionally backed by a Google Meet (default) or Zoom conference. The event is created with sendUpdates=none: attendees are recorded but NOT notified. Returns the complete attendee list and pending_notify=true; use send_calendar_invites only after human review. Pass conference=\"zoom\" when the meeting needs computer audio in a screen share — Google Meet cannot carry system audio from macOS."
    )]
    async fn create_calendar_event(
        &self,
        Parameters(params): Parameters<CreateCalendarEventParams>,
    ) -> Result<CallToolResult, McpError> {
        // ── Everything pure and refusable happens BEFORE the first DB write. ──
        // This used to `upsert_calendar` (three writes) before validating the
        // attendee list, so a rejected call still mutated the mirror.
        let summary = params.summary.trim();
        if summary.is_empty() {
            return Err(McpError::invalid_params("summary is required", None));
        }
        let conference = validate_conference(
            params.conference.as_deref(),
            params.add_meet,
            // An all-day event is expressed by date-only bounds.
            params.start.trim().len() == 10 || params.end.trim().len() == 10,
            // There is no recurrence parameter on this tool; the guard exists so
            // adding one cannot silently bypass the Zoom refusal.
            false,
        )?;
        let attendees = crate::email::event_input::validate_attendees(
            params.attendees.as_deref().unwrap_or_default(),
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let start = crate::email::event_input::event_datetime(&params.start, &params.time_zone)
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let end = crate::email::event_input::event_datetime(&params.end, &params.time_zone)
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        // Zoom's duration is `end − start`, and a negative one must never leave
        // the machine — it is accepted by Google (which stores whatever it is
        // told) and then produces a nonsense meeting.
        let (start_instant, end_instant) = timed_bounds(&start, &end)?;
        if let (Some(start_instant), Some(end_instant)) = (start_instant, end_instant) {
            if end_instant <= start_instant {
                return Err(McpError::invalid_params(
                    format!(
                        "end ({}) is not after start ({}). An event with a zero or negative length \
                         would be created with a nonsense duration on both Google and Zoom.",
                        params.end.trim(),
                        params.start.trim()
                    ),
                    None,
                ));
            }
        }

        let account = {
            let conn = self.open_db()?;
            self.get_account(&conn, &params.account_id)?
        };
        if account.provider != "gmail" {
            return Err(McpError::invalid_params(
                "Google Calendar requires a Gmail account",
                None,
            ));
        }
        let client = GcalClient::for_account(&account.email)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let calendars = client
            .list_calendars()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let selected = params
            .calendar_id
            .as_deref()
            .and_then(|id| calendars.iter().find(|calendar| calendar.id == id))
            .or_else(|| calendars.iter().find(|calendar| calendar.primary))
            .or_else(|| calendars.first())
            .ok_or_else(|| McpError::invalid_params("No Google calendar is available", None))?
            .clone();

        // ── First DB write. ──
        let calendar_row_id = {
            let conn = self.open_db()?;
            for calendar in &calendars {
                db::gcal::upsert_calendar(&conn, &params.account_id, calendar)
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            }
            db::gcal::upsert_calendar(&conn, &params.account_id, &selected)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?
        };

        let event_id = crate::email::gcal::generate_event_id();
        let mut description = params.description.clone();

        // The Zoom meeting is created BEFORE the Google event, because the join
        // link has to be in the description the insert carries. `attach_to_new_event`
        // writes its durable local record first, so the orphan window is one HTTP
        // call wide and instrumented even then.
        let zoom_db = std::sync::Mutex::new(self.open_db()?);
        let attachment = if conference == Conferencing::Zoom {
            let (Some(start_instant), Some(end_instant)) = (start_instant, end_instant) else {
                return Err(McpError::invalid_params(
                    "conference=\"zoom\" needs a start and end time, not dates.",
                    None,
                ));
            };
            let attachment = zoom_sync::attach_to_new_event(
                &zoom_db,
                &params.account_id,
                &selected.id,
                &event_id,
                summary,
                // MUST go through the shared formatter. The believed-remote state
                // seeded here is later string-compared against
                // `desired_from_event`'s output, so any other spelling of this
                // instant makes the first reconcile tick patch an event nobody
                // touched.
                &zoom_sync::wire_start_time(start_instant),
                (end_instant - start_instant).num_minutes(),
                description.as_deref(),
            )
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            description = Some(attachment.description.clone());
            Some(attachment)
        } else {
            None
        };

        let mut event = NewEvent {
            id: event_id.clone(),
            summary: summary.to_string(),
            description,
            location: params.location,
            start,
            end,
            attendees: attendees.clone(),
            conference_data: None,
        };
        if conference == Conferencing::Meet {
            event.add_meet(format!("meet-{event_id}"));
        }

        let remote = match client
            .insert_event(&selected.id, &event, SendUpdates::None)
            .await
        {
            Ok(remote) => remote,
            // A 409 means our generated id already exists — the event IS there,
            // so this is a success, not a compensation case.
            Err(crate::error::AppError::CalendarApi(409, _)) => client
                .get_event(&selected.id, &event_id)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?,
            Err(error) => {
                if let Some(attachment) = attachment.as_ref() {
                    let status = match &error {
                        crate::error::AppError::CalendarApi(status, _) => *status,
                        // Anything that is not a Calendar status is treated as
                        // "we do not know what happened", which is the safe read.
                        _ => 0,
                    };
                    zoom_sync::unwind_after_failed_insert(
                        &zoom_db,
                        attachment,
                        &client,
                        &selected.id,
                        &event_id,
                        status,
                    )
                    .await;
                }
                return Err(McpError::internal_error(format!("{error}"), None));
            }
        };

        let local_id = {
            let conn = self.open_db()?;
            let id = db::gcal::upsert_remote_event(
                &conn,
                calendar_row_id,
                &params.account_id,
                &selected.id,
                &remote,
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            db::gcal::set_pending_notify(&conn, id, !attendees.is_empty())
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            id
        };
        notify(
            &Envelope::new("calendar", &params.account_id, &selected.id, local_id as u32)
                .with_tool("create_calendar_event"),
        )
        .await;
        let output = serde_json::json!({
            "event_id": local_id,
            "google_event_id": remote.id,
            "htmlLink": remote.html_link,
            "conference": conference.as_str(),
            "meet_url": crate::email::gcal::meet_url(&remote),
            "zoom_join_url": attachment.as_ref().map(|a| a.join_url.clone()),
            "attendees": attendees.iter().filter_map(|a| a.email.as_ref()).collect::<Vec<_>>(),
            "pending_notify": !attendees.is_empty(),
            "attendees_notified": false
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| output.to_string()),
        )]))
    }

    #[tool(
        name = "update_calendar_event",
        description = "Update a Google Calendar event with sendUpdates=none. Attendee changes are saved but not notified; call send_calendar_invites after human review."
    )]
    async fn update_calendar_event(
        &self,
        Parameters(params): Parameters<UpdateCalendarEventParams>,
    ) -> Result<CallToolResult, McpError> {
        let row = {
            let conn = self.open_db()?;
            db::gcal::get_event(&conn, params.event_id)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                .ok_or_else(|| McpError::invalid_params("Calendar event not found", None))?
        };
        // `EventPatch.description` replaces the field wholesale, so "reschedule
        // this and update the description" would silently delete the Zoom join
        // block and report success — the user would see the meeting move and the
        // join button disappear, with nothing connecting the two.
        //
        // Refused only on the MCP path. The compose UI shows the description in a
        // textarea, so a person editing it there can see what they are replacing.
        if let Some(incoming) = params.description.as_deref() {
            if zoom_sync::zoom_block_would_be_dropped(row.description.as_deref(), incoming) {
                return Err(McpError::invalid_params(
                    format!(
                        "This event's description carries its Zoom join link, and the replacement \
                         does not — setting it would leave the meeting scheduled on Zoom with no \
                         way for anyone to join from the calendar. Keep the \
                         {open}…{close} block in the new description (copy it verbatim from \
                         list_calendar_events), or omit `description` entirely if you only meant \
                         to change the time.",
                        open = zoom_sync::BLOCK_OPEN,
                        close = zoom_sync::BLOCK_CLOSE,
                    ),
                    None,
                ));
            }
        }
        let account = {
            let conn = self.open_db()?;
            self.get_account(&conn, &row.account_id)?
        };
        let client = GcalClient::for_account(&account.email)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let zone = params
            .time_zone
            .as_deref()
            .or(row.start_tz.as_deref())
            .unwrap_or("UTC");
        let attendees = params
            .attendees
            .as_ref()
            .map(|items| crate::email::event_input::validate_attendees(items))
            .transpose()
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let patch = EventPatch {
            summary: params.summary,
            description: params.description,
            location: params.location,
            start: params
                .start
                .as_deref()
                .map(|value| crate::email::event_input::event_datetime(value, zone))
                .transpose()
                .map_err(|e| McpError::invalid_params(format!("{e}"), None))?,
            end: params
                .end
                .as_deref()
                .map(|value| crate::email::event_input::event_datetime(value, zone))
                .transpose()
                .map_err(|e| McpError::invalid_params(format!("{e}"), None))?,
            attendees: attendees.clone(),
            conference_data: None,
        };
        let remote = match client
            .patch_event(
                &row.gcal_calendar_id,
                &row.gcal_event_id,
                &patch,
                row.etag.as_deref(),
                SendUpdates::None,
            )
            .await
        {
            Ok(remote) => remote,
            Err(crate::error::AppError::CalendarApi(412, _)) => {
                let fresh = client
                    .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
                    .await
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
                if crate::email::gcal_sync::changes_are_disjoint(
                    row.raw_json.as_deref(),
                    &fresh,
                    &patch,
                ) {
                    client
                        .patch_event(
                            &row.gcal_calendar_id,
                            &row.gcal_event_id,
                            &patch,
                            fresh.etag.as_deref(),
                            SendUpdates::None,
                        )
                        .await
                        .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                } else {
                    let remote_json = serde_json::to_string(&fresh).map_err(|e| {
                        McpError::internal_error(format!("Conflict serialization failed: {e}"), None)
                    })?;
                    let conn = self.open_db()?;
                    db::gcal::record_push_error(&conn, row.id, Some(&remote_json))
                        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
                    return Err(McpError::invalid_params(
                        "The event changed in Google Calendar and conflicts with this update. Resolve it in CXMail.",
                        None,
                    ));
                }
            }
            Err(error) => return Err(McpError::internal_error(format!("{error}"), None)),
        };
        let refreshed_id = {
            let conn = self.open_db()?;
            let id = db::gcal::upsert_remote_event(
                &conn,
                row.calendar_row_id,
                &row.account_id,
                &row.gcal_calendar_id,
                &remote,
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            if attendees.is_some() {
                db::gcal::set_pending_notify(&conn, id, true)
                    .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            }
            id
        };
        // Reconcile immediately so a reschedule is reflected on Zoom now rather
        // than up to two minutes later. Non-fatal: the calendar edit the caller
        // asked for has already succeeded, and the background pass retries.
        let zoom_db = std::sync::Mutex::new(self.open_db()?);
        let zoom_note = match zoom_sync::reconcile_event(&zoom_db, refreshed_id).await {
            Ok(()) => String::new(),
            Err(error) => {
                log::warn!("Zoom reconcile after update_calendar_event failed: {error}");
                format!(" ⚠ the Google event was updated but Zoom was not: {error}")
            }
        };
        let zoom_link = {
            let conn = self.open_db()?;
            db::zoom::get_by_local_event(&conn, refreshed_id)
                .ok()
                .flatten()
                .and_then(|link| link.join_url)
        };
        notify(
            &Envelope::new(
                "calendar",
                &row.account_id,
                &row.gcal_calendar_id,
                row.id as u32,
            )
            .with_tool("update_calendar_event"),
        )
        .await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{}{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "event_id": row.id,
                "htmlLink": remote.html_link,
                "conference": if zoom_link.is_some() { "zoom" } else if remote.hangout_link.is_some() { "meet" } else { "none" },
                "meet_url": crate::email::gcal::meet_url(&remote),
                "zoom_join_url": zoom_link,
                "attendees": remote.attendees,
                "pending_notify": attendees.is_some(),
                "attendees_notified": false
            }))
            .unwrap_or_default(),
            zoom_note,
        ))]))
    }

    // ─── Approve tier ──────────────────────────────────────────

    #[tool(
        name = "send_calendar_invites",
        description = "Request the user's approval to notify every attendee on a previously created/updated calendar event. This tool does NOT send: it returns immediately with status=approval_pending, and CXMail shows the user an approval card. The user clicking Approve there is what delivers the invitation. Review the complete attendee list returned by create/update first, then pass confirmed=true. After calling this, tell the user it is waiting for their approval in CXMail — never report the invitation as sent."
    )]
    async fn send_calendar_invites(
        &self,
        Parameters(params): Parameters<SendCalendarInvitesParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.confirmed != Some(true) {
            return Err(McpError::invalid_params(
                "send_calendar_invites requires confirmed=true after reviewing all attendees",
                None,
            ));
        }
        let row = {
            let conn = self.open_db()?;
            db::gcal::get_event(&conn, params.event_id)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?
                .ok_or_else(|| McpError::invalid_params("Calendar event not found", None))?
        };
        let account = {
            let conn = self.open_db()?;
            self.get_account(&conn, &row.account_id)?
        };
        let client = GcalClient::for_account(&account.email)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let fresh = client
            .get_event(&row.gcal_calendar_id, &row.gcal_event_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let attendees = fresh.attendees.clone().unwrap_or_default();
        if attendees.is_empty() {
            return Err(McpError::invalid_params(
                "The event has no attendees to notify",
                None,
            ));
        }
        // Normalize through the SAME helper the delivery side uses. These two
        // lists are compared for equality across processes and across hours, so
        // any difference in how they trim, case-fold, dedupe, or drop blanks
        // shows up as phantom drift and silently blocks a legitimate send.
        let approved_recipients = crate::email::gcal_invite::normalize_recipients(
            attendees
                .iter()
                .filter_map(|attendee| attendee.email.as_deref()),
        );
        {
            // The app constructs its approval details from the shared mirror.
            // Refresh that row first so the modal displays the exact recipient
            // snapshot fetched above, not a potentially stale local copy.
            let conn = self.open_db()?;
            db::gcal::upsert_remote_event(
                &conn,
                row.calendar_row_id,
                &row.account_id,
                &row.gcal_calendar_id,
                &fresh,
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        }

        // `confirmed=true` starts the Approve-tier flow; it is not itself
        // authorization to notify attendees. The MCP records a single-use
        // approval request, pings the running app, and returns — it does NOT
        // wait for the decision, and it does NOT deliver.
        //
        // It used to block here for 60 seconds and send on approval. The
        // deadline was the failure mode: the approval card renders inside the
        // CXMail window, so a backgrounded app showed it to nobody and the tool
        // auto-denied while the user sat in a terminal. Approving in the app now
        // performs the send itself (`email::gcal_invite::deliver_approved_invite`),
        // so there is no clock to lose and no window in which this process must
        // stay alive. `approved_recipients` is persisted with the request so the
        // drift check on the delivery side compares against what the user
        // actually saw, however long they take.
        let approval_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = self.open_db()?;
            db::gcal::request_invite_approval(&conn, row.id, &approval_id, &approved_recipients)
                .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        }
        notify(
            &Envelope::new(
                "calendar-approval-request",
                &row.account_id,
                &row.gcal_calendar_id,
                row.id as u32,
            )
            .with_tool("send_calendar_invites")
            .with_approval_id(&approval_id),
        )
        .await;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&serde_json::json!({
                "event_id": row.id,
                "status": "approval_pending",
                "approval_id": approval_id,
                "attendees": approved_recipients,
                "attendees_notified": false,
                "pending_notify": true,
                "next_step": "NOTHING HAS BEEN SENT YET. CXMail is showing an approval card for exactly these recipients and its dock icon is bouncing. The user clicking Approve there is what delivers the invitation. Tell the user it is waiting on them — do NOT report the invitation as sent, and do NOT call this tool again unless they ask for a fresh request."
            }))
            .unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "send_email",
        description = "DISABLED BY DESIGN — this tool does NOT send. Sending is intentionally not exposed to the MCP so an automated assistant can never deliver mail on the user's behalf. To get an email out: call `compose_draft` (it supports full HTML bodies, the `rich`/`card` layouts, cc/bcc, threading via reply_to_message_id, the user's signature, and file `attachments`), then tell the user to review it in CXMail's Drafts and click Send. Use `edit_draft` to revise a draft. Do not retry send_email — it will only return this guidance."
    )]
    async fn send_email(
        &self,
        Parameters(params): Parameters<SendEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = params;
        Err(McpError::invalid_params(
            "send_email is disabled by design: the MCP never delivers mail directly. \
             Use compose_draft (full HTML, rich/card layouts, cc/bcc, threading, and \
             file attachments are all supported) to build the message, then have the \
             user review it in CXMail's Drafts folder and click Send. Use edit_draft to revise.",
            None,
        ))
    }

    #[tool(
        name = "delete_email",
        description = "Delete an email. By default moves to Trash; set permanent=true to permanently delete. Must pass confirmed=true."
    )]
    async fn delete_email(
        &self,
        Parameters(params): Parameters<DeleteEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.confirmed != Some(true) {
            return Err(McpError::invalid_params(
                "delete_email requires confirmed=true to proceed. This is a safety gate for destructive operations.",
                None,
            ));
        }

        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let permanent = params.permanent.unwrap_or(false);

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.folder).await;

        if permanent {
            // Mark \Deleted and expunge
            imap::store_flags(&mut session, params.uid, true, "\\Deleted")
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("Failed to mark deleted: {}", e), None)
                })?;
            imap::expunge(&mut session)
                .await
                .map_err(|e| McpError::internal_error(format!("Expunge failed: {}", e), None))?;
        } else {
            // Move to trash
            let trash = {
                let conn = self.open_db()?;
                db::folders::folder_for_account(&conn, &account.id, &account.provider, "trash")
            };
            let trash = trash.as_str();
            imap::move_message(&mut session, params.uid, trash)
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("Move to trash failed: {}", e), None)
                })?;
        }
        let _ = imap::disconnect(session, &account.email).await;

        // Remove from local DB cache
        conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            rusqlite::params![params.account_id, params.folder, params.uid],
        )
        .map_err(|e| McpError::internal_error(format!("DB cleanup failed: {}", e), None))?;

        notify(
            &Envelope::new("email-mutated", &params.account_id, &params.folder, params.uid)
                .with_tool("delete_email"),
        )
        .await;

        let action = if permanent {
            "Permanently deleted"
        } else {
            "Moved to trash"
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} UID {} from {}",
            action, params.uid, params.folder
        ))]))
    }

    #[tool(
        name = "bulk_delete_emails",
        description = "Delete multiple emails at once using a single IMAP connection. Much faster than calling delete_email in a loop. Must pass confirmed=true."
    )]
    async fn bulk_delete_emails(
        &self,
        Parameters(params): Parameters<BulkDeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.confirmed != Some(true) {
            return Err(McpError::invalid_params(
                "bulk_delete_emails requires confirmed=true to proceed.",
                None,
            ));
        }
        if params.uids.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No UIDs provided",
            )]));
        }

        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;
        let permanent = params.permanent.unwrap_or(false);
        let count = params.uids.len();

        let mut session = imap::connect_for_account(&account)
            .await
            .map_err(|e| McpError::internal_error(format!("IMAP connect failed: {}", e), None))?;
        let _ = imap::select_folder(&mut session, &params.folder).await;

        // Chunk UIDs into batches of 200 to stay under IMAP command line limits
        const CHUNK_SIZE: usize = 200;
        let chunks: Vec<&[u32]> = params.uids.chunks(CHUNK_SIZE).collect();
        let mut errors: Vec<String> = Vec::new();
        let mut succeeded: usize = 0;

        let trash = {
            let conn = self.open_db()?;
            db::folders::folder_for_account(&conn, &account.id, &account.provider, "trash")
        };
        let trash = trash.as_str();

        for (idx, chunk) in chunks.iter().enumerate() {
            let uid_list: String = chunk
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");

            if permanent {
                // STORE +\Deleted then EXPUNGE per chunk
                let store_ok = {
                    match session.uid_store(&uid_list, "+FLAGS (\\Deleted)").await {
                        Ok(stream) => {
                            futures::pin_mut!(stream);
                            while stream.as_mut().next().await.is_some() {}
                            Ok(())
                        }
                        Err(e) => Err(format!("STORE failed: {}", e)),
                    }
                };
                if let Err(e) = store_ok {
                    errors.push(format!("chunk {}: {}", idx, e));
                    continue;
                }
                if let Err(e) = imap::expunge(&mut session).await {
                    errors.push(format!("chunk {}: EXPUNGE failed: {}", idx, e));
                    continue;
                }
                succeeded += chunk.len();
            } else {
                // Bulk UID MOVE with comma-separated list (RFC 6851)
                let moved = session.uid_mv(&uid_list, trash).await.is_ok();
                if moved {
                    succeeded += chunk.len();
                } else {
                    // Fallback: bulk COPY + \Deleted + EXPUNGE
                    if let Err(e) = session.uid_copy(&uid_list, trash).await {
                        errors.push(format!("chunk {}: COPY failed: {}", idx, e));
                        continue;
                    }
                    let store_ok = {
                        match session.uid_store(&uid_list, "+FLAGS (\\Deleted)").await {
                            Ok(stream) => {
                                futures::pin_mut!(stream);
                                while stream.as_mut().next().await.is_some() {}
                                Ok(())
                            }
                            Err(e) => Err(format!("STORE failed: {}", e)),
                        }
                    };
                    if let Err(e) = store_ok {
                        errors.push(format!("chunk {}: {}", idx, e));
                        continue;
                    }
                    if let Err(e) = imap::expunge(&mut session).await {
                        errors.push(format!("chunk {}: EXPUNGE failed: {}", idx, e));
                        continue;
                    }
                    succeeded += chunk.len();
                }
            }
        }

        let _ = imap::disconnect(session, &account.email).await;

        // Bulk delete from local DB
        for uid in &params.uids {
            let _ = conn.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                rusqlite::params![params.account_id, params.folder, uid],
            );
        }

        // One notification triggers a full list refresh on the app side; the
        // representative UID is just a hint (deleted rows leave the list anyway).
        let rep_uid = params.uids.first().copied().unwrap_or(0);
        notify(
            &Envelope::new("email-mutated", &params.account_id, &params.folder, rep_uid)
                .with_tool("bulk_delete_emails"),
        )
        .await;

        let action = if permanent {
            "Permanently deleted"
        } else {
            "Moved to trash"
        };
        let mut msg = format!(
            "{} {}/{} emails from {} ({} chunks of up to {})",
            action,
            succeeded,
            count,
            params.folder,
            chunks.len(),
            CHUNK_SIZE
        );
        if !errors.is_empty() {
            msg.push_str(&format!(
                "\nErrors ({}):\n  {}",
                errors.len(),
                errors.join("\n  ")
            ));
        }
        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(
        name = "list_attachments",
        description = "List attachments on a message by account_id, folder, and UID. Output lines carry the Index value to pass to download_attachment. Reads the local cache — if empty, call read_email first and retry."
    )]
    async fn list_attachments(
        &self,
        Parameters(params): Parameters<ListAttachmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let atts =
            db::messages::get_attachments(&conn, &params.account_id, &params.folder, params.uid)
                .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;

        if atts.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No attachments recorded in cache. If you expect attachments, call read_email for this message first (it populates the cache via IMAP), then retry list_attachments.".to_string(),
            )]));
        }

        let lines: Vec<String> = atts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                format!(
                    "Index:{} | {} | {} | {} bytes | inline={}",
                    i,
                    a.filename.as_deref().unwrap_or("(no filename)"),
                    a.content_type,
                    a.size_bytes,
                    a.is_inline,
                )
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "download_attachment",
        description = "Download one attachment to disk. Provide exactly one of `index` (from list_attachments) or `filename`. If `save_path` is omitted, saves to ~/Downloads/<filename>. save_path may be a directory (trailing slash or existing dir) or a full file path. Overwrites existing files. Returns the absolute saved path and byte count."
    )]
    async fn download_attachment(
        &self,
        Parameters(params): Parameters<DownloadAttachmentParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let account = self.get_account(&conn, &params.account_id)?;

        let (resolved_index, hinted_filename) = match (params.index, params.filename.as_ref()) {
            (Some(_), Some(_)) => {
                return Err(McpError::invalid_params(
                    "Provide exactly one of `index` or `filename`, not both.",
                    None,
                ));
            }
            (None, None) => {
                return Err(McpError::invalid_params(
                    "Provide `index` (from list_attachments) or `filename`.",
                    None,
                ));
            }
            (Some(i), None) => (i, None),
            (None, Some(name)) => {
                let atts = db::messages::get_attachments(
                    &conn,
                    &params.account_id,
                    &params.folder,
                    params.uid,
                )
                .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
                let exact = atts
                    .iter()
                    .position(|a| a.filename.as_deref() == Some(name.as_str()));
                let idx = exact.or_else(|| {
                    atts.iter().position(|a| {
                        a.filename
                            .as_deref()
                            .map_or(false, |f| f.eq_ignore_ascii_case(name))
                    })
                });
                match idx {
                    Some(i) => (i, Some(name.clone())),
                    None => {
                        return Err(McpError::invalid_params(
                            format!("No attachment named {:?} on this message. Call list_attachments to see available filenames.", name),
                            None,
                        ));
                    }
                }
            }
        };

        // Drop the DB connection before the IMAP round-trip to avoid holding it across awaits.
        drop(conn);

        let fetch_account = account.clone();
        let folder = params.folder.clone();
        let uid = params.uid;
        let fetch = tokio::time::timeout(Duration::from_secs(60), async move {
            let mut session =
                imap::connect_for_account(&fetch_account).await?;
            let _ = imap::select_folder(&mut session, &folder).await?;
            let raw = imap::fetch_body(&mut session, uid).await?;
            let _ = imap::disconnect(session, &fetch_account.email).await;
            Ok::<Vec<u8>, crate::error::AppError>(raw)
        })
        .await
        .map_err(|_| McpError::internal_error("Timed out fetching message body after 60s", None))?
        .map_err(|e| McpError::internal_error(format!("IMAP fetch failed: {}", e), None))?;

        let (extracted_name, data) = parser::extract_attachment(&fetch, resolved_index)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "Parser has no attachment at index {} for this message.",
                        resolved_index
                    ),
                    None,
                )
            })?;

        let effective_filename = if extracted_name.is_empty() || extracted_name == "attachment" {
            hinted_filename.unwrap_or_else(|| format!("attachment-{}.bin", resolved_index))
        } else {
            extracted_name
        };

        let target_path = resolve_save_path(params.save_path.as_deref(), &effective_filename)?;
        if let Some(parent) = target_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    McpError::internal_error(
                        format!("Failed to create {}: {}", parent.display(), e),
                        None,
                    )
                })?;
            }
        }
        std::fs::write(&target_path, &data).map_err(|e| {
            McpError::internal_error(
                format!("Failed to write {}: {}", target_path.display(), e),
                None,
            )
        })?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Saved {} bytes to {}",
            data.len(),
            target_path.display()
        ))]))
    }

    // ─── Inbox groups ──────────────────────────────────────────

    #[tool(
        name = "list_groups",
        description = "List all inbox groups (a.k.a. mailbox groups), with their accounts, filter rules, and unread counts. Inbox groups filter the unified inbox by account membership and per-message rules. Returns JSON."
    )]
    async fn list_groups(&self) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let groups = db::inbox_groups::list_groups(&conn)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        let json = serde_json::to_string_pretty(&groups)
            .map_err(|e| McpError::internal_error(format!("Serialize failed: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "create_group",
        description = "Create a new inbox group with optional account memberships and filter rules. A message belongs to the group if it is from any listed account OR matches any rule. Returns the new group's id."
    )]
    async fn create_group(
        &self,
        Parameters(params): Parameters<CreateGroupParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.name.trim().is_empty() {
            return Err(McpError::invalid_params(
                "name is required and must be non-empty",
                None,
            ));
        }
        let rules = validate_group_rules(params.rules.as_deref().unwrap_or(&[]))?;
        let account_ids = params.account_ids.unwrap_or_default();
        let color = params.color.unwrap_or_else(|| "#0a84ff".to_string());
        let icon = params.icon.unwrap_or_else(|| "folder".to_string());

        let conn = self.open_db()?;
        let id = db::inbox_groups::create_group(&conn, &params.name, &color, &icon)
            .map_err(|e| McpError::internal_error(format!("Create failed: {}", e), None))?;
        db::inbox_groups::set_rules(&conn, id, &rules)
            .map_err(|e| McpError::internal_error(format!("Set rules failed: {}", e), None))?;
        db::inbox_groups::set_account_ids(&conn, id, &account_ids)
            .map_err(|e| McpError::internal_error(format!("Set accounts failed: {}", e), None))?;

        // Group membership/rules changed — nudge the app to refresh the filtered
        // inbox. No specific message, so account_id/folder/uid are placeholders.
        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("create_group")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Created group id={} name={:?} ({} rules, {} accounts)",
            id,
            params.name,
            rules.len(),
            account_ids.len()
        ))]))
    }

    #[tool(
        name = "update_group",
        description = "Update an inbox group. Replaces name, color, icon, rules, and account memberships in full. Call list_groups first to read the current values you want to preserve."
    )]
    async fn update_group(
        &self,
        Parameters(params): Parameters<UpdateGroupParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.name.trim().is_empty() {
            return Err(McpError::invalid_params(
                "name is required and must be non-empty",
                None,
            ));
        }
        let rules = validate_group_rules(&params.rules)?;

        let conn = self.open_db()?;
        db::inbox_groups::update_group(&conn, params.id, &params.name, &params.color, &params.icon)
            .map_err(|e| McpError::internal_error(format!("Update failed: {}", e), None))?;
        db::inbox_groups::set_rules(&conn, params.id, &rules)
            .map_err(|e| McpError::internal_error(format!("Set rules failed: {}", e), None))?;
        db::inbox_groups::set_account_ids(&conn, params.id, &params.account_ids)
            .map_err(|e| McpError::internal_error(format!("Set accounts failed: {}", e), None))?;

        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("update_group")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Updated group id={} ({} rules, {} accounts)",
            params.id,
            rules.len(),
            params.account_ids.len()
        ))]))
    }

    #[tool(
        name = "delete_group",
        description = "Delete an inbox group along with its rule and account associations (no messages are deleted). Must pass confirmed=true."
    )]
    async fn delete_group(
        &self,
        Parameters(params): Parameters<DeleteGroupParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.confirmed != Some(true) {
            return Err(McpError::invalid_params(
                "delete_group requires confirmed=true to proceed.",
                None,
            ));
        }
        let conn = self.open_db()?;
        // Explicitly clear children since the MCP connection doesn't enable
        // PRAGMA foreign_keys, so ON DELETE CASCADE wouldn't fire here.
        db::inbox_groups::set_rules(&conn, params.id, &[])
            .map_err(|e| McpError::internal_error(format!("Clear rules failed: {}", e), None))?;
        db::inbox_groups::set_account_ids(&conn, params.id, &[])
            .map_err(|e| McpError::internal_error(format!("Clear accounts failed: {}", e), None))?;
        db::inbox_groups::delete_group(&conn, params.id)
            .map_err(|e| McpError::internal_error(format!("Delete failed: {}", e), None))?;

        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("delete_group")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted group id={}",
            params.id
        ))]))
    }

    // ─── Nudges ───────────────────────────────────────────────────────

    #[tool(
        name = "list_nudges",
        description = "List stalled conversations: threads where we wrote last and got no answer (kind \"follow_up\"), and threads where they wrote last and we never answered (kind \"reply\"). Both are limited to people who have actually corresponded with us — someone who has replied to something we sent at least once — so cold-outreach batches and newsletters do not appear no matter how long they sit unanswered. Read-only and local; no message text leaves the machine. Each entry gives the thread's INBOX Folder/UID (usable with read_thread, read_email, compose_draft's reply_to_folder/reply_to_uid), the counterpart, and how many days it has been waiting. Use this to answer \"what am I forgetting / who is waiting on me\"; use dismiss_nudge to silence one that is deliberate."
    )]
    async fn list_nudges(&self) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let nudges = db::nudges::list(&conn, &db::nudges::NudgeOptions::default())
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;

        if nudges.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No stalled conversations. (Nudges only cover people who have replied to you \
                 before, threads 3–30 days old, and threads with a message in the inbox.)"
                    .to_string(),
            )]));
        }

        let json = serde_json::to_string_pretty(&nudges)
            .map_err(|e| McpError::internal_error(format!("Serialize failed: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "dismiss_nudge",
        description = "Permanently silence one nudge lane on one thread. `kind` is \"follow_up\" or \"reply\" — dismissing one leaves the other alone, since \"the ball isn't in their court\" says nothing about whether they later write something you owe an answer to. Pass the `thread_key` and `account_id` exactly as printed by list_nudges."
    )]
    async fn dismiss_nudge(
        &self,
        Parameters(params): Parameters<DismissNudgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        db::nudges::dismiss(&conn, &params.account_id, &params.thread_key, &params.kind)
            .map_err(|e| McpError::invalid_params(format!("{}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Dismissed the \"{}\" nudge for thread {}.",
            params.kind, params.thread_key
        ))]))
    }

    // ─── Mail rules ───────────────────────────────────────────────────
    //
    // Distinct from inbox groups above: these are the classification rules
    // applied to incoming mail (category / read / flagged), not a view filter.

    #[tool(
        name = "list_mail_rules",
        description = "List all mail rules — the filters applied to incoming mail at classification time. Each rule ANDs its conditions and, on a match, applies its actions (set_category / mark_read / mark_flagged). Returns JSON including a per-rule `warnings` array flagging any condition or action that can never fire. Distinct from list_groups, which filters the inbox view instead of classifying mail."
    )]
    async fn list_mail_rules(&self) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let rules = db::rules::list(&conn)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;

        let annotated: Vec<serde_json::Value> = rules
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or(serde_json::Value::Null);
                let warnings = mail_rule_warnings(r);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "scope".to_string(),
                        serde_json::Value::String(match r.account_id.as_deref() {
                            Some(a) => format!("account:{}", a),
                            None => "global".to_string(),
                        }),
                    );
                    obj.insert("warnings".to_string(), serde_json::json!(warnings));
                }
                v
            })
            .collect();

        let json = serde_json::to_string_pretty(&annotated)
            .map_err(|e| McpError::internal_error(format!("Serialize failed: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "create_mail_rule",
        description = "Create a mail rule that classifies incoming mail. ALL conditions must match (logical AND; there is no OR — use separate rules for that) and matching is case-insensitive. Conditions match on from / to / subject only. Actions are set_category (primary|updates|social|promotions|junk), mark_read, mark_flagged; \"move\" and \"delete\" are rejected because the classifier never executes them. Rules apply to mail synced AFTER creation — use preview_mail_rule first to see what an existing mailbox would match, then apply_mail_rule to make it take effect on that existing mail. Returns the new rule's id."
    )]
    async fn create_mail_rule(
        &self,
        Parameters(params): Parameters<CreateMailRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.name.trim().is_empty() {
            return Err(McpError::invalid_params(
                "name is required and must be non-empty",
                None,
            ));
        }
        let conditions = validate_rule_conditions(&params.conditions)?;
        let actions = validate_rule_actions(&params.actions)?;

        let conn = self.open_db()?;
        // Reject a scope that doesn't exist rather than silently creating a
        // rule that can never match an account.
        let account_id = match params.account_id.as_deref().map(str::trim) {
            Some(a) if !a.is_empty() => {
                self.get_account(&conn, a)?;
                Some(a.to_string())
            }
            _ => None,
        };

        let rule = db::rules::MailRule {
            id: None,
            account_id,
            name: params.name.trim().to_string(),
            is_active: params.is_active.unwrap_or(true),
            priority: params.priority.unwrap_or(0),
            conditions,
            actions,
        };
        let id = db::rules::insert(&conn, &rule)
            .map_err(|e| McpError::internal_error(format!("Create failed: {}", e), None))?;

        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("create_mail_rule")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Created mail rule id={} name={:?} ({}, {} conditions, {} actions, active={}).\n\
             Applies to mail synced from now on; existing messages are unaffected. \
             Use preview_mail_rule to see what it would match today, then \
             apply_mail_rule(id={}, dry_run=false) to classify that existing mail now.",
            id,
            rule.name,
            rule.account_id
                .as_deref()
                .map(|a| format!("scoped to account {}", a))
                .unwrap_or_else(|| "global".to_string()),
            rule.conditions.len(),
            rule.actions.len(),
            rule.is_active,
            id
        ))]))
    }

    #[tool(
        name = "update_mail_rule",
        description = "Update a mail rule. Replaces name, scope, active flag, priority, conditions and actions IN FULL — call list_mail_rules first to read the values you want to preserve. Same validation as create_mail_rule."
    )]
    async fn update_mail_rule(
        &self,
        Parameters(params): Parameters<UpdateMailRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.name.trim().is_empty() {
            return Err(McpError::invalid_params(
                "name is required and must be non-empty",
                None,
            ));
        }
        let conditions = validate_rule_conditions(&params.conditions)?;
        let actions = validate_rule_actions(&params.actions)?;

        let conn = self.open_db()?;
        let existing = db::rules::list(&conn)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        if !existing.iter().any(|r| r.id == Some(params.id)) {
            return Err(McpError::invalid_params(
                format!(
                    "Mail rule id={} not found. Use list_mail_rules to see current rules.",
                    params.id
                ),
                None,
            ));
        }
        let account_id = match params.account_id.as_deref().map(str::trim) {
            Some(a) if !a.is_empty() => {
                self.get_account(&conn, a)?;
                Some(a.to_string())
            }
            _ => None,
        };

        let rule = db::rules::MailRule {
            id: Some(params.id),
            account_id,
            name: params.name.trim().to_string(),
            is_active: params.is_active.unwrap_or(true),
            priority: params.priority.unwrap_or(0),
            conditions,
            actions,
        };
        db::rules::update(&conn, &rule)
            .map_err(|e| McpError::internal_error(format!("Update failed: {}", e), None))?;

        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("update_mail_rule")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Updated mail rule id={} name={:?} ({}, {} conditions, {} actions, active={}).",
            params.id,
            rule.name,
            rule.account_id
                .as_deref()
                .map(|a| format!("scoped to account {}", a))
                .unwrap_or_else(|| "global".to_string()),
            rule.conditions.len(),
            rule.actions.len(),
            rule.is_active
        ))]))
    }

    #[tool(
        name = "delete_mail_rule",
        description = "Delete a mail rule. No messages are affected and past classifications are not reverted — mail already filed by this rule stays where it is. Must pass confirmed=true."
    )]
    async fn delete_mail_rule(
        &self,
        Parameters(params): Parameters<DeleteMailRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.confirmed != Some(true) {
            return Err(McpError::invalid_params(
                "delete_mail_rule requires confirmed=true to proceed.",
                None,
            ));
        }
        let conn = self.open_db()?;
        let existing = db::rules::list(&conn)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        let Some(rule) = existing.iter().find(|r| r.id == Some(params.id)) else {
            return Err(McpError::invalid_params(
                format!(
                    "Mail rule id={} not found. Use list_mail_rules to see current rules.",
                    params.id
                ),
                None,
            ));
        };
        let name = rule.name.clone();

        db::rules::delete(&conn, params.id)
            .map_err(|e| McpError::internal_error(format!("Delete failed: {}", e), None))?;

        notify(&Envelope::new("email-mutated", "", "", 0).with_tool("delete_mail_rule")).await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted mail rule id={} name={:?}. Messages already classified by it are unchanged.",
            params.id, name
        ))]))
    }

    #[tool(
        name = "preview_mail_rule",
        description = "Dry run: report how many already-synced inbox messages a rule's conditions would match, with a sample. Read-only — nothing is classified, moved or modified. Pass either rule_id (an existing rule) or conditions (a hypothetical one). Use this before create_mail_rule to check a filter is neither too broad nor dead, since rules otherwise only take effect on the next sync. To actually classify the mail that is already there, use apply_mail_rule — it matches identically, via the same scanner."
    )]
    async fn preview_mail_rule(
        &self,
        Parameters(params): Parameters<PreviewMailRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;

        let (conditions, label, rule_account) = match (params.rule_id, &params.conditions) {
            (Some(_), Some(_)) => {
                return Err(McpError::invalid_params(
                    "Pass either rule_id or conditions, not both.",
                    None,
                ))
            }
            (None, None) => {
                return Err(McpError::invalid_params(
                    "Pass either rule_id (to preview an existing rule) or conditions (to preview a hypothetical one).",
                    None,
                ))
            }
            (Some(id), None) => {
                let rules = db::rules::list(&conn)
                    .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
                let rule = rules.into_iter().find(|r| r.id == Some(id)).ok_or_else(|| {
                    McpError::invalid_params(
                        format!(
                            "Mail rule id={} not found. Use list_mail_rules to see current rules.",
                            id
                        ),
                        None,
                    )
                })?;
                let acct = rule.account_id.clone();
                (rule.conditions.clone(), format!("rule id={}", id), acct)
            }
            (None, Some(c)) => (
                validate_rule_conditions(c)?,
                "hypothetical rule".to_string(),
                None,
            ),
        };

        // An account-scoped rule can only ever match its own account; an
        // explicit account_id narrows a global rule for this preview only.
        let scope_account = rule_account.or_else(|| {
            params
                .account_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
        if let Some(ref a) = scope_account {
            self.get_account(&conn, a)?;
        }

        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        // Same row set the reclassify backfill sees: every message in an
        // inbox-type folder.
        let rows = db::messages::list_for_reclassify(&conn, false)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        let scanned_total = rows.len();

        // Evaluate through the real engine so the preview cannot drift from
        // production matching semantics. The placeholder action only has to
        // make `evaluate` return non-empty on a match — a preview never
        // applies anything.
        let probe = db::rules::MailRule {
            id: None,
            account_id: None,
            name: "preview".to_string(),
            is_active: true,
            priority: 0,
            conditions,
            actions: vec![db::rules::RuleAction {
                action_type: "set_category".to_string(),
                value: Some("junk".to_string()),
            }],
        };

        let (scanned, hits) = scan_rule_matches(&rows, &probe, scope_account.as_deref());
        let matched = hits.len();
        let mut matched_user_overrides = 0usize;
        let mut samples: Vec<serde_json::Value> = Vec::new();
        for hit in &hits {
            let row = hit.row;
            if row.category_source.as_deref() == Some("user") {
                matched_user_overrides += 1;
            }
            if samples.len() < limit {
                samples.push(serde_json::json!({
                    "account_id": row.account_id,
                    "folder": row.folder_name,
                    "uid": row.uid,
                    "from": row.from_email,
                    "subject": row.subject,
                    "user_override": row.category_source.as_deref() == Some("user"),
                }));
            }
        }

        let out = serde_json::json!({
            "previewing": label,
            "scope": scope_account
                .as_deref()
                .map(|a| format!("account:{}", a))
                .unwrap_or_else(|| "all accounts".to_string()),
            "messages_scanned": scanned,
            "inbox_messages_total": scanned_total,
            "matched": matched,
            "matched_with_user_override": matched_user_overrides,
            "sample_shown": samples.len(),
            "samples": samples,
            "note": format!(
                "Read-only preview over already-synced inbox messages; nothing was changed. \
                 A live rule applies to mail synced after it is created, plus any message touched \
                 by the app's reclassify backfill. Of the {} match(es), {} carry a manual category \
                 override that the backfill deliberately skips.",
                matched, matched_user_overrides
            ),
        });

        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| McpError::internal_error(format!("Serialize failed: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "apply_mail_rule",
        description = "Apply an existing mail rule to mail ALREADY in the database. Rules otherwise only fire on the next sync, so a rule you just created leaves the matching mail exactly where it is — this is what makes it take effect retroactively. dry_run DEFAULTS TO TRUE: call it once to see the counts, then again with dry_run=false to write. Applies only set_category / mark_read / mark_flagged, only to messages the rule matches, and NEVER to a message whose category was set by hand (category_source='user') — those are reported as skipped. Scoped to inbox-type folders, like the app's own reclassify backfill. Nothing is moved or deleted. mark_read / mark_flagged are written locally AND pushed to the server as \\Seen / \\Flagged, so they survive the next sync's flag reconciliation; category is local by nature (it has no IMAP counterpart). If IMAP is unreachable the local writes are still kept and the result reports how many flag pushes failed — report that honestly rather than claiming a clean apply."
    )]
    async fn apply_mail_rule(
        &self,
        Parameters(params): Parameters<ApplyMailRuleParams>,
    ) -> Result<CallToolResult, McpError> {
        let conn = self.open_db()?;
        let dry_run = params.dry_run.unwrap_or(true);
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let rules = db::rules::list(&conn)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        let rule = rules
            .into_iter()
            .find(|r| r.id == Some(params.id))
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "Mail rule id={} not found. Use list_mail_rules to see current rules.",
                        params.id
                    ),
                    None,
                )
            })?;

        // A rule stored before the T17 guards — or one saved through an
        // older UI — can be dead or catch-all. Refuse rather than apply it
        // to a whole mailbox. (gotcha #36)
        let blockers = rule_apply_blockers(&rule);
        if !blockers.is_empty() {
            return Err(McpError::invalid_params(
                format!(
                    "Refusing to apply rule id={} ({:?}) — it is not safe to run over an existing mailbox:\n- {}\nFix it with update_mail_rule (which validates), then apply again.",
                    params.id,
                    rule.name,
                    blockers.join("\n- ")
                ),
                None,
            ));
        }

        // An account-scoped rule can only ever touch its own account; an
        // explicit account_id narrows a global rule for this run only.
        let scope_account = rule.account_id.clone().or_else(|| {
            params
                .account_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
        if let Some(ref a) = scope_account {
            self.get_account(&conn, a)?;
        }

        let rows = db::messages::list_for_reclassify(&conn, false)
            .map_err(|e| McpError::internal_error(format!("DB error: {}", e), None))?;
        let inbox_total = rows.len();
        let (scanned, hits) = scan_rule_matches(&rows, &rule, scope_account.as_deref());
        let matched = hits.len();

        // Resolve each match into the concrete writes it implies, mirroring
        // `commands::messages::reclassify_inbox_messages`: manual overrides
        // are left alone, and a field is only written when it actually
        // changes so the reported counts stay honest.
        struct Planned<'a> {
            row: &'a db::messages::ReclassifyRow,
            category: Option<String>,
            mark_read: bool,
            mark_flagged: bool,
        }
        let mut skipped_user_override = 0usize;
        let mut planned: Vec<Planned> = Vec::new();
        for hit in &hits {
            if hit.row.category_source.as_deref() == Some("user") {
                skipped_user_override += 1;
                continue;
            }
            let mut category = None;
            let mut mark_read = false;
            let mut mark_flagged = false;
            for action in &hit.actions {
                match action.action_type.as_str() {
                    "set_category" => {
                        if let Some(ref v) = action.value {
                            category = Some(v.clone());
                        }
                    }
                    "mark_read" => mark_read = true,
                    "mark_flagged" => mark_flagged = true,
                    // move/delete are blocked above; anything else is inert.
                    _ => {}
                }
            }
            planned.push(Planned {
                row: hit.row,
                category,
                mark_read,
                mark_flagged,
            });
        }

        // Read current state to distinguish "would change" from "already
        // correct". Done up front so a dry run reports the same numbers a
        // real run writes.
        let mut affected: Vec<serde_json::Value> = Vec::new();
        let mut to_write: Vec<(&Planned, bool, bool, bool)> = Vec::new();
        for p in &planned {
            let current: Option<(Option<String>, i64, i64)> = conn
                .query_row(
                    "SELECT category, is_read, is_flagged FROM messages
                     WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                    rusqlite::params![p.row.account_id, p.row.folder_name, p.row.uid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let (cur_cat, cur_read, cur_flag) =
                current.unwrap_or((None, 0, 0));
            let cat_changes = p
                .category
                .as_deref()
                .map(|c| cur_cat.as_deref().unwrap_or("primary") != c)
                .unwrap_or(false);
            let read_changes = p.mark_read && cur_read == 0;
            let flag_changes = p.mark_flagged && cur_flag == 0;
            if !cat_changes && !read_changes && !flag_changes {
                continue;
            }
            if affected.len() < limit {
                affected.push(serde_json::json!({
                    "account_id": p.row.account_id,
                    "folder": p.row.folder_name,
                    "uid": p.row.uid,
                    "from": p.row.from_email,
                    "subject": p.row.subject,
                    "category": cat_changes.then(|| serde_json::json!({
                        "from": cur_cat.as_deref().unwrap_or("primary"),
                        "to": p.category,
                    })),
                    "mark_read": read_changes,
                    "mark_flagged": flag_changes,
                }));
            }
            to_write.push((p, cat_changes, read_changes, flag_changes));
        }

        let would_change = to_write.len();
        let mut recategorized = 0usize;
        let mut marked_read = 0usize;
        let mut marked_flagged = 0usize;
        let mut pushed_read = 0usize;
        let mut pushed_flagged = 0usize;
        let mut push_failed = 0usize;
        let mut push_errors: Vec<String> = Vec::new();

        // The flag half has to reach the server or it does not survive.
        // `db::messages::apply_server_flags` reconciles flags FROM IMAP on
        // every full sync (and periodically on limited sync), so the server's
        // \Seen wins and a local-only write is silently undone — measured on
        // 2026-08-04: 57 messages marked read, 4 already reverted minutes
        // later. Group the writes by (account, folder) so one session covers
        // a whole folder, and resolve the account rows BEFORE any network
        // work so the DB connection can be dropped first.
        let mut push_groups: std::collections::BTreeMap<(String, String), (Vec<u32>, Vec<u32>)> =
            std::collections::BTreeMap::new();
        if !dry_run {
            for (p, _, read_changes, flag_changes) in &to_write {
                if !*read_changes && !*flag_changes {
                    continue;
                }
                let entry = push_groups
                    .entry((p.row.account_id.clone(), p.row.folder_name.clone()))
                    .or_insert_with(|| (Vec::new(), Vec::new()));
                if *read_changes {
                    entry.0.push(p.row.uid);
                }
                if *flag_changes {
                    entry.1.push(p.row.uid);
                }
            }
        }
        let mut push_accounts: std::collections::HashMap<String, db::accounts::Account> =
            std::collections::HashMap::new();
        for (account_id, _) in push_groups.keys() {
            if push_accounts.contains_key(account_id) {
                continue;
            }
            match db::accounts::get_by_id(&conn, account_id) {
                Ok(Some(a)) => {
                    push_accounts.insert(account_id.clone(), a);
                }
                Ok(None) => {
                    push_errors.push(format!("account {account_id}: not found"));
                }
                Err(e) => {
                    push_errors.push(format!("account {account_id}: {e}"));
                }
            }
        }

        if !dry_run {
            // Chunked transactions. The app holds this same SQLite file open
            // and its UI reads through it, so a single transaction spanning
            // a 14k-row mailbox would block `fetchBody` for the whole run —
            // the shape of gotcha #11. Committing per batch lets other
            // readers in between.
            const CHUNK: usize = 200;
            for batch in to_write.chunks(CHUNK) {
                let tx = conn.unchecked_transaction().map_err(|e| {
                    McpError::internal_error(format!("DB error: {}", e), None)
                })?;
                for (p, cat_changes, read_changes, flag_changes) in batch {
                    let key = rusqlite::params![
                        p.row.account_id,
                        p.row.folder_name,
                        p.row.uid
                    ];
                    if *cat_changes {
                        if let Some(ref c) = p.category {
                            tx.execute(
                                "UPDATE messages SET category = ?4, category_source = 'rule'
                                 WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                                rusqlite::params![
                                    p.row.account_id,
                                    p.row.folder_name,
                                    p.row.uid,
                                    c
                                ],
                            )
                            .map_err(|e| {
                                McpError::internal_error(format!("DB error: {}", e), None)
                            })?;
                            recategorized += 1;
                        }
                    }
                    if *read_changes {
                        tx.execute(
                            "UPDATE messages SET is_read = 1
                             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                            key,
                        )
                        .map_err(|e| {
                            McpError::internal_error(format!("DB error: {}", e), None)
                        })?;
                        marked_read += 1;
                    }
                    if *flag_changes {
                        tx.execute(
                            "UPDATE messages SET is_flagged = 1
                             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                            key,
                        )
                        .map_err(|e| {
                            McpError::internal_error(format!("DB error: {}", e), None)
                        })?;
                        marked_flagged += 1;
                    }
                }
                tx.commit().map_err(|e| {
                    McpError::internal_error(format!("DB error: {}", e), None)
                })?;
            }
        } else {
            // Report what a real run would do, computed from the same plan.
            for (_, cat_changes, read_changes, flag_changes) in &to_write {
                if *cat_changes {
                    recategorized += 1;
                }
                if *read_changes {
                    marked_read += 1;
                }
                if *flag_changes {
                    marked_flagged += 1;
                }
            }
        }

        // Push \Seen / \Flagged, strictly AFTER every transaction has
        // committed — a socket blocking inside the chunked write is the shape
        // of gotcha #11 (the app holds this same SQLite file open). Drop the
        // connection outright so nothing can hold it across the network.
        drop(conn);
        for ((account_id, folder), (seen_uids, flagged_uids)) in push_groups {
            let Some(account) = push_accounts.get(&account_id) else {
                // Already recorded above when the account row failed to load.
                push_failed += seen_uids.len() + flagged_uids.len();
                continue;
            };
            let email = account.email.as_str();
            let mut session = match imap::connect_for_account(account).await {
                Ok(s) => s,
                Err(e) => {
                    push_failed += seen_uids.len() + flagged_uids.len();
                    push_errors.push(format!("{email} {folder}: IMAP connect failed: {e}"));
                    continue;
                }
            };
            if let Err(e) = imap::select_folder(&mut session, &folder).await {
                push_failed += seen_uids.len() + flagged_uids.len();
                push_errors.push(format!("{email} {folder}: SELECT failed: {e}"));
                let _ = imap::disconnect(session, &account.email).await;
                continue;
            }
            for (uids, flag) in [(&seen_uids, "\\Seen"), (&flagged_uids, "\\Flagged")] {
                for uid in uids {
                    match imap::store_flags(&mut session, *uid, true, flag).await {
                        Ok(()) => {
                            if flag == "\\Seen" {
                                pushed_read += 1;
                            } else {
                                pushed_flagged += 1;
                            }
                        }
                        Err(e) => {
                            push_failed += 1;
                            push_errors.push(format!("{email} {folder} uid {uid}: {flag} {e}"));
                        }
                    }
                }
            }
            let _ = imap::disconnect(session, &account.email).await;
        }
        // A per-message error list can be as long as the apply; keep the
        // result readable and say how many were elided.
        let push_errors_total = push_errors.len();
        push_errors.truncate(5);

        let out = serde_json::json!({
            "rule": { "id": params.id, "name": rule.name, "is_active": rule.is_active },
            "dry_run": dry_run,
            "scope": scope_account
                .as_deref()
                .map(|a| format!("account:{}", a))
                .unwrap_or_else(|| "all accounts".to_string()),
            "messages_scanned": scanned,
            "inbox_messages_total": inbox_total,
            "matched": matched,
            "skipped_user_override": skipped_user_override,
            "changed": would_change,
            "recategorized": recategorized,
            "marked_read": marked_read,
            "marked_flagged": marked_flagged,
            "imap_pushed_read": pushed_read,
            "imap_pushed_flagged": pushed_flagged,
            "imap_push_failed": push_failed,
            "imap_push_errors_shown": push_errors.len(),
            "imap_push_errors_total": push_errors_total,
            "imap_push_errors": push_errors,
            "affected_shown": affected.len(),
            "affected": affected,
            "note": if dry_run {
                format!(
                    "DRY RUN — nothing was written and nothing was pushed to IMAP. A real run \
                     (dry_run=false) would change {} of {} matched message(s), and would push \
                     \\Seen/\\Flagged to the server for any read/flag change so it survives the \
                     next sync. {} match(es) carry a manual category override and are skipped \
                     either way. Matches not counted as changed are already in the target state.",
                    would_change, matched, skipped_user_override
                )
            } else {
                format!(
                    "Applied to {} of {} matched message(s). {} skipped for a manual category \
                     override. Open folders in the app may need a refresh to show the change.{}",
                    would_change,
                    matched,
                    skipped_user_override,
                    if push_failed > 0 {
                        format!(
                            " WARNING — PARTIAL: the local writes all landed (category is local by \
                             nature and is permanent), but {} of {} flag push(es) to IMAP FAILED, \
                             so those messages will revert to unread/unflagged on the next sync, \
                             which reconciles flags FROM the server. {} \\Seen and {} \\Flagged did \
                             reach the server. See imap_push_errors; re-run this tool once IMAP is \
                             reachable (it is idempotent — already-correct messages are skipped) \
                             or use flag_email per message.",
                            push_failed,
                            push_failed + pushed_read + pushed_flagged,
                            pushed_read,
                            pushed_flagged
                        )
                    } else if pushed_read > 0 || pushed_flagged > 0 {
                        format!(
                            " {} \\Seen and {} \\Flagged were also pushed to IMAP, so the read/flag \
                             changes survive the next sync's flag reconciliation.",
                            pushed_read, pushed_flagged
                        )
                    } else {
                        String::new()
                    }
                )
            },
        });

        let json = serde_json::to_string_pretty(&out)
            .map_err(|e| McpError::internal_error(format!("Serialize failed: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

/// Resolve the caller-supplied save_path into an absolute file path.
///
/// - Empty/omitted → `~/Downloads/<filename>`.
/// - Trailing slash or an existing directory → treat as a directory, append filename.
/// - Otherwise → treat as a full file path.
/// - Rejects paths that escape the user's home directory (e.g. `..` traversal,
///   or absolute paths outside $HOME) to contain MCP callers.
fn resolve_save_path(save_path: Option<&str>, filename: &str) -> Result<PathBuf, McpError> {
    let safe_filename = sanitize_filename_component(filename);

    let candidate = match save_path.map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            let base = dirs::download_dir()
                .or_else(dirs::home_dir)
                .ok_or_else(|| McpError::internal_error("Could not resolve ~/Downloads", None))?;
            base.join(&safe_filename)
        }
        Some(raw) => {
            let p = PathBuf::from(raw);
            let treat_as_dir =
                raw.ends_with('/') || raw.ends_with(std::path::MAIN_SEPARATOR) || p.is_dir();
            if treat_as_dir {
                p.join(&safe_filename)
            } else {
                p
            }
        }
    };

    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        std::env::current_dir()
            .map_err(|e| McpError::internal_error(format!("cwd unavailable: {}", e), None))?
            .join(candidate)
    };

    let home = dirs::home_dir()
        .ok_or_else(|| McpError::internal_error("Could not resolve home directory", None))?;
    if !path_is_within(&absolute, &home) {
        return Err(McpError::invalid_params(
            format!(
                "save_path must stay inside {}; refusing to write to {}",
                home.display(),
                absolute.display()
            ),
            None,
        ));
    }
    Ok(absolute)
}

fn sanitize_filename_component(name: &str) -> String {
    // Strip anything path-like from a filename the server pulled off the wire.
    let last = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment.bin");
    if last.is_empty() {
        "attachment.bin".to_string()
    } else {
        last.to_string()
    }
}

/// Validate caller-supplied attachment paths and read each file's bytes.
/// Each path must be absolute and stay inside `$HOME`; per-file 25 MB cap is
/// enforced inside `read_attachment_from_path`. Returns
/// (filename, content_type, bytes) tuples ready to feed into
/// `MessageBuilder::attachment`.
fn read_attachment_paths(paths: &[String]) -> Result<Vec<(String, String, Vec<u8>)>, McpError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let home = dirs::home_dir()
        .ok_or_else(|| McpError::internal_error("Could not resolve home directory", None))?;
    let mut out = Vec::with_capacity(paths.len());
    for raw in paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(McpError::invalid_params(
                "Attachment path cannot be empty",
                None,
            ));
        }
        let abs = PathBuf::from(trimmed);
        if !abs.is_absolute() {
            return Err(McpError::invalid_params(
                format!("Attachment path must be absolute: {}", trimmed),
                None,
            ));
        }
        if !path_is_within(&abs, &home) {
            return Err(McpError::invalid_params(
                format!(
                    "Attachment path must stay inside {}; refusing {}",
                    home.display(),
                    abs.display()
                ),
                None,
            ));
        }
        let tuple = crate::email::attachment_file::read_attachment_from_path(&abs.to_string_lossy())
            .map_err(|e| {
                McpError::internal_error(format!("Read attachment {}: {}", abs.display(), e), None)
            })?;
        out.push(tuple);
    }
    Ok(out)
}

fn path_is_within(candidate: &Path, root: &Path) -> bool {
    // Walk `candidate` resolving `.` / `..` without requiring the file to exist yet.
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    for comp in candidate.components() {
        match comp {
            std::path::Component::Prefix(p) => stack.push(p.as_os_str().to_os_string()),
            std::path::Component::RootDir => stack.push(std::ffi::OsString::from("/")),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                stack.pop();
            }
            std::path::Component::Normal(part) => stack.push(part.to_os_string()),
        }
    }
    let mut normalized = PathBuf::new();
    for part in stack {
        normalized.push(part);
    }
    normalized.starts_with(root)
}

#[tool_handler]
impl ServerHandler for CxMailMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // A real client connected — mark claimed so the orphan watchdog won't
        // reap us, then reproduce the default handshake (record peer info,
        // return server info).
        self.claimed.store(true, Ordering::Relaxed);
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        Ok(self.get_info())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_project_repo: three states that must not blur together ──

    fn a_resolved_repo() -> db::claude_repos::ResolvedRepo {
        db::claude_repos::ResolvedRepo {
            repo_path: "/Users/x/Projects/northwind".into(),
            scope: db::claude_repos::RepoScope::Contact,
            source: "northwind.example".into(),
        }
    }

    #[test]
    fn a_usable_repo_names_its_path_the_rule_and_the_claude_md() {
        let v = project_repo_payload(Some(&a_resolved_repo()), true, true);
        assert_eq!(v["status"], "mapped");
        assert_eq!(v["repo_path"], "/Users/x/Projects/northwind");
        assert_eq!(v["chosen_by"], "contact northwind.example");
        assert_eq!(v["claude_md"], "/Users/x/Projects/northwind/CLAUDE.md");
    }

    /// A stale mapping must not hand the agent a path to go reading.
    #[test]
    fn a_missing_directory_is_not_reported_as_mapped() {
        let v = project_repo_payload(Some(&a_resolved_repo()), false, false);
        assert_eq!(v["status"], "missing");
        assert!(v.get("claude_md").is_none(), "{v}");
        assert!(v["next_step"].as_str().unwrap().contains("Tell the user"), "{v}");
    }

    /// The in-app chat reads a draft's folder and UID back out of these two
    /// result lines (`email::chat_agent::extract_draft_ref`) to offer "Open
    /// in compose". Rewording either silently removes that button — change
    /// both sides together.
    #[test]
    fn draft_result_wording_the_chat_panel_parses_is_unchanged() {
        let src = include_str!("server.rs");
        assert!(src.contains("\"Draft saved to {} (UID {}) for {}{} — draft_id"));
        assert!(src.contains("\"Draft UID {} replaced with UID {} in {} for {}{} — draft_id"));
    }

    #[test]
    fn unmapped_says_ask_never_guess() {
        let v = project_repo_payload(None, false, false);
        assert_eq!(v["status"], "unmapped");
        assert!(v.get("repo_path").is_none(), "{v}");
        assert!(v["next_step"].as_str().unwrap().contains("never guess"), "{v}");
    }

    // ── resolve_authored_source: body XOR instruction ──
    // Every refusal must say "nothing was written/changed" and name the
    // remedy (#57's convention); each arm below is mutation-tested by hand —
    // deleting a check in `resolve_authored_source` fails its named test.

    #[test]
    fn authored_source_takes_exactly_one_of_body_or_instruction() {
        let both = resolve_authored_source(
            Some("text".into()),
            Some("say hi".into()),
            None,
            None,
            None,
            "default-model",
        );
        let err = format!("{:?}", both.unwrap_err());
        assert!(err.contains("nothing was written"), "{err}");

        let neither = resolve_authored_source(None, None, None, None, None, "default-model");
        let err = format!("{:?}", neither.unwrap_err());
        assert!(err.contains("nothing was written"), "{err}");
    }

    #[test]
    fn an_explicit_body_passes_through_untouched() {
        match resolve_authored_source(
            Some("Hi.".into()),
            None,
            None,
            Some("rich"),
            Some(true),
            "default-model",
        ) {
            Ok(AuthoredSource::Explicit(b)) => assert_eq!(b, "Hi."),
            other => panic!("explicit body must stay explicit: {other:?}"),
        }
    }

    #[test]
    fn an_instruction_yields_the_passed_default_model() {
        // The handlers pass `effective_default_model()` here, so this is the
        // "configured in the app settings" half of the precedence.
        match resolve_authored_source(
            None,
            Some("confirm the call".into()),
            None,
            None,
            None,
            "configured-in-settings",
        ) {
            Ok(AuthoredSource::Generated { instruction, model }) => {
                assert_eq!(instruction, "confirm the call");
                assert_eq!(model, "configured-in-settings");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_flag_shaped_model_is_refused_before_any_work() {
        // Per-call value and corrupt configured default both land here.
        let bad = resolve_authored_source(
            None,
            Some("i".into()),
            Some("--dangerously-skip-permissions".into()),
            None,
            None,
            "default-model",
        );
        let err = format!("{:?}", bad.unwrap_err());
        assert!(err.contains("nothing was written"), "{err}");
        assert!(resolve_authored_source(
            None,
            Some("i".into()),
            None,
            None,
            None,
            "--broken-default"
        )
        .is_err());
    }

    #[test]
    fn writer_model_overrides_the_default_and_is_refused_without_instruction() {
        match resolve_authored_source(
            None,
            Some("i".into()),
            Some("gemini-3.6-flash-low".into()),
            None,
            None,
            "default-model",
        ) {
            Ok(AuthoredSource::Generated { model, .. }) => {
                assert_eq!(model, "gemini-3.6-flash-low")
            }
            other => panic!("{other:?}"),
        }
        // With `body`, writer_model is confusion, not a no-op — refuse it.
        assert!(resolve_authored_source(
            Some("text".into()),
            None,
            Some("gemini-3.7-flash-high".into()),
            None,
            None,
            "default-model"
        )
        .is_err());
    }

    #[test]
    fn instruction_refuses_layout_and_html_because_the_writer_emits_prose() {
        let with_layout =
            resolve_authored_source(None, Some("i".into()), None, Some("card"), None, "default-model");
        let err = format!("{:?}", with_layout.unwrap_err());
        assert!(err.contains("plain prose"), "{err}");

        assert!(resolve_authored_source(None, Some("i".into()), None, None, Some(true), "default-model")
            .is_err());
        // is_html=false is compatible — it states what the writer produces.
        assert!(resolve_authored_source(None, Some("i".into()), None, None, Some(false), "default-model")
            .is_ok());
    }

    #[test]
    fn a_blank_instruction_is_refused_not_sent_to_the_writer() {
        assert!(resolve_authored_source(None, Some("   ".into()), None, None, None, "default-model").is_err());
    }

    // ── edit_draft: append first, targeted expunge, stale-UID preflight ──
    // (gotcha #57)

    /// The `edit_draft` handler, cut out of this file's own source — the
    /// ordering is the invariant, and only the source can pin an ordering.
    fn edit_draft_source() -> &'static str {
        let src = include_str!("server.rs");
        let start = src.find("async fn edit_draft(").expect("edit_draft exists");
        let rest = &src[start..];
        // The handler ends where the next handler begins.
        let end = rest[1..]
            .find("\n    async fn ")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn edit_draft_checks_the_old_uid_then_appends_then_deletes() {
        let body = edit_draft_source();
        let claim = body
            .find("db::drafts::claim(")
            .expect("the v60 claim is present");
        let connect = body
            .find("imap::connect_for_account(")
            .expect("IMAP connect is present");
        let preflight = body
            .find("imap::uid_exists(")
            .expect("stale-UID preflight is present");
        let append = body.find(".append(").expect("APPEND is present");
        let delete = body
            .find("\"\\\\Deleted\"")
            .expect("old-UID delete is present");
        assert!(
            claim < connect,
            "the claim (compare-and-set on current_uid) must be taken before any IMAP work"
        );
        assert!(
            preflight < append,
            "the stale-UID preflight must run before the APPEND, or a stale UID forks the draft"
        );
        assert!(
            append < delete,
            "the APPEND must precede the old UID's delete — the other order loses the draft on any failure in between"
        );
    }

    // ── send-as (`from`) ────────────────────────────────────────────────

    /// The From is resolved before the claim and before any IMAP call. An
    /// address the account cannot send as is a caller error, and a caller
    /// error must not cost the draft its claim or leave a revision behind
    /// (#30/#57's ordering rule, applied to a third parameter).
    #[test]
    fn edit_draft_resolves_the_from_before_the_claim_and_before_imap() {
        let body = edit_draft_source();
        let resolve = body
            .find("resolve_draft_from(")
            .expect("edit_draft resolves its From");
        let claim = body
            .find("db::drafts::claim(")
            .expect("the v60 claim is present");
        let connect = body
            .find("imap::connect_for_account(")
            .expect("IMAP connect is present");
        assert!(
            resolve < claim && resolve < connect,
            "an unconfigured From must be refused before anything is claimed or written"
        );
    }

    /// The rebuilt revision must carry the resolved address, not the account's
    /// own — that hardcode is exactly the bug this feature fixes, and it is
    /// the one line that decides what lands on the server.
    #[test]
    fn edit_draft_writes_the_resolved_from_not_the_account_address() {
        let body = edit_draft_source();
        assert!(
            !body.contains("message.from(account.email"),
            "the From header must come from the resolved send-as address"
        );
        assert!(
            body.contains("message.from(from_addr.email.as_str())"),
            "the resolved address is what reaches the MIME builder"
        );
    }

    /// Silence for the ordinary case, so the note means something when it
    /// appears (#34's reasoning) — and when it does appear it says WHY, since
    /// "CXMail picked a different From than you'd expect" is the whole
    /// surprise this feature can cause.
    #[test]
    fn the_from_note_is_silent_on_the_primary_and_explains_itself_otherwise() {
        let primary = db::identities::SendAsAddress {
            account_id: "acct".into(),
            email: "sidalias@icloud.com".into(),
            display_name: None,
            signature_html: None,
            is_primary: true,
            identity_id: None,
            from_sent: false,
        };
        let alias = db::identities::SendAsAddress {
            email: "rileyprime@icloud.com".into(),
            is_primary: false,
            ..primary.clone()
        };
        assert_eq!(send_as_note(&primary, false), "");
        assert_eq!(send_as_note(&primary, true), "");

        let defaulted = send_as_note(&alias, false);
        assert!(defaulted.contains("rileyprime@icloud.com"), "{defaulted}");
        assert!(
            defaulted.contains("the original was sent to"),
            "a From the caller did not ask for has to say where it came from: {defaulted}"
        );
        assert!(send_as_note(&alias, true).contains("as requested"));
    }

    /// An alias with no signature of its own inherits the account's rather
    /// than sending unsigned — the pre-alias world had exactly one signature
    /// and nobody configuring an alias is asking to lose it.
    #[test]
    fn an_alias_without_its_own_signature_inherits_the_accounts() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'sidalias@icloud.com', 'icloud', 'h', 'h')",
            [],
        )
        .unwrap();
        db::identities::insert(
            &conn,
            &db::identities::Identity {
                id: None,
                account_id: "acct".into(),
                email: "sidalias@icloud.com".into(),
                display_name: None,
                signature_html: Some("<p>account sig</p>".into()),
                is_default: true,
            },
        )
        .unwrap();

        let bare = db::identities::SendAsAddress {
            account_id: "acct".into(),
            email: "rileyprime@icloud.com".into(),
            display_name: None,
            signature_html: None,
            is_primary: false,
            identity_id: None,
            from_sent: false,
        };
        assert_eq!(
            signature_for_send_as(&conn, "acct", &bare).as_deref(),
            Some("<p>account sig</p>")
        );

        let own = db::identities::SendAsAddress {
            signature_html: Some("<p>alias sig</p>".into()),
            ..bare.clone()
        };
        assert_eq!(
            signature_for_send_as(&conn, "acct", &own).as_deref(),
            Some("<p>alias sig</p>"),
            "an alias that HAS a signature uses its own"
        );

        // Blank is not a signature — it must not shadow the account's.
        let blank = db::identities::SendAsAddress {
            signature_html: Some("   ".into()),
            ..bare.clone()
        };
        assert_eq!(
            signature_for_send_as(&conn, "acct", &blank).as_deref(),
            Some("<p>account sig</p>")
        );
    }

    /// T207's done-criterion, end to end through the MCP's own resolver: a
    /// reply to a stored message that was addressed to the alias comes out
    /// FROM the alias, with no `from` given — and an explicit `from` still
    /// wins, and an address the account cannot send as is refused.
    #[test]
    fn a_reply_to_mail_sent_to_an_alias_drafts_from_the_alias() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'sidalias@icloud.com', 'icloud', 'h', 'h')",
            [],
        )
        .unwrap();
        db::identities::insert(
            &conn,
            &db::identities::Identity {
                id: None,
                account_id: "acct".into(),
                email: "rileyprime@icloud.com".into(),
                display_name: None,
                signature_html: None,
                is_default: false,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_email, to_list, date)
             VALUES ('acct', 'INBOX', 42, 'Hello', 'friend@example.com',
                     '[{\"name\":null,\"email\":\"RileyPrime@icloud.com\"}]',
                     '2026-09-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let account = db::accounts::get_by_id(&conn, "acct").unwrap().unwrap();
        let target = ReplyTarget::Coordinates {
            folder: "INBOX".into(),
            uid: 42,
        };

        let hit = resolve_draft_from(&conn, &account, None, Some(&target), None).unwrap();
        assert_eq!(hit.email, "rileyprime@icloud.com");
        assert!(!hit.is_primary);

        let explicit =
            resolve_draft_from(&conn, &account, Some("sidalias@icloud.com"), Some(&target), None)
                .unwrap();
        assert!(explicit.is_primary, "an explicit from outranks the reply default");

        assert!(
            resolve_draft_from(&conn, &account, Some("nobody@example.com"), None, None).is_err(),
            "an unconfigured From must be refused, not silently corrected"
        );

        let fresh = resolve_draft_from(&conn, &account, None, None, None).unwrap();
        assert!(fresh.is_primary, "a new draft with no reply target stays on the primary");
    }

    #[test]
    fn edit_draft_expunges_only_the_old_uid() {
        let body = edit_draft_source();
        assert!(
            body.contains("imap::uid_expunge("),
            "the old revision is removed with a targeted UID EXPUNGE"
        );
        assert!(
            !body.contains("imap::expunge("),
            "a mailbox-wide EXPUNGE would also purge whatever another client marked \\Deleted"
        );
    }

    #[test]
    fn edit_draft_target_is_draft_id_xor_uid() {
        assert_eq!(
            resolve_edit_target(Some("d1"), None, None).unwrap(),
            EditTarget::ById {
                draft_id: "d1".into(),
                expected_uid: None
            }
        );
        assert_eq!(
            resolve_edit_target(Some(" d1 "), Some(9), None).unwrap(),
            EditTarget::ById {
                draft_id: "d1".into(),
                expected_uid: Some(9)
            }
        );
        assert_eq!(
            resolve_edit_target(None, None, Some(7)).unwrap(),
            EditTarget::ByUid(7)
        );
        // An empty draft_id is no draft_id.
        assert_eq!(
            resolve_edit_target(Some("  "), None, Some(7)).unwrap(),
            EditTarget::ByUid(7)
        );
        for (id, expected, uid) in [
            (None, None, None),
            (Some("d1"), None, Some(7)),
            (None, Some(9), Some(7)),
        ] {
            let msg = resolve_edit_target(id, expected, uid)
                .unwrap_err()
                .message
                .to_string();
            assert!(msg.contains("Nothing was changed"), "{msg}");
        }
    }

    #[test]
    fn conflict_messages_say_nothing_changed_and_name_the_next_step() {
        let unknown = unknown_draft_error("abc").message.to_string();
        assert!(unknown.contains("nothing was changed") && unknown.contains("search_emails"), "{unknown}");
        let busy = draft_busy_error("abc", "2026-08-25T00:00:00.000Z").message.to_string();
        assert!(busy.contains("nothing was changed") && busy.contains("Retry"), "{busy}");

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (account_id TEXT, folder_name TEXT, uid INTEGER, subject TEXT, to_list TEXT);
             INSERT INTO messages VALUES ('acct', 'Drafts', 12, 'Re: the deck', '[\"dana@northwind.example\"]');",
        )
        .unwrap();
        let row = db::drafts::DraftRow {
            draft_id: "abc".into(),
            account_id: "acct".into(),
            folder_name: "Drafts".into(),
            uidvalidity: 1,
            current_uid: 12,
        };
        let moved = draft_moved_error(&conn, &row, 10, 12).message.to_string();
        assert!(moved.contains("expected UID 10"), "{moved}");
        assert!(moved.contains("current revision is UID 12"), "{moved}");
        assert!(moved.contains("Re: the deck"), "{moved}");
        assert!(moved.contains("expected_uid=12"), "{moved}");
        assert!(moved.contains("Nothing was changed"), "{moved}");
        // Not synced yet: still a usable message, and honest about the gap.
        let unsynced = draft_moved_error(&conn, &row, 10, 13).message.to_string();
        assert!(unsynced.contains("not in the local cache yet"), "{unsynced}");
    }

    #[test]
    fn stale_draft_error_says_nothing_changed_and_names_the_remedy() {
        for local in [true, false] {
            let err = stale_draft_error(739, "[Gmail]/Drafts", "me@example.com", local);
            let msg = err.message.to_string();
            assert!(msg.contains("UID 739"), "{msg}");
            assert!(msg.contains("[Gmail]/Drafts"), "{msg}");
            assert!(msg.contains("nothing was changed"), "{msg}");
            assert!(msg.contains("search_emails"), "{msg}");
            assert!(msg.contains("retry edit_draft"), "{msg}");
        }
        let gone = stale_draft_error(1, "Drafts", "a@b", false).message.to_string();
        let present = stale_draft_error(1, "Drafts", "a@b", true).message.to_string();
        assert!(gone.contains("compose window"), "{gone}");
        assert!(present.contains("since the last sync"), "{present}");
        assert_ne!(gone, present);
    }

    // ── hidden accounts in an unscoped search ────────────────────────────

    fn account(id: &str, email: &str, hidden: bool) -> db::accounts::Account {
        db::accounts::Account {
            id: id.to_string(),
            email: email.to_string(),
            display_name: None,
            provider: "gmail".to_string(),
            imap_host: "imap.gmail.com".to_string(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            imap_security: "implicit".to_string(),
            smtp_security: "starttls".to_string(),
            imap_username: None,
            smtp_username: None,
            color: None,
            is_active: true,
            sort_order: 0,
            group_name: None,
            notify_enabled: true,
            track_opens_enabled: false,
            hidden_from_aggregates: hidden,
            triage_enabled: false,
        }
    }

    #[test]
    fn hidden_accounts_note_is_silent_when_nothing_was_skipped() {
        assert_eq!(hidden_accounts_note(&[]), None);
    }

    #[test]
    fn hidden_accounts_note_names_the_email_and_id_and_the_remedy() {
        let note = hidden_accounts_note(&[account("acc-warm", "chris@trycxventures.com", true)])
            .expect("one hidden account produces a note");
        assert_eq!(
            note,
            "(Skipped 1 account hidden from aggregated views: chris@trycxventures.com [ID: acc-warm]. Pass account_ids to search it.)"
        );

        let two = hidden_accounts_note(&[
            account("acc-warm", "chris@trycxventures.com", true),
            account("acc-other", "other@example.com", true),
        ])
        .unwrap();
        assert!(two.starts_with("(Skipped 2 accounts hidden"), "{two}");
        assert!(two.contains("other@example.com [ID: acc-other]"), "{two}");
        assert!(two.ends_with("Pass account_ids to search them.)"), "{two}");
    }

    // ── the house dash rule ───────────────────────────────────────────────

    #[test]
    fn enforce_dash_rule_rewrites_the_authored_body_and_says_so() {
        let mut subject = "August review".to_string();
        // The sentence the Northwind voice pass actually handed back.
        let mut body =
            "all 47 posts fetched and parsed without failures—and identifies the two decisions"
                .to_string();

        let note = enforce_dash_rule(&mut subject, &mut body);

        assert_eq!(
            body,
            "all 47 posts fetched and parsed without failures, and identifies the two decisions"
        );
        assert!(note.contains("Rewrote 1 long dash"), "note was: {note}");
        assert!(
            note.contains("failures, and identifies"),
            "the note must show what changed: {note}"
        );
    }

    #[test]
    fn enforce_dash_rule_covers_the_subject_line_too() {
        let mut subject = "Northwind roadmap — an August review".to_string();
        let mut body = "clean prose".to_string();

        let note = enforce_dash_rule(&mut subject, &mut body);

        assert_eq!(subject, "Northwind roadmap, an August review");
        assert_eq!(body, "clean prose");
        assert!(!note.is_empty());
    }

    #[test]
    fn enforce_dash_rule_is_silent_and_inert_on_clean_text() {
        let mut subject = "August review".to_string();
        let mut body = "Plain prose, with commas: and colons (and parentheses).".to_string();

        assert_eq!(enforce_dash_rule(&mut subject, &mut body), "");
        assert_eq!(subject, "August review");
        assert_eq!(body, "Plain prose, with commas: and colons (and parentheses).");
    }

    /// The ordering guard. `format_quoted_history` is a byte-contract with the
    /// frontend (gotcha #30) and the quoted text is the OTHER person's prose;
    /// both handlers therefore normalize the authored body and only then append
    /// the signature and the quote.
    #[test]
    fn the_quoted_original_is_out_of_the_dash_rules_reach() {
        let (quote_html, _) = format_quoted_history(
            Some("Dana"),
            Some("dana@example.com"),
            "2026-08-18T19:46:57+00:00",
            Some("<p>their prose — with a dash</p>"),
            None,
        );

        let mut subject = "Re: numbers".to_string();
        let mut body = "our prose — with a dash".to_string();
        enforce_dash_rule(&mut subject, &mut body);
        assert_eq!(body, "our prose, with a dash");

        let composed = format!("{body}{quote_html}");
        assert!(
            composed.ends_with(&quote_html),
            "the quote is appended verbatim, after normalization"
        );
        assert!(
            composed.contains("their prose — with a dash"),
            "the quoted original keeps its author's punctuation"
        );

        // Why that order is load-bearing rather than incidental: normalizing
        // the composed message would rewrite the other person's words and
        // break the byte-contract. If this assertion ever fails, the ordering
        // requirement has changed and this comment is wrong.
        let mut no_subject = String::new();
        let mut late = composed.clone();
        enforce_dash_rule(&mut no_subject, &mut late);
        assert!(
            !late.contains("their prose — with a dash"),
            "normalizing after the append is destructive; that is the point of the order"
        );
    }

    /// The header block a `read_email_source` caller actually gets. Folded
    /// continuations and repeated `Received` hops are the normal case, not an
    /// edge case.
    const SAMPLE_HEADERS: &str = "Received: from a.example (a.example [203.0.113.1])\r\n\
         \tby mx.google.com with ESMTPS id hop1\r\n\
         Received: from b.example (b.example [198.51.100.2])\r\n\
         \tby a.example with SMTP id hop2\r\n\
         Authentication-Results: mx.google.com;\r\n\
         \tspf=pass smtp.mailfrom=example.com;\r\n\
         \tdmarc=pass (p=REJECT) header.from=example.com\r\n\
         Subject: quarterly numbers\r\n\
         From: sender@example.com";

    #[test]
    fn filter_header_lines_keeps_folded_continuations() {
        let out = filter_header_lines(
            SAMPLE_HEADERS,
            &["authentication-results".to_string()],
        );
        assert!(out.contains("Authentication-Results: mx.google.com;"));
        assert!(
            out.contains("dmarc=pass (p=REJECT)"),
            "the verdict lives on a continuation line and is the whole point: {out}"
        );
        assert!(
            !out.contains("Subject:"),
            "must stop at the next unfolded header: {out}"
        );
    }

    #[test]
    fn filter_header_lines_is_case_insensitive_and_tolerates_a_trailing_colon() {
        for name in ["Authentication-Results", "AUTHENTICATION-RESULTS:", " subject "] {
            let out = filter_header_lines(SAMPLE_HEADERS, &[name.to_string()]);
            assert!(!out.is_empty(), "no match for {name:?}");
        }
    }

    #[test]
    fn filter_header_lines_returns_every_occurrence_in_order() {
        let out = filter_header_lines(SAMPLE_HEADERS, &["received".to_string()]);
        assert_eq!(
            out.matches("Received:").count(),
            2,
            "both hops must be returned: {out}"
        );
        assert!(out.find("hop1").unwrap() < out.find("hop2").unwrap());
    }

    #[test]
    fn filter_header_lines_returns_empty_when_nothing_matches() {
        assert!(filter_header_lines(SAMPLE_HEADERS, &["x-nope".to_string()]).is_empty());
        // An empty/blank request list must not be read as "match everything" —
        // the tool treats empty as "full block" before calling this, so a
        // silent match-all here would double up.
        assert!(filter_header_lines(SAMPLE_HEADERS, &["  ".to_string()]).is_empty());
    }

    #[test]
    fn redact_data_uris_replaces_base64_payload_with_placeholder() {
        let html = r#"<p>hi</p><img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==" alt="x">"#;
        let out = redact_data_uris(html);
        assert!(!out.contains("iVBORw0KGgoAAAANSUhEUg=="), "base64 leaked: {out}");
        assert!(out.contains("<p>hi</p>"));
        assert!(out.contains(r#"<img src="data:[inline image omitted, "#));
        assert!(out.contains(r#" bytes]" alt="x">"#));
    }

    #[test]
    fn redact_data_uris_leaves_plain_html_untouched() {
        let html = "<p>no images here, just text</p>";
        assert_eq!(redact_data_uris(html), html);
    }

    #[test]
    fn redact_data_uris_handles_multiple_images() {
        let html = r#"<img src="data:image/png;base64,AAAA"><img src="data:image/jpeg;base64,BBBB">"#;
        let out = redact_data_uris(html);
        assert!(!out.contains("AAAA") && !out.contains("BBBB"));
        assert_eq!(out.matches("inline image omitted").count(), 2);
    }

    #[test]
    fn body_to_html_paragraphs_lf() {
        assert_eq!(body_to_html("Hi\n\nThere"), "<p>Hi</p><p>There</p>");
    }

    #[test]
    fn body_to_html_paragraphs_crlf() {
        assert_eq!(body_to_html("Hi\r\n\r\nThere"), "<p>Hi</p><p>There</p>");
    }

    #[test]
    fn body_to_html_intra_paragraph_soft_break() {
        assert_eq!(body_to_html("One\nTwo"), "<p>One<br/>Two</p>");
    }

    #[test]
    fn body_to_html_blank_only_yields_empty() {
        assert_eq!(body_to_html("\n\n  \n\n"), "");
        assert_eq!(body_to_html(""), "");
    }

    #[test]
    fn body_to_html_escapes_special_chars() {
        assert_eq!(
            body_to_html("a < b & c > d"),
            "<p>a &lt; b &amp; c &gt; d</p>"
        );
    }

    #[test]
    fn body_to_html_html_passthrough_is_sanitized() {
        // The HTML branch must run through ammonia. Script tags must be stripped.
        let out = body_to_html("<script>alert(1)</script><p>ok</p>");
        assert!(!out.contains("<script"), "script survived sanitizer: {out}");
        assert!(out.contains("ok"));
    }

    fn addr(name: Option<&str>, email: &str) -> parser::EmailAddress {
        parser::EmailAddress {
            name: name.map(|s| s.to_string()),
            email: email.to_string(),
        }
    }

    #[test]
    fn recipient_lines_render_every_to_and_cc() {
        // The "Sam" incident: a reply went out to only the first To + the Cc,
        // silently dropping a SECOND To recipient. read_email must expose ALL
        // recipients so an agent rebuilding a reply-all can't omit one.
        let to = vec![
            addr(Some("Dana"), "dana@example.com"),
            addr(None, "sam@example.com"), // the one that was dropped
        ];
        let cc = vec![addr(Some("Morgan"), "morgan@example.com")];
        let out = fmt_recipient_lines(&to, &cc, &[]);
        assert_eq!(
            out,
            "To: Dana <dana@example.com>, sam@example.com\nCc: Morgan <morgan@example.com>\n"
        );
        // Explicitly assert the previously-dropped recipient is present.
        assert!(out.contains("sam@example.com"), "second To dropped: {out}");
    }

    #[test]
    fn recipient_lines_include_bcc_and_skip_empty() {
        let out = fmt_recipient_lines(
            &[addr(None, "a@x.com")],
            &[],
            &[addr(None, "secret@x.com")],
        );
        assert_eq!(out, "To: a@x.com\nBcc: secret@x.com\n");
        // No Cc line when Cc is empty.
        assert!(!out.contains("Cc:"), "empty Cc should be skipped: {out}");
    }

    #[test]
    fn recipient_lines_empty_when_no_recipients() {
        assert_eq!(fmt_recipient_lines(&[], &[], &[]), "");
    }

    #[test]
    fn parse_addr_json_roundtrips_and_tolerates_garbage() {
        let json = serde_json::to_string(&vec![
            addr(Some("A"), "a@x.com"),
            addr(None, "b@x.com"),
        ])
        .unwrap();
        let parsed = parse_addr_json(Some(&json));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].email, "b@x.com");
        // NULL / empty / corrupt inputs degrade to empty, never panic.
        assert!(parse_addr_json(None).is_empty());
        assert!(parse_addr_json(Some("")).is_empty());
        assert!(parse_addr_json(Some("not json")).is_empty());
    }

    #[test]
    fn addr_json_compact_joins_emails_for_thread_view() {
        let json = serde_json::to_string(&vec![
            addr(Some("A"), "a@x.com"),
            addr(None, "b@x.com"),
        ])
        .unwrap();
        assert_eq!(fmt_addr_json_compact(Some(&json)), "a@x.com, b@x.com");
        assert_eq!(fmt_addr_json_compact(None), "");
    }

    #[test]
    fn sanitize_voice_excerpt_strips_quoted_lines() {
        let input = "My take: ship it.\n> Original message\n> More quote";
        let out = sanitize_voice_excerpt(input, 400);
        assert!(out.contains("ship it"));
        assert!(!out.contains("Original message"));
        assert!(!out.contains("More quote"));
    }

    #[test]
    fn sanitize_voice_excerpt_cuts_at_signature_marker() {
        let input = "Sounds good — see you Thursday.\n-- \nChris\nhttps://example.com";
        let out = sanitize_voice_excerpt(input, 400);
        assert!(out.contains("Thursday"));
        assert!(!out.contains("Chris"));
        assert!(!out.contains("example.com"));
    }

    #[test]
    fn sanitize_voice_excerpt_cuts_at_iphone_signature() {
        let input = "Yep, on it.\nSent from my iPhone";
        let out = sanitize_voice_excerpt(input, 400);
        assert!(out.contains("on it"));
        assert!(!out.contains("Sent from my"));
    }

    #[test]
    fn sanitize_voice_excerpt_caps_length() {
        let long = "a".repeat(800);
        let out = sanitize_voice_excerpt(&long, 100);
        // Char-count cap of 100. Last char is the ellipsis; so total ≤ 100 chars.
        assert!(out.chars().count() <= 100, "len {}", out.chars().count());
        assert!(out.ends_with('…'));
    }

    #[test]
    fn sanitize_voice_excerpt_short_passthrough() {
        let input = "Tiny note.";
        assert_eq!(sanitize_voice_excerpt(input, 400), "Tiny note.");
    }

    #[test]
    fn body_to_html_style_tag_input_does_not_panic() {
        let out = std::panic::catch_unwind(|| body_to_html("<style>p{color:red}</style><p>ok</p>"))
            .expect("body_to_html should catch sanitizer panics");
        assert!(
            !out.contains("<style"),
            "style tag survived sanitizer: {out}"
        );
        assert!(out.contains("ok"));
    }

    #[test]
    fn body_to_html_html_passthrough_preserves_clean_markup() {
        let out = body_to_html("<p>raw</p>");
        assert!(out.contains("<p>"));
        assert!(out.contains("raw"));
    }

    #[test]
    fn render_body_forces_html_when_flagged() {
        // A <br> with no closing tag would auto-detect as plain text and be escaped.
        // The explicit flag must preserve it as real markup.
        let out = render_body("Line one<br>Line two", Some(true));
        assert!(out.contains("<br"), "expected real <br>, got: {out}");
        assert!(!out.contains("&lt;br"), "tag should not be escaped: {out}");
    }

    #[test]
    fn render_body_forces_plain_when_flagged_false() {
        // Text containing "</" would auto-detect as HTML; the flag forces escaping.
        let out = render_body("if a </ b then x", Some(false));
        assert_eq!(out, "<p>if a &lt;/ b then x</p>");
    }

    #[test]
    fn render_body_none_matches_auto_detect() {
        assert_eq!(render_body("Hi\n\nThere", None), body_to_html("Hi\n\nThere"));
        assert_eq!(render_body("<p>ok</p>", None), body_to_html("<p>ok</p>"));
    }

    // ─── layout="card" / layout="rich" (styled draft) ──────────────────────
    //
    // These tests exercise the REAL production pipeline:
    //   apply_inline_font_styles(render_styled(...))
    // because `apply_inline_font_styles` runs on every draft after render_styled
    // and mutates the styled block's <div> styles. Asserting only on
    // render_styled() output would miss the OUR_DECL_RE strip step (see
    // email::inline_styles).
    use crate::email::inline_styles::apply_inline_font_styles as font_pass;

    #[test]
    fn render_styled_card_full_pipeline_keeps_class_and_chrome() {
        let styled = font_pass(&render_styled("Hello team", Some(false), "card"));
        // Load-bearing: the class is what re-claims the block as atomic in TipTap
        // on reopen, so the styled markup survives editor.getHTML() and resend.
        assert!(
            styled.contains(r#"class="cx-html-block""#),
            "preservation class dropped: {styled}"
        );
        // Card chrome survives the font pass (values chosen to dodge OUR_DECL_RE).
        assert!(styled.contains("max-width:600px"), "max-width dropped: {styled}");
        assert!(styled.contains("border-radius:8px"), "border-radius dropped: {styled}");
        assert!(styled.contains("padding:32px"), "padding dropped: {styled}");
        // Inner content rendered into the card.
        assert!(styled.contains("Hello team"), "inner content missing: {styled}");
    }

    #[test]
    fn render_styled_card_keeps_background_for_dark_mode() {
        let styled = font_pass(&render_styled("Body", Some(false), "card"));
        // EmailFrame's dark-mode rule inverts only block elements whose style
        // lacks the substring "background"; the white card must keep its
        // background-color so it doesn't flip to dark/light-text in CXMail's view.
        assert!(
            styled.contains("background-color:#ffffff"),
            "card background dropped: {styled}"
        );
        assert!(styled.contains("background"), "no background substring: {styled}");
    }

    #[test]
    fn render_styled_card_drops_data_marker_keeps_class() {
        // Documents that `class`, not the `data-*` marker, is the survivor:
        // data-cx-html is not in the sanitize allowlist, so it is stripped.
        let out = render_styled("Body", Some(false), "card");
        assert!(out.contains(r#"class="cx-html-block""#), "class missing: {out}");
        assert!(
            !out.contains("data-cx-html"),
            "data-cx-html should be stripped by sanitize: {out}"
        );
    }

    #[test]
    fn render_styled_card_html_inner_is_sanitized() {
        let styled = font_pass(&render_styled(
            "<h1>Welcome</h1><script>alert(1)</script>",
            Some(true),
            "card",
        ));
        assert!(styled.contains("Welcome"), "inner heading missing: {styled}");
        assert!(!styled.contains("<script"), "script survived sanitizer: {styled}");
        assert!(styled.contains(r#"class="cx-html-block""#), "class dropped: {styled}");
    }

    #[test]
    fn render_styled_rich_preserves_author_inline_styles() {
        // Free-form path: a complete styled fragment must survive verbatim through
        // both sanitize (render_styled) and the font pass — guarding against
        // OUR_DECL_RE stripping author declarations and against sanitize dropping
        // inline style values.
        let body = r#"<div style="background-color:#fef3c7;padding:20px;border-radius:12px;">Hi</div>"#;
        let styled = font_pass(&render_styled(body, Some(true), "rich"));
        assert!(
            styled.contains("background-color:#fef3c7"),
            "author background dropped: {styled}"
        );
        assert!(styled.contains("padding:20px"), "author padding dropped: {styled}");
        assert!(styled.contains("border-radius:12px"), "author radius dropped: {styled}");
        assert!(
            styled.contains(r#"class="cx-html-block""#),
            "preservation class dropped: {styled}"
        );
    }

    #[test]
    fn render_styled_rich_sanitizes_scripts() {
        let styled = font_pass(&render_styled(
            r#"<div style="padding:20px;">ok</div><script>steal()</script>"#,
            Some(true),
            "rich",
        ));
        assert!(styled.contains("ok"));
        assert!(!styled.contains("<script"), "script survived: {styled}");
    }

    #[test]
    fn render_styled_card_none_autodetects_inner() {
        // layout="card" with is_html=None is reachable in production: compose_draft
        // /edit_draft pass params.is_html straight through, so a caller using
        // layout="card" without is_html lands on the auto-detect arm. Tag-bearing
        // bodies must pass through as markup; bare text must be escaped — both
        // wrapped in the card chrome + preservation block.
        let html_inner = font_pass(&render_styled("<h2>Hi</h2>", None, "card"));
        assert!(html_inner.contains("<h2"), "HTML inner should stay markup: {html_inner}");
        assert!(html_inner.contains(r#"class="cx-html-block""#), "class missing: {html_inner}");
        assert!(html_inner.contains("max-width:600px"), "card chrome missing: {html_inner}");

        // "< and >" contains neither "</" nor "/>", so the heuristic treats it as
        // plain text and escapes it rather than interpreting tags.
        let text_inner = font_pass(&render_styled("plain line with < and >", None, "card"));
        assert!(text_inner.contains("&lt;"), "plain text should be escaped: {text_inner}");
        assert!(text_inner.contains(r#"class="cx-html-block""#), "class missing: {text_inner}");
    }

    #[test]
    fn render_styled_card_survives_second_sanitize() {
        // The load-bearing class and the card background must survive the SECOND
        // sanitize pass that persist_local_draft / the reopen path apply on top of
        // the font-styled body — not just the single pass inside render_styled.
        let first = font_pass(&render_styled("Body", Some(false), "card"));
        let second = sanitize_html_lossy(&first);
        assert!(
            second.contains(r#"class="cx-html-block""#),
            "class lost on re-sanitize: {second}"
        );
        assert!(
            second.contains("background-color:#ffffff"),
            "card background lost on re-sanitize: {second}"
        );
        assert!(
            second.contains("max-width:600px"),
            "card chrome lost on re-sanitize: {second}"
        );
    }

    // ─── bare text in a table cell (email::loose_text) ─────────────────────
    //
    // The unit tests live next to the pass; these pin that it is actually wired
    // into BOTH styled paths, and that the wrapper survives the rest of the
    // pipeline (sanitize + font pass) — which is where a wrapper that looked
    // right in isolation would quietly stop being right.

    /// The 2026-08-18 Northwind pre-read shape, source-indented as an agent writes it.
    const LOOSE_CELL_BODY: &str = "<table><tr><td style=\"padding:14px 18px;font-size:15px\">\n\
        <div style=\"font-weight:bold\">TL;DR</div>\n\
        The growth is now a trend, not a hope.\n\
      </td></tr></table>";

    #[test]
    fn render_styled_rich_wraps_bare_cell_text() {
        let styled = font_pass(&render_styled(LOOSE_CELL_BODY, Some(true), "rich"));
        assert!(
            styled.contains("margin:0") && styled.contains("font-size:inherit"),
            "wrapper missing from rich output: {styled}"
        );
        // The wrapped run starts at its first real character — the source's
        // newline+indent must not survive as a first-line indent.
        assert!(
            styled.contains(">The growth is now a trend, not a hope.</p>"),
            "run not wrapped tightly: {styled}"
        );
        // Everything else is untouched.
        assert!(styled.contains("TL;DR"), "eyebrow lost: {styled}");
        assert!(styled.contains("padding:14px 18px"), "cell style lost: {styled}");
        assert!(
            styled.contains(r#"class="cx-html-block""#),
            "preservation class dropped: {styled}"
        );
    }

    #[test]
    fn render_styled_card_wraps_bare_cell_text_in_inner_content() {
        // The card path builds inner content first, so a normalizer wired only
        // into the rich branch would miss it entirely.
        let styled = font_pass(&render_styled(LOOSE_CELL_BODY, Some(true), "card"));
        assert!(
            styled.contains(">The growth is now a trend, not a hope.</p>"),
            "card inner content not normalized: {styled}"
        );
        assert!(styled.contains("max-width:600px"), "card chrome lost: {styled}");
    }

    #[test]
    fn render_styled_keeps_the_inherit_declarations_through_the_pipeline() {
        // Without these the font pass restyles the text the wrapper just
        // adopted: a 15px cell would come out 13px #222222. Sanitize must keep
        // them too — ammonia is configured with no CSS property filter, and a
        // future filter that dropped `inherit` would silently reintroduce the
        // restyle.
        let styled = font_pass(&render_styled(LOOSE_CELL_BODY, Some(true), "rich"));
        for decl in [
            "font-size:inherit",
            "color:inherit",
            "font-family:inherit",
            "line-height:inherit",
        ] {
            assert!(styled.contains(decl), "{decl} lost in pipeline: {styled}");
        }
        let base = styled.find("font-size:13px").expect("font pass ran");
        let inherit = styled.find("font-size:inherit").expect("inherit kept");
        assert!(base < inherit, "inherit must come last to win: {styled}");
    }

    #[test]
    fn render_styled_leaves_correctly_authored_cells_alone() {
        // The counterpart guarantee: a design that already wraps its copy is not
        // rewritten, so this pass is invisible on every email that was right.
        let body = r#"<table><tr><td style="padding:14px"><p style="margin:0;font-size:14px">Copy.</p></td></tr></table>"#;
        let with = render_styled(body, Some(true), "rich");
        assert!(!with.contains("font-size:inherit"), "wrapper added: {with}");
        assert!(with.contains("font-size:14px"), "author style lost: {with}");
    }

    #[test]
    fn layout_advisory_flags_a_cell_that_mixes_a_block_with_bare_text() {
        // Fires only for the mixed shape — the one where the author's
        // per-paragraph styling silently applied to nothing.
        let note = layout_advisory(Some("rich"), LOOSE_CELL_BODY);
        assert!(note.contains("table cell"), "mixed shape not flagged: {note:?}");
        assert_eq!(
            layout_advisory(Some("card"), LOOSE_CELL_BODY).is_empty(),
            false,
            "card path should flag it too"
        );
        // A cell that is nothing but text is normalized silently: no competing
        // styled element means nothing was lost, and warning on it would fire on
        // most designed email and train callers to ignore the note.
        assert_eq!(
            layout_advisory(
                Some("rich"),
                r#"<table style="width:100%"><tr><td style="padding:8px">Just copy.</td></tr></table>"#
            ),
            ""
        );
    }

    // ─── the `layout` contract guard ───────────────────────────────────────
    // `layout` is a free-form Option<String> on the wire, and both mismatches it
    // permits are silent at draft time: the flattening happens later, when the
    // user opens the draft in compose. These pin the two guards that make the
    // mismatch visible at the call.

    /// The exact shape of the three-month outage: a `Re:` draft with no parent.
    #[test]
    fn reply_shaped_subject_without_a_target_is_rejected() {
        for subject in [
            "Re: Northwind Company / CX Ventures/ Catalyst Partners Meeting Follow Up",
            "re: grant tool issue",
            "RE: Fwd: CRM Request",
            "  Re: leading whitespace",
        ] {
            let err = validate_reply_threading(subject, &None)
                .expect_err("a reply with no parent must be refused");
            let msg = format!("{err}");
            assert!(
                msg.contains("reply_to_folder") && msg.contains("reply_to_uid"),
                "the error must name the remedy: {msg}"
            );
            assert!(
                msg.contains("new thread"),
                "the error must name the consequence: {msg}"
            );
        }
    }

    #[test]
    fn a_real_reply_target_satisfies_the_guard() {
        let target = Some(ReplyTarget::Coordinates {
            folder: "INBOX".into(),
            uid: 410,
        });
        assert!(validate_reply_threading("Re: Scope", &target).is_ok());
        assert!(validate_reply_threading("Re: Scope", &Some(ReplyTarget::MessageId("<m@x>".into()))).is_ok());
    }

    /// Forwards to a third party are legitimately new threads, and a subject
    /// that merely contains "re" is not a reply. Over-rejecting here would
    /// block ordinary drafting.
    #[test]
    fn non_reply_subjects_pass_untouched() {
        for subject in [
            "Fwd: CRM Request",
            "Retainer proposal for Q3",
            "Rebuilding the Studio pipeline",
            "Recap: yesterday's call",
            "",
            "re",
        ] {
            assert!(
                validate_reply_threading(subject, &None).is_ok(),
                "{subject:?} is not a reply and must not be refused"
            );
        }
    }

    #[test]
    fn validate_layout_accepts_the_three_real_choices() {
        assert!(validate_layout(None).is_ok(), "omitted layout must stay valid");
        assert!(validate_layout(Some("card")).is_ok());
        assert!(validate_layout(Some("rich")).is_ok());
    }

    #[test]
    fn validate_layout_rejects_unknown_values_and_names_the_valid_ones() {
        // "Card" is the realistic typo — it previously fell through the match
        // arms onto the plain path and flattened the caller's design silently.
        for bad in ["Card", "RICH", "minimal", "plain", ""] {
            let err = validate_layout(Some(bad))
                .expect_err("unknown layout {bad:?} must be rejected");
            let msg = format!("{err}");
            assert!(
                msg.contains("rich") && msg.contains("card"),
                "error for {bad:?} must name the valid values: {msg}"
            );
            assert!(
                msg.contains("Omit"),
                "error for {bad:?} must explain the omit semantics: {msg}"
            );
        }
    }

    // ─── the `conference` contract guard ───────────────────────────────────

    /// Truth table for the conferencing choice. The first assertion is the
    /// back-compat pin: absent `conference` plus absent `add_meet` must still be
    /// Google Meet, exactly as before Zoom existed.
    #[test]
    fn validate_conference_truth_table() {
        use Conferencing::{Meet, None as NoConference, Zoom};

        let cases = [
            // (conference, add_meet, expected)
            (Option::<&str>::None, Option::<bool>::None, Meet), // ← the pin
            (None, Some(true), Meet),
            (None, Some(false), NoConference),
            (Some("meet"), None, Meet),
            (Some("meet"), Some(true), Meet),
            (Some("zoom"), None, Zoom),
            (Some("Zoom "), None, Zoom),
            (Some("none"), None, NoConference),
            (Some("none"), Some(false), NoConference),
            (Some(""), None, Meet),
        ];
        for (conference, add_meet, expected) in cases {
            let actual = validate_conference(conference, add_meet, false, false)
                .unwrap_or_else(|e| panic!("({conference:?}, {add_meet:?}) rejected: {e}"));
            assert_eq!(actual, expected, "for ({conference:?}, {add_meet:?})");
        }

        // A contradiction is rejected in BOTH directions: each one would
        // otherwise silently produce conferencing the caller did not ask for.
        for (conference, add_meet) in [
            (Some("zoom"), Some(true)),
            (Some("none"), Some(true)),
            (Some("meet"), Some(false)),
        ] {
            let err = validate_conference(conference, add_meet, false, false)
                .expect_err("({conference:?}, {add_meet:?}) must be rejected");
            let msg = format!("{err}");
            assert!(
                msg.contains("contradict"),
                "must say the two disagree: {msg}"
            );
            assert!(
                msg.contains("add_meet"),
                "must name the deprecated parameter to drop: {msg}"
            );
        }

        // An unknown value names all three valid ones and the default it would
        // otherwise have fallen through to.
        let err = format!(
            "{}",
            validate_conference(Some("webex"), None, false, false).unwrap_err()
        );
        for expected in ["meet", "zoom", "none", "Google Meet"] {
            assert!(err.contains(expected), "unknown value error must name {expected}: {err}");
        }
    }

    #[test]
    fn validate_conference_refuses_zoom_on_shapes_a_meeting_cannot_model() {
        // All-day: there is no instant to schedule, so the event would be created
        // with no join link while the call reports one.
        let all_day = format!(
            "{}",
            validate_conference(Some("zoom"), None, true, false).unwrap_err()
        );
        assert!(all_day.contains("all-day"), "{all_day}");
        assert!(all_day.contains("conference=\"none\""), "names the way out: {all_day}");

        let recurring = format!(
            "{}",
            validate_conference(Some("zoom"), None, false, true).unwrap_err()
        );
        assert!(recurring.contains("recurring"), "{recurring}");

        // Meet is unaffected by both — Google handles those shapes itself.
        assert!(validate_conference(Some("meet"), None, true, false).is_ok());
        assert!(validate_conference(Some("meet"), None, false, true).is_ok());
        assert!(validate_conference(None, None, true, true).is_ok());
    }

    /// `EventPatch.description` replaces the field wholesale, so a "reschedule
    /// and rewrite the description" call would delete the join block and report
    /// success. This is the predicate the MCP refuses on.
    #[test]
    fn a_description_rewrite_that_drops_the_zoom_block_is_refused() {
        let block = zoom_sync::zoom_block("https://us02web.zoom.us/j/869", "869");
        let stored = zoom_sync::upsert_zoom_block(Some("Agenda: roadmap"), &block);

        assert!(zoom_sync::zoom_block_would_be_dropped(
            Some(&stored),
            "Agenda: roadmap and hiring"
        ));
        assert!(!zoom_sync::zoom_block_would_be_dropped(
            Some(&stored),
            &zoom_sync::upsert_zoom_block(Some("Agenda: roadmap and hiring"), &block)
        ));
        // A Meet-only event has no block, so nothing is refused.
        assert!(!zoom_sync::zoom_block_would_be_dropped(
            Some("Agenda: roadmap"),
            "Anything"
        ));
    }

    #[test]
    fn layout_advisory_flags_designed_html_with_layout_omitted() {
        // Inline styles and tables are exactly what StarterKit drops.
        for body in [
            r#"<div style="padding:24px">Hi</div>"#,
            r#"<table><tr><td>Row</td></tr></table>"#,
            r#"<TABLE><TR><TD>caps</TD></TR></TABLE>"#,
            r#"<p STYLE="color:red">shouty attribute</p>"#,
        ] {
            let note = layout_advisory(None, body);
            assert!(
                note.contains("layout") && note.contains("rich"),
                "designed body {body:?} should be flagged, got {note:?}"
            );
        }
    }

    #[test]
    fn layout_advisory_flags_rich_on_trivial_markup() {
        // The mirror mistake: sealing an ordinary note into a preserved block,
        // which locks its structure in compose for no benefit.
        for body in [
            "Just a plain sentence.",
            "<p>Hello there.</p>",
            "<ul><li>one</li><li>two</li></ul>",
            "<strong>bold</strong> and a <a href=\"https://x.test\">link</a>",
        ] {
            let note = layout_advisory(Some("rich"), body);
            assert!(
                note.contains("locks"),
                "trivial body {body:?} with rich should be flagged, got {note:?}"
            );
        }
    }

    #[test]
    fn layout_advisory_is_silent_on_simple_markup_without_layout() {
        // The whole point of the correction: a real <ul> survives an omitted
        // layout untouched, so warning here would be noise that trains callers
        // to ignore the note that matters.
        for body in [
            "Plain prose, nothing more.",
            "<ul><li>one</li><li>two</li></ul>",
            "<strong>bold</strong> and <em>italic</em>",
            "<h2>Heading</h2><p>Body copy.</p>",
            "<div>a bare div with no styling</div>",
        ] {
            assert_eq!(
                layout_advisory(None, body),
                "",
                "simple body {body:?} must not be flagged"
            );
        }
    }

    #[test]
    fn layout_advisory_is_silent_on_rich_with_genuinely_designed_html() {
        for body in [
            r#"<div style="background:#fff;padding:32px">Designed</div>"#,
            r#"<table role="presentation"><tr><td>Cell</td></tr></table>"#,
            r#"<div><span>nested div, no styles</span></div>"#,
        ] {
            assert_eq!(
                layout_advisory(Some("rich"), body),
                "",
                "designed body {body:?} with rich is the correct pairing"
            );
        }
    }

    #[test]
    fn layout_advisory_never_fires_for_card() {
        // With "card" the chrome IS the design, so the choice is always
        // deliberate — in both directions.
        assert_eq!(layout_advisory(Some("card"), "Plain sentence."), "");
        assert_eq!(
            layout_advisory(Some("card"), r#"<table style="width:100%"><tr><td>x</td></tr></table>"#),
            ""
        );
    }

    #[test]
    fn body_to_html_three_paragraphs() {
        // The original failing case: greeting / body / sign-off.
        assert_eq!(
            body_to_html("Hi Dana,\n\nFollowing up on the Northwind Q3 push.\n\nThanks"),
            "<p>Hi Dana,</p><p>Following up on the Northwind Q3 push.</p><p>Thanks</p>"
        );
    }

    #[test]
    fn body_to_html_collapses_extra_blank_lines() {
        // Three+ blank lines collapse to a single paragraph boundary (no empty <p>s).
        assert_eq!(body_to_html("A\n\n\n\nB"), "<p>A</p><p>B</p>");
    }

    #[test]
    fn sanitize_voice_excerpt_strips_quotes_signature_and_caps_length() {
        let input = format!(
            "{}\n> quoted reply\n-- \nChristopher Robinson\nhttps://example.com",
            "A concise original note with enough detail to preserve the user's writing style. "
                .repeat(10)
        );
        let out = sanitize_voice_excerpt(&input, 120);
        assert!(out.chars().count() <= 120);
        assert!(!out.contains("quoted reply"));
        assert!(!out.contains("Christopher Robinson"));
        assert!(out.starts_with("A concise original note"));
    }

    #[test]
    fn archetype_voice_samples_return_assigned_message_bodies() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE messages (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                date TEXT NOT NULL
            );
            CREATE TABLE message_bodies (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                plain_text TEXT
            );
            CREATE TABLE message_archetype (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                archetype_id TEXT NOT NULL
            );
            INSERT INTO messages VALUES ('acct', 'Sent', 1, '2026-01-01T10:00:00Z');
            INSERT INTO message_bodies VALUES ('acct', 'Sent', 1, 'Fresh archetype sample text with enough original content to pass collection thresholds and provide a representative excerpt for MCP voice profile callers.
> quoted old thread');
            INSERT INTO message_archetype VALUES ('acct', 'Sent', 1, 'arch-1');
            INSERT INTO messages VALUES ('acct', 'Sent', 2, '2026-01-02T10:00:00Z');
            INSERT INTO message_bodies VALUES ('acct', 'Sent', 2, 'Different archetype text');
            INSERT INTO message_archetype VALUES ('acct', 'Sent', 2, 'arch-2');
            ",
        )
        .unwrap();

        let samples =
            crate::email::voice::collect_samples_for_archetype(&conn, "acct", "arch-1", 3).unwrap();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].contains("Fresh archetype sample text"));
        assert!(!samples[0].contains("quoted old thread"));
    }

    // ─── Quoted history (MCP reply path) ──────────────────────

    #[test]
    fn quoted_history_matches_frontend_blockquote_contract() {
        let (html, plain) = format_quoted_history(
            Some("Sam Ellis"),
            Some("sam@harborline.example"),
            "2026-01-01T10:00:00Z",
            Some("<p>original message</p>"),
            Some("original message"),
        );
        assert!(html.starts_with(
            r#"<blockquote data-cx-quote="1" class="cx-quote"><br/><br/><div style="border-left:2px solid rgb(74, 68, 57);padding-left:12px;margin-left:4px;color:rgb(160, 144, 120);">"#
        ));
        assert!(html.contains("<p><strong>Sam Ellis</strong> wrote on "));
        assert!(html.contains("<p>original message</p>"));
        assert!(html.ends_with("</div></blockquote>"));
        // Parseable RFC 3339 date renders as local time, not the raw string.
        assert!(!html.contains("2026-01-01T10:00:00Z"));
        assert!(plain.starts_with("\n\nOn "));
        assert!(plain.contains("Sam Ellis wrote:\n> original message"));
    }

    #[test]
    fn quoted_history_escapes_author_and_keeps_unparseable_date_raw() {
        let (html, _) = format_quoted_history(
            Some("Evil <script>&Co"),
            None,
            "not-a-date",
            Some("<p>x</p>"),
            None,
        );
        assert!(html.contains("<strong>Evil &lt;script&gt;&amp;Co</strong>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("wrote on not-a-date:"));
    }

    #[test]
    fn quoted_history_pre_fallback_when_no_html() {
        let (html, plain) = format_quoted_history(
            None,
            Some("a@b.c"),
            "2026-01-01T10:00:00Z",
            None,
            Some("line one <tag>\r\nline two"),
        );
        assert!(html.contains("<strong>a@b.c</strong>"));
        assert!(html.contains("<pre>line one &lt;tag&gt;\r\nline two</pre>"));
        assert!(plain.contains("a@b.c wrote:\n> line one <tag>\n> line two"));
    }

    #[test]
    fn quoted_history_empty_plain_text_yields_empty_plain_part() {
        let (html, plain) = format_quoted_history(
            Some("Sam"),
            None,
            "2026-01-01T10:00:00Z",
            Some("<p>html only</p>"),
            None,
        );
        assert!(html.contains("<p>html only</p>"));
        assert!(plain.is_empty());
    }

    #[test]
    fn body_already_quoted_detects_both_markers() {
        assert!(body_already_quoted(
            r#"<blockquote class="cx-quote">old</blockquote>"#
        ));
        assert!(body_already_quoted(
            r#"<blockquote data-cx-quote="1">old</blockquote>"#
        ));
        assert!(!body_already_quoted("Hi Sam, following up on the CRM."));
    }

    // ─── Reply targeting & quote planning (gotcha #30) ────────

    /// Full production schema + one account, so `plan_quote` sees the real
    /// `messages` / `message_bodies` / `folders` shapes.
    fn quote_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id, name, folder_type) VALUES ('acct', 'INBOX', 'inbox')",
            [],
        )
        .unwrap();
        conn
    }

    /// The exact live-DB shape of the message that produced the false
    /// "original body not cached": INBOX uid 410, bracketed Message-ID,
    /// healthy cached body.
    fn seed_live_shape(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, message_id, reference_ids, subject,
                 from_name, from_email, date)
             VALUES ('acct','INBOX',410,'<A7EA3536@harborline.example>','<root@x> <mid2@x>',
                     'Re: CRM Transfer Update','Sam Ellis','sam@harborline.example',
                     '2026-07-25T15:28:00Z')",
            [],
        )
        .unwrap();
        crate::db::messages::insert_body(
            conn,
            "acct",
            "INBOX",
            410,
            Some("original body"),
            Some("<p>original body</p>"),
            Some("<p>original body</p>"),
            true,
        )
        .unwrap();
    }

    #[test]
    fn resolve_reply_target_prefers_coordinates_over_message_id() {
        let t = resolve_reply_target(Some("INBOX"), Some(410), Some("<mid@x>"))
            .unwrap()
            .unwrap();
        assert_eq!(
            t,
            ReplyTarget::Coordinates {
                folder: "INBOX".into(),
                uid: 410
            }
        );
    }

    #[test]
    fn resolve_reply_target_rejects_half_specified_pairs() {
        let folder_only = resolve_reply_target(Some("INBOX"), None, None).unwrap_err();
        assert!(
            folder_only.message.contains("reply_to_uid"),
            "must name the missing half: {}",
            folder_only.message
        );
        let uid_only = resolve_reply_target(None, Some(410), None).unwrap_err();
        assert!(
            uid_only.message.contains("reply_to_folder"),
            "must name the missing half: {}",
            uid_only.message
        );
        // A message_id present alongside a half-pair does NOT rescue it —
        // silently ignoring the coordinates would quote the wrong thing.
        assert!(resolve_reply_target(Some("INBOX"), None, Some("<mid@x>")).is_err());
    }

    #[test]
    fn resolve_reply_target_normalizes_message_id_and_detects_non_replies() {
        assert!(resolve_reply_target(None, None, None).unwrap().is_none());
        // Blank strings are not a reply target either.
        assert!(resolve_reply_target(Some("  "), None, Some("   ")).unwrap().is_none());

        for input in ["<a@x>", "a@x", "&lt;a@x&gt;"] {
            assert_eq!(
                resolve_reply_target(None, None, Some(input)).unwrap().unwrap(),
                ReplyTarget::MessageId("<a@x>".into()),
                "input {input:?} must normalize to the bracketed form"
            );
        }
        assert!(resolve_reply_target(None, None, Some("not an id")).is_err());
    }

    #[test]
    fn resolve_threading_from_coordinates_uses_row_headers() {
        let conn = quote_conn();
        seed_live_shape(&conn);
        let (irt, refs) = resolve_threading(
            &conn,
            "acct",
            &ReplyTarget::Coordinates {
                folder: "INBOX".into(),
                uid: 410,
            },
        );
        assert_eq!(irt.as_deref(), Some("<A7EA3536@harborline.example>"));
        assert_eq!(
            refs.as_deref(),
            Some("<root@x> <mid2@x> <A7EA3536@harborline.example>")
        );
    }

    #[test]
    fn resolve_threading_emits_all_bracketed_references() {
        let conn = quote_conn();
        // The real bare Zoom chain from the live DB (INBOX uid 86605).
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, message_id, reference_ids, date)
             VALUES ('acct','INBOX',86605,'j93g140k9706ctu9x914mw8630@zoom.calendar.event-cxrobx@gmail.com',
                     'j93g140k9706ctu9x914mw8630@zoom.calendar.event-cxrobx@gmail.com',
                     '2026-05-29T10:00:00Z')",
            [],
        )
        .unwrap();

        let (irt, refs) = resolve_threading(
            &conn,
            "acct",
            &ReplyTarget::MessageId(
                "<j93g140k9706ctu9x914mw8630@zoom.calendar.event-cxrobx@gmail.com>".into(),
            ),
        );
        let refs = refs.expect("references");
        for token in refs.split(' ') {
            assert!(
                token.starts_with('<') && token.ends_with('>'),
                "every reference token must be bracketed, got {token:?} in {refs:?}"
            );
        }
        // Parent mid is last in the chain, per RFC 5322.
        assert!(refs.ends_with(irt.as_deref().unwrap()));
    }

    #[test]
    fn resolve_threading_message_id_miss_still_emits_valid_headers() {
        let conn = quote_conn();
        let (irt, refs) = resolve_threading(
            &conn,
            "acct",
            &ReplyTarget::MessageId("<ghost@x>".into()),
        );
        assert_eq!(irt.as_deref(), Some("<ghost@x>"));
        assert_eq!(refs.as_deref(), Some("<ghost@x>"));
    }

    /// ⭐ The precise assertion that was FALSE on 2026-07-25: the body was
    /// cached, the lookup just missed on spelling. Calling with the BARE ID
    /// against a bracketed row must plan a ready quote, no IMAP needed.
    #[test]
    fn plan_quote_ready_when_body_cached() {
        let conn = quote_conn();
        seed_live_shape(&conn);

        for input in [
            "<A7EA3536@harborline.example>",
            "A7EA3536@harborline.example",
            "&lt;A7EA3536@harborline.example&gt;",
        ] {
            let target = resolve_reply_target(None, None, Some(input)).unwrap().unwrap();
            match plan_quote(&conn, "acct", &target) {
                QuotePlan::Ready { html, .. } => {
                    assert!(html.contains("<p>original body</p>"));
                    assert!(html.contains("<strong>Sam Ellis</strong>"));
                }
                other => panic!("input {input:?} must plan Ready, got {other:?}"),
            }
        }
    }

    #[test]
    fn plan_quote_ready_from_coordinates_without_message_id() {
        let conn = quote_conn();
        seed_live_shape(&conn);
        let target = ReplyTarget::Coordinates {
            folder: "INBOX".into(),
            uid: 410,
        };
        match plan_quote(&conn, "acct", &target) {
            QuotePlan::Ready { html, plain } => {
                assert!(html.contains("<p>original body</p>"));
                assert!(plain.contains("> original body"));
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn plan_quote_falls_back_to_fetch_when_body_missing() {
        let conn = quote_conn();
        // Header row only, no message_bodies.
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, message_id, from_name, from_email, date)
             VALUES ('acct','INBOX',411,'<nobody@x>','Sam','sam@harborline.example','2026-07-25T15:28:00Z')",
            [],
        )
        .unwrap();
        match plan_quote(&conn, "acct", &ReplyTarget::MessageId("<nobody@x>".into())) {
            QuotePlan::Fetch(f) => {
                assert_eq!((f.folder.as_str(), f.uid), ("INBOX", 411));
                assert_eq!(f.from_email.as_deref(), Some("sam@harborline.example"));
            }
            other => panic!("expected Fetch, got {other:?}"),
        }
    }

    #[test]
    fn plan_quote_skips_when_message_id_unknown() {
        let conn = quote_conn();
        match plan_quote(&conn, "acct", &ReplyTarget::MessageId("<ghost@x>".into())) {
            QuotePlan::Skip(QuoteSkip::NotInDb { message_id }) => {
                assert_eq!(message_id, "<ghost@x>")
            }
            other => panic!("expected NotInDb skip, got {other:?}"),
        }
    }

    /// Coordinates naming a folder this account doesn't have are an agent
    /// error, not something to paper over with a blind IMAP fetch — a wrong
    /// folder with a valid-looking UID quotes a *different* message.
    #[test]
    fn plan_quote_skips_unknown_folder_with_no_local_row() {
        let conn = quote_conn();
        match plan_quote(
            &conn,
            "acct",
            &ReplyTarget::Coordinates {
                folder: "Inbox".into(), // wrong case — not a real folder here
                uid: 410,
            },
        ) {
            QuotePlan::Skip(QuoteSkip::NoLocalRow { folder, uid }) => {
                assert_eq!((folder.as_str(), uid), ("Inbox", 410))
            }
            other => panic!("expected NoLocalRow skip, got {other:?}"),
        }
        // A known folder with no local row still gets the IMAP attempt.
        match plan_quote(
            &conn,
            "acct",
            &ReplyTarget::Coordinates {
                folder: "INBOX".into(),
                uid: 9999,
            },
        ) {
            QuotePlan::Fetch(f) => assert_eq!(f.uid, 9999),
            other => panic!("expected Fetch for a known folder, got {other:?}"),
        }
    }

    #[test]
    fn quote_skip_reasons_are_distinct_and_actionable() {
        let skips = [
            QuoteSkip::NotInDb {
                message_id: "<A7EA3536@harborline.example>".into(),
            },
            QuoteSkip::NoLocalRow {
                folder: "Inbox".into(),
                uid: 410,
            },
            QuoteSkip::ImapConnect {
                error: "connection timed out".into(),
            },
            QuoteSkip::ImapSelect {
                folder: "INBOX".into(),
                error: "NO no such mailbox".into(),
            },
            QuoteSkip::ImapFetch {
                folder: "INBOX".into(),
                uid: 410,
                error: "BAD invalid uid".into(),
            },
            QuoteSkip::NoRenderableBody {
                folder: "INBOX".into(),
                uid: 410,
            },
        ];
        // The concrete detail each variant must surface — the thing the old
        // single static string threw away.
        let details = [
            "A7EA3536@harborline.example",
            "\"Inbox\"",
            "connection timed out",
            "NO no such mailbox",
            "BAD invalid uid",
            "UID 410",
        ];
        let messages: Vec<String> = skips.iter().map(|s| s.message()).collect();

        // Six distinct strings.
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), 6, "skip reasons collapsed: {messages:#?}");

        for (m, detail) in messages.iter().zip(details) {
            // Never the old lie.
            assert!(
                !m.contains("original body not cached"),
                "resurrected the false message: {m}"
            );
            assert!(m.contains(detail), "missing concrete detail {detail:?} in: {m}");
            // Every reason offers a next step…
            assert!(
                m.contains("reply_to_folder")
                    || m.contains("quote_original=false")
                    || m.contains("Retry"),
                "no actionable remedy in: {m}"
            );
            // …and every reason bans the paste workaround.
            assert!(m.contains("signature below the quote"), "no paste ban in: {m}");
        }

        // Bad input vs environment is reflected in the error code.
        assert_eq!(
            skips[0].to_error().code,
            McpError::invalid_params("x", None).code
        );
        assert_eq!(
            skips[2].to_error().code,
            McpError::internal_error("x", None).code
        );
        // The concrete detail survives into the error the agent sees.
        assert!(skips[0].to_error().message.contains("A7EA3536@harborline.example"));
        assert!(skips[4].to_error().message.contains("BAD invalid uid"));
    }

    /// Fail loudly: a skip must become an `Err` from `finish_quote`, which
    /// both callers `?` on — strictly before the IMAP APPEND in either
    /// handler, so no draft is created, duplicated, or destroyed.
    #[test]
    fn quote_failure_errors_before_append() {
        let conn = quote_conn();
        let outcome = QuoteOutcome {
            skip: Some(QuoteSkip::NotInDb {
                message_id: "<ghost@x>".into(),
            }),
            ..Default::default()
        };
        let err = finish_quote(&conn, "acct", outcome).unwrap_err();
        assert!(err.message.contains("QUOTED HISTORY UNAVAILABLE"));
        assert!(err.message.contains("<ghost@x>"));

        // Nothing was written on the way out.
        let drafts: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(drafts, 0, "a failed quote must not create rows");
    }

    #[test]
    fn quote_note_confirms_success_explicitly() {
        let conn = quote_conn();
        let outcome = QuoteOutcome {
            quote: Some(("<blockquote/>".into(), "> x".into())),
            ..Default::default()
        };
        let (quote, note) = finish_quote(&conn, "acct", outcome).unwrap();
        assert!(quote.is_some());
        assert_eq!(note, " + quoted original");
    }

    /// The deliberate opt-outs are not failures: they produce no quote, no
    /// error, and an empty note.
    #[test]
    fn body_already_quoted_and_quote_original_false_do_not_error() {
        // `build_reply_context` gates on exactly these two predicates before
        // ever planning a quote.
        let pasted = r#"Hi Sam,<blockquote class="cx-quote">old</blockquote>"#;
        assert!(body_already_quoted(pasted));
        assert!(!body_already_quoted("Hi Sam, following up."));

        let want_quote = |opt: Option<bool>, body: &str| opt.unwrap_or(true) && !body_already_quoted(body);
        assert!(!want_quote(Some(false), "plain body"));
        assert!(!want_quote(None, pasted));
        assert!(want_quote(None, "plain body"));

        // With no plan there is no outcome to fail on.
        let conn = quote_conn();
        let (quote, note) = finish_quote(&conn, "acct", QuoteOutcome::default()).unwrap();
        assert!(quote.is_none());
        assert!(note.is_empty());
    }

    #[test]
    fn finish_quote_persists_fetched_body_without_clobbering_summary() {
        let conn = quote_conn();
        seed_live_shape(&conn);
        conn.execute(
            "UPDATE message_bodies SET summary = 'AI summary', summary_model = 'claude-opus-5'
              WHERE account_id='acct' AND folder_name='INBOX' AND uid=410",
            [],
        )
        .unwrap();

        let outcome = QuoteOutcome {
            quote: Some(("<blockquote/>".into(), "> x".into())),
            skip: None,
            writeback: Some((
                "INBOX".into(),
                410,
                FetchedBody {
                    plain_text: Some("refetched body".into()),
                    html_body: Some("<p>refetched body</p>".into()),
                    sanitized_html: Some("<p>refetched body</p>".into()),
                },
            )),
            fetched_headers: None,
        };
        finish_quote(&conn, "acct", outcome).unwrap();

        let (plain, summary, model): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT plain_text, summary, summary_model FROM message_bodies
                  WHERE account_id='acct' AND folder_name='INBOX' AND uid=410",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(plain, "refetched body", "write-back must land");
        assert_eq!(summary.as_deref(), Some("AI summary"), "summary clobbered");
        assert_eq!(model.as_deref(), Some("claude-opus-5"));
    }

    /// A write-back for coordinates with no parent `messages` row violates
    /// the composite FK. It must log and be swallowed, never fail the tool.
    #[test]
    fn finish_quote_survives_a_failed_writeback() {
        let conn = quote_conn();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let outcome = QuoteOutcome {
            quote: Some(("<blockquote/>".into(), "> x".into())),
            skip: None,
            writeback: Some((
                "INBOX".into(),
                4242, // no messages row → FK violation
                FetchedBody {
                    plain_text: Some("orphan".into()),
                    html_body: None,
                    sanitized_html: Some("<p>orphan</p>".into()),
                },
            )),
            fetched_headers: None,
        };
        let (quote, note) = finish_quote(&conn, "acct", outcome).unwrap();
        assert!(quote.is_some(), "a failed cache write must not lose the quote");
        assert_eq!(note, " + quoted original");
    }

    #[test]
    fn get_reply_source_prefers_folder_with_cached_body() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE messages (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                message_id TEXT,
                from_name TEXT,
                from_email TEXT,
                date TEXT NOT NULL
            );
            CREATE TABLE message_bodies (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                plain_text TEXT,
                sanitized_html TEXT
            );
            INSERT INTO messages VALUES ('acct', 'INBOX', 10, '<mid@x>', 'Sam', 'sam@x.com', '2026-01-01T10:00:00Z');
            INSERT INTO messages VALUES ('acct', '[Gmail]/All Mail', 77, '<mid@x>', 'Sam', 'sam@x.com', '2026-01-01T10:00:00Z');
            INSERT INTO message_bodies VALUES ('acct', '[Gmail]/All Mail', 77, 'cached body', '<p>cached body</p>');
            ",
        )
        .unwrap();

        let src = crate::db::messages::get_reply_source(&conn, "acct", "<mid@x>")
            .unwrap()
            .expect("source row");
        assert_eq!(src.folder_name, "[Gmail]/All Mail");
        assert_eq!(src.uid, 77);
        assert_eq!(src.from_name.as_deref(), Some("Sam"));
        assert_eq!(src.from_email.as_deref(), Some("sam@x.com"));
        assert_eq!(src.date, "2026-01-01T10:00:00Z");

        assert!(crate::db::messages::get_reply_source(&conn, "acct", "<other@x>")
            .unwrap()
            .is_none());
    }

    // ─── Mail rules ───────────────────────────────────────────────────

    fn cond(field: &str, op: &str, value: &str) -> MailRuleConditionInput {
        MailRuleConditionInput {
            field: field.to_string(),
            operator: op.to_string(),
            value: value.to_string(),
        }
    }

    fn act(action_type: &str, value: Option<&str>) -> MailRuleActionInput {
        MailRuleActionInput {
            action_type: action_type.to_string(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn rule_actions_reject_move_and_delete_with_an_actionable_reason() {
        // The whole point of the guard: the classifier never executes these,
        // so accepting one would create a rule that looks saved and is dead.
        for dead in ["move", "delete", "MOVE", "Delete"] {
            let err = validate_rule_actions(&[act(dead, Some("Archive"))])
                .expect_err(&format!("{dead} must be rejected"));
            let msg = format!("{err:?}").to_lowercase();
            assert!(
                msg.contains("never executed"),
                "{dead}: error must say it is never executed, got: {msg}"
            );
            assert!(
                msg.contains("set_category"),
                "{dead}: error must name the alternative, got: {msg}"
            );
        }
    }

    #[test]
    fn rule_conditions_reject_body_because_it_can_never_match() {
        let err = validate_rule_conditions(&[cond("body", "contains", "invoice")])
            .expect_err("body must be rejected");
        let msg = format!("{err:?}").to_lowercase();
        assert!(
            msg.contains("header"),
            "error must explain that only headers are available, got: {msg}"
        );
    }

    #[test]
    fn rule_conditions_accept_to_now_that_the_engine_is_handed_one() {
        // `to` was evaluated against an empty string on both the sync and the
        // reclassify path; both now pass the real recipient list, so a `to`
        // condition is legitimate. If that regresses, reject `to` here too.
        let out = validate_rule_conditions(&[cond("to", "contains", "dmarc@")])
            .expect("to is a supported field");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].field, "to");
    }

    #[test]
    fn empty_conditions_are_rejected_as_a_whole_mailbox_hazard() {
        // `conditions.iter().all(..)` is vacuously TRUE on an empty list, so a
        // conditionless rule matches every message.
        let err = validate_rule_conditions(&[]).expect_err("empty must be rejected");
        assert!(format!("{err:?}").to_lowercase().contains("every message"));

        // Same hazard: "".contains("") == true.
        let err = validate_rule_conditions(&[cond("subject", "contains", "   ")])
            .expect_err("blank value must be rejected");
        assert!(format!("{err:?}").to_lowercase().contains("every message"));
    }

    #[test]
    fn empty_actions_are_rejected() {
        assert!(validate_rule_actions(&[]).is_err());
    }

    #[test]
    fn rule_validation_rejects_unknown_fields_operators_and_actions() {
        assert!(validate_rule_conditions(&[cond("cc", "contains", "x")]).is_err());
        assert!(validate_rule_conditions(&[cond("subject", "matches", "x")]).is_err());
        assert!(validate_rule_actions(&[act("archive", None)]).is_err());
    }

    #[test]
    fn set_category_requires_a_known_category() {
        assert!(validate_rule_actions(&[act("set_category", None)]).is_err());
        assert!(validate_rule_actions(&[act("set_category", Some("archive"))]).is_err());
        let ok = validate_rule_actions(&[act("set_category", Some("Updates"))]).unwrap();
        assert_eq!(ok[0].value.as_deref(), Some("updates"), "normalized to lowercase");
    }

    #[test]
    fn valid_rule_input_normalizes_case_and_drops_values_for_flag_actions() {
        let conds = validate_rule_conditions(&[cond("Subject", "STARTS_WITH", "Report Domain:")])
            .unwrap();
        assert_eq!(conds[0].field, "subject");
        assert_eq!(conds[0].operator, "starts_with");
        // Value case is preserved on the way in; `evaluate` lowercases both
        // sides at match time.
        assert_eq!(conds[0].value, "Report Domain:");

        let acts = validate_rule_actions(&[act("Mark_Read", Some("ignored"))]).unwrap();
        assert_eq!(acts[0].action_type, "mark_read");
        assert_eq!(acts[0].value, None, "flag actions carry no value");
    }

    #[test]
    fn warnings_flag_legacy_rules_that_cannot_fire_and_stay_quiet_otherwise() {
        let clean = crate::db::rules::MailRule {
            id: Some(1),
            account_id: None,
            name: "DMARC".into(),
            is_active: true,
            priority: 0,
            conditions: vec![crate::db::rules::RuleCondition {
                field: "subject".into(),
                operator: "contains".into(),
                value: "Report Domain:".into(),
            }],
            actions: vec![crate::db::rules::RuleAction {
                action_type: "set_category".into(),
                value: Some("updates".into()),
            }],
        };
        assert!(
            mail_rule_warnings(&clean).is_empty(),
            "a valid rule must not be annotated"
        );

        let dead = crate::db::rules::MailRule {
            conditions: vec![crate::db::rules::RuleCondition {
                field: "body".into(),
                operator: "contains".into(),
                value: "x".into(),
            }],
            actions: vec![crate::db::rules::RuleAction {
                action_type: "move".into(),
                value: Some("Archive".into()),
            }],
            ..clean.clone()
        };
        let w = mail_rule_warnings(&dead);
        assert_eq!(w.len(), 2, "one warning per dead part, got {w:?}");
        assert!(w.iter().any(|s| s.contains("body")));
        assert!(w.iter().any(|s| s.contains("never executed")));

        let catch_all = crate::db::rules::MailRule {
            conditions: vec![],
            ..clean.clone()
        };
        assert!(mail_rule_warnings(&catch_all)
            .iter()
            .any(|s| s.contains("EVERY message")));
    }

    /// End-to-end exercise of the five mail-rule tools against a real schema
    /// DB — the task's acceptance path: preview a filter, create the rule,
    /// see it listed, then delete it. Drives the actual tool functions, so a
    /// change to validation, wiring or output shape is caught here.
    #[tokio::test]
    async fn mail_rule_tools_round_trip_against_a_real_db() {
        let path = std::env::temp_dir().join(format!(
            "cxmail-mcp-rules-{}-{}.db",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            crate::db::schema::initialize(&conn).unwrap();
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
                 VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO folders (account_id, name, folder_type)
                 VALUES ('acct', 'INBOX', 'inbox')",
                [],
            )
            .unwrap();
            for (uid, subject, from) in [
                (1u32, "Report Domain: cxventures.io Submitter: google.com", "noreply-dmarc-support@google.com"),
                (2, "Report Domain: artistadvisory.io Submitter: yahoo.com", "dmarc@yahoo.com"),
                (3, "Lunch Thursday?", "sam@harborline.example"),
            ] {
                conn.execute(
                    "INSERT INTO messages
                        (account_id, folder_name, uid, subject, from_email, to_list, date)
                     VALUES ('acct', 'INBOX', ?1, ?2, ?3, 'chris@cxventures.io', '2026-08-01T10:00:00Z')",
                    rusqlite::params![uid, subject, from],
                )
                .unwrap();
            }
        }

        let mcp = CxMailMcp::new(path.to_string_lossy().to_string());
        let text = |r: CallToolResult| serde_json::to_string(&r).unwrap();

        // 1. Dry run before creating anything: 2 of 3 messages match.
        let preview = mcp
            .preview_mail_rule(Parameters(PreviewMailRuleParams {
                rule_id: None,
                conditions: Some(vec![cond("subject", "contains", "Report Domain:")]),
                account_id: None,
                limit: None,
            }))
            .await
            .expect("preview should succeed");
        let preview = text(preview);
        assert!(
            preview.contains(r#"\"matched\": 2"#),
            "expected 2 matches, got: {preview}"
        );
        assert!(
            preview.contains(r#"\"messages_scanned\": 3"#),
            "expected 3 scanned, got: {preview}"
        );

        // 2. Create the rule.
        let created = mcp
            .create_mail_rule(Parameters(CreateMailRuleParams {
                name: "DMARC reports out of inbox".to_string(),
                account_id: None,
                is_active: None,
                priority: None,
                conditions: vec![cond("subject", "contains", "Report Domain:")],
                actions: vec![act("set_category", Some("updates")), act("mark_read", None)],
            }))
            .await
            .expect("create should succeed");
        let created = text(created);
        assert!(created.contains("Created mail rule id=1"), "got: {created}");
        assert!(created.contains("global"), "scope should be reported: {created}");

        // 3. It shows up in the list, clean (no warnings).
        let listed = text(mcp.list_mail_rules().await.unwrap());
        assert!(listed.contains("DMARC reports out of inbox"), "got: {listed}");
        assert!(listed.contains(r#"\"warnings\": []"#), "a valid rule must carry no warnings: {listed}");
        assert!(listed.contains(r#"\"scope\": \"global\""#), "got: {listed}");

        // 4. Preview by id resolves the stored rule's conditions.
        let by_id = text(
            mcp.preview_mail_rule(Parameters(PreviewMailRuleParams {
                rule_id: Some(1),
                conditions: None,
                account_id: None,
                limit: Some(1),
            }))
            .await
            .unwrap(),
        );
        assert!(by_id.contains(r#"\"matched\": 2"#), "got: {by_id}");
        assert!(by_id.contains(r#"\"sample_shown\": 1"#), "limit must cap samples: {by_id}");

        // 5. The traps are refused at the boundary, not silently stored.
        assert!(mcp
            .create_mail_rule(Parameters(CreateMailRuleParams {
                name: "dead".to_string(),
                account_id: None,
                is_active: None,
                priority: None,
                conditions: vec![cond("subject", "contains", "x")],
                actions: vec![act("move", Some("Archive"))],
            }))
            .await
            .is_err());
        assert!(mcp
            .create_mail_rule(Parameters(CreateMailRuleParams {
                name: "dead".to_string(),
                account_id: None,
                is_active: None,
                priority: None,
                conditions: vec![cond("body", "contains", "x")],
                actions: vec![act("mark_read", None)],
            }))
            .await
            .is_err());
        // An unknown account is a hard error, not a rule that can never match.
        assert!(mcp
            .create_mail_rule(Parameters(CreateMailRuleParams {
                name: "ghost".to_string(),
                account_id: Some("no-such-account".to_string()),
                is_active: None,
                priority: None,
                conditions: vec![cond("subject", "contains", "x")],
                actions: vec![act("mark_read", None)],
            }))
            .await
            .is_err());

        // 6. Update replaces in full.
        mcp.update_mail_rule(Parameters(UpdateMailRuleParams {
            id: 1,
            name: "DMARC (paused)".to_string(),
            account_id: Some("acct".to_string()),
            is_active: Some(false),
            priority: Some(5),
            conditions: vec![cond("subject", "starts_with", "Report Domain:")],
            actions: vec![act("set_category", Some("junk"))],
        }))
        .await
        .expect("update should succeed");
        let listed = text(mcp.list_mail_rules().await.unwrap());
        assert!(listed.contains("DMARC (paused)"), "got: {listed}");
        assert!(listed.contains(r#"\"scope\": \"account:acct\""#), "got: {listed}");
        assert!(listed.contains(r#"\"is_active\": false"#), "got: {listed}");

        // 7. Delete needs the confirmation gate, then really removes it.
        assert!(mcp
            .delete_mail_rule(Parameters(DeleteMailRuleParams { id: 1, confirmed: None }))
            .await
            .is_err());
        assert!(mcp
            .delete_mail_rule(Parameters(DeleteMailRuleParams { id: 99, confirmed: Some(true) }))
            .await
            .is_err(), "deleting a nonexistent rule must fail loudly");
        mcp.delete_mail_rule(Parameters(DeleteMailRuleParams { id: 1, confirmed: Some(true) }))
            .await
            .expect("delete should succeed");
        let listed = text(mcp.list_mail_rules().await.unwrap());
        assert!(!listed.contains("DMARC"), "rule should be gone: {listed}");

        let _ = std::fs::remove_file(&path);
    }

    /// The T24 acceptance path: a rule that matches already-synced mail is
    /// applied to it, a dry run reports without writing, a manual category
    /// override is never touched, and a legacy rule that the T17 guards
    /// would have rejected cannot be applied at all.
    ///
    /// Mutation-tested: dropping the `dry_run` default to false, removing
    /// the `category_source = 'user'` skip, or removing the
    /// `rule_apply_blockers` gate each fail an assertion here.
    ///
    /// T26 additionally pins the IMAP-push DEGRADATION path. There is no
    /// reachable IMAP here (the fixture account is deliberately
    /// `…@….invalid`, see below), so every \Seen push fails — and the whole
    /// point of the failure mode is that the apply still SUCCEEDS, the local
    /// writes all land, and the result says the push failed instead of
    /// quietly claiming a lasting change.
    #[tokio::test]
    async fn apply_mail_rule_writes_only_when_told_to() {
        let path = std::env::temp_dir().join(format!(
            "cxmail-mcp-apply-{}-{}.db",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            crate::db::schema::initialize(&conn).unwrap();
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
                 VALUES ('acct', 'nobody@cxmail-test.invalid', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO folders (account_id, name, folder_type)
                 VALUES ('acct', 'INBOX', 'inbox')",
                [],
            )
            .unwrap();
            // uid 1,2 match and are fair game; uid 3 doesn't match; uid 4
            // matches but carries a manual override and must be left alone.
            for (uid, subject, source) in [
                (1u32, "Report Domain: cxventures.io", None),
                (2, "Report Domain: artistadvisory.io", None),
                (3, "Lunch Thursday?", None),
                (4, "Report Domain: chrisx.art", Some("user")),
            ] {
                conn.execute(
                    "INSERT INTO messages
                        (account_id, folder_name, uid, subject, from_email, to_list, date,
                         category, category_source, is_read, is_flagged)
                     VALUES ('acct', 'INBOX', ?1, ?2, 'dmarc@google.com', 'chris@cxventures.io',
                             '2026-08-01T10:00:00Z', 'primary', ?3, 0, 0)",
                    rusqlite::params![uid, subject, source],
                )
                .unwrap();
            }
        }

        let mcp = CxMailMcp::new(path.to_string_lossy().to_string());
        let text = |r: CallToolResult| serde_json::to_string(&r).unwrap();
        let state = |uid: u32| -> (String, i64) {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.query_row(
                "SELECT category, is_read FROM messages WHERE uid = ?1",
                rusqlite::params![uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };

        mcp.create_mail_rule(Parameters(CreateMailRuleParams {
            name: "DMARC reports".to_string(),
            account_id: None,
            is_active: None,
            priority: None,
            conditions: vec![cond("subject", "contains", "Report Domain:")],
            actions: vec![act("set_category", Some("updates")), act("mark_read", None)],
        }))
        .await
        .expect("create should succeed");

        // 1. dry_run DEFAULTS to true — an omitted flag must not write.
        let dry = text(
            mcp.apply_mail_rule(Parameters(ApplyMailRuleParams {
                id: 1,
                dry_run: None,
                account_id: None,
                limit: None,
            }))
            .await
            .expect("dry run should succeed"),
        );
        assert!(dry.contains(r#"\"dry_run\": true"#), "must default to a dry run: {dry}");
        assert!(dry.contains(r#"\"matched\": 3"#), "3 subjects match: {dry}");
        assert!(dry.contains(r#"\"changed\": 2"#), "only 2 are writable: {dry}");
        assert!(
            dry.contains(r#"\"skipped_user_override\": 1"#),
            "the manual override must be reported as skipped: {dry}"
        );
        assert!(
            dry.contains(r#"\"imap_push_failed\": 0"#)
                && dry.contains(r#"\"imap_pushed_read\": 0"#),
            "a dry run must not touch IMAP either: {dry}"
        );
        assert_eq!(state(1), ("primary".to_string(), 0), "a dry run must not write");
        assert_eq!(state(4), ("primary".to_string(), 0));

        // 2. dry_run=false writes, and the counts match what the dry run said.
        let live = text(
            mcp.apply_mail_rule(Parameters(ApplyMailRuleParams {
                id: 1,
                dry_run: Some(false),
                account_id: None,
                limit: None,
            }))
            .await
            .expect("apply should succeed"),
        );
        assert!(live.contains(r#"\"recategorized\": 2"#), "got: {live}");
        assert!(live.contains(r#"\"marked_read\": 2"#), "got: {live}");

        // 2b. (T26) IMAP is unreachable here, so both \Seen pushes fail —
        //     and that must NOT fail the apply or roll back the local half.
        //     A partial success reported honestly beats an abort that loses
        //     the category writes too.
        assert!(
            live.contains(r#"\"imap_push_failed\": 2"#),
            "both \\Seen pushes must be reported as failed: {live}"
        );
        assert!(
            live.contains(r#"\"imap_pushed_read\": 0"#),
            "nothing can have reached the server: {live}"
        );
        assert!(
            live.contains("PARTIAL"),
            "the note must say the flag push failed, not claim a clean apply: {live}"
        );
        assert!(
            live.contains("No access token found"),
            "the per-message reason must survive into imap_push_errors: {live}"
        );
        // The old (pre-T26) note claimed the read/flag half is local by
        // design. That is no longer true and must not come back.
        assert!(
            !live.contains("LOCAL ONLY"),
            "the local-only warning is stale — the tool pushes to IMAP now: {live}"
        );
        assert_eq!(state(1), ("updates".to_string(), 1));
        assert_eq!(state(2), ("updates".to_string(), 1));
        assert_eq!(state(3), ("primary".to_string(), 0), "a non-match must be untouched");
        assert_eq!(
            state(4),
            ("primary".to_string(), 0),
            "category_source='user' must survive an apply"
        );
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            let src: String = conn
                .query_row(
                    "SELECT category_source FROM messages WHERE uid = 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(src, "rule", "an applied category must be attributed to the rule");
        }

        // 3. Re-running is a no-op: nothing is left to change.
        let again = text(
            mcp.apply_mail_rule(Parameters(ApplyMailRuleParams {
                id: 1,
                dry_run: Some(false),
                account_id: None,
                limit: None,
            }))
            .await
            .unwrap(),
        );
        assert!(again.contains(r#"\"changed\": 0"#), "apply must be idempotent: {again}");
        assert!(
            again.contains(r#"\"imap_push_failed\": 0"#),
            "nothing changed, so nothing may be re-pushed: {again}"
        );

        // 4. A legacy rule the MCP would never have created — inserted
        //    straight into the DB, as the UI or a pre-T17 build could — is
        //    refused rather than run over the whole mailbox.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            crate::db::rules::insert(
                &conn,
                &crate::db::rules::MailRule {
                    id: None,
                    account_id: None,
                    name: "catch-all".to_string(),
                    is_active: true,
                    priority: 0,
                    conditions: vec![], // vacuously true — matches everything
                    actions: vec![crate::db::rules::RuleAction {
                        action_type: "delete".to_string(),
                        value: None,
                    }],
                },
            )
            .unwrap();
        }
        let err = mcp
            .apply_mail_rule(Parameters(ApplyMailRuleParams {
                id: 2,
                dry_run: Some(false),
                account_id: None,
                limit: None,
            }))
            .await
            .expect_err("a catch-all rule with a dead action must be refused");
        let msg = format!("{err:?}");
        assert!(msg.contains("EVERY message"), "must name the blast radius: {msg}");
        assert!(msg.contains("never executed"), "must name the dead action: {msg}");
        assert_eq!(state(3), ("primary".to_string(), 0), "refusal must write nothing");

        // 5. A missing rule fails loudly rather than silently doing nothing.
        assert!(mcp
            .apply_mail_rule(Parameters(ApplyMailRuleParams {
                id: 99,
                dry_run: Some(true),
                account_id: None,
                limit: None,
            }))
            .await
            .is_err());

        let _ = std::fs::remove_file(&path);
    }

    // ─── Pinned voice rules (T57) ──────────────────────────────

    #[test]
    fn pinned_rule_scope_is_inferred_from_the_address() {
        let (scope, recipient, rule) =
            validate_pinned_rule(None, Some("Sam@Harborline.example"), "  Address as Bro. Ellis  ")
                .unwrap();
        assert_eq!(scope, db::voice_pinned_rules::Scope::Recipient);
        assert_eq!(recipient, "sam@harborline.example", "must be normalized for lookup");
        assert_eq!(rule, "Address as Bro. Ellis", "must be trimmed");

        let (scope, recipient, _) = validate_pinned_rule(None, None, "Never say 'Hope you're well'").unwrap();
        assert_eq!(scope, db::voice_pinned_rules::Scope::Account);
        assert_eq!(recipient, "");
    }

    /// The two ways scope and address can contradict each other fail in
    /// opposite, equally silent directions — so both are refused.
    #[test]
    fn a_scope_that_contradicts_the_address_is_refused() {
        let err = format!(
            "{:?}",
            validate_pinned_rule(Some("account"), Some("sam@harborline.example"), "R").unwrap_err()
        );
        assert!(err.contains("every message"), "must name the blast radius: {err}");

        let err = format!("{:?}", validate_pinned_rule(Some("recipient"), None, "R").unwrap_err());
        assert!(err.contains("recipient_email"), "must name the missing field: {err}");
    }

    #[test]
    fn a_rule_pinned_to_a_name_instead_of_an_address_is_refused() {
        // The silent-failure case that motivates the check: this would be
        // stored happily and then never match anything.
        let err = format!(
            "{:?}",
            validate_pinned_rule(Some("recipient"), Some("Sam Ellis"), "R").unwrap_err()
        );
        assert!(err.contains("not an email address"), "got: {err}");
    }

    #[test]
    fn empty_overlong_and_unknown_scope_rules_are_refused() {
        assert!(validate_pinned_rule(None, None, "   ").is_err());
        let long = "x".repeat(db::voice_pinned_rules::MAX_RULE_CHARS + 1);
        assert!(validate_pinned_rule(None, None, &long).is_err());
        assert!(validate_pinned_rule(None, None, &"x".repeat(db::voice_pinned_rules::MAX_RULE_CHARS)).is_ok());

        // Scope is matched case-insensitively — "Account" is unambiguous, and
        // rejecting it would be pedantry, not a caught mistake.
        assert_eq!(
            validate_pinned_rule(Some("Account"), None, "R").unwrap().0,
            db::voice_pinned_rules::Scope::Account
        );
        // A value that is NOT one of the two is a real mistake and names both.
        let err = format!("{:?}", validate_pinned_rule(Some("global"), None, "R").unwrap_err());
        assert!(err.contains("Unknown scope"), "{err}");
        assert!(err.contains("recipient") && err.contains("account"), "must name the valid values: {err}");
    }

    #[test]
    fn pinned_rule_advisory_is_silent_with_no_rules() {
        assert_eq!(pinned_rule_advisory(&[]), "");
    }

    #[test]
    fn pinned_rule_advisory_names_the_rules_and_the_remedy() {
        let rule = db::voice_pinned_rules::PinnedRule {
            id: 1,
            account_id: "acct".to_string(),
            scope: db::voice_pinned_rules::Scope::Recipient,
            recipient_email: "sam@harborline.example".to_string(),
            rule: "Always address as \"Bro. Ellis\"".to_string(),
            created_at: "2026-08-10".to_string(),
        };
        let note = pinned_rule_advisory(&[rule]);
        assert!(note.contains("Bro. Ellis"), "the rule text must be quoted back: {note}");
        assert!(note.contains("OVERRIDE"), "{note}");
        assert!(note.contains("edit_draft"), "must name the fix: {note}");
    }

    /// End-to-end acceptance path for T57 against a real schema DB: pin a rule,
    /// see it on `get_voice_profile`, survive a forced profile rebuild, and
    /// delete it. Drives the actual tool functions.
    #[tokio::test]
    async fn pinned_voice_rules_survive_a_forced_profile_rebuild() {
        let path = std::env::temp_dir().join(format!(
            "cxmail-mcp-pinned-{}-{}.db",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            crate::db::schema::initialize(&conn).unwrap();
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
                 VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
                [],
            )
            .unwrap();
            // A second, untouched account: no profile and no rules anywhere, so
            // it can prove the "nothing to say" path still errors after the
            // first account has picked up an account-wide rule.
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
                 VALUES ('acct2', 'chris@artistadvisory.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
                [],
            )
            .unwrap();
            // The real shape of the problem: a derived profile that hedges,
            // because the sent corpus genuinely is mixed.
            crate::db::voice_profiles_recipient::upsert(
                &conn,
                "acct",
                "sam@harborline.example",
                r#"{"summary":"warm","typical_greeting":"Hi [Name], or Hey [Name], often with a casual address like 'Hey Bro. Ellis,'"}"#,
                "claude-opus-5",
                8,
                "2026-07-25T10:00:00Z",
            )
            .unwrap();
        }

        let mcp = CxMailMcp::new(path.to_string_lossy().to_string());
        let text = |r: CallToolResult| serde_json::to_string(&r).unwrap();

        // 1. Pin the rule.
        let set = text(
            mcp.set_voice_rule(Parameters(SetVoiceRuleParams {
                account_id: None,
                recipient_email: Some("sam@harborline.example".to_string()),
                scope: None,
                rule: "Always address as \"Bro. Ellis\", never \"Sam\".".to_string(),
            }))
            .await
            .expect("set should succeed"),
        );
        assert!(set.contains("Pinned rule id=1"), "got: {set}");

        // Re-pinning is a no-op, not a duplicate.
        let again = text(
            mcp.set_voice_rule(Parameters(SetVoiceRuleParams {
                account_id: None,
                recipient_email: Some("SAM@harborline.example".to_string()),
                scope: None,
                rule: "Always address as \"Bro. Ellis\", never \"Sam\".".to_string(),
            }))
            .await
            .unwrap(),
        );
        assert!(again.contains("already pinned"), "got: {again}");

        // 2. An account-wide rule applies to the same draft.
        mcp.set_voice_rule(Parameters(SetVoiceRuleParams {
            account_id: None,
            recipient_email: None,
            scope: None,
            rule: "Never open with \"Hope you're well\".".to_string(),
        }))
        .await
        .unwrap();

        // 3. get_voice_profile returns them in their OWN field, marked absolute.
        let profile = text(
            mcp.get_voice_profile(Parameters(GetVoiceProfileParams {
                account_id: None,
                recipient_email: Some("sam@harborline.example".to_string()),
                archetype_id: None,
                include_samples: Some(false),
            }))
            .await
            .unwrap(),
        );
        assert!(profile.contains("pinned_rules"), "got: {profile}");
        assert!(profile.contains("Bro. Ellis"));
        assert!(profile.contains("Hope you're well"), "account rules apply too: {profile}");
        assert!(profile.contains("ABSOLUTE"), "must not read as more soft description: {profile}");
        // Account rule first, recipient rule second — the precedence contract.
        assert!(
            profile.find("Hope you're well").unwrap() < profile.find("Bro. Ellis").unwrap(),
            "account rule must be listed before the recipient rule: {profile}"
        );

        // 4. THE ACCEPTANCE CRITERION: a forced rebuild overwrites profile_json
        //    and must leave the pinned rules untouched. `extract_recipient_profile`
        //    itself needs a live LLM, so this drives its one and only write.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            crate::db::voice_profiles_recipient::upsert(
                &conn,
                "acct",
                "sam@harborline.example",
                r#"{"summary":"rebuilt","typical_greeting":"Hi [Name]"}"#,
                "claude-opus-5",
                9,
                "2026-08-10T10:00:00Z",
            )
            .unwrap();
        }
        let after = text(
            mcp.get_voice_profile(Parameters(GetVoiceProfileParams {
                account_id: None,
                recipient_email: Some("sam@harborline.example".to_string()),
                archetype_id: None,
                include_samples: Some(false),
            }))
            .await
            .unwrap(),
        );
        assert!(after.contains("rebuilt"), "the profile really was rebuilt: {after}");
        assert!(
            after.contains("Bro. Ellis"),
            "a forced rebuild must NOT erase a pinned rule: {after}"
        );

        // 5. A recipient with rules but no cached profile still returns them —
        //    erroring here would hide the rule for every new contact.
        mcp.set_voice_rule(Parameters(SetVoiceRuleParams {
            account_id: None,
            recipient_email: Some("stranger@example.com".to_string()),
            scope: None,
            rule: "Keep it to three sentences.".to_string(),
        }))
        .await
        .unwrap();
        let stranger = text(
            mcp.get_voice_profile(Parameters(GetVoiceProfileParams {
                account_id: None,
                recipient_email: Some("stranger@example.com".to_string()),
                archetype_id: None,
                include_samples: Some(false),
            }))
            .await
            .expect("pinned rules must be reachable without a cached profile"),
        );
        assert!(stranger.contains("three sentences"), "got: {stranger}");
        assert!(stranger.contains("pinned-rules-only"), "got: {stranger}");

        // An unknown recipient on THIS account is not an error any more, and
        // should not be: the account-wide rule genuinely governs that message.
        let unknown = text(
            mcp.get_voice_profile(Parameters(GetVoiceProfileParams {
                account_id: None,
                recipient_email: Some("nobody@example.com".to_string()),
                archetype_id: None,
                include_samples: Some(false),
            }))
            .await
            .expect("an account-wide rule applies to every recipient"),
        );
        assert!(unknown.contains("Hope you're well"), "got: {unknown}");
        assert!(!unknown.contains("Bro. Ellis"), "got: {unknown}");

        // With no profile AND no rules of any scope there is genuinely nothing
        // to return, and the original error still stands.
        assert!(mcp
            .get_voice_profile(Parameters(GetVoiceProfileParams {
                account_id: Some("acct2".to_string()),
                recipient_email: Some("nobody@example.com".to_string()),
                archetype_id: None,
                include_samples: Some(false),
            }))
            .await
            .is_err());

        // 6. list_voice_rules: effective view vs full audit.
        let effective = text(
            mcp.list_voice_rules(Parameters(ListVoiceRulesParams {
                account_id: None,
                recipient_email: Some("sam@harborline.example".to_string()),
            }))
            .await
            .unwrap(),
        );
        assert!(effective.contains(r#"\"count\": 2"#), "account + recipient: {effective}");
        assert!(!effective.contains("three sentences"), "another recipient's rule must not leak: {effective}");

        let all = text(
            mcp.list_voice_rules(Parameters(ListVoiceRulesParams {
                account_id: None,
                recipient_email: None,
            }))
            .await
            .unwrap(),
        );
        assert!(all.contains(r#"\"count\": 3"#), "got: {all}");

        // 7. The draft advisory fires for the recipient's rules and the
        //    account's, and stays silent for an unrelated recipient's.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            let note = pinned_rule_advisory(&mcp.pinned_rules_for_draft(
                &conn,
                "acct",
                &["sam@harborline.example".to_string()],
            ));
            assert!(note.contains("Bro. Ellis"), "got: {note}");
            assert!(note.contains("Hope you're well"), "got: {note}");
            assert!(!note.contains("three sentences"), "got: {note}");

            // Account rules alone still warrant a note.
            let other = pinned_rule_advisory(&mcp.pinned_rules_for_draft(
                &conn,
                "acct",
                &["nobody@example.com".to_string()],
            ));
            assert!(other.contains("Hope you're well"), "got: {other}");
            assert!(!other.contains("Bro. Ellis"), "got: {other}");
        }

        // 8. Delete, and confirm it is gone from both read paths.
        let deleted = text(mcp.delete_voice_rule(Parameters(DeleteVoiceRuleParams { id: 1 })).await.unwrap());
        assert!(deleted.contains("Bro. Ellis"), "must quote what it removed: {deleted}");
        let after_delete = text(
            mcp.list_voice_rules(Parameters(ListVoiceRulesParams {
                account_id: None,
                recipient_email: Some("sam@harborline.example".to_string()),
            }))
            .await
            .unwrap(),
        );
        assert!(!after_delete.contains("Bro. Ellis"), "got: {after_delete}");
        assert!(mcp
            .delete_voice_rule(Parameters(DeleteVoiceRuleParams { id: 1 }))
            .await
            .is_err(), "deleting a gone rule must fail loudly");

        let _ = std::fs::remove_file(&path);
    }
}
