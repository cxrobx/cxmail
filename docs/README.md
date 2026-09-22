# CXMail Documentation

## Quick Links

| Document | Description |
|----------|-------------|
| [../README.md](../README.md) | Project landing page — what CXMail is, quick start |
| [CLAUDE.md](../CLAUDE.md) | Overview, commands, status |
| [CHANGELOG.md](../CHANGELOG.md) | Version history |

### Guides

| Document | Description |
|----------|-------------|
| [mcp-setup.md](mcp-setup.md) | Set up the built-in `cxmail-mcp` MCP server for Claude |
| [mail-providers.md](mail-providers.md) | Providers, generic IMAP, transport security, autodiscovery, folder classification |

### Rule Files (Auto-loaded by Claude Code)

| Document | Scope |
|----------|-------|
| [architecture.md](../.claude/rules/architecture.md) | Always loaded — tech stack, invariants, patterns |
| [gotchas.md](../.claude/rules/gotchas.md) | Always loaded — known issues and workarounds |
| [frontend.md](../.claude/rules/frontend.md) | Path-scoped: `src/**` — React/UI patterns |
| [backend.md](../.claude/rules/backend.md) | Path-scoped: `src-tauri/**` — Rust/Tauri patterns |

### Technical Specifications

| Document | Description |
|----------|-------------|
| [specs/README.md](../specs/README.md) | Spec lookup table |
| [specs/architecture.md](../specs/architecture.md) | System architecture spec |
| [specs/frontend.md](../specs/frontend.md) | Frontend component spec |
| [specs/database.md](../specs/database.md) | Database schema spec |
| [specs/ipc.md](../specs/ipc.md) | IPC command spec |
| [specs/imap.md](../specs/imap.md) | IMAP sync spec |
| [specs/oauth2.md](../specs/oauth2.md) | OAuth2 flow spec |
| [specs/parser.md](../specs/parser.md) | MIME parser spec |
| [specs/security.md](../specs/security.md) | Security model spec |
| [specs/sync.md](../specs/sync.md) | Sync engine spec |
| [specs/mcp.md](../specs/mcp.md) | MCP server spec |
| [specs/plugins.md](../specs/plugins.md) | Plugin system spec |
| [specs/plan.md](../specs/plan.md) | Implementation plan |

## Contributing

Run `/documenter` after development sessions to keep docs current.
