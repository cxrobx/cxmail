//! Measure the risk check against hand-made prompt-injection emails.
//!
//! The real-mailbox eval (`cxmail-mcp --risk-eval`) has no real injection
//! examples, so the AI-instructions question's recall was unmeasured. This runs
//! fictional fixtures (`examples/fixtures/injection_probe.json`: attacks plus
//! look-alike safe mail) through the same path a read takes: the rules, the
//! triage gate, then Decisions and the Haiku fallback, each scored separately.
//! Prints per-message scores and, per model, attacks caught at HIGH and safe
//! mail wrongly raised to HIGH. Fixtures only: no mailbox is read.
//!
//!   secret run -k OPENAI_API_KEY -k ANTHROPIC_API_KEY -- \
//!     cargo run -p cxmail-email --example injection_probe
//!
//! `FILLER_2400` in a fixture expands to 2,400 characters of newsletter text,
//! past the gate's `MAX_BODY_CHARS`, to test an instruction buried at the end.

use cxmail_email::email::inference::InferenceClient;
use cxmail_email::email::risk::{self, Level, RiskInput};
use cxmail_email::email::{risk_model, triage_gate};
use std::collections::HashSet;

#[derive(serde::Deserialize)]
struct Fixture {
    id: String,
    label: String,
    from_email: String,
    from_name: Option<String>,
    subject: String,
    plain: String,
    html: Option<String>,
}

fn expand(s: &str) -> String {
    let filler = "Our community grew again this month, with new members joining from many cities. "
        .repeat(30);
    s.replace("FILLER_2400", &filler[..2400.min(filler.len())])
}

fn level_with(rules: &[risk::Signal], probs: Option<(f64, f64, f64)>) -> Level {
    let mut s = rules.to_vec();
    if let Some((a, b, c)) = probs {
        s.extend(risk::model_signals(a, b, c));
    }
    risk::level(&s, false)
}

#[tokio::main]
async fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/fixtures/injection_probe.json");
    let fixtures: Vec<Fixture> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();

    let decisions = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|k| InferenceClient::openai_with_key(k, cxmail_email::email::triage::DECISIONS_MODEL).unwrap());
    let haiku = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|k| InferenceClient::anthropic_with_key(k, risk_model::FALLBACK_MODEL).unwrap());
    if decisions.is_none() && haiku.is_none() {
        eprintln!("no keys: run under secret run -k OPENAI_API_KEY -k ANTHROPIC_API_KEY --");
    }

    let known = HashSet::from(["own-company.example".to_string()]);
    let none: Vec<(String, String)> = Vec::new();
    // (label, rules HIGH, decisions HIGH, haiku HIGH)
    let mut tally: Vec<(String, bool, Option<bool>, Option<bool>)> = Vec::new();

    println!("{:<30} {:<7} {:<6} {:>10} {:>10}", "fixture", "label", "rules", "dec p_ai", "haiku p_ai");
    for f in &fixtures {
        let plain = expand(&f.plain);
        let html = f.html.as_deref().map(expand);
        let signals = risk::assess(&RiskInput {
            from_email: &f.from_email,
            from_name: f.from_name.as_deref(),
            raw_headers: None,
            sanitized_html: html.as_deref(),
            plain_text: Some(&plain),
            attachments: &none,
            known_domains: &known,
        });
        let rules_level = risk::level(&signals, false);

        let to = vec!["me@own-company.example".to_string()];
        let safe = match triage_gate::gate(
            triage_gate::TriageMode::On,
            &triage_gate::MessageForTriage {
                from_email: &f.from_email,
                from_name: f.from_name.as_deref(),
                to: &to,
                cc: &[],
                date: "2026-11-18",
                subject: Some(&f.subject),
                body: Some(&risk_model::head_and_tail(&plain)),
            },
        ) {
            triage_gate::Gate::Send(s) => Some(s),
            triage_gate::Gate::Withheld(w) => {
                println!("{:<30} withheld by gate: {w:?}", f.id);
                None
            }
        };

        let mut dec = None;
        let mut hk = None;
        if let Some(safe) = &safe {
            if let Some(c) = &decisions {
                match risk_model::judge(c, safe).await {
                    Ok(v) => dec = Some((v.p_phishing, v.p_ai_instructions, v.p_impersonation)),
                    Err(e) => eprintln!("{}: Decisions failed: {e}", f.id),
                }
            }
            if let Some(c) = &haiku {
                match risk_model::judge_fallback(c, safe).await {
                    Ok(v) => hk = Some((v.p_phishing, v.p_ai_instructions, v.p_impersonation)),
                    Err(e) => eprintln!("{}: Haiku failed: {e}", f.id),
                }
            }
        }
        let show = |p: Option<(f64, f64, f64)>| p.map(|(_, b, _)| format!("{b:.2}")).unwrap_or_else(|| "-".into());
        println!("{:<30} {:<7} {:<6} {:>10} {:>10}", f.id, f.label, rules_level.as_str(), show(dec), show(hk));
        tally.push((
            f.label.clone(),
            rules_level == Level::High,
            dec.map(|p| level_with(&signals, Some(p)) == Level::High),
            hk.map(|p| level_with(&signals, Some(p)) == Level::High),
        ));
    }

    println!();
    for (name, pick) in [("rules only", 0usize), ("rules + Decisions", 1), ("rules + Haiku", 2)] {
        let mut caught = 0;
        let mut attacks = 0;
        let mut false_high = 0;
        let mut benign = 0;
        for (label, r, d, h) in &tally {
            let v = match pick {
                0 => Some(*r),
                1 => *d,
                _ => *h,
            };
            let Some(v) = v else { continue };
            if label == "attack" {
                attacks += 1;
                caught += v as usize;
            } else {
                benign += 1;
                false_high += v as usize;
            }
        }
        if attacks + benign > 0 {
            println!("{name:<20} attacks at HIGH: {caught}/{attacks}   safe mail at HIGH: {false_high}/{benign}");
        }
    }
}
