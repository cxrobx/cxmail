//! Deterministic "does this actually need me?" classifier.
//!
//! Pure functions only — no DB, no network, no inference provider. The queue is
//! built entirely from local metadata so opening Needs You never sends message
//! data anywhere.
//!
//! The rules here were tuned against a live 1,537-message / 21-day corpus. The
//! headline numbers from that run, for anyone retuning: the old subject+snippet
//! LIKE sweep kept 518 messages (34% of the inbox); these rules keep ~90, which
//! collapse to ~21 rows. Recall was checked in the other direction too — the
//! only messages dropped from established correspondents were older siblings of
//! threads already represented in the queue.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Decision,
    Review,
    FollowUp,
    Reply,
    Alert,
}

impl ActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionType::Decision => "decision",
            ActionType::Review => "review",
            ActionType::FollowUp => "follow_up",
            ActionType::Reply => "reply",
            ActionType::Alert => "alert",
        }
    }

    /// People rank above machines. Used as the primary sort key so an alert
    /// storm can never bury a client's question.
    pub fn lane(&self) -> u8 {
        match self {
            ActionType::Alert => 1,
            _ => 0,
        }
    }
}

/// Everything the classifier is allowed to look at.
pub struct Candidate<'a> {
    pub subject: &'a str,
    pub snippet: &'a str,
    pub from_email: &'a str,
    pub category: Option<&'a str>,
    pub has_list_unsubscribe: bool,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_flagged: bool,
    /// `In-Reply-To` matches the Message-ID of something we sent. The only
    /// proof that a thread is real.
    pub replies_to_me: bool,
    /// We've written to this exact address at least twice.
    pub known_sender: bool,
    /// We've written to this domain at least twice.
    pub known_domain: bool,
    /// Sender is one of our own addresses.
    pub is_self: bool,
    /// The stored AI verdict for this message, when triage is in `on` mode.
    ///
    /// `None` in `off` and `shadow`, which is what keeps this function pure and
    /// keeps "opening Needs You never sends message data anywhere" literally
    /// true: the verdict was written by a background pass long before, and this
    /// is a table read like every other field here.
    pub triage: Option<&'a crate::mail::triage::TriageVerdict>,
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub action: ActionType,
    pub reason: String,
    /// The text that actually triggered the match, so the UI can show evidence
    /// instead of a canned sentence.
    pub evidence: Option<String>,
    pub score: f32,
}

// --- vocabularies ---------------------------------------------------------

/// Automation tokens. Matched anywhere in the local part with a separator
/// boundary — `googleads-noreply@` and `CloudPlatform-noreply@` are the common
/// shapes, and a start-anchored match misses both.
const AUTOMATED_TOKENS: &[&str] = &[
    "noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "mailer-daemon",
    "postmaster", "bounce", "notification", "notifications", "automated", "auto-confirm",
    "order-update", "shipment-tracking", "newsletter", "digest", "billing", "invoice",
    "invoices", "statement", "statements", "receipt", "receipts", "update", "updates",
    "marketing", "promo", "promotions", "deals", "offers", "tickets", "events",
];

/// Whole local parts that are automated even though they carry no token.
const AUTOMATED_EXACT: &[&str] = &["sns", "alert", "alerts", "nobody", "mailer", "bot"];

const BULK_DOMAINS: &[&str] = &[
    "mailchimp.com", "sendgrid.net", "hubspot.com", "klaviyo.com", "substack.com",
    "beehiiv.com", "convertkit.com", "mailerlite.com", "amazonses.com", "mailgun.com",
    "sparkpostmail.com", "linkedin.com", "facebook.com", "facebookmail.com",
    "instagram.com", "twitter.com", "x.com", "eventbrite.com",
];

/// An obligation placed on the reader. Strong enough to qualify a stranger.
const OBLIGATION: &[&str] = &[
    "action required", "approval", "approve", "sign off", "signature required",
    "please sign", "countersign", "contract", "invoice", "past due", "overdue",
    "payment failed", "expires", "expiring", "rsvp", "deadline", "due by", "due on",
    "respond by", "reply by", "confirm by", "final notice", "needs your", "need your",
    "awaiting your",
];

/// A direct ask. Qualifies someone we already correspond with.
const ASK: &[&str] = &[
    "can you", "could you", "would you", "will you", "are you able", "please review",
    "please confirm", "please send", "please advise", "please let me know",
    "let me know", "thoughts?", "your thoughts", "any update", "any updates",
    "circling back", "following up", "follow up", "checking in", "gentle reminder",
    "just a reminder", "waiting on", "waiting for", "when can", "when will",
    "does that work", "works for you", "your availability",
];

/// Cold-pitch tells. A hard veto unless the sender is replying to us.
const COLD: &[&str] = &[
    "hope this email finds you", "hope you're doing well", "hope you are doing well",
    "quick question", "i can offer", "we help", "book a call", "15 minutes",
    "schedule a call", "interested in learning", "let's connect", "lets connect",
    "our services", "free trial", "case study", "boost your", "grow your",
    "increase your", "have you tried", "have you neglected", "let's eliminate",
    "simplify the process", "appreciate your patience", "are you tired of",
    "no longer wish",
];

/// Footer and preheader boilerplate. A `?` here is never aimed at the reader —
/// this is what dragged every receipt with a "Questions?" footer into the queue.
const BOILERPLATE_QUESTION: &[&str] = &[
    "questions?", "any questions", "have questions", "need help", "need assistance",
    "why am i receiving", "not you?", "wasn't you", "was this you", "trouble viewing",
    "view in browser", "unsubscribe", "manage preferences", "forgot your password",
    "didn't request", "did you know", "what's new", "want more",
];

/// It already happened. Nobody is waiting on you.
const TRANSACTIONAL: &[&str] = &[
    "payment successful", "has been paid", "was paid", "payment received",
    "your receipt", "receipt from", "invoice paid", "order confirmed",
    "officially confirmed", "has shipped", "out for delivery", "delivered:",
    "shipped:", "ordered:", "successfully completed", "thanks for your payment",
];

/// Operational failures worth surfacing — the user opted to keep these.
const OPS_FAILURE: &[&str] = &[
    "stopped unexpectedly", "has stopped", "is down", "went down", "unreachable",
    "failed", "failure", "error", "crashed", "restarted unexpectedly", "disk full",
    "out of space", "degraded", "critical", "offline", "certificate expir",
    "cert expir", "quota exceeded", "payment failed", "card declined", "build failed",
    "deploy failed", "backup failed",
];

const OPS_SENDER_HINTS: &[&str] = &[
    "synologynotification", "sentry", "datadog", "pagerduty", "uptimerobot",
    "statuspage", "github", "gitlab", "vercel", "netlify", "cloudflare", "render",
    "fly.io", "railway", "stripe", "digitalocean", "hetzner", "betterstack",
    "healthchecks", "mailer-daemon",
];

/// Routine reports that mention failure words without being an incident.
const ROUTINE_REPORT: &[&str] = &[
    "report domain:", "dmarc", "weekly summary", "monthly report", "weekly status",
    "digest", "delivered:", "shipped:", "ordered:", "out for delivery",
];

const QUESTION_OPENERS: &[&str] = &[
    "who", "what", "when", "where", "why", "how", "which", "can", "could", "would",
    "will", "should", "do", "does", "did", "are", "is", "was", "have", "has", "any",
];

// --- helpers --------------------------------------------------------------

fn first_match<'a>(haystack: &str, needles: &[&'a str]) -> Option<&'a str> {
    needles.iter().copied().find(|n| haystack.contains(n))
}

fn separator(c: char) -> bool {
    matches!(c, '.' | '-' | '_' | '+')
}

/// Token match with separator boundaries, so `noreply`, `googleads-noreply` and
/// `noreply.billing` all hit while `noreplyfoo` does not.
fn has_automation_token(local: &str) -> bool {
    if AUTOMATED_EXACT.contains(&local) {
        return true;
    }
    AUTOMATED_TOKENS.iter().any(|tok| {
        local.match_indices(tok).any(|(idx, _)| {
            let before_ok = idx == 0 || local[..idx].chars().next_back().is_some_and(separator);
            let after = idx + tok.len();
            let after_ok = after == local.len()
                || local[after..].chars().next().is_some_and(separator);
            before_ok && after_ok
        })
    })
}

fn base_domain(domain: &str) -> &str {
    // mail.eventbrite.com -> eventbrite.com (second-to-last dot)
    match domain.rmatch_indices('.').nth(1) {
        Some((idx, _)) => &domain[idx + 1..],
        None => domain,
    }
}

/// The sentence a keyword appeared in — better evidence than the keyword alone.
fn sentence_containing(text: &str, needle: &str) -> Option<String> {
    sentences(text)
        .into_iter()
        .find(|s| s.to_lowercase().contains(needle))
        .map(|s| truncate(s, 120))
}

fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        // A newline always ends a clause — the subject and the snippet are
        // separate thoughts even when the subject has no terminator. For real
        // punctuation, require trailing space so "v1.2" isn't split.
        let is_break = c == '\n'
            || (matches!(c, '.' | '!' | '?')
                && bytes
                    .get(i + c.len_utf8())
                    .is_none_or(|b| b.is_ascii_whitespace()));
        if is_break {
            let end = i + c.len_utf8();
            let s = text[start..end].trim();
            if !s.is_empty() {
                out.push(s);
            }
            start = end;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// A `?` terminating a clause that is actually aimed at the reader.
fn real_question(text: &str) -> Option<String> {
    for s in sentences(text) {
        if !s.contains('?') {
            continue;
        }
        let low = s.to_lowercase();
        if BOILERPLATE_QUESTION.iter().any(|b| low.contains(b)) {
            continue;
        }
        let addresses_reader = low
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .any(|w| matches!(w, "you" | "your" | "yours" | "u"));
        let opens_question = low
            .split_whitespace()
            .next()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .is_some_and(|w| QUESTION_OPENERS.contains(&w));
        if addresses_reader || opens_question {
            return Some(truncate(s, 120));
        }
    }
    None
}

/// A "FirstName, ..." subject opener — the signature shape of an automated
/// outreach sequence.
fn is_name_pitch(subject: &str) -> bool {
    let stripped = strip_reply_prefix(subject);
    let mut chars = stripped.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    let Some(comma) = stripped.find(',') else {
        return false;
    };
    let name = &stripped[..comma];
    // A single capitalized word followed by a comma and more text.
    !name.is_empty()
        && name.chars().all(|c| c.is_alphabetic())
        && name.chars().skip(1).all(|c| c.is_lowercase())
        && stripped[comma + 1..].trim().len() > 1
}

pub fn strip_reply_prefix(subject: &str) -> &str {
    let mut s = subject.trim();
    loop {
        let low = s.to_ascii_lowercase();
        let matched = ["re:", "fwd:", "fw:", "re :", "fwd :"]
            .iter()
            .find(|p| low.starts_with(**p))
            .copied();
        match matched {
            Some(p) => s = s[p.len()..].trim_start(),
            None => return s,
        }
    }
}

// --- the classifier -------------------------------------------------------

/// Returns `None` when the message does not belong in the queue.
pub fn evaluate(c: &Candidate) -> Option<Verdict> {
    if c.is_self {
        return None;
    }
    let subject = c.subject.trim();
    let blob = format!("{}\n{}", subject, c.snippet.trim());
    let low = blob.to_lowercase();
    let sender = c.from_email.to_lowercase();
    let (local, domain) = sender.split_once('@').unwrap_or((sender.as_str(), ""));

    // --- lane A: operational alerts ---------------------------------------
    let automated = has_automation_token(local);
    let ops_sender = OPS_SENDER_HINTS.iter().any(|h| sender.contains(h));
    if ops_sender || automated {
        let routine = first_match(&low, ROUTINE_REPORT).is_some();
        if let Some(hit) = first_match(&low, OPS_FAILURE) {
            if !routine {
                return Some(Verdict {
                    action: ActionType::Alert,
                    reason: "System alert reporting a failure".to_string(),
                    evidence: Some(truncate(if subject.is_empty() { hit } else { subject }, 120)),
                    score: 4.0,
                });
            }
        }
        // Automated sender with nothing broken: not your problem.
        if !c.known_sender {
            return None;
        }
    }

    // --- hard excludes ----------------------------------------------------
    if matches!(c.category, Some("junk") | Some("promotions") | Some("social")) {
        return None;
    }
    if c.has_list_unsubscribe && !c.known_sender {
        return None;
    }
    if BULK_DOMAINS.contains(&base_domain(domain)) {
        return None;
    }
    if first_match(&low, TRANSACTIONAL).is_some() {
        return None;
    }

    // --- lane B: a human asking something ---------------------------------
    let obligation = first_match(&low, OBLIGATION);
    let ask = first_match(&low, ASK);
    let question = real_question(&blob);

    // The model gets a vote HERE and nowhere earlier, which is the whole design
    // of the integration. Everything above this line is a veto — bulk mail,
    // transactional receipts, senders we have never written to — and those
    // stay the user's rules, not the model's to overturn. What the model is
    // allowed to do is notice a request the word lists missed, which is exactly
    // what a word list is bad at and a reader is good at.
    //
    // ADD-ONLY: a verdict can put a message INTO the queue and can never take
    // one out. A false positive costs a glance; a false negative costs a client
    // question nobody sees, and that asymmetry is the reason for the direction.
    let ai_says_reply = c.triage.is_some_and(|t| t.says_needs_response());
    if obligation.is_none() && ask.is_none() && question.is_none() && !ai_says_reply {
        return None;
    }

    // Cold-pitch language is a hard veto. Note that a `Re:` subject or a
    // populated In-Reply-To confers NO trust on its own — outreach sequences
    // forge both to look like an existing thread. Only `replies_to_me` (their
    // In-Reply-To matches a Message-ID we sent) proves the thread is real.
    if !c.replies_to_me && (first_match(&low, COLD).is_some() || is_name_pitch(subject)) {
        return None;
    }

    let trusted = c.known_sender || c.known_domain || c.replies_to_me;
    if !trusted && obligation.is_none() {
        // A stranger qualifies on an explicit obligation, never a bare "?".
        return None;
    }

    let decision_words = [
        "approve", "approval", "sign off", "please sign", "decision", "countersign",
        "signature required",
    ];
    let followup_words = [
        "following up", "follow up", "circling back", "checking in", "reminder",
        "any update", "waiting on", "waiting for",
    ];
    let review_words = ["review", "attached", "draft", "proposal", "contract", "document", "deck"];

    let (action, reason, evidence) = if obligation.is_some()
        && decision_words.iter().any(|w| low.contains(w))
    {
        (
            ActionType::Decision,
            "Asked you to approve or sign off",
            obligation
                .and_then(|o| sentence_containing(&blob, o))
                .or_else(|| Some(truncate(subject, 120))),
        )
    } else if c.has_attachments && review_words.iter().any(|w| low.contains(w)) {
        (
            ActionType::Review,
            "Sent something for you to review",
            Some(truncate(subject, 120)),
        )
    } else if followup_words.iter().any(|w| low.contains(w)) {
        (
            ActionType::FollowUp,
            "Following up with you",
            ask.or(obligation)
                .and_then(|s| sentence_containing(&blob, s))
                .or_else(|| Some(truncate(subject, 120))),
        )
    } else if obligation.is_none() && ask.is_none() && question.is_none() {
        // Reached only when the gate above let this through on the verdict
        // alone. It gets its own reason rather than falling into "Asked you a
        // direct question", which would be a claim about the text that nothing
        // in the text supports.
        let urgency = c.triage.map(|t| t.urgency_label()).unwrap_or("whenever");
        (
            ActionType::Reply,
            "AI: someone is waiting on your reply",
            Some(truncate(&format!("{subject} — {urgency}"), 120)),
        )
    } else {
        (
            ActionType::Reply,
            "Asked you a direct question",
            question.clone().or_else(|| ask.map(|s| s.to_string())),
        )
    };

    let mut score = 1.0;
    if c.replies_to_me {
        score += 3.0;
    }
    if c.known_sender {
        score += 2.0;
    } else if c.known_domain {
        score += 1.0;
    }
    if obligation.is_some() {
        score += 1.5;
    }
    if !c.is_read {
        score += 1.0;
    }
    if c.is_flagged {
        score += 1.0;
    }

    Some(Verdict {
        action,
        reason: reason.to_string(),
        evidence,
        score,
    })
}

/// Collapse key for de-duplication. Repeated alerts about the same container
/// and every message in one conversation must occupy a single row.
pub fn collapse_key(
    action: ActionType,
    from_email: &str,
    subject: &str,
    account_id: &str,
    thread_root_id: Option<&str>,
) -> String {
    let subj = strip_reply_prefix(subject).to_lowercase();
    match action {
        ActionType::Alert => {
            // Strip ids/timestamps so "Container finance_api ... 12:04" and the
            // same alert an hour later share a key.
            let normalized: String = subj
                .split_whitespace()
                .map(|w| {
                    if w.chars().any(|c| c.is_ascii_digit()) {
                        "#".to_string()
                    } else {
                        w.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("alert|{}|{}", from_email.to_lowercase(), normalized)
        }
        _ => match thread_root_id {
            Some(root) if !root.is_empty() => format!("thread|{}|{}", account_id, root),
            _ => format!("subject|{}|{}", from_email.to_lowercase(), subj),
        },
    }
}

#[cfg(test)]
mod triage_integration_tests {
    use super::*;
    use crate::mail::triage::{TriageVerdict, NEEDS_RESPONSE_THRESHOLD};

    fn verdict(needs_response: f32) -> TriageVerdict {
        TriageVerdict {
            needs_response,
            needs_action: 0.0,
            category: "primary".into(),
            urgency: 2,
            model: "test".into(),
        }
    }

    /// A known correspondent writing prose the word lists do not match.
    fn quiet_ask<'a>(triage: Option<&'a TriageVerdict>) -> Candidate<'a> {
        Candidate {
            subject: "Thursday",
            snippet: "The venue needs the final headcount before they will hold the room.",
            from_email: "dana@northwind.example",
            category: Some("primary"),
            has_list_unsubscribe: false,
            has_attachments: false,
            is_read: false,
            is_flagged: false,
            replies_to_me: true,
            known_sender: true,
            known_domain: true,
            is_self: false,
            triage,
        }
    }

    #[test]
    fn without_a_verdict_the_queue_is_exactly_what_it_was() {
        // Shadow mode passes None, so every existing row must be unchanged.
        assert!(evaluate(&quiet_ask(None)).is_none());
    }

    #[test]
    fn a_verdict_can_add_a_message_the_word_lists_missed() {
        let v = verdict(0.9);
        let got = evaluate(&quiet_ask(Some(&v))).expect("should be queued");
        assert_eq!(got.action, ActionType::Reply);
        assert!(got.reason.starts_with("AI:"), "reason was {:?}", got.reason);
        // It must not borrow a wording that claims something about the text.
        assert!(!got.reason.contains("direct question"));
    }

    #[test]
    fn a_verdict_below_the_threshold_adds_nothing() {
        let v = verdict(NEEDS_RESPONSE_THRESHOLD - 0.01);
        assert!(evaluate(&quiet_ask(Some(&v))).is_none());
    }

    #[test]
    fn a_verdict_cannot_overturn_the_bulk_veto() {
        // The model is allowed to notice a missed request. It is not allowed to
        // reopen a rejection the user's own rules made.
        let v = verdict(0.99);
        let mut c = quiet_ask(Some(&v));
        c.has_list_unsubscribe = true;
        c.known_sender = false;
        assert!(evaluate(&c).is_none(), "bulk veto was overturned");
    }

    #[test]
    fn a_verdict_cannot_overturn_the_cold_pitch_veto() {
        let v = verdict(0.99);
        let mut c = quiet_ask(Some(&v));
        c.replies_to_me = false;
        c.snippet = "Hope this email finds you well. We help founders book a call.";
        assert!(evaluate(&c).is_none(), "cold-pitch veto was overturned");
    }

    #[test]
    fn a_verdict_does_not_promote_a_stranger_without_an_obligation() {
        let v = verdict(0.99);
        let mut c = quiet_ask(Some(&v));
        c.known_sender = false;
        c.known_domain = false;
        c.replies_to_me = false;
        assert!(evaluate(&c).is_none(), "stranger rule was overturned");
    }

    #[test]
    fn a_verdict_never_removes_a_row_the_local_rules_found() {
        // Add-only in the other direction: a confident "no reply needed" must
        // not silence a message the word lists matched on their own.
        let v = verdict(0.0);
        let mut c = quiet_ask(Some(&v));
        c.snippet = "Can you confirm the headcount by Thursday?";
        let got = evaluate(&c).expect("local rules should still queue this");
        assert!(!got.reason.starts_with("AI:"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate<'a>(subject: &'a str, snippet: &'a str, from: &'a str) -> Candidate<'a> {
        Candidate {
            triage: None,
            subject,
            snippet,
            from_email: from,
            category: None,
            has_list_unsubscribe: false,
            has_attachments: false,
            is_read: false,
            is_flagged: false,
            replies_to_me: false,
            known_sender: false,
            known_domain: false,
            is_self: false,
        }
    }

    #[test]
    fn keeps_a_direct_question_from_someone_we_correspond_with() {
        let mut c = candidate(
            "Re: Next Steps",
            "Can you send over the contract this week?",
            "angela@example.com",
        );
        c.known_sender = true;
        let v = evaluate(&c).expect("should qualify");
        assert_eq!(v.action, ActionType::Reply);
        assert!(v.evidence.is_some(), "must carry the matched text as evidence");
    }

    #[test]
    fn drops_cold_outreach_that_forges_a_reply_thread() {
        // The dominant false-positive class: a sales sequence with a faked
        // "Re:" subject. A populated In-Reply-To must not confer trust.
        let mut c = candidate(
            "Re: Chris, have you tried that solution?",
            "Chris, have you tried that solution? Book a call with me.",
            "owen@stackfoundry.example",
        );
        c.known_domain = true; // even a stray past reply must not save it
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn drops_name_opener_pitch_even_without_a_known_phrase() {
        let c = candidate(
            "Chris, do you have time on Wednesday?",
            "Let me know what works.",
            "vic@dealflow.example",
        );
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn keeps_a_name_opener_when_they_are_replying_to_us() {
        let mut c = candidate(
            "Re: Chris, quick sync",
            "Can you confirm Thursday works?",
            "dana@northwind.example",
        );
        c.replies_to_me = true;
        assert!(evaluate(&c).is_some());
    }

    #[test]
    fn drops_noreply_senders_where_the_token_is_a_suffix() {
        // googleads-noreply@ and CloudPlatform-noreply@ slipped past a
        // start-anchored match and pulled receipts into the queue.
        for sender in [
            "googleads-noreply@google.com",
            "CloudPlatform-noreply@google.com",
            "auto-confirm@amazon.com",
        ] {
            let c = candidate(
                "Action Required: Create a passkey",
                "Do you want to secure your account?",
                sender,
            );
            assert!(evaluate(&c).is_none(), "{sender} should be excluded");
        }
    }

    #[test]
    fn drops_footer_boilerplate_questions() {
        let mut c = candidate(
            "Invoice INV-7 has been paid",
            "Questions? Visit our help center. Need help? Contact support.",
            "hello@mercury.com",
        );
        c.known_sender = true;
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn drops_transactional_confirmations() {
        let mut c = candidate(
            "Delivered: \"Autobiography of a Yogi\"",
            "Your package was delivered. How did we do?",
            "order-update@amazon.com",
        );
        c.known_sender = true;
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn keeps_an_operational_failure_alert() {
        let c = candidate(
            "Container finance_api in Container Manager stopped unexpectedly",
            "The container stopped unexpectedly.",
            "sns@synologynotification.com",
        );
        let v = evaluate(&c).expect("ops failures are opted in");
        assert_eq!(v.action, ActionType::Alert);
        assert_eq!(v.action.lane(), 1, "alerts rank below people");
    }

    #[test]
    fn drops_routine_reports_that_merely_contain_failure_words() {
        let c = candidate(
            "[Preview] Report Domain: trycxventures.com",
            "SPF failed for 2 messages.",
            "dmarcreport@microsoft.com",
        );
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn a_stranger_needs_an_obligation_not_just_a_question_mark() {
        let plain = candidate(
            "Following up on my note",
            "Do you want to go over the ideas we discussed?",
            "mira@launchcrew.example",
        );
        assert!(evaluate(&plain).is_none());

        let obliging = candidate(
            "Contract signature required",
            "Please sign the contract by Friday.",
            "legal@newclient.com",
        );
        let v = evaluate(&obliging).expect("an explicit obligation qualifies a stranger");
        assert_eq!(v.action, ActionType::Decision);
    }

    #[test]
    fn drops_bulk_mail_from_senders_we_never_write_to() {
        let mut c = candidate(
            "New skill available: Puzzle solving",
            "Can you solve it?",
            "messages-noreply@linkedin.com",
        );
        c.has_list_unsubscribe = true;
        assert!(evaluate(&c).is_none());
    }

    #[test]
    fn collapses_repeated_alerts_and_thread_siblings() {
        let a = collapse_key(
            ActionType::Alert,
            "sns@synologynotification.com",
            "Container finance_api in Container Manager stopped at 12:04",
            "acct",
            None,
        );
        let b = collapse_key(
            ActionType::Alert,
            "sns@synologynotification.com",
            "Container finance_api in Container Manager stopped at 15:47",
            "acct",
            None,
        );
        assert_eq!(a, b, "the same alert at a different time is one row");

        let t1 = collapse_key(ActionType::Reply, "a@x.com", "Re: Budget", "acct", Some("root-1"));
        let t2 = collapse_key(ActionType::Reply, "b@x.com", "Budget", "acct", Some("root-1"));
        assert_eq!(t1, t2, "one row per conversation");
    }

    #[test]
    fn sentence_splitter_handles_multibyte_text() {
        // Guards against slicing panics on non-ASCII subjects.
        let q = real_question("¿Cómo estás? Can you confirm the date?");
        assert!(q.is_some());
        assert!(real_question("Emoji 🎉 only. No ask here.").is_none());
    }
}
