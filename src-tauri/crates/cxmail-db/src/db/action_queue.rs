use super::{intelligence, messages, needs_you, nudges};
use crate::error::AppError;
use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize)]
pub struct ActionItem {
    #[serde(flatten)]
    pub message: needs_you::NeedsYouItem,
    pub lane: String,
    pub request: String,
    /// Date-only: email prose rarely establishes a trustworthy time zone.
    pub deadline: Option<String>,
    pub suggestion: Option<intelligence::ReplySuggestion>,
    pub thread_key: Option<String>,
}

pub fn deadline(text: &str, received: &str) -> Option<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:due(?:\s+by|\s+on)?|deadline(?:\s+is)?|respond\s+by|reply\s+by|by|before)\s*[:\-]?\s*(\d{4}-\d{2}-\d{2}|today|tomorrow|monday|tuesday|wednesday|thursday|friday|saturday|sunday|(?:january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{1,2}(?:,?\s+\d{4})?)\b").unwrap()
    });
    let base = NaiveDate::parse_from_str(received.get(..10)?, "%Y-%m-%d").ok()?;
    // Quoted deadlines belong to the previous request, not today's reply.
    let mut authored = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if (lower.starts_with("on ") && lower.ends_with("wrote:"))
            || lower.starts_with("-----original message-----")
            || lower.starts_with("---------- forwarded message")
            || lower.starts_with("begin forwarded message:")
        {
            break;
        }
        if !trimmed.starts_with('>') {
            authored.push_str(line);
            authored.push('\n');
        }
    }
    let caps = RE.captures(&authored)?;
    let raw = caps[1].to_ascii_lowercase();
    let day = if raw == "today" {
        base
    } else if raw == "tomorrow" {
        base.checked_add_signed(Duration::days(1))?
    } else if let Ok(day) = NaiveDate::parse_from_str(&raw, "%Y-%m-%d") {
        day
    } else if let Some(weekday) = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ]
    .iter()
    .position(|d| *d == raw)
    {
        let delta = (weekday as i64 - base.weekday().num_days_from_monday() as i64 + 7) % 7;
        base.checked_add_signed(Duration::days(delta))?
    } else {
        let normalized = raw.replace(',', "");
        if normalized.split_whitespace().count() == 3 {
            NaiveDate::parse_from_str(&normalized, "%B %d %Y").ok()?
        } else {
            let mut day =
                NaiveDate::parse_from_str(&format!("{normalized} {}", base.year()), "%B %d %Y")
                    .ok()?;
            if day < base - Duration::days(30) {
                day = day.with_year(base.year() + 1)?;
            }
            day
        }
    };
    Some(day.to_string())
}

fn enrich(
    c: &Connection,
    item: needs_you::NeedsYouItem,
    lane: &str,
    thread_key: Option<String>,
) -> Result<ActionItem, AppError> {
    let id = intelligence::message_id(c, &item.account_id, &item.folder_name, item.uid)?;
    let body = messages::get_body(c, &item.account_id, &item.folder_name, item.uid)?
        .and_then(|b| b.plain_text)
        .unwrap_or_default();
    let request = item
        .evidence
        .clone()
        .or_else(|| item.snippet.clone())
        .unwrap_or_else(|| item.reason.clone());
    let deadline = deadline(&request, &item.date).or_else(|| deadline(&body, &item.date));
    Ok(ActionItem {
        suggestion: intelligence::suggestion(c, id)?,
        message: item,
        lane: lane.into(),
        request,
        deadline,
        thread_key,
    })
}

pub fn list(c: &Connection, limit: u32, use_triage: bool) -> Result<Vec<ActionItem>, AppError> {
    let mut items = needs_you::list(c, limit, use_triage)?
        .into_iter()
        .map(|m| enrich(c, m, "needs_response", None))
        .collect::<Result<Vec<_>, _>>()?;
    let waiting = nudges::list(
        c,
        &nudges::NudgeOptions {
            min_age_days: 0,
            max_age_days: 30,
            limit: limit as usize,
        },
    )?;
    for n in waiting.into_iter().filter(|n| n.kind == "follow_up") {
        let Some(m) = messages::get_by_uid(c, &n.account_id, &n.folder_name, n.uid)? else {
            continue;
        };
        // Queue dismissal is separate from the nudge badge and remains local.
        let dismissed=c.query_row("SELECT EXISTS(SELECT 1 FROM needs_you_dismissals WHERE account_id=?1 AND folder_name=?2 AND uid=?3)",params![n.account_id,n.folder_name,n.uid],|r|r.get::<_,bool>(0))?;
        if dismissed {
            continue;
        }
        let item = needs_you::NeedsYouItem {
            uid: m.uid,
            account_id: n.account_id.clone(),
            folder_name: n.folder_name.clone(),
            subject: n.subject,
            from_name: n.counterpart_name,
            from_email: Some(n.counterpart_email),
            date: n.date,
            snippet: m.snippet,
            is_read: true,
            has_attachments: m.has_attachments,
            action_type: "follow_up".into(),
            reason: "Waiting for their response".into(),
            evidence: None,
            duplicate_count: 1,
            members: vec![needs_you::MessageRef {
                account_id: n.account_id,
                folder_name: n.folder_name,
                uid: n.uid,
            }],
            score: 0.0,
            urgency: None,
        };
        let mut enriched = enrich(c, item, "waiting", Some(n.thread_key))?;
        // The anchor is an older inbound message, not the last sent one.
        enriched.deadline = None;
        enriched.suggestion = None;
        enriched.request = format!(
            "You wrote {} day{} ago",
            n.days_ago,
            if n.days_ago == 1 { "" } else { "s" }
        );
        items.push(enriched);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadlines_require_an_explicit_ask_and_parse_relative_to_the_email() {
        assert_eq!(
            deadline("Please reply by tomorrow.", "2026-12-31T12:00:00Z").as_deref(),
            Some("2027-01-01")
        );
        assert_eq!(
            deadline("Please send it by Friday", "2026-09-30T12:00:00Z").as_deref(),
            Some("2026-10-02")
        );
        assert_eq!(
            deadline("Deadline: October 4, 2026", "2026-09-30T12:00:00Z").as_deref(),
            Some("2026-10-04")
        );
        assert!(deadline("We met on 2026-09-12", "2026-09-30").is_none());
        assert!(deadline("due 2026-02-31", "2026-09-30").is_none());
        assert!(deadline(
            "Thanks!\nOn Tuesday, Sam wrote:\nPlease reply by tomorrow.",
            "2026-09-30"
        )
        .is_none());
        assert!(deadline("> Please reply by Friday.", "2026-09-30").is_none());
    }
}
