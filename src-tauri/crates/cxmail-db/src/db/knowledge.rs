//! Trigger-maintained email/attachment chunks and exact cosine retrieval.
use super::search::{SearchFilters, SearchResult};
use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

pub const CHUNK_CHARS: usize = 1_600;
pub const MAX_CHUNKS: usize = 64;

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    pub result: SearchResult,
    pub source: String,
    pub text: String,
    pub semantic: bool,
}
#[derive(Debug, Clone)]
pub struct PendingChunk {
    pub id: i64,
    pub message_id: i64,
    pub text: String,
    pub from_email: String,
    pub subject: String,
    pub date: String,
}

pub(crate) fn create_tables(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS knowledge_chunks(
        id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
        source TEXT NOT NULL, chunk_index INTEGER NOT NULL, text TEXT NOT NULL,
        embedding TEXT, embedding_key TEXT, UNIQUE(message_id,source,chunk_index)
    );
    CREATE INDEX IF NOT EXISTS idx_knowledge_message ON knowledge_chunks(message_id);
    CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts USING fts5(text, tokenize='porter unicode61 remove_diacritics 2');
    CREATE TRIGGER IF NOT EXISTS knowledge_ai AFTER INSERT ON knowledge_chunks BEGIN
      INSERT INTO knowledge_fts(rowid,text) VALUES(new.id,new.text); END;
    CREATE TRIGGER IF NOT EXISTS knowledge_ad AFTER DELETE ON knowledge_chunks BEGIN
      DELETE FROM knowledge_fts WHERE rowid=old.id; END;
    CREATE TRIGGER IF NOT EXISTS knowledge_au AFTER UPDATE OF text ON knowledge_chunks BEGIN
      UPDATE knowledge_fts SET text=new.text WHERE rowid=new.id; END;
    CREATE TRIGGER IF NOT EXISTS knowledge_body_ad AFTER DELETE ON message_bodies BEGIN
      DELETE FROM knowledge_chunks WHERE source='Email' AND message_id IN
        (SELECT id FROM messages WHERE account_id=old.account_id AND folder_name=old.folder_name AND uid=old.uid); END;
    CREATE TRIGGER IF NOT EXISTS knowledge_body_au AFTER UPDATE OF plain_text ON message_bodies BEGIN
      DELETE FROM knowledge_chunks WHERE source='Email' AND message_id IN
        (SELECT id FROM messages WHERE account_id=new.account_id AND folder_name=new.folder_name AND uid=new.uid)
        AND COALESCE(old.plain_text,'')<>COALESCE(new.plain_text,''); END;")
}

pub fn chunks(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().take(CHUNK_CHARS * MAX_CHUNKS).collect();
    chars
        .chunks(CHUNK_CHARS)
        .map(|x| x.iter().collect::<String>())
        .filter(|s| !s.trim().is_empty())
        .collect()
}

pub fn index_source(
    c: &Connection,
    message_id: i64,
    source: &str,
    text: &str,
) -> Result<(), AppError> {
    let parts = chunks(text);
    for (i, text) in parts.iter().enumerate() {
        c.execute("INSERT INTO knowledge_chunks(message_id,source,chunk_index,text) VALUES(?1,?2,?3,?4)
          ON CONFLICT(message_id,source,chunk_index) DO UPDATE SET text=excluded.text,
          embedding=CASE WHEN knowledge_chunks.text=excluded.text THEN knowledge_chunks.embedding ELSE NULL END,
          embedding_key=CASE WHEN knowledge_chunks.text=excluded.text THEN knowledge_chunks.embedding_key ELSE NULL END",
          params![message_id,source,i as i64,text])?;
    }
    c.execute(
        "DELETE FROM knowledge_chunks WHERE message_id=?1 AND source=?2 AND chunk_index>=?3",
        params![message_id, source, parts.len() as i64],
    )?;
    Ok(())
}

/// Bounded lazy backfill of cached bodies. Attachments are indexed when their
/// raw MIME bytes are next parsed or explicitly requested from the UI.
///
/// One transaction per batch: autocommitting every chunk insert made each a
/// separate commit while the caller held the app's DB mutex (gotcha #11).
/// Callers loop small batches and release the lock between them.
pub fn backfill(c: &Connection, limit: usize) -> Result<usize, AppError> {
    let tx = c.unchecked_transaction()?;
    let batch = tx
        .prepare(
            "SELECT m.id,b.plain_text FROM messages m JOIN message_bodies b
      ON b.account_id=m.account_id AND b.folder_name=m.folder_name AND b.uid=m.uid
      WHERE length(trim(COALESCE(b.plain_text,'')))>0 AND NOT EXISTS
        (SELECT 1 FROM knowledge_chunks k WHERE k.message_id=m.id AND k.source='Email')
      ORDER BY m.id DESC LIMIT ?1",
        )?
        .query_map([limit as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, text) in &batch {
        index_source(&tx, *id, "Email", text)?;
    }
    tx.commit()?;
    Ok(batch.len())
}

/// Chunks of opted-in, visible accounts. `pending` and `progress` share it so
/// the Settings count can never disagree with what the indexer will pick up.
fn eligible_sql() -> String {
    format!(
        "FROM knowledge_chunks k JOIN messages m ON m.id=k.message_id
      JOIN intelligence_accounts i ON i.account_id=m.account_id
      WHERE i.semantic_search=1 AND {}",
        super::accounts::visible_in_aggregates_sql("m.account_id")
    )
}

pub fn pending(c: &Connection, key: &str, limit: usize) -> Result<Vec<PendingChunk>, AppError> {
    Ok(c.prepare(&format!("SELECT k.id,k.message_id,k.text,COALESCE(m.from_email,''),COALESCE(m.subject,''),m.date
      {} AND (k.embedding IS NULL OR COALESCE(k.embedding_key,'')<>?1)
      ORDER BY datetime(m.date) DESC,k.id LIMIT ?2", eligible_sql()))?
      .query_map(params![key,limit as i64],|r|Ok(PendingChunk{id:r.get(0)?,message_id:r.get(1)?,text:r.get(2)?,from_email:r.get(3)?,subject:r.get(4)?,date:r.get(5)?}))?
      .collect::<rusqlite::Result<Vec<_>>>()?)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IndexProgress {
    /// Passages in accounts opted into meaning search.
    pub chunks: i64,
    /// Of those, searchable with the current endpoint and model.
    pub embedded: i64,
    /// Still to send. Withheld passages are done, so they count in neither.
    pub pending: i64,
}

pub fn progress(c: &Connection, key: &str) -> Result<IndexProgress, AppError> {
    Ok(c.query_row(
        &format!(
            "SELECT count(*),
              COALESCE(sum(k.embedding_key=?1 AND k.embedding IS NOT NULL AND k.embedding<>'[]'),0),
              COALESCE(sum(k.embedding IS NULL OR COALESCE(k.embedding_key,'')<>?1),0) {}",
            eligible_sql()
        ),
        [key],
        |r| {
            Ok(IndexProgress {
                chunks: r.get(0)?,
                embedded: r.get(1)?,
                pending: r.get(2)?,
            })
        },
    )?)
}

pub fn save_embedding(
    c: &Connection,
    chunk: &PendingChunk,
    key: &str,
    vector: &[f32],
) -> Result<bool, AppError> {
    validate_vector(vector)?;
    let json = serde_json::to_string(vector).map_err(|e| AppError::General(e.to_string()))?;
    Ok(c.execute(
        "UPDATE knowledge_chunks SET embedding=?1,embedding_key=?2 WHERE id=?3 AND text=?4
      AND EXISTS(SELECT 1 FROM intelligence_accounts i JOIN messages m ON m.account_id=i.account_id
                 WHERE m.id=knowledge_chunks.message_id AND i.semantic_search=1)",
        params![json, key, chunk.id, chunk.text],
    )? == 1)
}

pub fn validate_vector(v: &[f32]) -> Result<(), AppError> {
    if v.is_empty()
        || v.len() > 8192
        || v.iter().any(|x| !x.is_finite())
        || v.iter().all(|x| *x == 0.0)
    {
        return Err(AppError::AiService("Invalid embedding vector".into()));
    }
    Ok(())
}

/// Page one chronological union, so document-only hits neither disappear on
/// load-more nor displace keyword rows before OFFSET is applied.
pub fn lexical_search(
    c: &Connection,
    f: &SearchFilters,
    limit: u32,
    offset: u32,
    prefix: bool,
) -> Result<Vec<SearchResult>, AppError> {
    if f.account_ids.as_ref().is_some_and(|ids| ids.is_empty()) || limit == 0 {
        return Ok(Vec::new());
    }
    let Some(doc_expr) = f
        .keywords
        .as_deref()
        .and_then(|s| super::search::build_match_expr(s, prefix))
    else {
        return super::search::search(c, f, limit, offset, prefix);
    };
    let classic_expr = match &f.filename {
        Some(name) => format!(
            "{{attachment_names}}: \"{}\"* AND ({doc_expr})",
            name.replace('"', "\"\"")
        ),
        None => doc_expr.clone(),
    };
    let (mut conditions, mut bind, idx) = super::search::metadata_predicates(f, 1);
    let classic = idx;
    let document = idx + 1;
    let limit_idx = idx + 2;
    let offset_idx = idx + 3;
    bind.push(Box::new(classic_expr));
    bind.push(Box::new(doc_expr));
    bind.push(Box::new(limit));
    bind.push(Box::new(offset));
    conditions.push(format!("m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?{classic} UNION SELECT k.message_id FROM knowledge_chunks k JOIN knowledge_fts ON knowledge_fts.rowid=k.id WHERE knowledge_fts MATCH ?{document})"));
    let sql=format!("SELECT m.account_id,m.folder_name,m.uid,m.subject,m.from_name,m.from_email,m.date,m.is_read,m.has_attachments,
      COALESCE((SELECT k.source||': '||substr(k.text,1,220) FROM knowledge_chunks k JOIN knowledge_fts ON knowledge_fts.rowid=k.id WHERE k.message_id=m.id AND k.source<>'Email' AND knowledge_fts MATCH ?{document} ORDER BY k.chunk_index LIMIT 1),
      (SELECT snippet(messages_fts,-1,'{hs}','{he}',' … ',12) FROM messages_fts WHERE rowid=m.id AND messages_fts MATCH ?{classic}),m.snippet,'')
      FROM messages m WHERE {} ORDER BY datetime(m.date) DESC,m.id DESC LIMIT ?{limit_idx} OFFSET ?{offset_idx}",conditions.join(" AND "),hs=super::search::HIGHLIGHT_START,he=super::search::HIGHLIGHT_END);
    let bound: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|p| p.as_ref()).collect();
    Ok(c.prepare(&sql)?
        .query_map(bound.as_slice(), |r| {
            Ok(SearchResult {
                account_id: r.get(0)?,
                folder_name: r.get(1)?,
                uid: r.get(2)?,
                subject: r.get(3)?,
                from_name: r.get(4)?,
                from_email: r.get(5)?,
                date: r.get(6)?,
                is_read: r.get(7)?,
                has_attachments: r.get(8)?,
                snippet: super::search::clean_snippet_display(&r.get::<_, String>(9)?),
                score: 0.0,
                semantic_match: false,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || validate_vector(a).is_err() || validate_vector(b).is_err() {
        return None;
    }
    let (mut dot, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (&x, &y) in a.iter().zip(b) {
        dot += x as f64 * y as f64;
        aa += (x as f64).powi(2);
        bb += (y as f64).powi(2);
    }
    Some((dot / (aa * bb).sqrt()) as f32)
}

pub fn retrieve(
    c: &Connection,
    f: &SearchFilters,
    vector: Option<(&str, &[f32])>,
    limit: usize,
) -> Result<Vec<KnowledgeHit>, AppError> {
    if f.account_ids.as_ref().is_some_and(|ids| ids.is_empty()) {
        return Ok(Vec::new());
    }
    let (mut conditions, mut bind, mut idx) = super::search::metadata_predicates(f, 1);
    let lexical = vector.is_none();
    if let Some((key, v)) = vector {
        validate_vector(v)?;
        conditions.push(format!("k.embedding_key=?{idx} AND k.embedding IS NOT NULL AND EXISTS(SELECT 1 FROM intelligence_accounts i WHERE i.account_id=m.account_id AND i.semantic_search=1)"));
        bind.push(Box::new(key.to_string()));
        idx += 1;
    } else {
        let Some(expr) = f
            .keywords
            .as_deref()
            .and_then(|s| super::search::build_match_expr(s, false))
        else {
            return Ok(Vec::new());
        };
        conditions.push(format!("knowledge_fts MATCH ?{idx}"));
        bind.push(Box::new(expr));
        idx += 1;
    }
    let sql=format!("SELECT m.account_id,m.folder_name,m.uid,m.subject,m.from_name,m.from_email,m.date,m.is_read,m.has_attachments,
        k.source,k.text,k.embedding FROM knowledge_chunks k JOIN messages m ON m.id=k.message_id {} WHERE {} {}",
        if lexical{"JOIN knowledge_fts ON knowledge_fts.rowid=k.id"}else{""},conditions.join(" AND "),
        if lexical{format!("ORDER BY datetime(m.date) DESC LIMIT ?{idx}")}else{String::new()});
    if lexical {
        bind.push(Box::new((limit * 4) as i64));
    }
    let bound: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|x| x.as_ref()).collect();
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(bound.as_slice(), |r| {
        Ok((
            KnowledgeHit {
                result: SearchResult {
                    account_id: r.get(0)?,
                    folder_name: r.get(1)?,
                    uid: r.get(2)?,
                    subject: r.get(3)?,
                    from_name: r.get(4)?,
                    from_email: r.get(5)?,
                    date: r.get(6)?,
                    is_read: r.get(7)?,
                    has_attachments: r.get(8)?,
                    snippet: String::new(),
                    score: 0.0,
                    semantic_match: !lexical,
                },
                source: r.get(9)?,
                text: r.get(10)?,
                semantic: !lexical,
            },
            r.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut hits: HashMap<(String, String, u32), KnowledgeHit> = HashMap::new();
    for row in rows {
        let (mut hit, json) = row?;
        if let Some((_, query)) = vector {
            let Some(v) = json.and_then(|s| serde_json::from_str::<Vec<f32>>(&s).ok()) else {
                continue;
            };
            let Some(score) = cosine(query, &v) else {
                continue;
            };
            if score < 0.35 {
                continue;
            }
            hit.result.score = -score;
        }
        hit.result.snippet = format!(
            "{}: {}",
            hit.source,
            hit.text.chars().take(220).collect::<String>()
        );
        let key = (
            hit.result.account_id.clone(),
            hit.result.folder_name.clone(),
            hit.result.uid,
        );
        if hits
            .get(&key)
            .is_none_or(|old| hit.result.score < old.result.score)
        {
            hits.insert(key, hit);
        }
        // Keep memory bounded while streaming the index.
        if hits.len() > limit * 4 {
            let mut ordered: Vec<_> = hits.into_iter().collect();
            ordered.sort_by(|a, b| a.1.result.score.total_cmp(&b.1.result.score));
            ordered.truncate(limit * 2);
            hits = ordered.into_iter().collect();
        }
    }
    let mut hits: Vec<_> = hits.into_values().collect();
    hits.sort_by(|a, b| {
        a.result
            .score
            .total_cmp(&b.result.score)
            .then_with(|| b.result.date.cmp(&a.result.date))
    });
    hits.truncate(limit);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_require_finite_values_and_matching_dimensions() {
        assert!(cosine(&[1.0, 0.0], &[2.0, 0.0]).unwrap() > 0.99);
        assert!(cosine(&[1.0], &[1.0, 0.0]).is_none());
        assert!(validate_vector(&[f32::NAN]).is_err());
        assert!(validate_vector(&[0.0]).is_err());
    }
    #[test]
    fn unicode_chunks_never_split_codepoints_or_grow_without_bound() {
        let s = "é😎".repeat(100_000);
        let c = chunks(&s);
        assert_eq!(c.len(), MAX_CHUNKS);
        assert!(c.iter().all(|x| x.chars().count() <= CHUNK_CHARS));
    }
}
