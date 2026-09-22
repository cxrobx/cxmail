# Plugin System

Phase 5. Embedded JS/TS runtime for user-written extensions.

## Runtime

Uses `rquickjs` (QuickJS) for:
- Lightweight sandboxed JavaScript runtime (smaller binary than V8)
- No filesystem/network access by default
- Async support
- Well-maintained by the Deno team

Alternative: `quickjs` via rquickjs crate (lighter, but no TypeScript).

## Plugin Directory

```
~/.config/cxmail/plugins/
├── auto-tagger/
│   ├── plugin.json
│   └── index.ts
├── newsletter-digest/
│   ├── plugin.json
│   └── index.ts
└── snooze/
    ├── plugin.json
    └── index.ts
```

## Manifest (plugin.json)

```json
{
  "name": "auto-tagger",
  "version": "1.0.0",
  "description": "Automatically tag emails based on content rules",
  "author": "cx",
  "permissions": ["read_emails", "flag_email", "search_emails"],
  "triggers": ["on_new_email"],
  "main": "index.ts"
}
```

### Permissions

Plugins declare which MCP-style tools they need access to. The plugin is only granted these capabilities. User approves permissions on install.

### Triggers

| Trigger | When it fires |
|---------|--------------|
| `on_new_email` | After sync fetches new messages |
| `on_send` | Before an email is sent (can modify or cancel) |
| `on_folder_change` | When user switches folders |
| `on_startup` | When the app launches |
| `on_schedule` | Cron-style schedule (defined in manifest) |

## Plugin API

Plugins access a `cxmail` global object injected into the JS runtime. Same tool interface as MCP.

```typescript
// Available in plugin context
declare const cxmail: {
  // Read
  searchEmails(query: string, options?: { folder?: string; limit?: number }): Promise<SearchResult[]>;
  readEmail(uid: number): Promise<MessageDetail>;
  listFolders(): Promise<Folder[]>;

  // Write (subject to declared permissions)
  composeDraft(to: string[], subject: string, body: string): Promise<Draft>;
  moveEmail(uid: number, fromFolder: string, toFolder: string): Promise<void>;
  flagEmail(uid: number, flag: string): Promise<void>;

  // Plugin-specific
  log(message: string): void;              // Logs to plugin console
  notify(title: string, body: string): void; // System notification
  getConfig(): Promise<Record<string, unknown>>; // Plugin-specific config
  setConfig(key: string, value: unknown): Promise<void>;
};
```

## Example Plugins

### auto-tagger
Tags emails based on rules: newsletters → "newsletter", GitHub → "dev", receipts → "finance".

### newsletter-digest
Collects newsletter emails throughout the day. At a scheduled time, generates a digest summary and presents it as a single view.

### snooze
Hides messages until a specified date/time. On trigger, moves snoozed messages back to inbox.

### tracking-pixel-reporter
Counts blocked tracking pixels per sender. Shows a report of which senders track you most.
