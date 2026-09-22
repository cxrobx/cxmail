use crate::error::AppError;
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub const MAX_PROMPT_CHARS: usize = 50_000;
pub const MIN_SAMPLES_REQUIRED: usize = 3;

pub const MAX_SAMPLES: usize = 50;
pub const MAX_CHARS_PER_SAMPLE: usize = 2000;
pub const MIN_CHARS_AFTER_STRIP: usize = 500;
pub const RECIPIENT_SAMPLES: usize = 3;
pub const MAX_RECIPIENT_SAMPLE_CHARS: usize = 700;
pub const RECIPIENT_EXTRACT_SAMPLES: usize = 16;
pub const RECIPIENT_REFRESH_THRESHOLD: i64 = 10;
// Bucketing is local and incremental, so the initial migration deliberately
// scans the full available history. Only the balanced 16-message learning set
// leaves the device for extraction.
pub const RECIPIENT_HISTORY_SCAN_SAMPLES: usize = usize::MAX;
pub const RECIPIENT_MAX_STYLE_BUCKETS: usize = 8;
pub const RECIPIENT_BUCKET_SIMILARITY_THRESHOLD: f64 = 0.78;
pub const ARCHETYPE_MAX_SAMPLES: usize = 100;
pub const ARCHETYPE_SAMPLE_CHARS: usize = 800;

pub struct ExtractionResult {
    pub voice_profile_json: String,
    pub voice_examples_json: String,
    pub model_used: String,
}

#[derive(Deserialize)]
struct VoiceExtractionResponse {
    voice_profile: serde_json::Value,
    #[serde(default = "empty_json_array")]
    voice_examples: serde_json::Value,
}

fn empty_json_array() -> serde_json::Value {
    serde_json::Value::Array(Vec::new())
}

const VOICE_MODEL_PROMPT: &str = r#"Analyze the following email samples (all written by the same person) and produce a detailed email-writing voice profile.

<email-samples>
{SAMPLES}
</email-samples>

Return a JSON object with exactly this structure:
{
  "voice_profile": {
    "summary": "2-3 sentence overview of how this person writes email",
    "tone": "description of tone and emotional register",
    "sentence_patterns": "characteristic sentence structures, rhythm, typical length",
    "vocabulary_level": "vocabulary range and distinctive word choices",
    "distinctive_phrases": ["phrase 1", "phrase 2", "phrase 3"],
    "typical_greeting": "how they usually open emails (e.g. 'Hi [Name],', no greeting, 'Hey —')",
    "typical_signoff": "how they usually close (e.g. 'Best,', 'Thanks,', often nothing)",
    "typical_length_words": 80,
    "formality": "casual | neutral | formal",
    "punctuation_style": "comma and colon habits, semicolons, emoji use, exclamation frequency",
    "what_to_avoid": "what does NOT sound like this person"
  },
  "voice_examples": [
    {
      "excerpt": "verbatim passage (40-200 words) from the samples",
      "demonstrates": "what this excerpt shows about the voice"
    }
  ]
}

Include 3-5 voice_examples chosen as the most representative passages. Prefer short, self-contained emails.
Return ONLY the JSON object, no markdown fences, no prose."#;

/// Extract a voice profile from pre-cleaned sent-mail samples.
pub async fn extract_profile(
    client: &crate::email::inference::InferenceClient,
    samples: &[String],
) -> Result<ExtractionResult, AppError> {
    if samples.len() < MIN_SAMPLES_REQUIRED {
        return Err(AppError::AiService(format!(
            "Need at least {} samples, got {}",
            MIN_SAMPLES_REQUIRED,
            samples.len()
        )));
    }

    let concat = concat_samples(samples, MAX_PROMPT_CHARS);
    let system =
        "You are a writing-style analyst. You return only raw JSON — no prose, no markdown fences.";
    let user = VOICE_MODEL_PROMPT.replace("{SAMPLES}", &concat);

    let raw = crate::email::ai::call_inference(client, system, &user, 2000, 0.3).await?;

    let json = extract_json_object(&raw).ok_or_else(|| {
        let head: String = raw.chars().take(300).collect();
        AppError::AiService(format!("voice extraction: no JSON in response: {}", head))
    })?;

    let parsed = parse_voice_extraction_response(&json)?;

    let voice_profile_json = serde_json::to_string(&parsed.voice_profile)
        .map_err(|e| AppError::AiService(format!("voice_profile serialize: {}", e)))?;
    let voice_examples_json = serde_json::to_string(&parsed.voice_examples)
        .map_err(|e| AppError::AiService(format!("voice_examples serialize: {}", e)))?;

    Ok(ExtractionResult {
        voice_profile_json,
        voice_examples_json,
        model_used: client.model().to_string(),
    })
}

fn parse_voice_extraction_response(json: &str) -> Result<VoiceExtractionResponse, AppError> {
    let mut parsed: VoiceExtractionResponse = serde_json::from_str(json)
        .map_err(|e| AppError::AiService(format!("voice extraction: invalid JSON: {e}")))?;
    if !parsed.voice_profile.is_object() {
        return Err(AppError::AiService(
            "voice extraction: voice_profile must be a JSON object".to_string(),
        ));
    }
    // A profile that says the author "uses em-dashes" is a true statement about
    // the past and a standing argument against the pinned rule that forbids
    // them — and `build_voice_context_full` feeds it in as the style to match.
    // Both profile paths (account and per-recipient) come through here.
    //
    // `voice_examples` is deliberately NOT scrubbed: those are verbatim
    // excerpts of mail actually sent. Governing what gets written next is one
    // thing; rewriting the record of what someone wrote is another.
    crate::email::dashes::scrub_value(&mut parsed.voice_profile);
    Ok(parsed)
}

fn concat_samples(samples: &[String], max_chars: usize) -> String {
    let mut out = String::new();
    for (i, s) in samples.iter().enumerate() {
        let header = format!("Email {}:\n", i + 1);
        let separator = if out.is_empty() { "" } else { "\n\n---\n\n" };
        if out.len() + separator.len() + header.len() + s.len() > max_chars {
            let remaining = max_chars.saturating_sub(out.len() + separator.len() + header.len());
            if remaining > 200 {
                out.push_str(separator);
                out.push_str(&header);
                out.push_str(&truncate_on_word_boundary(s, remaining));
            }
            break;
        }
        out.push_str(separator);
        out.push_str(&header);
        out.push_str(s);
    }
    out
}

#[derive(Deserialize, Debug, Clone)]
pub struct InsightItem {
    pub insight: String,
    pub prompt_addition: String,
    #[serde(default)]
    pub source_edit_count: i64,
    #[serde(default)]
    pub confidence: f64,
}

const LEARNING_EXTRACTION_PROMPT: &str = r#"You are analyzing how a user edits AI-generated email drafts to learn their preferences. Below are pairs of (ai_draft, final_sent_version). Extract RECURRING patterns in how the user modifies these drafts.

<edit-pairs>
{EDITS}
</edit-pairs>

Return a JSON array of insights. Each insight should capture a pattern visible in 2+ edits — not a one-off change. For each, return an object:
{
  "insight": "short human-readable description (max 120 chars)",
  "prompt_addition": "concrete imperative rule to append to a future system prompt (max 150 chars). Start with a verb. Examples: 'Lead with the ask instead of the context.' 'Omit the sign-off when writing to internal colleagues.' 'Never use the word leverage.'",
  "source_edit_count": <integer — number of edits showing this pattern>,
  "confidence": <float 0.0-1.0>
}

Rules:
- Ignore one-off edits, typos, and context-specific changes (names, dates, numbers, project details).
- Each prompt_addition must be a GENERAL rule the AI should follow in FUTURE drafts.
- Confidence should reflect how consistently the pattern appears (1.0 = always; 0.7 = usually).
- Return an empty array [] if no clear patterns emerge.

Return ONLY the JSON array, no prose, no markdown fences."#;

/// Extract recurring edit patterns from (ai_draft, sent_body) pairs via LLM.
pub async fn extract_learning_insights(
    client: &crate::email::inference::InferenceClient,
    edits: &[crate::db::voice_edits::DraftEditRow],
) -> Result<Vec<InsightItem>, AppError> {
    if edits.len() < 3 {
        return Err(AppError::AiService(format!(
            "Need at least 3 edits to extract insights, got {}",
            edits.len()
        )));
    }

    // Render edit pairs, capping total prompt size
    let mut rendered = String::new();
    for (i, e) in edits.iter().enumerate() {
        let ai = truncate_on_word_boundary(&e.ai_draft, 800);
        let sent = truncate_on_word_boundary(&e.sent_body, 800);
        let block = format!(
            "\n--- Edit {} (similarity {:.2}) ---\nAI DRAFT:\n{}\n\nFINAL SENT:\n{}\n",
            i + 1,
            e.similarity,
            ai,
            sent
        );
        if rendered.len() + block.len() > MAX_PROMPT_CHARS {
            break;
        }
        rendered.push_str(&block);
    }

    let system = "You are a pattern-extraction analyst. You return only raw JSON arrays — no prose, no markdown fences.";
    let user = LEARNING_EXTRACTION_PROMPT.replace("{EDITS}", &rendered);

    let raw = crate::email::ai::call_inference(client, system, &user, 1500, 0.2).await?;

    let json = extract_json_array(&raw).ok_or_else(|| {
        let head: String = raw.chars().take(300).collect();
        AppError::AiService(format!(
            "insight extraction: no JSON array in response: {}",
            head
        ))
    })?;

    let items: Vec<InsightItem> = serde_json::from_str(&json)
        .map_err(|e| AppError::AiService(format!("insight extraction: invalid JSON: {}", e)))?;

    Ok(items)
}

fn extract_json_array(text: &str) -> Option<String> {
    let t = text.trim();
    if serde_json::from_str::<serde_json::Value>(t)
        .ok()
        .map(|v| v.is_array())
        .unwrap_or(false)
    {
        return Some(t.to_string());
    }
    // Strip markdown fences (input was already trimmed above, so trim_end_matches works)
    let stripped = t
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if serde_json::from_str::<serde_json::Value>(stripped)
        .ok()
        .map(|v| v.is_array())
        .unwrap_or(false)
    {
        return Some(stripped.to_string());
    }
    let start = t.find('[')?;
    let end = t.rfind(']')?;
    if end > start {
        let slice = &t[start..=end];
        if serde_json::from_str::<serde_json::Value>(slice)
            .ok()
            .map(|v| v.is_array())
            .unwrap_or(false)
        {
            return Some(slice.to_string());
        }
    }
    None
}

fn extract_json_object(text: &str) -> Option<String> {
    let t = text.trim();
    if serde_json::from_str::<serde_json::Value>(t)
        .ok()
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        return Some(t.to_string());
    }
    // Strip markdown fences (trim first so trailing whitespace doesn't block trim_end_matches)
    let stripped = t
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if serde_json::from_str::<serde_json::Value>(stripped)
        .ok()
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        return Some(stripped.to_string());
    }
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end > start {
        let slice = &t[start..=end];
        if serde_json::from_str::<serde_json::Value>(slice)
            .ok()
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            return Some(slice.to_string());
        }
    }
    None
}

/// Pull recent sent messages for an account, strip quotes/signatures, return clean samples.
pub fn collect_sent_samples(conn: &Connection, account_id: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT mb.plain_text
         FROM messages m
         JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
         JOIN message_bodies mb
           ON mb.account_id = m.account_id
          AND mb.folder_name = m.folder_name
          AND mb.uid = m.uid
         WHERE m.account_id = ?1
           AND f.folder_type = 'sent'
           AND mb.plain_text IS NOT NULL
           AND LENGTH(mb.plain_text) > 200
         ORDER BY datetime(m.date) DESC
         LIMIT ?2",
    )?;

    let fetch_limit = (MAX_SAMPLES as i64) * 3;
    let raw: Vec<String> = stmt
        .query_map(rusqlite::params![account_id, fetch_limit], |row| {
            row.get::<_, Option<String>>(0)
        })?
        .filter_map(|r| r.ok().flatten())
        .collect();

    let mut cleaned = Vec::with_capacity(MAX_SAMPLES);
    for body in raw {
        let stripped = strip_email_noise(&body);
        if stripped.len() < MIN_CHARS_AFTER_STRIP {
            continue;
        }
        cleaned.push(truncate_on_word_boundary(&stripped, MAX_CHARS_PER_SAMPLE));
        if cleaned.len() >= MAX_SAMPLES {
            break;
        }
    }
    Ok(cleaned)
}

/// One sent-mail sample addressed to a recipient — used by both reply context
/// (which only needs `body`) and extraction/archetype paths (which also need
/// folder/uid for archetype assignment and `date` for the staleness watermark).
#[derive(Debug, Clone)]
pub struct RecipientSample {
    pub folder_name: String,
    pub uid: u32,
    pub date: String,
    pub body: String,
}

/// Collect recent sent messages that were addressed to `to_email`.
///
/// Queries `message_bodies.to_json` (parsed addresses) — NOT `messages.to_list`,
/// which is hardcoded to `"[]"` in the IMAP sync path and is unusable.
///
/// `since_date` is an ISO 8601 cutoff (empty string = no cutoff). Returns up to
/// `max` cleaned samples in newest-first order.
pub fn collect_samples_for_recipient_detailed(
    conn: &Connection,
    account_id: &str,
    to_email: &str,
    max: usize,
    since_date: &str,
) -> Result<Vec<RecipientSample>, AppError> {
    if to_email.trim().is_empty() {
        return Ok(Vec::new());
    }
    let needle = to_email.trim().to_lowercase();

    // Match against the canonical address list. Prefer `messages.to_list`
    // (populated at IMAP sync time from the envelope as of v34); fall back to
    // `message_bodies.to_json` for old prefetched messages that may pre-date
    // the envelope-parsing fix and never got backfilled.
    let mut stmt = conn.prepare(
        "SELECT m.folder_name, m.uid, m.date, mb.plain_text
         FROM messages m
         JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
         JOIN message_bodies mb
           ON mb.account_id = m.account_id
          AND mb.folder_name = m.folder_name
          AND mb.uid = m.uid
         WHERE m.account_id = ?1
           AND f.folder_type = 'sent'
           AND mb.plain_text IS NOT NULL
           AND LENGTH(mb.plain_text) > 100
           AND (
             EXISTS (
               SELECT 1 FROM json_each(COALESCE(NULLIF(m.to_list, ''), '[]')) je
               WHERE LOWER(json_extract(je.value, '$.email')) = ?2
             )
             OR (
               mb.to_json IS NOT NULL AND EXISTS (
                 SELECT 1 FROM json_each(mb.to_json) je
                 WHERE LOWER(json_extract(je.value, '$.email')) = ?2
               )
             )
           )
           AND (?3 = '' OR datetime(m.date) > datetime(?3))
         ORDER BY datetime(m.date) DESC
         LIMIT ?4",
    )?;

    let fetch_limit = if max == usize::MAX {
        i64::MAX
    } else {
        max.saturating_mul(3).min(i64::MAX as usize) as i64
    };
    let mut out = Vec::with_capacity(max.min(1024));
    let rows = stmt.query_map(
        rusqlite::params![account_id, needle, since_date, fetch_limit],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        },
    )?;
    for row in rows {
        let Ok((folder_name, uid, date, body)) = row else {
            continue;
        };
        let stripped = strip_email_noise(&body);
        if stripped.len() < 80 {
            continue;
        }
        out.push(RecipientSample {
            folder_name,
            uid,
            date,
            body: truncate_on_word_boundary(&stripped, MAX_RECIPIENT_SAMPLE_CHARS),
        });
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

/// Convenience wrapper for the reply-context use case (body strings only).
pub fn collect_samples_for_recipient(
    conn: &Connection,
    account_id: &str,
    to_email: &str,
    max: usize,
) -> Result<Vec<String>, AppError> {
    let detailed = collect_samples_for_recipient_detailed(conn, account_id, to_email, max, "")?;
    Ok(detailed.into_iter().map(|s| s.body).collect())
}

const STYLE_FEATURE_DIMS: usize = 15;
const SEMANTIC_FEATURE_DIMS: usize = 48;

#[derive(Debug, Clone)]
pub struct RecipientLearningSample {
    pub sample: RecipientSample,
    pub bucket_id: String,
    pub bucket_sample_count: i64,
}

#[derive(Debug, Clone)]
pub struct RecipientLearningSet {
    pub samples: Vec<RecipientLearningSample>,
    pub source_sample_count: i64,
    pub bucket_count: usize,
    pub newest_message_date: String,
}

fn normalize_slice(values: &mut [f64]) {
    let norm = values.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm > f64::EPSILON {
        for value in values {
            *value /= norm;
        }
    }
}

fn stable_feature_slot(token: &str) -> usize {
    // FNV-1a keeps persisted feature vectors stable across Rust versions and
    // processes (DefaultHasher does not promise that).
    let mut hash = 0xcbf29ce484222325u64;
    for byte in token.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash as usize) % SEMANTIC_FEATURE_DIMS
}

fn style_semantic_feature(text: &str) -> Vec<f64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .filter(|word| !word.is_empty())
        .collect();
    let word_count = words.len().max(1) as f64;
    let sentence_count = text
        .chars()
        .filter(|c| matches!(c, '.' | '!' | '?'))
        .count()
        .max(1) as f64;
    let lines: Vec<&str> = text.lines().collect();
    let nonblank_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let line_count = nonblank_lines.len().max(1) as f64;
    let paragraph_count = text
        .split("\n\n")
        .filter(|part| !part.trim().is_empty())
        .count()
        .max(1) as f64;
    let bullet_lines = nonblank_lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("• ")
                || trimmed
                    .split_once(". ")
                    .map(|(prefix, _)| prefix.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
        })
        .count() as f64;
    let label_lines = nonblank_lines
        .iter()
        .filter(|line| {
            line.split_once(':')
                .map(|(label, _)| {
                    let words = label.split_whitespace().count();
                    (1..=5).contains(&words)
                })
                .unwrap_or(false)
        })
        .count() as f64;
    let punctuation_rate =
        |needle: char| text.chars().filter(|c| *c == needle).count() as f64 / sentence_count;
    let contractions = words
        .iter()
        .filter(|word| word.contains('\'') || word.contains('’'))
        .count() as f64;
    let first_person = words
        .iter()
        .filter(|word| {
            matches!(
                **word,
                "i" | "i'm" | "i’ve" | "i've" | "we" | "we're" | "our"
            )
        })
        .count() as f64;
    let second_person = words
        .iter()
        .filter(|word| matches!(**word, "you" | "you're" | "you’ve" | "you've" | "your"))
        .count() as f64;
    let starts_with_greeting = [
        "hi ",
        "hi,",
        "hey ",
        "hey,",
        "hello ",
        "hello,",
        "good morning",
        "good afternoon",
    ]
    .iter()
    .any(|prefix| lower.trim_start().starts_with(prefix));
    let tail: String = lower
        .chars()
        .rev()
        .take(180)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let has_signoff = [
        "\nthanks,",
        "\nthank you,",
        "\nbest,",
        "\ncheers,",
        "\nregards,",
    ]
    .iter()
    .any(|marker| tail.contains(marker));

    let mut feature = vec![0.0; STYLE_FEATURE_DIMS + SEMANTIC_FEATURE_DIMS];
    feature[..STYLE_FEATURE_DIMS].copy_from_slice(&[
        (word_count.ln_1p() / 6.0).min(1.0),
        (word_count / sentence_count / 30.0).min(1.0),
        (word_count / paragraph_count / 80.0).min(1.0),
        bullet_lines / line_count,
        label_lines / line_count,
        punctuation_rate('?').min(1.0),
        punctuation_rate('!').min(1.0),
        punctuation_rate(':').min(1.0),
        punctuation_rate(';').min(1.0),
        (text.matches('—').count() as f64 / sentence_count).min(1.0),
        (contractions / word_count * 12.0).min(1.0),
        (first_person / word_count * 12.0).min(1.0),
        (second_person / word_count * 12.0).min(1.0),
        if starts_with_greeting { 1.0 } else { 0.0 },
        if has_signoff { 1.0 } else { 0.0 },
    ]);

    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "had", "has",
        "have", "he", "her", "his", "i", "in", "is", "it", "its", "me", "my", "of", "on", "or",
        "our", "she", "that", "the", "their", "them", "they", "this", "to", "was", "we", "were",
        "will", "with", "you", "your",
    ];
    for word in words {
        if word.len() < 3 || STOP_WORDS.contains(&word) {
            continue;
        }
        let slot = STYLE_FEATURE_DIMS + stable_feature_slot(word);
        feature[slot] += 1.0;
    }
    normalize_slice(&mut feature[..STYLE_FEATURE_DIMS]);
    normalize_slice(&mut feature[STYLE_FEATURE_DIMS..]);
    feature
}

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let a_norm = a.iter().map(|v| v * v).sum::<f64>().sqrt();
    let b_norm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
    if a_norm <= f64::EPSILON || b_norm <= f64::EPSILON {
        0.0
    } else {
        (dot / (a_norm * b_norm)).clamp(0.0, 1.0)
    }
}

fn voice_feature_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() < STYLE_FEATURE_DIMS || b.len() < STYLE_FEATURE_DIMS {
        return cosine_similarity(a, b);
    }
    let style = cosine_similarity(&a[..STYLE_FEATURE_DIMS], &b[..STYLE_FEATURE_DIMS]);
    let semantic = cosine_similarity(&a[STYLE_FEATURE_DIMS..], &b[STYLE_FEATURE_DIMS..]);
    0.7 * style + 0.3 * semantic
}

fn updated_centroid(current: &[f64], sample_count: i64, sample: &[f64]) -> Vec<f64> {
    if sample_count <= 0 || current.len() != sample.len() {
        return sample.to_vec();
    }
    let old_weight = sample_count as f64;
    current
        .iter()
        .zip(sample)
        .map(|(old, new)| (old * old_weight + new) / (old_weight + 1.0))
        .collect()
}

fn sync_recipient_style_buckets(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    history: &[RecipientSample],
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    let mut buckets = crate::db::voice_buckets::list_buckets(&tx, account_id, recipient_email)?;
    let mut assigned =
        crate::db::voice_buckets::assigned_message_keys(&tx, account_id, recipient_email)?;

    // Oldest-first makes initial clustering stable. Later calls only assign
    // previously unseen messages and never rewrite an older lesson's bucket.
    for sample in history.iter().rev() {
        let key = (sample.folder_name.clone(), sample.uid);
        if assigned.contains(&key) {
            continue;
        }
        let feature = style_semantic_feature(&sample.body);
        let closest = buckets
            .iter()
            .enumerate()
            .map(|(index, bucket)| (index, voice_feature_similarity(&feature, &bucket.centroid)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        let (bucket_index, similarity) = match closest {
            Some((index, similarity))
                if similarity >= RECIPIENT_BUCKET_SIMILARITY_THRESHOLD
                    || buckets.len() >= RECIPIENT_MAX_STYLE_BUCKETS =>
            {
                (index, similarity)
            }
            _ => {
                let bucket_id = uuid::Uuid::new_v4().to_string();
                crate::db::voice_buckets::insert_bucket(
                    &tx,
                    account_id,
                    recipient_email,
                    &bucket_id,
                    &feature,
                    &sample.date,
                )?;
                buckets.push(crate::db::voice_buckets::RecipientVoiceBucketRow {
                    bucket_id,
                    centroid: feature.clone(),
                    sample_count: 0,
                    first_message_date: sample.date.clone(),
                    last_message_date: sample.date.clone(),
                });
                (buckets.len() - 1, 1.0)
            }
        };

        let bucket = &mut buckets[bucket_index];
        if crate::db::voice_buckets::insert_assignment(
            &tx,
            account_id,
            recipient_email,
            &sample.folder_name,
            sample.uid,
            &sample.date,
            &bucket.bucket_id,
            similarity,
            &feature,
        )? {
            bucket.centroid = updated_centroid(&bucket.centroid, bucket.sample_count, &feature);
            bucket.sample_count += 1;
            if sample.date < bucket.first_message_date {
                bucket.first_message_date = sample.date.clone();
            }
            if sample.date > bucket.last_message_date {
                bucket.last_message_date = sample.date.clone();
            }
            crate::db::voice_buckets::update_bucket_after_assignment(
                &tx,
                account_id,
                recipient_email,
                &bucket.bucket_id,
                &bucket.centroid,
                bucket.sample_count,
                &bucket.first_message_date,
                &bucket.last_message_date,
            )?;
            assigned.insert(key);
        }
    }
    tx.commit()?;
    Ok(())
}

fn select_recipient_learning_set(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    max: usize,
) -> Result<RecipientLearningSet, AppError> {
    let buckets = crate::db::voice_buckets::list_buckets(conn, account_id, recipient_email)?;
    let centroids: HashMap<String, Vec<f64>> = buckets
        .iter()
        .map(|bucket| (bucket.bucket_id.clone(), bucket.centroid.clone()))
        .collect();
    let rows = crate::db::voice_buckets::list_bucketed_samples(conn, account_id, recipient_email)?;

    let mut grouped: HashMap<String, Vec<RecipientLearningSample>> = HashMap::new();
    let mut features: HashMap<(String, u32), Vec<f64>> = HashMap::new();
    let mut newest_message_date = String::new();
    for row in rows {
        let stripped = strip_email_noise(&row.body);
        if stripped.len() < 80 {
            continue;
        }
        if row.date > newest_message_date {
            newest_message_date = row.date.clone();
        }
        features.insert((row.folder_name.clone(), row.uid), row.feature);
        grouped
            .entry(row.bucket_id.clone())
            .or_default()
            .push(RecipientLearningSample {
                sample: RecipientSample {
                    folder_name: row.folder_name,
                    uid: row.uid,
                    date: row.date,
                    body: truncate_on_word_boundary(&stripped, MAX_RECIPIENT_SAMPLE_CHARS),
                },
                bucket_id: row.bucket_id,
                bucket_sample_count: row.bucket_sample_count,
            });
    }

    let source_sample_count = grouped.values().map(|samples| samples.len() as i64).sum();
    let mut bucket_ids: Vec<String> = grouped.keys().cloned().collect();
    bucket_ids.sort_by(|a, b| {
        let a_count = grouped
            .get(a)
            .and_then(|samples| samples.first())
            .map(|sample| sample.bucket_sample_count)
            .unwrap_or(0);
        let b_count = grouped
            .get(b)
            .and_then(|samples| samples.first())
            .map(|sample| sample.bucket_sample_count)
            .unwrap_or(0);
        b_count.cmp(&a_count).then_with(|| a.cmp(b))
    });

    let mut selected = Vec::new();
    let mut selected_keys: HashSet<(String, u32)> = HashSet::new();

    // First reserve one medoid-like representative from every durable group.
    for bucket_id in &bucket_ids {
        if selected.len() >= max {
            break;
        }
        let Some(samples) = grouped.get(bucket_id) else {
            continue;
        };
        let Some(centroid) = centroids.get(bucket_id) else {
            continue;
        };
        let representative = samples.iter().max_by(|a, b| {
            let a_feature = features
                .get(&(a.sample.folder_name.clone(), a.sample.uid))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let b_feature = features
                .get(&(b.sample.folder_name.clone(), b.sample.uid))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            voice_feature_similarity(a_feature, centroid)
                .total_cmp(&voice_feature_similarity(b_feature, centroid))
        });
        if let Some(sample) = representative {
            selected_keys.insert((sample.sample.folder_name.clone(), sample.sample.uid));
            selected.push(sample.clone());
        }
    }

    // Then round-robin through each group's newest remaining messages. A large,
    // recent group cannot crowd an older but still-valid style out of the prompt.
    let mut offset = 0usize;
    while selected.len() < max {
        let mut added = false;
        for bucket_id in &bucket_ids {
            let Some(samples) = grouped.get(bucket_id) else {
                continue;
            };
            let Some(sample) = samples.get(offset) else {
                continue;
            };
            let key = (sample.sample.folder_name.clone(), sample.sample.uid);
            if selected_keys.insert(key) {
                selected.push(sample.clone());
                added = true;
                if selected.len() >= max {
                    break;
                }
            }
        }
        if !added && grouped.values().all(|samples| samples.len() <= offset + 1) {
            break;
        }
        offset += 1;
    }

    Ok(RecipientLearningSet {
        samples: selected,
        source_sample_count,
        bucket_count: grouped.len(),
        newest_message_date,
    })
}

/// Incrementally assign newly-seen sent messages into durable style/semantic
/// groups, then return a balanced extraction set spanning the full history.
pub fn build_recipient_learning_set(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<RecipientLearningSet, AppError> {
    let history = collect_samples_for_recipient_detailed(
        conn,
        account_id,
        recipient_email,
        RECIPIENT_HISTORY_SCAN_SAMPLES,
        "",
    )?;
    sync_recipient_style_buckets(conn, account_id, recipient_email, &history)?;
    select_recipient_learning_set(conn, account_id, recipient_email, RECIPIENT_EXTRACT_SAMPLES)
}

/// Representative examples for drafting. Once buckets exist this is diverse
/// across long-lived styles rather than simply being the last N messages.
pub fn collect_diverse_samples_for_recipient(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
    max: usize,
) -> Result<Vec<String>, AppError> {
    if crate::db::voice_buckets::assignment_count(conn, account_id, recipient_email)? == 0 {
        return collect_samples_for_recipient(conn, account_id, recipient_email, max);
    }
    let selected = select_recipient_learning_set(conn, account_id, recipient_email, max)?;
    Ok(selected
        .samples
        .into_iter()
        .map(|sample| sample.sample.body)
        .collect())
}

/// Collect cleaned plain-text samples assigned to a specific archetype.
/// Joins `message_archetype` to `message_bodies`. Used by the MCP tool to
/// surface representative excerpts for non-recipient sources.
pub fn collect_samples_for_archetype(
    conn: &Connection,
    account_id: &str,
    archetype_id: &str,
    max: usize,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT mb.plain_text
         FROM message_archetype ma
         JOIN message_bodies mb
           ON mb.account_id = ma.account_id
          AND mb.folder_name = ma.folder_name
          AND mb.uid = ma.uid
         JOIN messages m
           ON m.account_id = ma.account_id
          AND m.folder_name = ma.folder_name
          AND m.uid = ma.uid
         WHERE ma.account_id = ?1
           AND ma.archetype_id = ?2
           AND mb.plain_text IS NOT NULL
           AND LENGTH(mb.plain_text) > 100
         ORDER BY datetime(m.date) DESC
         LIMIT ?3",
    )?;
    let fetch_limit = (max as i64) * 3;
    let raw: Vec<String> = stmt
        .query_map(
            rusqlite::params![account_id, archetype_id, fetch_limit],
            |row| row.get::<_, Option<String>>(0),
        )?
        .filter_map(|r| r.ok().flatten())
        .collect();

    let mut cleaned = Vec::with_capacity(max);
    for body in raw {
        let stripped = strip_email_noise(&body);
        if stripped.len() < 80 {
            continue;
        }
        cleaned.push(truncate_on_word_boundary(
            &stripped,
            MAX_RECIPIENT_SAMPLE_CHARS,
        ));
        if cleaned.len() >= max {
            break;
        }
    }
    Ok(cleaned)
}

/// Outcome of extracting a per-recipient voice profile from collected samples.
#[derive(Debug, Clone)]
pub struct RecipientExtractionOutcome {
    pub profile_json: String,
    pub model_used: String,
    /// Total durable history represented by the style buckets, not merely the
    /// capped number of examples sent to the model.
    pub sample_count: i64,
    /// ISO 8601 date — the newest assigned source message.
    /// Stored as the staleness watermark.
    pub last_extracted_message_date: String,
}

const RECIPIENT_PROFILE_PROMPT: &str = r#"Analyze how one author writes to a particular recipient. The samples are selected from durable style/similarity groups across their full sent history, not just the newest messages.

<previous-profile>
{PREVIOUS_PROFILE}
</previous-profile>

<grouped-email-samples>
{SAMPLES}
</grouped-email-samples>

Return a JSON object with this structure:
{
  "voice_profile": {
    "summary": "2-3 sentence overview of how this person writes to this recipient",
    "tone": "tone and emotional register",
    "sentence_patterns": "characteristic sentence structures and rhythm",
    "vocabulary_level": "vocabulary range and distinctive word choices",
    "distinctive_phrases": ["phrase 1", "phrase 2"],
    "typical_greeting": "usual opening",
    "typical_signoff": "usual closing",
    "typical_length_words": 80,
    "formality": "casual | neutral | formal",
    "punctuation_style": "punctuation habits",
    "style_modes": [
      {
        "group": "group 1",
        "when_used": "the apparent situation where this mode is used",
        "traits": "format, tone, and structural traits that distinguish it"
      }
    ],
    "durable_observations": ["recurring lesson supported across the history"],
    "what_to_avoid": "what does NOT sound like this person"
  },
  "voice_examples": []
}

Rules:
- Treat every style group as a lasting mode unless newer evidence clearly contradicts it.
- Preserve supported observations from the previous profile; refine or remove one only when the grouped history contradicts it.
- Capture structural habits such as bullets, short labels, update formatting, greetings, and sign-offs.
- Distinguish recurring style from one-off subject matter.
- Return the full merged profile, not a delta.
- Return ONLY the JSON object, no markdown fences or prose."#;

/// Run extraction over a balanced, durable learning set. The caller must
/// release any DB lock before awaiting this network call (gotcha #11).
pub async fn extract_recipient_profile_from_learning_set(
    client: &crate::email::inference::InferenceClient,
    learning_set: &RecipientLearningSet,
    previous_profile: Option<&str>,
) -> Result<RecipientExtractionOutcome, AppError> {
    if learning_set.samples.len() < MIN_SAMPLES_REQUIRED {
        return Err(AppError::AiService(format!(
            "Need at least {} samples for recipient profile, got {}",
            MIN_SAMPLES_REQUIRED,
            learning_set.samples.len()
        )));
    }

    let mut group_numbers: HashMap<&str, usize> = HashMap::new();
    let mut rendered = String::new();
    for learning_sample in &learning_set.samples {
        let next_group = group_numbers.len() + 1;
        let group_number = *group_numbers
            .entry(learning_sample.bucket_id.as_str())
            .or_insert(next_group);
        let block = format!(
            "\n--- group {} ({} messages represented) ---\n{}\n",
            group_number, learning_sample.bucket_sample_count, learning_sample.sample.body
        );
        if rendered.len() + block.len() > MAX_PROMPT_CHARS {
            break;
        }
        rendered.push_str(&block);
    }

    let previous = previous_profile
        .filter(|profile| !profile.trim().is_empty())
        .unwrap_or("{}");
    let user = RECIPIENT_PROFILE_PROMPT
        .replace("{PREVIOUS_PROFILE}", previous)
        .replace("{SAMPLES}", &rendered);
    let system =
        "You are a writing-style analyst maintaining a cumulative profile. Return only raw JSON.";
    let raw = crate::email::ai::call_inference(client, system, &user, 2400, 0.2).await?;
    let json = extract_json_object(&raw).ok_or_else(|| {
        let head: String = raw.chars().take(300).collect();
        AppError::AiService(format!(
            "recipient voice extraction: no JSON in response: {head}"
        ))
    })?;
    let parsed = parse_voice_extraction_response(&json)?;
    let profile_json = serde_json::to_string(&parsed.voice_profile)
        .map_err(|e| AppError::AiService(format!("voice_profile serialize: {e}")))?;

    Ok(RecipientExtractionOutcome {
        profile_json,
        model_used: client.model().to_string(),
        sample_count: learning_set.source_sample_count,
        last_extracted_message_date: learning_set.newest_message_date.clone(),
    })
}

/// Whether a cached recipient profile is stale enough to rebuild, based on
/// the count of sent messages to that recipient newer than the watermark.
/// Returns true when no cached profile exists OR when the count crosses
/// `RECIPIENT_REFRESH_THRESHOLD`.
pub fn should_refresh_recipient(
    conn: &Connection,
    account_id: &str,
    recipient_email: &str,
) -> Result<bool, AppError> {
    let cached = crate::db::voice_profiles_recipient::get(conn, account_id, recipient_email)?;
    let watermark = match cached {
        Some(row) if row.learning_version >= 1 => row.last_extracted_message_date,
        Some(_) => return Ok(true),
        None => return Ok(true),
    };
    let new_count = count_sent_to_recipient_since(conn, account_id, recipient_email, &watermark)?;
    Ok(new_count >= RECIPIENT_REFRESH_THRESHOLD)
}

/// One sample row used in archetype clustering. The synthetic `id` field
/// (e.g. "s3") is what the LLM sees and round-trips back; raw UIDs never
/// leave the server.
#[derive(Debug, Clone)]
pub struct ArchetypeSample {
    pub id: String,
    pub folder_name: String,
    pub uid: u32,
    pub body: String,
}

/// One archetype produced by clustering, ready to write to the DB.
#[derive(Debug, Clone)]
pub struct ClusteredArchetype {
    pub archetype_id: String,
    pub name: String,
    pub description: String,
    pub profile_json: String,
    /// `(folder_name, uid)` tuples to write into `message_archetype`.
    pub assignments: Vec<(String, u32)>,
}

/// Pull up to `max` recent sent messages for an account with folder/uid metadata.
/// Used as the input set to archetype clustering.
pub fn collect_sent_samples_for_clustering(
    conn: &Connection,
    account_id: &str,
    max: usize,
) -> Result<Vec<ArchetypeSample>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT m.folder_name, m.uid, mb.plain_text
         FROM messages m
         JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
         JOIN message_bodies mb
           ON mb.account_id = m.account_id
          AND mb.folder_name = m.folder_name
          AND mb.uid = m.uid
         WHERE m.account_id = ?1
           AND f.folder_type = 'sent'
           AND mb.plain_text IS NOT NULL
           AND LENGTH(mb.plain_text) > 200
         ORDER BY datetime(m.date) DESC
         LIMIT ?2",
    )?;

    let fetch_limit = (max as i64) * 2;
    let raw: Vec<(String, u32, String)> = stmt
        .query_map(rusqlite::params![account_id, fetch_limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut out = Vec::with_capacity(max);
    for (folder_name, uid, body) in raw {
        let stripped = strip_email_noise(&body);
        if stripped.len() < 200 {
            continue;
        }
        let id = format!("s{}", out.len() + 1);
        out.push(ArchetypeSample {
            id,
            folder_name,
            uid,
            body: truncate_on_word_boundary(&stripped, ARCHETYPE_SAMPLE_CHARS),
        });
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

const ARCHETYPE_PROMPT: &str = r#"You will analyze sent emails from one author and cluster them by writing style. Return 3-6 archetypes that capture the distinct ways this author writes (e.g. "warm personal", "terse ops to internal team", "prospect outreach").

Each email is tagged with a synthetic ID like [id: s3]. You MUST refer back to those IDs in your response.

<email-samples>
{SAMPLES}
</email-samples>

Return a JSON array. Each element:
{
  "name": "short descriptive label (max 40 chars)",
  "description": "one sentence on when this style is used (max 160 chars)",
  "profile": {
    "summary": "2-3 sentence overview of the writing style in this archetype",
    "tone": "tone and emotional register",
    "typical_greeting": "how emails of this style usually open (or '' if none)",
    "typical_signoff": "how they usually close (or '')",
    "typical_length_words": 60,
    "formality": "casual | neutral | formal",
    "punctuation_style": "punctuation/emoji habits",
    "distinctive_phrases": ["phrase 1", "phrase 2"],
    "what_to_avoid": "what does NOT sound like this archetype"
  },
  "sample_ids": ["s3", "s7", "s12"]
}

Rules:
- Each sample_id must appear in exactly one archetype.
- Use only IDs that appeared in <email-samples>.
- Aim for 3-6 archetypes; do not produce more than 6 even if you see fine-grained variation.
- Return ONLY the JSON array — no markdown, no prose."#;

#[derive(Deserialize, Debug)]
struct ArchetypeLLMItem {
    name: String,
    #[serde(default)]
    description: String,
    profile: serde_json::Value,
    #[serde(default)]
    sample_ids: Vec<String>,
}

/// Cluster sent samples into archetypes via one LLM call. Caller releases the
/// DB lock before awaiting this — extraction is network-bound.
/// Each returned archetype has a server-generated UUID and a list of
/// `(folder_name, uid)` assignments validated against the input set.
pub async fn cluster_archetypes(
    client: &crate::email::inference::InferenceClient,
    samples: &[ArchetypeSample],
) -> Result<Vec<ClusteredArchetype>, AppError> {
    if samples.len() < MIN_SAMPLES_REQUIRED {
        return Err(AppError::AiService(format!(
            "Need at least {} samples to cluster archetypes, got {}",
            MIN_SAMPLES_REQUIRED,
            samples.len()
        )));
    }

    let mut rendered = String::new();
    for s in samples {
        let block = format!("\n[id: {}]\n{}\n", s.id, s.body);
        if rendered.len() + block.len() > MAX_PROMPT_CHARS {
            break;
        }
        rendered.push_str(&block);
    }

    let system =
        "You are a writing-style cluster analyst. Return only raw JSON arrays — no prose, no markdown fences.";
    let user = ARCHETYPE_PROMPT.replace("{SAMPLES}", &rendered);

    let raw = crate::email::ai::call_inference(client, system, &user, 3000, 0.3).await?;

    let json = extract_json_array(&raw).ok_or_else(|| {
        let head: String = raw.chars().take(300).collect();
        AppError::AiService(format!(
            "archetype clustering: no JSON array in response: {}",
            head
        ))
    })?;

    let items: Vec<ArchetypeLLMItem> = serde_json::from_str(&json)
        .map_err(|e| AppError::AiService(format!("archetype clustering: invalid JSON: {}", e)))?;

    let id_to_meta: std::collections::HashMap<&str, (&str, u32)> = samples
        .iter()
        .map(|s| (s.id.as_str(), (s.folder_name.as_str(), s.uid)))
        .collect();

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let mut assignments: Vec<(String, u32)> = Vec::new();
        for sid in &item.sample_ids {
            if let Some((folder, uid)) = id_to_meta.get(sid.as_str()) {
                assignments.push((folder.to_string(), *uid));
            }
            // unknown IDs from the LLM are silently dropped — server is the source of truth
        }
        if assignments.is_empty() {
            continue;
        }
        let mut profile = item.profile;
        crate::email::dashes::scrub_value(&mut profile);
        let profile_json = serde_json::to_string(&profile)
            .map_err(|e| AppError::AiService(format!("archetype profile serialize: {}", e)))?;
        out.push(ClusteredArchetype {
            archetype_id: uuid::Uuid::new_v4().to_string(),
            name: item.name.trim().to_string(),
            description: item.description.trim().to_string(),
            profile_json,
            assignments,
        });
    }
    Ok(out)
}

/// Count sent messages to `to_email` whose date is strictly after `since_date`.
/// Used to decide whether a cached recipient profile is stale enough to refresh.
pub fn count_sent_to_recipient_since(
    conn: &Connection,
    account_id: &str,
    to_email: &str,
    since_date: &str,
) -> Result<i64, AppError> {
    if to_email.trim().is_empty() {
        return Ok(0);
    }
    let needle = to_email.trim().to_lowercase();
    // Same to_list/to_json fallback as collect_samples_for_recipient_detailed.
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM messages m
         JOIN folders f ON f.account_id = m.account_id AND f.name = m.folder_name
         LEFT JOIN message_bodies mb
           ON mb.account_id = m.account_id
          AND mb.folder_name = m.folder_name
          AND mb.uid = m.uid
         WHERE m.account_id = ?1
           AND f.folder_type = 'sent'
           AND datetime(m.date) > datetime(?3)
           AND (
             EXISTS (
               SELECT 1 FROM json_each(COALESCE(NULLIF(m.to_list, ''), '[]')) je
               WHERE LOWER(json_extract(je.value, '$.email')) = ?2
             )
             OR (
               mb.to_json IS NOT NULL AND EXISTS (
                 SELECT 1 FROM json_each(mb.to_json) je
                 WHERE LOWER(json_extract(je.value, '$.email')) = ?2
               )
             )
           )",
        rusqlite::params![account_id, needle, since_date],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn truncate_on_word_boundary(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // Reserve 3 chars for the ellipsis so total len <= max.
    let target = max.saturating_sub(3);
    if target == 0 {
        return text[..max].to_string();
    }
    let cutoff = text[..target].rfind(char::is_whitespace).unwrap_or(target);
    let mut out = text[..cutoff].trim_end().to_string();
    out.push_str("...");
    out
}

/// Strip common email noise: quoted replies, signatures, forwarded blocks.
/// Heuristic — aims to recover the user's own writing; some noise may remain.
pub fn strip_email_noise(text: &str) -> String {
    // 1. Cut at signature delimiter "\n-- \n" or "\n--\n"
    let mut end = text.len();
    for delim in ["\n-- \n", "\n--\n", "\n-- \r\n"] {
        if let Some(i) = text.find(delim) {
            if i < end {
                end = i;
            }
        }
    }

    // 2. Cut at common forward/original-message markers
    for marker in [
        "\n-----Original Message-----",
        "\n---------- Forwarded message",
        "\n--------- Forwarded message",
        "\nBegin forwarded message:",
        "\n________________________________", // Outlook horizontal rule
    ] {
        if let Some(i) = text.find(marker) {
            if i < end {
                end = i;
            }
        }
    }

    // 3. Cut at attribution lines: "On <date>, <name> wrote:" + locale variants
    let body = &text[..end];
    if let Some(i) = find_attribution_line(body) {
        end = i;
    }
    let body = &text[..end];

    // 4. Drop lines starting with '>' (inline quotes)
    let mut kept = String::with_capacity(body.len());
    for line in body.split('\n') {
        if line.trim_start().starts_with('>') {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }

    // 5. Collapse runs of blank lines
    collapse_blank_lines(&kept).trim().to_string()
}

fn find_attribution_line(body: &str) -> Option<usize> {
    let suffixes: &[&str] = &[
        " wrote:",
        " a écrit :",
        " a écrit:",
        " schrieb:",
        " ha scritto:",
        " escribió:",
        " scrisse:",
        " napisał(a):",
    ];
    let prefixes: &[&str] = &["On ", "Am ", "Le ", "Il ", "El ", "Den "];

    let mut offset = 0usize;
    for line in body.split('\n') {
        let trimmed = line.trim();
        let has_suffix = suffixes.iter().any(|s| trimmed.ends_with(s));
        let has_prefix = prefixes.iter().any(|p| trimmed.starts_with(p));
        if has_suffix && has_prefix {
            return Some(offset);
        }
        offset += line.len() + 1; // +1 for the '\n' we split on
    }
    None
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_blank = false;
    for line in s.split('\n') {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !last_blank {
                out.push('\n');
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
        last_blank = is_blank;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn recipient_sample_body(label: &str) -> String {
        format!(
            "{label}: This is a sufficiently detailed sent email sample with enough original author text to pass the recipient voice collection thresholds. It should survive noise stripping and be available as a representative voice sample."
        )
    }

    fn recipient_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            PRAGMA foreign_keys=ON;
            CREATE TABLE accounts (
                id TEXT PRIMARY KEY
            );
            CREATE TABLE folders (
                account_id TEXT NOT NULL,
                name TEXT NOT NULL,
                folder_type TEXT
            );
            CREATE TABLE messages (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                date TEXT NOT NULL,
                to_list TEXT,
                cc_list TEXT,
                UNIQUE(account_id, folder_name, uid)
            );
            CREATE TABLE message_bodies (
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                plain_text TEXT,
                to_json TEXT
            );
            CREATE TABLE voice_profiles_recipient (
                account_id TEXT NOT NULL,
                recipient_email TEXT NOT NULL,
                profile_json TEXT NOT NULL,
                sample_count INTEGER NOT NULL DEFAULT 0,
                last_extracted_message_date TEXT NOT NULL,
                model_used TEXT,
                generated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (account_id, recipient_email)
            );
            INSERT INTO accounts (id) VALUES ('acct');
            INSERT INTO folders (account_id, name, folder_type)
            VALUES ('acct', 'Sent', 'sent'), ('acct', 'INBOX', 'inbox');
            ",
        )
        .unwrap();
        crate::db::schema::migrate_v44_recipient_voice_buckets(&conn).unwrap();
        conn
    }

    fn insert_recipient_sample(conn: &Connection, uid: u32, date: &str, to_json: &str, body: &str) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, date)
             VALUES ('acct', 'Sent', ?1, ?2)",
            rusqlite::params![uid, date],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text, to_json)
             VALUES ('acct', 'Sent', ?1, ?2, ?3)",
            rusqlite::params![uid, body, to_json],
        )
        .unwrap();
    }

    #[test]
    fn strips_standard_reply_attribution() {
        let input = "Hi Sarah,\n\nThanks for the update — I agree with the plan.\n\nBest,\nChris\n\nOn Mon, Jan 15, 2026 at 3:14 PM, Sarah <sarah@x.com> wrote:\n> Can we move the meeting to Tuesday?\n> Let me know.\n";
        let out = strip_email_noise(input);
        assert!(out.contains("Thanks for the update"));
        assert!(!out.contains("move the meeting"));
        assert!(!out.contains("wrote:"));
    }

    #[test]
    fn strips_signature_after_dash_dash() {
        let input = "Hey!\n\nSounds good, see you then.\n\n-- \nChristopher Robinson\nhttps://example.com\n";
        let out = strip_email_noise(input);
        assert!(out.contains("Sounds good"));
        assert!(!out.contains("Christopher Robinson"));
    }

    #[test]
    fn drops_gt_prefixed_quote_lines() {
        let input = "Agreed, let's do it.\n\n> original text\n> more original\n";
        let out = strip_email_noise(input);
        assert!(out.contains("Agreed"));
        assert!(!out.contains("original text"));
    }

    #[test]
    fn cuts_at_forwarded_block() {
        let input = "FYI below — thought you'd want to see this.\n\n---------- Forwarded message ---------\nFrom: x@y.com\nSubject: hi\nBody here\n";
        let out = strip_email_noise(input);
        assert!(out.contains("FYI below"));
        assert!(!out.contains("Body here"));
    }

    #[test]
    fn handles_french_attribution() {
        let input = "Salut Jean,\n\nBien reçu, merci pour la note.\n\nLe 12 janv. 2026 à 10:00, Jean a écrit :\n> question ?\n";
        let out = strip_email_noise(input);
        assert!(out.contains("Bien reçu"));
        assert!(!out.contains("question"));
    }

    #[test]
    fn leaves_clean_email_intact() {
        let input = "Hi Alex, thanks for looping me in. Let's plan to meet next week.";
        let out = strip_email_noise(input);
        assert_eq!(out, input);
    }

    #[test]
    fn does_not_false_match_sentence_ending_wrote() {
        let input = "I wanted to share what Sarah wrote: the numbers look strong.";
        let out = strip_email_noise(input);
        assert!(out.contains("numbers look strong"));
    }

    #[test]
    fn cuts_at_outlook_horizontal_rule() {
        let input = "My take: ship it.\n\n________________________________\nFrom: someone@x.com\nSubject: old thread\n";
        let out = strip_email_noise(input);
        assert!(out.contains("ship it"));
        assert!(!out.contains("old thread"));
    }

    #[test]
    fn truncate_respects_word_boundary() {
        let s = "one two three four five six seven eight nine ten";
        let out = truncate_on_word_boundary(s, 20);
        assert!(out.ends_with("..."));
        assert!(!out.contains("eight"));
        assert!(out.starts_with("one two"));
    }

    #[test]
    fn extract_json_handles_code_fences() {
        let raw = "```json\n{\"voice_profile\": {\"tone\": \"warm\"}, \"voice_examples\": []}\n```";
        let out = extract_json_object(raw).unwrap();
        assert!(out.contains("\"tone\""));
    }

    #[test]
    fn extract_json_handles_preamble() {
        let raw = "Here is the JSON:\n{\"voice_profile\": {\"tone\": \"warm\"}, \"voice_examples\": []}\nThanks!";
        let out = extract_json_object(raw).unwrap();
        assert!(out.starts_with("{"));
        assert!(out.ends_with("}"));
    }

    #[test]
    fn extraction_response_tolerates_missing_voice_examples() {
        let parsed =
            parse_voice_extraction_response(r#"{"voice_profile":{"tone":"warm"}}"#).unwrap();
        assert_eq!(parsed.voice_profile["tone"], "warm");
        assert_eq!(parsed.voice_examples, serde_json::json!([]));
    }

    #[test]
    fn concat_samples_respects_char_limit() {
        let samples = vec!["a".repeat(500), "b".repeat(500), "c".repeat(500)];
        let out = concat_samples(&samples, 800);
        assert!(out.len() <= 800);
        assert!(out.starts_with("Email 1:"));
    }

    #[test]
    fn recipient_samples_use_parsed_to_json_exact_email_match() {
        let conn = recipient_test_db();
        let target_body = recipient_sample_body("target");
        let other_body = recipient_sample_body("other");
        insert_recipient_sample(
            &conn,
            1,
            "2026-01-03T10:00:00Z",
            r#"[{"name":"Taylor","email":"Taylor@Example.com"}]"#,
            &target_body,
        );
        insert_recipient_sample(
            &conn,
            2,
            "2026-01-04T10:00:00Z",
            r#"[{"name":"Other","email":"notaylor@example.com"}]"#,
            &other_body,
        );

        let samples =
            collect_samples_for_recipient(&conn, "acct", "taylor@example.com", 5).unwrap();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].contains("target"));

        let substring_match =
            collect_samples_for_recipient(&conn, "acct", "aylor@example.com", 5).unwrap();
        assert!(substring_match.is_empty());
    }

    #[test]
    fn recipient_refresh_count_uses_strict_since_date() {
        let conn = recipient_test_db();
        let body = recipient_sample_body("count");
        insert_recipient_sample(
            &conn,
            1,
            "2026-01-01T10:00:00Z",
            r#"[{"email":"taylor@example.com"}]"#,
            &body,
        );
        insert_recipient_sample(
            &conn,
            2,
            "2026-01-03T10:00:00Z",
            r#"[{"email":"taylor@example.com"}]"#,
            &body,
        );

        let count = count_sent_to_recipient_since(
            &conn,
            "acct",
            "taylor@example.com",
            "2026-01-02T00:00:00Z",
        )
        .unwrap();
        assert_eq!(count, 1);
    }

    /// Inserts a sent message whose addresses live ONLY in `messages.to_list`
    /// (not `message_bodies.to_json`) — the post-v34 canonical path.
    fn insert_recipient_via_messages_to_list(
        conn: &Connection,
        uid: u32,
        date: &str,
        to_list: &str,
        body: &str,
    ) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, date, to_list)
             VALUES ('acct', 'Sent', ?1, ?2, ?3)",
            rusqlite::params![uid, date, to_list],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text, to_json)
             VALUES ('acct', 'Sent', ?1, ?2, NULL)",
            rusqlite::params![uid, body],
        )
        .unwrap();
    }

    #[test]
    fn recipient_samples_match_via_messages_to_list_when_body_to_json_is_null() {
        let conn = recipient_test_db();
        let body = recipient_sample_body("via-to-list");
        insert_recipient_via_messages_to_list(
            &conn,
            1,
            "2026-02-01T10:00:00Z",
            r#"[{"name":"Taylor","email":"taylor@example.com"}]"#,
            &body,
        );
        let samples =
            collect_samples_for_recipient(&conn, "acct", "taylor@example.com", 5).unwrap();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].contains("via-to-list"));
    }

    #[test]
    fn recipient_count_matches_via_messages_to_list() {
        let conn = recipient_test_db();
        let body = recipient_sample_body("count2");
        insert_recipient_via_messages_to_list(
            &conn,
            1,
            "2026-02-05T10:00:00Z",
            r#"[{"email":"taylor@example.com"}]"#,
            &body,
        );
        let count = count_sent_to_recipient_since(
            &conn,
            "acct",
            "taylor@example.com",
            "2026-02-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn learning_set_keeps_an_old_style_group_after_many_newer_messages() {
        let conn = recipient_test_db();
        let recipient_json = r#"[{"email":"sam@example.com"}]"#;

        for uid in 1..=3u32 {
            let body = format!(
                "LEGACY_STRUCTURED_MODE\n\nWhat happened: The deployment interrupted the running search and left its profile locked.\n\nThree changes are live:\n- The worker now detects interrupted jobs and clears them automatically.\n- The status view explains stalls instead of spinning indefinitely.\n- Deployments no longer interrupt work already in progress.\n\nNothing was lost, and the user can retry whenever they are ready. Reference {uid}."
            );
            insert_recipient_sample(
                &conn,
                uid,
                &format!("2025-01-{uid:02}T10:00:00Z"),
                recipient_json,
                &body,
            );
        }
        let first = build_recipient_learning_set(&conn, "acct", "sam@example.com").unwrap();
        assert!(first
            .samples
            .iter()
            .any(|sample| sample.sample.body.contains("LEGACY_STRUCTURED_MODE")));

        for uid in 4..=513u32 {
            let body = format!(
                "Hey Sam,\n\nQuick note number {uid} — I saw your message and everything looks good on my side. I can take care of the remaining detail this afternoon, so there is nothing you need to do right now.\n\nThanks,\nChris"
            );
            insert_recipient_sample(&conn, uid, "2026-02-01T10:00:00Z", recipient_json, &body);
        }

        let updated = build_recipient_learning_set(&conn, "acct", "sam@example.com").unwrap();
        assert_eq!(updated.source_sample_count, 513);
        assert!(updated.bucket_count >= 2);
        assert_eq!(updated.samples.len(), RECIPIENT_EXTRACT_SAMPLES);
        assert!(
            updated
                .samples
                .iter()
                .any(|sample| sample.sample.body.contains("LEGACY_STRUCTURED_MODE")),
            "the older structured mode must retain a reserved representative"
        );
    }

    #[test]
    fn legacy_profile_stays_stale_until_bucketed_extraction_succeeds() {
        let conn = recipient_test_db();
        let recipient_json = r#"[{"email":"sam@example.com"}]"#;
        for uid in 1..=3u32 {
            insert_recipient_sample(
                &conn,
                uid,
                &format!("2026-01-{uid:02}T10:00:00Z"),
                recipient_json,
                &recipient_sample_body("legacy"),
            );
        }
        conn.execute(
            "INSERT INTO voice_profiles_recipient
                (account_id, recipient_email, profile_json, sample_count,
                 last_extracted_message_date, learning_version)
             VALUES ('acct', 'sam@example.com', '{}', 3,
                     '2026-01-03T10:00:00Z', 0)",
            [],
        )
        .unwrap();

        build_recipient_learning_set(&conn, "acct", "sam@example.com").unwrap();
        assert!(should_refresh_recipient(&conn, "acct", "sam@example.com").unwrap());

        crate::db::voice_profiles_recipient::upsert(
            &conn,
            "acct",
            "sam@example.com",
            r#"{"tone":"warm"}"#,
            "test-model",
            3,
            "2026-01-03T10:00:00Z",
        )
        .unwrap();
        assert!(!should_refresh_recipient(&conn, "acct", "sam@example.com").unwrap());
    }
}
