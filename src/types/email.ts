export interface Account {
  id: string;
  email: string;
  display_name: string | null;
  provider: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  /** "implicit" | "starttls" — stored, never inferred from the port. */
  imap_security: string;
  smtp_security: string;
  /** Login name when it differs from `email`; null = authenticate as `email`. */
  imap_username: string | null;
  smtp_username: string | null;
  color: string | null;
  is_active: boolean;
  sort_order: number;
  group_name: string | null;
  notify_enabled: boolean;
  track_opens_enabled: boolean;
  /** Kept out of every cross-account surface — All Inboxes, account folders,
   * inbox groups, Needs You, nudges, unscoped search, the category counts and
   * the dock badge. Its mail shows only when the account itself is clicked.
   * Notifications are the separate `notify_enabled` toggle. */
  hidden_from_aggregates: boolean;
  /** Let the background AI triage pass read this account's inbox. Default off.
   * Opt-in per account because the value of triage is concentrated exactly
   * where the sensitivity is, and that trade is made one account at a time. */
  triage_enabled: boolean;
}

/** Status of the background AI triage pass, for the settings panel. */
export interface TriageStatus {
  /** "off" | "shadow" | "on". Off is the default. */
  mode: string;
  /** Model triage uses. Independent of `ai:model`, which drives Reply with AI. */
  model: string;
  /** reasoning_effort sent with each call. Reasoning tokens bill as output. */
  effort: string;
  /** Tokens actually billed across every stored verdict — read, not estimated. */
  input_tokens: number;
  output_tokens: number;
  /** Messages the gate refused to send. Not verdicts. */
  withheld: number;
  /** Verdicts stored in total. */
  verdicts: number;
  /** Verdicts written in the last rolling day, against `daily_cap`. */
  today: number;
  daily_cap: number;
  /** Messages awaiting a verdict on triage-enabled accounts. */
  pending: number;
  /** False when no inference provider is configured — the pass no-ops. */
  provider_ready: boolean;
}

export interface TriagePassStats {
  considered: number;
  classified: number;
  withheld: number;
  failed: number;
  capped: boolean;
}

/**
 * Server settings for `add_imap_account`. Every field is optional: the backend
 * fills gaps from the preset (explicit values win) and refuses rather than
 * guessing when neither answers. Mirrors the Rust `ImapAccountSettings`.
 */
export interface ImapAccountSettings {
  preset_id?: string | null;
  imap_host?: string | null;
  imap_port?: number | null;
  imap_security?: string | null;
  smtp_host?: string | null;
  smtp_port?: number | null;
  smtp_security?: string | null;
  imap_username?: string | null;
  smtp_username?: string | null;
}

/**
 * Result of `discover_mail_config`. Discovery never connects and never sees the
 * password — these are settings for the form to DISPLAY, so the user can see
 * the hostname before any credential is sent. Mirrors Rust `DiscoveredConfig`.
 */
export interface DiscoveredConfig {
  display_name: string | null;
  imap_host: string;
  imap_port: number;
  imap_security: string;
  imap_username: string | null;
  smtp_host: string;
  smtp_port: number;
  smtp_security: string;
  smtp_username: string | null;
  /** Which tier answered: "preset" | "ispdb" | "autoconfig" | "srv" | "mx". */
  source: string;
  hint: string | null;
}

export interface Folder {
  id: number;
  account_id: string;
  name: string;
  display_name: string | null;
  folder_type: string | null;
  delimiter: string | null;
  total_count: number;
  unread_count: number;
  uidvalidity: number | null;
  uidnext: number | null;
}

export interface MessageSummary {
  uid: number;
  account_id: string | null;
  folder_name?: string | null;
  subject: string | null;
  from_name: string | null;
  from_email: string | null;
  date: string;
  snippet: string | null;
  is_read: boolean;
  is_flagged: boolean;
  has_attachments: boolean;
  size_bytes: number;
  category: string | null;
  is_muted: boolean;
  is_pinned: boolean;
  thread_count: number;
  /** Unsent drafts in this thread. Deliberately not folded into
   * `thread_count`, which is what the row's badge prints (`thread_count + 1`
   * = messages actually exchanged) — see the Rust doc on `MessageRow`. */
  thread_draft_count: number;
  thread_root_id: string | null;
  thread_has_unread: boolean;
}

export type NeedsYouAction = "decision" | "reply" | "follow_up" | "review" | "alert";

export interface NeedsYouRef {
  account_id: string;
  folder_name: string;
  uid: number;
}

export type NudgeKind = "follow_up" | "reply";

/** An automatic prompt on a stalled conversation — Gmail's "nudge". */
export interface Nudge {
  kind: NudgeKind;
  account_id: string;
  /** The row the badge attaches to: the newest INBOX message of the thread. */
  folder_name: string;
  uid: number;
  thread_key: string;
  subject: string | null;
  /** Who we're waiting on (follow_up) or who's waiting on us (reply). */
  counterpart_email: string;
  counterpart_name: string | null;
  /** Date of the message the nudge is about — our last send, or their last
   * message. Not the badge row's date; on a follow-up they differ. */
  date: string;
  days_ago: number;
}

export interface NeedsYouItem {
  uid: number;
  account_id: string;
  folder_name: string;
  subject: string | null;
  from_name: string | null;
  from_email: string | null;
  date: string;
  snippet: string | null;
  is_read: boolean;
  has_attachments: boolean;
  action_type: NeedsYouAction;
  reason: string;
  /** The text that actually triggered the match — shown instead of boilerplate. */
  evidence: string | null;
  /** How many messages this row stands for (repeat alerts, thread siblings). */
  duplicate_count: number;
  /** Every message behind the row; dismissing the row dismisses all of them. */
  members: NeedsYouRef[];
  score: number;
}

export type EmailCategory = "primary" | "updates" | "social" | "promotions" | "junk";

export interface MessagePage {
  messages: MessageSummary[];
  total: number;
  page: number;
  page_size: number;
  has_more: boolean;
}

export interface EmailAddress {
  name: string | null;
  email: string;
}

export interface AttachmentMeta {
  filename: string | null;
  content_type: string;
  size_bytes: number;
  content_id: string | null;
  is_inline: boolean;
}

export interface MessageDetail {
  uid: number;
  subject: string | null;
  from_name: string | null;
  from_email: string;
  to_list: EmailAddress[];
  cc_list: EmailAddress[];
  bcc_list: EmailAddress[];
  date: string | null;
  plain_text: string | null;
  sanitized_html: string | null;
  attachments: AttachmentMeta[];
  is_read: boolean;
  is_flagged: boolean;
  list_unsubscribe: string | null;
  list_unsubscribe_post: string | null;
  message_id: string | null;
  references: string | null;
  /** The message's own In-Reply-To. Drafts carry their own threading headers;
   * reopening one must round-trip them (gotcha #39). */
  in_reply_to: string | null;
}

export interface AccountSyncStatus {
  account_id: string;
  email: string;
  success: boolean;
  error: string | null;
  new_count: number;
  needs_reauth: boolean;
}

export interface SyncResult {
  new_count: number;
  updated_count: number;
  account_statuses: AccountSyncStatus[];
}

export interface ContactResult {
  name: string | null;
  email: string;
}

export interface SearchResult {
  account_id: string;
  folder_name: string;
  uid: number;
  subject: string | null;
  from_name: string | null;
  from_email: string | null;
  date: string;
  /** Contains U+E000 / U+E001 marker pairs around matched spans. */
  snippet: string;
  is_read: boolean;
  has_attachments: boolean;
  score: number;
  /** Client-side only: hit arrived via the IMAP server-search fallback. */
  from_server?: boolean;
}

/** One removable chip above search results (mirrors backend AppliedFilter). */
export interface AppliedFilter {
  kind: string;
  value: string;
}

/** Mirrors the backend db::search::SearchFilters (snake_case fields). */
export interface SearchFilters {
  keywords?: string | null;
  from?: string | null;
  to?: string | null;
  cc?: string | null;
  subject_contains?: string | null;
  filename?: string | null;
  date_after?: string | null;
  date_before?: string | null;
  folder?: string | null;
  has_attachments?: boolean | null;
  is_starred?: boolean | null;
  is_unread?: boolean | null;
  larger_bytes?: number | null;
  smaller_bytes?: number | null;
  account_ids?: string[] | null;
}

export interface AiSearchResponse {
  results: SearchResult[];
  applied_filters: AppliedFilter[];
  used_ai: boolean;
  ai_failed: boolean;
}

export interface OutgoingAttachment {
  filename: string;
  content_type: string;
  data_base64: string;
}

export interface OutgoingEmail {
  from_email: string;
  from_name: string | null;
  to: { name: string | null; email: string }[];
  cc: { name: string | null; email: string }[];
  bcc: { name: string | null; email: string }[];
  subject: string;
  html_body: string;
  plain_body: string | null;
  in_reply_to: string | null;
  references: string | null;
  track_opens?: boolean;
  attachments?: OutgoingAttachment[];
}

export interface TrackingConfig {
  is_enabled: boolean;
  service_url: string | null;
  api_key?: string | null;
  api_key_configured: boolean;
  last_synced: string | null;
}

export interface TrackingPixel {
  id: number;
  pixel_code: string;
  account_id: string;
  message_id: string | null;
  to_email: string;
  subject: string | null;
  open_count: number;
  first_open_at: string | null;
  last_open_at: string | null;
  created_at: string;
}

export interface SnoozedMessageView {
  uid: number;
  account_id: string;
  folder_name: string;
  subject: string | null;
  from_name: string | null;
  from_email: string | null;
  date: string;
  snippet: string | null;
  wake_at: string;
  is_read: boolean;
  is_flagged: boolean;
  has_attachments: boolean;
}

export interface ScheduledEmail {
  id: number;
  account_id: string;
  email_json: string;
  send_at: string;
  status: string;
  error_message: string | null;
  created_at: string;
}

export interface SendResult {
  send_id: string | null;
  message_id: string;
}

export interface SavedDraftRef {
  folder: string;
  uid: number;
  /** The draft's stable identity (v60); `uid` names one revision of it. */
  draft_id?: string | null;
}

export interface FollowupReminder {
  id: number;
  account_id: string;
  sent_message_id: string;
  to_email: string;
  subject: string | null;
  remind_at: string;
  status: string;
  created_at: string;
}

export interface EmailTemplate {
  id: number;
  account_id: string | null;
  name: string;
  subject: string;
  html_body: string;
  plain_body: string | null;
  created_at: string;
  updated_at: string;
}

export interface Identity {
  id: number | null;
  account_id: string;
  email: string;
  display_name: string | null;
  signature_html: string | null;
  is_default: boolean;
}

export interface InboxGroupRule {
  id: number;
  group_id: number;
  field: string;
  operator: string;
  value: string;
}

export interface InboxGroup {
  id: number;
  name: string;
  color: string;
  icon: string;
  sort_order: number;
  rules: InboxGroupRule[];
  account_ids: string[];
  unread_count: number;
}

export interface RuleCondition {
  field: string;    // "from" | "to" | "subject" | "body"
  operator: string; // "contains" | "equals" | "starts_with" | "ends_with"
  value: string;
}

export interface RuleAction {
  action_type: string; // "set_category" | "mark_read" | "mark_flagged" | "move" | "delete"
  value: string | null;
}

export interface MailRule {
  id: number | null;
  account_id: string | null;
  name: string;
  is_active: boolean;
  priority: number;
  conditions: RuleCondition[];
  actions: RuleAction[];
}

export interface ReclassifyResult {
  total: number;
  changed: number;
  junk: number;
  skipped_user_override: number;
}

export type UnsubscribeResult =
  | { type: "one-click"; confirmed: boolean }
  | { type: "browser"; url: string }
  | { type: "mailto"; email: string; subject: string | null; body: string | null };

export interface CalendarEvent {
  id: number;
  kind?: "mail" | "gcal";
  account_id: string;
  folder_name: string;
  message_uid: number;
  event_uid: string | null;
  summary: string | null;
  description: string | null;
  location: string | null;
  dtstart: string;
  dtend: string | null;
  organizer_name: string | null;
  organizer_email: string | null;
  status: string | null;
  method: string | null;
  rsvp_status: string;
  raw_ics: string | null;
  source: "ics" | "detected" | "gcal";
  confidence: number | null;
  dismissed: boolean;
  created_at: string;
  gcal_calendar_id?: string | null;
  gcal_event_id?: string | null;
  recurring_event_id?: string | null;
  attendees_json?: string | null;
  html_link?: string | null;
  hangout_link?: string | null;
  is_all_day?: boolean;
  start_tz?: string | null;
  sync_state?: "synced" | "local_new" | "local_dirty" | "local_deleted" | "conflict" | null;
  pending_notify?: boolean;
  attendee_delivery?: AttendeeDelivery[];
  /**
   * Whether deleting this event would actually email anyone. Decided in Rust
   * (`invite_notifications::cancellation_should_notify`) so the confirmation
   * gate can name real consequences — a dialog promising to email four guests
   * when no mail is sent is its own kind of lie.
   */
  cancellation_notifies?: boolean;
}

/**
 * What we can honestly say about whether one attendee knows about a meeting.
 *
 * Resolved in Rust (`db::invite_notifications::derive`) and rendered verbatim —
 * do NOT re-derive any of this from `attendees_json` here. Google records no
 * "was this invitation sent" fact anywhere on the event, and `responseStatus`
 * reads `needsAction` from the instant an attendee is attached, so the only
 * thing that can tell these states apart is CXMail's own ledger.
 *
 * `responded` is the sole proof of *receipt*. `sent` means Google accepted our
 * request to notify — it is not a delivery receipt and must never be labelled
 * "received", or it becomes the same lie in a nicer glyph.
 */
export type AttendeeDeliveryState = "responded" | "sent" | "unsent" | "unknown";

export interface AttendeeDelivery {
  email: string;
  display_name: string | null;
  /** Google's raw value: needsAction | accepted | declined | tentative. */
  response_status: string | null;
  is_self: boolean;
  state: AttendeeDeliveryState;
}

export interface GoogleCalendar {
  id: number;
  account_id: string;
  gcal_calendar_id: string;
  summary: string | null;
  time_zone: string | null;
  access_role: string | null;
  bg_color: string | null;
  is_primary: boolean;
  selected: boolean;
  sync_token: string | null;
  sync_window_days: number;
  last_synced_at: string | null;
  last_error: string | null;
}

export interface CalendarConnection {
  account_id: string;
  connected: boolean;
  calendars: GoogleCalendar[];
}

export interface CalendarEventInput {
  account_id: string;
  calendar_id?: string | null;
  summary: string;
  description?: string | null;
  location?: string | null;
  start: string;
  end: string;
  time_zone: string;
  attendees: string[];
  add_meet: boolean;
}

export interface CalendarEventUpdate {
  summary?: string;
  description?: string | null;
  location?: string | null;
  start?: string;
  end?: string;
  time_zone?: string;
  attendees?: string[];
}

export interface VoiceProfileStatus {
  exists: boolean;
  sample_count: number;
  model_used: string | null;
  generated_at: string | null;
}

export interface InsightStatus {
  pending_edits: number;
  active_insights: number;
  total_insights: number;
}

export interface LearnResult {
  extracted: number;
  activated: number;
}

export type RecipientProfileStatusKind =
  | "fresh"
  | "stale"
  | "missing"
  | "insufficient_samples";

export interface RecipientProfileStatus {
  status: RecipientProfileStatusKind;
  profile: string | null;
  sample_count: number;
  generated_at: string | null;
}

export interface Archetype {
  archetype_id: string;
  name: string;
  description: string;
  profile_json: string;
  sample_count: number;
  generated_at: string;
}

/** Payload of the `mcp-activity` Tauri event: a one-shot notification that the
 * standalone cxmail-mcp process mutated a draft/email. Treated as a hint to
 * re-fetch from the DB — see `src-tauri/src/mcp/bridge.rs`. */
export interface McpActivity {
  v: number;
  kind:
    | "draft-created"
    | "draft-updated"
    | "email-mutated"
    | "calendar"
    | "calendar-approval-request";
  account_id: string;
  folder: string;
  uid: number;
  /** Present on `draft-updated`: the UID the edit replaced (now expunged). */
  old_uid?: number;
  /** Originating MCP tool name (e.g. "edit_draft", "archive_email"). */
  tool?: string;
  /** Single-use correlation ID for an Approve-tier request. */
  approval_id?: string;
}

/**
 * A contextual finding from `ai_validate_draft`.
 *
 * Mirrors `email::ai::DraftFinding`. The Rust side guarantees `quote`, when
 * present, occurs verbatim in the draft body — the panel relies on that to
 * turn a finding into a find-and-replace.
 */
export interface AiDraftFinding {
  category:
    | "pinned_rule"
    | "unanswered_question"
    | "missing_context"
    | "clarity"
    | "next_step"
    | "consistency"
    | "tone";
  severity: "error" | "warning" | "info";
  where: string;
  title: string;
  detail: string;
  quote: string | null;
  suggestion: string | null;
}
