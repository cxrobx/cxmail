import { invoke } from "@tauri-apps/api/core";
import type { ChatSeed, ChatStarted } from "@/types/chat";
import type {
  Account,
  ImapAccountSettings,
  DiscoveredConfig,
  Folder,
  MessagePage,
  MessageDetail,
  SyncResult,
  ContactResult,
  SearchResult,
  SearchFilters,
  AiSearchResponse,
  OutgoingEmail,
  OutgoingAttachment,
  SendResult,
  SavedDraftRef,
  SnoozedMessageView,
  ScheduledEmail,
  FollowupReminder,
  TrackingConfig,
  TrackingPixel,
  EmailTemplate,
  Identity,
  SendAsAddress,
  SendAsSuggestion,
  CalendarEvent,
  CalendarConnection,
  CalendarEventInput,
  CalendarEventUpdate,
  InboxGroup,
  MailRule,
  ReclassifyResult,
  UnsubscribeResult,
  VoiceProfileStatus,
  AiDraftFinding,
  InsightStatus,
  LearnResult,
  RecipientProfileStatus,
  Archetype,
  NeedsYouItem,
  NeedsYouRef,
  Nudge,
  NudgeKind,
  TriageStatus,
  TriagePassStats,
} from "@/types/email";

/**
 * Where "Open in Claude" lands, and what decided it.
 *
 * Four scopes, resolved most-specific-first in Rust
 * (`db::claude_repos::resolve_for_message`): a mapped `contact` (address or
 * domain) beats a `group` whose rules match, which beats the `account` the mail
 * arrived in, which beats a group merely containing that account, which beats
 * the `default`. Nothing mapped means the old behaviour — a scratch directory.
 */
export type ClaudeRepoScope = "contact" | "group" | "account" | "default";

/** Which mapping to write or clear. Tagged so an impossible combination — a
 *  contact row carrying a group id — cannot be expressed. */
export type ClaudeRepoKey =
  | { scope: "contact"; contact: string }
  | { scope: "group"; group_id: number }
  | { scope: "account"; account_id: string }
  | { scope: "default" };

export interface ClaudeRepo {
  scope: ClaudeRepoScope;
  contact: string | null;
  group_id: number | null;
  account_id: string | null;
  repo_path: string;
  /** Computed per read — a mapping stays valid while its volume is unmounted. */
  exists: boolean;
}

export interface ResolvedClaudeRepo {
  repo_path: string;
  scope: ClaudeRepoScope;
  /** The row that decided it: a domain, a group name, an account address. */
  source: string;
}

export interface ClaudeHandoff {
  working_dir: string;
  repo: ResolvedClaudeRepo | null;
  /** Mapped, but the directory is gone — the session landed in the scratch
   *  directory instead. Never a failure. */
  missing_repo_path: string | null;
}

export type AIProvider = "openai" | "anthropic" | "compatible";

export interface AIProviderSettings {
  provider: AIProvider;
  model: string;
  baseUrl: string;
  apiKeyConfigured: boolean;
  maskedApiKey: string | null;
}

export interface SaveAIProviderSettings {
  provider: AIProvider;
  model: string;
  baseUrl: string;
  apiKey?: string | null;
  clearApiKey?: boolean;
}

export interface WriterSettings {
  /** Stored override; null = using the built-in default. */
  model: string | null;
  /** What an MCP draft call omitting writer_model will actually use. */
  effectiveModel: string;
  builtinDefault: string;
}

export interface WriterModel {
  id: string;
  label: string;
}

export interface ZoomStatus {
  configured: boolean;
  /** Masked — the client secret is never returned in any form. */
  maskedAccountId: string | null;
  /** Links whose Google event is gone; a real meeting may be running unreferenced. */
  orphanCount: number;
  /** Links that could not be verified. Never auto-deleted, so they need surfacing. */
  unverifiedCount: number;
}

export interface SaveZoomCredentials {
  accountId: string;
  clientId: string;
  clientSecret: string;
}

export interface LicenseStatus {
  active: boolean;
  maskedKey: string | null;
  validatedAt: string | null;
  offlineGrace: boolean;
  developmentBuild: boolean;
}

export const api = {
  license: {
    getStatus: () => invoke<LicenseStatus>("get_license_status"),
    refresh: () => invoke<LicenseStatus>("refresh_license_status"),
    activate: (licenseKey: string) => invoke<LicenseStatus>("activate_license", { licenseKey }),
  },
  accounts: {
    list: () => invoke<Account[]>("list_accounts"),
    remove: (accountId: string) =>
      invoke<void>("remove_account", { accountId }),
    rename: (accountId: string, displayName: string) =>
      invoke<void>("rename_account", { accountId, displayName }),
    reorder: (accountIds: string[]) =>
      invoke<void>("reorder_accounts", { accountIds }),
    setGroup: (accountId: string, groupName: string | null) =>
      invoke<void>("set_account_group", { accountId, groupName }),
    setNotifyEnabled: (accountId: string, enabled: boolean) =>
      invoke<void>("set_account_notify_enabled", { accountId, enabled }),
    setHiddenFromAggregates: (accountId: string, hidden: boolean) =>
      invoke<void>("set_account_hidden_from_aggregates", { accountId, hidden }),
    setTrackOpensEnabled: (accountId: string, enabled: boolean) =>
      invoke<void>("set_account_track_opens_enabled", { accountId, enabled }),
  },
  auth: {
    startOAuth2: (provider: string) =>
      invoke<string>("start_oauth2", { provider }),
    addICloudAccount: (email: string, password: string) =>
      invoke<void>("add_icloud_account", { email, password }),
    /**
     * Generic IMAP/SMTP account. Omit any `settings` field the preset should
     * fill — the backend resolves preset → explicit value → port-implied
     * security, and refuses rather than guessing when none of those answer.
     */
    /**
     * Look up server settings for an address. Safe to call as the user types:
     * no connection is made and no credential is sent.
     */
    discoverMailConfig: (email: string) =>
      invoke<DiscoveredConfig | null>("discover_mail_config", { email }),
    addImapAccount: (
      email: string,
      password: string,
      settings: ImapAccountSettings,
    ) => invoke<void>("add_imap_account", { email, password, settings }),
  },
  folders: {
    list: (accountId: string) =>
      invoke<Folder[]>("list_folders", { accountId }),
    sync: (accountId: string) =>
      invoke<Folder[]>("sync_folders", { accountId }),
    create: (accountId: string, folderName: string) =>
      invoke<Folder[]>("create_folder", { accountId, folderName }),
    delete: (accountId: string, folderName: string) =>
      invoke<Folder[]>("delete_folder", { accountId, folderName }),
    rename: (accountId: string, oldName: string, newName: string) =>
      invoke<Folder[]>("rename_folder", { accountId, oldName, newName }),
  },
  messages: {
    fetch: (
      accountId: string,
      folder: string,
      page: number,
      pageSize: number,
      category?: string | null,
      unreadOnly?: boolean,
    ) =>
      invoke<MessagePage>("fetch_messages", {
        accountId,
        folder,
        page,
        pageSize,
        category: category ?? null,
        unreadOnly: unreadOnly ?? null,
      }),
    fetchBody: (accountId: string, folder: string, uid: number) =>
      invoke<MessageDetail>("fetch_message_body", {
        accountId,
        folder,
        uid,
      }),
    fetchCachedBody: (accountId: string, folder: string, uid: number) =>
      invoke<MessageDetail | null>("get_cached_message_body", {
        accountId,
        folder,
        uid,
      }),
    sync: (accountId: string, folder: string) =>
      invoke<SyncResult>("sync_folder", { accountId, folder }),
    markRead: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>("mark_as_read", { accountId, folder, uids }),
    markUnread: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>("mark_as_unread", { accountId, folder, uids }),
    fetchUnifiedInbox: (
      page: number,
      pageSize: number,
      category?: string | null,
      accountIds?: string[] | null,
      unreadOnly?: boolean,
    ) =>
      invoke<MessagePage>("fetch_unified_inbox", {
        page,
        pageSize,
        category: category ?? null,
        accountIds: accountIds ?? null,
        unreadOnly: unreadOnly ?? null,
      }),
    syncAllInboxes: () => invoke<SyncResult>("sync_all_inboxes"),
    forceFullSync: (accountId?: string) =>
      invoke<SyncResult>("force_full_sync", { accountId: accountId ?? null }),
    move: (accountId: string, folder: string, uids: number[], destination: string) =>
      invoke<void>("move_messages", { accountId, folder, uids, destination }),
    archive: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>("archive_messages", { accountId, folder, uids }),
    delete: (accountId: string, folder: string, uids: number[]) =>
      invoke<void>("delete_messages", { accountId, folder, uids }),
    toggleStar: (accountId: string, folder: string, uid: number, starred: boolean) =>
      invoke<void>("toggle_star", { accountId, folder, uid, starred }),
    toggleMute: (accountId: string, folder: string, uid: number, muted: boolean) =>
      invoke<void>("toggle_mute", { accountId, folder, uid, muted }),
    togglePin: (accountId: string, folder: string, uid: number, pinned: boolean) =>
      invoke<void>("toggle_pin", { accountId, folder, uid, pinned }),
    search: (
      query: string,
      opts?: { accountIds?: string[]; prefix?: boolean; limit?: number; offset?: number },
    ) =>
      invoke<SearchResult[]>("search_messages", {
        query,
        accountIds: opts?.accountIds ?? null,
        prefix: opts?.prefix ?? null,
        limit: opts?.limit ?? null,
        offset: opts?.offset ?? null,
      }),
    aiSearch: (query: string, accountIds?: string[]) =>
      invoke<AiSearchResponse>("ai_search_messages", {
        query,
        accountIds: accountIds ?? null,
      }),
    searchWithFilters: (
      filters: SearchFilters,
      opts?: { limit?: number; offset?: number },
    ) =>
      invoke<SearchResult[]>("search_with_filters", {
        filters,
        limit: opts?.limit ?? null,
        offset: opts?.offset ?? null,
      }),
    serverSearch: (filters: SearchFilters, accountIds?: string[]) =>
      invoke<SearchResult[]>("server_search", {
        filters,
        accountIds: accountIds ?? null,
      }),
    getThread: (accountId: string, messageId: string) =>
      invoke<MessagePage["messages"]>("get_thread", { accountId, messageId }),
    threadUidsInFolder: (accountId: string, folder: string, threadRootId: string) =>
      invoke<number[]>("get_thread_uids_in_folder", { accountId, folder, threadRootId }),
    searchContacts: (query: string) =>
      invoke<ContactResult[]>("search_contacts", { query }),
    unsubscribe: (accountId: string, folder: string, uid: number) =>
      invoke<UnsubscribeResult>("unsubscribe", { accountId, folder, uid }),
    unsubscribeSender: (accountId: string, senderEmail: string) =>
      invoke<UnsubscribeResult>("unsubscribe_sender", { accountId, senderEmail }),
    listUnsubscribedSenders: (accountId?: string | null) =>
      invoke<string[]>("list_unsubscribed_senders", { accountId: accountId ?? null }),
    recordUnsubscribedSender: (accountId: string, senderEmail: string, method: string) =>
      invoke<void>("record_unsubscribed_sender", { accountId, senderEmail, method }),
    removeUnsubscribedSender: (accountId: string, senderEmail: string) =>
      invoke<void>("remove_unsubscribed_sender", { accountId, senderEmail }),
  },
  compose: {
    send: (accountId: string, email: OutgoingEmail, delaySeconds?: number) =>
      invoke<SendResult>("send_email", { accountId, email, delaySeconds: delaySeconds ?? null }),
    cancelSend: (sendId: string) =>
      invoke<void>("cancel_send", { sendId }),
    saveDraft: (accountId: string, email: OutgoingEmail) =>
      invoke<SavedDraftRef>("save_draft", { accountId, email }),
    editDraft: (accountId: string, folder: string, uid: number, email: OutgoingEmail) =>
      invoke<SavedDraftRef>("edit_draft", { accountId, folder, uid, email }),
    downloadAttachment: (
      accountId: string,
      folder: string,
      uid: number,
      attachmentIndex: number,
      savePath: string
    ) =>
      invoke<void>("download_attachment", {
        accountId,
        folder,
        uid,
        attachmentIndex,
        savePath,
      }),
    readFile: (path: string) =>
      invoke<[string, string, string]>("read_file_as_base64", { path }),
    fetchOutgoingAttachments: (accountId: string, folder: string, uid: number) =>
      invoke<OutgoingAttachment[]>("fetch_outgoing_attachments", { accountId, folder, uid }),
  },
  snooze: {
    snooze: (accountId: string, folder: string, uid: number, wakeAt: string) =>
      invoke<void>("snooze_message", { accountId, folder, uid, wakeAt }),
    unsnooze: (accountId: string, folder: string, uid: number) =>
      invoke<void>("unsnooze_message", { accountId, folder, uid }),
    listSnoozed: () =>
      invoke<SnoozedMessageView[]>("list_snoozed_messages"),
  },
  schedule: {
    schedule: (accountId: string, email: OutgoingEmail, sendAt: string) =>
      invoke<{ id: number; superseded: number }>("schedule_send", { accountId, email, sendAt }),
    list: () =>
      invoke<ScheduledEmail[]>("list_scheduled"),
    cancel: (id: number) =>
      invoke<void>("cancel_scheduled", { id }),
    edit: (id: number, email: OutgoingEmail, sendAt: string) =>
      invoke<void>("edit_scheduled", { id, email, sendAt }),
  },
  ai: {
    summarize: (accountId: string, folder: string, uid: number) =>
      invoke<string>("summarize_message", { accountId, folder, uid }),
    getApiKey: () => invoke<string | null>("get_ai_api_key"),
    saveApiKey: (apiKey: string) =>
      invoke<void>("save_ai_api_key", { apiKey }),
    getProviderSettings: () =>
      invoke<AIProviderSettings>("get_ai_provider_settings"),
    saveProviderSettings: (settings: SaveAIProviderSettings) =>
      invoke<AIProviderSettings>("save_ai_provider_settings", { settings }),
    testProvider: () => invoke<string>("test_ai_provider"),
    getWriterSettings: () => invoke<WriterSettings>("get_writer_settings"),
    saveWriterModel: (model: string | null) =>
      invoke<WriterSettings>("save_writer_model", { model }),
    listWriterModels: () => invoke<WriterModel[]>("list_writer_models"),
    getTriageStatus: () => invoke<TriageStatus>("get_triage_status"),
    setTriageMode: (mode: string) => invoke<string>("set_triage_mode", { mode }),
    setTriageModel: (model: string) => invoke<string>("set_triage_model", { model }),
    setTriageEffort: (effort: string) => invoke<string>("set_triage_effort", { effort }),
    setAccountTriageEnabled: (accountId: string, enabled: boolean) =>
      invoke<void>("set_account_triage_enabled", { accountId, enabled }),
    runTriagePassNow: () => invoke<TriagePassStats>("run_triage_pass_now"),
    generateReply: (accountId: string, folder: string, uid: number, context?: string) =>
      invoke<string>("ai_generate_reply", { accountId, folder, uid, context: context ?? "" }),
    rewriteText: (text: string, instruction: string) =>
      invoke<string>("ai_rewrite_text", { text, instruction }),
    adjustTone: (text: string, tone: string) =>
      invoke<string>("ai_adjust_tone", { text, tone }),
    proofread: (text: string) =>
      invoke<string>("ai_proofread", { text }),
    /** The contextual half of Validate. The instant half is `validateDraft()`
     *  in `src/lib/draftValidation.ts` and never touches IPC. */
    validateDraft: (args: {
      accountId: string;
      recipientEmail: string | null;
      subject: string;
      bodyText: string;
      replyFolder: string | null;
      replyUid: number | null;
    }) => invoke<AiDraftFinding[]>("ai_validate_draft", args),
    smartReplies: (accountId: string, folder: string, uid: number) =>
      invoke<{ replies: string[] }>("ai_smart_replies", { accountId, folder, uid }),
    suggestSubject: (text: string) =>
      invoke<string>("ai_suggest_subject", { text }),
    getVoiceProfileStatus: (accountId: string) =>
      invoke<VoiceProfileStatus>("ai_get_voice_profile_status", { accountId }),
    extractVoiceProfile: (accountId: string) =>
      invoke<VoiceProfileStatus>("ai_extract_voice_profile", { accountId }),
    logReplyEdit: (
      accountId: string,
      aiDraft: string,
      sentBody: string,
      recipientEmail?: string | null,
    ) =>
      invoke<void>("ai_log_reply_edit", {
        accountId,
        aiDraft,
        sentBody,
        recipientEmail: recipientEmail ?? null,
      }),
    getInsightStatus: (accountId: string) =>
      invoke<InsightStatus>("ai_get_insight_status", { accountId }),
    learnFromEdits: (accountId: string) =>
      invoke<LearnResult>("ai_learn_from_edits", { accountId }),
    getRecipientProfile: (accountId: string, recipientEmail: string) =>
      invoke<RecipientProfileStatus>("ai_get_recipient_profile", {
        accountId,
        recipientEmail,
      }),
    refreshRecipientProfile: (accountId: string, recipientEmail: string) =>
      invoke<RecipientProfileStatus>("ai_refresh_recipient_profile", {
        accountId,
        recipientEmail,
      }),
    generateCompose: (
      accountId: string,
      recipientEmail: string,
      subject?: string | null,
      instruction?: string | null,
    ) =>
      invoke<string>("ai_generate_compose", {
        accountId,
        recipientEmail,
        subject: subject ?? null,
        instruction: instruction ?? null,
      }),
    listArchetypes: (accountId: string) =>
      invoke<Archetype[]>("ai_list_archetypes", { accountId }),
    extractArchetypes: (accountId: string) =>
      invoke<Archetype[]>("ai_extract_archetypes", { accountId }),
    reclusterArchetypes: (accountId: string) =>
      invoke<Archetype[]>("ai_recluster_archetypes", { accountId }),
    renameArchetype: (
      accountId: string,
      archetypeId: string,
      newName: string,
    ) =>
      invoke<void>("ai_rename_archetype", {
        accountId,
        archetypeId,
        newName,
      }),
    deleteArchetype: (accountId: string, archetypeId: string) =>
      invoke<void>("ai_delete_archetype", { accountId, archetypeId }),
  },
  needsYou: {
    list: () => invoke<NeedsYouItem[]>("list_needs_you"),
    dismiss: (accountId: string, folder: string, uid: number) =>
      invoke<void>("dismiss_needs_you", { accountId, folder, uid }),
    // Collapsed rows stand for many messages; dismissing only the representative
    // would let the row return, represented by its next member.
    dismissGroup: (members: NeedsYouRef[]) =>
      invoke<number>("dismiss_needs_you_group", { members }),
  },
  nudges: {
    list: () => invoke<Nudge[]>("list_nudges"),
    // Dismissal is per-lane and per-thread: silencing "follow up?" says nothing
    // about whether they later write something you owe an answer to.
    dismiss: (accountId: string, threadKey: string, kind: NudgeKind) =>
      invoke<void>("dismiss_nudge", { accountId, threadKey, kind }),
  },
  categories: {
    getCounts: (accountId?: string | null, accountIds?: string[] | null) =>
      invoke<Record<string, number>>("get_category_counts", { accountId: accountId ?? null, accountIds: accountIds ?? null }),
    setCategory: (accountId: string, folder: string, uid: number, category: string) =>
      invoke<void>("set_message_category", { accountId, folder, uid, category }),
    setBatchCategory: (accountId: string, folder: string, uids: number[], category: string) =>
      invoke<void>("set_messages_category", { accountId, folder, uids, category }),
  },
  followup: {
    create: (
      accountId: string,
      sentMessageId: string,
      senderEmail: string,
      toEmail: string,
      subject: string | null,
      remindAt: string,
    ) =>
      invoke<number>("create_followup_reminder", {
        accountId,
        sentMessageId,
        senderEmail,
        toEmail,
        subject,
        remindAt,
      }),
    cancel: (id: number) =>
      invoke<void>("cancel_followup_reminder", { id }),
    dismiss: (id: number) =>
      invoke<void>("dismiss_followup_reminder", { id }),
    list: () =>
      invoke<FollowupReminder[]>("list_followup_reminders"),
  },
  tracking: {
    getConfig: () => invoke<TrackingConfig>("get_tracking_config"),
    setConfig: (config: TrackingConfig) =>
      invoke<void>("set_tracking_config", { config }),
    listPixels: (accountId?: string, page?: number) =>
      invoke<TrackingPixel[]>("list_tracking_pixels", { accountId: accountId ?? null, page: page ?? null }),
    getPixel: (pixelCode: string) =>
      invoke<TrackingPixel | null>("get_tracking_pixel", { pixelCode }),
    getPixelForMessage: (messageId: string) =>
      invoke<TrackingPixel | null>("get_tracking_pixel_for_message", { messageId }),
    syncEvents: () => invoke<void>("sync_tracking_events"),
    testConnection: () => invoke<boolean>("test_tracker_connection"),
  },
  templates: {
    list: (accountId?: string | null) =>
      invoke<EmailTemplate[]>("list_templates", { accountId: accountId ?? null }),
    create: (accountId: string | null, name: string, subject: string, htmlBody: string, plainBody?: string | null) =>
      invoke<number>("create_template", { accountId, name, subject, htmlBody, plainBody: plainBody ?? null }),
    update: (id: number, name: string, subject: string, htmlBody: string, plainBody?: string | null) =>
      invoke<void>("update_template", { id, name, subject, htmlBody, plainBody: plainBody ?? null }),
    delete: (id: number) =>
      invoke<void>("delete_template", { id }),
  },
  identities: {
    list: (accountId: string) =>
      invoke<Identity[]>("list_identities", { accountId }),
    /** Every address this account may send as, primary first. */
    listSendAs: (accountId: string) =>
      invoke<SendAsAddress[]>("list_send_as", { accountId }),
    /** The address a reply to this message should go out FROM — the send-as
     * the original was addressed to, or the account's own address. The match
     * runs in Rust so compose and the MCP cannot disagree about it. */
    replyFrom: (accountId: string, folder: string, uid: number) =>
      invoke<SendAsAddress>("reply_from_for_message", { accountId, folder, uid }),
    /** Same-domain addresses this account has received mail at and is not yet
     * configured to send as. Offered, never applied. */
    /** Remove a non-primary send-as (configured or found in Sent) and keep it
     * removed; adding it back undoes that. */
    removeSendAs: (accountId: string, email: string) =>
      invoke<void>("remove_send_as", { accountId, email }),
    suggestSendAs: (accountId: string) =>
      invoke<SendAsSuggestion[]>("suggest_send_as", { accountId }),
    create: (identity: Identity) =>
      invoke<number>("create_identity", { identity }),
    update: (identity: Identity) =>
      invoke<void>("update_identity", { identity }),
    delete: (id: number) =>
      invoke<void>("delete_identity", { id }),
  },
  calendar: {
    getEvents: (accountId: string, folder: string, uid: number) =>
      invoke<CalendarEvent[]>("get_calendar_events", { accountId, folder, uid }),
    rsvp: (eventId: number, accountId: string, response: string) =>
      invoke<void>("rsvp_event", { eventId, accountId, response }),
    listUpcoming: (accountId?: string | null, limit?: number) =>
      invoke<CalendarEvent[]>("list_upcoming_events", { accountId: accountId ?? null, limit: limit ?? 10 }),
    listByRange: (rangeStart: string, rangeEnd: string, accountId?: string | null) =>
      invoke<CalendarEvent[]>("list_calendar_events_in_range", {
        accountId: accountId ?? null,
        rangeStart,
        rangeEnd,
      }),
    dismiss: (eventId: number) =>
      invoke<void>("dismiss_calendar_event", { eventId }),
    connect: (email: string) =>
      invoke<string>("start_calendar_oauth", { email }),
    connectionStatus: (accountId: string) =>
      invoke<boolean>("calendar_connection_status", { accountId }),
    listGoogleCalendars: (accountId: string) =>
      invoke<CalendarConnection>("list_google_calendars", { accountId }),
    create: (input: CalendarEventInput) =>
      invoke<unknown>("create_google_calendar_event", { input }),
    update: (eventId: number, update: CalendarEventUpdate) =>
      invoke<unknown>("update_google_calendar_event", { eventId, update }),
    // `notify` is required, not defaulted: this is one of the few calls that
    // puts real mail in other people's inboxes, so every caller has to have
    // asked a human first. The backend clamps it — a caller can always choose
    // silence, and can never force mail the cancellation rule would not send.
    delete: (eventId: number, notify: boolean) =>
      invoke<void>("delete_google_calendar_event", { eventId, notify }),
    sendInvites: (eventId: number) =>
      invoke<unknown>("send_google_calendar_invites", { eventId }),
    syncNow: (accountId: string) =>
      invoke<void>("sync_google_calendar_now", { accountId }),
    resolveConflict: (eventId: number, resolution: "mine" | "theirs") =>
      invoke<void>("resolve_google_calendar_conflict", { eventId, resolution }),
  },
  zoom: {
    status: () => invoke<ZoomStatus>("zoom_connection_status"),
    save: (credentials: SaveZoomCredentials) =>
      invoke<ZoomStatus>("save_zoom_credentials", { credentials }),
    clear: () => invoke<ZoomStatus>("clear_zoom_credentials"),
    test: () => invoke<string>("test_zoom_connection"),
  },
  rules: {
    list: () => invoke<MailRule[]>("list_mail_rules"),
    create: (rule: MailRule) => invoke<number>("create_mail_rule", { rule }),
    update: (rule: MailRule) => invoke<void>("update_mail_rule", { rule }),
    delete: (id: number) => invoke<void>("delete_mail_rule", { id }),
    reclassifyAll: () =>
      invoke<ReclassifyResult>("reclassify_inbox_messages"),
  },
  claude: {
    openEmail: (accountId: string, folderName: string, uid: number) =>
      invoke<ClaudeHandoff>("open_email_in_claude", { accountId, folderName, uid }),
    getPrompt: (accountId: string, folderName: string, uid: number) =>
      invoke<string>("get_claude_prompt", { accountId, folderName, uid }),
    listRepos: () => invoke<ClaudeRepo[]>("list_claude_repos"),
    setRepo: (key: ClaudeRepoKey, repoPath: string) =>
      invoke<ClaudeRepo>("set_claude_repo", { key, repoPath }),
    clearRepo: (key: ClaudeRepoKey) => invoke<void>("clear_claude_repo", { key }),
  },
  chat: {
    start: (seed: ChatSeed | null, model: string | null) =>
      invoke<ChatStarted>("chat_start", { seed, model }),
    send: (text: string) => invoke<void>("chat_send", { text }),
    answerPermission: (requestId: string, allow: boolean, remember: boolean) =>
      invoke<void>("chat_answer_permission", { requestId, allow, remember }),
    interrupt: () => invoke<void>("chat_interrupt"),
    stop: () => invoke<void>("chat_stop"),
    continueInTerminal: () => invoke<void>("chat_continue_in_terminal"),
  },
  inboxGroups: {
    list: () => invoke<InboxGroup[]>("list_inbox_groups"),
    create: (name: string, color: string, icon: string, rules: { field: string; operator: string; value: string }[], accountIds: string[]) =>
      invoke<number>("create_inbox_group", { name, color, icon, rules, accountIds }),
    update: (id: number, name: string, color: string, icon: string, rules: { field: string; operator: string; value: string }[], accountIds: string[]) =>
      invoke<void>("update_inbox_group", { id, name, color, icon, rules, accountIds }),
    delete: (id: number) => invoke<void>("delete_inbox_group", { id }),
    fetchMessages: (groupId: number, page: number, pageSize: number, unreadOnly?: boolean) =>
      invoke<MessagePage>("fetch_inbox_group_messages", {
        groupId,
        page,
        pageSize,
        unreadOnly: unreadOnly ?? null,
      }),
  },
  imageTrust: {
    isTrusted: (senderEmail: string) =>
      invoke<boolean>("is_image_sender_trusted", { senderEmail }),
    trust: (senderEmail: string) =>
      invoke<void>("trust_image_sender", { senderEmail }),
    untrust: (senderEmail: string) =>
      invoke<void>("untrust_image_sender", { senderEmail }),
    list: () => invoke<string[]>("list_trusted_image_senders"),
  },
  images: {
    saveToPath: (src: string, destPath: string) =>
      invoke<void>("save_image_to_path", { src, destPath }),
    copyToClipboard: (src: string) =>
      invoke<void>("copy_image_to_clipboard", { src }),
  },
  system: {
    /** Request that CXMail become the default `mailto:` handler. Resolves to
     * whether Launch Services accepted — NOT a guarantee it silently applied
     * (macOS may show a confirmation prompt). */
    setAsDefaultMailClient: () => invoke<boolean>("set_as_default_mail_client"),
    isDefaultMailClient: () => invoke<boolean>("is_default_mail_client"),
    /** Record a frontend failure in the Rust log. Release builds have no
     * DevTools, so `console.warn` is invisible in production — use this for
     * anything a user would file a bug about. Never rejects. */
    logClientError: (scope: string, message: string) =>
      invoke<void>("log_client_error", { scope, message }).catch(() => {}),
  },
  appearance: {
    /** Pin AppKit's `NSApp.appearance`. The web layer cannot reach native
     * chrome (NSMenu, sheets, scrollbars), so the theme toggle has to say it
     * twice: once in CSS, once here. See `appearance_macos` on the Rust side.
     *
     * `null` means the `system` preference: stop pinning, follow macOS. It is
     * not the same as sending the resolved value — a pinned appearance is also
     * what the webview reads `prefers-color-scheme` from, so pinning would
     * freeze the frontend's system-change listener. */
    setNative: (dark: boolean | null) => invoke<void>("set_native_appearance", { dark }),
  },
  glass: {
    /** Turn the window's desktop blur on or off (`glass_macos` on the Rust side).
     *
     * The RGB triple is the theme's base colour and is sent on BOTH edges:
     * turning glass off has to repaint the window opaque in something, and the
     * place that colour is authoritative is `globals.css`. Passing it from here
     * rather than hardcoding it in Rust is what stops the two drifting the next
     * time a theme token moves. */
    set: (enabled: boolean, radius: number, rgb: readonly [number, number, number]) =>
      invoke<void>("set_window_glass", { enabled, radius, r: rgb[0], g: rgb[1], b: rgb[2] }),
    /** Change only the radius, for the transparency slider. Separate from
     * `set` so a drag doesn't re-run the window setup on every step. */
    setRadius: (radius: number) => invoke<void>("set_blur_radius", { radius }),
    /** Is macOS's Reduce Transparency on? Read at hydrate; changes after that
     * arrive as a `reduce-transparency-changed` event. */
    reduceTransparency: () => invoke<boolean>("get_reduce_transparency"),
  },
} as const;
