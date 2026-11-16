//! Read-only companion search over the shared desktop search engines.
use super::{embeddings::EmbeddingClient, search_parse};
use crate::{db, error::AppError};
use db::search::SearchResult;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

const CANDIDATES: usize = 500;
#[derive(Serialize)]
pub struct SearchPage {
    pub results: Vec<SearchResult>,
    pub semantic_status: String,
}
fn open(path: &Path) -> Result<Connection, AppError> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

/// Reciprocal rank fusion avoids comparing BM25 to cosine scores. Collapse
/// Gmail folder copies BEFORE assigning ranks, never across accounts.
fn fuse(
    c: &Connection,
    lanes: Vec<(f64, Vec<SearchResult>)>,
) -> Result<Vec<SearchResult>, AppError> {
    let mut fused: HashMap<String, (f64, SearchResult)> = HashMap::new();
    let mut identity = c.prepare(
        "SELECT message_id FROM messages WHERE account_id=?1 AND folder_name=?2 AND uid=?3",
    )?;
    for (weight, results) in lanes {
        let mut seen = HashSet::new();
        let mut rank = 0;
        for hit in results {
            let mid: Option<Option<String>> = identity
                .query_row(
                    rusqlite::params![hit.account_id, hit.folder_name, hit.uid],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(mid) = mid else {
                continue;
            };
            let key = match mid.filter(|s| !s.trim().is_empty()) {
                Some(mid) => format!(
                    "{}|{}",
                    hit.account_id,
                    mid.trim().trim_start_matches('<').trim_end_matches('>')
                ),
                None => format!("{}|{}|{}", hit.account_id, hit.folder_name, hit.uid),
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            rank += 1;
            let contribution = weight / (60.0 + rank as f64);
            match fused.get_mut(&key) {
                Some((score, old)) => {
                    *score += contribution;
                    old.semantic_match &= hit.semantic_match;
                }
                None => {
                    fused.insert(key, (contribution, hit));
                }
            }
        }
    }
    let mut results: Vec<_> = fused.into_values().collect();
    results.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.date.cmp(&a.1.date))
            .then_with(|| a.1.account_id.cmp(&b.1.account_id))
            .then_with(|| a.1.folder_name.cmp(&b.1.folder_name))
            .then_with(|| a.1.uid.cmp(&b.1.uid))
    });
    Ok(results.into_iter().map(|(_, r)| r).collect())
}

fn query_intent(query: &str) -> (String, bool) {
    let lower = query.to_lowercase();
    for phrase in ["most recent ", "latest ", "newest "] {
        if lower.starts_with(phrase) {
            return (query[phrase.len()..].trim().to_string(), true);
        }
    }
    (query.into(), false)
}

pub async fn search(
    path: &Path,
    query: &str,
    account: Option<&str>,
    offset: usize,
    semantic: bool,
) -> Result<SearchPage, AppError> {
    if query.trim().is_empty() || query.chars().count() > 2_000 || offset > CANDIDATES * 3 {
        return Err(AppError::General(
            "Enter a query of 1–2,000 characters".into(),
        ));
    }
    let (retrieval_query, newest) = query_intent(query);
    let parsed = search_parse::parse(&retrieval_query, chrono::Local::now().date_naive());
    let mut filters = parsed.filters;
    filters.exclude_bins = filters.folder.is_none();
    let c = open(path)?;
    let accounts = db::accounts::list(&c)?;
    filters.account_ids = Some(
        accounts
            .iter()
            .filter(|a| {
                a.is_active
                    && match account {
                        Some(id) => a.id == id,
                        None => !a.hidden_from_aggregates,
                    }
            })
            .map(|a| a.id.clone())
            .collect(),
    );
    let strict = query.contains('"') || query.split_whitespace().any(|s| s.starts_with('-'));
    let classic = db::search::ranked_search(&c, &filters, CANDIDATES as u32, !strict)?;
    let mut lanes = lexical_lanes(&c, &filters, classic, strict)?;
    let available = db::intelligence::options(&c)?.iter().any(|o| {
        o.semantic_search
            && filters
                .account_ids
                .as_ref()
                .is_some_and(|ids| ids.contains(&o.account_id))
    });
    // Preserve exact phrases and exclusions: similarity cannot override them.
    let words = filters.keywords.as_deref().unwrap_or("");
    let mut status = if !available {
        "disabled"
    } else if strict || words.trim().is_empty() {
        "exact"
    } else {
        "unavailable"
    }
    .to_string();
    drop(c);
    if semantic && available && !strict && !words.trim().is_empty() {
        if let Ok(client) = EmbeddingClient::load() {
            let key = client.key();
            let c = open(path)?;
            let progress = db::knowledge::progress_in_accounts(
                &c,
                &key,
                filters.account_ids.as_deref().unwrap_or(&[]),
            )?;
            drop(c);
            if progress.embedded == 0 {
                status = "indexing".into();
            } else {
                match tokio::time::timeout(
                    Duration::from_secs(12),
                    client.embed_query(&[words.chars().take(1_600).collect()]),
                )
                .await
                {
                    Ok(Ok(v)) => {
                        let c = open(path)?;
                        // retrieve rechecks opt-in and metadata after the await.
                        lanes.push((
                            1.0,
                            db::knowledge::retrieve(&c, &filters, Some((&key, &v[0])), CANDIDATES)?
                                .into_iter()
                                .filter(|h| h.result.score <= -client.minimum_similarity())
                                .map(|h| h.result)
                                .collect(),
                        ));
                        status = if progress.pending > 0 {
                            "partial"
                        } else {
                            "ready"
                        }
                        .into();
                    }
                    _ => log::warn!(
                        "Companion semantic search unavailable; retaining lexical results"
                    ),
                }
            }
        }
    }
    if !semantic {
        status = "keyword".into();
    }
    page(&open(path)?, lanes, offset, &status, newest)
}
fn lexical_lanes(
    c: &Connection,
    filters: &db::search::SearchFilters,
    classic: Vec<SearchResult>,
    strict: bool,
) -> Result<Vec<(f64, Vec<SearchResult>)>, AppError> {
    let mut lanes = vec![(1.0, classic)];
    // Passage-level NOT cannot enforce a message-wide exclusion: a forbidden
    // term can sit in another passage. Exact queries use the whole-message FTS.
    if !strict {
        lanes.push((
            0.6,
            db::knowledge::ranked_documents(c, filters, CANDIDATES)?
                .into_iter()
                .map(|h| h.result)
                .collect(),
        ));
    }
    Ok(lanes)
}

fn page(
    c: &Connection,
    lanes: Vec<(f64, Vec<SearchResult>)>,
    offset: usize,
    status: &str,
    newest: bool,
) -> Result<SearchPage, AppError> {
    let mut results = fuse(c, lanes)?;
    if newest {
        // Date intent applies to lexical matches. Keep looser semantic-only
        // suggestions below them, in relevance order, rather than letting a
        // newer unrelated meeting displace an exact XRF match.
        results.sort_by(|a, b| {
            a.semantic_match.cmp(&b.semantic_match).then_with(|| {
                if !a.semantic_match {
                    b.date.cmp(&a.date)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
    }
    Ok(SearchPage {
        results: results
            .into_iter()
            .skip(offset)
            .take(CANDIDATES * 3)
            .collect(),
        semantic_status: status.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::search::SearchFilters;

    /// Synthetic scale probe: no model or real-mail IO.
    #[test]
    #[ignore = "44k-vector scale benchmark; run explicitly"]
    fn full_mailbox_vector_scan_budget() {
        let c = fixture();
        let vector = (0..768)
            .map(|i| ((i as f32) * 0.1).sin())
            .collect::<Vec<_>>();
        let json = serde_json::to_string(&vector).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        for i in 1..=44_117 {
            tx.execute("INSERT INTO messages(account_id,folder_name,uid,subject,date,message_id) VALUES('a','INBOX',?1,'Scale probe','2026-11-16T12:00:00Z',?2)",rusqlite::params![i,format!("<{i}@example.com>")]).unwrap();
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO knowledge_chunks(message_id,source,chunk_index,text,embedding,embedding_key) VALUES(?1,'Email',0,'Fictional scale probe',?2,'scale')",rusqlite::params![id,json]).unwrap();
        }
        tx.commit().unwrap();
        let began = std::time::Instant::now();
        let hits = db::knowledge::retrieve(
            &c,
            &SearchFilters {
                account_ids: Some(vec!["a".into()]),
                ..Default::default()
            },
            Some(("scale", &vector)),
            500,
        )
        .unwrap();
        assert_eq!(hits.len(), 500);
        println!(
            "SCALE_PROBE passages=44117 dimensions=768 serialized_bytes={} scan_ms={}",
            json.len() * 44_117,
            began.elapsed().as_millis()
        );
    }
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        db::schema::initialize(&c).unwrap();
        for (id, hidden) in [("a", false), ("hidden", true)] {
            c.execute("INSERT INTO accounts(id,email,provider,imap_host,smtp_host,hidden_from_aggregates) VALUES(?1,?2,'imap','imap.example.com','smtp.example.com',?3)", rusqlite::params![id,format!("{id}@example.com"),hidden]).unwrap();
            for (folder, kind) in [
                ("INBOX", "inbox"),
                ("Archive", "archive"),
                ("Trash", "trash"),
            ] {
                c.execute(
                    "INSERT INTO folders(account_id,name,folder_type) VALUES(?1,?2,?3)",
                    rusqlite::params![id, folder, kind],
                )
                .unwrap();
            }
            db::intelligence::save_options(
                &c,
                &db::intelligence::AccountOptions {
                    account_id: id.into(),
                    semantic_search: true,
                    auto_draft: false,
                },
            )
            .unwrap();
        }
        c
    }
    fn mail(
        c: &Connection,
        account: &str,
        folder: &str,
        uid: u32,
        subject: &str,
        text: &str,
        mid: &str,
    ) {
        c.execute("INSERT INTO messages(account_id,folder_name,uid,subject,date,message_id) VALUES(?1,?2,?3,?4,'2026-11-16T12:00:00Z',?5)", rusqlite::params![account,folder,uid,subject,mid]).unwrap();
        let id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO message_bodies(account_id,folder_name,uid,plain_text) VALUES(?1,?2,?3,?4)",
            rusqlite::params![account, folder, uid, text],
        )
        .unwrap();
        db::knowledge::index_source(c, id, "Email", text).unwrap();
        c.execute("UPDATE knowledge_chunks SET embedding='[1.0,0.0]',embedding_key='model' WHERE message_id=?1",[id]).unwrap();
    }
    #[test]
    fn exact_exclusions_apply_to_entire_mail_not_individual_passages() {
        let c = fixture();
        let body = format!("budget {} confidential", "padding ".repeat(300));
        mail(&c, "a", "INBOX", 1, "Finance", &body, "<excluded>");
        let f = SearchFilters {
            keywords: Some("budget -confidential".into()),
            ..Default::default()
        };
        // The passage index alone leaks this mail. The whole-message lane
        // rejects it, and strict companion search must not put it back.
        assert!(!db::knowledge::ranked_documents(&c, &f, 50)
            .unwrap()
            .is_empty());
        let classic = db::search::ranked_search(&c, &f, 50, false).unwrap();
        assert!(classic.is_empty());
        assert!(fuse(&c, lexical_lanes(&c, &f, classic, true).unwrap())
            .unwrap()
            .is_empty());
    }
    #[test]
    fn latest_keeps_exact_project_hits_above_newer_semantic_suggestions() {
        let c = fixture();
        mail(
            &c,
            "a",
            "INBOX",
            1,
            "XRF meeting notes",
            "Decision summary.",
            "<xrf>",
        );
        mail(
            &c,
            "a",
            "INBOX",
            2,
            "Other meeting notes",
            "Unrelated project.",
            "<other>",
        );
        c.execute(
            "UPDATE messages SET date='2026-11-01T12:00:00Z' WHERE uid=1",
            [],
        )
        .unwrap();
        let f = SearchFilters {
            keywords: Some("XRF meeting notes".into()),
            ..Default::default()
        };
        let classic = db::search::ranked_search(&c, &f, 50, false).unwrap();
        let semantic = db::knowledge::retrieve(&c, &f, Some(("model", &[1.0, 0.0])), 50)
            .unwrap()
            .into_iter()
            .map(|h| h.result)
            .collect();
        let page = page(&c, vec![(1.0, classic), (1.0, semantic)], 0, "ready", true).unwrap();
        assert_eq!(
            page.results.iter().map(|r| r.uid).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
    #[test]
    fn latest_is_sort_intent_not_a_required_keyword() {
        assert_eq!(
            query_intent("most recent XRF meeting notes"),
            ("XRF meeting notes".into(), true)
        );
        assert_eq!(query_intent("latest invoice"), ("invoice".into(), true));
        assert_eq!(
            query_intent("from:alex@example.com newest"),
            ("from:alex@example.com newest".into(), false)
        );
    }
    #[test]
    fn hybrid_keeps_semantic_only_mail_and_fuses_dual_matches_first() {
        let c = fixture();
        mail(
            &c,
            "a",
            "INBOX",
            1,
            "Hotel booking",
            "Confirmation for your accommodation stay.",
            "<hotel>",
        );
        mail(
            &c,
            "a",
            "INBOX",
            2,
            "Lodging receipt",
            "Lodging receipt for your stay.",
            "<receipt>",
        );
        mail(
            &c,
            "a",
            "INBOX",
            3,
            "Dinner",
            "Restaurant reservation.",
            "<dinner>",
        );
        c.execute("UPDATE knowledge_chunks SET embedding='[0.0,1.0]' WHERE message_id=(SELECT id FROM messages WHERE uid=3)",[]).unwrap();
        let f = SearchFilters {
            keywords: Some("lodging receipt".into()),
            account_ids: Some(vec!["a".into()]),
            exclude_bins: true,
            ..Default::default()
        };
        let classic = db::search::ranked_search(&c, &f, 50, true).unwrap();
        let docs = db::knowledge::ranked_documents(&c, &f, 50)
            .unwrap()
            .into_iter()
            .map(|h| h.result)
            .collect();
        let semantic = db::knowledge::retrieve(&c, &f, Some(("model", &[1.0, 0.0])), 50)
            .unwrap()
            .into_iter()
            .map(|h| h.result)
            .collect();
        let fused = fuse(&c, vec![(1.0, classic), (0.6, docs), (1.0, semantic)]).unwrap();
        assert_eq!(fused.iter().map(|r| r.uid).collect::<Vec<_>>(), vec![2, 1]);
        assert!(!fused[0].semantic_match);
        assert!(fused[1].semantic_match);
        assert!(
            db::knowledge::retrieve(&c, &f, Some(("changed-model", &[1.0, 0.0])), 50)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn ranked_lanes_obey_scope_bins_metadata_and_opt_out() {
        let c = fixture();
        for (account, folder, uid) in [("a", "INBOX", 1), ("hidden", "INBOX", 2), ("a", "Trash", 3)]
        {
            mail(
                &c,
                account,
                folder,
                uid,
                "Budget",
                "Budget approval.",
                &format!("<{uid}>"),
            );
        }
        let f = SearchFilters {
            keywords: Some("budget".into()),
            exclude_bins: true,
            ..Default::default()
        };
        let keyword = db::search::ranked_search(&c, &f, 50, true).unwrap();
        let doc = db::knowledge::ranked_documents(&c, &f, 50).unwrap();
        let semantic = db::knowledge::retrieve(&c, &f, Some(("model", &[1.0, 0.0])), 50).unwrap();
        assert_eq!(keyword.len(), 1);
        assert_eq!(doc.len(), 1);
        assert_eq!(semantic.len(), 1);
        assert_eq!(
            db::knowledge::progress_in_accounts(&c, "model", &["hidden".into()])
                .unwrap()
                .embedded,
            1
        );
        let hidden = SearchFilters {
            account_ids: Some(vec!["hidden".into()]),
            ..f.clone()
        };
        assert_eq!(
            db::knowledge::retrieve(&c, &hidden, Some(("model", &[1.0, 0.0])), 50).unwrap()[0]
                .result
                .account_id,
            "hidden"
        );
        let empty = SearchFilters {
            account_ids: Some(vec![]),
            ..f.clone()
        };
        assert!(db::search::ranked_search(&c, &empty, 50, true)
            .unwrap()
            .is_empty());
        assert!(db::knowledge::ranked_documents(&c, &empty, 50)
            .unwrap()
            .is_empty());
        assert!(
            db::knowledge::retrieve(&c, &empty, Some(("model", &[1.0, 0.0])), 50)
                .unwrap()
                .is_empty()
        );
        let wrong_subject = SearchFilters {
            subject_contains: Some("Nonexistent".into()),
            ..f.clone()
        };
        assert!(
            db::knowledge::retrieve(&c, &wrong_subject, Some(("model", &[1.0, 0.0])), 50)
                .unwrap()
                .is_empty()
        );
        c.execute("UPDATE intelligence_accounts SET semantic_search=0", [])
            .unwrap();
        assert!(
            db::knowledge::retrieve(&c, &f, Some(("model", &[1.0, 0.0])), 50)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db::search::ranked_search(&c, &f, 50, true).unwrap().len(),
            1
        );
    }
    #[test]
    fn relevance_beats_recency_and_folder_copies_do_not_consume_ranks() {
        let c = fixture();
        mail(&c, "a", "INBOX", 1, "Needle invoice", "", "<one>");
        mail(&c, "a", "Archive", 2, "Needle invoice", "", "<one>");
        mail(
            &c,
            "a",
            "INBOX",
            3,
            "General update",
            "Needle appears in the body.",
            "<three>",
        );
        c.execute(
            "UPDATE messages SET date='2020-01-01T12:00:00Z' WHERE uid IN (1,2)",
            [],
        )
        .unwrap();
        let f = SearchFilters {
            keywords: Some("needle".into()),
            ..Default::default()
        };
        let hits = db::search::ranked_search(&c, &f, 50, true).unwrap();
        assert!(
            hits[0].uid == 1 || hits[0].uid == 2,
            "Subject match must outrank a newer body match"
        );
        let fused = fuse(&c, vec![(1.0, hits)]).unwrap();
        assert_eq!(fused.len(), 2);
        // Equal Message-IDs in two accounts are separate mail identities.
        mail(&c, "hidden", "INBOX", 4, "Needle invoice", "", "<one>");
        let explicit = SearchFilters {
            account_ids: Some(vec!["a".into(), "hidden".into()]),
            ..f
        };
        let hits = db::search::ranked_search(&c, &explicit, 50, true).unwrap();
        assert_eq!(fuse(&c, vec![(1.0, hits)]).unwrap().len(), 3);
    }
}

/// Activation is a local CLI action, not a remote API or MCP mutation.
pub fn enable_local(path: &Path) -> Result<(), AppError> {
    super::embeddings::save(&super::embeddings::EmbeddingSettings {
        source: "local-vault".into(),
        model: "embeddinggemma-2".into(),
    })?;
    let c = Connection::open(path)?;
    for account in db::accounts::list(&c)?
        .into_iter()
        .filter(|a| a.is_active && !a.hidden_from_aggregates)
    {
        // Preserve every unrelated intelligence setting, particularly auto-draft.
        c.execute("INSERT INTO intelligence_accounts(account_id,semantic_search) VALUES(?1,1) ON CONFLICT(account_id) DO UPDATE SET semantic_search=1",[account.id])?;
    }
    Ok(())
}

/// Companion-owned local-only catch-up, including when the desktop window is
/// closed. Bounded batches, no DB lock across HTTP, and CAS on every write.
pub async fn index_local(path: &Path) -> Result<serde_json::Value, AppError> {
    let c = Connection::open(path)?;
    c.busy_timeout(Duration::from_secs(5))?;
    if !db::intelligence::options(&c)?
        .iter()
        .any(|o| o.semantic_search)
    {
        return Ok(serde_json::json!({"embedded":0,"pending":0}));
    }
    let client = EmbeddingClient::load()?;
    if !client.is_local() {
        return Err(AppError::General(
            "Companion indexer requires a local embedding service".into(),
        ));
    }
    let key = client.key();
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    let mut embedded = 0;
    // Body backfill is chunked; attachment indexing remains with the mail engine.
    for _ in 0..4 {
        if db::knowledge::backfill(&c, 200)? < 200 {
            break;
        }
    }
    while std::time::Instant::now() < deadline {
        let batch = db::knowledge::pending(&c, &key, 2)?;
        if batch.is_empty() {
            break;
        }
        let texts = batch.iter().map(|p| p.text.clone()).collect::<Vec<_>>();
        let vectors = match client.embed(&texts).await {
            Ok(v) => v.into_iter().map(Some).collect::<Vec<_>>(),
            Err(batch_error) => {
                // Isolate deterministic bad input; never mark passages done
                // for an outage, timeout, or malformed provider response.
                let mut output = Vec::new();
                for chunk in &batch {
                    if std::time::Instant::now() >= deadline {
                        return Err(batch_error);
                    }
                    match client.embed(&[chunk.text.clone()]).await {
                        Ok(mut v) => output.push(Some(v.remove(0))),
                        Err(AppError::Parse(_)) => output.push(None),
                        Err(e) => return Err(e),
                    }
                }
                output
            }
        };
        let mut saved = 0;
        let mut rejected = 0;
        let tx = c.unchecked_transaction()?;
        for (chunk, vector) in batch.iter().zip(vectors) {
            if let Some(vector) = vector {
                if db::knowledge::save_embedding(&tx, chunk, &key, &vector)? {
                    saved += 1;
                }
            } else {
                rejected += tx.execute("UPDATE knowledge_chunks SET embedding='[]',embedding_key=?1 WHERE id=?2 AND text=?3 AND EXISTS(SELECT 1 FROM intelligence_accounts i JOIN messages m ON m.account_id=i.account_id WHERE m.id=knowledge_chunks.message_id AND i.semantic_search=1)",rusqlite::params![key,chunk.id,chunk.text])?;
                log::warn!("Local embedding input rejected; skipped one unchanged passage");
            }
        }
        tx.commit()?;
        embedded += saved;
        if saved + rejected == 0 {
            break;
        }
    }
    let progress = db::knowledge::progress(&c, &key)?;
    Ok(
        serde_json::json!({"embedded":embedded,"pending":progress.pending,"total_embedded":progress.embedded}),
    )
}
