//! FTS5-backed full-text search — the single search engine shared by the
//! Tauri UI commands and the standalone MCP binary. The `messages_fts` table
//! is maintained transactionally by triggers (schema.rs v40), so this module
//! only ever reads.

use crate::error::AppError;
use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Private-use markers wrapped around matched spans in `SearchResult.snippet`.
/// The frontend splits on these and renders `<mark>` elements.
pub const HIGHLIGHT_START: char = '\u{E000}';
pub const HIGHLIGHT_END: char = '\u{E001}';

/// All-optional, AND-composed search filters. `keywords` goes through
/// `build_match_expr` (never interpolated raw); everything else binds as a
/// SQL parameter against the `messages` metadata columns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchFilters {
    pub keywords: Option<String>,
    pub from: Option<String>,
    /// Matches to_list OR cc_list.
    pub to: Option<String>,
    /// Matches cc_list only (the `cc:` operator).
    pub cc: Option<String>,
    pub subject_contains: Option<String>,
    /// Emitted as an FTS column filter on `attachment_names`.
    pub filename: Option<String>,
    /// Inclusive; date or datetime, compared via datetime() (gotcha #23).
    pub date_after: Option<String>,
    /// Compared as datetime(m.date) <= datetime(?). A bare YYYY-MM-DD value
    /// normalizes to midnight, i.e. effectively exclusive of that day —
    /// callers wanting an inclusive end append T23:59:59 themselves.
    pub date_before: Option<String>,
    /// Matches folder_name exactly OR any folder of that folder_type
    /// (so "sent" finds "[Gmail]/Sent Mail" and "Sent Messages" alike).
    pub folder: Option<String>,
    pub has_attachments: Option<bool>,
    pub is_starred: Option<bool>,
    /// Some(true) → unread only; Some(false) → read only.
    pub is_unread: Option<bool>,
    pub larger_bytes: Option<i64>,
    pub smaller_bytes: Option<i64>,
    pub account_ids: Option<Vec<String>>,
}

impl SearchFilters {
    pub fn has_any_filter(&self) -> bool {
        self.from.is_some()
            || self.to.is_some()
            || self.cc.is_some()
            || self.subject_contains.is_some()
            || self.filename.is_some()
            || self.date_after.is_some()
            || self.date_before.is_some()
            || self.folder.is_some()
            || self.has_attachments.is_some()
            || self.is_starred.is_some()
            || self.is_unread.is_some()
            || self.larger_bytes.is_some()
            || self.smaller_bytes.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub account_id: String,
    pub folder_name: String,
    pub uid: u32,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
    /// Contains \u{E000}/\u{E001} highlight markers around matched spans.
    pub snippet: String,
    pub is_read: bool,
    pub has_attachments: bool,
    pub score: f32,
}

/// Build a lenient, injection-proof FTS5 MATCH expression from raw user text.
///
/// - `"quoted phrases"` become FTS5 phrase queries (internal `"` doubled).
/// - Every bare token is double-quoted, neutralizing `AND`, `OR`, `NEAR`,
///   `:`, `(`, `*`, `^` and any other syntax.
/// - A leading `-` negates a token/phrase (`a NOT b`); a `-` anywhere else is
///   quoted literally. If ALL tokens are negated there is no left operand for
///   FTS5's binary NOT, so negations are dropped and `None` is returned.
/// - `prefix == true` appends `*` to the last positive token (as-you-type).
pub fn build_match_expr(input: &str, prefix: bool) -> Option<String> {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut i = 0;
    // (text, negated)
    let mut tokens: Vec<(String, bool)> = Vec::new();

    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let mut negated = false;
        if chars[i] == '-' && i + 1 < n && !chars[i + 1].is_whitespace() {
            negated = true;
            i += 1;
        }
        if i < n && chars[i] == '"' {
            // Phrase: read to the closing quote, or end of input if unbalanced.
            i += 1;
            let start = i;
            while i < n && chars[i] != '"' {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            if i < n {
                i += 1;
            }
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                tokens.push((trimmed.to_string(), negated));
            }
        } else {
            let start = i;
            while i < n && !chars[i].is_whitespace() {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            // A bare "-" (negation with nothing behind it after quote-strip)
            // or pure-punctuation tokens still get quoted — FTS5 treats a
            // phrase with no tokens as a no-op term, which errors; filter
            // tokens that contain no alphanumeric content at all.
            if raw.chars().any(|c| c.is_alphanumeric()) {
                tokens.push((raw, negated));
            }
        }
    }

    let positives: Vec<&(String, bool)> = tokens.iter().filter(|(_, neg)| !neg).collect();
    let negatives: Vec<&(String, bool)> = tokens.iter().filter(|(_, neg)| *neg).collect();
    if positives.is_empty() {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    for (idx, (text, _)) in positives.iter().enumerate() {
        let mut term = format!("\"{}\"", text.replace('"', "\"\""));
        if prefix && idx == positives.len() - 1 {
            term.push('*');
        }
        parts.push(term);
    }
    let mut expr = parts.join(" AND ");
    for (text, _) in negatives {
        expr.push_str(&format!(" NOT \"{}\"", text.replace('"', "\"\"")));
    }
    Some(expr)
}

/// One AND-composed search over FTS5 + message metadata.
///
/// With keywords (or a filename filter): FTS MATCH joined to `messages`.
/// Results are ALWAYS newest first. A BM25 score (subject 8, sender 4,
/// recipients 2, body 1, attachment_names 3, plus a 0.02/day recency
/// penalty; smaller is better) still rides along as `rank`, but it is
/// data, never the sort key — a list that is not chronological reads as
/// a bug, and the score was quietly interleaving Jul/Sep/Aug hits.
///
/// Without keywords: pure metadata query over `messages`, newest first.
/// No keywords AND no filters returns [] (never the whole mailbox).
pub fn search(
    conn: &Connection,
    f: &SearchFilters,
    limit: u32,
    offset: u32,
    prefix: bool,
) -> Result<Vec<SearchResult>, AppError> {
    if let Some(ids) = &f.account_ids {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
    }

    let keywords_expr = f
        .keywords
        .as_deref()
        .and_then(|k| build_match_expr(k, prefix));
    let match_expr: Option<String> = {
        let fname = f.filename.as_ref().map(|x| {
            format!("{{attachment_names}}: \"{}\"*", x.replace('"', "\"\""))
        });
        match (fname, keywords_expr) {
            (Some(a), Some(b)) => Some(format!("{} AND ({})", a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    };

    if match_expr.is_none() && !f.has_any_filter() {
        return Ok(Vec::new());
    }

    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx: usize = 1;

    if let Some(ref expr) = match_expr {
        conditions.push(format!("messages_fts MATCH ?{idx}"));
        params.push(Box::new(expr.clone()));
        idx += 1;
    }
    if let Some(ref from) = f.from {
        conditions.push(format!(
            "(m.from_email LIKE ?{idx} OR m.from_name LIKE ?{idx})"
        ));
        params.push(Box::new(format!("%{from}%")));
        idx += 1;
    }
    if let Some(ref to) = f.to {
        conditions.push(format!(
            "(m.to_list LIKE ?{idx} OR m.cc_list LIKE ?{idx})"
        ));
        params.push(Box::new(format!("%{to}%")));
        idx += 1;
    }
    if let Some(ref cc) = f.cc {
        conditions.push(format!("m.cc_list LIKE ?{idx}"));
        params.push(Box::new(format!("%{cc}%")));
        idx += 1;
    }
    if let Some(ref subject) = f.subject_contains {
        conditions.push(format!("m.subject LIKE ?{idx}"));
        params.push(Box::new(format!("%{subject}%")));
        idx += 1;
    }
    // Gotcha #23: wrap BOTH operands in datetime() — the stored ISO format
    // ("...T...Z") compares lexicographically wrong against SQLite's
    // space-separated datetime('now') form.
    if let Some(ref after) = f.date_after {
        conditions.push(format!("datetime(m.date) >= datetime(?{idx})"));
        params.push(Box::new(after.clone()));
        idx += 1;
    }
    if let Some(ref before) = f.date_before {
        conditions.push(format!("datetime(m.date) <= datetime(?{idx})"));
        params.push(Box::new(before.clone()));
        idx += 1;
    }
    if let Some(ref folder) = f.folder {
        conditions.push(format!(
            "(m.folder_name = ?{idx} OR m.folder_name IN (
                SELECT f.name FROM folders f
                WHERE f.account_id = m.account_id AND f.folder_type = lower(?{idx})))"
        ));
        params.push(Box::new(folder.clone()));
        idx += 1;
    }
    if f.has_attachments == Some(true) {
        conditions.push("m.has_attachments = 1".to_string());
    }
    if f.is_starred == Some(true) {
        conditions.push("m.is_flagged = 1".to_string());
    }
    match f.is_unread {
        Some(true) => conditions.push("m.is_read = 0".to_string()),
        Some(false) => conditions.push("m.is_read = 1".to_string()),
        None => {}
    }
    if let Some(larger) = f.larger_bytes {
        conditions.push(format!("m.size_bytes > ?{idx}"));
        params.push(Box::new(larger));
        idx += 1;
    }
    if let Some(smaller) = f.smaller_bytes {
        conditions.push(format!("m.size_bytes < ?{idx}"));
        params.push(Box::new(smaller));
        idx += 1;
    }
    match f.account_ids {
        Some(ref ids) => {
            let placeholders: Vec<String> = (0..ids.len())
                .map(|i| format!("?{}", idx + i))
                .collect();
            conditions.push(format!("m.account_id IN ({})", placeholders.join(", ")));
            for id in ids {
                params.push(Box::new(id.clone()));
            }
            idx += ids.len();
        }
        // Unscoped = "all accounts" = all VISIBLE accounts. This is the one
        // place the hidden rule is `None`-only: `search` serves explicit
        // contexts too (the search bar inside the hidden account, an MCP call
        // naming it), and naming an account is the one way its mail is
        // reachable by search. Explicit ids are honoured exactly.
        None => conditions.push(super::accounts::visible_in_aggregates_sql("m.account_id")),
    }

    // Some(false) flag filters add no condition — keep the WHERE valid.
    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };
    let sql = if match_expr.is_some() {
        // julianday('') is NULL → coalesce to the 1970 epoch julian day so
        // garbage dates rank as very old instead of NULL-sorting to the top.
        format!(
            "SELECT m.account_id, m.folder_name, m.uid, m.subject, m.from_name, m.from_email,
                    m.date, m.is_read, m.has_attachments,
                    snippet(messages_fts, -1, '{hs}', '{he}', ' … ', 12) AS snip,
                    (bm25(messages_fts, 8.0, 4.0, 2.0, 1.0, 3.0)
                      + (julianday('now') - coalesce(julianday(m.date), 2440587.5)) * 0.02) AS rank
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
             WHERE {where_clause}
             ORDER BY datetime(m.date) DESC, rank ASC
             LIMIT ?{idx} OFFSET ?{off}",
            hs = HIGHLIGHT_START,
            he = HIGHLIGHT_END,
            off = idx + 1,
        )
    } else {
        format!(
            "SELECT m.account_id, m.folder_name, m.uid, m.subject, m.from_name, m.from_email,
                    m.date, m.is_read, m.has_attachments,
                    coalesce(m.snippet, '') AS snip,
                    0.0 AS rank
             FROM messages m
             WHERE {where_clause}
             ORDER BY datetime(m.date) DESC
             LIMIT ?{idx} OFFSET ?{off}",
            off = idx + 1,
        )
    };
    params.push(Box::new(limit));
    params.push(Box::new(offset));

    let mut stmt = conn.prepare(&sql)?;
    let bound: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(bound.as_slice(), |row| {
            Ok(SearchResult {
                account_id: row.get(0)?,
                folder_name: row.get(1)?,
                uid: row.get::<_, i64>(2)? as u32,
                subject: row.get(3)?,
                from_name: row.get(4)?,
                from_email: row.get(5)?,
                date: row.get(6)?,
                is_read: row.get::<_, i32>(7)? != 0,
                has_attachments: row.get::<_, i32>(8)? != 0,
                snippet: clean_snippet_display(
                    &row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                ),
                score: row.get::<_, f64>(10)? as f32,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Post-process a snippet for display. Marketing ESPs generate the text/plain
/// part by crudely stripping their HTML, leaking preheader padding as literal
/// entity text (`&#847;` = U+034F combining grapheme joiner, repeated hundreds
/// of times) — the FTS path serves `plain_text` verbatim, so without this the
/// snippet shows the raw entities. Decode entities, drop the invisible
/// padding characters (also present pre-decoded in the metadata path's
/// `m.snippet`), and collapse the whitespace runs they leave behind. The
/// U+E000/U+E001 highlight markers are private-use, non-whitespace, and never
/// produced by entity decoding of sane input, so they pass through intact.
fn clean_snippet_display(raw: &str) -> String {
    // FTS5's snippet() window ends at a TOKEN boundary, and the tokenizer
    // sees `847` as the token inside `&#847;` — so the window can cut the
    // trailing `;` off the last entity, leaving `&#847` that a strict
    // decoder ignores. Decode numeric references with an optional
    // terminator first (HTML5 parsers accept semicolonless numeric refs,
    // so this matches what a browser would render).
    static NUMERIC_REF: OnceLock<Regex> = OnceLock::new();
    let re = NUMERIC_REF.get_or_init(|| {
        Regex::new(r"&#(?:x([0-9a-fA-F]{1,6})|([0-9]{1,7}));?").unwrap()
    });
    let raw = re.replace_all(raw, |caps: &regex::Captures| {
        let value = caps
            .get(1)
            .map(|hex| u32::from_str_radix(hex.as_str(), 16))
            .unwrap_or_else(|| caps[2].parse());
        match value.ok().and_then(char::from_u32) {
            Some(c) => c.to_string(),
            None => caps[0].to_string(), // invalid scalar — leave as-is
        }
    });
    let decoded = html_escape::decode_html_entities(raw.as_ref());
    let stripped: String = decoded
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{00AD}'                // soft hyphen
                | '\u{034F}'              // combining grapheme joiner
                | '\u{200B}'..='\u{200D}' // zero-width space / non-joiner / joiner
                | '\u{2060}'              // word joiner
                | '\u{FEFF}'              // zero-width no-break space (BOM)
            )
        })
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rebuild `messages_fts` from scratch: clear it, then re-insert every message
/// joined with its cached body and attachment filenames. Used by the v40
/// migration and the startup integrity net. Plain rusqlite::Result so the
/// migration runner can call it directly.
pub fn rebuild(conn: &Connection) -> rusqlite::Result<i64> {
    conn.execute("DELETE FROM messages_fts", [])?;
    let inserted = conn.execute(
        "INSERT INTO messages_fts(rowid, subject, sender, recipients, body, attachment_names)
         SELECT m.id, coalesce(m.subject,''),
                trim(coalesce(m.from_name,'')||' '||coalesce(m.from_email,'')),
                trim(coalesce(m.to_list,'')||' '||coalesce(m.cc_list,'')),
                coalesce(b.plain_text,''),
                coalesce((SELECT group_concat(a.filename, ' ') FROM attachments a
                          WHERE a.account_id=m.account_id AND a.folder_name=m.folder_name AND a.message_uid=m.uid), '')
         FROM messages m
         LEFT JOIN message_bodies b ON m.account_id=b.account_id AND m.folder_name=b.folder_name AND m.uid=b.uid",
        [],
    )?;
    Ok(inserted as i64)
}

/// Startup safety net: rebuild the FTS table when the row counts drift in
/// EITHER direction (ghosts or missing docs) or the FTS table errors
/// (corruption / pre-v40 DB opened by a newer build). Never panics.
pub fn verify_or_rebuild(conn: &Connection) {
    let msg_count: i64 = match conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("search: cannot count messages, skipping FTS check: {e}");
            return;
        }
    };
    let fts_count: rusqlite::Result<i64> =
        conn.query_row("SELECT count(*) FROM messages_fts", [], |r| r.get(0));
    match fts_count {
        Ok(fc) if fc == msg_count => {
            log::info!("search: FTS index consistent ({msg_count} messages)");
        }
        other => {
            log::warn!(
                "search: FTS index out of sync (messages={msg_count}, fts={other:?}), rebuilding"
            );
            match rebuild(conn) {
                Ok(n) => log::info!("search: FTS rebuild indexed {n} messages"),
                Err(e) => log::error!("search: FTS rebuild failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Minimal schema with every column the search SQL and the v40 triggers
    /// touch, then the real v40 migration on top.
    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                subject TEXT, from_name TEXT, from_email TEXT, to_list TEXT, cc_list TEXT,
                date TEXT NOT NULL DEFAULT '', snippet TEXT,
                is_read INTEGER NOT NULL DEFAULT 0, is_flagged INTEGER NOT NULL DEFAULT 0,
                has_attachments INTEGER NOT NULL DEFAULT 0, size_bytes INTEGER DEFAULT 0,
                UNIQUE(account_id, folder_name, uid));
             CREATE TABLE message_bodies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                plain_text TEXT, html_body TEXT, sanitized_html TEXT,
                attachment_metadata_checked INTEGER NOT NULL DEFAULT 0,
                cid_resolved INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account_id, folder_name, uid),
                FOREIGN KEY (account_id, folder_name, uid)
                    REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE);
             CREATE TABLE attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, message_uid INTEGER NOT NULL,
                filename TEXT, content_type TEXT, size_bytes INTEGER, content_id TEXT,
                is_inline INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (account_id, folder_name, message_uid)
                    REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE);
             CREATE TABLE folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL, name TEXT NOT NULL, folder_type TEXT,
                UNIQUE(account_id, name));
             -- An unscoped search embeds the hidden-account predicate, which
             -- reads this table; without it every call dies with
             -- `no such table: accounts`. Empty = every account visible.
             CREATE TABLE accounts (
                id TEXT PRIMARY KEY, hidden_from_aggregates INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        crate::db::schema::migrate_v40_fts5(&conn).unwrap();
        conn
    }

    fn insert_msg(
        conn: &Connection,
        uid: u32,
        subject: &str,
        from_name: &str,
        from_email: &str,
        date: &str,
    ) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_name, from_email, date)
             VALUES ('acct', 'INBOX', ?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![uid, subject, from_name, from_email, date],
        )
        .unwrap();
    }

    fn kw(keywords: &str) -> SearchFilters {
        SearchFilters {
            keywords: Some(keywords.to_string()),
            ..Default::default()
        }
    }

    // ── build_match_expr escaping table ──────────────────────────────

    #[test]
    fn match_expr_quotes_bare_tokens() {
        assert_eq!(build_match_expr("budget", false).unwrap(), "\"budget\"");
        assert_eq!(
            build_match_expr("budget report", false).unwrap(),
            "\"budget\" AND \"report\""
        );
    }

    #[test]
    fn match_expr_neutralizes_fts_syntax() {
        // Unbalanced paren, operators, colons — all quoted literals.
        assert_eq!(build_match_expr("budget (", false).unwrap(), "\"budget\"");
        assert_eq!(
            build_match_expr("a AND b", false).unwrap(),
            "\"a\" AND \"AND\" AND \"b\""
        );
        assert_eq!(build_match_expr("from:", false).unwrap(), "\"from:\"");
        assert_eq!(build_match_expr("NEAR", false).unwrap(), "\"NEAR\"");
        assert_eq!(build_match_expr("x^2*", false).unwrap(), "\"x^2*\"");
    }

    #[test]
    fn match_expr_preserves_phrases_and_escapes_quotes() {
        assert_eq!(
            build_match_expr("\"quarterly budget\" review", false).unwrap(),
            "\"quarterly budget\" AND \"review\""
        );
        // Unbalanced trailing quote → phrase runs to end of input.
        assert_eq!(
            build_match_expr("\"open phrase", false).unwrap(),
            "\"open phrase\""
        );
        // Interior quote in a bare token is doubled.
        assert_eq!(build_match_expr("ab\"cd", false).unwrap(), "\"ab\"\"cd\"");
    }

    #[test]
    fn match_expr_negation() {
        assert_eq!(
            build_match_expr("budget -promo", false).unwrap(),
            "\"budget\" NOT \"promo\""
        );
        assert_eq!(
            build_match_expr("budget -\"special offer\"", false).unwrap(),
            "\"budget\" NOT \"special offer\""
        );
        // All-negated → no left operand for FTS5 NOT → None.
        assert!(build_match_expr("-promo", false).is_none());
        // Hyphen inside a token is not negation.
        assert_eq!(build_match_expr("e-mail", false).unwrap(), "\"e-mail\"");
    }

    #[test]
    fn match_expr_prefix_mode_stars_last_positive() {
        assert_eq!(build_match_expr("budg", true).unwrap(), "\"budg\"*");
        assert_eq!(
            build_match_expr("q3 budg", true).unwrap(),
            "\"q3\" AND \"budg\"*"
        );
        // Trailing whitespace: star still lands on the last real token.
        assert_eq!(build_match_expr("budg ", true).unwrap(), "\"budg\"*");
        // Negation last: star goes to the last POSITIVE token.
        assert_eq!(
            build_match_expr("budg -promo", true).unwrap(),
            "\"budg\"* NOT \"promo\""
        );
    }

    #[test]
    fn match_expr_unicode_and_empty() {
        assert_eq!(build_match_expr("café", false).unwrap(), "\"café\"");
        assert_eq!(build_match_expr("日本語 メール", false).unwrap(), "\"日本語\" AND \"メール\"");
        assert!(build_match_expr("", false).is_none());
        assert!(build_match_expr("   ", false).is_none());
        assert!(build_match_expr("(((", false).is_none());
    }

    // ── trigger + search behavior ────────────────────────────────────

    #[test]
    fn insert_message_is_immediately_searchable() {
        let conn = test_conn();
        insert_msg(&conn, 1, "Quarterly budget review", "Sarah", "sarah@x.com", "2026-06-01T10:00:00Z");
        let hits = search(&conn, &kw("budget"), 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].uid, 1);
        assert_eq!(hits[0].date, "2026-06-01T10:00:00Z");
        assert!(hits[0].snippet.contains(HIGHLIGHT_START));
        assert!(hits[0].snippet.contains(HIGHLIGHT_END));
    }

    #[test]
    fn keyword_search_is_newest_first_even_when_an_older_hit_scores_higher() {
        let conn = test_conn();
        // Dates are relative to now so the 0.02/day recency term stays tiny
        // and BM25 alone decides the score: the OLDER rows match in the
        // subject (weight 8, repeated), the newest only in the sender
        // (weight 4). Score order would put uid 2 last — date wins, always.
        let day = |n: i64| {
            (chrono::Utc::now() - chrono::Duration::days(n))
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        };
        // Filler that does NOT match, so the term has IDF weight — in a
        // corpus where every row matches, bm25 is flat and the test proves
        // nothing (that is how the first version of this test passed
        // against the old ORDER BY).
        for i in 10..30u32 {
            insert_msg(&conn, i, "weekly digest", "Someone", "s@y.org", &day(5));
        }
        insert_msg(&conn, 1, "simplefin simplefin simplefin renewal", "Bridge", "info@x.org", &day(3));
        insert_msg(&conn, 2, "Transaction data accessed", "SimpleFIN", "info@x.org", &day(1));
        insert_msg(&conn, 3, "simplefin simplefin receipt", "Bridge", "info@x.org", &day(2));
        let hits = search(&conn, &kw("simplefin"), 50, 0, false).unwrap();
        let uids: Vec<u32> = hits.iter().map(|h| h.uid).collect();
        assert_eq!(uids, vec![2, 3, 1], "search results must be newest first, not score order");
    }

    #[test]
    fn snippet_display_strips_entity_padding_and_invisibles() {
        // FTS path: literal entities straight from a crude HTML-strip.
        assert_eq!(
            clean_snippet_display("lululemon &#847; &#847;&#847; &#847; new arrivals"),
            "lululemon new arrivals"
        );
        // Metadata path: the same padding already decoded to U+034F chars.
        assert_eq!(
            clean_snippet_display("lululemon \u{34f} \u{34f}\u{34f} sale"),
            "lululemon sale"
        );
        // Ordinary entities decode; highlight markers survive untouched.
        assert_eq!(
            clean_snippet_display("Tom &amp; \u{e000}Jerry\u{e001} \u{200b}\u{feff}show"),
            "Tom & \u{e000}Jerry\u{e001} show"
        );
        // FTS5's snippet window ends at a token boundary, cutting the `;`
        // off the last entity — the semicolonless form must decode too.
        assert_eq!(
            clean_snippet_display("lululemon &#847; &#847 …"),
            "lululemon …"
        );
        // Hex form, and an invalid scalar left untouched.
        assert_eq!(clean_snippet_display("&#x34F;x&#x34f"), "x");
        assert_eq!(clean_snippet_display("&#1114112;"), "&#1114112;");
    }

    #[test]
    fn search_snippet_has_no_entity_padding() {
        let conn = test_conn();
        insert_msg(&conn, 1, "Comfort, at your fingertips", "lululemon", "hello@e.lululemon.com", "2026-07-20T10:00:00Z");
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text)
             VALUES ('acct', 'INBOX', 1, 'lululemon
 &#847; &#847;&#847; &#847;&#847; &#847; &#847; &#847;&#847; new alignment gear inside')",
            [],
        )
        .unwrap();
        let hits = search(&conn, &kw("lululemon"), 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].snippet.contains("&#"), "snippet leaked entities: {}", hits[0].snippet);
        assert!(!hits[0].snippet.contains('\u{34f}'));
        assert!(!hits[0].snippet.contains("  "), "whitespace not collapsed: {:?}", hits[0].snippet);
        assert!(hits[0].snippet.contains(HIGHLIGHT_START));
    }

    #[test]
    fn insert_or_replace_body_yields_one_searchable_row() {
        let conn = test_conn();
        insert_msg(&conn, 1, "hello", "A", "a@x.com", "2026-06-01T10:00:00Z");
        // Twice, like insert_body's INSERT OR REPLACE on a cache refresh.
        for text in ["the zanzibar report first pass", "the zanzibar report second pass"] {
            conn.execute(
                "INSERT OR REPLACE INTO message_bodies (account_id, folder_name, uid, plain_text)
                 VALUES ('acct', 'INBOX', 1, ?1)",
                [text],
            )
            .unwrap();
        }
        let hits = search(&conn, &kw("zanzibar"), 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1, "exactly one FTS row after OR REPLACE");
        let fts_rows: i64 = conn
            .query_row("SELECT count(*) FROM messages_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_rows, 1);
        // The latest body won.
        let hits = search(&conn, &kw("second"), 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn deleting_message_row_evicts_fts_ghost() {
        // Regression: move/archive/delete_messages issue raw DELETEs — the
        // Tantivy engine never evicted these (ghost results forever).
        let conn = test_conn();
        insert_msg(&conn, 1, "ghost hunt", "A", "a@x.com", "2026-06-01T10:00:00Z");
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text)
             VALUES ('acct', 'INBOX', 1, 'spectral body text')",
            [],
        )
        .unwrap();
        assert_eq!(search(&conn, &kw("ghost"), 50, 0, false).unwrap().len(), 1);

        conn.execute(
            "DELETE FROM messages WHERE account_id = 'acct' AND folder_name = 'INBOX' AND uid = 1",
            [],
        )
        .unwrap();

        assert!(search(&conn, &kw("ghost"), 50, 0, false).unwrap().is_empty());
        assert!(search(&conn, &kw("spectral"), 50, 0, false).unwrap().is_empty());
        let fts_rows: i64 = conn
            .query_row("SELECT count(*) FROM messages_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_rows, 0);
    }

    #[test]
    fn subject_update_reindexes_and_flag_flips_do_not_error() {
        let conn = test_conn();
        insert_msg(&conn, 1, "old title", "A", "a@x.com", "2026-06-01T10:00:00Z");
        conn.execute("UPDATE messages SET subject = 'fresh headline' WHERE uid = 1", [])
            .unwrap();
        assert!(search(&conn, &kw("old"), 50, 0, false).unwrap().is_empty());
        assert_eq!(search(&conn, &kw("headline"), 50, 0, false).unwrap().len(), 1);

        // Scoped UPDATE trigger: read-state flips leave FTS untouched.
        conn.execute("UPDATE messages SET is_read = 1 WHERE uid = 1", [])
            .unwrap();
        assert_eq!(search(&conn, &kw("headline"), 50, 0, false).unwrap().len(), 1);
    }

    #[test]
    fn attachment_filenames_are_searchable_and_evicted() {
        let conn = test_conn();
        insert_msg(&conn, 1, "invoice attached", "A", "a@x.com", "2026-06-01T10:00:00Z");
        conn.execute(
            "INSERT INTO attachments (account_id, folder_name, message_uid, filename)
             VALUES ('acct', 'INBOX', 1, 'q3_财务_report.pdf')",
            [],
        )
        .unwrap();
        // Bare keyword hits the filename column.
        assert_eq!(search(&conn, &kw("report"), 50, 0, false).unwrap().len(), 1);
        // filename: filter hits it too.
        let f = SearchFilters {
            filename: Some("report".to_string()),
            ..Default::default()
        };
        assert_eq!(search(&conn, &f, 50, 0, false).unwrap().len(), 1);
        // filename: filter does NOT match subject-only terms.
        let f2 = SearchFilters {
            filename: Some("invoice".to_string()),
            ..Default::default()
        };
        assert!(search(&conn, &f2, 50, 0, false).unwrap().is_empty());

        conn.execute("DELETE FROM attachments WHERE message_uid = 1", [])
            .unwrap();
        assert!(search(&conn, &kw("report"), 50, 0, false).unwrap().is_empty());
    }

    #[test]
    fn filters_and_compose_not_union() {
        // Regression for the union bug: "from sarah" + "budget" must return
        // only the intersection, not every budget email + every Sarah email.
        let conn = test_conn();
        insert_msg(&conn, 1, "budget planning", "Sarah Lee", "sarah@x.com", "2026-06-01T10:00:00Z");
        insert_msg(&conn, 2, "budget planning", "Bob", "bob@x.com", "2026-06-02T10:00:00Z");
        insert_msg(&conn, 3, "lunch tomorrow", "Sarah Lee", "sarah@x.com", "2026-06-03T10:00:00Z");

        let f = SearchFilters {
            keywords: Some("budget".to_string()),
            from: Some("sarah".to_string()),
            ..Default::default()
        };
        let hits = search(&conn, &f, 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].uid, 1);
    }

    #[test]
    fn prefix_mode_matches_partial_token() {
        let conn = test_conn();
        insert_msg(&conn, 1, "budget review", "A", "a@x.com", "2026-06-01T10:00:00Z");
        assert!(search(&conn, &kw("budg"), 50, 0, false).unwrap().is_empty());
        assert_eq!(search(&conn, &kw("budg"), 50, 0, true).unwrap().len(), 1);
    }

    #[test]
    fn same_day_date_filter_fires_per_gotcha_23() {
        // Testing trap from gotcha #23: the fixture must share TODAY's UTC
        // date — a 2020/2999 fixture passes even with broken comparisons.
        let conn = test_conn();
        let today_start: String = conn
            .query_row("SELECT date('now') || 'T00:00:00.001Z'", [], |r| r.get(0))
            .unwrap();
        insert_msg(&conn, 1, "today mail", "A", "a@x.com", &today_start);
        let f = SearchFilters {
            keywords: Some("today".to_string()),
            date_after: Some(
                conn.query_row("SELECT date('now')", [], |r| r.get(0))
                    .unwrap(),
            ),
            ..Default::default()
        };
        assert_eq!(search(&conn, &f, 50, 0, false).unwrap().len(), 1);
        // And date_before of yesterday excludes it.
        let f2 = SearchFilters {
            keywords: Some("today".to_string()),
            date_before: Some(
                conn.query_row("SELECT date('now', '-1 day')", [], |r| r.get(0))
                    .unwrap(),
            ),
            ..Default::default()
        };
        assert!(search(&conn, &f2, 50, 0, false).unwrap().is_empty());
    }

    #[test]
    fn metadata_only_search_without_keywords() {
        let conn = test_conn();
        insert_msg(&conn, 1, "starred one", "A", "a@x.com", "2026-06-01T10:00:00Z");
        conn.execute("UPDATE messages SET is_flagged = 1 WHERE uid = 1", [])
            .unwrap();
        insert_msg(&conn, 2, "plain one", "A", "a@x.com", "2026-06-02T10:00:00Z");

        let f = SearchFilters {
            is_starred: Some(true),
            ..Default::default()
        };
        let hits = search(&conn, &f, 50, 0, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].uid, 1);

        // No keywords, no filters → empty, never the whole mailbox.
        assert!(search(&conn, &SearchFilters::default(), 50, 0, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn folder_filter_matches_name_or_folder_type() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, date)
             VALUES ('acct', '[Gmail]/Sent Mail', 9, 'sent budget note', '2026-06-01T10:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id, name, folder_type) VALUES ('acct', '[Gmail]/Sent Mail', 'sent')",
            [],
        )
        .unwrap();
        let f = SearchFilters {
            keywords: Some("budget".to_string()),
            folder: Some("sent".to_string()),
            ..Default::default()
        };
        assert_eq!(search(&conn, &f, 50, 0, false).unwrap().len(), 1);
    }

    /// `None` means every VISIBLE account; naming the hidden account is the
    /// one way its mail is reachable. Both the FTS path (keywords) and the
    /// metadata-only path (no keywords) carry the rule.
    #[test]
    fn unscoped_search_skips_a_hidden_account_but_naming_it_finds_its_mail() {
        let conn = test_conn();
        conn.execute_batch(
            "INSERT INTO accounts (id, hidden_from_aggregates) VALUES ('acct', 0), ('acc-h', 1);",
        )
        .unwrap();
        insert_msg(&conn, 1, "budget review", "A", "a@x.com", "2026-06-01T10:00:00Z");
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_name, from_email, date)
             VALUES ('acc-h', 'INBOX', 1, 'budget warmup', 'H', 'h@x.com', '2026-06-02T10:00:00Z')",
            [],
        )
        .unwrap();

        let unscoped = SearchFilters { keywords: Some("budget".into()), ..Default::default() };
        let accounts: Vec<String> = search(&conn, &unscoped, 50, 0, false)
            .unwrap()
            .into_iter()
            .map(|r| r.account_id)
            .collect();
        assert_eq!(accounts, vec!["acct"]);

        let named = SearchFilters {
            keywords: Some("budget".into()),
            account_ids: Some(vec!["acc-h".into()]),
            ..Default::default()
        };
        let accounts: Vec<String> = search(&conn, &named, 50, 0, false)
            .unwrap()
            .into_iter()
            .map(|r| r.account_id)
            .collect();
        assert_eq!(accounts, vec!["acc-h"]);

        // Metadata-only path: same rule.
        let meta = SearchFilters { from: Some("x.com".into()), ..Default::default() };
        let accounts: Vec<String> = search(&conn, &meta, 50, 0, false)
            .unwrap()
            .into_iter()
            .map(|r| r.account_id)
            .collect();
        assert_eq!(accounts, vec!["acct"]);
    }

    #[test]
    fn empty_account_ids_returns_empty() {
        let conn = test_conn();
        insert_msg(&conn, 1, "budget", "A", "a@x.com", "2026-06-01T10:00:00Z");
        let f = SearchFilters {
            keywords: Some("budget".to_string()),
            account_ids: Some(vec![]),
            ..Default::default()
        };
        assert!(search(&conn, &f, 50, 0, false).unwrap().is_empty());
    }

    #[test]
    fn rebuild_repairs_ghosts_and_missing_rows() {
        let conn = test_conn();
        insert_msg(&conn, 1, "alpha", "A", "a@x.com", "2026-06-01T10:00:00Z");
        // Simulate drift: an orphan FTS row + a missing FTS row.
        conn.execute(
            "INSERT INTO messages_fts(rowid, subject, sender, recipients, body, attachment_names)
             VALUES (999, 'orphan', '', '', '', '')",
            [],
        )
        .unwrap();
        let n = rebuild(&conn).unwrap();
        assert_eq!(n, 1);
        let fts_rows: i64 = conn
            .query_row("SELECT count(*) FROM messages_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_rows, 1);
        assert_eq!(search(&conn, &kw("alpha"), 50, 0, false).unwrap().len(), 1);
        assert!(search(&conn, &kw("orphan"), 50, 0, false).unwrap().is_empty());
    }

    #[test]
    fn v40_migration_is_idempotent() {
        let conn = test_conn();
        insert_msg(&conn, 1, "alpha", "A", "a@x.com", "2026-06-01T10:00:00Z");
        // Run the migration again on a live DB (e.g. version row lost).
        crate::db::schema::migrate_v40_fts5(&conn).unwrap();
        crate::db::schema::migrate_v40_fts5(&conn).unwrap();
        let fts_rows: i64 = conn
            .query_row("SELECT count(*) FROM messages_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_rows, 1, "no duplicate FTS rows after re-runs");
        assert_eq!(search(&conn, &kw("alpha"), 50, 0, false).unwrap().len(), 1);
    }

    #[test]
    fn porter_stemming_matches_variants() {
        let conn = test_conn();
        insert_msg(&conn, 1, "running the projections", "A", "a@x.com", "2026-06-01T10:00:00Z");
        assert_eq!(search(&conn, &kw("run"), 50, 0, false).unwrap().len(), 1);
        assert_eq!(search(&conn, &kw("projection"), 50, 0, false).unwrap().len(), 1);
    }
}
