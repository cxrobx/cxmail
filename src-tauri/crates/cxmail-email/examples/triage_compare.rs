//! Triage A/B: the production Luna path vs OpenAI's Decisions API.
//!
//! Three arms over the same gated payloads (`triage_gate::gate`, so nothing
//! leaves that production would not send):
//!   - `luna-medium`: `triage::classify` on the prompted path (the pre-v2 default)
//!   - `luna-none`:   the same call with reasoning off, the free tuning to beat
//!   - `decisions`:   `triage::classify(.., "decisions")`, the production default since v2
//!
//! Ground truth for `needs_response` is behaviour the model never sees: did the
//! recipient reply in that thread from this account. Everything else is scored
//! as agreement with `luna-medium`.
//!
//! Reads the mail DB read-only and writes only scores and ids, never message
//! text, to `--out`.
//!
//!   OPENAI_API_KEY=… cargo run -p cxmail-email --example triage_compare -- \
//!     --account <address> [--limit 500] [--concurrency 6] [--out triage-compare.json]
//!   (run it under `secret run -k OPENAI_API_KEY --`)

use std::sync::Arc;
use std::time::Instant;

use cxmail_email::email::inference::InferenceClient;
use cxmail_email::email::triage;
use cxmail_email::email::triage_gate::{self, Gate, MessageForTriage, SafeMessage, TriageMode};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

const LUNA_IN: f64 = 0.10;
const LUNA_OUT: f64 = 0.50;

fn arg(name: &str, fallback: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| fallback.to_string())
}

struct Row {
    id: String,
    replied: bool,
    safe: SafeMessage,
}

fn load(account: &str, limit: usize) -> Vec<Row> {
    let home = std::env::var("HOME").expect("HOME");
    let path = format!("{home}/Library/Application Support/com.cxmail.app/cxmail.db");
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open db");
    let mut stmt = conn
        .prepare(
            "WITH acct AS (SELECT id FROM accounts WHERE email = ?1)
             SELECT m.folder_name || ':' || m.uid, m.from_email, m.from_name, m.subject,
                    m.to_list, m.cc_list, m.date, b.plain_text,
                    EXISTS (SELECT 1 FROM messages s
                              JOIN folders sf ON sf.account_id = s.account_id AND sf.name = s.folder_name
                                             AND sf.folder_type = 'sent'
                             WHERE s.account_id = m.account_id AND m.message_id IS NOT NULL
                               AND (s.in_reply_to = m.message_id
                                    OR instr(coalesce(s.reference_ids, ''), m.message_id) > 0))
               FROM messages m
               JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
                             AND f.folder_type = 'inbox'
               JOIN message_bodies b ON b.account_id = m.account_id
                                    AND b.folder_name = m.folder_name AND b.uid = m.uid
              WHERE m.account_id = (SELECT id FROM acct)
                AND b.plain_text IS NOT NULL AND trim(b.plain_text) <> ''
              ORDER BY m.date DESC
              LIMIT ?2",
        )
        .expect("prepare");
    let raw: Vec<(String, String, Option<String>, Option<String>, Option<String>, Option<String>, String, String, bool)> = stmt
        .query_map(rusqlite::params![account, limit as i64], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");

    let mut withheld = 0;
    let rows: Vec<Row> = raw
        .into_iter()
        .filter_map(|(id, from_email, from_name, subject, to, cc, date, body, replied)| {
            let to = parse_addrs(to.as_deref());
            let cc = parse_addrs(cc.as_deref());
            match triage_gate::gate(
                TriageMode::Shadow,
                &MessageForTriage {
                    from_email: &from_email,
                    from_name: from_name.as_deref(),
                    to: &to,
                    cc: &cc,
                    date: &date,
                    subject: subject.as_deref(),
                    body: Some(&body),
                },
            ) {
                Gate::Send(safe) => Some(Row { id, replied, safe: *safe }),
                Gate::Withheld(_) => {
                    withheld += 1;
                    None
                }
            }
        })
        .collect();
    eprintln!("loaded {} sendable messages ({withheld} withheld by the gate)", rows.len());
    rows
}

/// The same parsing the triage command uses for stored address lists.
fn parse_addrs(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return vec![] };
    if let Ok(v) = serde_json::from_str::<Vec<String>>(raw) {
        return v;
    }
    raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

async fn ask(client: &InferenceClient, safe: &SafeMessage, model: &str, effort: &str) -> Value {
    let started = Instant::now();
    let r = triage::classify(client, safe, model, effort).await;
    let ms = started.elapsed().as_millis() as u64;
    match r {
        Ok(c) => json!({
            "needs_response": c.verdict.needs_response,
            "needs_action": c.verdict.needs_action,
            "category": c.verdict.category,
            "urgency": c.verdict.urgency,
            "input_tokens": c.input_tokens,
            "output_tokens": c.output_tokens,
            "ms": ms,
        }),
        Err(e) => json!({ "error": e.to_string(), "ms": ms }),
    }
}

#[tokio::main]
async fn main() {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        eprintln!("OPENAI_API_KEY is not set (run under: secret run -k OPENAI_API_KEY -- …)");
        std::process::exit(2);
    }
    let account = arg("account", "");
    if account.is_empty() {
        eprintln!("--account <address> is required");
        std::process::exit(2);
    }
    let limit: usize = arg("limit", "500").parse().unwrap();
    let concurrency: usize = arg("concurrency", "6").parse().unwrap();
    let none_effort = arg("none-effort", "none");
    let out = arg("out", "triage-compare.json");

    let rows = load(&account, limit);
    let luna = Arc::new(InferenceClient::openai_with_key(key.clone(), "gpt-6-luna").unwrap());
    let gate = Arc::new(Semaphore::new(concurrency));

    let mut tasks = Vec::new();
    for (i, row) in rows.into_iter().enumerate() {
        let (luna, gate, none_effort) = (luna.clone(), gate.clone(), none_effort.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = gate.acquire_owned().await.unwrap();
            // Rotate the order so no arm is always measured first.
            let (mut medium, mut none, mut dec) = (Value::Null, Value::Null, Value::Null);
            for step in 0..3 {
                match (i + step) % 3 {
                    0 => medium = ask(&luna, &row.safe, "gpt-6-luna", "medium").await,
                    1 => none = ask(&luna, &row.safe, "gpt-6-luna", &none_effort).await,
                    _ => dec = ask(&luna, &row.safe, triage::DECISIONS_ENGINE, "").await,
                }
            }
            if (i + 1) % 50 == 0 {
                eprintln!("  {}", i + 1);
            }
            json!({ "id": row.id, "replied": row.replied, "luna_medium": medium, "luna_none": none, "decisions": dec })
        }));
    }
    let mut results = Vec::new();
    for t in tasks {
        results.push(t.await.unwrap());
    }

    let rates = json!({ "luna_in": LUNA_IN, "luna_out": LUNA_OUT, "decisions_in": 0.10 });
    std::fs::write(&out, serde_json::to_string_pretty(&json!({ "account": account, "rates": rates, "rows": results })).unwrap()).unwrap();
    eprintln!("wrote {out}");
}
