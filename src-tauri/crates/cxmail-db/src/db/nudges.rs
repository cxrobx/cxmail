//! Follow-up and reply nudges — "Sent 5 days ago. Follow up?" on a thread you
//! are waiting on, "Received 4 days ago. Reply?" on one that is waiting on you.
//!
//! The whole feature is the filter. Measured against the live mailbox, the
//! obvious rule — "I sent it, nobody answered, it's been a few days" — nudges
//! **299 of 317** recent sends, because cold-outreach batches are outbound mail
//! that by design never gets a reply. What separates a client from a sequence is
//! not how much we wrote to them but whether they ever wrote back, which is why
//! this module keys on `Correspondence::repliers` rather than the
//! `CORRESPONDENCE_THRESHOLD` used by Needs You. Same data, 8 nudges instead of
//! 299.
//!
//! Both lanes are one question asked of the newest message in a thread: is it
//! mine, or theirs? Mine and unanswered → follow up. Theirs and unanswered →
//! reply. The reply lane defers to `cxmail_core::mail::needs_you::evaluate` for "is this a
//! person worth surfacing", so the automation/cold-pitch vocabulary has exactly
//! one definition in the codebase.

use crate::db::needs_you::{extract_addresses, load_correspondence};
use cxmail_core::mail::needs_you::{evaluate, Candidate};
use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// How long a thread must sit untouched before it is worth mentioning. Below
/// this, a nudge is just noise about mail the user remembers sending.
pub const DEFAULT_MIN_AGE_DAYS: i64 = 3;

/// Past this, the thread is not "waiting on a reply" any more — it is dropped,
/// and a badge saying otherwise is a lie about its own significance.
pub const DEFAULT_MAX_AGE_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeKind {
    /// We wrote last and nobody answered.
    FollowUp,
    /// They wrote last and we never answered.
    Reply,
}

impl NudgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NudgeKind::FollowUp => "follow_up",
            NudgeKind::Reply => "reply",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Nudge {
    /// `follow_up` | `reply`
    pub kind: String,
    pub account_id: String,
    /// The row the badge attaches to — the newest INBOX message of the thread,
    /// which is what the message list is already showing.
    pub folder_name: String,
    pub uid: u32,
    pub thread_key: String,
    pub subject: Option<String>,
    /// Who we are waiting on (follow-up) or who is waiting on us (reply).
    pub counterpart_email: String,
    pub counterpart_name: Option<String>,
    /// Date of the message the nudge is *about* — our last send, or their last
    /// message. Deliberately not the badge row's date: on a follow-up those are
    /// different messages and the age is the whole point.
    pub date: String,
    pub days_ago: i64,
}

pub struct NudgeOptions {
    pub min_age_days: i64,
    pub max_age_days: i64,
    pub limit: usize,
}

impl Default for NudgeOptions {
    fn default() -> Self {
        Self {
            min_age_days: DEFAULT_MIN_AGE_DAYS,
            max_age_days: DEFAULT_MAX_AGE_DAYS,
            limit: 50,
        }
    }
}

struct Row {
    account_id: String,
    folder_type: String,
    folder_name: String,
    uid: u32,
    subject: Option<String>,
    from_name: Option<String>,
    from_email: String,
    date: String,
    age_days: f64,
    snippet: Option<String>,
    is_read: bool,
    is_flagged: bool,
    is_muted: bool,
    has_attachments: bool,
    category: Option<String>,
    has_list_unsubscribe: bool,
    in_reply_to: Option<String>,
    to_list: Option<String>,
    cc_list: Option<String>,
    thread_key: String,
}

struct Thread {
    /// Newest message in the thread across inbox + sent.
    newest: Row,
    /// Newest INBOX message — the row a badge can attach to. A thread with no
    /// inbox row is not shown in the list, so there is nothing to nudge on.
    badge: Option<(String, u32, Option<String>)>,
    muted: bool,
}

/// Build both nudge lanes.
///
/// Deterministic and local: opening the inbox must never ship message text to an
/// inference provider, same posture as Needs You.
pub fn list(conn: &Connection, opts: &NudgeOptions) -> Result<Vec<Nudge>, AppError> {
    let corr = load_correspondence(conn)?;
    let dismissed = load_dismissals(conn)?;
    let unsubscribed = load_unsubscribed(conn)?;

    // One pass over inbox + sent inside the window. `julianday` parses the
    // timestamp instead of comparing it bytewise, which is what makes the age
    // correct for the ISO-8601-with-offset strings this column holds — the
    // trap gotcha #23 documents. The window bound wraps BOTH sides in
    // `datetime()` for the same reason.
    let window = format!("-{} days", opts.max_age_days);
    // The hidden-account rule drops the hidden account's SENT side as well as
    // its inbox, so no thread of its ever forms in either lane.
    let sql = format!(
        "SELECT m.account_id, f.folder_type, m.folder_name, m.uid, m.subject, m.from_name,
                COALESCE(m.from_email, ''), m.date,
                julianday('now') - julianday(m.date) AS age_days,
                m.snippet, m.is_read, m.is_flagged, m.is_muted, m.has_attachments, m.category,
                m.list_unsubscribe, m.in_reply_to, m.to_list, m.cc_list,
                COALESCE(m.thread_root_id, m.message_id,
                         'uid:' || m.folder_name || ':' || m.uid) AS thread_key
           FROM messages m
           JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
          WHERE f.folder_type IN ('inbox', 'sent')
            AND {}
            AND datetime(m.date) >= datetime('now', ?1)
          ORDER BY datetime(m.date) ASC",
        super::accounts::visible_in_aggregates_sql("m.account_id")
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map(params![window], |row| {
        Ok(Row {
            account_id: row.get(0)?,
            folder_type: row.get(1)?,
            folder_name: row.get(2)?,
            uid: row.get(3)?,
            subject: row.get(4)?,
            from_name: row.get(5)?,
            from_email: row.get(6)?,
            date: row.get(7)?,
            age_days: row.get::<_, Option<f64>>(8)?.unwrap_or(f64::MAX),
            snippet: row.get(9)?,
            is_read: row.get::<_, i32>(10)? != 0,
            is_flagged: row.get::<_, i32>(11)? != 0,
            is_muted: row.get::<_, i32>(12)? != 0,
            has_attachments: row.get::<_, i32>(13)? != 0,
            category: row.get(14)?,
            has_list_unsubscribe: row.get::<_, Option<String>>(15)?.is_some(),
            in_reply_to: row.get(16)?,
            to_list: row.get(17)?,
            cc_list: row.get(18)?,
            thread_key: row.get(19)?,
        })
    })?;

    // Rows arrive oldest-first, so "last one wins" yields the newest of each.
    let mut threads: HashMap<(String, String), Thread> = HashMap::new();
    for row in rows {
        let row = row?;
        let key = (row.account_id.clone(), row.thread_key.clone());
        let is_inbox = row.folder_type == "inbox";
        let badge_now = is_inbox.then(|| (row.folder_name.clone(), row.uid, row.subject.clone()));
        let muted_now = is_inbox && row.is_muted;

        match threads.get_mut(&key) {
            Some(thread) => {
                if let Some(b) = badge_now {
                    thread.badge = Some(b);
                }
                thread.muted |= muted_now;
                thread.newest = row;
            }
            None => {
                threads.insert(
                    key,
                    Thread {
                        badge: badge_now,
                        muted: muted_now,
                        newest: row,
                    },
                );
            }
        }
    }

    let mut nudges: Vec<Nudge> = Vec::new();
    for ((account_id, thread_key), thread) in threads {
        if thread.muted {
            continue;
        }
        let Some((badge_folder, badge_uid, badge_subject)) = thread.badge else {
            // No inbox row: the thread isn't in the list, so a badge has nothing
            // to attach to. Threads whose last send predates the In-Reply-To fix
            // land here — they root to themselves and never join their own
            // conversation (gotcha #39).
            continue;
        };

        let newest = &thread.newest;
        let days_ago = newest.age_days.floor() as i64;
        if days_ago < opts.min_age_days || days_ago > opts.max_age_days {
            continue;
        }

        let sender = newest.from_email.to_lowercase();
        let mine = corr.own_addresses.contains(&sender);

        let (kind, counterpart_email, counterpart_name) = if mine {
            // Follow-up: we wrote last. The counterpart must be someone who has
            // answered us before — outbound volume alone is what makes a cold
            // sequence look like a relationship.
            let mut recipients = extract_addresses(newest.to_list.as_deref());
            recipients.extend(extract_addresses(newest.cc_list.as_deref()));
            let Some(counterpart) = recipients
                .into_iter()
                .find(|addr| !corr.own_addresses.contains(addr) && corr.repliers.contains(addr))
            else {
                continue;
            };
            (NudgeKind::FollowUp, counterpart, None)
        } else {
            // Reply: they wrote last. `evaluate` owns the question of whether
            // this is a person worth surfacing; lane 1 is machine noise.
            if unsubscribed.contains(&(account_id.clone(), sender.clone())) {
                continue;
            }
            let replies_to_me = newest
                .in_reply_to
                .as_deref()
                .and_then(cxmail_core::mail::message_id::message_id_match_variants)
                .is_some_and(|(_, bare)| corr.sent_message_ids.contains(&bare.to_lowercase()));
            let domain = sender.split_once('@').map(|(_, d)| d).unwrap_or("");
            let candidate = Candidate {
                // Deliberately None, not an oversight. The nudge lane asks a
                // different question — did mail WE sent go unanswered — and
                // reuses this classifier only for its two-way correspondent
                // test. A verdict about an inbound message has no bearing on
                // it, and feeding one in would let triage quietly reshape a
                // lane it was never measured against.
                triage: None,
                subject: newest.subject.as_deref().unwrap_or(""),
                snippet: newest.snippet.as_deref().unwrap_or(""),
                from_email: &newest.from_email,
                category: newest.category.as_deref(),
                has_list_unsubscribe: newest.has_list_unsubscribe,
                has_attachments: newest.has_attachments,
                is_read: newest.is_read,
                is_flagged: newest.is_flagged,
                replies_to_me,
                known_sender: corr.senders.contains(&sender),
                known_domain: !domain.is_empty() && corr.domains.contains(domain),
                is_self: false,
            };
            // `evaluate` alone is not enough here. It was tuned for Needs You —
            // a queue the user opens deliberately, where a borderline row costs
            // a glance. A nudge is an unsolicited badge in the inbox list, so it
            // has to clear a higher bar: measured on the live mailbox, the
            // verdict alone admitted mortgage promos and a store-credit blast
            // from senders whose DOMAIN we had corresponded with.
            //
            // Same two-way standard as the follow-up lane, and it collapses to
            // one test: `replies_to_me` proves they answered us, which is
            // exactly what puts an address in `repliers`.
            if !corr.repliers.contains(&sender) {
                continue;
            }
            match evaluate(&candidate) {
                Some(verdict) if verdict.action.lane() == 0 => {
                    (NudgeKind::Reply, sender, newest.from_name.clone())
                }
                _ => continue,
            }
        };

        if dismissed.contains(&(
            account_id.clone(),
            thread_key.clone(),
            kind.as_str().to_string(),
        )) {
            continue;
        }

        nudges.push(Nudge {
            kind: kind.as_str().to_string(),
            account_id,
            folder_name: badge_folder,
            uid: badge_uid,
            thread_key,
            subject: badge_subject.or_else(|| newest.subject.clone()),
            counterpart_email,
            counterpart_name,
            date: newest.date.clone(),
            days_ago,
        });
    }

    // Oldest first: the thread that has been ignored longest is the one most
    // likely to be actually dropped.
    nudges.sort_by(|a, b| b.days_ago.cmp(&a.days_ago).then(a.thread_key.cmp(&b.thread_key)));
    nudges.truncate(opts.limit);
    Ok(nudges)
}

fn load_dismissals(conn: &Connection) -> Result<HashSet<(String, String, String)>, AppError> {
    let mut stmt = conn.prepare("SELECT account_id, thread_key, kind FROM nudge_dismissals")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

fn load_unsubscribed(conn: &Connection) -> Result<HashSet<(String, String)>, AppError> {
    let mut stmt =
        conn.prepare("SELECT account_id, lower(sender_email) FROM unsubscribed_senders")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Timestamps are written in the ISO-8601-with-offset shape the sync path
    /// actually stores, NOT SQLite's native space format. That distinction is
    /// the whole point: an implementation that compares these bytewise against
    /// `datetime('now')` passes with space-format fixtures and misbehaves on
    /// real mail (gotcha #23).
    fn iso_days_ago(days: i64) -> String {
        format!("strftime('%Y-%m-%dT%H:%M:%S+00:00','now','-{days} days')")
    }

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

    #[allow(clippy::too_many_arguments)]
    fn insert(
        conn: &Connection,
        folder: &str,
        uid: u32,
        from: &str,
        to: &str,
        subject: &str,
        snippet: &str,
        thread: &str,
        days: i64,
        message_id: &str,
        in_reply_to: Option<&str>,
    ) {
        conn.execute(
            &format!(
                "INSERT INTO messages
                   (account_id,folder_name,uid,from_email,to_list,subject,date,snippet,
                    thread_root_id,message_id,in_reply_to)
                 VALUES ('a',?1,?2,?3,?4,?5,{},?6,?7,?8,?9)",
                iso_days_ago(days)
            ),
            params![
                folder,
                uid,
                from,
                format!("[{{\"name\":null,\"email\":\"{to}\"}}]"),
                subject,
                snippet,
                thread,
                message_id,
                in_reply_to,
            ],
        )
        .unwrap();
    }

    /// Make `who` a proven correspondent: they once answered something we sent.
    fn record_reply_from(conn: &Connection, who: &str, uid_base: u32) {
        insert(conn, "Sent", uid_base, "me@example.com", who, "Kickoff", "hi",
               "<old@x>", 60, "<old@x>", None);
        insert(conn, "INBOX", uid_base + 1, who, "me@example.com", "Re: Kickoff", "sounds good",
               "<old@x>", 59, "<their-old@y>", Some("<old@x>"));
    }

    /// THE regression. An SDR pitches, we reply once to decline, they go quiet —
    /// and that is not a thread we are waiting on. Left unfiltered this shape
    /// nudges 299 of 317 recent sends on the real mailbox.
    #[test]
    fn a_cold_pitch_we_answered_never_becomes_a_follow_up() {
        let conn = setup();
        // Their first contact carries no In-Reply-To: it answers nothing.
        insert(&conn, "INBOX", 1, "sdr@outreach.io", "me@example.com",
               "Chris, let's develop a plan", "Would you be open to a chat?",
               "<pitch@out>", 8, "<pitch@out>", None);
        insert(&conn, "Sent", 2, "me@example.com", "sdr@outreach.io",
               "Re: Chris, let's develop a plan", "No thanks.",
               "<pitch@out>", 6, "<mine@x>", Some("<pitch@out>"));

        assert!(
            list(&conn, &NudgeOptions::default()).unwrap().is_empty(),
            "outbound volume is not evidence of a relationship"
        );
    }

    #[test]
    fn a_real_correspondent_we_are_waiting_on_nudges() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        insert(&conn, "INBOX", 1, "dana@client.com", "me@example.com",
               "Meeting Follow Up", "Let's talk Monday.", "<thread@x>", 9, "<theirs@y>", None);
        insert(&conn, "Sent", 2, "me@example.com", "dana@client.com",
               "Re: Meeting Follow Up", "Can we grab 30 minutes?",
               "<thread@x>", 5, "<mine@x>", Some("<theirs@y>"));

        let nudges = list(&conn, &NudgeOptions::default()).unwrap();
        assert_eq!(nudges.len(), 1, "got: {nudges:?}");
        assert_eq!(nudges[0].kind, "follow_up");
        assert_eq!(nudges[0].counterpart_email, "dana@client.com");
        assert_eq!(nudges[0].days_ago, 5);
        // The badge attaches to the INBOX row, which is what the list renders —
        // the Sent message has no row there.
        assert_eq!(nudges[0].folder_name, "INBOX");
        assert_eq!(nudges[0].uid, 1);
    }

    /// Nudges are a cross-account lane, so a hidden account contributes no
    /// thread — even the textbook follow-up shape above. Built in a second
    /// account (`h`) that is the exact mirror of `a_real_correspondent_we_are_waiting_on_nudges`;
    /// the un-hide at the end proves it is the flag and nothing else.
    #[test]
    fn a_hidden_account_contributes_no_nudges() {
        let conn = setup();
        conn.execute(
            "INSERT INTO accounts (id,email,provider,imap_host,smtp_host)
             VALUES ('h','warm@example.com','imap','imap.example.com','smtp.example.com')",
            [],
        )
        .unwrap();
        for (name, kind) in [("INBOX", "inbox"), ("Sent", "sent")] {
            conn.execute(
                "INSERT INTO folders (account_id,name,folder_type) VALUES ('h',?1,?2)",
                params![name, kind],
            )
            .unwrap();
        }
        let insert_h = |folder: &str, uid: u32, from: &str, to: &str, subject: &str,
                        thread: &str, days: i64, mid: &str, irt: Option<&str>| {
            conn.execute(
                &format!(
                    "INSERT INTO messages
                       (account_id,folder_name,uid,from_email,to_list,subject,date,snippet,
                        thread_root_id,message_id,in_reply_to)
                     VALUES ('h',?1,?2,?3,?4,?5,{},'…',?6,?7,?8)",
                    iso_days_ago(days)
                ),
                params![
                    folder,
                    uid,
                    from,
                    format!("[{{\"name\":null,\"email\":\"{to}\"}}]"),
                    subject,
                    thread,
                    mid,
                    irt,
                ],
            )
            .unwrap();
        };
        // A proven correspondent (they answered us once) …
        insert_h("Sent", 100, "warm@example.com", "peer@warm.test", "Kickoff", "<old@h>", 60, "<old@h>", None);
        insert_h("INBOX", 101, "peer@warm.test", "warm@example.com", "Re: Kickoff", "<old@h>", 59, "<their-old@h>", Some("<old@h>"));
        // … and a thread we are waiting on.
        insert_h("INBOX", 1, "peer@warm.test", "warm@example.com", "Meeting Follow Up", "<thread@h>", 9, "<theirs@h>", None);
        insert_h("Sent", 2, "warm@example.com", "peer@warm.test", "Re: Meeting Follow Up", "<thread@h>", 5, "<mine@h>", Some("<theirs@h>"));

        super::super::accounts::set_hidden_from_aggregates(&conn, "h", true).unwrap();
        assert!(
            list(&conn, &NudgeOptions::default()).unwrap().is_empty(),
            "a hidden account must not nudge"
        );

        super::super::accounts::set_hidden_from_aggregates(&conn, "h", false).unwrap();
        let nudges = list(&conn, &NudgeOptions::default()).unwrap();
        assert_eq!(nudges.len(), 1, "visible again: the follow-up is back — {nudges:?}");
        assert_eq!(nudges[0].account_id, "h");
    }

    #[test]
    fn a_thread_they_wrote_last_becomes_a_reply_nudge() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        insert(&conn, "Sent", 1, "me@example.com", "dana@client.com",
               "Scope", "Here it is.", "<thread@x>", 9, "<mine@x>", None);
        insert(&conn, "INBOX", 2, "dana@client.com", "me@example.com",
               "Re: Scope", "Can you confirm Friday?", "<thread@x>", 4, "<theirs@y>",
               Some("<mine@x>"));

        let nudges = list(&conn, &NudgeOptions::default()).unwrap();
        assert_eq!(nudges.len(), 1, "got: {nudges:?}");
        assert_eq!(nudges[0].kind, "reply");
        assert_eq!(nudges[0].counterpart_email, "dana@client.com");
        assert_eq!(nudges[0].days_ago, 4);
    }

    #[test]
    fn the_age_window_is_honored_at_both_ends() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        // Yesterday: too fresh to be worth mentioning.
        insert(&conn, "INBOX", 1, "dana@client.com", "me@example.com",
               "Fresh", "hi", "<fresh@x>", 2, "<f1@y>", None);
        insert(&conn, "Sent", 2, "me@example.com", "dana@client.com",
               "Re: Fresh", "replying", "<fresh@x>", 1, "<f2@x>", Some("<f1@y>"));
        // Beyond the window: dropped, not waiting.
        insert(&conn, "INBOX", 3, "dana@client.com", "me@example.com",
               "Ancient", "hi", "<old@z>", 90, "<o1@y>", None);
        insert(&conn, "Sent", 4, "me@example.com", "dana@client.com",
               "Re: Ancient", "replying", "<old@z>", 80, "<o2@x>", Some("<o1@y>"));

        assert!(list(&conn, &NudgeOptions::default()).unwrap().is_empty());
    }

    #[test]
    fn dismissing_one_lane_leaves_the_other_alone() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        insert(&conn, "INBOX", 1, "dana@client.com", "me@example.com",
               "Meeting Follow Up", "Let's talk.", "<thread@x>", 9, "<theirs@y>", None);
        insert(&conn, "Sent", 2, "me@example.com", "dana@client.com",
               "Re: Meeting Follow Up", "Can we grab 30?", "<thread@x>", 5, "<mine@x>",
               Some("<theirs@y>"));

        assert_eq!(list(&conn, &NudgeOptions::default()).unwrap().len(), 1);

        // Dismissing the other lane must not silence this one.
        dismiss(&conn, "a", "<thread@x>", "reply").unwrap();
        assert_eq!(list(&conn, &NudgeOptions::default()).unwrap().len(), 1);

        dismiss(&conn, "a", "<thread@x>", "follow_up").unwrap();
        assert!(list(&conn, &NudgeOptions::default()).unwrap().is_empty());
    }

    #[test]
    fn dismiss_rejects_an_unknown_lane() {
        let conn = setup();
        assert!(dismiss(&conn, "a", "<t@x>", "snooze").is_err());
    }

    #[test]
    fn a_muted_thread_never_nudges() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        insert(&conn, "INBOX", 1, "dana@client.com", "me@example.com",
               "Meeting Follow Up", "Let's talk.", "<thread@x>", 9, "<theirs@y>", None);
        insert(&conn, "Sent", 2, "me@example.com", "dana@client.com",
               "Re: Meeting Follow Up", "Can we grab 30?", "<thread@x>", 5, "<mine@x>",
               Some("<theirs@y>"));
        conn.execute("UPDATE messages SET is_muted = 1 WHERE folder_name='INBOX'", [])
            .unwrap();

        assert!(list(&conn, &NudgeOptions::default()).unwrap().is_empty());
    }

    /// Run the real filter over a real mailbox — the only check that can catch
    /// the failure this module exists to prevent, since no fixture reproduces
    /// the shape of 317 real sends. Point it at a COPY (the app holds the live
    /// file open, and the copy gets migrated to the current schema):
    ///
    /// ```sh
    /// cp ~/Library/Application\ Support/com.cxmail.app/cxmail.db /tmp/probe.db
    /// CXMAIL_NUDGE_PROBE_DB=/tmp/probe.db \
    ///   cargo test --lib -- db::nudges::tests::probe --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a real mailbox; set CXMAIL_NUDGE_PROBE_DB"]
    fn probe_against_a_real_mailbox() {
        let Ok(path) = std::env::var("CXMAIL_NUDGE_PROBE_DB") else {
            panic!("set CXMAIL_NUDGE_PROBE_DB to a COPY of cxmail.db");
        };
        let conn = Connection::open(&path).unwrap();
        crate::db::schema::initialize(&conn).unwrap();

        let nudges = list(&conn, &NudgeOptions::default()).unwrap();
        let follow_ups = nudges.iter().filter(|n| n.kind == "follow_up").count();
        let replies = nudges.iter().filter(|n| n.kind == "reply").count();
        println!("\n{} nudges — {follow_ups} follow-up, {replies} reply", nudges.len());
        for n in &nudges {
            println!(
                "  [{:>9}] {:>2}d  {:<32} {}",
                n.kind,
                n.days_ago,
                n.counterpart_email,
                n.subject.as_deref().unwrap_or("(no subject)")
            );
        }
        // Not an assertion about correctness — a tripwire on volume. A nudge
        // lane that fires on hundreds of threads has stopped being a signal,
        // which is precisely what the naive rule did here (299 of 317).
        assert!(
            nudges.len() < 60,
            "{} nudges is an alert storm, not a signal — check the filter",
            nudges.len()
        );
    }

    /// A thread with no inbox row has nothing in the list to badge. This is the
    /// documented consequence of threads whose last send predates the
    /// In-Reply-To fix — they root to themselves (gotcha #39).
    #[test]
    fn a_thread_with_no_inbox_row_is_skipped() {
        let conn = setup();
        record_reply_from(&conn, "dana@client.com", 100);
        insert(&conn, "Sent", 1, "me@example.com", "dana@client.com",
               "Re: Orphaned", "no parent header", "<orphan@x>", 5, "<orphan@x>", None);

        assert!(list(&conn, &NudgeOptions::default()).unwrap().is_empty());
    }
}

/// Silence one lane of one conversation, permanently.
///
/// Per-lane rather than per-thread: dismissing "follow up?" says the ball is not
/// in their court, which says nothing about whether they later write something
/// you owe an answer to.
pub fn dismiss(
    conn: &Connection,
    account_id: &str,
    thread_key: &str,
    kind: &str,
) -> Result<(), AppError> {
    if kind != NudgeKind::FollowUp.as_str() && kind != NudgeKind::Reply.as_str() {
        return Err(AppError::General(format!(
            "Unknown nudge kind {kind:?} — expected \"follow_up\" or \"reply\""
        )));
    }
    conn.execute(
        "INSERT OR REPLACE INTO nudge_dismissals (account_id, thread_key, kind, dismissed_at)
         VALUES (?1, ?2, ?3, datetime('now'))",
        params![account_id, thread_key, kind],
    )?;
    Ok(())
}
