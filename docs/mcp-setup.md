# CXMail MCP Server — Setup Guide

CXMail ships a built-in [Model Context Protocol](https://modelcontextprotocol.io) server
(`cxmail-mcp`) so Claude can search, read, draft, and send email on your behalf. This is the
authoritative setup guide — follow it top to bottom.

> **The MCP is not standalone.** It shares the desktop app's SQLite database and credential store
> (`bin/mcp.rs:22-28` resolves `~/Library/Application Support/com.cxmail.app/cxmail.db`). You **must**
> run the CXMail app and add at least one account first, or every tool returns empty / no-account
> results. This is the single most common "it doesn't work" cause — see Step 1.

## Prerequisites

| Tool | Notes |
|------|-------|
| Rust toolchain | Install via [rustup](https://rustup.rs). Source it before building: `source "$HOME/.cargo/env"` |
| Node.js | For building/running the desktop app (`npm`) |
| Claude Code CLI | The MCP client that talks to the server over stdio |

## Step 1 — Set up the app first (required)

The MCP reads the same database and credentials the desktop app writes. Bring those up before
touching the MCP:

```bash
source "$HOME/.cargo/env"
npm install
npm run tauri dev          # or: npm run tauri build, then launch the app
```

Then **in the app's GUI**, add at least one email account (Gmail / iCloud / Outlook) and let it
sync. This creates `~/Library/Application Support/com.cxmail.app/cxmail.db` and stores credentials in
the encrypted file store (dev) or macOS Keychain (production).

*Why this matters:* `cxmail-mcp` opens that exact DB path on startup (`src-tauri/src/bin/mcp.rs:22-28`)
and relies on the app-managed credentials for IMAP/SMTP. With no app run and no accounts added, the
DB is empty (or absent) and tools like `list_accounts` / `search_emails` return nothing.

## Step 2 — Build the MCP binary

The binary is not committed (`target/` is gitignored) — build it from source:

```bash
cd src-tauri && source "$HOME/.cargo/env" && cargo build --release --bin cxmail-mcp
```

This produces `src-tauri/target/release/cxmail-mcp` — an argless stdio server.

## Step 3 — Register the server

Pick one of the two paths. Both point Claude at the same binary; you don't need both.

### (a) Project `.mcp.json` (committed — easiest)

The repo root ships a `.mcp.json` that registers `cxmail` with a relative command path:

```json
{
  "mcpServers": {
    "cxmail": {
      "command": "./src-tauri/target/release/cxmail-mcp"
    }
  }
}
```

Open the repo in Claude Code from the project root; Claude prompts you to approve the `cxmail`
project MCP server. Approve it. No `args` — the binary takes none.

> If you already have a global `cxmail` entry in `~/.mcp.json` pointing at the same binary, the
> project entry harmlessly duplicates it (same server, same binary).

### (b) Manual / global registration

If you prefer a global registration (works from any directory), use an **absolute** path:

```bash
claude mcp add cxmail /absolute/path/to/cxmail/src-tauri/target/release/cxmail-mcp
```

No `args`. This writes the entry to your user-level MCP config.

## Step 4 — Verify

```bash
claude mcp list            # should list: cxmail
```

Then, in a Claude Code session, ask Claude to call `list_accounts` and `search_emails`. You should
see your real accounts and matching mail.

**Empty results?** That almost always means Step 1 isn't done — the app hasn't run or no account is
added yet. Go back and add an account through the GUI.

## Troubleshooting

| Symptom | Cause / fix |
|---------|-------------|
| `Failed to connect` | Binary not built or wrong path. Build it (Step 2); confirm `src-tauri/target/release/cxmail-mcp` exists and the registered command path matches. |
| Tools return empty / no accounts | App-first dependency not met (Step 1). Run the app, add an account, let it sync. |
| Tempted to add `--stdio` (or any flag) | Don't. The binary is **argless** — flags make it fail to start. |
| MCP stopped working after `cargo clean` | `target/` is gitignored and `clean` wipes the binary. Rebuild immediately (Step 2). See the build-cache landmine note in the root `CLAUDE.md`. |

## What the server is

A dedicated `cxmail-mcp` binary, separate from the desktop app, speaking MCP over **stdio** with **no
flags**. It shares the app's SQLite database and credential store. Full tool list and permission
tiers are in [`specs/mcp.md`](../specs/mcp.md).
