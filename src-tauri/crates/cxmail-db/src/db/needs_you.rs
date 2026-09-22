use cxmail_core::mail::needs_you::{collapse_key, evaluate, Candidate};
use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How much correspondence makes someone a correspondent. One reply to a cold
/// pitch must not brand that sender — or their whole domain — trusted forever.
const CORRESPONDENCE_THRESHOLD: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRef {
    pub account_id: String,
    pub folder_name: String,
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct NeedsYouItem {
    pub uid: u32,
    pub account_id: String,
    pub folder_name: String,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub has_attachments: bool,
    pub action_type: String,
    pub reason: String,
    /// The text that actually triggered the match.
    pub evidence: Option<String>,
    /// How many messages this row stands for (repeat alerts, thread siblings).
    pub duplicate_count: u32,
    /// Every message behind this row — dismissing the row dismisses them all,
    /// otherwise a collapsed alert storm reappears on the next refresh.
    pub members: Vec<MessageRef>,
    pub score: f32,
}

pub(crate) struct Correspondence {
    pub(crate) senders: HashSet<String>,
    pub(crate) domains: HashSet<String>,
    pub(crate) sent_message_ids: HashSet<String>,
    pub(crate) own_addresses: HashSet<String>,
    /// Addresses that have at least once replied to something we sent.
    ///
    /// A STRONGER test than `senders`, and the distinction matters. `senders`
    /// counts outbound volume (≥2 messages to that address), which a three-touch
    /// cold sequence satisfies on its own — fine for Needs You, where the
    /// candidate is an inbound message that already had to survive the
    /// classifier, and wrong for follow-up nudges, where the candidate is our
    /// own outbound mail and outbound volume is exactly what we must not treat
    /// as evidence. Measured on the live mailbox: "no reply yet" nudges 299 of
    /// 317 recent sends, `senders` still passes 19 (the cold sequences survive),
    /// and this set passes 8 — the real correspondents.
    pub(crate) repliers: HashSet<String>,
}

pub(crate) fn extract_addresses(json: Option<&str>) -> Vec<String> {
    let Some(raw) = json else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .ok()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.get("email").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

const FREEMAIL: &[&str] = &[
    "gmail.com", "googlemail.com", "icloud.com", "me.com", "yahoo.com", "hotmail.com",
    "outlook.com", "live.com", "aol.com", "proton.me", "protonmail.com",
];

/// Who do we actually write to? Built from Sent folders — the strongest local
/// signal for "this is a real correspondent" and the reason the queue can tell
/// a client apart from a stranger with the same phrasing.
pub(crate) fn load_correspondence(conn: &Connection) -> Result<Correspondence, AppError> {
    let mut own_addresses = HashSet::new();
    {
        let mut stmt = conn.prepare("SELECT lower(email) FROM accounts")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            own_addresses.insert(r?);
        }
    }

    let mut sender_counts: HashMap<String, i64> = HashMap::new();
    let mut domain_counts: HashMap<String, i64> = HashMap::new();
    let mut sent_message_ids = HashSet::new();

    let mut stmt = conn.prepare(
        "SELECT m.to_list, m.cc_list, m.message_id
           FROM messages m
           JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
          WHERE f.folder_type = 'sent'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    for row in rows {
        let (to_json, cc_json, message_id) = row?;
        let mut recipients = extract_addresses(to_json.as_deref());
        recipients.extend(extract_addresses(cc_json.as_deref()));
        for addr in recipients {
            if own_addresses.contains(&addr) {
                continue;
            }
            *sender_counts.entry(addr.clone()).or_insert(0) += 1;
            if let Some((_, domain)) = addr.split_once('@') {
                if !FREEMAIL.contains(&domain) {
                    *domain_counts.entry(domain.to_string()).or_insert(0) += 1;
                }
            }
        }
        // Bare spelling, via the helper that also copes with the bracketed and
        // HTML-escaped forms the column holds (gotcha #30).
        if let Some((_, bare)) = message_id
            .as_deref()
            .and_then(cxmail_core::mail::message_id::message_id_match_variants)
        {
            sent_message_ids.insert(bare.to_lowercase());
        }
    }

    // Who has ever answered us. Scoped to rows that actually carry an
    // `In-Reply-To` (a few thousand, vs ~29k messages) and matched in Rust
    // through `message_id_match_variants`, because the column holds three
    // spellings of the same ID and a SQL-side `OR` over them would defeat
    // `idx_messages_message_id` (gotcha #30).
    let mut repliers = HashSet::new();
    {
        let mut stmt = conn.prepare(
            "SELECT lower(from_email), in_reply_to FROM messages
              WHERE in_reply_to IS NOT NULL AND from_email IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (from_email, in_reply_to) = row?;
            if own_addresses.contains(&from_email) {
                continue;
            }
            if let Some((_, bare)) =
                cxmail_core::mail::message_id::message_id_match_variants(&in_reply_to)
            {
                if sent_message_ids.contains(&bare.to_lowercase()) {
                    repliers.insert(from_email);
                }
            }
        }
    }

    Ok(Correspondence {
        senders: sender_counts
            .into_iter()
            .filter(|(_, n)| *n >= CORRESPONDENCE_THRESHOLD)
            .map(|(a, _)| a)
            .collect(),
        domains: domain_counts
            .into_iter()
            .filter(|(_, n)| *n >= CORRESPONDENCE_THRESHOLD)
            .map(|(d, _)| d)
            .collect(),
        sent_message_ids,
        own_addresses,
        repliers,
    })
}

struct Row {
    uid: u32,
    account_id: String,
    folder_name: String,
    subject: Option<String>,
    from_name: Option<String>,
    from_email: Option<String>,
    date: String,
    snippet: Option<String>,
    is_read: bool,
    is_flagged: bool,
    has_attachments: bool,
    category: Option<String>,
    has_list_unsubscribe: bool,
    in_reply_to: Option<String>,
    thread_root_id: Option<String>,
}

/// Build the focused action queue from high-signal local metadata. Deliberately
/// deterministic: opening Needs You never sends message data to an inference
/// provider. Users can dismiss false positives permanently.
/// Build the queue.
///
/// `use_triage` is the `on`-mode switch, passed in rather than read here: this
/// module must not know about the credential store, and the caller is the one
/// place that knows whether verdicts may surface. In `off` and `shadow` it is
/// false and every candidate gets `triage: None`, which makes the queue
/// byte-identical to what it was before this feature existed.
pub fn list(conn: &Connection, limit: u32, use_triage: bool) -> Result<Vec<NeedsYouItem>, AppError> {
    // `load_correspondence` is deliberately NOT scoped by the hidden flag: it
    // is the global "who has replied to me" set, and it also feeds
    // single-account classification — a hidden account's own inbox must
    // classify exactly as before.
    let corr = load_correspondence(conn)?;

    let sql = format!(
        "SELECT m.uid, m.account_id, m.folder_name, m.subject, m.from_name, m.from_email,
                m.date, m.snippet, m.is_read, m.is_flagged, m.has_attachments, m.category,
                m.list_unsubscribe, m.in_reply_to, m.thread_root_id
           FROM messages m
           JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
          WHERE f.folder_type = 'inbox'
            AND m.is_muted = 0
            AND {}
            AND datetime(m.date) >= datetime('now', '-21 days')
            AND NOT EXISTS (
                 SELECT 1 FROM needs_you_dismissals d
                  WHERE d.account_id = m.account_id
                    AND d.folder_name = m.folder_name
                    AND d.uid = m.uid
            )
            AND NOT EXISTS (
                 SELECT 1 FROM unsubscribed_senders u
                  WHERE u.account_id = m.account_id
                    AND lower(u.sender_email) = lower(COALESCE(m.from_email, ''))
            )
          ORDER BY datetime(m.date) DESC, m.id DESC",
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map([], |row| {
        Ok(Row {
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
            category: row.get(11)?,
            has_list_unsubscribe: row.get::<_, Option<String>>(12)?.is_some(),
            in_reply_to: row.get(13)?,
            thread_root_id: row.get(14)?,
        })
    })?;

    // key -> item, plus insertion order so the newest message of a group stays
    // the representative (the query is already date DESC).
    let mut groups: HashMap<String, NeedsYouItem> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for row in rows {
        let row = row?;
        let from_email = row.from_email.clone().unwrap_or_default();
        let sender = from_email.to_lowercase();
        let domain = sender.split_once('@').map(|(_, d)| d).unwrap_or("");
        let replies_to_me = row
            .in_reply_to
            .as_deref()
            .and_then(cxmail_core::mail::message_id::message_id_match_variants)
            .is_some_and(|(_, bare)| corr.sent_message_ids.contains(&bare.to_lowercase()));

        // Read per row rather than joined into the main query: the queue's
        // SELECT is already the widest in this module, and a verdict is a
        // primary-key lookup on a table with one row per message. In `off` and
        // `shadow` this is not even called.
        let triage = if use_triage {
            super::triage::get(conn, &row.account_id, &row.folder_name, row.uid).unwrap_or(None)
        } else {
            None
        };

        let candidate = Candidate {
            triage: triage.as_ref(),
            subject: row.subject.as_deref().unwrap_or(""),
            snippet: row.snippet.as_deref().unwrap_or(""),
            from_email: &from_email,
            category: row.category.as_deref(),
            has_list_unsubscribe: row.has_list_unsubscribe,
            has_attachments: row.has_attachments,
            is_read: row.is_read,
            is_flagged: row.is_flagged,
            replies_to_me,
            known_sender: corr.senders.contains(&sender),
            known_domain: !domain.is_empty() && corr.domains.contains(domain),
            is_self: corr.own_addresses.contains(&sender),
        };

        let Some(verdict) = evaluate(&candidate) else {
            continue;
        };

        let key = collapse_key(
            verdict.action,
            &from_email,
            row.subject.as_deref().unwrap_or(""),
            &row.account_id,
            row.thread_root_id.as_deref(),
        );
        let member = MessageRef {
            account_id: row.account_id.clone(),
            folder_name: row.folder_name.clone(),
            uid: row.uid,
        };

        match groups.get_mut(&key) {
            Some(existing) => {
                existing.duplicate_count += 1;
                existing.members.push(member);
                if verdict.score > existing.score {
                    existing.score = verdict.score;
                }
                // An unread member keeps the whole group unread-looking.
                if !row.is_read {
                    existing.is_read = false;
                }
            }
            None => {
                order.push(key.clone());
                groups.insert(
                    key,
                    NeedsYouItem {
                        uid: row.uid,
                        account_id: row.account_id,
                        folder_name: row.folder_name,
                        subject: row.subject,
                        from_name: row.from_name,
                        from_email: row.from_email,
                        date: row.date,
                        snippet: row.snippet,
                        is_read: row.is_read,
                        has_attachments: row.has_attachments,
                        action_type: verdict.action.as_str().to_string(),
                        reason: verdict.reason,
                        evidence: verdict.evidence,
                        duplicate_count: 1,
                        members: vec![member],
                        score: verdict.score,
                    },
                );
            }
        }
    }

    // Newest first, and nothing re-sorts it. The query orders by
    // `datetime(m.date) DESC`, and a collapsed row is represented by the first
    // member seen — its newest — so `order` already is the answer. A
    // lane → score → date sort used to sit here, which put a two-week-old reply
    // above one from this morning and every alert last no matter when it
    // arrived: `score` is a match-strength weight the user never sees, not a
    // priority they can reason about, and an action queue that is not in date
    // order reads as broken. The score still rides on the item for callers.
    let mut items: Vec<NeedsYouItem> = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect();
    items.truncate(limit as usize);
    Ok(items)
}

pub fn dismiss(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
) -> Result<(), AppError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE account_id=?1 AND folder_name=?2 AND uid=?3)",
        params![account_id, folder_name, uid],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(AppError::NotFound("Message not found".to_string()));
    }
    conn.execute(
        "INSERT OR REPLACE INTO needs_you_dismissals (account_id, folder_name, uid, dismissed_at)
         VALUES (?1, ?2, ?3, datetime('now'))",
        params![account_id, folder_name, uid],
    )?;
    Ok(())
}

/// Dismiss every message behind a collapsed row. Without this, dismissing an
/// alert that stands for 36 messages just promotes the 37th to representative
/// and the row comes straight back.
pub fn dismiss_group(conn: &mut Connection, members: &[MessageRef]) -> Result<usize, AppError> {
    if members.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut dismissed = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO needs_you_dismissals (account_id, folder_name, uid, dismissed_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
        )?;
        for m in members {
            stmt.execute(params![m.account_id, m.folder_name, m.uid])?;
            dismissed += 1;
        }
    }
    tx.commit()?;
    Ok(dismissed)
}

#[cfg(test)]
mod tests {
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
        conn.execute(
            "INSERT INTO folders (account_id,name,folder_type) VALUES ('a','Sent','sent')",
            [],
        )
        .unwrap();
        conn
    }

    /// Two sent messages to an address make them a correspondent.
    fn record_correspondence(conn: &Connection, addr: &str, times: usize) {
        for i in 0..times {
            conn.execute(
                "INSERT INTO messages (account_id,folder_name,uid,subject,date,to_list,message_id)
                 VALUES ('a','Sent',?1,'hi','2026-07-01T00:00:00+00:00',?2,?3)",
                params![
                    9000 + i as i64,
                    format!("[{{\"name\":null,\"email\":\"{addr}\"}}]"),
                    format!("<sent-{i}@example.com>")
                ],
            )
            .unwrap();
        }
    }

    fn insert_inbox(conn: &Connection, uid: u32, from: &str, subject: &str, snippet: &str) {
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,snippet)
             VALUES ('a','INBOX',?1,?2,?3,datetime('now','-1 day'),?4)",
            params![uid, subject, from, snippet],
        )
        .unwrap();
    }

    /// Like `insert_inbox`, but at a chosen age, and in the shape the real
    /// column holds (`2026-08-24T23:47:21+00:00`) rather than sqlite's
    /// space-separated form — an ordering test should sort what production sorts.
    fn insert_inbox_at(conn: &Connection, uid: u32, from: &str, subject: &str, snippet: &str, age: &str) {
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,snippet)
             VALUES ('a','INBOX',?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%S+00:00','now',?5),?4)",
            params![uid, subject, from, snippet, age],
        )
        .unwrap();
    }

    /// Needs You is a cross-account queue, so an account hidden from
    /// aggregates contributes nothing to it — even a row that would otherwise
    /// qualify on every other test. The same row in a visible account is the
    /// control.
    #[test]
    fn a_hidden_account_never_reaches_the_queue() {
        let conn = setup();
        conn.execute(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
             VALUES ('h','warm@example.com','imap','imap.example.com','smtp.example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id,name,folder_type) VALUES ('h','INBOX','inbox')",
            [],
        )
        .unwrap();
        record_correspondence(&conn, "client@acme.com", 2);
        // The identical qualifying message in each account. Threaded per
        // account, because an unthreaded row collapses on (sender, subject)
        // across accounts — the queue's own duplicate-delivery rule.
        for acct in ["a", "h"] {
            conn.execute(
                "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,snippet,
                                       thread_root_id)
                 VALUES (?1,'INBOX',1,'Re: Scope','client@acme.com',datetime('now','-1 day'),
                         'Can you confirm Friday?','<scope-' || ?1 || '@acme.com>')",
                params![acct],
            )
            .unwrap();
        }
        assert_eq!(list(&conn, 50, false).unwrap().len(), 2, "both visible: both queued");

        super::super::accounts::set_hidden_from_aggregates(&conn, "h", true).unwrap();
        let items = list(&conn, 50, false).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].account_id, "a");
    }

    #[test]
    fn keeps_a_correspondents_question_and_drops_a_strangers_pitch() {
        let conn = setup();
        record_correspondence(&conn, "client@acme.com", 2);
        insert_inbox(&conn, 1, "client@acme.com", "Re: Scope", "Can you confirm Friday?");
        insert_inbox(
            &conn,
            2,
            "cold@vendor.io",
            "Chris, have you tried that solution?",
            "Book a call with me?",
        );

        let items = list(&conn, 50, false).unwrap();
        assert_eq!(items.len(), 1, "only the real correspondent qualifies");
        assert_eq!(items[0].uid, 1);
        assert_eq!(items[0].action_type, "reply");
        assert!(items[0].evidence.is_some());
    }

    #[test]
    fn collapses_repeat_alerts_into_one_row_that_dismisses_wholesale() {
        let mut conn = setup();
        for uid in 1..=5u32 {
            insert_inbox(
                &conn,
                uid,
                "sns@synologynotification.com",
                &format!("Container api in Container Manager stopped unexpectedly {uid}"),
                "It stopped unexpectedly.",
            );
        }

        let items = list(&conn, 50, false).unwrap();
        assert_eq!(items.len(), 1, "five notices about one container = one row");
        assert_eq!(items[0].duplicate_count, 5);
        assert_eq!(items[0].members.len(), 5);

        let n = dismiss_group(&mut conn, &items[0].members.clone()).unwrap();
        assert_eq!(n, 5);
        assert!(
            list(&conn, 50, false).unwrap().is_empty(),
            "dismissing the row must not promote the next member"
        );
    }

    /// The queue reads like an inbox: newest first, full stop. A score-first
    /// sort once put a two-week-old reply above one from this morning, and a
    /// people-before-alerts lane put every alert last however fresh it was —
    /// both read as "not sorted" to the person looking at it. The fixture is
    /// built so score runs strictly OPPOSITE to recency and the alert is the
    /// newest item, so restoring either rule fails this test.
    #[test]
    fn newest_first_regardless_of_score_or_lane() {
        let conn = setup();
        record_correspondence(&conn, "client@acme.com", 2);
        // The highest-scoring thing the queue can hold — a correspondent
        // replying to our own message, unread, flagged — and the oldest.
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,snippet,in_reply_to,is_flagged)
             VALUES ('a','INBOX',1,'Re: Scope','client@acme.com',
                     strftime('%Y-%m-%dT%H:%M:%S+00:00','now','-12 days'),
                     'Can you confirm Friday? Please send the invite.','<sent-0@example.com>',1)",
            [],
        )
        .unwrap();
        // A flagged question from the same correspondent, more recent — scores
        // below the reply above it and above the alert below it.
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,subject,from_email,date,snippet,is_flagged)
             VALUES ('a','INBOX',2,'Re: Deck','client@acme.com',
                     strftime('%Y-%m-%dT%H:%M:%S+00:00','now','-2 days'),
                     'Do you have the deck?',1)",
            [],
        )
        .unwrap();
        // An alert from a stranger: lowest score, newest of all.
        insert_inbox_at(
            &conn,
            3,
            "sns@synologynotification.com",
            "Container api in Container Manager stopped unexpectedly",
            "It stopped unexpectedly.",
            "-1 hour",
        );

        let items = list(&conn, 50, false).unwrap();
        let uids: Vec<u32> = items.iter().map(|i| i.uid).collect();
        let shape: Vec<_> = items.iter().map(|i| (i.uid, i.action_type.as_str(), i.score, i.date.as_str())).collect();
        assert_eq!(uids, vec![3, 2, 1], "newest first; got {shape:?}");
        assert!(
            items[0].score < items[1].score && items[1].score < items[2].score,
            "fixture must run score opposite to recency or it proves nothing; got {shape:?}"
        );
        assert_eq!(items[0].action_type, "alert", "the alert lane no longer sinks a fresh alert");
    }

    /// A collapsed row sits where its NEWEST member does: an alert that has
    /// been firing for ten days and fired again an hour ago is an hour old.
    #[test]
    fn a_collapsed_row_sorts_by_its_newest_member() {
        let conn = setup();
        record_correspondence(&conn, "client@acme.com", 2);
        let alert = "Container api in Container Manager stopped unexpectedly";
        insert_inbox_at(&conn, 1, "sns@synologynotification.com", alert, "It stopped unexpectedly.", "-10 days");
        insert_inbox_at(&conn, 2, "client@acme.com", "Re: Scope", "Can you confirm Friday?", "-1 day");
        insert_inbox_at(&conn, 3, "sns@synologynotification.com", alert, "It stopped unexpectedly.", "-1 hour");

        let items = list(&conn, 50, false).unwrap();
        assert_eq!(items.len(), 2, "the two alerts collapse");
        assert_eq!(items[0].uid, 3, "represented by — and dated as — its newest member");
        assert_eq!(items[0].duplicate_count, 2);
        assert_eq!(items[1].uid, 2);
    }

    #[test]
    fn a_single_reply_does_not_make_a_cold_sender_trusted() {
        let conn = setup();
        // One reply — e.g. writing back to decline — must not qualify their
        // whole sequence, nor everyone else at that domain.
        record_correspondence(&conn, "sdr@outreach.io", 1);
        insert_inbox(
            &conn,
            1,
            "sdr@outreach.io",
            "Re: following up",
            "Do you want to go over the ideas we discussed?",
        );
        insert_inbox(
            &conn,
            2,
            "other@outreach.io",
            "Streamlining your process",
            "Would you be open to a chat?",
        );
        assert!(list(&conn, 50, false).unwrap().is_empty());
    }

    #[test]
    fn dismissal_is_honored_and_limit_is_respected() {
        let conn = setup();
        record_correspondence(&conn, "client@acme.com", 2);
        insert_inbox(&conn, 1, "client@acme.com", "Re: One", "Can you confirm?");
        insert_inbox(&conn, 2, "client@acme.com", "Re: Two", "Could you send it?");
        assert_eq!(list(&conn, 50, false).unwrap().len(), 2);
        assert_eq!(list(&conn, 1, false).unwrap().len(), 1);

        dismiss(&conn, "a", "INBOX", 1).unwrap();
        let items = list(&conn, 50, false).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].uid, 2);
    }

    #[test]
    fn unsubscribed_senders_are_excluded() {
        let conn = setup();
        record_correspondence(&conn, "news@brand.com", 2);
        insert_inbox(&conn, 1, "news@brand.com", "Re: Your order", "Can you rate us?");
        assert_eq!(list(&conn, 50, false).unwrap().len(), 1);

        conn.execute(
            "INSERT INTO unsubscribed_senders (account_id, sender_email, sender_domain)
             VALUES ('a','news@brand.com','brand.com')",
            [],
        )
        .unwrap();
        assert!(list(&conn, 50, false).unwrap().is_empty());
    }
}
