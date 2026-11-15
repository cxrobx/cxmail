//! Calibrate `email::risk` against a copy of a real mailbox.
//!
//! Runs the deterministic risk check over the newest N inbox messages that
//! have a cached body and prints only aggregates: the level distribution, how
//! often each signal fired, and, per level, the signal codes of a few
//! examples by uid. Never prints message text, subjects or addresses.
//!
//!   cargo run -p cxmail-email --example risk_probe -- <copy.db> [limit]
//!
//! Point it at a `.backup` copy, never the live file.

use cxmail_email::email::risk::{self, RiskInput};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: risk_probe <copy.db> [limit]");
    let limit: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

    let t = Instant::now();
    let trust = cxmail_email::db::needs_you::sender_trust(&conn).unwrap();
    println!(
        "sender_trust: {} ms, {} known domains, {} established addresses",
        t.elapsed().as_millis(),
        trust.known_domains.len(),
        trust.established.len()
    );

    let mut stmt = conn
        .prepare(
            "SELECT m.account_id, m.folder_name, m.uid, coalesce(m.from_email,''), m.from_name,
                    b.sanitized_html, b.plain_text, h.raw_headers
               FROM messages m
               JOIN message_bodies b USING (account_id, folder_name, uid)
               LEFT JOIN message_headers h USING (account_id, folder_name, uid)
              WHERE m.folder_name = 'INBOX'
              ORDER BY m.date DESC LIMIT ?1",
        )
        .unwrap();
    let rows: Vec<(String, String, u32, String, Option<String>, Option<String>, Option<String>, Option<String>)> = stmt
        .query_map([limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let mut att_stmt = conn
        .prepare("SELECT coalesce(filename,''), content_type FROM attachments WHERE account_id=?1 AND folder_name=?2 AND message_uid=?3 AND is_inline=0")
        .unwrap();
    let mut levels: BTreeMap<&str, usize> = BTreeMap::new();
    let mut signals: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    // Details of the identity and link signals (domains only) to tune rules.
    let mut details: BTreeMap<String, usize> = BTreeMap::new();
    let (mut with_headers, mut unestablished) = (0, 0);
    let t = Instant::now();
    for (acct, folder, uid, from, name, html, plain, headers) in &rows {
        let atts: Vec<(String, String)> = att_stmt
            .query_map(rusqlite::params![acct, folder, uid], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        with_headers += headers.is_some() as usize;
        let first = trust.is_unestablished(from);
        unestablished += first as usize;
        let s = risk::assess(&RiskInput {
            from_email: from,
            from_name: name.as_deref(),
            raw_headers: headers.as_deref(),
            sanitized_html: html.as_deref(),
            plain_text: plain.as_deref(),
            attachments: &atts,
            known_domains: &trust.known_domains,
        });
        let lvl = risk::level(&s, first).as_str();
        *levels.entry(lvl).or_default() += 1;
        for x in &s {
            *signals.entry(format!("{:?}:{}", x.severity, x.code)).or_default() += 1;
            if x.code == "ai_instructions" {
                if let Some(m) = std::env::var("RISK_PROBE_SHOW_MATCH").ok().and(plain.as_deref()).and_then(risk::instruction_match) {
                    println!("match uid {uid}: {m:?}");
                }
            }
            if ["link_mismatch", "lookalike_domain", "display_name_spoof", "ai_instructions", "risky_attachment"].contains(&x.code.as_str()) {
                *details.entry(format!("{}: {}", x.code, x.detail)).or_default() += 1;
            }
        }
        let ex = examples.entry(lvl).or_default();
        if ex.len() < 8 && lvl != "low" {
            let mut c: Vec<&str> = s.iter().filter(|x| x.severity != risk::Severity::Info).map(|x| x.code.as_str()).collect();
            c.dedup();
            ex.push(format!("uid {uid}: {}", c.join(",")));
        }
    }
    let n = rows.len().max(1);
    println!(
        "{} messages in {} ms ({:.2} ms each); headers cached for {}; unestablished senders {}",
        rows.len(),
        t.elapsed().as_millis(),
        t.elapsed().as_secs_f64() * 1000.0 / n as f64,
        with_headers,
        unestablished
    );
    for (l, c) in &levels {
        println!("level {l:<6} {c:>6}  ({:.1}%)", *c as f64 * 100.0 / n as f64);
    }
    let mut sv: Vec<_> = signals.into_iter().collect();
    sv.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, c) in sv {
        println!("signal {k:<32} {c:>6}");
    }
    let mut dv: Vec<_> = details.into_iter().collect();
    dv.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, c) in dv.iter().take(60) {
        println!("detail {c:>4}  {k}");
    }
    for (l, ex) in examples {
        println!("examples {l}: {}", ex.join(" | "));
    }
}
