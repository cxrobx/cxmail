import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  Folder,
  FolderOpen,
  Loader2,
  Plus,
  Terminal,
  X,
} from "lucide-react";
import { api, type ClaudeRepo, type ClaudeRepoKey } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { cn } from "@/lib/utils";

interface Props {
  onClose: () => void;
}

/** Stable string form of a key, for React keys and per-row busy state. */
function keyId(key: ClaudeRepoKey): string {
  switch (key.scope) {
    case "contact":
      return `contact:${key.contact.toLowerCase()}`;
    case "group":
      return `group:${key.group_id}`;
    case "account":
      return `account:${key.account_id}`;
    case "default":
      return "default";
  }
}

function repoKeyOf(repo: ClaudeRepo): ClaudeRepoKey | null {
  if (repo.scope === "contact" && repo.contact) return { scope: "contact", contact: repo.contact };
  if (repo.scope === "group" && repo.group_id !== null)
    return { scope: "group", group_id: repo.group_id };
  if (repo.scope === "account" && repo.account_id)
    return { scope: "account", account_id: repo.account_id };
  if (repo.scope === "default") return { scope: "default" };
  return null;
}

/**
 * Where "Open in Claude" lands, per correspondent / group / account.
 *
 * The panel mirrors the resolution order top to bottom — contacts first,
 * default last — so the list reads as the precedence it actually implements.
 * Every write goes through `set_claude_repo`, which is where path and contact
 * validation lives; this component never decides what a legal path is.
 */
export default function ClaudeRepoSettings({ onClose }: Props) {
  const accounts = useAccountStore((s) => s.accounts);
  const inboxGroups = useMailStore((s) => s.inboxGroups);
  const [repos, setRepos] = useState<ClaudeRepo[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newContact, setNewContact] = useState("");
  const [newContactPath, setNewContactPath] = useState("");

  const reload = useCallback(async () => {
    try {
      setRepos(await api.claude.listRepos());
    } catch (e) {
      setError(String(e));
      setRepos([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const byKey = useMemo(() => {
    const map = new Map<string, ClaudeRepo>();
    for (const repo of repos ?? []) {
      const key = repoKeyOf(repo);
      if (key) map.set(keyId(key), repo);
    }
    return map;
  }, [repos]);

  const write = async (key: ClaudeRepoKey, path: string) => {
    const id = keyId(key);
    setBusy(id);
    setError(null);
    try {
      if (path.trim() === "") {
        await api.claude.clearRepo(key);
      } else {
        await api.claude.setRepo(key, path);
      }
      await reload();
      setSaved(id);
      window.setTimeout(() => setSaved((cur) => (cur === id ? null : cur)), 1500);
      return true;
    } catch (e) {
      // The message names the reason (relative path, malformed contact) — show
      // it verbatim rather than a generic failure.
      setError(String(e).replace(/^Error:\s*/, ""));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const addContact = async () => {
    const contact = newContact.trim();
    if (!contact || !newContactPath.trim()) return;
    if (await write({ scope: "contact", contact }, newContactPath)) {
      setNewContact("");
      setNewContactPath("");
    }
  };

  if (repos === null) {
    return (
      <Shell onClose={onClose}>
        <div className="flex items-center gap-2 p-6 text-sm text-content-muted">
          <Loader2 className="h-4 w-4 animate-spin" /> Loading repo mappings…
        </div>
      </Shell>
    );
  }

  const contacts = repos.filter((r) => r.scope === "contact");

  return (
    <Shell onClose={onClose}>
      <div className="flex-1 overflow-auto px-4 py-3">
        <p className="mb-4 text-xs leading-relaxed text-content-muted">
          "Open in Claude" starts a session in the repo an email belongs to, so Claude arrives
          with that project's <code className="text-content-secondary">CLAUDE.md</code> and code.
          The first match wins, top to bottom; anything unmatched opens in a scratch folder with
          just the email.
        </p>

        <Section
          title="Correspondents"
          hint="An address, or a bare domain to cover everyone there. Matches the sender and the recipients, so mail you sent lands in the same repo."
        >
          {contacts.map((repo) => (
            <Row
              key={repo.contact}
              label={repo.contact ?? ""}
              mono
              repo={repo}
              busy={busy === keyId({ scope: "contact", contact: repo.contact ?? "" })}
              saved={saved === keyId({ scope: "contact", contact: repo.contact ?? "" })}
              onWrite={(path) => write({ scope: "contact", contact: repo.contact ?? "" }, path)}
            />
          ))}
          <div className="mt-1 flex items-center gap-2">
            <input
              value={newContact}
              onChange={(e) => setNewContact(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addContact()}
              aria-label="New correspondent address or domain"
              placeholder="northwind.example"
              className="w-[190px] shrink-0 rounded bg-input px-2 py-1 font-mono text-xs text-content outline-none ring-1 ring-border focus:ring-accent"
            />
            <PathField
              value={newContactPath}
              onChange={setNewContactPath}
              onCommit={addContact}
              placeholder="~/clients/…"
              ariaLabel="Repo path for new correspondent"
            />
            <button
              onClick={addContact}
              disabled={!newContact.trim() || !newContactPath.trim()}
              title="Add correspondent mapping"
              className="rounded p-1.5 text-content-muted hover:bg-surface hover:text-content disabled:opacity-40"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </div>
        </Section>

        {inboxGroups.length > 0 && (
          <Section title="Groups" hint="Applies when the group's rules match the message.">
            {inboxGroups.map((group) => {
              const key: ClaudeRepoKey = { scope: "group", group_id: group.id };
              return (
                <Row
                  key={group.id}
                  label={group.name}
                  swatch={group.color}
                  repo={byKey.get(keyId(key))}
                  busy={busy === keyId(key)}
                  saved={saved === keyId(key)}
                  onWrite={(path) => write(key, path)}
                />
              );
            })}
          </Section>
        )}

        <Section title="Accounts" hint="Everything that arrives in this mailbox.">
          {accounts.map((account) => {
            const key: ClaudeRepoKey = { scope: "account", account_id: account.id };
            return (
              <Row
                key={account.id}
                label={account.email}
                swatch={account.color ?? undefined}
                repo={byKey.get(keyId(key))}
                busy={busy === keyId(key)}
                saved={saved === keyId(key)}
                onWrite={(path) => write(key, path)}
              />
            );
          })}
        </Section>

        <Section title="Everything else" hint="Leave empty to open unmatched mail in a scratch folder.">
          <Row
            label="Default"
            repo={byKey.get("default")}
            busy={busy === "default"}
            saved={saved === "default"}
            onWrite={(path) => write({ scope: "default" }, path)}
          />
        </Section>
      </div>

      {error && (
        <div className="flex shrink-0 items-start gap-2 border-t border-border-subtle bg-surface px-4 py-2 text-xs text-error">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="break-words">{error}</span>
        </div>
      )}
    </Shell>
  );
}

function Shell({ children, onClose }: { children: React.ReactNode; onClose: () => void }) {
  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-overlay"
      onClick={onClose}
    >
      <div
        className="flex max-h-[90vh] w-[640px] flex-col overflow-hidden rounded-xl border border-border bg-base-solid shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex shrink-0 items-center justify-between border-b border-border-subtle bg-surface px-4 py-3">
          <div className="flex items-center gap-2">
            <Terminal className="h-4 w-4 text-content-secondary" />
            <h2 className="text-sm font-medium text-content">Open in Claude — repos</h2>
          </div>
          <button
            onClick={onClose}
            aria-label="Close repo settings"
            className="text-content-muted hover:text-content"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-5">
      <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wider text-content-muted">
        {title}
      </div>
      <div className="mb-2 text-[11px] leading-snug text-content-muted">{hint}</div>
      <div className="space-y-1">{children}</div>
    </div>
  );
}

function Row({
  label,
  mono,
  swatch,
  repo,
  busy,
  saved,
  onWrite,
}: {
  label: string;
  mono?: boolean;
  swatch?: string;
  repo?: ClaudeRepo;
  busy: boolean;
  saved: boolean;
  onWrite: (path: string) => void | Promise<unknown>;
}) {
  const stored = repo?.repo_path ?? "";
  const [value, setValue] = useState(stored);

  // Re-seed only when this row's stored value actually changed — i.e. after a
  // write or a reload that touched it. A reload triggered by editing a
  // different row leaves `stored` here identical, so uncommitted text in this
  // field survives instead of being yanked back.
  useEffect(() => {
    setValue(stored);
  }, [stored]);

  const commit = () => {
    if (value.trim() === stored) return;
    void onWrite(value);
  };

  return (
    <div className="flex items-center gap-2">
      <div className="flex w-[190px] shrink-0 items-center gap-1.5 overflow-hidden">
        {swatch ? (
          <Folder className="h-3.5 w-3.5 shrink-0" style={{ color: swatch }} />
        ) : (
          <span className="w-3.5 shrink-0" />
        )}
        <span
          title={label}
          className={cn(
            "truncate text-xs text-content-secondary",
            mono && "font-mono",
          )}
        >
          {label}
        </span>
      </div>
      <PathField
        value={value}
        onChange={setValue}
        onCommit={commit}
        placeholder="~/Projects/…"
        ariaLabel={`Repo path for ${label}`}
        invalid={repo !== undefined && !repo.exists}
      />
      <div className="flex w-[22px] shrink-0 justify-center">
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-content-muted" />
        ) : saved ? (
          <Check className="h-3.5 w-3.5 text-success" />
        ) : repo && !repo.exists ? (
          <AlertTriangle
            className="h-3.5 w-3.5 text-warning"
            aria-label="Directory not found"
          >
            <title>This directory isn't there right now</title>
          </AlertTriangle>
        ) : null}
      </div>
    </div>
  );
}

function PathField({
  value,
  onChange,
  onCommit,
  placeholder,
  ariaLabel,
  invalid,
}: {
  value: string;
  onChange: (v: string) => void;
  onCommit: () => void;
  placeholder: string;
  ariaLabel: string;
  invalid?: boolean;
}) {
  const pick = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = (await open({ directory: true })) as string | null;
    if (picked) {
      onChange(picked);
      // The picker is a commit in itself — the user chose a real directory, so
      // there is nothing left to confirm by blurring the field.
      void Promise.resolve().then(onCommit);
    }
  };

  return (
    <div className="relative flex-1">
      <input
        value={value}
        aria-label={ariaLabel}
        // A repo path is longer than the field; hovering beats scrolling it.
        title={value || undefined}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onCommit}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
        }}
        placeholder={placeholder}
        spellCheck={false}
        className={cn(
          "w-full rounded bg-input py-1 pl-2 pr-7 font-mono text-xs text-content outline-none ring-1 focus:ring-accent",
          invalid ? "ring-warning/60" : "ring-border",
        )}
      />
      <button
        onClick={pick}
        title="Choose a folder"
        className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-content-muted hover:text-content"
      >
        <FolderOpen className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
