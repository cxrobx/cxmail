use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftEditRow {
    pub id: i64,
    pub account_id: String,
    pub ai_draft: String,
    pub sent_body: String,
    pub similarity: f64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningInsightRow {
    pub id: i64,
    pub account_id: String,
    pub insight_text: String,
    pub prompt_addition: String,
    pub source_edit_count: i64,
    pub confidence: f64,
    pub is_active: bool,
    pub dedupe_key: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn insert_draft_edit(
    conn: &Connection,
    account_id: &str,
    ai_draft: &str,
    sent_body: &str,
    similarity: f64,
    recipient_email: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO ai_draft_edits (account_id, ai_draft, sent_body, similarity, recipient_email)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            account_id,
            ai_draft,
            sent_body,
            similarity,
            recipient_email.map(|e| e.to_lowercase()),
        ],
    )?;
    Ok(())
}

pub fn count_pending_edits(conn: &Connection, account_id: &str) -> Result<i64, AppError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM ai_draft_edits WHERE account_id = ?1 AND similarity < 0.95",
        params![account_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn list_recent_edits(
    conn: &Connection,
    account_id: &str,
    limit: usize,
) -> Result<Vec<DraftEditRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, ai_draft, sent_body, similarity, created_at
         FROM ai_draft_edits
         WHERE account_id = ?1 AND similarity < 0.95
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![account_id, limit as i64], |row| {
            Ok(DraftEditRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                ai_draft: row.get(2)?,
                sent_body: row.get(3)?,
                similarity: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Returns the `prompt_addition` strings of all currently-active insights, newest first.
pub fn list_active_insights(
    conn: &Connection,
    account_id: &str,
    limit: usize,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT prompt_addition FROM voice_learning_insights
         WHERE account_id = ?1 AND is_active = 1
         ORDER BY confidence DESC, source_edit_count DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![account_id, limit as i64], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn upsert_insight(
    conn: &Connection,
    account_id: &str,
    insight_text: &str,
    prompt_addition: &str,
    source_edit_count: i64,
    confidence: f64,
    dedupe_key: &str,
) -> Result<(), AppError> {
    let is_active = if source_edit_count >= 5 && confidence >= 0.8 {
        1
    } else {
        0
    };
    conn.execute(
        "INSERT INTO voice_learning_insights
           (account_id, insight_text, prompt_addition, source_edit_count, confidence, is_active, dedupe_key, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
         ON CONFLICT(account_id, dedupe_key) DO UPDATE SET
            insight_text = excluded.insight_text,
            prompt_addition = excluded.prompt_addition,
            source_edit_count = MAX(voice_learning_insights.source_edit_count, excluded.source_edit_count),
            confidence = excluded.confidence,
            is_active = excluded.is_active,
            updated_at = datetime('now')",
        params![
            account_id,
            insight_text,
            prompt_addition,
            source_edit_count,
            confidence,
            is_active,
            dedupe_key,
        ],
    )?;
    Ok(())
}

pub fn list_all_insights(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<LearningInsightRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, insight_text, prompt_addition, source_edit_count, confidence,
                is_active, dedupe_key, created_at, updated_at
         FROM voice_learning_insights
         WHERE account_id = ?1
         ORDER BY confidence DESC, source_edit_count DESC",
    )?;
    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok(LearningInsightRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                insight_text: row.get(2)?,
                prompt_addition: row.get(3)?,
                source_edit_count: row.get(4)?,
                confidence: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                dedupe_key: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Jaccard similarity on word sets. Returns 1.0 when texts are identical sets, 0.0 when disjoint.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let normalize = |s: &str| -> HashSet<String> {
        s.split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    let a_set = normalize(a);
    let b_set = normalize(b);
    if a_set.is_empty() && b_set.is_empty() {
        return 1.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    if union == 0.0 {
        return 0.0;
    }
    intersection / union
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_identical_texts() {
        assert!(
            (jaccard_similarity("Hi Alex, sounds good.", "Hi Alex, sounds good.") - 1.0).abs()
                < 0.001
        );
    }

    #[test]
    fn jaccard_no_overlap() {
        assert!(jaccard_similarity("apple banana", "carrot dragonfruit") < 0.01);
    }

    #[test]
    fn jaccard_partial_overlap() {
        let s = jaccard_similarity("hi alex sounds good", "hi alex sounds great");
        assert!(s > 0.4 && s < 0.9);
    }

    #[test]
    fn jaccard_case_insensitive() {
        assert!((jaccard_similarity("Hello World", "hello world") - 1.0).abs() < 0.001);
    }
}
