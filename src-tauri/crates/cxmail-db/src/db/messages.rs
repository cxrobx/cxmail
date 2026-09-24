use cxmail_core::mail::message_id::compute_thread_root_id;
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;

/// Validate a frontend-supplied category against the fixed enum so we can
/// safely interpolate it into SQL. Anything else is rejected — IPC commands
/// must never trust raw frontend strings inside SQL fragments.
fn validated_category(cat: &str) -> Result<&str, AppError> {
    match cat {
        "primary" | "updates" | "social" | "promotions" | "junk" => Ok(cat),
        other => Err(AppError::General(format!("Invalid category: {}", other))),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageRow {
    pub uid: u32,
    pub account_id: Option<String>,
    pub folder_name: Option<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub has_attachments: bool,
    pub size_bytes: i64,
    pub category: Option<String>,
    pub is_muted: bool,
    pub is_pinned: bool,
    pub thread_count: u32,
    /// How many UNSENT DRAFTS this thread holds, deduplicated the same way
    /// `thread_count` is.
    ///
    /// Separate from `thread_count` because the two answer different
    /// questions and one number cannot do both. `thread_count` is what the
    /// list row's badge prints (`thread_count + 1` = messages actually
    /// exchanged), so counting a draft there claims a message was sent that
    /// never was — the bug this field exists to fix. But the reading pane
    /// gates thread loading on the same number, so simply excluding drafts
    /// would take "one message plus my unfinished reply" to zero and the
    /// draft would never be rendered at all. The gate reads this; the badge
    /// does not.
    pub thread_draft_count: u32,
    pub thread_root_id: Option<String>,
    pub thread_has_unread: bool,
}

/// Thread message with message_id included (for MCP threading).
#[derive(Debug, Clone, Serialize)]
pub struct ThreadMessageRow {
    pub uid: u32,
    /// UIDs are folder-scoped, so `folder_name` + `uid` is the smallest
    /// addressable coordinate pair. Surfaced so an MCP agent reading a thread
    /// can hand `reply_to_folder` / `reply_to_uid` straight to `compose_draft`.
    pub folder_name: String,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
    /// `serde_json::to_string(&Vec<EmailAddress>)` — surfaced so an MCP agent
    /// reading a thread sees who each message went to (recipient integrity).
    pub to_list: Option<String>,
    pub cc_list: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageBodyRow {
    pub plain_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub attachment_metadata_checked: bool,
    pub cid_resolved: bool,
}

pub fn insert_batch(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    #[allow(clippy::type_complexity)] messages: &[(
        u32,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        &str,
        Option<&str>,
        &str,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        bool,
        bool,
        bool,
        i64,
        Option<&str>,
        Option<&str>,
        &str,
        &str,
    )],
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO messages (account_id, folder_name, uid, subject, from_name, from_email, date, snippet, flags, message_id, in_reply_to, reference_ids, is_read, is_flagged, has_attachments, size_bytes, list_unsubscribe, list_unsubscribe_post, thread_root_id, to_list, cc_list)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        )?;
        for (
            uid,
            subject,
            from_name,
            from_email,
            date,
            snippet,
            flags,
            message_id,
            in_reply_to,
            references,
            is_read,
            is_flagged,
            has_attachments,
            size_bytes,
            list_unsubscribe,
            list_unsubscribe_post,
            to_list,
            cc_list,
        ) in messages
        {
            let thread_root =
                compute_thread_root_id(*message_id, *in_reply_to, *references, folder_name, *uid);
            stmt.execute(params![
                account_id,
                folder_name,
                uid,
                subject,
                from_name,
                from_email,
                date,
                snippet,
                flags,
                message_id,
                in_reply_to,
                references,
                *is_read as i32,
                *is_flagged as i32,
                *has_attachments as i32,
                size_bytes,
                list_unsubscribe,
                list_unsubscribe_post,
                thread_root,
                to_list,
                cc_list,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn get_unsubscribe_headers(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<(Option<String>, Option<String>), AppError> {
    let mut stmt = conn.prepare(
        "SELECT list_unsubscribe, list_unsubscribe_post FROM messages
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let result = stmt.query_row(params![account_id, folder_name, uid], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    });
    match result {
        Ok(headers) => Ok(headers),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
        Err(e) => Err(AppError::Database(e)),
    }
}

pub fn get_by_uid(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<MessageRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT uid, subject, from_name, from_email, date, snippet, is_read, is_flagged, has_attachments, size_bytes, category, is_muted, is_pinned, thread_root_id
         FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let mut rows = stmt.query_map(params![account_id, folder_name, uid], |row| {
        Ok(MessageRow {
            uid: row.get(0)?,
            account_id: Some(account_id.to_string()),
            folder_name: Some(folder_name.to_string()),
            subject: row.get(1)?,
            from_name: row.get(2)?,
            from_email: row.get(3)?,
            date: row.get(4)?,
            snippet: row.get(5)?,
            is_read: row.get::<_, i32>(6)? != 0,
            is_flagged: row.get::<_, i32>(7)? != 0,
            has_attachments: row.get::<_, i32>(8)? != 0,
            size_bytes: row.get(9)?,
            category: row.get(10)?,
            is_muted: row.get::<_, i32>(11)? != 0,
            is_pinned: row.get::<_, i32>(12)? != 0,
            // thread_count stored column dropped in v37; single-message fetches
            // don't surface it. List views recompute it via thread_root_id.
            thread_count: 0,
            thread_draft_count: 0,
            thread_root_id: row.get(13)?,
            thread_has_unread: false,
        })
    })?;
    match rows.next() {
        Some(Ok(row)) => Ok(Some(row)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

/// The `aggregates` CTE's counting expressions, in ONE place (gotcha #36)
/// because three list queries — this module's folder list and All Inboxes,
/// plus `inbox_groups` — have to agree about what "N messages in this thread"
/// means. They used to spell `COUNT(*)` each, which counted a thread's rows
/// rather than its messages, and so was wrong twice over:
///
///  * **Unsent drafts counted as messages.** A reply you started and never
///    sent made a two-message thread read as three.
///  * **Gmail's `[Gmail]/All Mail` mirror counted again.** Every sent message
///    is also a row there, so a thread with two of your replies in it counted
///    them twice. `COUNT(DISTINCT …)` on the message's own identity collapses
///    the copies; the `folder:uid` fallback keeps two genuinely
///    id-less messages apart.
///
/// `draft_exempt_folder_expr` names the folder the list is scoped to, when
/// that folder can itself be a drafts folder. Browsing Drafts, the rows on
/// screen ARE drafts, and excluding them would report a count that does not
/// include the message being looked at. All Inboxes and the inbox groups are
/// `INBOX`-scoped and pass `None`.
///
/// `has_unread_any` takes the same exclusion, which fixes a second-order bug:
/// a draft saved unread (6 of them in the real mailbox) left its whole thread
/// bolded as if someone had written to you.
pub(super) fn thread_aggregate_columns(draft_exempt_folder_expr: Option<&str>) -> String {
    let key = "COALESCE(NULLIF(m.message_id, ''), 'uid:' || m.folder_name || ':' || m.uid)";
    let is_draft = super::folders::is_draft_sql("m.account_id", "m.folder_name");
    let counts_as_message = match draft_exempt_folder_expr {
        Some(expr) => format!("(NOT {is_draft} OR m.folder_name = {expr})"),
        None => format!("(NOT {is_draft})"),
    };
    format!(
        "COUNT(DISTINCT CASE WHEN {counts_as_message} THEN {key} END) AS total_count,
                   COUNT(DISTINCT CASE WHEN NOT {counts_as_message} THEN {key} END) AS draft_count,
                   MAX(CASE WHEN m.is_read = 0 AND {counts_as_message} THEN 1 ELSE 0 END) AS has_unread_any"
    )
}

pub fn list_by_folder(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    page: u32,
    page_size: u32,
    category: Option<&str>,
    unread_only: bool,
) -> Result<(Vec<MessageRow>, u32), AppError> {
    let mute_filter = if folder_name == "INBOX" {
        " AND m.is_muted = 0"
    } else {
        ""
    };
    let unread_filter = if unread_only {
        " AND m.is_read = 0"
    } else {
        ""
    };

    let category_filter = if let Some(cat) = category {
        format!(
            " AND COALESCE(m.category, 'primary') = '{}'",
            validated_category(cat)?
        )
    } else {
        // "All" tab: hide junk so it only shows when the Junk tab is active.
        " AND COALESCE(m.category, 'primary') != 'junk'".to_string()
    };

    let eligible_predicate = format!(
        "m.account_id = ?1 AND m.folder_name = ?2{}{}{}
         AND NOT EXISTS (SELECT 1 FROM snoozed_messages s WHERE s.account_id = m.account_id AND s.folder_name = m.folder_name AND s.uid = m.uid)",
        category_filter, mute_filter, unread_filter
    );

    let count_sql = format!(
        "WITH eligible AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         )
         SELECT COUNT(*) FROM (SELECT DISTINCT account_id, group_key FROM eligible)",
        predicate = eligible_predicate
    );

    let list_sql = format!(
        "WITH eligible AS (
            SELECT m.uid, m.subject, m.from_name, m.from_email, m.date, m.snippet,
                   m.is_read, m.is_flagged, m.has_attachments, m.size_bytes,
                   m.category, m.is_muted, m.is_pinned, m.account_id, m.folder_name,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         ),
         keys AS (SELECT DISTINCT account_id, group_key FROM eligible),
         aggregates AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key,
                   {aggregate_columns}
              FROM messages m
              JOIN keys k
                ON k.account_id = m.account_id
               AND k.group_key = COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid)
             GROUP BY m.account_id, group_key
         ),
         ranked AS (
            SELECT e.*, a.total_count, a.draft_count, a.has_unread_any,
                   ROW_NUMBER() OVER (
                     PARTITION BY e.account_id, e.group_key
                     ORDER BY e.is_pinned DESC, datetime(e.date) DESC, e.uid DESC
                   ) AS rn
              FROM eligible e
              JOIN aggregates a
                ON a.account_id = e.account_id AND a.group_key = e.group_key
         )
         SELECT uid, subject, from_name, from_email, date, snippet,
                is_read, is_flagged, has_attachments, size_bytes,
                category, is_muted, is_pinned,
                (total_count - 1) AS thread_count,
                draft_count AS thread_draft_count,
                group_key AS thread_root_id,
                has_unread_any AS thread_has_unread
           FROM ranked
          WHERE rn = 1
          ORDER BY is_pinned DESC, datetime(date) DESC
          LIMIT ?3 OFFSET ?4",
        predicate = eligible_predicate,
        aggregate_columns = thread_aggregate_columns(Some("?2")),
    );

    let total: u32 = conn.query_row(&count_sql, params![account_id, folder_name], |row| {
        row.get(0)
    })?;

    let offset = page * page_size;
    let mut stmt = conn.prepare(&list_sql)?;
    let messages = stmt
        .query_map(params![account_id, folder_name, page_size, offset], |row| {
            Ok(MessageRow {
                uid: row.get(0)?,
                account_id: Some(account_id.to_string()),
                folder_name: Some(folder_name.to_string()),
                subject: row.get(1)?,
                from_name: row.get(2)?,
                from_email: row.get(3)?,
                date: row.get(4)?,
                snippet: row.get(5)?,
                is_read: row.get::<_, i32>(6)? != 0,
                is_flagged: row.get::<_, i32>(7)? != 0,
                has_attachments: row.get::<_, i32>(8)? != 0,
                size_bytes: row.get(9)?,
                category: row.get(10)?,
                is_muted: row.get::<_, i32>(11)? != 0,
                is_pinned: row.get::<_, i32>(12)? != 0,
                thread_count: row.get::<_, i32>(13).unwrap_or(0) as u32,
                thread_draft_count: row.get::<_, i32>(14).unwrap_or(0) as u32,
                thread_root_id: row.get(15)?,
                thread_has_unread: row.get::<_, i32>(16).unwrap_or(0) != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((messages, total))
}

/// List messages from INBOX across all accounts (for unified inbox).
/// When `account_ids` is provided, only include messages from those accounts.
pub fn list_all_inboxes(
    conn: &Connection,
    page: u32,
    page_size: u32,
    category: Option<&str>,
    account_ids: Option<&[String]>,
    unread_only: bool,
) -> Result<(Vec<MessageRow>, u32), AppError> {
    // Empty account list means no results
    if let Some(ids) = account_ids {
        if ids.is_empty() {
            return Ok((vec![], 0));
        }
    }

    let cat_filter = if let Some(cat) = category {
        format!(
            " AND COALESCE(m.category, 'primary') = '{}'",
            validated_category(cat)?
        )
    } else {
        // "All" tab: hide junk unless the Junk tab is explicitly selected.
        " AND COALESCE(m.category, 'primary') != 'junk'".to_string()
    };

    let acct_filter = if let Some(ids) = account_ids {
        let placeholders: Vec<String> = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        format!(" AND m.account_id IN ({})", placeholders.join(", "))
    } else {
        String::new()
    };

    let unread_filter = if unread_only {
        " AND m.is_read = 0"
    } else {
        ""
    };

    // Hidden accounts are excluded for `None` AND `Some(ids)` alike: the only
    // caller passing ids is an account folder, which is still an aggregate.
    let eligible_predicate = format!(
        "m.folder_name = 'INBOX'{}{}{} AND m.is_muted = 0
         AND {}
         AND NOT EXISTS (SELECT 1 FROM snoozed_messages s WHERE s.account_id = m.account_id AND s.folder_name = m.folder_name AND s.uid = m.uid)",
        acct_filter,
        cat_filter,
        unread_filter,
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );

    let count_sql = format!(
        "WITH eligible AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         )
         SELECT COUNT(*) FROM (SELECT DISTINCT account_id, group_key FROM eligible)",
        predicate = eligible_predicate
    );

    // Build dynamic params for account_ids
    let acct_params: Vec<&dyn rusqlite::types::ToSql> = account_ids
        .map(|ids| {
            ids.iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect()
        })
        .unwrap_or_default();

    let mut count_stmt = conn.prepare(&count_sql)?;
    let total: u32 = count_stmt.query_row(acct_params.as_slice(), |row| row.get(0))?;

    let offset = page * page_size;
    let param_offset = acct_params.len();
    let list_sql = format!(
        "WITH eligible AS (
            SELECT m.uid, m.account_id, m.subject, m.from_name, m.from_email, m.date, m.snippet,
                   m.is_read, m.is_flagged, m.has_attachments, m.size_bytes,
                   m.category, m.is_muted, m.is_pinned, m.folder_name,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key
              FROM messages m
             WHERE {predicate}
         ),
         keys AS (SELECT DISTINCT account_id, group_key FROM eligible),
         aggregates AS (
            SELECT m.account_id,
                   COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid) AS group_key,
                   {aggregate_columns}
              FROM messages m
              JOIN keys k
                ON k.account_id = m.account_id
               AND k.group_key = COALESCE(m.thread_root_id, m.message_id, 'uid:' || m.folder_name || ':' || m.uid)
             GROUP BY m.account_id, group_key
         ),
         ranked AS (
            SELECT e.*, a.total_count, a.draft_count, a.has_unread_any,
                   ROW_NUMBER() OVER (
                     PARTITION BY e.account_id, e.group_key
                     ORDER BY e.is_pinned DESC, datetime(e.date) DESC, e.uid DESC
                   ) AS rn
              FROM eligible e
              JOIN aggregates a
                ON a.account_id = e.account_id AND a.group_key = e.group_key
         )
         SELECT uid, account_id, subject, from_name, from_email, date, snippet,
                is_read, is_flagged, has_attachments, size_bytes,
                category, is_muted, is_pinned,
                (total_count - 1) AS thread_count,
                draft_count AS thread_draft_count,
                group_key AS thread_root_id,
                has_unread_any AS thread_has_unread
           FROM ranked
          WHERE rn = 1
          ORDER BY is_pinned DESC, datetime(date) DESC
          LIMIT ?{} OFFSET ?{}",
        param_offset + 1, param_offset + 2,
        predicate = eligible_predicate,
        aggregate_columns = thread_aggregate_columns(None),
    );

    let mut list_params: Vec<&dyn rusqlite::types::ToSql> = acct_params.clone();
    let ps = page_size;
    list_params.push(&ps);
    list_params.push(&offset);

    let mut stmt = conn.prepare(&list_sql)?;
    let messages = stmt
        .query_map(list_params.as_slice(), |row| {
            Ok(MessageRow {
                uid: row.get(0)?,
                account_id: row.get(1)?,
                folder_name: Some("INBOX".to_string()),
                subject: row.get(2)?,
                from_name: row.get(3)?,
                from_email: row.get(4)?,
                date: row.get(5)?,
                snippet: row.get(6)?,
                is_read: row.get::<_, i32>(7)? != 0,
                is_flagged: row.get::<_, i32>(8)? != 0,
                has_attachments: row.get::<_, i32>(9)? != 0,
                size_bytes: row.get(10)?,
                category: row.get(11)?,
                is_muted: row.get::<_, i32>(12)? != 0,
                is_pinned: row.get::<_, i32>(13)? != 0,
                thread_count: row.get::<_, i32>(14).unwrap_or(0) as u32,
                thread_draft_count: row.get::<_, i32>(15).unwrap_or(0) as u32,
                thread_root_id: row.get(16)?,
                thread_has_unread: row.get::<_, i32>(17).unwrap_or(0) != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((messages, total))
}

/// Minimal message row for bulk reclassification. Only the fields the
/// heuristic classifier needs, plus identifiers to target the update.
#[derive(Debug, Clone)]
pub struct ReclassifyRow {
    pub account_id: String,
    pub folder_name: String,
    pub uid: u32,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    pub subject: Option<String>,
    /// Full recipient list. Needed so `mail_rules` conditions on the `to`
    /// field can actually match — they were previously evaluated against an
    /// empty string on both this path and the sync path.
    pub to_list: String,
    pub category_source: Option<String>,
    /// Whether the message carried a `List-Unsubscribe` header.
    ///
    /// It IS stored — `messages.list_unsubscribe` — and this path used to pass
    /// `false` with a comment claiming otherwise. Half the unclassified inbox
    /// (5,909 of 11,943 on the live mailbox) carries it, and without it the
    /// classifier has no bulk signal at all and files them as Primary.
    pub has_list_unsubscribe: bool,
}

/// List every message in an inbox-type folder for a full reclassification
/// pass. Used by the backfill command — returns everything at once; the
/// caller is responsible for chunking any expensive work.
/// Inbox rows for the classifier.
///
/// `only_unclassified` narrows to rows that never got a category at all, which
/// is what the startup backfill wants: it is idempotent by construction (after
/// one pass nothing matches) and it cannot revisit a message whose category a
/// rule or the user already decided. The RulesManager button passes `false` and
/// re-decides everything, as it always has.
pub fn list_for_reclassify(
    conn: &Connection,
    only_unclassified: bool,
) -> Result<Vec<ReclassifyRow>, AppError> {
    let scope = if only_unclassified {
        " AND m.category IS NULL"
    } else {
        ""
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT m.account_id, m.folder_name, m.uid, m.from_email, m.from_name, m.subject, m.to_list, m.category_source,
                m.list_unsubscribe
         FROM messages m
         JOIN folders f ON m.account_id = f.account_id AND m.folder_name = f.name
         WHERE f.folder_type = 'inbox'{scope}"
    ))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ReclassifyRow {
                account_id: row.get(0)?,
                folder_name: row.get(1)?,
                uid: row.get(2)?,
                from_email: row.get(3)?,
                from_name: row.get(4)?,
                subject: row.get(5)?,
                to_list: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                category_source: row.get(7)?,
                has_list_unsubscribe: row
                    .get::<_, Option<String>>(8)?
                    .is_some_and(|v| !v.trim().is_empty()),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Return the set of UIDs that already have cached bodies in the given folder.
pub fn get_cached_body_uids(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<std::collections::HashSet<u32>, AppError> {
    let mut stmt =
        conn.prepare("SELECT uid FROM message_bodies WHERE account_id = ?1 AND folder_name = ?2")?;
    let uids = stmt
        .query_map(params![account_id, folder_name], |row| row.get::<_, u32>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(uids)
}

pub fn get_body(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<MessageBodyRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT plain_text, sanitized_html, attachment_metadata_checked, cid_resolved FROM message_bodies
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let mut rows = stmt.query_map(params![account_id, folder_name, uid], |row| {
        Ok(MessageBodyRow {
            plain_text: row.get(0)?,
            sanitized_html: row.get(1)?,
            attachment_metadata_checked: row.get::<_, i32>(2)? != 0,
            cid_resolved: row.get::<_, i32>(3)? != 0,
        })
    })?;
    match rows.next() {
        Some(Ok(body)) => Ok(Some(body)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

pub fn insert_body(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    plain_text: Option<&str>,
    html_body: Option<&str>,
    sanitized_html: Option<&str>,
    cid_resolved: bool,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO message_bodies (account_id, folder_name, uid, plain_text, html_body, sanitized_html, attachment_metadata_checked, cid_resolved)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![account_id, folder_name, uid, plain_text, html_body, sanitized_html, cid_resolved as i32],
    )?;
    Ok(())
}

/// Targeted UPDATE used by the cache-HIT self-heal path. Preserves
/// `summary`, `summary_model`, `to_json`, `cc_json`, and
/// `attachment_metadata_checked` — only touches `sanitized_html` and the
/// `cid_resolved` flag. INSERT OR REPLACE would clobber summary fields.
pub fn update_sanitized_html_after_cid_repair(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    sanitized_html: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE message_bodies
            SET sanitized_html = ?1,
                cid_resolved   = 1
          WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![sanitized_html, account_id, folder_name, uid],
    )?;
    Ok(())
}

/// Cache a freshly-fetched body without destroying anything already on the
/// row. Third member of the "don't reach for `insert_body` here" family
/// (gotcha #14): `insert_body` is `INSERT OR REPLACE`, which re-creates the
/// row from scratch and NULLs **six** columns the caller never meant to touch
/// — `summary`, `summary_model`, `to_json`, `cc_json`, `bcc_json`, and
/// `attachment_metadata_checked`.
///
/// FTS-safe under gotcha #26: the insert branch fires `message_bodies_fts_ai`,
/// the conflict branch fires `message_bodies_fts_au` (`AFTER UPDATE OF
/// plain_text`, which is in the SET list), and there is **no implicit DELETE**
/// — strictly safer than `INSERT OR REPLACE`.
///
/// Refuses to overwrite a non-blank cached body with a blank one (BUG-03's
/// failure mode); returns `Ok(false)` in that case, `Ok(true)` when written.
// Signature deliberately mirrors `insert_body` so the two are drop-in
// comparable at every call site.
#[allow(clippy::too_many_arguments)]
pub fn upsert_body_preserving_metadata(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    plain_text: Option<&str>,
    html_body: Option<&str>,
    sanitized_html: Option<&str>,
    cid_resolved: bool,
) -> Result<bool, AppError> {
    // Spelled out rather than `is_none_or` — that's 1.82 and the crate's
    // declared MSRV is 1.77.2.
    let blank = |v: Option<&str>| v.map(|s| s.trim().is_empty()).unwrap_or(true);
    let incoming_is_blank = blank(plain_text) && blank(sanitized_html);
    if incoming_is_blank {
        if let Some(existing) = get_body(conn, account_id, folder_name, uid)? {
            let existing_has_content = existing
                .plain_text
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty())
                || existing
                    .sanitized_html
                    .as_deref()
                    .is_some_and(|h| !h.trim().is_empty());
            if existing_has_content {
                return Ok(false);
            }
        }
    }

    conn.execute(
        "INSERT INTO message_bodies
             (account_id, folder_name, uid, plain_text, html_body, sanitized_html, cid_resolved)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(account_id, folder_name, uid) DO UPDATE SET
             plain_text     = excluded.plain_text,
             html_body      = excluded.html_body,
             sanitized_html = excluded.sanitized_html,
             cid_resolved   = excluded.cid_resolved,
             fetched_at     = datetime('now')",
        params![
            account_id,
            folder_name,
            uid,
            plain_text,
            html_body,
            sanitized_html,
            cid_resolved as i32
        ],
    )?;
    Ok(true)
}

/// Cache the verbatim RFC 5322 header block for one message (v46).
///
/// Not part of the `insert_body` family — `message_headers` is its own table
/// precisely so this write can happen without a `message_bodies` row existing
/// (see `schema::migrate_v46_message_headers` for why that matters). Idempotent
/// upsert: re-storing refreshes `fetched_at`, which is the honest thing to
/// record since a re-fetch may legitimately return a different block (a message
/// re-delivered under the same UID cannot happen, but a `BODY[HEADER]` fetch
/// and a `BODY[]`-derived block can differ in whitespace).
pub fn store_raw_headers(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    raw_headers: &str,
) -> Result<(), AppError> {
    if raw_headers.trim().is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO message_headers (account_id, folder_name, uid, raw_headers)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, folder_name, uid) DO UPDATE SET
             raw_headers = excluded.raw_headers,
             fetched_at  = datetime('now')",
        params![account_id, folder_name, uid, raw_headers],
    )?;
    Ok(())
}

/// Every address this message was addressed to, in the order a send-as match
/// should consider them: `To`, then `Cc`, then the delivery headers
/// (`Delivered-To` / `X-Original-To` / `Envelope-To`).
///
/// To/Cc come from the cached `messages` row and are always available. The
/// delivery headers come from `message_headers`, which is populated only where
/// a raw header block has already passed through (gotcha #37) — about 15% of
/// the live mailbox — so this deliberately does NOT fetch: a reply's default
/// From must not wait on an IMAP round trip. When the block is absent the
/// To/Cc legs answer on their own, which is the common case; a forwarder that
/// rewrote `To` simply falls back to the primary.
pub fn recipients_for_send_as_match(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT to_list, cc_list FROM messages
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let mut rows = stmt.query_map(params![account_id, folder_name, uid], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    let (to_json, cc_json) = match rows.next() {
        Some(Ok(pair)) => pair,
        Some(Err(e)) => return Err(AppError::Database(e)),
        None => (None, None),
    };

    let mut out: Vec<String> = Vec::new();
    let push = |addr: String, out: &mut Vec<String>| {
        let key = super::identities::normalize_addr(&addr);
        if key.is_empty() {
            return;
        }
        if !out
            .iter()
            .any(|e| super::identities::normalize_addr(e) == key)
        {
            out.push(addr);
        }
    };
    for json in [to_json, cc_json].into_iter().flatten() {
        for addr in addresses_from_json(&json) {
            push(addr, &mut out);
        }
    }
    if let Some(block) = get_raw_headers(conn, account_id, folder_name, uid)? {
        for addr in super::identities::delivery_header_addresses(&block) {
            push(addr, &mut out);
        }
    }
    Ok(out)
}

/// `to_list` / `cc_list` hold `serde_json::to_string(&Vec<EmailAddress>)`.
/// A row written before those columns existed, or by a path that stored a bare
/// string, degrades to no addresses rather than an error — this feeds a
/// default, never a correctness decision.
pub(crate) fn addresses_from_json_pub(json: &str) -> Vec<String> {
    addresses_from_json(json)
}

fn addresses_from_json(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| v.get("email").and_then(|e| e.as_str()))
        .map(str::to_string)
        .collect()
}

pub fn get_raw_headers(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT raw_headers FROM message_headers
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let mut rows = stmt.query_map(params![account_id, folder_name, uid], |row| {
        row.get::<_, String>(0)
    })?;
    match rows.next() {
        Some(Ok(h)) => Ok(Some(h)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

pub fn store_address_lists(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    to_json: &str,
    cc_json: &str,
    bcc_json: &str,
) -> Result<(), AppError> {
    // `bcc_json` only lives on `message_bodies` (there's no `messages.bcc_list`
    // column — Bcc is never surfaced in list/search views, only carried back
    // into compose on draft reopen). So no `messages` sync for it, unlike
    // to_list/cc_list below.
    conn.execute(
        "UPDATE message_bodies SET to_json = ?1, cc_json = ?2, bcc_json = ?3
         WHERE account_id = ?4 AND folder_name = ?5 AND uid = ?6",
        params![to_json, cc_json, bcc_json, account_id, folder_name, uid],
    )?;
    // Keep messages.to_list / cc_list in sync with the body's address lists so
    // recipient search, voice per-recipient queries, and group `to_list` rules
    // see the data (BUG-02: these drifted NULL because only the one-shot v34
    // migration ever populated them). Only fill when the column is empty (never
    // clobber a richer envelope-derived value) and only from a non-empty source.
    conn.execute(
        "UPDATE messages SET to_list = ?1
         WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4
           AND (to_list IS NULL OR to_list = '' OR to_list = '[]')
           AND ?1 != '' AND ?1 != '[]'",
        params![to_json, account_id, folder_name, uid],
    )?;
    conn.execute(
        "UPDATE messages SET cc_list = ?1
         WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4
           AND (cc_list IS NULL OR cc_list = '' OR cc_list = '[]')
           AND ?1 != '' AND ?1 != '[]'",
        params![cc_json, account_id, folder_name, uid],
    )?;
    Ok(())
}

#[allow(clippy::type_complexity)]
pub fn get_address_lists(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<(Option<String>, Option<String>, Option<String>), AppError> {
    let mut stmt = conn.prepare(
        "SELECT to_json, cc_json, bcc_json FROM message_bodies
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let result = stmt.query_row(params![account_id, folder_name, uid], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    });
    match result {
        Ok(lists) => Ok(lists),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None, None)),
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Both spellings `messages.message_id` may hold for one logical ID, for
/// binding into an `IN (?2, ?3)` lookup. Falls back to the caller's raw string
/// (twice) when the input isn't Message-ID-shaped at all, so the query stays
/// well-formed and simply matches nothing.
///
/// `IN (?2, ?3)` — never `trim(message_id, '<>') = ?2`: a function on the
/// column defeats `idx_messages_message_id` and turns every reply lookup into
/// a ~29k-row scan. See gotcha #30.
fn message_id_variants(message_id: &str) -> (String, String) {
    cxmail_core::mail::message_id::message_id_match_variants(message_id)
        .unwrap_or_else(|| (message_id.to_string(), message_id.to_string()))
}

/// Look up threading headers by message_id string (for MCP reply threading).
/// Matches both the bracketed and bare spellings — see [`message_id_variants`].
pub fn get_threading_headers(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Option<(String, Option<String>)>, AppError> {
    let (bracketed, bare) = message_id_variants(message_id);
    let mut stmt = conn.prepare(
        "SELECT message_id, reference_ids FROM messages
         WHERE account_id = ?1 AND message_id IN (?2, ?3) LIMIT 1",
    )?;
    let result = stmt.query_row(params![account_id, bracketed, bare], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    });
    match result {
        Ok(headers) => Ok(Some(headers)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Source row for quoting the original message in an MCP-drafted reply.
#[derive(Debug, Clone)]
pub struct ReplySource {
    pub folder_name: String,
    pub uid: u32,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
}

/// Locate the message being replied to by Message-ID. Matches both spellings
/// of the column (see [`message_id_variants`]).
///
/// When the same message exists in multiple folders (INBOX + [Gmail]/All Mail),
/// prefer a copy whose body is not just present but *non-blank* — a bare
/// `mb.uid IS NOT NULL` test happily picks a folder holding an empty body row
/// and sends the caller down the IMAP-refetch path (or worse, quotes nothing).
pub fn get_reply_source(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Option<ReplySource>, AppError> {
    let (bracketed, bare) = message_id_variants(message_id);
    let mut stmt = conn.prepare(
        "SELECT m.folder_name, m.uid, m.from_name, m.from_email, m.date
           FROM messages m
           LEFT JOIN message_bodies mb
             ON mb.account_id = m.account_id
            AND mb.folder_name = m.folder_name
            AND mb.uid = m.uid
          WHERE m.account_id = ?1 AND m.message_id IN (?2, ?3)
          ORDER BY CASE
                     WHEN trim(coalesce(mb.sanitized_html, '')) != ''
                       OR trim(coalesce(mb.plain_text, '')) != '' THEN 2
                     WHEN mb.uid IS NOT NULL THEN 1
                     ELSE 0
                   END DESC
          LIMIT 1",
    )?;
    let result = stmt
        .query_row(params![account_id, bracketed, bare], |row| {
            Ok(ReplySource {
                folder_name: row.get(0)?,
                uid: row.get(1)?,
                from_name: row.get(2)?,
                from_email: row.get(3)?,
                date: row.get(4)?,
            })
        })
        .optional()?;
    Ok(result)
}

/// `(message_id, references, in_reply_to)` for one message, each normalized.
///
/// `in_reply_to` is returned because a draft carries its OWN threading headers,
/// and reopening a draft must round-trip them — rebuilding the MIME without
/// them is what silently unthreaded every reply sent from CXMail.
///
/// Normalization is load-bearing, not cosmetic: the column holds three
/// spellings of the same ID (bracketed, bare, HTML-escaped — gotcha #30) and
/// these values are fed straight into outgoing `In-Reply-To` / `References`
/// headers. An escaped `&lt;id&gt;` emitted verbatim is a header no server
/// will thread on, and `strip_message_id_brackets` can't rescue it.
pub fn get_message_headers(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<(Option<String>, Option<String>, Option<String>), AppError> {
    let mut stmt = conn.prepare(
        "SELECT message_id, reference_ids, in_reply_to FROM messages
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
    )?;
    let result = stmt.query_row(params![account_id, folder_name, uid], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    });
    match result {
        Ok((message_id, references, in_reply_to)) => Ok((
            message_id
                .as_deref()
                .and_then(cxmail_core::mail::message_id::normalize_message_id),
            references
                .as_deref()
                .map(cxmail_core::mail::message_id::normalize_reference_chain)
                .filter(|chain| !chain.is_empty()),
            in_reply_to
                .as_deref()
                .and_then(cxmail_core::mail::message_id::normalize_message_id),
        )),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None, None)),
        Err(e) => Err(AppError::Database(e)),
    }
}

fn is_user_visible_attachment(att: &cxmail_core::mail::text::AttachmentMeta) -> bool {
    !(att.is_inline && att.content_id.is_some())
}

pub fn insert_attachments(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    attachments: &[cxmail_core::mail::text::AttachmentMeta],
) -> Result<(), AppError> {
    // Replace any prior rows for this message so repeated parses (cache miss
    // after purge, or a later prefetch run) don't accumulate duplicates.
    conn.execute(
        "DELETE FROM attachments WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3",
        params![account_id, folder_name, uid],
    )?;

    if !attachments.is_empty() {
        let mut stmt = conn.prepare(
            "INSERT INTO attachments (account_id, folder_name, message_uid, filename, content_type, size_bytes, content_id, is_inline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for att in attachments {
            stmt.execute(params![
                account_id,
                folder_name,
                uid,
                att.filename,
                att.content_type,
                att.size_bytes as i64,
                att.content_id,
                att.is_inline as i32,
            ])?;
        }
    }

    let has_visible_attachments = attachments.iter().any(is_user_visible_attachment);
    conn.execute(
        "UPDATE messages
         SET has_attachments = ?1
         WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![has_visible_attachments as i32, account_id, folder_name, uid],
    )?;
    conn.execute(
        "UPDATE message_bodies
         SET attachment_metadata_checked = 1
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
        params![account_id, folder_name, uid],
    )?;
    Ok(())
}

pub fn get_attachments(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Vec<cxmail_core::mail::text::AttachmentMeta>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT filename, content_type, size_bytes, content_id, is_inline FROM attachments
         WHERE account_id = ?1 AND folder_name = ?2 AND message_uid = ?3
         ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![account_id, folder_name, uid], |row| {
            Ok(cxmail_core::mail::text::AttachmentMeta {
                filename: row.get(0)?,
                content_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                size_bytes: row.get::<_, i64>(2).unwrap_or(0) as u64,
                content_id: row.get(3)?,
                is_inline: row.get::<_, i32>(4).unwrap_or(0) != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Search known contacts (from_name, from_email) extracted from cached messages.
/// Get the thread (conversation) for a given thread root or member message_id.
/// Matches by `thread_root_id` first (post-v29 collapse key), then falls back
/// to the legacy reference-chain match so this still works when called with
/// any member's `message_id` rather than the root.
///
/// **One card per MESSAGE, not per row.** The match is account-wide and a
/// message routinely exists in several folders at once — on Gmail every sent
/// message is also a row in `[Gmail]/All Mail` — so the raw rows rendered a
/// thread with each of your own replies printed twice. Members are collapsed
/// on the message's own identity (`message_id`, falling back to the row's
/// `folder:uid` so a NULL or blank id can never merge two distinct messages),
/// and the survivor is the copy in the most meaningful folder — see
/// [`folders::folder_rank_sql`] for why Sent outranks both Drafts and Archive.
///
/// Drafts are deliberately NOT excluded. They are returned like any other
/// member, and the caller tells them apart by their folder: an unsent reply is
/// something you want to see sitting in the thread it belongs to, as long as
/// it does not read as a message that was sent. What it must not do is inflate
/// the thread COUNT — that exclusion lives in the list queries above.
pub fn get_thread(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<MessageRow>, AppError> {
    let sql = format!(
        "WITH members AS (
            SELECT uid, account_id, folder_name, subject, from_name, from_email, date, snippet,
                   is_read, is_flagged, has_attachments, size_bytes, category, is_muted, is_pinned,
                   thread_root_id,
                   COALESCE(NULLIF(message_id, ''), 'uid:' || folder_name || ':' || uid) AS dedupe_key,
                   {rank} AS folder_rank
              FROM messages
             WHERE account_id = ?1
             AND (thread_root_id = ?2
                  OR message_id = ?2 OR in_reply_to = ?2 OR reference_ids LIKE ?3
                  OR message_id IN (
                    SELECT in_reply_to FROM messages WHERE account_id = ?1 AND message_id = ?2
                  )
                  OR message_id IN (
                    SELECT message_id FROM messages WHERE account_id = ?1 AND (in_reply_to = ?2 OR reference_ids LIKE ?3)
                  ))
         ),
         ranked AS (
            SELECT members.*, ROW_NUMBER() OVER (
                     PARTITION BY dedupe_key
                     ORDER BY folder_rank, LENGTH(folder_name), folder_name, uid
                   ) AS rn
              FROM members
         )
         SELECT uid, account_id, folder_name, subject, from_name, from_email, date, snippet,
                is_read, is_flagged, has_attachments, size_bytes, category, is_muted, is_pinned,
                thread_root_id
           FROM ranked
          WHERE rn = 1
          ORDER BY datetime(date) ASC",
        rank = super::folders::folder_rank_sql("messages.account_id", "messages.folder_name"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let pattern = format!("%{}%", message_id);
    let messages = stmt
        .query_map(params![account_id, message_id, pattern], |row| {
            Ok(MessageRow {
                uid: row.get(0)?,
                account_id: row.get(1)?,
                folder_name: row.get(2)?,
                subject: row.get(3)?,
                from_name: row.get(4)?,
                from_email: row.get(5)?,
                date: row.get(6)?,
                snippet: row.get(7)?,
                is_read: row.get::<_, i32>(8)? != 0,
                is_flagged: row.get::<_, i32>(9)? != 0,
                has_attachments: row.get::<_, i32>(10)? != 0,
                size_bytes: row.get(11)?,
                category: row.get(12)?,
                is_muted: row.get::<_, i32>(13)? != 0,
                is_pinned: row.get::<_, i32>(14)? != 0,
                // thread_count stored column dropped in v37; thread members
                // don't surface a per-message count (the list row's recomputed
                // count gates thread loading).
                thread_count: 0,
                thread_draft_count: 0,
                thread_root_id: row.get(15)?,
                thread_has_unread: false,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(messages)
}

/// Get thread messages with message_id included (for MCP reply threading).
pub fn get_thread_with_ids(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<ThreadMessageRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT uid, folder_name, message_id, subject, from_email, date, to_list, cc_list
         FROM messages
         WHERE account_id = ?1
         AND (message_id = ?2 OR in_reply_to = ?2 OR reference_ids LIKE ?3
              OR message_id IN (
                SELECT in_reply_to FROM messages WHERE account_id = ?1 AND message_id = ?2
              )
              OR message_id IN (
                SELECT message_id FROM messages WHERE account_id = ?1 AND (in_reply_to = ?2 OR reference_ids LIKE ?3)
              ))
         ORDER BY datetime(date) ASC",
    )?;
    let pattern = format!("%{}%", message_id);
    let messages = stmt
        .query_map(params![account_id, message_id, pattern], |row| {
            Ok(ThreadMessageRow {
                uid: row.get(0)?,
                folder_name: row.get(1)?,
                message_id: row.get(2)?,
                subject: row.get(3)?,
                from_email: row.get(4)?,
                date: row.get(5)?,
                to_list: row.get(6)?,
                cc_list: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(messages)
}

/// Search known contacts (from_name, from_email) extracted from cached messages.
pub fn search_contacts(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<(Option<String>, String)>, AppError> {
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT DISTINCT from_name, from_email FROM messages
         WHERE (from_email LIKE ?1 OR from_name LIKE ?1)
         AND from_email IS NOT NULL AND from_email != ''
         ORDER BY datetime(date) DESC LIMIT ?2",
    )?;
    let contacts = stmt
        .query_map(params![pattern, limit], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(contacts)
}

/// Find the most recent message from a sender that has a List-Unsubscribe header.
pub fn find_unsubscribable_by_sender(
    conn: &Connection,
    account_id: &str,
    sender_email: &str,
) -> Result<Option<(String, u32)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT folder_name, uid FROM messages
         WHERE account_id = ?1 AND LOWER(from_email) = LOWER(?2)
         AND list_unsubscribe IS NOT NULL
         ORDER BY datetime(date) DESC LIMIT 1",
    )?;
    let result = stmt.query_row(params![account_id, sender_email], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    });
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Find any message from a sender that has a cached body (for body link scanning).
pub fn find_any_by_sender_with_body(
    conn: &Connection,
    account_id: &str,
    sender_email: &str,
) -> Result<Option<(String, u32)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT m.folder_name, m.uid FROM messages m
         INNER JOIN message_bodies mb
           ON m.account_id = mb.account_id AND m.folder_name = mb.folder_name AND m.uid = mb.uid
         WHERE m.account_id = ?1 AND LOWER(m.from_email) = LOWER(?2)
         ORDER BY datetime(m.date) DESC LIMIT 1",
    )?;
    let result = stmt.query_row(params![account_id, sender_email], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    });
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Database(e)),
    }
}

pub fn get_summary(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<Option<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT summary FROM message_bodies
         WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3 AND summary IS NOT NULL",
    )?;
    let mut rows = stmt.query_map(params![account_id, folder_name, uid], |row| {
        row.get::<_, String>(0)
    })?;
    match rows.next() {
        Some(Ok(summary)) => Ok(Some(summary)),
        Some(Err(e)) => Err(AppError::Database(e)),
        None => Ok(None),
    }
}

pub fn save_summary(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    summary: &str,
    model: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE message_bodies SET summary = ?1, summary_model = ?2
         WHERE account_id = ?3 AND folder_name = ?4 AND uid = ?5",
        params![summary, model, account_id, folder_name, uid],
    )?;
    Ok(())
}

pub fn update_flags(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    is_read: bool,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE messages SET is_read = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![is_read as i32, account_id, folder_name, uid],
    )?;
    Ok(())
}

pub fn apply_server_flags(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    flags: &[(u32, bool, bool)],
) -> Result<u32, AppError> {
    let tx = conn.unchecked_transaction()?;
    let mut changed = 0u32;
    {
        let mut select = tx.prepare(
            "SELECT is_read, is_flagged FROM messages
             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
        )?;
        let mut update = tx.prepare(
            "UPDATE messages SET is_read = ?1, is_flagged = ?2
             WHERE account_id = ?3 AND folder_name = ?4 AND uid = ?5",
        )?;

        for (uid, is_read, is_flagged) in flags {
            let current = select
                .query_row(params![account_id, folder_name, uid], |row| {
                    Ok((row.get::<_, i32>(0)? != 0, row.get::<_, i32>(1)? != 0))
                })
                .optional()?;

            if let Some((current_read, current_flagged)) = current {
                if current_read != *is_read || current_flagged != *is_flagged {
                    update.execute(params![
                        *is_read as i32,
                        *is_flagged as i32,
                        account_id,
                        folder_name,
                        uid
                    ])?;
                    changed += 1;
                }
            }
        }
    }
    tx.commit()?;
    Ok(changed)
}

/// Get UIDs of recent messages without snippets (no cached body either).
pub fn uids_without_snippets(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    limit: u32,
) -> Result<Vec<u32>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT m.uid FROM messages m
         WHERE m.account_id = ?1 AND m.folder_name = ?2
           AND (m.snippet IS NULL OR m.snippet = '')
           AND NOT EXISTS (
             SELECT 1 FROM message_bodies mb
             WHERE mb.account_id = m.account_id AND mb.folder_name = m.folder_name AND mb.uid = m.uid
           )
         ORDER BY m.uid DESC LIMIT ?3",
    )?;
    let uids = stmt
        .query_map(params![account_id, folder_name, limit], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(uids)
}

pub fn update_snippet(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    snippet: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE messages SET snippet = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![snippet, account_id, folder_name, uid],
    )?;
    Ok(())
}

/// Backfill snippets from cached message bodies for messages that have no snippet.
/// Uses the parser's clean_snippet to strip CSS/HTML artifacts.
pub fn backfill_snippets_from_bodies(conn: &Connection) -> Result<u32, AppError> {
    use cxmail_core::mail::text as parser;

    // Include rows where plain_text is empty but html_body exists — the parse-time
    // flow falls back to HTML in that case (see parser.rs:126) and the backfill
    // needs to mirror that so emails with HTML-only bodies get clean snippets too.
    let mut stmt = conn.prepare(
        "SELECT m.account_id, m.folder_name, m.uid, mb.plain_text, mb.html_body
         FROM messages m
         JOIN message_bodies mb ON mb.account_id = m.account_id
           AND mb.folder_name = m.folder_name AND mb.uid = m.uid
         WHERE ((mb.plain_text IS NOT NULL AND mb.plain_text != '')
                OR (mb.html_body IS NOT NULL AND mb.html_body != ''))
           AND (m.snippet IS NULL OR m.snippet = '' OR m.snippet LIKE '%{%' OR m.snippet LIKE '%margin%' OR m.snippet LIKE '%padding%'
                OR m.snippet LIKE '%&#%' OR m.snippet LIKE '%&amp;%' OR m.snippet LIKE '%&lt;%' OR m.snippet LIKE '%&gt;%' OR m.snippet LIKE '%&quot;%')
         LIMIT 50"
    )?;

    let rows: Vec<(String, String, u32, Option<String>, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0u32;
    for (account_id, folder_name, uid, plain_text, html_body) in &rows {
        // Derive snippet inside catch_unwind — some HTML emails crash the
        // parser (see Gotcha #9); a bad row shouldn't abort the whole batch.
        let snippet = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plain_text
                .as_deref()
                .map(parser::clean_snippet)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    html_body.as_deref().and_then(|html| {
                        let text = parser::html_to_plain_text_public(html);
                        let s = parser::clean_snippet(&text);
                        if s.is_empty() {
                            None
                        } else {
                            Some(s)
                        }
                    })
                })
        }));

        match snippet {
            Ok(Some(ref s)) => {
                conn.execute(
                    "UPDATE messages SET snippet = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
                    params![s, account_id, folder_name, uid],
                )?;
                count += 1;
            }
            Ok(None) => {}
            Err(_) => {
                log::error!(
                    "backfill_snippets: panicked on {}:{}:{}, skipping",
                    account_id,
                    folder_name,
                    uid
                );
            }
        }
    }

    Ok(count)
}

pub fn update_muted(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    is_muted: bool,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE messages SET is_muted = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![is_muted as i32, account_id, folder_name, uid],
    )?;
    Ok(())
}

pub fn update_pinned(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    is_pinned: bool,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE messages SET is_pinned = ?1 WHERE account_id = ?2 AND folder_name = ?3 AND uid = ?4",
        params![is_pinned as i32, account_id, folder_name, uid],
    )?;
    Ok(())
}

// `update_thread_counts` removed (bug-bash BUG-06 cleanup): the stored
// `messages.thread_count` column was a vestigial reply-neighbor metric that no
// view surfaced — every list/detail path recomputes thread size live via
// `thread_root_id` as `(total_count - 1)`. The column itself is dropped in
// migration v37; canonical thread grouping is maintained by
// `update_thread_root_ids` below.

/// Compute and persist `thread_root_id` for any messages in this folder
/// that are missing one. Used by sync to keep the column populated for
/// rows inserted before the v29 migration ran (or during partial inserts).
/// Adopt an existing thread for messages that carry no threading headers.
///
/// A message with neither `References` nor `In-Reply-To` roots to itself
/// (`compute_thread_root_id`'s last resort), which makes it its own
/// single-message conversation. That is correct for genuinely new mail and
/// wrong for the ~16 replies CXMail sent without threading headers before
/// gotcha #39 was fixed — and for anything else that arrives header-less. Those
/// cannot be repaired at the source: the message is already delivered.
///
/// So we do what Gmail does when `References` is absent — match on subject and
/// participants — but only ever as a fallback, and only for orphans:
///
/// * a message with real threading headers is **never** re-rooted; a header is
///   a fact and a subject match is a guess.
/// * the normalized subject must be specific enough to identify a conversation
///   (`normalize_subject_for_threading` refuses generic and short ones).
/// * the two messages must share a participant. Same subject alone merges every
///   "Weekly Status Update" in the mailbox into one thread.
/// * both must fall inside `window_days` of each other.
///
/// * the orphan's subject must actually announce itself as a reply
///   (`has_reply_prefix`). **This one is not optional.** Without it the rule
///   "same subject + same participant" describes every recurring notification
///   in the mailbox: the first live run fused 379 identically-titled Synology
///   alerts into a single thread, along with 180 calendar reminders and 85
///   receipts. A repeated subject is a repeated notification; a repeated
///   subject with `Re:` in front is someone answering.
///
/// Orphans adopt the root of the OLDEST message in the matched group, so a
/// chain of orphans converges on one root instead of pairing off.
///
/// `reset_first` recomputes every orphan's root from its own headers before
/// stitching, so the pass is computed from a clean base and a changed rule
/// self-corrects instead of layering on top of an older run's output. Use it
/// for the whole-history migration; leave it OFF for the per-sync call, whose
/// window is short and would otherwise strip stitches whose anchor lies outside
/// it.
///
/// Returns how many rows were re-rooted.
pub fn stitch_orphan_threads(
    conn: &Connection,
    window_days: i64,
    reset_first: bool,
) -> Result<usize, AppError> {
    if reset_first {
        // Safe to undo unconditionally: an orphan carries no threading headers,
        // so this restores exactly what `compute_thread_root_id` would derive.
        // Messages WITH headers are untouched — their root is a fact, not a
        // guess, and was never ours to rewrite.
        conn.execute(
            "UPDATE messages
                SET thread_root_id = COALESCE(message_id, 'uid:' || folder_name || ':' || uid)
              WHERE in_reply_to IS NULL AND reference_ids IS NULL
                AND datetime(date) >= datetime('now', ?1)",
            params![format!("-{window_days} days")],
        )?;
    }
    struct Candidate {
        id: i64,
        account_id: String,
        subject: String,
        date: String,
        thread_root_id: Option<String>,
        is_orphan: bool,
        is_reply_shaped: bool,
        participants: Vec<String>,
    }

    let mut stmt = conn.prepare(
        "SELECT m.id, m.account_id, COALESCE(m.subject, ''), m.date, m.thread_root_id,
                (m.in_reply_to IS NULL AND m.reference_ids IS NULL) AS is_orphan,
                COALESCE(m.from_email, ''), m.to_list, m.cc_list
           FROM messages m
          WHERE COALESCE(m.subject, '') <> ''
            AND datetime(m.date) >= datetime('now', ?1)
          ORDER BY datetime(m.date) ASC",
    )?;

    let window = format!("-{window_days} days");
    let rows = stmt.query_map(params![window], |row| {
        let from_email: String = row.get(6)?;
        let to_json: Option<String> = row.get(7)?;
        let cc_json: Option<String> = row.get(8)?;
        let mut participants = crate::db::needs_you::extract_addresses(to_json.as_deref());
        participants.extend(crate::db::needs_you::extract_addresses(cc_json.as_deref()));
        if !from_email.is_empty() {
            participants.push(from_email.to_lowercase());
        }
        let subject: String = row.get(2)?;
        Ok(Candidate {
            id: row.get(0)?,
            account_id: row.get(1)?,
            date: row.get(3)?,
            thread_root_id: row.get(4)?,
            is_orphan: row.get::<_, i32>(5)? != 0,
            is_reply_shaped: cxmail_core::mail::message_id::has_reply_prefix(&subject),
            subject,
            participants,
        })
    })?;

    // (account, normalized subject) → members, oldest first (the query is ASC).
    let mut groups: HashMap<(String, String), Vec<Candidate>> = HashMap::new();
    for row in rows {
        let row = row?;
        let Some(key) = cxmail_core::mail::message_id::normalize_subject_for_threading(&row.subject)
        else {
            continue;
        };
        groups
            .entry((row.account_id.clone(), key))
            .or_default()
            .push(row);
    }

    let mut rewrites: Vec<(i64, String)> = Vec::new();
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        // The anchor is the oldest member that already belongs to a thread and
        // is NOT an orphan itself — a real, header-derived root. Falling back to
        // the oldest orphan lets a run of header-less messages still converge.
        let anchor_idx = members
            .iter()
            .position(|m| !m.is_orphan && m.thread_root_id.is_some())
            .unwrap_or(0);
        let anchor = &members[anchor_idx];
        let Some(root) = anchor.thread_root_id.clone() else {
            continue;
        };

        for (idx, member) in members.iter().enumerate() {
            if !member.is_orphan || member.id == anchor.id {
                continue;
            }
            // The gate that separates a reply from a recurring notification.
            if !member.is_reply_shaped {
                continue;
            }
            // Threading only ever flows forward: you join a conversation that
            // already existed, you never retroactively pull earlier mail into
            // yours. Without this, an orphan carrying a real `In-Reply-To` to
            // some OTHER thread becomes the anchor and drags the genuine thread
            // start in behind it — a guess overriding a fact, which is the one
            // thing this fallback must never do. (Caught by
            // `a_message_with_real_headers_is_never_re_rooted`.)
            //
            // Compared by POSITION, not by date string: rows arrive
            // `ORDER BY datetime(date) ASC`, so the index is already a correct
            // chronological rank. Comparing the raw strings would be the
            // gotcha #23 trap — this column holds two timestamp formats.
            if idx < anchor_idx {
                continue;
            }
            if member.thread_root_id.as_deref() == Some(root.as_str()) {
                continue; // already stitched — this is what makes reruns free
            }
            let shares_participant = member
                .participants
                .iter()
                .any(|p| anchor.participants.contains(p));
            if !shares_participant {
                continue;
            }
            if days_between(&anchor.date, &member.date) > window_days {
                continue;
            }
            rewrites.push((member.id, root.clone()));
        }
    }

    if rewrites.is_empty() {
        return Ok(0);
    }

    let tx = conn.unchecked_transaction()?;
    {
        let mut update = tx.prepare("UPDATE messages SET thread_root_id = ?1 WHERE id = ?2")?;
        for (id, root) in &rewrites {
            update.execute(params![root, id])?;
        }
    }
    tx.commit()?;
    Ok(rewrites.len())
}

/// Whole days between two stored timestamps, tolerant of the formats this
/// column holds (ISO-8601 with offset from sync, SQLite's space format from
/// local writes). Unparseable → `i64::MAX`, which fails the window check
/// closed: a date we cannot read is not evidence that two messages are related.
fn days_between(a: &str, b: &str) -> i64 {
    fn parse(s: &str) -> Option<chrono::NaiveDateTime> {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.naive_utc())
            .ok()
            .or_else(|| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
            .or_else(|| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
    }
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => (x - y).num_days().abs(),
        _ => i64::MAX,
    }
}

pub fn update_thread_root_ids(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<(), AppError> {
    let mut select = conn.prepare(
        "SELECT id, uid, message_id, in_reply_to, reference_ids
           FROM messages
          WHERE account_id = ?1 AND folder_name = ?2 AND thread_root_id IS NULL",
    )?;
    let rows: Vec<(i64, u32, Option<String>, Option<String>, Option<String>)> = select
        .query_map(params![account_id, folder_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if rows.is_empty() {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    {
        let mut update = tx.prepare("UPDATE messages SET thread_root_id = ?1 WHERE id = ?2")?;
        for (id, uid, message_id, in_reply_to, reference_ids) in &rows {
            let root = compute_thread_root_id(
                message_id.as_deref(),
                in_reply_to.as_deref(),
                reference_ids.as_deref(),
                folder_name,
                *uid,
            );
            update.execute(params![root, id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Return all UIDs that belong to the given thread within one (account, folder).
/// Used to fan out per-thread actions (archive/delete/markRead) to every member
/// in the displayed folder, while leaving members in other folders (Sent, etc.)
/// untouched.
pub fn get_thread_uids_in_folder(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    thread_root_id: &str,
) -> Result<Vec<u32>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT uid FROM messages
          WHERE account_id = ?1
            AND folder_name = ?2
            AND COALESCE(thread_root_id, message_id, 'uid:' || folder_name || ':' || uid) = ?3",
    )?;
    let uids = stmt
        .query_map(params![account_id, folder_name, thread_root_id], |row| {
            row.get::<_, i64>(0).map(|v| v as u32)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(uids)
}

/// Purge cached message bodies older than the given retention period.
/// Returns the number of purged rows.
pub fn purge_old_bodies(conn: &Connection, retention_days: u32) -> Result<usize, AppError> {
    let affected = conn.execute(
        "DELETE FROM message_bodies WHERE fetched_at < datetime('now', ?1)",
        params![format!("-{} days", retention_days)],
    )?;
    Ok(affected)
}

// `search_by_filters` removed with the FTS5 migration (v40): all structured
// search goes through `db::search::search`, which ANDs metadata filters with
// the FTS MATCH instead of unioning two engines.

/// Count total unread messages across all inbox folders — the dock badge.
/// Follows All Inboxes, so an account hidden from aggregates is not counted.
pub fn count_total_inbox_unread(conn: &Connection) -> Result<u32, AppError> {
    let sql = format!(
        "SELECT COUNT(*) FROM messages m
         JOIN folders f ON m.account_id = f.account_id AND m.folder_name = f.name
         WHERE f.folder_type = 'inbox' AND m.is_read = 0
           AND {}",
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );
    let count: u32 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count)
}

/// Get all UIDs for a specific folder (used to detect genuinely new messages after sync).
pub fn get_uids_for_folder(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<Vec<u32>, AppError> {
    let mut stmt =
        conn.prepare("SELECT uid FROM messages WHERE account_id = ?1 AND folder_name = ?2")?;
    let uids = stmt
        .query_map(params![account_id, folder_name], |row| {
            row.get::<_, i64>(0).map(|v| v as u32)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(uids)
}

pub fn get_recent_uids_for_folder(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    limit: u32,
) -> Result<Vec<u32>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT uid FROM messages
         WHERE account_id = ?1 AND folder_name = ?2
         ORDER BY uid DESC LIMIT ?3",
    )?;
    let uids = stmt
        .query_map(params![account_id, folder_name, limit], |row| {
            row.get::<_, i64>(0).map(|v| v as u32)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(uids)
}

pub fn get_max_uid_for_folder(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<u32, AppError> {
    conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
        |row| row.get::<_, i64>(0).map(|value| value as u32),
    )
    .map_err(AppError::Database)
}

pub fn delete_folder_cache(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
) -> Result<u32, AppError> {
    let deleted = conn.execute(
        "DELETE FROM messages WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
    )?;
    Ok(deleted as u32)
}

pub fn delete_uids(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uids: &[u32],
) -> Result<u32, AppError> {
    let tx = conn.unchecked_transaction()?;
    let mut deleted = 0u32;
    {
        let mut stmt = tx.prepare(
            "DELETE FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
        )?;
        for uid in uids {
            deleted += stmt.execute(params![account_id, folder_name, uid])? as u32;
        }
    }
    tx.commit()?;
    Ok(deleted)
}

#[cfg(test)]
mod recipient_integrity_tests {
    use super::*;

    // Minimal schema for the recipient round-trip: message_bodies holds the
    // authoritative JSON address lists; messages mirrors to_list/cc_list.
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message_bodies (
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                to_json TEXT, cc_json TEXT, bcc_json TEXT);
             CREATE TABLE messages (
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                message_id TEXT, in_reply_to TEXT, reference_ids TEXT,
                subject TEXT, from_email TEXT, date TEXT NOT NULL DEFAULT '',
                to_list TEXT, cc_list TEXT);",
        )
        .unwrap();
        conn
    }

    // The "Sam" incident in the persistence layer: a message with TWO To
    // recipients plus a Cc and a Bcc must round-trip with every recipient
    // intact — never just the first To.
    #[test]
    fn store_and_get_address_lists_preserve_all_recipients() {
        let conn = setup();
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid) VALUES ('acct','INBOX',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid) VALUES ('acct','INBOX',1)",
            [],
        )
        .unwrap();

        let to_json = r#"[{"name":"Dana","email":"dana@example.com"},{"name":null,"email":"sam@example.com"}]"#;
        let cc_json = r#"[{"name":"Morgan","email":"morgan@example.com"}]"#;
        let bcc_json = r#"[{"name":null,"email":"secret@example.com"}]"#;

        store_address_lists(&conn, "acct", "INBOX", 1, to_json, cc_json, bcc_json).unwrap();

        let (to, cc, bcc) = get_address_lists(&conn, "acct", "INBOX", 1).unwrap();
        assert_eq!(to.as_deref(), Some(to_json));
        assert_eq!(cc.as_deref(), Some(cc_json));
        assert_eq!(bcc.as_deref(), Some(bcc_json));
        // The dropped-recipient regression: the SECOND To must survive.
        assert!(to.unwrap().contains("sam@example.com"));

        // messages mirror populated for search/thread views.
        let mirror: String = conn
            .query_row("SELECT to_list FROM messages WHERE uid = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(mirror.contains("dana@example.com") && mirror.contains("sam@example.com"));
    }

    // read_thread surfaces recipients: get_thread_with_ids must return the
    // to_list/cc_list columns so an agent sees who each message went to.
    #[test]
    fn thread_rows_carry_recipient_lists() {
        let conn = setup();
        conn.execute(
            "INSERT INTO messages
               (account_id, folder_name, uid, message_id, subject, from_email, date, to_list, cc_list)
             VALUES ('acct','INBOX',1,'<m1@x>','Hi','sender@x.com','2026-01-01T00:00:00Z',
                     '[{\"name\":null,\"email\":\"a@x.com\"},{\"name\":null,\"email\":\"b@x.com\"}]',
                     '[{\"name\":null,\"email\":\"c@x.com\"}]')",
            [],
        )
        .unwrap();

        let rows = get_thread_with_ids(&conn, "acct", "<m1@x>").unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.folder_name, "INBOX");
        assert!(r.to_list.as_deref().unwrap().contains("a@x.com"));
        assert!(r.to_list.as_deref().unwrap().contains("b@x.com"));
        assert!(r.cc_list.as_deref().unwrap().contains("c@x.com"));
    }
}

#[cfg(test)]
mod reclassify_row_tests {
    use super::*;

    /// `mail_rules` conditions on the `to` field were evaluated against an
    /// empty string on BOTH the sync and the reclassify path, so they could
    /// never match even though the model and the UI offered the field. The
    /// recipient list was available all along. This pins the reclassify half:
    /// if `to_list` drops out of the SELECT again, `to` rules go quietly dead.
    #[test]
    fn list_for_reclassify_carries_the_recipient_list() {
        let conn = Connection::open_in_memory().unwrap();
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
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, subject, from_email, to_list, date)
             VALUES ('acct', 'INBOX', 1, 'Report Domain: cxventures.io',
                     'noreply-dmarc-support@google.com', 'dmarc@cxventures.io',
                     '2026-08-01T10:00:00Z')",
            [],
        )
        .unwrap();
        // A NULL to_list must degrade to "" rather than failing the query.
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, subject, from_email, date)
             VALUES ('acct', 'INBOX', 2, 'No recipients', 'x@y.com', '2026-08-01T11:00:00Z')",
            [],
        )
        .unwrap();

        let rows = list_for_reclassify(&conn, false).unwrap();
        assert_eq!(rows.len(), 2);
        let dmarc = rows.iter().find(|r| r.uid == 1).unwrap();
        assert_eq!(dmarc.to_list, "dmarc@cxventures.io");
        assert_eq!(rows.iter().find(|r| r.uid == 2).unwrap().to_list, "");

        // And it really drives a `to` rule through the production engine.
        let rule = crate::db::rules::MailRule {
            id: None,
            account_id: None,
            name: "dmarc".into(),
            is_active: true,
            priority: 0,
            conditions: vec![crate::db::rules::RuleCondition {
                field: "to".into(),
                operator: "contains".into(),
                value: "dmarc@".into(),
            }],
            actions: vec![crate::db::rules::RuleAction {
                action_type: "set_category".into(),
                value: Some("updates".into()),
            }],
        };
        let hit = crate::db::rules::evaluate(
            std::slice::from_ref(&rule),
            dmarc.from_email.as_deref().unwrap_or(""),
            &dmarc.to_list,
            dmarc.subject.as_deref().unwrap_or(""),
            "",
            &dmarc.account_id,
        );
        assert_eq!(hit.len(), 1, "a `to` condition must now match");
    }
}

/// Reply-source lookup (gotcha #30) and the guarded body write-back
/// (gotcha #14 / #26). These run against the FULL production schema so the
/// v40 FTS triggers, the composite FK, and the real column set are all in
/// play — the shapes that actually broke.
#[cfg(test)]
mod reply_lookup_tests {
    use super::*;

    fn schema_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();
        conn
    }

    fn seed_message(conn: &Connection, folder: &str, uid: u32, mid: &str, refs: Option<&str>) {
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, message_id, reference_ids, subject,
                 from_name, from_email, date)
             VALUES ('acct', ?1, ?2, ?3, ?4, 'Re: CRM Transfer Update',
                     'Sam Ellis', 'sam@harborline.example', '2026-07-25T15:28:00Z')",
            params![folder, uid, mid, refs],
        )
        .unwrap();
    }

    fn seed_body(conn: &Connection, folder: &str, uid: u32, plain: &str, html: &str) {
        insert_body(
            conn,
            "acct",
            folder,
            uid,
            Some(plain),
            Some(html),
            Some(html),
            true,
        )
        .unwrap();
    }

    /// ⭐ THE REGRESSION TEST. `messages.message_id` holds two spellings —
    /// 29k bracketed rows from the IMAP sync and 16 bare ones minted by
    /// `draft_local` — and the old exact-match lookup silently returned None
    /// on any mismatch, which the MCP then reported as "original body not
    /// cached". Both spellings of the *input* must resolve both spellings of
    /// the *stored* row.
    #[test]
    fn get_reply_source_matches_bare_and_bracketed_message_id() {
        let conn = schema_conn();

        // Bracketed row — the live shape of INBOX uid 410.
        seed_message(&conn, "INBOX", 410, "<A7EA3536@harborline.example>", None);
        seed_body(&conn, "INBOX", 410, "original body", "<p>original body</p>");

        for input in [
            "<A7EA3536@harborline.example>",
            "A7EA3536@harborline.example",
            // The third live spelling: HTML-escaped, as found in
            // [Gmail]/Drafts uid 578's In-Reply-To.
            "&lt;A7EA3536@harborline.example&gt;",
        ] {
            let src = get_reply_source(&conn, "acct", input)
                .unwrap()
                .unwrap_or_else(|| panic!("bracketed row must resolve for input {input:?}"));
            assert_eq!(src.folder_name, "INBOX");
            assert_eq!(src.uid, 410);
        }

        // Bare row — the `draft_local` shape (16 such rows in the live DB).
        seed_message(&conn, "[Gmail]/Drafts", 578, "7455e0a7@cxmail.app", None);
        seed_body(
            &conn,
            "[Gmail]/Drafts",
            578,
            "draft body",
            "<p>draft body</p>",
        );

        for input in ["<7455e0a7@cxmail.app>", "7455e0a7@cxmail.app"] {
            let src = get_reply_source(&conn, "acct", input)
                .unwrap()
                .unwrap_or_else(|| panic!("bare row must resolve for input {input:?}"));
            assert_eq!(src.uid, 578);
        }

        assert!(get_reply_source(&conn, "acct", "<nope@x>")
            .unwrap()
            .is_none());
    }

    #[test]
    fn get_threading_headers_matches_bare_and_bracketed() {
        let conn = schema_conn();
        seed_message(
            &conn,
            "INBOX",
            410,
            "<A7EA3536@harborline.example>",
            Some("<root@x>"),
        );
        seed_message(&conn, "[Gmail]/Drafts", 578, "7455e0a7@cxmail.app", None);

        for input in ["<A7EA3536@harborline.example>", "A7EA3536@harborline.example"] {
            let (mid, refs) = get_threading_headers(&conn, "acct", input)
                .unwrap()
                .unwrap_or_else(|| panic!("no headers for {input:?}"));
            assert_eq!(mid, "<A7EA3536@harborline.example>");
            assert_eq!(refs.as_deref(), Some("<root@x>"));
        }
        for input in ["<7455e0a7@cxmail.app>", "7455e0a7@cxmail.app"] {
            let (mid, _) = get_threading_headers(&conn, "acct", input)
                .unwrap()
                .unwrap_or_else(|| panic!("no headers for {input:?}"));
            assert_eq!(mid, "7455e0a7@cxmail.app");
        }
    }

    /// The old `ORDER BY (mb.uid IS NOT NULL) DESC` happily picked a folder
    /// whose body row exists but is blank, sending the caller down the IMAP
    /// refetch path (or quoting nothing) while a perfectly good copy sat in
    /// another folder.
    #[test]
    fn get_reply_source_prefers_folder_with_nonblank_body() {
        let conn = schema_conn();
        seed_message(&conn, "INBOX", 10, "<mid@x>", None);
        seed_message(&conn, "[Gmail]/All Mail", 77, "<mid@x>", None);
        // Blank body row in INBOX (present, but nothing to quote), real body
        // in All Mail. INBOX sorts first by rowid, so only the ORDER BY can
        // save this.
        insert_body(
            &conn,
            "acct",
            "INBOX",
            10,
            Some("   "),
            None,
            Some(""),
            true,
        )
        .unwrap();
        seed_body(
            &conn,
            "[Gmail]/All Mail",
            77,
            "real body",
            "<p>real body</p>",
        );

        let src = get_reply_source(&conn, "acct", "<mid@x>").unwrap().unwrap();
        assert_eq!(
            src.folder_name, "[Gmail]/All Mail",
            "must prefer the copy with a NON-BLANK body, not merely a body row"
        );
        assert_eq!(src.uid, 77);
    }

    #[test]
    fn upsert_body_preserves_summary_and_address_lists() {
        let conn = schema_conn();
        seed_message(&conn, "INBOX", 410, "<A7EA3536@harborline.example>", None);
        seed_body(&conn, "INBOX", 410, "stale", "<p>stale</p>");

        // Metadata the user/app accumulated on the row (gotcha #14's trap:
        // `insert_body` is INSERT OR REPLACE and NULLs all six of these).
        conn.execute(
            "UPDATE message_bodies
                SET summary = 'AI summary', summary_model = 'claude-opus-5',
                    to_json = '[{\"email\":\"a@x.com\"}]', cc_json = '[{\"email\":\"c@x.com\"}]',
                    bcc_json = '[{\"email\":\"b@x.com\"}]', attachment_metadata_checked = 1
              WHERE account_id='acct' AND folder_name='INBOX' AND uid=410",
            [],
        )
        .unwrap();

        let wrote = upsert_body_preserving_metadata(
            &conn,
            "acct",
            "INBOX",
            410,
            Some("fresh"),
            Some("<p>fresh</p>"),
            Some("<p>fresh</p>"),
            true,
        )
        .unwrap();
        assert!(wrote);

        // Every column `insert_body`'s INSERT OR REPLACE would have NULLed.
        let col = |name: &str| -> Option<String> {
            conn.query_row(
                &format!(
                    "SELECT {name} FROM message_bodies
                      WHERE account_id='acct' AND folder_name='INBOX' AND uid=410"
                ),
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
        };

        assert_eq!(
            col("plain_text").as_deref(),
            Some("fresh"),
            "body must be refreshed"
        );
        assert_eq!(
            col("summary").as_deref(),
            Some("AI summary"),
            "summary clobbered"
        );
        assert_eq!(col("summary_model").as_deref(), Some("claude-opus-5"));
        assert_eq!(col("to_json").as_deref(), Some(r#"[{"email":"a@x.com"}]"#));
        assert_eq!(col("cc_json").as_deref(), Some(r#"[{"email":"c@x.com"}]"#));
        assert_eq!(col("bcc_json").as_deref(), Some(r#"[{"email":"b@x.com"}]"#));
        let checked: i64 = conn
            .query_row(
                "SELECT attachment_metadata_checked FROM message_bodies
                  WHERE account_id='acct' AND folder_name='INBOX' AND uid=410",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(checked, 1);

        // Exactly one row — the ON CONFLICT branch updated, never duplicated.
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_bodies WHERE account_id='acct' AND folder_name='INBOX' AND uid=410",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn upsert_body_inserts_when_absent() {
        let conn = schema_conn();
        seed_message(&conn, "INBOX", 410, "<A7EA3536@harborline.example>", None);

        assert!(get_body(&conn, "acct", "INBOX", 410).unwrap().is_none());
        let wrote = upsert_body_preserving_metadata(
            &conn,
            "acct",
            "INBOX",
            410,
            Some("first cache"),
            None,
            Some("<p>first cache</p>"),
            true,
        )
        .unwrap();
        assert!(wrote);

        let body = get_body(&conn, "acct", "INBOX", 410).unwrap().unwrap();
        assert_eq!(body.plain_text.as_deref(), Some("first cache"));
        assert!(body.cid_resolved);
    }

    /// BUG-03's failure mode: never let an empty refetch erase a good body.
    #[test]
    fn upsert_body_refuses_to_blank_an_existing_body() {
        let conn = schema_conn();
        seed_message(&conn, "INBOX", 410, "<A7EA3536@harborline.example>", None);
        seed_body(&conn, "INBOX", 410, "good body", "<p>good body</p>");

        let wrote = upsert_body_preserving_metadata(
            &conn,
            "acct",
            "INBOX",
            410,
            Some("  "),
            None,
            None,
            true,
        )
        .unwrap();
        assert!(!wrote, "blank overwrite must be refused");
        let body = get_body(&conn, "acct", "INBOX", 410).unwrap().unwrap();
        assert_eq!(body.plain_text.as_deref(), Some("good body"));
    }

    /// Gotcha #26 invariant 1, mirroring
    /// `insert_or_replace_body_yields_one_searchable_row`: the upsert must
    /// leave exactly one searchable FTS row, on both the insert and the
    /// conflict branch.
    #[test]
    fn upsert_body_keeps_one_searchable_row() {
        use crate::db::search::{self, SearchFilters};
        let conn = schema_conn();
        seed_message(&conn, "INBOX", 410, "<A7EA3536@harborline.example>", None);

        upsert_body_preserving_metadata(
            &conn,
            "acct",
            "INBOX",
            410,
            Some("distinctivephrase alpha"),
            None,
            Some("<p>distinctivephrase alpha</p>"),
            true,
        )
        .unwrap();
        upsert_body_preserving_metadata(
            &conn,
            "acct",
            "INBOX",
            410,
            Some("distinctivephrase beta"),
            None,
            Some("<p>distinctivephrase beta</p>"),
            true,
        )
        .unwrap();

        let hits = search::search(
            &conn,
            &SearchFilters {
                keywords: Some("distinctivephrase".into()),
                ..Default::default()
            },
            10,
            0,
            false,
        )
        .unwrap();
        assert_eq!(hits.len(), 1, "exactly one searchable row, got {hits:?}");

        // The UPDATE trigger refreshed the indexed body, so the stale term is
        // gone and the fresh one is findable.
        let beta = search::search(
            &conn,
            &SearchFilters {
                keywords: Some("beta".into()),
                ..Default::default()
            },
            10,
            0,
            false,
        )
        .unwrap();
        assert_eq!(beta.len(), 1, "updated body must be re-indexed");

        let counts: (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM messages), (SELECT COUNT(*) FROM messages_fts)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts.0, counts.1, "messages/messages_fts count drift");
    }
}

/// An account hidden from aggregates disappears from the cross-account list
/// and the badge — and ONLY from those. Each test pins one query; the
/// single-account path is the negative control that proves the mail is still
/// there when the account itself is clicked.
#[cfg(test)]
mod aggregate_visibility_tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        for (id, hidden) in [("a", 0), ("h", 1)] {
            conn.execute(
                "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
                 VALUES (?1, ?1 || '@example.com','imap','imap.example.com','smtp.example.com')",
                params![id],
            )
            .unwrap();
            super::super::accounts::set_hidden_from_aggregates(&conn, id, hidden == 1).unwrap();
            conn.execute(
                "INSERT INTO folders (account_id,name,folder_type) VALUES (?1,'INBOX','inbox')",
                params![id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,is_read)
                 VALUES (?1,'INBOX',1,'hello from ' || ?1,'x@example.com',
                         strftime('%Y-%m-%dT%H:%M:%S+00:00','now','-1 day'),0)",
                params![id],
            )
            .unwrap();
        }
        conn
    }

    fn account_ids(rows: &[MessageRow]) -> Vec<String> {
        rows.iter().filter_map(|r| r.account_id.clone()).collect()
    }

    #[test]
    fn all_inboxes_skips_the_hidden_account_with_and_without_explicit_ids() {
        let conn = setup();

        let (rows, total) = list_all_inboxes(&conn, 0, 50, None, None, false).unwrap();
        assert_eq!(account_ids(&rows), vec!["a"]);
        assert_eq!(total, 1);

        // An account folder that happens to contain the hidden account is
        // still an aggregate — naming the id does not opt it back in here.
        let both = ["a".to_string(), "h".to_string()];
        let (rows, total) =
            list_all_inboxes(&conn, 0, 50, None, Some(&both), false).unwrap();
        assert_eq!(account_ids(&rows), vec!["a"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn the_dock_badge_follows_all_inboxes() {
        let conn = setup();
        assert_eq!(count_total_inbox_unread(&conn).unwrap(), 1);
        super::super::accounts::set_hidden_from_aggregates(&conn, "h", false).unwrap();
        assert_eq!(count_total_inbox_unread(&conn).unwrap(), 2);
    }

    /// The negative control: clicking the account is the one way in, so the
    /// single-account list must be untouched by the flag.
    #[test]
    fn the_hidden_accounts_own_inbox_still_lists_its_mail() {
        let conn = setup();
        let (rows, total) = list_by_folder(&conn, "h", "INBOX", 0, 50, None, false).unwrap();
        assert_eq!(account_ids(&rows), vec!["h"]);
        assert_eq!(total, 1);
    }
}

#[cfg(test)]
mod stitch_tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
             VALUES ('a','me@example.com','imap','imap.example.com','smtp.example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id,name,folder_type) VALUES ('a','INBOX','inbox')",
            [],
        )
        .unwrap();
        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        conn: &Connection,
        uid: u32,
        from: &str,
        to: &str,
        subject: &str,
        days: i64,
        message_id: &str,
        in_reply_to: Option<&str>,
        thread_root_id: &str,
    ) {
        conn.execute(
            &format!(
                "INSERT INTO messages
                   (account_id,folder_name,uid,from_email,to_list,subject,date,message_id,
                    in_reply_to,thread_root_id)
                 VALUES ('a','INBOX',?1,?2,?3,?4,
                         strftime('%Y-%m-%dT%H:%M:%S+00:00','now','-{days} days'),?5,?6,?7)"
            ),
            params![
                uid,
                from,
                format!("[{{\"name\":null,\"email\":\"{to}\"}}]"),
                subject,
                message_id,
                in_reply_to,
                thread_root_id,
            ],
        )
        .unwrap();
    }

    fn root_of(conn: &Connection, uid: u32) -> String {
        conn.query_row(
            "SELECT thread_root_id FROM messages WHERE uid = ?1",
            params![uid],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// The gotcha #39 shape: our header-less reply rooted to itself, orphaned
    /// from the conversation it was plainly part of.
    #[test]
    fn an_orphan_adopts_the_thread_it_belongs_to() {
        let conn = setup();
        insert(&conn, 1, "dana@client.com", "me@example.com",
               "Northwind Company Meeting Follow Up", 10, "<theirs@y>", None, "<theirs@y>");
        insert(&conn, 2, "me@example.com", "dana@client.com",
               "Re: Northwind Company Meeting Follow Up", 5, "<mine@cxmail.app>", None,
               "<mine@cxmail.app>");

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 1);
        assert_eq!(root_of(&conn, 2), "<theirs@y>");
        // Idempotent — the whole point of checking the root before rewriting.
        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 0);
    }

    /// A header is a fact; a subject match is a guess. Never let the guess win.
    #[test]
    fn a_message_with_real_headers_is_never_re_rooted() {
        let conn = setup();
        insert(&conn, 1, "dana@client.com", "me@example.com",
               "Northwind Company Meeting Follow Up", 10, "<theirs@y>", None, "<theirs@y>");
        insert(&conn, 2, "me@example.com", "dana@client.com",
               "Re: Northwind Company Meeting Follow Up", 5, "<mine@x>", Some("<other@z>"),
               "<other@z>");

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 0);
        assert_eq!(root_of(&conn, 2), "<other@z>");
    }

    /// THE false-merge risk. Everyone's vendor sends "Weekly Status Update";
    /// subject alone would fuse them into one conversation.
    #[test]
    fn a_shared_subject_between_strangers_does_not_merge() {
        let conn = setup();
        insert(&conn, 1, "alex@brightloom.example", "me@example.com",
               "The Weekly Status Update Report", 10, "<a@y>", None, "<a@y>");
        insert(&conn, 2, "someone@elsewhere.com", "other@nowhere.com",
               "The Weekly Status Update Report", 5, "<b@y>", None, "<b@y>");

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 0);
        assert_eq!(root_of(&conn, 2), "<b@y>");
    }

    /// The over-merge this gate exists to stop. A recurring notification
    /// repeats its subject forever; without the reply-prefix check the live run
    /// fused 379 identical Synology alerts into one 379-message "thread".
    #[test]
    fn repeated_notifications_from_one_sender_do_not_become_a_thread() {
        let conn = setup();
        for uid in 1..=5u32 {
            insert(&conn, uid, "sns@synologynotification.com", "me@example.com",
                   "Container finance_api in Container Manager stopped unexpectedly",
                   10 - i64::from(uid), &format!("<n{uid}@y>"), None, &format!("<n{uid}@y>"));
        }

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 0);
        for uid in 1..=5u32 {
            assert_eq!(root_of(&conn, uid), format!("<n{uid}@y>"));
        }
    }

    /// `reset_first` recomputes orphan roots from headers before stitching, so a
    /// changed rule self-corrects instead of layering on the previous run.
    #[test]
    fn reset_first_undoes_a_previous_pass() {
        let conn = setup();
        insert(&conn, 1, "sns@synologynotification.com", "me@example.com",
               "Container finance_api in Container Manager stopped unexpectedly",
               10, "<n1@y>", None, "<n1@y>");
        // Simulate v48's output: stitched under a rule we no longer apply.
        insert(&conn, 2, "sns@synologynotification.com", "me@example.com",
               "Container finance_api in Container Manager stopped unexpectedly",
               5, "<n2@y>", None, "<n1@y>");

        assert_eq!(stitch_orphan_threads(&conn, 365, true).unwrap(), 0);
        assert_eq!(root_of(&conn, 2), "<n2@y>", "the bad merge must be undone");
    }

    #[test]
    fn a_generic_subject_is_never_a_thread_key() {
        let conn = setup();
        insert(&conn, 1, "dana@client.com", "me@example.com", "Hi", 10, "<a@y>", None, "<a@y>");
        insert(&conn, 2, "me@example.com", "dana@client.com", "Re: Hi", 5, "<b@y>", None, "<b@y>");

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 0);
    }

    #[test]
    fn the_window_bounds_how_far_back_a_thread_reaches() {
        let conn = setup();
        insert(&conn, 1, "dana@client.com", "me@example.com",
               "Northwind Company Meeting Follow Up", 300, "<theirs@y>", None, "<theirs@y>");
        insert(&conn, 2, "me@example.com", "dana@client.com",
               "Re: Northwind Company Meeting Follow Up", 5, "<mine@x>", None, "<mine@x>");

        // Inside the SELECT window but far outside the pairing window.
        assert_eq!(stitch_orphan_threads(&conn, 30, false).unwrap(), 0);
        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 1);
    }

    /// A run of header-less messages must converge on ONE root, not pair off.
    #[test]
    fn multiple_orphans_converge_on_a_single_root() {
        let conn = setup();
        insert(&conn, 1, "dana@client.com", "me@example.com",
               "Northwind Company Meeting Follow Up", 12, "<theirs@y>", None, "<theirs@y>");
        insert(&conn, 2, "me@example.com", "dana@client.com",
               "Re: Northwind Company Meeting Follow Up", 8, "<m1@x>", None, "<m1@x>");
        insert(&conn, 3, "me@example.com", "dana@client.com",
               "Re: Re: Northwind Company Meeting Follow Up", 4, "<m2@x>", None, "<m2@x>");

        assert_eq!(stitch_orphan_threads(&conn, 365, false).unwrap(), 2);
        assert_eq!(root_of(&conn, 2), "<theirs@y>");
        assert_eq!(root_of(&conn, 3), "<theirs@y>");
    }
}

#[cfg(test)]
mod raw_header_tests {
    use super::*;

    fn seeded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', 'chris@cxventures.io', 'gmail', 'imap.gmail.com', 'smtp.gmail.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_email, date)
             VALUES ('acct', 'INBOX', 410, 'hi', 'a@b.c', '2026-08-04T10:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn store_and_get_raw_headers_round_trips_and_is_idempotent() {
        let conn = seeded();
        assert!(get_raw_headers(&conn, "acct", "INBOX", 410).unwrap().is_none());

        store_raw_headers(&conn, "acct", "INBOX", 410, "Authentication-Results: dmarc=pass")
            .unwrap();
        assert_eq!(
            get_raw_headers(&conn, "acct", "INBOX", 410).unwrap().as_deref(),
            Some("Authentication-Results: dmarc=pass")
        );

        // Re-storing must upsert, not violate the primary key.
        store_raw_headers(&conn, "acct", "INBOX", 410, "Authentication-Results: dmarc=fail")
            .unwrap();
        assert_eq!(
            get_raw_headers(&conn, "acct", "INBOX", 410).unwrap().as_deref(),
            Some("Authentication-Results: dmarc=fail")
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM message_headers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    /// The header row must NOT double as a body-cache record. `message_headers`
    /// is a separate table precisely so a lazy header fetch cannot make sync's
    /// prefetch (`get_cached_body_uids`) or `read_email` (`get_body`) believe the
    /// body is cached — see `schema::migrate_v46_message_headers`.
    #[test]
    fn storing_headers_does_not_look_like_a_cached_body() {
        let conn = seeded();
        store_raw_headers(&conn, "acct", "INBOX", 410, "Subject: hi").unwrap();
        assert!(
            get_body(&conn, "acct", "INBOX", 410).unwrap().is_none(),
            "a header row must not present as a cached body"
        );
        assert!(
            get_cached_body_uids(&conn, "acct", "INBOX").unwrap().is_empty(),
            "a header row must not suppress body prefetch"
        );
    }

    /// Deleting a message (move / archive / delete all raw-DELETE `messages`)
    /// must evict its headers in the same transaction, like every other child.
    #[test]
    fn deleting_the_message_cascades_to_its_headers() {
        let conn = seeded();
        store_raw_headers(&conn, "acct", "INBOX", 410, "Subject: hi").unwrap();
        conn.execute("DELETE FROM messages WHERE uid = 410", []).unwrap();
        assert!(get_raw_headers(&conn, "acct", "INBOX", 410).unwrap().is_none());
    }

    #[test]
    fn blank_headers_are_not_stored() {
        let conn = seeded();
        store_raw_headers(&conn, "acct", "INBOX", 410, "   \n  ").unwrap();
        assert!(get_raw_headers(&conn, "acct", "INBOX", 410).unwrap().is_none());
    }
}

/// One card per MESSAGE, and an unsent draft is not a message.
///
/// Both halves of the same defect: `get_thread` matches account-wide, and a
/// message routinely occupies several folders at once, so the raw rows showed
/// each of your own replies twice (Sent + Gmail's `[Gmail]/All Mail` mirror)
/// and showed an unsent draft as though it had gone out. The counting queries
/// had the identical blind spot, which is what put a wrong number in the
/// list row's badge.
#[cfg(test)]
mod thread_membership_tests {
    use super::*;

    const ACCT: &str = "acct";

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
             VALUES (?1,'me@example.com','gmail','imap.gmail.com','smtp.gmail.com')",
            params![ACCT],
        )
        .unwrap();
        for (name, kind) in [
            ("INBOX", "inbox"),
            ("[Gmail]/Sent Mail", "sent"),
            ("[Gmail]/Drafts", "drafts"),
            ("[Gmail]/All Mail", "archive"),
        ] {
            conn.execute(
                "INSERT INTO folders (account_id,name,folder_type) VALUES (?1,?2,?3)",
                params![ACCT, name, kind],
            )
            .unwrap();
        }
        conn
    }

    /// `mid = None` seeds a row with no Message-ID at all — the case where the
    /// dedupe key has to fall back to the row's own coordinates.
    fn seed(
        conn: &Connection,
        folder: &str,
        uid: u32,
        mid: Option<&str>,
        root: &str,
        date: &str,
        is_read: bool,
    ) {
        conn.execute(
            "INSERT INTO messages
                (account_id, folder_name, uid, message_id, thread_root_id, subject,
                 from_name, from_email, date, is_read)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Re: Kickoff', 'Me', 'me@example.com', ?6, ?7)",
            params![ACCT, folder, uid, mid, root, date, is_read as i32],
        )
        .unwrap();
    }

    fn folders_of(rows: &[MessageRow]) -> Vec<String> {
        rows.iter()
            .map(|r| r.folder_name.clone().unwrap_or_default())
            .collect()
    }

    /// The reported bug, top half: every reply printed twice, because Gmail
    /// keeps a copy of it in All Mail as well as in Sent.
    #[test]
    fn a_thread_shows_one_card_per_message_not_one_per_folder_copy() {
        let conn = setup();
        seed(&conn, "[Gmail]/Sent Mail", 25, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/All Mail", 664, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "INBOX", 753, Some("<b@x>"), "<a@x>", "2026-09-15T14:06:42Z", true);

        let rows = get_thread(&conn, ACCT, "<a@x>").unwrap();
        assert_eq!(
            folders_of(&rows),
            vec!["[Gmail]/Sent Mail", "INBOX"],
            "the All Mail mirror is the same message as the Sent copy"
        );
    }

    /// The rank orders; it must never filter. On iCloud and Outlook — and on
    /// Gmail for anything you archived out of the inbox — the archive folder
    /// holds the ONLY copy, and dropping it would delete the message from its
    /// own thread.
    #[test]
    fn an_archived_message_with_no_other_copy_is_never_dropped() {
        let conn = setup();
        seed(&conn, "[Gmail]/All Mail", 100, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "INBOX", 101, Some("<b@x>"), "<a@x>", "2026-09-15T14:06:42Z", true);

        let rows = get_thread(&conn, ACCT, "<a@x>").unwrap();
        assert_eq!(folders_of(&rows), vec!["[Gmail]/All Mail", "INBOX"]);
    }

    /// A blank or absent Message-ID must not collapse two distinct messages
    /// into one — `COALESCE(NULLIF(...), 'uid:folder:uid')` is what keeps the
    /// key unique per row when the header is missing.
    #[test]
    fn messages_with_no_message_id_are_never_merged_with_each_other() {
        let conn = setup();
        // Two NULLs and two blanks: PARTITION BY groups equal values, and it
        // treats NULLs as equal to each other, so a key of bare `message_id`
        // would collapse each pair into one card. Only one of each would not
        // catch that — the pairs are the test.
        seed(&conn, "INBOX", 1, None, "<root@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "INBOX", 2, None, "<root@x>", "2026-06-04T13:05:06Z", true);
        seed(&conn, "INBOX", 3, Some(""), "<root@x>", "2026-06-05T13:05:06Z", true);
        seed(&conn, "INBOX", 4, Some(""), "<root@x>", "2026-06-06T13:05:06Z", true);
        seed(&conn, "[Gmail]/Sent Mail", 5, Some("<root@x>"), "<root@x>", "2026-06-07T13:05:06Z", true);

        let rows = get_thread(&conn, ACCT, "<root@x>").unwrap();
        let uids: Vec<u32> = rows.iter().map(|r| r.uid).collect();
        assert_eq!(uids, vec![1, 2, 3, 4, 5], "id-less messages are still messages");
    }

    /// Drafts rank below ordinary folders on purpose. A draft that was sent
    /// and whose Drafts row has not been expunged yet shares its Message-ID
    /// with the Sent copy — and the safe reading of that pair is "this went
    /// out", not "this is still unsent".
    #[test]
    fn a_stale_draft_of_an_already_sent_message_loses_to_the_sent_copy() {
        let conn = setup();
        seed(&conn, "[Gmail]/Drafts", 9, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Sent Mail", 25, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);

        let rows = get_thread(&conn, ACCT, "<a@x>").unwrap();
        assert_eq!(folders_of(&rows), vec!["[Gmail]/Sent Mail"]);
    }

    /// A genuine unsent draft still belongs in the thread — it is the caller's
    /// job to render it as a draft rather than as correspondence. Hiding it
    /// here would make an unfinished reply invisible in the one place you
    /// would look for it.
    #[test]
    fn a_genuine_unsent_draft_stays_in_the_thread() {
        let conn = setup();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 712, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", true);

        let rows = get_thread(&conn, ACCT, "<a@x>").unwrap();
        assert_eq!(folders_of(&rows), vec!["INBOX", "[Gmail]/Drafts"]);
    }

    /// The reported bug, bottom half: the badge counted the draft as a message
    /// that had been sent. It must count what was exchanged — and report the
    /// draft separately, because the reading pane gates thread loading on the
    /// count and a plain exclusion would stop the draft card ever rendering.
    #[test]
    fn an_unsent_draft_is_reported_separately_from_the_messages_exchanged() {
        let conn = setup();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Sent Mail", 2, Some("<b@x>"), "<a@x>", "2026-06-04T13:05:06Z", true);
        seed(&conn, "[Gmail]/All Mail", 3, Some("<b@x>"), "<a@x>", "2026-06-04T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 4, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", true);

        let (rows, _) = list_by_folder(&conn, ACCT, "INBOX", 0, 50, None, false).unwrap();
        assert_eq!(rows.len(), 1);
        // Two messages exchanged (the All Mail mirror is not a third), so the
        // badge — which prints thread_count + 1 — reads 2.
        assert_eq!(rows[0].thread_count, 1);
        assert_eq!(rows[0].thread_draft_count, 1);
    }

    /// A draft saved unread must not bold its whole thread as though someone
    /// had written to you.
    #[test]
    fn an_unread_draft_does_not_mark_its_thread_unread() {
        let conn = setup();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 4, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", false);

        let (rows, _) = list_by_folder(&conn, ACCT, "INBOX", 0, 50, None, false).unwrap();
        assert!(!rows[0].thread_has_unread);
    }

    /// …but the same exclusion must not erase the row you are actually
    /// looking at. In the Drafts folder the draft IS the listed message, so it
    /// counts there — otherwise its badge would be short by one.
    #[test]
    fn browsing_drafts_still_counts_the_draft_being_listed() {
        let conn = setup();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Sent Mail", 2, Some("<b@x>"), "<a@x>", "2026-06-04T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 4, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", true);

        let (rows, _) = list_by_folder(&conn, ACCT, "[Gmail]/Drafts", 0, 50, None, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_count, 2, "the draft plus the two exchanged messages");
        assert_eq!(rows[0].thread_draft_count, 0);
    }

    /// All Inboxes and the inbox groups are INBOX-scoped, so a draft is never
    /// the listed row there and is always excluded.
    #[test]
    fn all_inboxes_excludes_the_draft_too() {
        let conn = setup();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 4, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", true);

        let (rows, _) = list_all_inboxes(&conn, 0, 50, None, None, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_count, 0);
        assert_eq!(rows[0].thread_draft_count, 1);
    }

    /// An account whose folder list has not synced yet has no `folders` rows
    /// at all. `is_draft_sql` is an EXISTS, so everything reads as ordinary
    /// mail and the thread behaves exactly as it did before this existed —
    /// the failure direction that shows too much rather than hiding mail.
    #[test]
    fn an_unsynced_folder_list_leaves_every_message_ordinary() {
        let conn = setup();
        conn.execute("DELETE FROM folders", []).unwrap();
        seed(&conn, "INBOX", 1, Some("<a@x>"), "<a@x>", "2026-06-03T13:05:06Z", true);
        seed(&conn, "[Gmail]/Drafts", 4, Some("<d@x>"), "<a@x>", "2026-08-23T00:04:13Z", true);

        let (rows, _) = list_by_folder(&conn, ACCT, "INBOX", 0, 50, None, false).unwrap();
        assert_eq!(rows[0].thread_count, 1);
        assert_eq!(rows[0].thread_draft_count, 0);
        assert_eq!(get_thread(&conn, ACCT, "<a@x>").unwrap().len(), 2);
    }
}

#[cfg(test)]
mod send_as_recipient_tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, smtp_host)
             VALUES ('acct', 'sidalias@icloud.com', 'iCloud', 'icloud', 'imap.mail.me.com', 'smtp.mail.me.com')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_msg(conn: &Connection, uid: u32, to: &str, cc: &str) {
        conn.execute(
            "INSERT INTO messages
               (account_id, folder_name, uid, message_id, subject, from_email, date, to_list, cc_list)
             VALUES ('acct','INBOX',?1,'<m@x>','Hi','sender@x.com','2026-01-01T00:00:00Z',?2,?3)",
            params![uid, to, cc],
        )
        .unwrap();
    }

    /// The end-to-end reply default, at the layer the UI and the MCP both go
    /// through: mail that arrived at the alias resolves to the alias.
    #[test]
    fn a_message_addressed_to_the_alias_resolves_to_the_alias() {
        let conn = setup();
        insert_msg(
            &conn,
            1,
            r#"[{"name":null,"email":"RileyPrime@icloud.com"}]"#,
            "[]",
        );
        crate::db::identities::insert(
            &conn,
            &crate::db::identities::Identity {
                id: None,
                account_id: "acct".to_string(),
                email: "rileyprime@icloud.com".to_string(),
                display_name: None,
                signature_html: None,
                is_default: false,
            },
        )
        .unwrap();

        let recipients = recipients_for_send_as_match(&conn, "acct", "INBOX", 1).unwrap();
        let addresses = crate::db::identities::send_as_addresses(
            &conn,
            "acct",
            "sidalias@icloud.com",
            Some("iCloud"),
        )
        .unwrap();
        let hit = crate::db::identities::match_send_as(&addresses, &recipients).unwrap();
        assert_eq!(hit.email, "rileyprime@icloud.com");
        assert!(!hit.is_primary);
    }

    #[test]
    fn to_comes_before_cc_and_both_are_read() {
        let conn = setup();
        insert_msg(
            &conn,
            2,
            r#"[{"name":null,"email":"list@discuss.example"}]"#,
            r#"[{"name":null,"email":"alias@icloud.com"}]"#,
        );
        let got = recipients_for_send_as_match(&conn, "acct", "INBOX", 2).unwrap();
        assert_eq!(got, vec!["list@discuss.example", "alias@icloud.com"]);
    }

    /// `message_headers` is populated for only part of the mailbox (gotcha
    /// #37) and this read must never fetch, so the delivery headers ride along
    /// only when they are already cached.
    #[test]
    fn delivery_headers_are_appended_last_when_the_block_is_cached() {
        let conn = setup();
        insert_msg(
            &conn,
            3,
            r#"[{"name":null,"email":"list@discuss.example"}]"#,
            "[]",
        );
        store_raw_headers(
            &conn,
            "acct",
            "INBOX",
            3,
            "Delivered-To: rileyprime@icloud.com\r\nSubject: Hi\r\n",
        )
        .unwrap();
        let got = recipients_for_send_as_match(&conn, "acct", "INBOX", 3).unwrap();
        assert_eq!(
            got,
            vec!["list@discuss.example", "rileyprime@icloud.com"]
        );
    }

    #[test]
    fn an_unknown_uid_and_malformed_json_both_degrade_to_nothing() {
        let conn = setup();
        assert!(recipients_for_send_as_match(&conn, "acct", "INBOX", 999)
            .unwrap()
            .is_empty());
        insert_msg(&conn, 4, "not json at all", "");
        assert!(recipients_for_send_as_match(&conn, "acct", "INBOX", 4)
            .unwrap()
            .is_empty());
    }
}
