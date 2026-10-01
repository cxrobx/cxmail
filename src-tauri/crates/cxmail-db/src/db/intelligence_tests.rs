use super::{action_queue, intelligence, knowledge, schema, search::SearchFilters};
use rusqlite::{params, Connection};

fn setup() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    schema::initialize(&c).unwrap();
    for (id, hidden) in [("a", false), ("h", true)] {
        c.execute("INSERT INTO accounts(id,email,provider,imap_host,smtp_host,hidden_from_aggregates) VALUES(?1,?2,'imap','imap.example.com','smtp.example.com',?3)", params![id,format!("{id}@example.com"),hidden]).unwrap();
        for (folder, kind) in [("INBOX", "inbox"), ("Sent", "sent"), ("Unsent", "drafts")] {
            c.execute(
                "INSERT INTO folders(account_id,name,folder_type) VALUES(?1,?2,?3)",
                params![id, folder, kind],
            )
            .unwrap();
        }
    }
    c
}
fn message(
    c: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    age: &str,
    root: &str,
    reply_to: Option<&str>,
) -> i64 {
    c.execute("INSERT INTO messages(account_id,folder_name,uid,subject,from_email,to_list,date,snippet,thread_root_id,message_id,in_reply_to) VALUES(?1,?2,?3,'Project review',?4,?5,strftime('%Y-%m-%dT%H:%M:%S+00:00','now',?6),'Can you confirm the project by tomorrow?',?7,?8,?9)",params![account,folder,uid,if folder=="Sent"{format!("{account}@example.com")}else{"sam@peer.test".into()},if folder=="Sent"{"[{\"email\":\"sam@peer.test\"}]".to_string()}else{format!("[{{\"email\":\"{account}@example.com\"}}]")},age,root,format!("<{folder}-{uid}@{account}>"),reply_to]).unwrap();
    let id = intelligence::message_id(c, account, folder, uid).unwrap();
    c.execute("INSERT INTO message_bodies(account_id,folder_name,uid,plain_text) VALUES(?1,?2,?3,'Can you confirm the project by tomorrow?')",params![account,folder,uid]).unwrap();
    id
}
fn enable(c: &Connection, account: &str) {
    intelligence::save_options(
        c,
        &intelligence::AccountOptions {
            account_id: account.into(),
            auto_draft: true,
            semantic_search: true,
        },
    )
    .unwrap();
}

#[test]
fn suggestions_are_opt_in_and_late_responses_invalidate_them() {
    let c = setup();
    let id = message(&c, "a", "INBOX", 1, "-2 days", "<thread>", None);
    assert!(intelligence::options(&c)
        .unwrap()
        .iter()
        .all(|o| !o.auto_draft && !o.semantic_search));
    enable(&c, "a");
    let initial = intelligence::snapshot(&c, id).unwrap();
    assert!(intelligence::save_suggestion(
        &c,
        id,
        &initial,
        "I can review [confirm date].",
        "test"
    )
    .unwrap());
    assert!(intelligence::suggestion(&c, id).unwrap().is_some());
    message(&c, "a", "Unsent", 2, "-1 day", "<thread>", None);
    assert_eq!(
        intelligence::snapshot(&c, id).unwrap(),
        initial,
        "Human drafts do not trigger regeneration"
    );
    intelligence::dismiss_suggestion(&c, id).unwrap();
    assert!(intelligence::suggestion(&c, id).unwrap().is_none());
    assert!(intelligence::already_generated(&c, id, &initial).unwrap());
    message(
        &c,
        "a",
        "Sent",
        3,
        "-1 day",
        "<thread>",
        Some("<INBOX-1@a>"),
    );
    assert!(!intelligence::save_suggestion(&c, id, &initial, "Stale reply", "test").unwrap());
    assert!(intelligence::suggestion(&c, id).unwrap().is_none());
}

#[test]
fn opting_out_hides_suggestions_and_invalidates_pending_embedding_writes() {
    let c = setup();
    let id = message(&c, "a", "INBOX", 1, "-1 day", "<thread>", None);
    enable(&c, "a");
    knowledge::index_source(&c, id, "Email", "Policy requires fourteen days notice.").unwrap();
    let chunk = knowledge::pending(&c, "model-a", 16).unwrap().remove(0);
    let snapshot = intelligence::snapshot(&c, id).unwrap();
    intelligence::save_suggestion(&c, id, &snapshot, "Reply", "test").unwrap();
    intelligence::save_options(
        &c,
        &intelligence::AccountOptions {
            account_id: "a".into(),
            auto_draft: false,
            semantic_search: false,
        },
    )
    .unwrap();
    assert!(intelligence::suggestion(&c, id).unwrap().is_none());
    assert!(!knowledge::save_embedding(&c, &chunk, "model-a", &[1.0, 0.0]).unwrap());
    assert!(knowledge::pending(&c, "model-a", 16).unwrap().is_empty());
}

#[test]
fn document_and_semantic_search_obey_scope_filters_and_model_identity() {
    let c = setup();
    let mut ids = Vec::new();
    for account in ["a", "h"] {
        enable(&c, account);
        let id = message(&c, account, "INBOX", 1, "-1 day", "<thread>", None);
        ids.push(id);
        knowledge::index_source(
            &c,
            id,
            "Attachment 1: policy.pdf",
            "Cancellation needs fourteen days notice.",
        )
        .unwrap();
        c.execute("INSERT INTO attachments(account_id,folder_name,message_uid,filename,content_type,size_bytes) VALUES(?1,'INBOX',1,'policy.pdf','application/pdf',123)",[account]).unwrap();
    }
    let mut f = SearchFilters {
        keywords: Some("cancellation".into()),
        ..Default::default()
    };
    let hits = knowledge::retrieve(&c, &f, None, 20).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].result.account_id, "a");
    assert!(hits[0].result.snippet.contains("policy.pdf"));
    f.account_ids = Some(vec![]);
    assert!(knowledge::retrieve(&c, &f, None, 20).unwrap().is_empty());
    f.account_ids = Some(vec!["h".into()]);
    assert_eq!(
        knowledge::retrieve(&c, &f, None, 20).unwrap()[0]
            .result
            .account_id,
        "h"
    );
    f.filename = Some("other.pdf".into());
    assert!(knowledge::retrieve(&c, &f, None, 20).unwrap().is_empty());
    f.filename = Some("policy.pdf".into());
    f.from = Some("other@peer.test".into());
    assert!(knowledge::retrieve(&c, &f, None, 20).unwrap().is_empty());
    f.from = None;
    f.date_before = Some("2000-01-01".into());
    assert!(knowledge::retrieve(&c, &f, None, 20).unwrap().is_empty());
    let pending = knowledge::pending(&c, "model-a", 16).unwrap();
    assert_eq!(
        pending.len(),
        1,
        "Hidden accounts are never sent in background indexing"
    );
    assert!(knowledge::save_embedding(&c, &pending[0], "model-a", &[1.0, 0.0]).unwrap());
    f = SearchFilters {
        keywords: Some("return window".into()),
        ..Default::default()
    };
    assert_eq!(
        knowledge::retrieve(&c, &f, Some(("model-a", &[1.0, 0.0])), 20)
            .unwrap()
            .len(),
        1
    );
    assert!(
        knowledge::retrieve(&c, &f, Some(("model-b", &[1.0, 0.0])), 20)
            .unwrap()
            .is_empty()
    );
    assert_eq!(knowledge::pending(&c, "model-b", 16).unwrap().len(), 1);
    knowledge::index_source(
        &c,
        ids[0],
        "Attachment 1: policy.pdf",
        "Changed cancellation policy",
    )
    .unwrap();
    assert!(
        !knowledge::save_embedding(&c, &pending[0], "model-a", &[1.0, 0.0]).unwrap(),
        "Content changing during inference cannot publish a stale vector"
    );
}

#[test]
fn fts_triggers_updates_and_deletion_never_leave_orphan_sources() {
    let c = setup();
    let id = message(&c, "a", "INBOX", 1, "-1 day", "<thread>", None);
    knowledge::backfill(&c, 100).unwrap();
    let f = SearchFilters {
        keywords: Some("confirm".into()),
        ..Default::default()
    };
    assert_eq!(knowledge::retrieve(&c, &f, None, 20).unwrap().len(), 1);
    c.execute("UPDATE message_bodies SET plain_text='Updated body'", [])
        .unwrap();
    assert!(knowledge::retrieve(&c, &f, None, 20).unwrap().is_empty());
    knowledge::backfill(&c, 100).unwrap();
    knowledge::index_source(&c, id, "Attachment 1: old.txt", "Obsolete policy").unwrap();
    knowledge::index_source(&c, id, "Attachment 1: old.txt", "Replacement policy").unwrap();
    assert!(knowledge::retrieve(
        &c,
        &SearchFilters {
            keywords: Some("obsolete".into()),
            ..Default::default()
        },
        None,
        20
    )
    .unwrap()
    .is_empty());
    c.execute("DELETE FROM messages WHERE id=?1", [id]).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM knowledge_chunks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM knowledge_fts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn document_search_pages_one_deduplicated_union_and_preserves_body_highlights() {
    let c = setup();
    for uid in 1..=6 {
        let id = message(
            &c,
            "a",
            "INBOX",
            uid,
            &format!("-{uid} hours"),
            &format!("<thread-{uid}>"),
            None,
        );
        if uid % 2 == 0 {
            knowledge::index_source(&c, id, "Attachment 1: policy.txt", "Cancellation policy")
                .unwrap();
        } else {
            c.execute("UPDATE message_bodies SET plain_text='Cancellation policy' WHERE account_id='a' AND folder_name='INBOX' AND uid=?1",[uid]).unwrap();
        }
    }
    let f = SearchFilters {
        keywords: Some("cancellation".into()),
        ..Default::default()
    };
    let first = knowledge::lexical_search(&c, &f, 3, 0, false).unwrap();
    let second = knowledge::lexical_search(&c, &f, 3, 3, false).unwrap();
    assert_eq!(
        first
            .iter()
            .chain(&second)
            .map(|m| m.uid)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    assert!(first[0].snippet.contains(super::search::HIGHLIGHT_START));
    assert!(first[1].snippet.contains("policy.txt"));
    assert_eq!(
        knowledge::lexical_search(
            &c,
            &SearchFilters {
                keywords: Some("cancell".into()),
                ..Default::default()
            },
            3,
            0,
            true
        )
        .unwrap()
        .len(),
        3
    );
}

#[test]
fn migration_reruns_preserve_mail_and_opt_ins() {
    let c = setup();
    let id = message(&c, "a", "INBOX", 1, "-1 day", "<thread>", None);
    enable(&c, "a");
    knowledge::index_source(&c, id, "Email", "Keep this").unwrap();
    c.execute("UPDATE schema_version SET version=64", [])
        .unwrap();
    schema::initialize(&c).unwrap();
    schema::initialize(&c).unwrap();
    assert_eq!(intelligence::message_id(&c, "a", "INBOX", 1).unwrap(), id);
    assert!(intelligence::options(&c)
        .unwrap()
        .iter()
        .any(|o| o.account_id == "a" && o.auto_draft));
    assert_eq!(
        c.query_row("SELECT count(*) FROM knowledge_chunks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn action_queue_moves_answered_threads_to_waiting_and_back_on_new_mail() {
    let c = setup();
    message(&c, "a", "Sent", 900, "-60 days", "<history>", None);
    message(&c, "a", "Sent", 901, "-61 days", "<history2>", None);
    message(
        &c,
        "a",
        "INBOX",
        902,
        "-59 days",
        "<history>",
        Some("<Sent-900@a>"),
    );
    message(&c, "a", "INBOX", 1, "-2 days", "<thread>", None);
    assert!(action_queue::list(&c, 50, false)
        .unwrap()
        .iter()
        .any(|i| i.lane == "needs_response" && i.message.uid == 1));
    message(
        &c,
        "h",
        "Sent",
        4,
        "-1 day",
        "<thread>",
        Some("<INBOX-1@a>"),
    );
    assert!(
        action_queue::list(&c, 50, false)
            .unwrap()
            .iter()
            .any(|i| i.lane == "needs_response" && i.message.uid == 1),
        "Another account cannot resolve the request"
    );
    message(
        &c,
        "a",
        "Sent",
        2,
        "-1 day",
        "<thread>",
        Some("<INBOX-1@a>"),
    );
    let queue = action_queue::list(&c, 50, false).unwrap();
    assert!(!queue
        .iter()
        .any(|i| i.lane == "needs_response" && i.message.uid == 1));
    assert!(queue.iter().any(|i| i.lane == "waiting"));
    message(
        &c,
        "a",
        "INBOX",
        3,
        "+0 days",
        "<thread>",
        Some("<Sent-2@a>"),
    );
    let queue = action_queue::list(&c, 50, false).unwrap();
    assert!(!queue.iter().any(|i| i.lane == "waiting"));
    assert!(queue
        .iter()
        .any(|i| i.lane == "needs_response" && i.message.uid == 3));
}

#[test]
fn index_progress_counts_exactly_what_the_indexer_will_pick_up() {
    let c = setup();
    let a = message(&c, "a", "INBOX", 1, "-1 day", "<one>", None);
    message(&c, "a", "INBOX", 2, "-2 days", "<two>", None);
    message(&c, "h", "INBOX", 3, "-1 day", "<three>", None);
    // Two short transactions' worth, then nothing left to chunk.
    assert_eq!(knowledge::backfill(&c, 2).unwrap(), 2);
    assert_eq!(knowledge::backfill(&c, 2).unwrap(), 1);
    assert_eq!(knowledge::backfill(&c, 2).unwrap(), 0);
    assert_eq!(knowledge::progress(&c, "k").unwrap().chunks, 0, "nothing is opted in");
    enable(&c, "a");
    enable(&c, "h");
    let p = knowledge::progress(&c, "k").unwrap();
    assert_eq!((p.chunks, p.embedded, p.pending), (2, 0, 2), "the hidden account is never indexed");
    assert_eq!(p.pending as usize, knowledge::pending(&c, "k", 100).unwrap().len());
    let first = knowledge::pending(&c, "k", 1).unwrap().remove(0);
    assert_eq!(first.message_id, a, "newest mail first");
    assert!(knowledge::save_embedding(&c, &first, "k", &[1.0, 0.0]).unwrap());
    c.execute("UPDATE knowledge_chunks SET embedding='[]', embedding_key='k' WHERE id<>?1 AND embedding IS NULL", [first.id]).unwrap();
    let p = knowledge::progress(&c, "k").unwrap();
    assert_eq!((p.chunks, p.embedded, p.pending), (2, 1, 0), "withheld is done but not searchable");
    let p = knowledge::progress(&c, "another-model").unwrap();
    assert_eq!((p.embedded, p.pending), (0, 2), "a model change re-queues everything");
}
