//! Where an "Open in Claude" handoff lands.
//!
//! Before this table every handoff `cd`ed into its own scratch directory, so
//! Claude arrived with the email and nothing else — no repo, no `CLAUDE.md`, no
//! code. cxtasks has carried a `repo_path` per task since it shipped; this is
//! the mail-shaped version of the same idea, except mail has no field to put a
//! repo in, so the repo is inferred from *who the message involves*.
//!
//! Four scopes, most specific first:
//!
//! | scope | keyed on | answers |
//! |---|---|---|
//! | `contact` | an address or a bare domain | "anything from Northwind is Northwind work" |
//! | `group` | an inbox group | "the Northwind group is Northwind work" |
//! | `account` | one mail account | "everything in this inbox is that product" |
//! | `default` | nothing | "otherwise, land here" |
//!
//! `contact` is the layer that reaches past the inbox: a client writes from
//! their own domain regardless of which of your addresses they reached, and a
//! group rule is a coarser instrument (it also matches on subject text). See
//! [`resolve_for_message`] for the full order and why it is that order.

use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoScope {
    Contact,
    Group,
    Account,
    Default,
}

impl RepoScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoScope::Contact => "contact",
            RepoScope::Group => "group",
            RepoScope::Account => "account",
            RepoScope::Default => "default",
        }
    }

    fn parse(raw: &str) -> Result<Self, AppError> {
        match raw {
            "contact" => Ok(RepoScope::Contact),
            "group" => Ok(RepoScope::Group),
            "account" => Ok(RepoScope::Account),
            "default" => Ok(RepoScope::Default),
            other => Err(AppError::General(format!("unknown repo scope '{other}'"))),
        }
    }
}

/// Which mapping to write or clear.
///
/// An enum rather than four nullable columns on the wire, so the two ways to
/// get this wrong — a contact row carrying a `group_id`, an account row
/// carrying nothing — cannot be spelled at all. The same reasoning as
/// `voice_pinned_rules`' scope/address check, one step earlier: there the
/// contradiction is rejected, here it is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum RepoKey {
    Contact { contact: String },
    Group { group_id: i64 },
    Account { account_id: String },
    Default,
}

impl RepoKey {
    pub fn scope(&self) -> RepoScope {
        match self {
            RepoKey::Contact { .. } => RepoScope::Contact,
            RepoKey::Group { .. } => RepoScope::Group,
            RepoKey::Account { .. } => RepoScope::Account,
            RepoKey::Default => RepoScope::Default,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMapping {
    pub id: i64,
    pub scope: RepoScope,
    pub contact: Option<String>,
    pub group_id: Option<i64>,
    pub account_id: Option<String>,
    pub repo_path: String,
}

/// The mapping that won, plus a short human label for the toast and the seed
/// prompt. `source` is what makes an unexpected landing diagnosable without a
/// log: "Northwind" and "northwind.example" are different answers to "why here?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRepo {
    pub repo_path: String,
    pub scope: RepoScope,
    pub source: String,
}

/// Canonical spelling of a repo path, so two spellings of one directory can't
/// become two rows.
///
/// Expands a leading `~/`, trims, drops trailing slashes — deliberately
/// WITHOUT touching the filesystem, so it stays usable on a path that is merely
/// typed, or on an external drive that is currently unmounted. (`canonicalize`
/// resolves symlinks and fails outright on a missing path.) Same shape as
/// cxtasks' `db::projects::normalize_repo_path`.
pub fn normalize_repo_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let expanded = match trimmed.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => std::path::Path::new(&home)
                .join(rest)
                .to_string_lossy()
                .to_string(),
            None => trimmed.to_string(),
        },
        None => trimmed.to_string(),
    };
    let stripped = expanded.trim_end_matches('/');
    if stripped.is_empty() && expanded.starts_with('/') {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

/// Validate a path on the way in. Absolute only: a relative path would be
/// resolved against whatever directory the app happened to be launched from,
/// which is not a thing the user can see or predict.
///
/// Existence is deliberately NOT required here — a repo on an unmounted volume
/// is still a correct mapping, and the handoff reports a missing directory at
/// the moment it matters instead of refusing to save it.
fn validate_repo_path(raw: &str) -> Result<String, AppError> {
    let path = normalize_repo_path(raw);
    if path.is_empty() {
        return Err(AppError::General("Repo path must not be empty".into()));
    }
    if !path.starts_with('/') {
        return Err(AppError::General(format!(
            "Repo path must be absolute (or start with ~/), got '{path}'"
        )));
    }
    Ok(path)
}

/// Canonical spelling of a contact key: lowercase, `@`-stripped, `mailto:`-stripped.
///
/// Accepts both a full address (`dana@northwind.example`) and a bare domain
/// (`northwind.example`) — the domain form is what makes one row cover a whole
/// client, including people you have not met yet.
pub fn normalize_contact(raw: &str) -> Result<String, AppError> {
    let cleaned = raw
        .trim()
        .trim_start_matches("mailto:")
        .trim()
        .trim_matches(|c: char| c == '<' || c == '>')
        .trim()
        .to_lowercase();
    // A user typing a domain often types the `@` with it.
    let cleaned = cleaned.strip_prefix('@').unwrap_or(&cleaned).to_string();

    if cleaned.is_empty() {
        return Err(AppError::General("Contact must not be empty".into()));
    }
    if cleaned.contains(char::is_whitespace) {
        return Err(AppError::General(format!(
            "Contact must be one address or domain, got '{cleaned}'"
        )));
    }
    match cleaned.split_once('@') {
        Some((local, domain)) => {
            if local.is_empty() || !is_domainish(domain) {
                return Err(AppError::General(format!(
                    "'{cleaned}' is not an email address or a domain"
                )));
            }
        }
        None => {
            if !is_domainish(&cleaned) {
                return Err(AppError::General(format!(
                    "'{cleaned}' is not an email address or a domain \
                     (a domain needs a dot, e.g. northwind.example)"
                )));
            }
        }
    }
    Ok(cleaned)
}

fn is_domainish(s: &str) -> bool {
    s.contains('.') && !s.starts_with('.') && !s.ends_with('.') && !s.contains('@')
}

pub fn list(conn: &Connection) -> Result<Vec<RepoMapping>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, scope, contact, group_id, account_id, repo_path
           FROM claude_repos
          ORDER BY scope, contact, group_id, account_id",
    )?;
    let rows = stmt
        .query_map([], row_to_mapping)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().collect::<Result<Vec<_>, _>>()
}

fn row_to_mapping(row: &rusqlite::Row) -> rusqlite::Result<Result<RepoMapping, AppError>> {
    let scope_raw: String = row.get(1)?;
    Ok(RepoScope::parse(&scope_raw).map(|scope| RepoMapping {
        id: row.get::<_, i64>(0).unwrap_or_default(),
        scope,
        contact: row.get::<_, Option<String>>(2).ok().flatten(),
        group_id: row.get::<_, Option<i64>>(3).ok().flatten(),
        account_id: row.get::<_, Option<String>>(4).ok().flatten(),
        repo_path: row.get::<_, String>(5).unwrap_or_default(),
    }))
}

/// Upsert one mapping. Idempotent per key — the unique indexes make "one repo
/// per contact / group / account, one default" a schema fact rather than a
/// convention this function has to remember.
pub fn set(conn: &Connection, key: &RepoKey, repo_path: &str) -> Result<RepoMapping, AppError> {
    let path = validate_repo_path(repo_path)?;
    match key {
        RepoKey::Contact { contact } => {
            let contact = normalize_contact(contact)?;
            conn.execute(
                "INSERT INTO claude_repos (scope, contact, repo_path)
                 VALUES ('contact', ?1, ?2)
                 ON CONFLICT(contact) WHERE contact IS NOT NULL DO UPDATE
                    SET repo_path = excluded.repo_path, updated_at = datetime('now')",
                params![contact, path],
            )?;
        }
        RepoKey::Group { group_id } => {
            conn.execute(
                "INSERT INTO claude_repos (scope, group_id, repo_path)
                 VALUES ('group', ?1, ?2)
                 ON CONFLICT(group_id) WHERE group_id IS NOT NULL DO UPDATE
                    SET repo_path = excluded.repo_path, updated_at = datetime('now')",
                params![group_id, path],
            )?;
        }
        RepoKey::Account { account_id } => {
            conn.execute(
                "INSERT INTO claude_repos (scope, account_id, repo_path)
                 VALUES ('account', ?1, ?2)
                 ON CONFLICT(account_id) WHERE account_id IS NOT NULL DO UPDATE
                    SET repo_path = excluded.repo_path, updated_at = datetime('now')",
                params![account_id, path],
            )?;
        }
        RepoKey::Default => {
            conn.execute(
                "INSERT INTO claude_repos (scope, repo_path)
                 VALUES ('default', ?1)
                 ON CONFLICT(scope) WHERE scope = 'default' DO UPDATE
                    SET repo_path = excluded.repo_path, updated_at = datetime('now')",
                params![path],
            )?;
        }
    }
    get(conn, key)?.ok_or_else(|| AppError::General("mapping vanished after write".into()))
}

pub fn clear(conn: &Connection, key: &RepoKey) -> Result<(), AppError> {
    match key {
        RepoKey::Contact { contact } => {
            let contact = normalize_contact(contact)?;
            conn.execute(
                "DELETE FROM claude_repos WHERE contact = ?1",
                params![contact],
            )?;
        }
        RepoKey::Group { group_id } => {
            conn.execute(
                "DELETE FROM claude_repos WHERE group_id = ?1",
                params![group_id],
            )?;
        }
        RepoKey::Account { account_id } => {
            conn.execute(
                "DELETE FROM claude_repos WHERE account_id = ?1",
                params![account_id],
            )?;
        }
        RepoKey::Default => {
            conn.execute("DELETE FROM claude_repos WHERE scope = 'default'", [])?;
        }
    }
    Ok(())
}

pub fn get(conn: &Connection, key: &RepoKey) -> Result<Option<RepoMapping>, AppError> {
    let sql = "SELECT id, scope, contact, group_id, account_id, repo_path FROM claude_repos WHERE ";
    let row = match key {
        RepoKey::Contact { contact } => {
            let contact = normalize_contact(contact)?;
            conn.query_row(
                &format!("{sql} contact = ?1"),
                params![contact],
                row_to_mapping,
            )
        }
        RepoKey::Group { group_id } => conn.query_row(
            &format!("{sql} group_id = ?1"),
            params![group_id],
            row_to_mapping,
        ),
        RepoKey::Account { account_id } => conn.query_row(
            &format!("{sql} account_id = ?1"),
            params![account_id],
            row_to_mapping,
        ),
        RepoKey::Default => conn.query_row(&format!("{sql} scope = 'default'"), [], row_to_mapping),
    }
    .optional()?;
    row.transpose()
}

/// Resolve the repo an "Open in Claude" on this message should land in.
///
/// Order, most specific first — and each step is skipped when the thing it
/// names has no repo mapped, so a group you never mapped never shadows the
/// account mapping behind it:
///
/// 1. **contact** — a mapped address or domain among the message's
///    correspondents. Most specific: it names a party, not a container.
/// 2. **group rules** — a group whose *rules* match this message, in the
///    sidebar's own order. Below contact because a group rule may be as loose
///    as `subject contains "Northwind"`.
/// 3. **account** — the mailbox the message arrived in. An explicit choice
///    about one inbox.
/// 4. **group membership** — a group that merely *contains* this account. The
///    weakest group signal: it says nothing about this message.
/// 5. **default**.
///
/// `None` means "no mapping" and the caller keeps the old behaviour — the
/// scratch directory — rather than inventing a directory to land in.
pub fn resolve_for_message(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<ResolvedRepo>, AppError> {
    // 1. contact
    if let Some(resolved) = resolve_contact(conn, account_id, folder_name, uid)? {
        return Ok(Some(resolved));
    }

    // 2. a group whose RULES match this message
    for group_id in
        super::inbox_groups::groups_with_rules_matching(conn, account_id, folder_name, uid)?
    {
        if let Some(mapping) = get(conn, &RepoKey::Group { group_id })? {
            return Ok(Some(ResolvedRepo {
                repo_path: mapping.repo_path,
                scope: RepoScope::Group,
                source: group_label(conn, group_id),
            }));
        }
    }

    // 3. this account
    if let Some(mapping) = get(
        conn,
        &RepoKey::Account {
            account_id: account_id.to_string(),
        },
    )? {
        return Ok(Some(ResolvedRepo {
            repo_path: mapping.repo_path,
            scope: RepoScope::Account,
            source: account_label(conn, account_id),
        }));
    }

    // 4. a group this account belongs to
    for group_id in super::inbox_groups::groups_containing_account(conn, account_id)? {
        if let Some(mapping) = get(conn, &RepoKey::Group { group_id })? {
            return Ok(Some(ResolvedRepo {
                repo_path: mapping.repo_path,
                scope: RepoScope::Group,
                source: group_label(conn, group_id),
            }));
        }
    }

    // 5. default
    Ok(get(conn, &RepoKey::Default)?.map(|mapping| ResolvedRepo {
        repo_path: mapping.repo_path,
        scope: RepoScope::Default,
        source: "default".to_string(),
    }))
}

/// A mapping as a person reads it — "contact northwind.example", "group
/// Northwind", "account me@example.com", "default". The same labels
/// `resolve_for_message` puts in `ResolvedRepo::source`, so the chat's list of
/// readable directories and a resolution's "chosen by" say the same thing.
pub fn describe(conn: &Connection, mapping: &RepoMapping) -> String {
    match mapping.scope {
        RepoScope::Contact => format!("contact {}", mapping.contact.as_deref().unwrap_or("?")),
        RepoScope::Group => format!(
            "group {}",
            mapping.group_id.map(|g| group_label(conn, g)).unwrap_or_default()
        ),
        RepoScope::Account => format!(
            "account {}",
            mapping
                .account_id
                .as_deref()
                .map(|a| account_label(conn, a))
                .unwrap_or_default()
        ),
        RepoScope::Default => "default".to_string(),
    }
}

fn group_label(conn: &Connection, group_id: i64) -> String {
    conn.query_row(
        "SELECT name FROM inbox_groups WHERE id = ?1",
        params![group_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| format!("group {group_id}"))
}

fn account_label(conn: &Connection, account_id: &str) -> String {
    conn.query_row(
        "SELECT email FROM accounts WHERE id = ?1",
        params![account_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| account_id.to_string())
}

fn resolve_contact(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<ResolvedRepo>, AppError> {
    let mut stmt = conn.prepare("SELECT contact, repo_path FROM claude_repos WHERE scope = 'contact'")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Ok(None);
    }

    let candidates = correspondents(conn, account_id, folder_name, uid)?;
    if candidates.is_empty() {
        return Ok(None);
    }

    // Exact address beats domain, whichever correspondent it came from: it is
    // the only key that can name one person inside a mapped company.
    for candidate in &candidates {
        if let Some((contact, path)) = rows.iter().find(|(c, _)| c == candidate) {
            return Ok(Some(ResolvedRepo {
                repo_path: path.clone(),
                scope: RepoScope::Contact,
                source: contact.clone(),
            }));
        }
    }

    // Then the most specific domain: `mail.harborline.example` outranks
    // `harborline.example`, so a bulk-send subdomain can be split off later without
    // disturbing the entry that covers the humans.
    let mut best: Option<(usize, &str, &str)> = None;
    for candidate in &candidates {
        let Some((_, domain)) = candidate.split_once('@') else {
            continue;
        };
        for (contact, path) in &rows {
            if contact.contains('@') {
                continue;
            }
            let matches = domain == contact || domain.ends_with(&format!(".{contact}"));
            if matches && best.map_or(true, |(len, _, _)| contact.len() > len) {
                best = Some((contact.len(), contact, path));
            }
        }
    }
    Ok(best.map(|(_, contact, path)| ResolvedRepo {
        repo_path: path.to_string(),
        scope: RepoScope::Contact,
        source: contact.to_string(),
    }))
}

/// Every address on the message that is not one of the user's own, sender
/// first.
///
/// Both directions on purpose: a received message names the client in `From`, a
/// message in Sent names them in `To`, and "Open in Claude" is reachable from
/// either. The user's own addresses are excluded because they appear on both
/// sides of their own mail and would otherwise match every message — to key on
/// one of your own addresses, use the `account` scope, which is what it is for.
fn correspondents(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<String>, AppError> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT from_email, to_list, cc_list FROM messages
              WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            params![account_id, folder_name, uid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((from_email, to_list, cc_list)) = row else {
        return Ok(vec![]);
    };

    let own: Vec<String> = {
        let mut stmt = conn.prepare("SELECT lower(email) FROM accounts")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        rows
    };

    let mut out: Vec<String> = Vec::new();
    let push = |addr: &str, out: &mut Vec<String>| {
        let addr = addr.trim().to_lowercase();
        if addr.is_empty() || !addr.contains('@') || own.contains(&addr) || out.contains(&addr) {
            return;
        }
        out.push(addr);
    };

    if let Some(from) = from_email.as_deref() {
        push(from, &mut out);
    }
    for list in [to_list.as_deref(), cc_list.as_deref()].into_iter().flatten() {
        for addr in addresses_in_json_list(list) {
            push(&addr, &mut out);
        }
    }
    Ok(out)
}

/// `to_list` / `cc_list` hold `[{"name":…,"email":…}]`. Malformed JSON yields
/// nothing rather than an error: a mapping that silently does not apply is a
/// far smaller problem than a handoff that refuses to open.
fn addresses_in_json_list(raw: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Entry {
        email: Option<String>,
    }
    serde_json::from_str::<Vec<Entry>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.email)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL);
             CREATE TABLE inbox_groups (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT NOT NULL DEFAULT '#0a84ff',
                icon TEXT NOT NULL DEFAULT 'folder',
                sort_order INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE inbox_group_rules (
                id INTEGER PRIMARY KEY,
                group_id INTEGER NOT NULL REFERENCES inbox_groups(id) ON DELETE CASCADE,
                field TEXT NOT NULL, operator TEXT NOT NULL, value TEXT NOT NULL
             );
             CREATE TABLE inbox_group_accounts (
                group_id INTEGER NOT NULL REFERENCES inbox_groups(id) ON DELETE CASCADE,
                account_id TEXT NOT NULL,
                PRIMARY KEY (group_id, account_id)
             );
             CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                subject TEXT, from_name TEXT, from_email TEXT, to_list TEXT, cc_list TEXT,
                date TEXT NOT NULL DEFAULT '2026-01-01', is_read INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account_id, folder_name, uid)
             );",
        )
        .unwrap();
        crate::db::schema::create_claude_repos_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email) VALUES ('acct-cxv', 'chris@cxventures.io')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_message(conn: &Connection, uid: u32, from: &str, subject: &str) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_email, to_list, cc_list)
             VALUES ('acct-cxv', 'INBOX', ?1, ?2, ?3, '[{\"name\":null,\"email\":\"chris@cxventures.io\"}]', '[]')",
            params![uid, subject, from],
        )
        .unwrap();
    }

    #[test]
    fn contact_beats_group_beats_account_beats_default() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO inbox_groups (id, name, sort_order) VALUES (1, 'Northwind', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO inbox_group_rules (group_id, field, operator, value)
             VALUES (1, 'subject', 'contains', 'Northwind')",
            [],
        )
        .unwrap();

        set(&conn, &RepoKey::Default, "/tmp/default").unwrap();
        set(
            &conn,
            &RepoKey::Account {
                account_id: "acct-cxv".into(),
            },
            "/tmp/account",
        )
        .unwrap();
        set(&conn, &RepoKey::Group { group_id: 1 }, "/tmp/group").unwrap();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "northwind.example".into(),
            },
            "/tmp/contact",
        )
        .unwrap();

        // Everything applies -> contact wins.
        insert_message(&conn, 1, "dana@northwind.example", "Northwind weekly");
        let r = resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
            .unwrap()
            .unwrap();
        assert_eq!(r.repo_path, "/tmp/contact");
        assert_eq!(r.scope, RepoScope::Contact);

        // No mapped contact -> the group rule.
        insert_message(&conn, 2, "someone@else.com", "Northwind weekly");
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 2)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/group"
        );

        // Neither -> the account.
        insert_message(&conn, 3, "someone@else.com", "unrelated");
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 3)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/account"
        );

        // Account mapping gone -> the default.
        clear(
            &conn,
            &RepoKey::Account {
                account_id: "acct-cxv".into(),
            },
        )
        .unwrap();
        let r = resolve_for_message(&conn, "acct-cxv", "INBOX", 3)
            .unwrap()
            .unwrap();
        assert_eq!(r.repo_path, "/tmp/default");
        assert_eq!(r.scope, RepoScope::Default);
    }

    /// Nothing mapped must stay "no repo", never a guess — the caller's
    /// fallback is the scratch dir, which is where handoffs landed before this
    /// table existed.
    #[test]
    fn an_empty_table_resolves_to_nothing() {
        let conn = test_db();
        insert_message(&conn, 1, "dana@northwind.example", "Northwind weekly");
        assert!(resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
            .unwrap()
            .is_none());
    }

    /// A group that exists but has no repo must not shadow the account mapping
    /// behind it. This is the bug that turns "I mapped my inbox" into "Open in
    /// Claude does nothing different".
    #[test]
    fn an_unmapped_group_does_not_shadow_the_account() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO inbox_groups (id, name, sort_order) VALUES (1, 'Northwind', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO inbox_group_rules (group_id, field, operator, value)
             VALUES (1, 'subject', 'contains', 'Northwind')",
            [],
        )
        .unwrap();
        set(
            &conn,
            &RepoKey::Account {
                account_id: "acct-cxv".into(),
            },
            "/tmp/account",
        )
        .unwrap();
        insert_message(&conn, 1, "someone@else.com", "Northwind weekly");
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/account"
        );
    }

    /// A subdomain must fall to the parent domain's mapping, and an explicit
    /// subdomain row must win over it. `mail.harborline.example` (bulk send) and
    /// `harborline.example` (the people) are the real pair.
    #[test]
    fn the_most_specific_domain_wins_and_subdomains_fall_through() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "harborline.example".into(),
            },
            "/tmp/impact",
        )
        .unwrap();
        insert_message(&conn, 1, "community@mail.harborline.example", "hi");
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/impact",
            "a subdomain should inherit the parent domain's repo"
        );

        set(
            &conn,
            &RepoKey::Contact {
                contact: "mail.harborline.example".into(),
            },
            "/tmp/impact-bulk",
        )
        .unwrap();
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/impact-bulk",
            "the longer, more specific domain must win"
        );
    }

    /// A domain must not match a *prefix* of another domain. `harborline.example`
    /// mapped must not swallow `notharborline.example`, and — the case that motivated
    /// the whole check — `bluestonepresents.example` (the concert promoter) must not be
    /// caught by a mapping for Bluestone Advisors Group.
    #[test]
    fn a_domain_matches_on_a_label_boundary_not_a_suffix_of_text() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "harborline.example".into(),
            },
            "/tmp/impact",
        )
        .unwrap();
        insert_message(&conn, 1, "hi@notharborline.example", "hi");
        assert!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
                .unwrap()
                .is_none(),
            "notharborline.example is a different company"
        );
    }

    /// The user's own addresses appear on both sides of their own mail, so a
    /// contact row keyed on one would match everything. Own addresses are
    /// excluded; that is what the `account` scope is for.
    #[test]
    fn your_own_address_is_never_a_correspondent() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "cxventures.io".into(),
            },
            "/tmp/contact",
        )
        .unwrap();
        // Addressed TO the user's own account address, from a stranger.
        insert_message(&conn, 1, "stranger@example.com", "hello");
        assert!(resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
            .unwrap()
            .is_none());
    }

    /// Sent mail names the client in `To`, not `From` — the handoff has to work
    /// from either side or it silently does nothing in the Sent folder.
    #[test]
    fn a_recipient_matches_too_so_sent_mail_lands_in_the_same_repo() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "northwind.example".into(),
            },
            "/tmp/northwind",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_email, to_list, cc_list)
             VALUES ('acct-cxv', 'Sent', 9, 'Re: scope', 'chris@cxventures.io',
                     '[{\"name\":\"Dana\",\"email\":\"dana@northwind.example\"}]', '[]')",
            [],
        )
        .unwrap();
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "Sent", 9)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/northwind"
        );
    }

    #[test]
    fn an_exact_address_outranks_its_own_domain() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "northwind.example".into(),
            },
            "/tmp/northwind",
        )
        .unwrap();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "dana@northwind.example".into(),
            },
            "/tmp/dana",
        )
        .unwrap();
        insert_message(&conn, 1, "dana@northwind.example", "hi");
        assert_eq!(
            resolve_for_message(&conn, "acct-cxv", "INBOX", 1)
                .unwrap()
                .unwrap()
                .repo_path,
            "/tmp/dana"
        );
    }

    /// `inbox_groups.id` is a plain INTEGER PRIMARY KEY, so SQLite hands a
    /// deleted group's id to the next group created. Without the cascade the
    /// stale mapping would silently reattach to an unrelated group — mail
    /// landing in a repo nobody pointed it at, with nothing to read that says
    /// why.
    #[test]
    fn deleting_a_group_deletes_its_mapping_because_ids_get_reused() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO inbox_groups (id, name, sort_order) VALUES (1, 'Northwind', 0)",
            [],
        )
        .unwrap();
        set(&conn, &RepoKey::Group { group_id: 1 }, "/tmp/northwind").unwrap();
        conn.execute("DELETE FROM inbox_groups WHERE id = 1", []).unwrap();
        assert!(
            get(&conn, &RepoKey::Group { group_id: 1 }).unwrap().is_none(),
            "the mapping outlived the group it was keyed on"
        );
    }

    /// The wire contract with `src/lib/tauri.ts`'s `ClaudeRepoKey`. The panel
    /// sends these four shapes verbatim; a serde attribute changed here would
    /// break every write at runtime with a deserialization error the type
    /// checker cannot see, since the two type systems never meet.
    #[test]
    fn the_key_deserializes_from_exactly_what_the_frontend_sends() {
        let cases = [
            (
                r#"{"scope":"contact","contact":"northwind.example"}"#,
                RepoKey::Contact {
                    contact: "northwind.example".into(),
                },
            ),
            (r#"{"scope":"group","group_id":4}"#, RepoKey::Group { group_id: 4 }),
            (
                r#"{"scope":"account","account_id":"acct-cxv"}"#,
                RepoKey::Account {
                    account_id: "acct-cxv".into(),
                },
            ),
            (r#"{"scope":"default"}"#, RepoKey::Default),
        ];
        for (json, expected) in cases {
            let parsed: RepoKey =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} did not parse: {e}"));
            assert_eq!(parsed, expected);
        }

        // And the shapes the frontend's union makes unspellable stay rejected
        // here too, rather than quietly becoming a row with no key.
        assert!(serde_json::from_str::<RepoKey>(r#"{"scope":"contact"}"#).is_err());
        assert!(serde_json::from_str::<RepoKey>(r#"{"scope":"nonsense"}"#).is_err());
    }

    /// `ResolvedRepo` and the scope enum are read by the toast, so their JSON
    /// spelling is a contract too — `snake_case` scopes, snake_case fields.
    #[test]
    fn the_resolved_repo_serializes_the_way_the_toast_reads_it() {
        let json = serde_json::to_string(&ResolvedRepo {
            repo_path: "/Users/x/clients/bluestone".into(),
            scope: RepoScope::Contact,
            source: "bluestoneadvisors.example".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"repo_path":"/Users/x/clients/bluestone","scope":"contact","source":"bluestoneadvisors.example"}"#
        );
    }

    #[test]
    fn paths_are_normalized_and_relative_ones_refused() {
        let conn = test_db();
        assert_eq!(normalize_repo_path("  /tmp/x/  "), "/tmp/x");
        assert!(set(&conn, &RepoKey::Default, "Projects/cxventures").is_err());
        assert!(set(&conn, &RepoKey::Default, "   ").is_err());
        let saved = set(&conn, &RepoKey::Default, "/tmp/ok/").unwrap();
        assert_eq!(saved.repo_path, "/tmp/ok");
    }

    #[test]
    fn contacts_are_normalized_and_nonsense_refused() {
        assert_eq!(normalize_contact("  @Northwind.example ").unwrap(), "northwind.example");
        assert_eq!(
            normalize_contact("mailto:Dana@Northwind.example").unwrap(),
            "dana@northwind.example"
        );
        assert_eq!(normalize_contact("<sam@harborline.example>").unwrap(), "sam@harborline.example");
        assert!(normalize_contact("localhost").is_err(), "a domain needs a dot");
        assert!(normalize_contact("two words").is_err());
        assert!(normalize_contact("@example.com extra").is_err());
        assert!(normalize_contact("").is_err());
    }

    /// One repo per key. Saving twice must move the mapping, not accumulate a
    /// second row that shadows the first depending on row order.
    #[test]
    fn writing_the_same_key_twice_updates_in_place() {
        let conn = test_db();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "northwind.example".into(),
            },
            "/tmp/one",
        )
        .unwrap();
        set(
            &conn,
            &RepoKey::Contact {
                contact: "Northwind.example".into(),
            },
            "/tmp/two",
        )
        .unwrap();
        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 1, "case-different spellings are one contact");
        assert_eq!(all[0].repo_path, "/tmp/two");
    }
}
