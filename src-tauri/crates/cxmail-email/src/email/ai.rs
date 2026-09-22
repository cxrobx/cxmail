use crate::error::AppError;
use crate::email::inference::InferenceClient;
use chrono::Local;
use serde::{Deserialize, Serialize};

const MAX_INPUT_CHARS: usize = 6000;

pub(crate) async fn call_inference(
    client: &InferenceClient,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<String, AppError> {
    client
        .complete(system_prompt, user_prompt, max_tokens, temperature)
        .await
}

/// Appended to every prose-generating system prompt below.
///
/// This is the cheap half. `call_inference_prose` is the half that holds: the
/// model is told, and then the output is normalized regardless. Wording moves
/// the odds; it does not make a rule absolute (gotcha #47).
pub(crate) const NO_LONG_DASH_RULE: &str =
    "- Never use an em-dash (\u{2014}), a spaced en-dash, or any other long dash. \
Use a comma, colon, period, or parentheses instead, and never a hyphen as a stand-in.";

/// `call_inference` for anything whose output is prose the user may send.
///
/// Applies the house dash rule to the model's answer. Deliberately NOT used by
/// `parse_search_query` or `validate_draft`, whose responses are JSON destined
/// for a parser rather than for a mailbox.
async fn call_inference_prose(
    client: &InferenceClient,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<String, AppError> {
    let raw = call_inference(client, system_prompt, user_prompt, max_tokens, temperature).await?;
    Ok(crate::email::dashes::normalize_dashes(&raw).text)
}

/// Cut `text` to at most `max` BYTES, backing up to a char boundary.
///
/// `text.len()` is bytes and `&text[..max]` PANICS when `max` lands inside a
/// multi-byte sequence — a length check is not a boundary check (gotcha #21b).
/// Every caller passes arbitrary email text, so any message whose 6000th byte
/// falls mid-character would have taken down the AI path.
fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Generate a draft reply to an email. If `voice_ctx` is provided, the reply is
/// written in the user's extracted voice; otherwise a generic assistant tone is used.
/// `voice_ctx` should be the assembled output of `build_voice_context`, optionally
/// including per-recipient few-shot examples and learned preferences.
pub async fn generate_reply(
    client: &InferenceClient,
    original_email: &str,
    sender_name: &str,
    context: &str,
    voice_ctx: Option<&str>,
) -> Result<String, AppError> {
    let system: String = match voice_ctx {
        Some(vctx) => format!(
            "You are helping the user reply to an email. Write the reply in their exact voice and style.\n\n\
{vctx}\n\n\
Instructions:\n\
- Match the vocabulary, sentence structure, greeting, and sign-off shown above.\n\
- Do NOT sound like AI; sound like the human author of those examples.\n\
- Write only the reply body: no subject line, no preamble like 'Here is a draft'.\n\
- Start directly with an appropriate greeting (or none, if that matches the voice).\n\
- Keep it concise. Match the formality of the original email."
        ),
        None => "You are a helpful email writing assistant. Generate a professional, natural-sounding reply to the email below. \
Write only the reply body: no subject line, no greeting preamble like 'Here is a draft'. \
Start directly with an appropriate greeting (e.g. 'Hi [Name],'). Keep it concise and relevant. \
Match the formality level of the original email.".to_string(),
    };

    let user = format!(
        "Original email from {}:\n\n{}\n\n{}",
        sender_name,
        truncate(original_email, MAX_INPUT_CHARS),
        if context.is_empty() {
            String::new()
        } else {
            format!("Additional context/instructions: {}", context)
        }
    );

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(client, &system, &user, 500, 0.7).await
}

/// Assemble voice context XML blocks from stored JSON plus optional per-recipient
/// few-shot and learned preferences.
///
/// Precedence for the `<voice-profile>` block:
///   1. `recipient_profile_json` — distilled profile of how the user writes to this specific person
///   2. `archetype_profile_json` — distilled profile of the archetype this recipient maps to
///   3. `profile_json`           — account-level voice profile (fallback)
///
/// When a `recipient_profile_json` is present, raw `recipient_examples` are
/// capped at 1 because the structured profile already captures the patterns.
///
/// `pinned_rules` (`db::voice_pinned_rules`) outrank all of the above and are
/// emitted FIRST, as absolutes. Everything else in this context is a
/// *description* of past behaviour, and a description is something a model may
/// reasonably trade off against the request; a pinned rule is an instruction
/// from the user that it may not. Ordering is the caller's contract —
/// account-wide rules then recipient-specific ones, so the more specific rule
/// reads last and wins.
///
/// Returns None if no usable profile data exists (graceful degradation).
pub fn build_voice_context_full(
    profile_json: &str,
    examples_json: &str,
    recipient_profile_json: Option<&str>,
    archetype_profile_json: Option<&str>,
    recipient_examples: &[String],
    learned_preferences: &[String],
    pinned_rules: &[String],
) -> Option<String> {
    let effective_profile = recipient_profile_json
        .or(archetype_profile_json)
        .unwrap_or(profile_json);

    let max_recipient_examples = if recipient_profile_json.is_some() {
        1
    } else {
        3
    };

    let mut blocks: Vec<String> = Vec::new();

    // 0. Pinned rules — absolute, and first so nothing below can read as an
    //    exception to them.
    if !pinned_rules.is_empty() {
        let mut lines = vec!["<pinned-rules>".to_string()];
        lines.push(
            "## ABSOLUTE RULES set by the user. These OVERRIDE every profile, example and \
             learned preference below, including any that describes the opposite habit. \
             Follow them exactly, in every message."
                .to_string(),
        );
        for rule in pinned_rules {
            let rule = rule.trim();
            if rule.is_empty() {
                continue;
            }
            lines.push(format!("- {}", rule));
        }
        lines.push("</pinned-rules>".to_string());
        if lines.len() > 3 {
            blocks.push(lines.join("\n"));
        }
    }

    // 1. Per-recipient few-shot (most targeted raw signal)
    if !recipient_examples.is_empty() {
        let mut lines = vec!["<recent-replies-to-this-recipient>".to_string()];
        for (i, ex) in recipient_examples
            .iter()
            .take(max_recipient_examples)
            .enumerate()
        {
            lines.push(format!("Example {}:\n{}", i + 1, ex));
        }
        lines.push("</recent-replies-to-this-recipient>".to_string());
        blocks.push(lines.join("\n\n"));
    }

    // 2. + 3. Profile + examples (existing helper, using the most specific available profile)
    if let Some(core) = build_voice_context(effective_profile, examples_json) {
        blocks.push(core);
    } else if recipient_examples.is_empty() && learned_preferences.is_empty() && blocks.is_empty() {
        // `blocks` is non-empty only when pinned rules were emitted above. A
        // pinned rule is reason enough to build a context on its own — bailing
        // out here would drop the user's absolutes for any recipient who has no
        // profile yet, which is most of them.
        return None;
    }

    // 4. Learned preferences from edits
    if !learned_preferences.is_empty() {
        let mut lines = vec!["<learned-preferences>".to_string()];
        lines.push("## Rules learned from how you've edited previous AI drafts:".to_string());
        for p in learned_preferences.iter().take(5) {
            let truncated: String = p.chars().take(100).collect();
            lines.push(format!("- {}", truncated));
        }
        lines.push("</learned-preferences>".to_string());
        blocks.push(lines.join("\n"));
    }

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

/// Assemble `<voice-profile>` and `<voice-examples>` XML blocks from stored JSON.
/// Returns None if either JSON is unparseable (graceful degradation to generic prompt).
/// Total context is capped at ~3500 chars; lowest-priority profile fields and extra
/// examples are dropped first.
pub fn build_voice_context(profile_json: &str, examples_json: &str) -> Option<String> {
    const PROFILE_BUDGET: usize = 2000;
    const EXAMPLES_BUDGET: usize = 1500;
    const MAX_EXCERPT_CHARS: usize = 500;

    let profile: serde_json::Value = serde_json::from_str(profile_json).ok()?;
    let profile_obj = profile.as_object()?;

    // Priority-ordered profile fields (email-specific ones early).
    let priority_fields: &[(&str, &str)] = &[
        ("summary", "Summary"),
        ("tone", "Tone"),
        ("typical_greeting", "Typical greeting"),
        ("typical_signoff", "Typical sign-off"),
        ("typical_length_words", "Typical length (words)"),
        ("formality", "Formality"),
        ("punctuation_style", "Punctuation"),
        ("sentence_patterns", "Sentence patterns"),
        ("vocabulary_level", "Vocabulary"),
        ("what_to_avoid", "What to avoid"),
    ];

    let mut profile_lines = vec!["<voice-profile>".to_string()];
    let mut used = 0usize;
    for (key, label) in priority_fields {
        let val = match profile_obj.get(*key) {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => continue,
        };
        let line = format!("- {}: {}", label, val);
        if used + line.len() > PROFILE_BUDGET {
            continue;
        }
        used += line.len() + 1;
        profile_lines.push(line);
    }

    if let Some(arr) = profile_obj
        .get("distinctive_phrases")
        .and_then(|v| v.as_array())
    {
        let phrases: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", s.trim())))
            .filter(|s| s.len() > 2)
            .take(10)
            .collect();
        if !phrases.is_empty() {
            let line = format!("- Distinctive phrases: {}", phrases.join(", "));
            if used + line.len() <= PROFILE_BUDGET {
                profile_lines.push(line);
            }
        }
    }

    profile_lines.push("</voice-profile>".to_string());
    let profile_block = profile_lines.join("\n");

    // Examples block
    let mut example_block = String::new();
    if let Ok(examples) = serde_json::from_str::<serde_json::Value>(examples_json) {
        if let Some(arr) = examples.as_array() {
            let mut example_lines: Vec<String> = vec!["<voice-examples>".to_string()];
            let mut ex_used = 0usize;
            for (i, ex) in arr.iter().enumerate() {
                let excerpt = match ex.get("excerpt").and_then(|v| v.as_str()) {
                    Some(s) if !s.trim().is_empty() => s.trim(),
                    _ => continue,
                };
                let trimmed = truncate_excerpt(excerpt, MAX_EXCERPT_CHARS);
                let block = format!("Example {}:\n{}", i + 1, trimmed);
                if ex_used + block.len() > EXAMPLES_BUDGET {
                    break;
                }
                ex_used += block.len() + 2;
                example_lines.push(block);
            }
            if example_lines.len() > 1 {
                example_lines.push("</voice-examples>".to_string());
                example_block = example_lines.join("\n\n");
            }
        }
    }

    if example_block.is_empty() {
        Some(profile_block)
    } else {
        Some(format!("{}\n\n{}", profile_block, example_block))
    }
}

fn truncate_excerpt(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let target = max.saturating_sub(3);
    if target == 0 {
        return text[..max].to_string();
    }
    let cutoff = text[..target].rfind(char::is_whitespace).unwrap_or(target);
    format!("{}...", text[..cutoff].trim_end())
}

/// Draft a fresh email to a recipient. No inbound message — used by the
/// "Draft for me" action in the compose modal. Conditioned on the same voice
/// context as `generate_reply` when provided.
pub async fn generate_compose(
    client: &InferenceClient,
    recipient_email: &str,
    subject: Option<&str>,
    instruction: Option<&str>,
    voice_ctx: Option<&str>,
) -> Result<String, AppError> {
    let system: String = match voice_ctx {
        Some(vctx) => format!(
            "You are helping the user draft a new email. Write in their exact voice and style.\n\n\
{vctx}\n\n\
Instructions:\n\
- Match the vocabulary, sentence structure, greeting, and sign-off shown above.\n\
- Do NOT sound like AI; sound like the human author of those examples.\n\
- Write only the email body: no subject line, no preamble like 'Here is a draft'.\n\
- Start directly with an appropriate greeting (or none, if that matches the voice).\n\
- Keep it concise and aim for the typical length implied by the voice profile."
        ),
        None => "You are a helpful email writing assistant. Draft a professional, natural-sounding email. \
Write only the email body: no subject line, no preamble like 'Here is a draft'. \
Start directly with an appropriate greeting (e.g. 'Hi [Name],'). Keep it concise."
            .to_string(),
    };

    let mut user = format!("Recipient: {}\n", recipient_email);
    if let Some(s) = subject {
        if !s.trim().is_empty() {
            user.push_str(&format!("Subject: {}\n", s.trim()));
        }
    }
    if let Some(i) = instruction {
        if !i.trim().is_empty() {
            user.push_str(&format!(
                "\nWhat to say: {}\n",
                truncate(i, MAX_INPUT_CHARS)
            ));
        }
    }
    user.push_str("\nDraft the email body now.");

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(client, &system, &user, 600, 0.7).await
}

/// Rewrite text according to custom instructions.
pub async fn rewrite_text(
    client: &InferenceClient,
    text: &str,
    instruction: &str,
) -> Result<String, AppError> {
    let system = "You are an email writing assistant. Rewrite the given text according to the user's instructions. \
Return only the rewritten text: no explanations, no preamble, no quotes around it.";

    let user = format!(
        "Instructions: {}\n\nText to rewrite:\n{}",
        instruction,
        truncate(text, MAX_INPUT_CHARS)
    );

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(client, &system, &user, 800, 0.6).await
}

/// Fix spelling, grammar, and punctuation without changing meaning or voice.
pub async fn proofread(client: &InferenceClient, text: &str) -> Result<String, AppError> {
    let system = "You are a proofreader for email. Correct spelling, grammar, punctuation, \
and obvious word-choice/tense errors in the text below. Preserve the author's meaning, \
voice, tone, and approximate length. Do NOT rephrase for style, add content, or reformat. \
Return only the corrected text: no explanations, no preamble, no quotes.";
    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(client, &system, truncate(text, MAX_INPUT_CHARS), 800, 0.2).await
}

/// Adjust the tone of text.
pub async fn adjust_tone(client: &InferenceClient, text: &str, tone: &str) -> Result<String, AppError> {
    let system = format!(
        "You are an email writing assistant. Rewrite the given text in a {} tone. \
Maintain the same meaning, key points, and approximate length. \
Return only the rewritten text: no explanations or preamble.",
        tone
    );

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(client, &system, truncate(text, MAX_INPUT_CHARS), 800, 0.6).await
}

/// Generate smart reply suggestions (returns 3 short options).
pub async fn suggest_replies(
    client: &InferenceClient,
    email_text: &str,
    sender_name: &str,
) -> Result<Vec<String>, AppError> {
    let system = "You are an email assistant. Generate exactly 3 brief reply options for the email below. \
Each reply should be 1-2 sentences: short and ready to send as-is. \
Cover different intents: one positive/agreeable, one requesting more info or asking a question, and one brief acknowledgment. \
Return as a JSON array of 3 strings. No markdown, no code fences, just the raw JSON array.";

    let user = format!(
        "Email from {}:\n\n{}",
        sender_name,
        truncate(email_text, MAX_INPUT_CHARS)
    );

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    let response = call_inference(client, &system, &user, 300, 0.8).await?;

    // Parse JSON array
    let replies: Vec<String> = serde_json::from_str(&response)
        .map_err(|_| {
            // Fallback: try to extract from markdown code fence
            let cleaned = response
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            serde_json::from_str::<Vec<String>>(cleaned)
        })
        .or_else(|r| r)
        .map_err(|e| AppError::AiService(format!("Failed to parse reply suggestions: {}", e)))?;

    // Each suggestion is one click from being sent, so it gets the same
    // treatment as a generated draft.
    Ok(replies
        .into_iter()
        .take(3)
        .map(|r| crate::email::dashes::normalize_dashes(&r).text)
        .collect())
}

/// Suggest a subject line from an email body.
pub async fn suggest_subject(client: &InferenceClient, email_body: &str) -> Result<String, AppError> {
    let system = "You are an email writing assistant. Generate a concise, natural email subject line for the given email body. \
Return only the subject line: no quotes, no prefix like 'Subject:', no explanation.";

    let system = format!("{system}\n{NO_LONG_DASH_RULE}");
    call_inference_prose(
        client,
        &system,
        truncate(email_body, MAX_INPUT_CHARS),
        50,
        0.7,
    )
    .await
}

/// Structured search parameters extracted from a natural language query.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ParsedSearchQuery {
    pub keywords: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject_contains: Option<String>,
    pub date_after: Option<String>,
    pub date_before: Option<String>,
    pub folder: Option<String>,
    pub has_attachments: Option<bool>,
    pub is_starred: Option<bool>,
    pub is_unread: Option<bool>,
}

/// Parse a natural language search query into structured filters using the LLM.
pub async fn parse_search_query(client: &InferenceClient, query: &str) -> Result<ParsedSearchQuery, AppError> {
    let current_date = Local::now().format("%Y-%m-%d").to_string();

    let system = format!(
        r#"You are a search query parser for an email client. Today's date is {current_date}.

Extract structured search parameters from the user's natural language query. Return a JSON object with these optional fields (omit or set null for fields not mentioned):

- "keywords": string — free-text search terms for email body/content (strip filler like "emails about")
- "from": string — sender email or name
- "to": string — recipient email or name
- "subject_contains": string — subject line keywords
- "date_after": string — ISO 8601 date (YYYY-MM-DD), inclusive start
- "date_before": string — ISO 8601 date (YYYY-MM-DD), inclusive end
- "folder": string — one of "inbox", "sent", "drafts", "trash", "archive", "spam"
- "has_attachments": boolean — whether the email has attachments
- "is_starred": boolean — whether the email is starred/flagged
- "is_unread": boolean — whether the email is unread

Examples:
User: "emails from sarah about the budget last week"
{{"keywords": "budget", "from": "sarah", "date_after": "2026-03-22", "date_before": "2026-03-29"}}

User: "show me receipts from Amazon"
{{"keywords": "receipt order confirmation", "from": "amazon"}}

User: "starred emails with attachments"
{{"has_attachments": true, "is_starred": true}}

User: "unread mail I sent john yesterday"
{{"to": "john", "is_unread": true, "folder": "sent", "date_after": "2026-03-28", "date_before": "2026-03-28"}}

Return ONLY the JSON object, no markdown fences, no explanation."#
    );

    // 10s cap specifically for search parsing: SearchBar fires this on Enter
    // and must fail fast into the deterministic fallback chain.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        call_inference(client, &system, query, 200, 0.0),
    )
    .await
    .map_err(|_| AppError::AiService("Search query parse timed out".to_string()))??;

    let cleaned = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str::<ParsedSearchQuery>(cleaned)
        .map_err(|e| AppError::AiService(format!("Failed to parse search query response: {}", e)))
}

/* ── Draft review ───────────────────────────────────────────── */

/// One contextual problem found in an outgoing draft.
///
/// Deliberately narrower than the frontend's `Finding`: this covers only what
/// needs the model. Mechanical checks (recipients, placeholders, dead links,
/// missing attachment) run instantly in `src/lib/draftValidation.ts` and are
/// never asked of the LLM — asking would spend a round trip to duplicate an
/// answer we already have, and invite it to disagree with itself.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DraftFinding {
    pub category: String,
    pub severity: String,
    /// Subject / Greeting / Body / Closing / Thread, or "¶N".
    #[serde(rename = "where")]
    pub location: String,
    pub title: String,
    pub detail: String,
    /// Verbatim text from the draft, or None. Verified to actually occur in
    /// the body — see `parse_draft_findings`.
    pub quote: Option<String>,
    pub suggestion: Option<String>,
}

const MAX_FINDINGS: usize = 6;
const VALID_SEVERITIES: [&str; 3] = ["error", "warning", "info"];
const VALID_CATEGORIES: [&str; 7] = [
    "pinned_rule",
    "unanswered_question",
    "missing_context",
    "clarity",
    "next_step",
    "consistency",
    "tone",
];

/// Strip a markdown code fence, if the model wrapped its JSON in one.
fn strip_fence(response: &str) -> &str {
    response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
}

/// Parse and SANITIZE the model's response.
///
/// Split out as a pure function because every interesting failure lives here
/// and none of it needs a network call to test. Four guards:
///
/// 1. **A `quote` that isn't in the body is dropped.** Models paraphrase, and
///    the frontend turns `quote` into a find-and-replace against the real
///    document. A near-miss quote would produce an Apply button that silently
///    does nothing — worse than no button. Whitespace is normalized before
///    comparing, since the draft's plain text may wrap differently.
/// 2. **Unknown severity/category fall back** rather than reaching the UI as
///    an unstyled string.
/// 3. **Findings with no title are dropped** — nothing to show.
/// 4. **Capped at MAX_FINDINGS**, so a runaway response can't flood the panel.
///
/// A response that is not JSON at all yields an error; a response that is a
/// well-formed empty array yields an empty vec, which is the common and
/// correct answer for a clean draft.
pub fn parse_draft_findings(response: &str, body_text: &str) -> Result<Vec<DraftFinding>, AppError> {
    let cleaned = strip_fence(response);
    let raw: Vec<DraftFinding> = serde_json::from_str(cleaned).map_err(|e| {
        AppError::AiService(format!("Failed to parse draft review response: {}", e))
    })?;

    let haystack = normalize_ws(body_text);

    let out = raw
        .into_iter()
        .filter(|f| !f.title.trim().is_empty())
        .map(|mut f| {
            if !VALID_SEVERITIES.contains(&f.severity.as_str()) {
                f.severity = "warning".to_string();
            }
            if !VALID_CATEGORIES.contains(&f.category.as_str()) {
                f.category = "clarity".to_string();
            }
            if f.location.trim().is_empty() {
                f.location = "Body".to_string();
            }
            f.quote = f.quote.filter(|q| {
                let q = q.trim();
                !q.is_empty() && haystack.contains(&normalize_ws(q))
            });
            // A suggestion with nothing to attach it to cannot be applied
            // mechanically; the UI degrades those to "ask in chat".
            f.suggestion = f.suggestion.filter(|s| !s.trim().is_empty());
            f
        })
        .take(MAX_FINDINGS)
        .collect();

    Ok(out)
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Review a draft for problems that need judgement rather than pattern matching.
///
/// `pinned_rules` is the load-bearing input. Those are the user's absolute
/// instructions (v51) — "always address Sam Ellis as Bro. Ellis" — and until
/// now nothing in CXMail ever *verified* one was honoured; `compose_draft`
/// only appends an advisory note the agent is free to ignore (gotcha #43).
/// Checking compliance here is the first enforcement point.
pub async fn validate_draft(
    client: &InferenceClient,
    recipient_email: Option<&str>,
    subject: &str,
    body_text: &str,
    original_message: Option<&str>,
    pinned_rules: &[String],
) -> Result<Vec<DraftFinding>, AppError> {
    let system = format!(
        r#"You are reviewing an email draft before the user sends it. Report only substantive problems the writer would genuinely want to fix.

DO NOT report:
- spelling, grammar or punctuation (a separate proofreader handles those)
- a missing subject, missing recipients, missing attachments, unfilled [placeholders] or dead links (already checked mechanically — reporting them again duplicates a warning the user can see)
- style preferences, or suggestions to make the writing "more engaging"

DO report:
- pinned_rule: an absolute rule listed below is broken
- unanswered_question: the original message asked something this draft does not answer
- missing_context: the draft references something the recipient has no way to resolve (an unstated date, amount, or name)
- consistency: the draft contradicts the original message or itself
- next_step: the draft has no clear ask, so the recipient cannot act
- clarity: a sentence is genuinely ambiguous about who does what, or when
- tone: the register is wrong for this specific recipient given the thread

Return a JSON array. Each element:
{{"category": one of {cats},
  "severity": "error" | "warning" | "info",
  "where": "Subject" | "Greeting" | "Body" | "Closing" | "Thread",
  "title": "under 60 characters, states the problem",
  "detail": "one or two sentences, concrete, naming the specific thing",
  "quote": "text copied VERBATIM from the draft, or null",
  "suggestion": "replacement text for the quote, or the text to add, or null"}}

Rules for the output:
- "quote" MUST be copied character-for-character from the draft. If you cannot copy it exactly, use null. Never paraphrase into this field.
- severity "error" is for something that would embarrass the sender or break a pinned rule. Most findings are "warning" or "info".
- At most {max} findings, most important first.
- Be sparing. An empty array [] is a valid and COMMON answer — most drafts are fine. Do not invent problems to fill the list.
- No markdown, no code fences, no prose. Just the raw JSON array."#,
        cats = VALID_CATEGORIES.join(" | "),
        max = MAX_FINDINGS,
    );

    let mut user = String::new();
    if let Some(r) = recipient_email {
        user.push_str(&format!("Recipient: {}\n", r));
    }
    user.push_str(&format!("Subject: {}\n", subject.trim()));

    if !pinned_rules.is_empty() {
        user.push_str("\nABSOLUTE RULES the user has pinned for this message. Breaking one is always an \"error\" finding with category \"pinned_rule\":\n");
        for rule in pinned_rules {
            if rule.trim().is_empty() {
                continue;
            }
            user.push_str(&format!("- {}\n", rule.trim()));
        }
    }

    if let Some(original) = original_message {
        if !original.trim().is_empty() {
            user.push_str(&format!(
                "\nThe message being replied to:\n\"\"\"\n{}\n\"\"\"\n",
                truncate(original.trim(), MAX_INPUT_CHARS)
            ));
        }
    }

    user.push_str(&format!(
        "\nThe draft to review:\n\"\"\"\n{}\n\"\"\"\n\nReturn the JSON array now.",
        truncate(body_text.trim(), MAX_INPUT_CHARS)
    ));

    // Low temperature: this is an analysis task, and a creative reviewer
    // invents problems.
    let response = call_inference(client, &system, &user, 900, 0.2).await?;
    parse_draft_findings(&response, body_text)
}

/// Build the full layered voice context for drafting an email to
/// `recipient_email`. Loads the user's pinned rules, the per-recipient profile
/// (if any), the matching archetype profile (if any), the account-level profile,
/// raw recipient few-shot, and active learned insights — then layers them
/// through `build_voice_context_full`'s precedence rules.
///
/// Returns `None` when no profile data and no pinned rules exist; callers fall
/// back to a generic prompt.
///
/// Moved here from the app's `commands::ai` (2026-08-31) so the MCP's external
/// writer gets the IDENTICAL context the in-app AI drafting gets — one
/// assembler, per gotcha #36's one-matcher rule. It only ever needed `db::` +
/// `email::voice`, both visible from this crate.
pub fn build_voice_ctx_for_recipient(
    conn: &rusqlite::Connection,
    account_id: &str,
    recipient_email: &str,
) -> Option<String> {
    let recipient_norm = recipient_email.trim().to_lowercase();

    let account_profile = crate::db::voice_profiles::get_by_account(conn, account_id)
        .ok()
        .flatten();
    let recipient_profile = if recipient_norm.is_empty() {
        None
    } else {
        crate::db::voice_profiles_recipient::get(conn, account_id, &recipient_norm)
            .ok()
            .flatten()
    };
    let archetype_profile = if recipient_norm.is_empty() {
        None
    } else {
        crate::db::voice_archetypes::archetype_for_recipient(conn, account_id, &recipient_norm)
            .ok()
            .flatten()
            .and_then(|aid| {
                crate::db::voice_archetypes::get(conn, account_id, &aid)
                    .ok()
                    .flatten()
            })
    };

    let recipient_examples = if recipient_norm.is_empty() {
        Vec::new()
    } else {
        crate::email::voice::collect_diverse_samples_for_recipient(
            conn,
            account_id,
            &recipient_norm,
            crate::email::voice::RECIPIENT_SAMPLES,
        )
        .unwrap_or_default()
    };
    let learned = crate::db::voice_edits::list_active_insights(conn, account_id, 5).unwrap_or_default();

    // Pinned rules apply even with no recipient in hand — an account-scoped rule
    // ("never open with 'Hope you're well'") governs every outgoing message.
    let pinned: Vec<String> = if recipient_norm.is_empty() {
        crate::db::voice_pinned_rules::list_account_scoped(conn, account_id).unwrap_or_default()
    } else {
        crate::db::voice_pinned_rules::list_effective(conn, account_id, &recipient_norm)
            .unwrap_or_default()
    }
    .into_iter()
    .map(|r| r.rule)
    .collect();

    let (account_profile_json, account_examples_json) = match &account_profile {
        Some(p) => (
            p.voice_profile_json.as_str(),
            p.voice_examples_json.as_str(),
        ),
        None => ("{}", "[]"),
    };

    if account_profile.is_none()
        && recipient_profile.is_none()
        && archetype_profile.is_none()
        && recipient_examples.is_empty()
        && learned.is_empty()
        && pinned.is_empty()
    {
        return None;
    }

    build_voice_context_full(
        account_profile_json,
        account_examples_json,
        recipient_profile.as_ref().map(|p| p.profile_json.as_str()),
        archetype_profile.as_ref().map(|a| a.profile_json.as_str()),
        &recipient_examples,
        &learned,
        &pinned,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_voice_context_renders_profile_and_examples() {
        let profile = r#"{
            "summary": "Terse, warm, direct.",
            "tone": "friendly but brief",
            "typical_greeting": "Hey [Name] —",
            "typical_signoff": "",
            "typical_length_words": 60,
            "distinctive_phrases": ["sounds good", "let me know"],
            "what_to_avoid": "corporate speak"
        }"#;
        let examples = r#"[
            {"excerpt": "Hey Alex — sounds good, shipping it today.", "demonstrates": "brevity"},
            {"excerpt": "Quick one: can we move Tuesday's sync to 3pm? Let me know.", "demonstrates": "informal asks"}
        ]"#;
        let ctx = build_voice_context(profile, examples).unwrap();
        assert!(ctx.contains("<voice-profile>"));
        assert!(ctx.contains("Terse, warm, direct."));
        assert!(ctx.contains("Hey [Name]"));
        assert!(ctx.contains("Typical length (words): 60"));
        assert!(ctx.contains("Distinctive phrases:"));
        assert!(ctx.contains("<voice-examples>"));
        assert!(ctx.contains("Example 1:"));
        assert!(ctx.contains("sounds good, shipping it today"));
    }

    #[test]
    fn build_voice_context_returns_none_on_bad_profile_json() {
        assert!(build_voice_context("not json", "[]").is_none());
    }

    #[test]
    fn build_voice_context_tolerates_bad_examples_json() {
        let profile = r#"{"summary": "x"}"#;
        let ctx = build_voice_context(profile, "not json").unwrap();
        assert!(ctx.contains("<voice-profile>"));
        assert!(!ctx.contains("<voice-examples>"));
    }

    #[test]
    fn truncate_excerpt_respects_max() {
        let s = "a ".repeat(300); // 600 chars
        let out = truncate_excerpt(&s, 100);
        assert!(out.len() <= 100);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn build_voice_context_full_uses_recipient_profile_when_present() {
        let account = r#"{"summary": "ACCOUNT-LEVEL profile.", "tone": "neutral"}"#;
        let recipient = r#"{"summary": "RECIPIENT-SPECIFIC profile.", "tone": "warm and casual"}"#;
        let ctx =
            build_voice_context_full(account, "[]", Some(recipient), None, &[], &[], &[]).unwrap();
        assert!(ctx.contains("RECIPIENT-SPECIFIC"));
        assert!(!ctx.contains("ACCOUNT-LEVEL"));
        assert!(ctx.contains("warm and casual"));
    }

    #[test]
    fn build_voice_context_full_falls_back_to_archetype_when_no_recipient() {
        let account = r#"{"summary": "ACCOUNT", "tone": "neutral"}"#;
        let archetype = r#"{"summary": "ARCHETYPE warm-personal", "tone": "warm"}"#;
        let ctx =
            build_voice_context_full(account, "[]", None, Some(archetype), &[], &[], &[]).unwrap();
        assert!(ctx.contains("ARCHETYPE warm-personal"));
        assert!(!ctx.contains("ACCOUNT"));
    }

    #[test]
    fn build_voice_context_full_falls_back_to_account_when_neither_present() {
        let account = r#"{"summary": "ACCOUNT only", "tone": "neutral"}"#;
        let ctx = build_voice_context_full(account, "[]", None, None, &[], &[], &[]).unwrap();
        assert!(ctx.contains("ACCOUNT only"));
    }

    #[test]
    fn build_voice_context_full_caps_raw_examples_when_recipient_profile_present() {
        let recipient = r#"{"summary": "RECIPIENT", "tone": "warm"}"#;
        let examples = vec![
            "first raw sample".to_string(),
            "second raw sample".to_string(),
            "third raw sample".to_string(),
        ];
        let ctx = build_voice_context_full(
            r#"{"summary": "ACCOUNT"}"#,
            "[]",
            Some(recipient),
            None,
            &examples,
            &[],
            &[],
        )
        .unwrap();
        assert!(ctx.contains("first raw sample"));
        assert!(!ctx.contains("second raw sample"));
        assert!(!ctx.contains("third raw sample"));
    }

    /// A pinned rule must precede everything it is supposed to override — a
    /// model reading "typical greeting: Hi [Name]" *before* the absolute has
    /// already formed the wrong plan.
    #[test]
    fn pinned_rules_lead_the_context_and_are_marked_absolute() {
        let recipient = r#"{"summary": "warm", "typical_greeting": "Hi [Name], or Hey [Name]"}"#;
        let ctx = build_voice_context_full(
            r#"{"summary": "ACCOUNT"}"#,
            "[]",
            Some(recipient),
            None,
            &["a raw sample".to_string()],
            &["a learned preference".to_string()],
            &["Always address as \"Bro. Ellis\", never \"Sam\".".to_string()],
        )
        .unwrap();

        assert!(ctx.starts_with("<pinned-rules>"), "got: {ctx}");
        assert!(ctx.contains("Bro. Ellis"));
        assert!(ctx.to_uppercase().contains("ABSOLUTE"));
        assert!(ctx.contains("OVERRIDE"));
        let pinned_at = ctx.find("Bro. Ellis").unwrap();
        for later in ["<recent-replies-to-this-recipient>", "<voice-profile>", "<learned-preferences>"] {
            assert!(
                ctx.find(later).unwrap() > pinned_at,
                "{later} must come after the pinned rules: {ctx}"
            );
        }
    }

    /// The common case for a new contact: a rule but no profile of any kind.
    /// Returning None here would silently drop the user's instruction.
    #[test]
    fn a_pinned_rule_alone_still_produces_a_context() {
        let ctx = build_voice_context_full(
            "not json",
            "[]",
            None,
            None,
            &[],
            &[],
            &["Never open with \"Hope you're well\".".to_string()],
        )
        .expect("a pinned rule is reason enough to build a context");
        assert!(ctx.contains("Hope you're well"));
        assert!(!ctx.contains("<voice-profile>"));
    }

    #[test]
    fn no_pinned_rules_emits_no_block() {
        let ctx =
            build_voice_context_full(r#"{"summary": "x"}"#, "[]", None, None, &[], &[], &[]).unwrap();
        assert!(!ctx.contains("<pinned-rules>"));
    }

    /// Blank entries must not produce an empty directive list that reads as
    /// "there are absolute rules" while naming none.
    #[test]
    fn blank_pinned_rules_are_dropped_entirely() {
        let ctx = build_voice_context_full(
            r#"{"summary": "x"}"#,
            "[]",
            None,
            None,
            &[],
            &[],
            &["   ".to_string()],
        )
        .unwrap();
        assert!(!ctx.contains("<pinned-rules>"), "got: {ctx}");
    }

    /* ── truncate ───────────────────────────────────────────── */

    /// `&text[..max]` with only a length guard is a panic, not a parse — the
    /// same mistake as gotcha #21b. Every char here is 3 bytes, so a cut at 10
    /// lands mid-sequence.
    #[test]
    fn truncate_cuts_on_a_char_boundary_not_a_byte_offset() {
        let s = "日本語のテキストです";
        assert_eq!(truncate(s, 10), "日本語");
        assert_eq!(truncate(s, 10).len(), 9);
    }

    #[test]
    fn truncate_leaves_short_text_alone() {
        assert_eq!(truncate("hello", 6000), "hello");
        assert_eq!(truncate("héllo", 100), "héllo");
    }

    #[test]
    fn truncate_can_return_empty_rather_than_panicking() {
        // max lands inside the very first character.
        assert_eq!(truncate("日", 1), "");
    }

    /* ── parse_draft_findings ───────────────────────────────── */

    const BODY: &str = "Hi Sam,\n\nBoth CRM asks are live. It comes to $20 a month, flat.\n\nHere is the link to set it up.";

    fn one(json: &str) -> Vec<DraftFinding> {
        parse_draft_findings(json, BODY).expect("valid json")
    }

    #[test]
    fn an_empty_array_is_a_clean_draft_not_an_error() {
        assert_eq!(one("[]"), vec![]);
    }

    #[test]
    fn strips_a_markdown_fence() {
        let wrapped = "```json\n[]\n```";
        assert_eq!(one(wrapped), vec![]);
    }

    #[test]
    fn non_json_is_an_error_not_a_silent_empty_list() {
        // Degrading to "no findings" would report a clean bill of health for
        // a review that never ran.
        assert!(parse_draft_findings("I could not review this draft.", BODY).is_err());
    }

    #[test]
    fn keeps_a_quote_that_occurs_in_the_draft() {
        let json = r#"[{"category":"next_step","severity":"info","where":"Closing",
            "title":"No ask","detail":"Ends on information.",
            "quote":"Here is the link to set it up.","suggestion":"Let me know if the 1st works."}]"#;
        assert_eq!(one(json)[0].quote.as_deref(), Some("Here is the link to set it up."));
    }

    /// The guard that matters most: the frontend turns `quote` into a
    /// find-and-replace against the live document, so a paraphrased quote
    /// yields an Apply button that silently does nothing.
    #[test]
    fn drops_a_quote_the_model_paraphrased() {
        let json = r#"[{"category":"clarity","severity":"warning","where":"Body",
            "title":"Vague","detail":"x",
            "quote":"Here's the link for setting it up","suggestion":"y"}]"#;
        let f = &one(json)[0];
        assert_eq!(f.quote, None, "a near-miss quote must not survive");
        assert!(f.suggestion.is_some(), "the suggestion itself is still useful");
    }

    #[test]
    fn matches_a_quote_across_different_wrapping() {
        // The draft's plain text may wrap differently than the model echoes it.
        let json = r#"[{"category":"clarity","severity":"warning","where":"Body",
            "title":"x","detail":"y","quote":"Both CRM asks   are\nlive.","suggestion":null}]"#;
        assert!(one(json)[0].quote.is_some());
    }

    #[test]
    fn unknown_severity_and_category_fall_back_to_safe_values() {
        let json = r#"[{"category":"vibes","severity":"catastrophic","where":"Body",
            "title":"x","detail":"y","quote":null,"suggestion":null}]"#;
        let f = &one(json)[0];
        assert_eq!(f.severity, "warning");
        assert_eq!(f.category, "clarity");
    }

    #[test]
    fn a_blank_location_defaults_rather_than_rendering_empty() {
        let json = r#"[{"category":"tone","severity":"info","where":"  ",
            "title":"x","detail":"y","quote":null,"suggestion":null}]"#;
        assert_eq!(one(json)[0].location, "Body");
    }

    #[test]
    fn drops_findings_with_no_title() {
        let json = r#"[{"category":"tone","severity":"info","where":"Body",
            "title":"   ","detail":"y","quote":null,"suggestion":null}]"#;
        assert_eq!(one(json).len(), 0);
    }

    #[test]
    fn blank_suggestions_are_dropped() {
        let json = r#"[{"category":"tone","severity":"info","where":"Body",
            "title":"x","detail":"y","quote":null,"suggestion":"  "}]"#;
        assert_eq!(one(json)[0].suggestion, None);
    }

    #[test]
    fn caps_a_runaway_response() {
        let item = r#"{"category":"tone","severity":"info","where":"Body","title":"x","detail":"y","quote":null,"suggestion":null}"#;
        let json = format!("[{}]", vec![item; 20].join(","));
        assert_eq!(one(&json).len(), MAX_FINDINGS);
    }
}
