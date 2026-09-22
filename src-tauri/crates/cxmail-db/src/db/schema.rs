use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, Result};

const CURRENT_VERSION: i32 = 63;

/// Initialize the database, creating tables if needed and running migrations.
pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    conn.execute_batch("PRAGMA optimize;")?;
    // Cap WAL file at 64 MB. After a checkpoint that resets the WAL, SQLite
    // truncates the file back to this size instead of letting it grow unbounded.
    conn.execute_batch("PRAGMA journal_size_limit = 67108864;")?;

    let version = get_schema_version(conn)?;

    if version < 1 {
        create_tables_v1(conn)?;
    }
    if version < 2 {
        create_tables_v2(conn)?;
    }
    if version < 3 {
        create_tables_v3(conn)?;
    }
    if version < 4 {
        create_tables_v4(conn)?;
    }
    if version < 5 {
        create_tables_v5(conn)?;
    }
    if version < 6 {
        create_tables_v6(conn)?;
    }
    if version < 7 {
        create_tables_v7(conn)?;
    }
    if version < 8 {
        create_tables_v8(conn)?;
    }
    if version < 9 {
        migrate_dates_to_iso8601(conn)?;
    }
    if version < 10 {
        // Re-run to catch dates without day-of-week prefix (e.g., "14 Oct 2025 ...")
        migrate_dates_to_iso8601(conn)?;
    }
    if version < 11 {
        create_tables_v11(conn)?;
    }
    if version < 12 {
        // Re-run with fixed ISO detection (previous runs skipped "14 Oct 2025..." dates)
        migrate_dates_to_iso8601(conn)?;
    }
    if version < 13 {
        create_tables_v13(conn)?;
    }
    if version < 14 {
        create_tables_v14(conn)?;
    }
    if version < 15 {
        create_tables_v15(conn)?;
    }
    if version < 16 {
        create_tables_v16(conn)?;
    }
    if version < 17 {
        create_tables_v17(conn)?;
    }
    if version < 18 {
        migrate_v18_account_sort_order(conn)?;
    }
    if version < 19 {
        migrate_v19_account_group(conn)?;
    }
    if version < 20 {
        migrate_v20_unsubscribed_senders(conn)?;
    }
    if version < 21 {
        migrate_v21_in_reply_to_index(conn)?;
    }
    if version < 22 {
        migrate_v22_sync_state_reconciliation_columns(conn)?;
    }
    if version < 23 {
        migrate_v23_sync_state_error_columns(conn)?;
    }
    if version < 24 {
        // Re-run date normalization for messages inserted with RFC 2822 dates
        migrate_dates_to_iso8601(conn)?;
    }
    if version < 25 {
        migrate_v25_account_notify_enabled(conn)?;
    }
    if version < 26 {
        migrate_v26_voice_profiles(conn)?;
    }
    if version < 27 {
        migrate_v27_voice_edits_and_insights(conn)?;
    }
    if version < 28 {
        migrate_v28_attachment_metadata_checked(conn)?;
    }
    if version < 29 {
        migrate_v29_thread_root_id(conn)?;
    }
    if version < 30 {
        migrate_v30_trusted_image_senders(conn)?;
    }
    if version < 31 {
        migrate_v31_cid_resolved(conn)?;
    }
    if version < 32 {
        migrate_v32_unescape_subjects(conn)?;
    }
    if version < 33 {
        migrate_v33_recipient_voice_and_archetypes(conn)?;
    }
    if version < 34 {
        migrate_v34_backfill_to_list(conn)?;
    }
    if version < 35 {
        migrate_v35_salvage_malformed_dates(conn)?;
    }
    if version < 36 {
        // Re-run the v34 backfill to recover messages whose body cache gained a
        // `to_json` after v34 already ran (BUG-02).
        migrate_v34_backfill_to_list(conn)?;
    }
    if version < 37 {
        migrate_v37_drop_thread_count(conn)?;
    }
    if version < 38 {
        migrate_v38_backfill_detected_events(conn)?;
    }
    if version < 39 {
        migrate_v39_bcc_json(conn)?;
    }
    if version < 40 {
        migrate_v40_fts5(conn)?;
    }
    if version < 41 {
        migrate_v41_account_track_opens_enabled(conn)?;
    }
    if version < 42 {
        migrate_v42_needs_you_dismissals(conn)?;
    }
    if version < 43 {
        migrate_v43_gcal(conn)?;
    }
    if version < 44 {
        migrate_v44_recipient_voice_buckets(conn)?;
    }
    if version < 45 {
        migrate_v45_invite_approval_snapshot(conn)?;
    }
    if version < 46 {
        migrate_v46_message_headers(conn)?;
    }
    if version < 47 {
        migrate_v47_nudge_dismissals(conn)?;
    }
    if version < 48 {
        migrate_v48_stitch_orphan_threads(conn)?;
    }
    if version < 49 {
        // v48 shipped without the reply-prefix gate and fused recurring
        // notifications into giant threads (379 Synology alerts in one). The
        // pass resets orphan roots before re-deriving, so re-running it with
        // the corrected rule undoes that rather than compounding it.
        migrate_v48_stitch_orphan_threads(conn)?;
    }
    if version < 50 {
        migrate_v50_generic_imap(conn)?;
    }
    if version < 51 {
        migrate_v51_voice_pinned_rules(conn)?;
    }
    if version < 52 {
        migrate_v52_zoom_meetings(conn)?;
    }
    if version < 53 {
        migrate_v53_recorrect_detected_events(conn)?;
    }
    if version < 54 {
        migrate_v54_repair_mojibake_snippets(conn)?;
    }
    if version < 55 {
        migrate_v55_scrub_dash_claims_from_voice_profiles(conn)?;
    }
    if version < 56 {
        migrate_v56_claude_repos(conn)?;
    }
    if version < 57 {
        migrate_v57_invite_notifications(conn)?;
    }
    if version < 58 {
        migrate_v58_account_hidden_from_aggregates(conn)?;
    }
    if version < 59 {
        migrate_v59_draft_attachment_blobs(conn)?;
    }
    if version < 60 {
        migrate_v60_drafts(conn)?;
    }
    if version < 61 {
        migrate_v61_triage(conn)?;
    }
    if version < 62 {
        migrate_v62_triage_tokens(conn)?;
    }
    if version < 63 {
        migrate_v63_triage_withheld(conn)?;
    }

    set_schema_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// v57: the invitation-delivery ledger — which attendees we actually asked
/// Google to notify.
///
/// **Google exposes no such fact.** An attendee object carries `email`,
/// `displayName`, `organizer`, `self` and `responseStatus`, and nothing that
/// says an invitation was sent; `responseStatus` reads `needsAction` from the
/// moment the attendee is attached, so "invited, awaiting reply" and "never
/// told" are indistinguishable on the organizer's own calendar. Attaching an
/// attendee is a field write; notifying is a separate side effect controlled by
/// `sendUpdates`, which defaults to `none`. The fact therefore has to be
/// recorded by whoever makes the call — us.
///
/// **A side table on the remote triple with NO foreign keys**, exactly as
/// `zoom_meetings` (v52) and for the same two reasons: `upsert_remote_event` is
/// driven entirely by Google's payload, so a column here would be nulled on
/// every sync tick, and four paths DELETE from `gcal_events`, so the record
/// would die with the mirror row. A ledger row whose event was swept is a
/// tombstone — it is the evidence that a human being was told about a meeting.
///
/// **No backfill, deliberately.** The fact is unreconstructible from stored
/// data (gotcha #37's situation), so every pre-existing attendee resolves to
/// `unknown`. That is the honest answer; back-dating history to `sent` would
/// recreate the precise ambiguity this table removes.
pub(crate) fn migrate_v57_invite_notifications(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS invite_notifications (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id       TEXT NOT NULL,
            gcal_calendar_id TEXT NOT NULL,
            gcal_event_id    TEXT NOT NULL,
            attendee_email   TEXT NOT NULL,
            notified_at      TEXT NOT NULL DEFAULT (datetime('now')),
            source           TEXT NOT NULL DEFAULT 'cxmail',
            UNIQUE(account_id, gcal_calendar_id, gcal_event_id, attendee_email)
        );
        CREATE INDEX IF NOT EXISTS idx_invite_notifications_event
            ON invite_notifications(account_id, gcal_calendar_id, gcal_event_id);",
    )
}

/// v58: `accounts.hidden_from_aggregates` — the per-account switch that keeps
/// an account (a warm-up mailbox, say) out of every cross-account surface:
/// All Inboxes, the account folders, rule-based inbox groups, Needs You,
/// nudges, unscoped search, the category counts and the dock badge. The mail
/// is still synced and still there when the account itself is clicked.
///
/// Enforced in the DB queries through the single predicate
/// `db::accounts::visible_in_aggregates_sql` — no aggregate query reads the
/// `accounts` table otherwise, so "all accounts" has always meant "every row
/// in `messages`". Guarded like v41 so a re-run (an older binary stamping the
/// version back down, gotchas #26/#37) never fails and never clobbers a
/// choice. No seed: the flag is flipped from the account's context menu.
/// v59: attachment bytes for drafts we authored, so reopening one does not
/// re-download the whole message from IMAP. See `db::draft_blobs`. No
/// backfill — drafts saved before this fill in on their next save or their
/// next IMAP-fallback open.
pub(crate) fn migrate_v59_draft_attachment_blobs(conn: &Connection) -> Result<()> {
    super::draft_blobs::create_table(conn)
}

/// v60: a draft's identity across its IMAP revisions — `drafts(draft_id →
/// current_uid)` with a claim slot for the compare-and-set writers take before
/// any APPEND (gotcha #57). Additive; no backfill — pre-v60 drafts carry no
/// `X-CXMail-Draft-Id` header and get a row lazily on their next save.
pub(crate) fn migrate_v60_drafts(conn: &Connection) -> Result<()> {
    super::drafts::create_table(conn)
}

/// v63: `withheld_reason`, so a message the gate refused leaves the queue.
///
/// Without it the pass livelocked: withheld messages stayed pending, and
/// `list_pending` is newest-first with a small limit, so the same few were
/// re-picked every tick forever. Additive and nullable — every existing row is
/// a real verdict and reads as one.
pub(crate) fn migrate_v63_triage_withheld(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "triage_verdicts", "withheld_reason")? {
        conn.execute_batch("ALTER TABLE triage_verdicts ADD COLUMN withheld_reason TEXT;")?;
    }
    Ok(())
}

/// v62: what each verdict cost. Additive with a 0 default, so the verdicts
/// written before this read as "unknown spend" rather than as free — and
/// `token_totals` is then a fact rather than an estimate from prompt lengths.
pub(crate) fn migrate_v62_triage_tokens(conn: &Connection) -> Result<()> {
    for column in ["input_tokens", "output_tokens"] {
        if !table_has_column(conn, "triage_verdicts", column)? {
            conn.execute_batch(&format!(
                "ALTER TABLE triage_verdicts ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0;"
            ))?;
        }
    }
    Ok(())
}

/// v61: the AI triage pass. `accounts.triage_enabled` (default OFF — the
/// v58 `hidden_from_aggregates` shape; Chris opts individual accounts in) and
/// `triage_verdicts`, a cascading side table on the message triple. Additive;
/// no backfill — the app's background pass fills it lazily, newest first,
/// under a spend cap.
pub(crate) fn migrate_v61_triage(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "accounts", "triage_enabled")? {
        conn.execute_batch(
            "ALTER TABLE accounts ADD COLUMN triage_enabled INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    super::triage::create_table(conn)
}

pub(crate) fn migrate_v58_account_hidden_from_aggregates(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "accounts", "hidden_from_aggregates")? {
        conn.execute_batch(
            "ALTER TABLE accounts ADD COLUMN hidden_from_aggregates INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    Ok(())
}

/// The table behind "Open in Claude" landing in a repo instead of a scratch
/// directory. See `db::claude_repos` for the scopes and the resolution order.
///
/// Split out of the migration so tests can build the table without replaying 56
/// versions of history.
///
/// **Both foreign keys cascade, and the group one is not a tidiness choice.**
/// `inbox_groups.id` is a plain INTEGER PRIMARY KEY, so SQLite reissues a
/// deleted group's id to the next group created — an orphaned mapping would
/// silently reattach itself to an unrelated group and start sending that mail
/// into a repo nobody pointed it at. (Contrast `zoom_meetings`, gotcha #44,
/// where a missing row is ambiguous and an orphan must survive as a tombstone;
/// here the row's whole meaning is the group it names.)
///
/// The unique indexes are partial, which is what makes "one repo per contact /
/// group / account, and one default" a schema fact rather than something every
/// writer has to remember. Note that a partial index requires its `WHERE`
/// clause to be repeated in any `ON CONFLICT` target that uses it.
pub(crate) fn create_claude_repos_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS claude_repos (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            scope      TEXT NOT NULL CHECK (scope IN ('contact','group','account','default')),
            contact    TEXT,
            group_id   INTEGER REFERENCES inbox_groups(id) ON DELETE CASCADE,
            account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
            repo_path  TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_claude_repos_contact
             ON claude_repos(contact) WHERE contact IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_claude_repos_group
             ON claude_repos(group_id) WHERE group_id IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_claude_repos_account
             ON claude_repos(account_id) WHERE account_id IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_claude_repos_default
             ON claude_repos(scope) WHERE scope = 'default';",
    )
}

/// v56: `claude_repos` — which repo an "Open in Claude" handoff lands in.
///
/// Creates the table and nothing else. Mappings are the user's to make, in
/// Settings → Claude repos; a migration has no business knowing whose machine
/// it is running on, or which contacts belong to which project.
fn migrate_v56_claude_repos(conn: &Connection) -> Result<()> {
    create_claude_repos_table(conn)
}

/// v55: stop stored voice profiles from prescribing the punctuation the pinned
/// rule forbids.
///
/// Every profile in the live DB when this shipped described the user's real
/// past habit — `"punctuation_style": "Uses em-dashes, parentheses, bullets…"`
/// — while an account-scoped pinned rule said "Never use em-dashes". Both reach
/// the model in one context window (`ai::build_voice_context_full` renders the
/// profile as the style to MATCH), so the draft was being argued at from two
/// sides and the description usually won.
///
/// A code fix reaches none of this. `voice.rs` now scrubs at extraction time,
/// but a profile is written once and only rebuilt when someone forces a
/// re-extraction, so today's rows would keep arguing indefinitely (the same
/// shape of gap as v54's snippets).
///
/// Safety is `dashes::scrub_dash_claims`'s, not this query's: it returns None —
/// meaning leave the row exactly as it is — whenever the claim cannot be
/// removed cleanly. Idempotent, because a scrubbed string no longer matches.
/// `voice_examples_json` is untouched on purpose: those are verbatim excerpts
/// of mail the user actually sent.
fn migrate_v55_scrub_dash_claims_from_voice_profiles(conn: &Connection) -> Result<()> {
    // (table, json column). Addressed by `rowid`, not by a named key:
    // `voice_profiles_recipient` and `voice_archetypes` both have COMPOSITE
    // primary keys and no `id` column at all. None of the three is
    // WITHOUT ROWID, so `rowid` is the one addressing scheme all three share.
    //
    // Guarded on existence because the MCP can reach `initialize` against an
    // older DB via its non-fatal migration path.
    const TARGETS: &[(&str, &str)] = &[
        ("voice_profiles", "voice_profile_json"),
        ("voice_profiles_recipient", "profile_json"),
        ("voice_archetypes", "profile_json"),
    ];

    let mut scrubbed = 0usize;
    for (table, column) in TARGETS {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !exists {
            continue;
        }

        let rows: Vec<(i64, String)> = {
            let mut stmt =
                conn.prepare(&format!("SELECT rowid, {column} FROM {table}"))?;
            let collected = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>>>()?;
            collected
        };

        let tx = conn.unchecked_transaction()?;
        for (id, json) in &rows {
            let Some(fixed) = cxmail_core::mail::dashes::scrub_profile_json(json) else {
                continue;
            };
            match tx.execute(
                &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
                rusqlite::params![fixed, id],
            ) {
                Ok(_) => scrubbed += 1,
                Err(e) => log::warn!("v55: scrub failed for {table} row {id}: {e}"),
            }
        }
        if let Err(e) = tx.commit() {
            log::warn!("v55: commit failed for {table} (those profiles keep the claim): {e}");
        }
    }

    log::info!("v55: scrubbed dash claims from {scrubbed} voice profile(s)");
    Ok(())
}

/// v54: repair snippets corrupted by the Latin-1 bug in `html_to_plain_text`.
///
/// `out.push(bytes[i] as char)` decoded every HTML byte as Latin-1, so a smart
/// quote reached `messages.snippet` as three re-encoded chars — the inbox list
/// rendered `“A writer—and` as `âA writerâand`, since two of each three are
/// invisible C1 controls. Only snippets taken from the HTML fallback are
/// affected (a newsletter whose text/plain part is CSS boilerplate), which is
/// why the message bodies themselves are clean: `mail_parser` decoded those.
///
/// A code fix alone would never reach these. A snippet is written once, by
/// `persist_parsed_metadata` at ingest, and nothing recomputes it afterwards —
/// so every already-synced message would stay corrupted forever.
///
/// **Repaired in place rather than recomputed from cached bodies.** The
/// transform is the exact inverse of the one corruption applied, so it needs
/// nothing but the stored string — and roughly a quarter of the affected rows
/// have no cached body to recompute from, so a body-driven backfill would leave
/// them broken. Safety is `repair_double_encoded_utf8`'s three guards, not this
/// query's: a row it declines is left exactly as it was.
fn migrate_v54_repair_mojibake_snippets(conn: &Connection) -> Result<()> {
    const PAGE: usize = 2000;

    let mut last_id: i64 = 0;
    let mut repaired = 0usize;
    loop {
        // Materialize the page before opening the write transaction so the
        // SELECT statement is dropped first (same shape as v53).
        //
        // Unfiltered beyond "has a snippet": the corruption is not expressible
        // as a SQL predicate, and `repair_double_encoded_utf8` rejects a clean
        // string on its first char, so scanning them is cheaper than trying to
        // out-guess it in SQL.
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, snippet FROM messages
                 WHERE id > ?1 AND snippet IS NOT NULL AND snippet <> ''
                 ORDER BY id ASC
                 LIMIT ?2",
            )?;
            let page = stmt
                .query_map(rusqlite::params![last_id, PAGE as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?
                .collect::<Result<Vec<_>>>()?;
            page
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len();
        last_id = rows.last().map(|r| r.0).unwrap_or(last_id);

        let tx = conn.unchecked_transaction()?;
        for (id, snippet) in &rows {
            let Some(fixed) = cxmail_core::mail::text::repair_double_encoded_utf8(snippet) else {
                continue;
            };
            match tx.execute(
                "UPDATE messages SET snippet = ?1 WHERE id = ?2",
                rusqlite::params![fixed, id],
            ) {
                Ok(_) => repaired += 1,
                Err(e) => log::warn!("v54: snippet repair failed for message {id}: {e}"),
            }
        }
        if let Err(e) = tx.commit() {
            log::warn!("v54: page commit failed (those snippets stay mojibake'd): {e}");
        }

        if page_len < PAGE {
            break;
        }
    }

    log::info!("v54: repaired {repaired} double-encoded snippet(s)");
    Ok(())
}

/// v53: re-run heuristic event detection and CORRECT rows already stored.
///
/// A detector fix is otherwise invisible for mail already in the DB, and that is
/// not a small gap — detection runs only at sync-ingest (gotcha #21), and
/// `db::calendar::insert_detected` early-returns an existing row's id without
/// touching its fields. So every message that was mis-detected stays
/// mis-detected forever, on a screen the user looks at.
///
/// **Refresh-only: this migration inserts nothing.** It corrects rows that
/// already exist and is otherwise inert. That is a deliberate narrowing after
/// measuring the alternative against a copy of a real 14.6k-message mailbox: a
/// version that also inserted (v38's population) added **41** rows, nearly all
/// historical Google Calendar *notification* mail — "Invitation: … Therapy
/// Session 8-4-25", "Accepted: Peris Party @ …" — from 2025. That is clutter, not
/// value, and two things made it worse: v38's `NOT EXISTS (SELECT 1 FROM
/// calendar_events …)` guard is what stops a heuristic row being added next to an
/// ICS-sourced one for the same message, and `insert_detected` only checks for a
/// `source='detected'` row, so dropping that guard silently permits duplicates.
///
/// Inserting is also simply not needed: v38 already backfilled historical mail,
/// and new mail goes through the fixed detector at sync-ingest. The bug being
/// fixed is *stored rows being wrong*, so correcting them is the whole job.
///
/// Deliberately conservative in three further ways:
/// * A detected row is **updated, never deleted and reinserted**, so the user's
///   `dismissed` flag and the row id both survive.
/// * Only `source = 'detected'` rows are touched (enforced inside
///   `refresh_detected`), so an ICS invite or a user's own event is untouchable.
/// * A message that **no longer** detects leaves its existing row alone rather
///   than being deleted. Removing something the user can see is a bigger claim
///   than this migration should make; a stale row is visible and dismissable.
///
/// Failures are logged and skipped, and the whole pass returns `Ok` regardless —
/// refusing to open the app because a cosmetic re-parse failed would be a far
/// worse outcome than a stale event, the same call v48 makes.
fn migrate_v53_recorrect_detected_events(conn: &Connection) -> Result<()> {
    const PAGE: usize = 1000;
    type Row = (
        i64,            // messages.id (keyset cursor)
        String,         // account_id
        String,         // folder_name
        u32,            // uid
        Option<String>, // subject
        Option<String>, // from_email
        String,         // date
        Option<String>, // plain_text
    );

    let mut last_id: i64 = 0;
    let mut corrected = 0usize;
    loop {
        // Materialize the page before opening the write transaction, so the
        // SELECT statement is dropped first (same shape as v38).
        //
        // Scoped by an EXISTS on a `source='detected'` row, so the scan visits
        // only messages that can actually be corrected — a few dozen rather than
        // the whole mailbox.
        let rows: Vec<Row> = {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.account_id, m.folder_name, m.uid,
                        m.subject, m.from_email, m.date, mb.plain_text
                 FROM messages m
                 JOIN message_bodies mb
                   ON mb.account_id = m.account_id
                  AND mb.folder_name = m.folder_name
                  AND mb.uid = m.uid
                 WHERE m.id > ?1
                   AND mb.plain_text IS NOT NULL
                   AND EXISTS (
                       SELECT 1 FROM calendar_events ce
                       WHERE ce.account_id = m.account_id
                         AND ce.folder_name = m.folder_name
                         AND ce.message_uid = m.uid
                         AND ce.source = 'detected'
                   )
                 ORDER BY m.id ASC
                 LIMIT ?2",
            )?;
            // Bound to a local so `stmt` outlives the borrow (same as v38).
            let page = stmt
                .query_map(rusqlite::params![last_id, PAGE as i64], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>>>()?;
            page
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len();
        last_id = rows.last().map(|r| r.0).unwrap_or(last_id);

        let tx = conn.unchecked_transaction()?;
        for (_id, account_id, folder_name, uid, subject, from_email, date, plain_text) in &rows {
            let detected = cxmail_core::mail::detect_events::detect_events(
                subject.as_deref(),
                plain_text.as_deref(),
                from_email.as_deref().unwrap_or(""),
                date,
            );
            for event in detected.iter().filter(|e| e.confidence >= 0.6) {
                match crate::db::calendar::refresh_detected(
                    &tx,
                    account_id,
                    folder_name,
                    *uid,
                    &event.summary,
                    &event.dtstart,
                    event.dtend.as_deref(),
                    event.location.as_deref(),
                    event.description.as_deref(),
                    event.confidence,
                ) {
                    Ok(true) => corrected += 1,
                    // No `source='detected'` row to correct. Deliberately does
                    // NOT fall through to an insert — see the docstring.
                    Ok(false) => {}
                    Err(e) => log::warn!(
                        "v53: refresh_detected failed for {account_id}/{folder_name}/{uid}: {e}"
                    ),
                }
            }
        }
        if let Err(e) = tx.commit() {
            log::warn!("v53: page commit failed (detected events stay stale): {e}");
        }

        if page_len < PAGE {
            break;
        }
    }

    log::info!("v53: re-detection corrected {corrected} stored calendar event(s); inserted none by design");
    Ok(())
}

/// v52: the link between a Google Calendar event and the Zoom meeting behind it.
///
/// **A SIDE TABLE, not a `zoom_meeting_id` column on `gcal_events`** — the
/// obvious shape, and it fails two different ways.
///
/// 1. *The remote payload would null it.* `db::gcal::upsert_remote_event` is the
///    sole INSERT into `gcal_events` and it is `INSERT … ON CONFLICT DO UPDATE`
///    driven entirely by what Google just sent. Google knows nothing about Zoom,
///    so every sync tick would overwrite the column with NULL. The one column
///    that survives — `pending_notify` — only does so because it is explicitly
///    rescued by a correlated `COALESCE((SELECT … WHERE account_id=?2 AND …),0)`
///    inside the INSERT. A column here would depend on someone remembering to
///    add a second such rescue; a side table makes the hazard *structurally
///    impossible* rather than maintained by vigilance. Pinned by
///    `db::zoom::tests::a_google_sync_cannot_null_the_zoom_link`.
/// 2. *Deleting the mirror row would orphan the meeting invisibly.* Four paths
///    DELETE from `gcal_events` — `sweep_full_sync`, `purge_old_cancelled`,
///    `delete_local`, and the `gcal_calendars`/`accounts` CASCADEs. With a
///    column, the Zoom id dies with the row and a real meeting is left running
///    on Zoom with nothing pointing at it. An unlinked row in this table
///    survives as a tombstone that `list_orphans` finds via
///    `LEFT JOIN gcal_events … WHERE g.id IS NULL`.
///
/// **No foreign keys at all, deliberately** — same reasoning as v51's
/// `voice_pinned_rules`: evicting a cached mirror row is a cache operation, not
/// permission to forget an object living in another service. Note this goes one
/// step further than v51, which does keep an `accounts` FK: a Zoom meeting
/// belongs to the *Zoom* account, and the credentials are account-global, so
/// removing a Gmail account is not a reason to abandon a scheduled meeting
/// either.
///
/// **Keyed on the remote triple `(account_id, gcal_calendar_id, gcal_event_id)`,
/// not on `gcal_events.id`.** That id is AUTOINCREMENT and never reused, so a
/// sweep-then-repull (which is routine — an expired sync token forces it) would
/// leave an id-keyed link pointing at nothing. The triple is stable across
/// delete/re-pull and is exactly what `upsert_remote_event`'s `ON CONFLICT`
/// clause uses.
///
/// `pushed_topic` / `pushed_start` / `pushed_duration` record *what Zoom is
/// believed to hold*, advanced only on a 2xx. That is what lets change detection
/// be a string compare (`gcal_events.dtstart` is always
/// `YYYY-MM-DDTHH:MM:SSZ`, byte-identical to Zoom's `start_time`) and what makes
/// a failed push self-retrying: the recorded state does not move, so the next
/// tick recomputes the same patch.
pub(crate) fn migrate_v52_zoom_meetings(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS zoom_meetings (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id       TEXT NOT NULL,
            gcal_calendar_id TEXT NOT NULL,
            gcal_event_id    TEXT NOT NULL,
            zoom_meeting_id  TEXT,
            join_url         TEXT,
            state            TEXT NOT NULL DEFAULT 'creating'
                CHECK(state IN ('creating','active','delete_pending','unverified','retired')),
            pushed_topic     TEXT,
            pushed_start     TEXT,
            pushed_duration  INTEGER,
            attempts         INTEGER NOT NULL DEFAULT 0,
            last_error       TEXT,
            created_at       TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at       TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, gcal_calendar_id, gcal_event_id)
        );
        CREATE INDEX IF NOT EXISTS idx_zoom_meetings_state
            ON zoom_meetings(state);
        CREATE INDEX IF NOT EXISTS idx_zoom_meetings_account
            ON zoom_meetings(account_id, state);",
    )
}

/// v51: pinned, human-authored writing rules — absolute directives, not style.
///
/// **A SEPARATE TABLE, and that is the entire point.** `voice_profiles_recipient
/// .profile_json` is LLM-authored, statistical, and *rebuildable*:
/// `extract_recipient_profile(force=true)` overwrites it wholesale from the sent
/// corpus. A rule hand-written by the user ("always address Sam as 'Bro.
/// Ellis'") is none of those things — it is permanent and absolute — so storing
/// it inside that blob makes it a rebuild casualty the first time anyone
/// refreshes the profile. Two different kinds of data, two tables. Do NOT
/// "tidy" these back into `profile_json`; the split is load-bearing and pinned
/// by `db::voice_pinned_rules::tests::a_forced_profile_rebuild_leaves_pinned_rules_intact`.
///
/// Also deliberately **not** foreign-keyed to `voice_profiles_recipient`: a
/// CASCADE from the derived profile would reintroduce the same casualty by a
/// different route (deleting a profile is a cache eviction, not an instruction
/// to forget what the user told us). The FK to `accounts` is correct, though —
/// removing an account should take its rules with it.
///
/// `scope` is `'account'` (applies to every message sent from that account, e.g.
/// "never open with 'Hope you're well'") or `'recipient'` (applies to mail to one
/// address). `recipient_email` is `''` for account scope rather than NULL, so the
/// UNIQUE index actually enforces uniqueness — SQLite treats NULLs as distinct,
/// so a nullable column here would happily store the same rule a dozen times.
fn migrate_v51_voice_pinned_rules(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS voice_pinned_rules (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            scope           TEXT NOT NULL DEFAULT 'recipient',
            recipient_email TEXT NOT NULL DEFAULT '',
            rule            TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_voice_pinned_rules_unique
            ON voice_pinned_rules(account_id, scope, recipient_email, rule);
        CREATE INDEX IF NOT EXISTS idx_voice_pinned_rules_lookup
            ON voice_pinned_rules(account_id, scope, recipient_email);",
    )
}

/// v48: one-time repair of threads fragmented by missing `In-Reply-To`.
///
/// Unlike v46's headers, this one CAN be backfilled — the evidence (subject and
/// participants) is already in the DB, which is the whole reason a subject
/// fallback works at all. Scoped to 365 days: older mail is not what anyone is
/// looking at, and every extra day widens the window in which two unrelated
/// conversations can share a subject.
///
/// A failure here is logged and swallowed rather than aborting the migration.
/// Thread grouping is a display concern; refusing to open the app because a
/// cosmetic repair failed would be a far worse outcome than un-stitched threads.
fn migrate_v48_stitch_orphan_threads(conn: &Connection) -> Result<()> {
    match crate::db::messages::stitch_orphan_threads(conn, 365, true) {
        Ok(n) => log::info!("migrate_v48: stitched {n} orphaned messages into existing threads"),
        Err(e) => log::warn!("migrate_v48: thread stitching failed (threads stay fragmented): {e}"),
    }
    Ok(())
}

/// v47: dismissed follow-up / reply nudges.
///
/// Keyed on the THREAD, not on a message. A nudge is a statement about a
/// conversation ("you sent this 5 days ago and nobody answered"), so dismissing
/// it must silence the conversation — keying on the message uid would let the
/// next message in the same thread resurrect it, which is the mistake
/// `needs_you_dismissals` had to fix wholesale with `dismiss_group`.
///
/// `thread_key` mirrors the grouping the message list itself uses:
/// `COALESCE(thread_root_id, message_id, 'uid:'||folder||':'||uid)`. It is
/// deliberately NOT a foreign key — the row a thread key was derived from can be
/// archived or deleted while the conversation lives on in another folder, and a
/// CASCADE there would silently un-dismiss the nudge.
pub(crate) fn migrate_v47_nudge_dismissals(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS nudge_dismissals (
            account_id   TEXT NOT NULL,
            thread_key   TEXT NOT NULL,
            kind         TEXT NOT NULL,
            dismissed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, thread_key, kind)
        );",
    )
}

/// v46: verbatim RFC 5322 header blocks.
///
/// A SEPARATE TABLE, not a `raw_headers` column on `message_bodies`, and that
/// choice is load-bearing. `message_bodies` doubles as the "is the body cached?"
/// record: `db::messages::get_cached_body_uids` drives sync's body prefetch and
/// `get_body` returning `Some` is what makes `fetch_message_body` and the MCP's
/// `read_email` skip IMAP. A header-only lazy fetch (the point of this feature —
/// see `mcp::server::read_email_source`) would have to create a
/// `message_bodies` row with NULL `plain_text`, which suppresses prefetch for
/// that message and makes `read_email` report an empty body as a cache hit. It
/// would also drag the write into the gotcha #14 `insert_body` family and the
/// gotcha #26 FTS trigger surface for no benefit. Keyed identically and
/// ON DELETE CASCADE, so move/archive/delete evict headers in the same
/// transaction they evict everything else.
///
/// **No backfill, deliberately.** The header block cannot be reconstructed from
/// anything already in the DB — sync discards the raw bytes and only ever asked
/// the server for `BODY.PEEK[HEADER.FIELDS (…)]`, a named subset. Rows appear
/// (a) for free whenever a full raw message passes through
/// `persist_parsed_metadata` from here on, and (b) on demand, one `BODY.PEEK
/// [HEADER]` fetch for a single UID, the first time anyone asks for that
/// message's source. So already-synced mail has no headers stored until it is
/// looked at, and the first look costs one small IMAP round trip. Widening the
/// sync FETCH was rejected: on a 14.6k-message mailbox that is megabytes of
/// header text pulled for mail nobody will audit.
pub(crate) fn migrate_v46_message_headers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_headers (
            account_id  TEXT NOT NULL,
            folder_name TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            raw_headers TEXT NOT NULL,
            fetched_at  TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, folder_name, uid),
            FOREIGN KEY (account_id, folder_name, uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );",
    )
}

fn migrate_v42_needs_you_dismissals(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS needs_you_dismissals (
            account_id  TEXT NOT NULL,
            folder_name TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            dismissed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, folder_name, uid),
            FOREIGN KEY (account_id, folder_name, uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );",
    )
}

pub(crate) fn migrate_v43_gcal(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gcal_calendars (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            gcal_calendar_id TEXT NOT NULL,
            summary TEXT,
            time_zone TEXT,
            access_role TEXT,
            bg_color TEXT,
            is_primary INTEGER NOT NULL DEFAULT 0,
            selected INTEGER NOT NULL DEFAULT 1,
            sync_token TEXT,
            sync_window_days INTEGER NOT NULL DEFAULT 90,
            last_synced_at TEXT,
            last_error TEXT,
            last_error_at TEXT,
            UNIQUE(account_id, gcal_calendar_id)
        );

        CREATE TABLE IF NOT EXISTS gcal_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            calendar_row_id INTEGER NOT NULL REFERENCES gcal_calendars(id) ON DELETE CASCADE,
            account_id TEXT NOT NULL,
            gcal_calendar_id TEXT NOT NULL,
            gcal_event_id TEXT NOT NULL,
            ical_uid TEXT,
            recurring_event_id TEXT,
            original_start_time TEXT,
            etag TEXT,
            sequence INTEGER NOT NULL DEFAULT 0,
            updated TEXT,
            summary TEXT,
            description TEXT,
            location TEXT,
            dtstart TEXT NOT NULL,
            dtend TEXT,
            is_all_day INTEGER NOT NULL DEFAULT 0,
            start_tz TEXT,
            end_tz TEXT,
            organizer_name TEXT,
            organizer_email TEXT,
            organizer_self INTEGER NOT NULL DEFAULT 0,
            attendees_json TEXT,
            self_response_status TEXT,
            status TEXT NOT NULL DEFAULT 'confirmed',
            transparency TEXT,
            html_link TEXT,
            hangout_link TEXT,
            conference_json TEXT,
            sync_state TEXT NOT NULL DEFAULT 'synced'
                CHECK(sync_state IN ('synced','local_new','local_dirty','local_deleted','conflict')),
            pending_notify INTEGER NOT NULL DEFAULT 0,
            push_attempts INTEGER NOT NULL DEFAULT 0,
            local_updated_at TEXT,
            last_seen_at TEXT,
            raw_json TEXT,
            remote_json TEXT,
            invite_approval_id TEXT,
            invite_approval_status TEXT
                CHECK(invite_approval_status IS NULL OR invite_approval_status IN ('pending','approved','denied')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, gcal_calendar_id, gcal_event_id)
        );

        CREATE INDEX IF NOT EXISTS idx_gcal_events_range
            ON gcal_events(account_id, dtstart);
        CREATE INDEX IF NOT EXISTS idx_gcal_events_dirty
            ON gcal_events(sync_state);
        CREATE INDEX IF NOT EXISTS idx_gcal_events_icaluid
            ON gcal_events(ical_uid);",
    )
}

/// Durable per-recipient writing-style groups. Assignments are append-only so
/// a style learned from older mail remains represented when newer messages
/// arrive; profiles can then be rebuilt from a balanced sample across groups.
/// Persist the recipient set a calendar-invite approval was granted for.
///
/// Delivery used to happen inside the blocking `send_calendar_invites` tool
/// call, so the approved snapshot could live in a stack local and the drift
/// check compared against it directly. Approval is non-blocking now — the MCP
/// returns immediately and the app performs the send whenever the user decides
/// — so that snapshot has to outlive the tool call, and an app restart, or the
/// check silently degrades to "notify whoever is on the event right now".
///
/// `invite_approval_requested_at` exists for the launch-time staleness sweep:
/// `request_invite_approval` refuses to open a second request while one is
/// pending, so an approval nobody ever answers would otherwise wedge that event
/// forever.
pub(crate) fn migrate_v45_invite_approval_snapshot(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "gcal_events", "invite_approval_recipients")? {
        conn.execute(
            "ALTER TABLE gcal_events ADD COLUMN invite_approval_recipients TEXT",
            [],
        )?;
    }
    if !table_has_column(conn, "gcal_events", "invite_approval_requested_at")? {
        conn.execute(
            "ALTER TABLE gcal_events ADD COLUMN invite_approval_requested_at TEXT",
            [],
        )?;
    }
    Ok(())
}

// `pub`, not `pub(crate)`: `email::voice`'s tests drive this migration directly to
// build their fixture, and that call is now cross-crate.
pub fn migrate_v44_recipient_voice_buckets(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS recipient_voice_buckets (
            account_id        TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            recipient_email   TEXT NOT NULL,
            bucket_id         TEXT NOT NULL,
            centroid_json     TEXT NOT NULL,
            sample_count      INTEGER NOT NULL DEFAULT 0,
            first_message_date TEXT NOT NULL,
            last_message_date  TEXT NOT NULL,
            created_at        TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, recipient_email, bucket_id)
        );
        CREATE INDEX IF NOT EXISTS idx_recipient_voice_buckets_lookup
            ON recipient_voice_buckets(account_id, recipient_email);

        CREATE TABLE IF NOT EXISTS recipient_voice_sample_buckets (
            account_id       TEXT NOT NULL,
            recipient_email  TEXT NOT NULL,
            folder_name      TEXT NOT NULL,
            uid              INTEGER NOT NULL,
            message_date     TEXT NOT NULL,
            bucket_id        TEXT NOT NULL,
            similarity       REAL NOT NULL,
            feature_json     TEXT NOT NULL,
            assigned_at      TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, recipient_email, folder_name, uid),
            FOREIGN KEY (account_id, recipient_email, bucket_id)
                REFERENCES recipient_voice_buckets(account_id, recipient_email, bucket_id)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_recipient_voice_samples_bucket
            ON recipient_voice_sample_buckets(account_id, recipient_email, bucket_id);
        CREATE INDEX IF NOT EXISTS idx_recipient_voice_samples_date
            ON recipient_voice_sample_buckets(account_id, recipient_email, message_date);
        ",
    )?;
    if !table_has_column(conn, "voice_profiles_recipient", "learning_version")? {
        conn.execute(
            "ALTER TABLE voice_profiles_recipient
             ADD COLUMN learning_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn get_schema_version(conn: &Connection) -> Result<i32> {
    // Check if schema_version table exists
    let table_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |row| row.get(0),
    )?;

    if !table_exists {
        return Ok(0);
    }

    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )
}

fn set_schema_version(conn: &Connection, version: i32) -> Result<()> {
    // The schema_version table has no UNIQUE/PK on `version`, so INSERT OR
    // REPLACE has no conflict target and would append a fresh row on every
    // launch (BUG-05: unbounded growth — observed at 500+ rows). Keep a single
    // high-water-mark row by clearing first. Wrapped in a transaction so a
    // concurrent reader never sees an empty table mid-update. This also collapses
    // any rows accumulated by older builds on the next launch.
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM schema_version", [])?;
    tx.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [version],
    )?;
    tx.commit()?;
    Ok(())
}

fn create_tables_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS accounts (
            id          TEXT PRIMARY KEY,
            email       TEXT NOT NULL UNIQUE,
            display_name TEXT,
            provider    TEXT NOT NULL,
            imap_host   TEXT NOT NULL,
            imap_port   INTEGER NOT NULL DEFAULT 993,
            smtp_host   TEXT NOT NULL,
            smtp_port   INTEGER NOT NULL DEFAULT 587,
            color       TEXT,
            is_active   INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS folders (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            name        TEXT NOT NULL,
            display_name TEXT,
            folder_type TEXT,
            delimiter   TEXT,
            flags       TEXT,
            total_count INTEGER DEFAULT 0,
            unread_count INTEGER DEFAULT 0,
            uidvalidity INTEGER,
            uidnext     INTEGER,
            last_synced TEXT,
            UNIQUE(account_id, name)
        );

        CREATE TABLE IF NOT EXISTS messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_name TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            message_id  TEXT,
            in_reply_to TEXT,
            reference_ids TEXT,
            subject     TEXT,
            from_name   TEXT,
            from_email  TEXT,
            to_list     TEXT,
            cc_list     TEXT,
            date        TEXT NOT NULL,
            snippet     TEXT,
            flags       TEXT,
            is_read     INTEGER NOT NULL DEFAULT 0,
            is_flagged  INTEGER NOT NULL DEFAULT 0,
            has_attachments INTEGER NOT NULL DEFAULT 0,
            size_bytes  INTEGER DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_name, uid)
        );

        CREATE TABLE IF NOT EXISTS message_bodies (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL,
            folder_name TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            plain_text  TEXT,
            html_body   TEXT,
            sanitized_html TEXT,
            fetched_at  TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_name, uid),
            FOREIGN KEY (account_id, folder_name, uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS attachments (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL,
            folder_name TEXT NOT NULL,
            message_uid INTEGER NOT NULL,
            filename    TEXT,
            content_type TEXT,
            size_bytes  INTEGER,
            content_id  TEXT,
            is_inline   INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (account_id, folder_name, message_uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS sync_state (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_name TEXT NOT NULL,
            last_uid    INTEGER DEFAULT 0,
            uidvalidity INTEGER,
            last_synced TEXT,
            UNIQUE(account_id, folder_name)
        );

        CREATE INDEX IF NOT EXISTS idx_messages_account_folder ON messages(account_id, folder_name);
        CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, folder_name, date DESC);
        CREATE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id);
        CREATE INDEX IF NOT EXISTS idx_messages_in_reply_to ON messages(in_reply_to);
        CREATE INDEX IF NOT EXISTS idx_messages_is_read ON messages(account_id, folder_name, is_read);
        CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);
        ",
    )?;
    Ok(())
}

fn create_tables_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS mail_rules (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT REFERENCES accounts(id) ON DELETE CASCADE,
            name        TEXT NOT NULL,
            is_active   INTEGER NOT NULL DEFAULT 1,
            priority    INTEGER NOT NULL DEFAULT 0,
            conditions  TEXT NOT NULL,
            actions     TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS identities (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            email       TEXT NOT NULL,
            display_name TEXT,
            signature_html TEXT,
            is_default  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_identities_account ON identities(account_id);
        ",
    )?;
    Ok(())
}

fn create_tables_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS snoozed_messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_name TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            wake_at     TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_name, uid)
        );

        CREATE INDEX IF NOT EXISTS idx_snoozed_wake_at ON snoozed_messages(wake_at);

        CREATE TABLE IF NOT EXISTS scheduled_emails (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            email_json      TEXT NOT NULL,
            send_at         TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'pending',
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_scheduled_send_at ON scheduled_emails(send_at, status);
        ",
    )?;
    Ok(())
}

fn create_tables_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE message_bodies ADD COLUMN summary TEXT;
        ALTER TABLE message_bodies ADD COLUMN summary_model TEXT;
        ",
    )?;
    Ok(())
}

fn create_tables_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tracking_pixels (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            pixel_code      TEXT NOT NULL UNIQUE,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            message_id      TEXT,
            to_email        TEXT NOT NULL,
            subject         TEXT,
            open_count      INTEGER NOT NULL DEFAULT 0,
            first_open_at   TEXT,
            last_open_at    TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_tracking_pixels_code ON tracking_pixels(pixel_code);
        CREATE INDEX IF NOT EXISTS idx_tracking_pixels_account ON tracking_pixels(account_id);
        CREATE INDEX IF NOT EXISTS idx_tracking_pixels_message_id ON tracking_pixels(message_id);
        CREATE INDEX IF NOT EXISTS idx_tracking_pixels_created ON tracking_pixels(created_at DESC);

        CREATE TABLE IF NOT EXISTS tracking_config (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            is_enabled      INTEGER NOT NULL DEFAULT 0,
            service_url     TEXT,
            api_key         TEXT,
            last_synced     TEXT
        );
        ",
    )?;
    Ok(())
}

fn create_tables_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE messages ADD COLUMN category TEXT;
        ALTER TABLE messages ADD COLUMN category_source TEXT;

        CREATE INDEX IF NOT EXISTS idx_messages_category
            ON messages(account_id, folder_name, category);

        CREATE TABLE IF NOT EXISTS sender_categories (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            email_pattern   TEXT NOT NULL UNIQUE,
            category        TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS followup_reminders (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            sent_message_id TEXT NOT NULL,
            sender_email    TEXT NOT NULL,
            to_email        TEXT NOT NULL,
            subject         TEXT,
            remind_at       TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'pending',
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, sent_message_id)
        );

        CREATE INDEX IF NOT EXISTS idx_followup_remind_at
            ON followup_reminders(remind_at, status);
        CREATE INDEX IF NOT EXISTS idx_followup_message_id
            ON followup_reminders(sent_message_id);
        ",
    )?;
    Ok(())
}

fn create_tables_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE messages ADD COLUMN list_unsubscribe TEXT;
        ALTER TABLE messages ADD COLUMN list_unsubscribe_post TEXT;

        CREATE TABLE IF NOT EXISTS email_templates (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT REFERENCES accounts(id) ON DELETE CASCADE,
            name        TEXT NOT NULL,
            subject     TEXT NOT NULL DEFAULT '',
            html_body   TEXT NOT NULL DEFAULT '',
            plain_body  TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_templates_account ON email_templates(account_id);

        CREATE TABLE IF NOT EXISTS calendar_events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_name     TEXT NOT NULL,
            message_uid     INTEGER NOT NULL,
            event_uid       TEXT,
            summary         TEXT,
            description     TEXT,
            location        TEXT,
            dtstart         TEXT NOT NULL,
            dtend           TEXT,
            organizer_name  TEXT,
            organizer_email TEXT,
            status          TEXT,
            method          TEXT,
            rsvp_status     TEXT DEFAULT 'needs-action',
            raw_ics         TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id, folder_name, message_uid)
                REFERENCES messages(account_id, folder_name, uid) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_calendar_events_message
            ON calendar_events(account_id, folder_name, message_uid);
        CREATE INDEX IF NOT EXISTS idx_calendar_events_date ON calendar_events(dtstart);
        ",
    )?;
    Ok(())
}

fn create_tables_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE messages ADD COLUMN is_muted INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE messages ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;
        ",
    )?;
    Ok(())
}

/// Convert all non-ISO dates to ISO 8601 UTC so ORDER BY datetime(date) works.
fn migrate_dates_to_iso8601(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT rowid, date FROM messages")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut update = conn.prepare("UPDATE messages SET date = ?1 WHERE rowid = ?2")?;
    for (rowid, date_str) in &rows {
        // Skip if already ISO 8601 (e.g., "2026-03-28T07:48:21+00:00")
        // ISO dates start with a 4-digit year; non-ISO start with 1-2 digit day
        if date_str.len() > 4
            && date_str[..4].chars().all(|c| c.is_ascii_digit())
            && date_str.as_bytes().get(4) == Some(&b'-')
        {
            continue;
        }
        // Try RFC 2822 first (e.g., "Wed, 28 Jan 2026 14:39:35 +0000")
        if let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc2822(date_str) {
            let iso = dt.with_timezone(&Utc).to_rfc3339();
            update.execute(rusqlite::params![iso, rowid])?;
            continue;
        }
        // Handle dates without day-of-week (e.g., "14 Oct 2025 23:02:24 -0000")
        // Split into datetime part and timezone part
        let parts: Vec<&str> = date_str.trim().rsplitn(2, ' ').collect();
        if parts.len() == 2 {
            let tz_str = parts[0]; // e.g., "-0000"
            let dt_str = parts[1]; // e.g., "14 Oct 2025 23:02:24"
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(dt_str, "%d %b %Y %H:%M:%S") {
                let tz_secs = parse_tz_offset(tz_str);
                if let Some(offset) = chrono::FixedOffset::east_opt(tz_secs) {
                    if let Some(dt_with_tz) = dt.and_local_timezone(offset).single() {
                        let iso = dt_with_tz.with_timezone(&Utc).to_rfc3339();
                        update.execute(rusqlite::params![iso, rowid])?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn create_tables_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE message_bodies ADD COLUMN to_json TEXT;
        ALTER TABLE message_bodies ADD COLUMN cc_json TEXT;
        ",
    )?;
    Ok(())
}

fn create_tables_v13(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE messages ADD COLUMN thread_count INTEGER NOT NULL DEFAULT 0;")?;
    Ok(())
}

fn create_tables_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        ALTER TABLE calendar_events ADD COLUMN source TEXT NOT NULL DEFAULT 'ics';
        ALTER TABLE calendar_events ADD COLUMN confidence REAL;
        ALTER TABLE calendar_events ADD COLUMN dismissed INTEGER NOT NULL DEFAULT 0;
        ",
    )?;
    Ok(())
}

fn create_tables_v15(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS inbox_groups (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            color       TEXT NOT NULL DEFAULT '#0a84ff',
            icon        TEXT NOT NULL DEFAULT 'folder',
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS inbox_group_rules (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id    INTEGER NOT NULL REFERENCES inbox_groups(id) ON DELETE CASCADE,
            field       TEXT NOT NULL,
            operator    TEXT NOT NULL,
            value       TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_inbox_group_rules_group ON inbox_group_rules(group_id);
        ",
    )?;

    // Seed default groups: Business, Music, Personal
    conn.execute(
        "INSERT INTO inbox_groups (name, color, icon, sort_order) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["Business", "#0a84ff", "briefcase", 0],
    )?;
    let business_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO inbox_groups (name, color, icon, sort_order) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["Music", "#ff375f", "music", 1],
    )?;
    let music_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO inbox_groups (name, color, icon, sort_order) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["Personal", "#30d158", "user", 2],
    )?;
    let personal_id = conn.last_insert_rowid();

    // Seed example rules — users can customize these
    let mut stmt = conn.prepare(
        "INSERT INTO inbox_group_rules (group_id, field, operator, value) VALUES (?1, ?2, ?3, ?4)",
    )?;

    // Business rules
    stmt.execute(rusqlite::params![
        business_id,
        "from_email",
        "contains",
        "invoice"
    ])?;
    stmt.execute(rusqlite::params![
        business_id,
        "from_email",
        "contains",
        "noreply"
    ])?;
    stmt.execute(rusqlite::params![
        business_id,
        "subject",
        "contains",
        "invoice"
    ])?;
    stmt.execute(rusqlite::params![
        business_id,
        "subject",
        "contains",
        "meeting"
    ])?;

    // Music rules
    stmt.execute(rusqlite::params![
        music_id,
        "from_email",
        "contains",
        "spotify"
    ])?;
    stmt.execute(rusqlite::params![
        music_id,
        "from_email",
        "contains",
        "distrokid"
    ])?;
    stmt.execute(rusqlite::params![
        music_id,
        "from_email",
        "contains",
        "soundcloud"
    ])?;
    stmt.execute(rusqlite::params![
        music_id,
        "from_email",
        "contains",
        "bandcamp"
    ])?;
    stmt.execute(rusqlite::params![music_id, "subject", "contains", "music"])?;

    // Personal rules
    stmt.execute(rusqlite::params![
        personal_id,
        "from_email",
        "contains",
        "gmail.com"
    ])?;
    stmt.execute(rusqlite::params![
        personal_id,
        "from_email",
        "contains",
        "icloud.com"
    ])?;
    stmt.execute(rusqlite::params![
        personal_id,
        "subject",
        "contains",
        "family"
    ])?;

    Ok(())
}

fn create_tables_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS inbox_group_accounts (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id    INTEGER NOT NULL REFERENCES inbox_groups(id) ON DELETE CASCADE,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            UNIQUE(group_id, account_id)
        );

        CREATE INDEX IF NOT EXISTS idx_inbox_group_accounts_group ON inbox_group_accounts(group_id);
        ",
    )?;
    Ok(())
}

fn create_tables_v17(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_message_bodies_fetched_at ON message_bodies(fetched_at);",
    )?;
    Ok(())
}

fn migrate_v18_account_sort_order(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE accounts ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;")?;
    // Backfill sequential sort_order based on existing created_at order
    conn.execute_batch(
        "UPDATE accounts SET sort_order = (
            SELECT COUNT(*) FROM accounts a2 WHERE a2.created_at < accounts.created_at
        );",
    )?;
    Ok(())
}

fn migrate_v19_account_group(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE accounts ADD COLUMN group_name TEXT;")?;
    Ok(())
}

fn migrate_v25_account_notify_enabled(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE accounts ADD COLUMN notify_enabled INTEGER NOT NULL DEFAULT 1;",
    )?;
    Ok(())
}

fn migrate_v41_account_track_opens_enabled(conn: &Connection) -> Result<()> {
    // Guarded so a re-run never clobbers per-account choices made after the
    // first migration. Seeds from the old global tracking_config.is_enabled
    // (per-account flags replace it as the injection gate).
    if !table_has_column(conn, "accounts", "track_opens_enabled")? {
        conn.execute_batch(
            "ALTER TABLE accounts ADD COLUMN track_opens_enabled INTEGER NOT NULL DEFAULT 0;
             UPDATE accounts SET track_opens_enabled =
                 COALESCE((SELECT is_enabled FROM tracking_config WHERE id = 1), 0);",
        )?;
    }
    Ok(())
}

fn migrate_v26_voice_profiles(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "BEGIN;
        CREATE TABLE IF NOT EXISTS voice_profiles (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id          TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
            voice_profile_json  TEXT NOT NULL,
            voice_examples_json TEXT NOT NULL,
            model_used          TEXT,
            sample_count        INTEGER NOT NULL DEFAULT 0,
            generated_at        TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_voice_profiles_account ON voice_profiles(account_id);
        COMMIT;",
    )?;
    Ok(())
}

fn migrate_v27_voice_edits_and_insights(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "BEGIN;
        CREATE TABLE IF NOT EXISTS ai_draft_edits (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            ai_draft    TEXT NOT NULL,
            sent_body   TEXT NOT NULL,
            similarity  REAL NOT NULL DEFAULT 0.0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_ai_draft_edits_account ON ai_draft_edits(account_id, created_at DESC);

        CREATE TABLE IF NOT EXISTS voice_learning_insights (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            insight_text       TEXT NOT NULL,
            prompt_addition    TEXT NOT NULL,
            source_edit_count  INTEGER NOT NULL DEFAULT 0,
            confidence         REAL NOT NULL DEFAULT 0.0,
            is_active          INTEGER NOT NULL DEFAULT 0,
            dedupe_key         TEXT NOT NULL,
            created_at         TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at         TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, dedupe_key)
        );
        CREATE INDEX IF NOT EXISTS idx_voice_learning_active ON voice_learning_insights(account_id, is_active);
        COMMIT;",
    )?;
    Ok(())
}

fn migrate_v28_attachment_metadata_checked(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "message_bodies", "attachment_metadata_checked")? {
        conn.execute(
            "ALTER TABLE message_bodies ADD COLUMN attachment_metadata_checked INTEGER NOT NULL DEFAULT 0;",
            [],
        )?;
    }

    conn.execute_batch(
        "
        UPDATE messages
        SET has_attachments = EXISTS (
            SELECT 1
            FROM attachments a
            WHERE a.account_id = messages.account_id
              AND a.folder_name = messages.folder_name
              AND a.message_uid = messages.uid
              AND NOT (a.is_inline = 1 AND a.content_id IS NOT NULL)
        );

        UPDATE message_bodies
        SET attachment_metadata_checked = 1
        WHERE EXISTS (
            SELECT 1
            FROM attachments a
            WHERE a.account_id = message_bodies.account_id
              AND a.folder_name = message_bodies.folder_name
              AND a.message_uid = message_bodies.uid
        );
        ",
    )?;
    Ok(())
}

fn migrate_v20_unsubscribed_senders(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS unsubscribed_senders (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            sender_email    TEXT NOT NULL,
            sender_domain   TEXT NOT NULL,
            method          TEXT NOT NULL DEFAULT 'one-click',
            unsubscribed_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, sender_email)
        );
        CREATE INDEX IF NOT EXISTS idx_unsubscribed_senders_lookup
            ON unsubscribed_senders(account_id, sender_email);",
    )?;
    Ok(())
}

fn migrate_v21_in_reply_to_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_messages_in_reply_to ON messages(in_reply_to);",
    )?;
    Ok(())
}

/// v29: introduce `thread_root_id` so the list query can collapse threads.
/// Computed in Rust via `compute_thread_root_id` to handle the raw RFC 5322
/// `References` header (whitespace-separated `<id> <id>`) which a single-pass
/// SQL `SUBSTR/INSTR` cannot parse correctly.
fn migrate_v29_thread_root_id(conn: &Connection) -> Result<()> {
    use cxmail_core::mail::message_id::compute_thread_root_id;

    if !table_has_column(conn, "messages", "thread_root_id")? {
        conn.execute("ALTER TABLE messages ADD COLUMN thread_root_id TEXT;", [])?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_messages_thread_root
            ON messages(account_id, thread_root_id);",
    )?;

    // Backfill in pages so we never load the full table into memory.
    let page_size: i64 = 1000;
    loop {
        let mut select = conn.prepare(
            "SELECT id, folder_name, uid, message_id, in_reply_to, reference_ids
               FROM messages
              WHERE thread_root_id IS NULL
              LIMIT ?1",
        )?;
        let rows: Vec<(
            i64,
            String,
            u32,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = select
            .query_map(rusqlite::params![page_size], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u32,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        if rows.is_empty() {
            break;
        }

        let tx = conn.unchecked_transaction()?;
        {
            let mut update = tx.prepare("UPDATE messages SET thread_root_id = ?1 WHERE id = ?2")?;
            for (id, folder_name, uid, message_id, in_reply_to, reference_ids) in &rows {
                let root = compute_thread_root_id(
                    message_id.as_deref(),
                    in_reply_to.as_deref(),
                    reference_ids.as_deref(),
                    folder_name,
                    *uid,
                );
                update.execute(rusqlite::params![root, id])?;
            }
        }
        tx.commit()?;

        // If we got fewer rows than the page size, we're done.
        if (rows.len() as i64) < page_size {
            break;
        }
    }

    Ok(())
}

fn migrate_v22_sync_state_reconciliation_columns(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "sync_state", "last_flags_reconciled_at")? {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN last_flags_reconciled_at TEXT;",
            [],
        )?;
    }
    if !table_has_column(conn, "sync_state", "last_existence_reconciled_at")? {
        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN last_existence_reconciled_at TEXT;",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v23_sync_state_error_columns(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "sync_state", "last_error")? {
        conn.execute("ALTER TABLE sync_state ADD COLUMN last_error TEXT;", [])?;
    }
    if !table_has_column(conn, "sync_state", "last_error_at")? {
        conn.execute("ALTER TABLE sync_state ADD COLUMN last_error_at TEXT;", [])?;
    }
    Ok(())
}

/// v30: per-sender allowlist for remote-image rendering. Global (no
/// `account_id`): trusting `newsletters@x.com` once should apply to every
/// inbox the user has, since the trust is about the sender, not the receiving
/// mailbox. Different shape from `unsubscribed_senders` (which is per-account
/// because the IMAP unsubscribe action is per-account).
fn migrate_v30_trusted_image_senders(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trusted_image_senders (
            sender_email TEXT PRIMARY KEY,
            added_at     INTEGER NOT NULL DEFAULT (unixepoch())
        );",
    )?;
    Ok(())
}

/// v32: backfill cleanup for subjects with literal JSON-style `\"` and `\\`
/// escapes (Amazon's templating engine emits these into the RFC 5322 Subject
/// header). Going forward, both ingress paths normalize on parse — see
/// `normalize_unstructured_header` in `cxmail_core::mail::text`.
fn migrate_v32_unescape_subjects(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, subject FROM messages WHERE subject IS NOT NULL AND instr(subject, '\\') > 0",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut update = conn.prepare("UPDATE messages SET subject = ?1 WHERE id = ?2")?;
    for (id, subject) in rows {
        let normalized = cxmail_core::mail::text::normalize_unstructured_header(&subject);
        if normalized != subject {
            update.execute(rusqlite::params![normalized, id])?;
        }
    }
    Ok(())
}

/// v33: per-recipient voice profiles + AI-clustered archetypes.
/// `voice_profiles_recipient` caches a distilled voice JSON for each (account, recipient).
/// `voice_archetypes` stores free-form clustered email archetypes (e.g. "warm personal", "terse ops").
/// `message_archetype` assigns sent messages to archetypes; folder_name is required because UIDs are folder-scoped.
/// Also adds nullable `recipient_email` to `ai_draft_edits` for future recipient-scoped learning.
fn migrate_v33_recipient_voice_and_archetypes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "BEGIN;
        CREATE TABLE IF NOT EXISTS voice_profiles_recipient (
            account_id                  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            recipient_email             TEXT NOT NULL,
            profile_json                TEXT NOT NULL,
            sample_count                INTEGER NOT NULL DEFAULT 0,
            last_extracted_message_date TEXT NOT NULL,
            model_used                  TEXT,
            generated_at                TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, recipient_email)
        );
        CREATE INDEX IF NOT EXISTS idx_voice_recipient_account
            ON voice_profiles_recipient(account_id);

        CREATE TABLE IF NOT EXISTS voice_archetypes (
            account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            archetype_id TEXT NOT NULL,
            name         TEXT NOT NULL,
            description  TEXT NOT NULL DEFAULT '',
            profile_json TEXT NOT NULL,
            sample_count INTEGER NOT NULL DEFAULT 0,
            model_used   TEXT,
            generated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, archetype_id)
        );
        CREATE INDEX IF NOT EXISTS idx_voice_archetypes_account
            ON voice_archetypes(account_id);

        CREATE TABLE IF NOT EXISTS message_archetype (
            account_id   TEXT NOT NULL,
            folder_name  TEXT NOT NULL,
            uid          INTEGER NOT NULL,
            archetype_id TEXT NOT NULL,
            assigned_at  TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (account_id, folder_name, uid),
            FOREIGN KEY (account_id, archetype_id)
                REFERENCES voice_archetypes(account_id, archetype_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_msg_arch_archetype
            ON message_archetype(account_id, archetype_id);
        COMMIT;",
    )?;

    if !table_has_column(conn, "ai_draft_edits", "recipient_email")? {
        conn.execute(
            "ALTER TABLE ai_draft_edits ADD COLUMN recipient_email TEXT;",
            [],
        )?;
    }
    Ok(())
}

/// v34: backfill `messages.to_list` / `cc_list` from `message_bodies.to_json`
/// for old prefetched messages. Pre-v34, the IMAP sync path hardcoded these
/// columns to `"[]"` (gotcha: `messages.to_list` was unusable for the voice
/// per-recipient queries). Going forward, `parse_fetch_to_header` populates
/// them from the IMAP envelope. This migration recovers what we can from
/// already-cached body data so the voice feature works on existing DBs
/// without forcing a re-sync.
fn migrate_v34_backfill_to_list(conn: &Connection) -> Result<()> {
    // Both columns must already exist on the messages table — they've been
    // there since v1. We're only changing how they're populated.
    conn.execute(
        "UPDATE messages
         SET to_list = (
             SELECT mb.to_json
             FROM message_bodies mb
             WHERE mb.account_id = messages.account_id
               AND mb.folder_name = messages.folder_name
               AND mb.uid = messages.uid
               AND mb.to_json IS NOT NULL
               AND mb.to_json != ''
         )
         WHERE (to_list IS NULL OR to_list = '' OR to_list = '[]')
           AND EXISTS (
             SELECT 1 FROM message_bodies mb
             WHERE mb.account_id = messages.account_id
               AND mb.folder_name = messages.folder_name
               AND mb.uid = messages.uid
               AND mb.to_json IS NOT NULL
               AND mb.to_json != ''
         )",
        [],
    )?;
    conn.execute(
        "UPDATE messages
         SET cc_list = (
             SELECT mb.cc_json
             FROM message_bodies mb
             WHERE mb.account_id = messages.account_id
               AND mb.folder_name = messages.folder_name
               AND mb.uid = messages.uid
               AND mb.cc_json IS NOT NULL
               AND mb.cc_json != ''
         )
         WHERE (cc_list IS NULL OR cc_list = '' OR cc_list = '[]')
           AND EXISTS (
             SELECT 1 FROM message_bodies mb
             WHERE mb.account_id = messages.account_id
               AND mb.folder_name = messages.folder_name
               AND mb.uid = messages.uid
               AND mb.cc_json IS NOT NULL
               AND mb.cc_json != ''
         )",
        [],
    )?;
    Ok(())
}

/// v35: re-normalize `messages.date` values that are present but not ISO-8601.
/// Spam senders append junk after the timezone offset (e.g.
/// "Wed, 25 Mar 2026 10:05:36 +0000.1731-577592") which the live parser used to
/// store raw — those then sort to the TOP of date-DESC views (BUG-01). Re-run
/// the (now salvaging) normalizer; truly unrecoverable values become a low
/// sentinel so they sort to the bottom instead.
fn migrate_v35_salvage_malformed_dates(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT rowid, date FROM messages")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut update = conn.prepare("UPDATE messages SET date = ?1 WHERE rowid = ?2")?;
    for (rowid, date_str) in &rows {
        // Skip rows that are already ISO-8601 (4-digit year + dash) or empty
        // (empty already sorts to the bottom).
        if date_str.is_empty()
            || (date_str.len() > 4
                && date_str[..4].chars().all(|c| c.is_ascii_digit())
                && date_str.as_bytes().get(4) == Some(&b'-'))
        {
            continue;
        }
        let normalized = cxmail_core::mail::date::normalize_date_to_iso8601(date_str);
        if &normalized != date_str {
            update.execute(rusqlite::params![normalized, rowid])?;
        }
    }
    Ok(())
}

/// v37: drop the vestigial `messages.thread_count` column (bug-bash BUG-06).
/// It stored an immediate reply-neighbor count maintained on every sync by a
/// correlated-subquery UPDATE, but no view ever surfaced it — list/detail paths
/// recompute thread size live via `thread_root_id` as `(total_count - 1)`. The
/// write was removed; this drops the dead column. Guarded so re-runs are no-ops.
fn migrate_v37_drop_thread_count(conn: &Connection) -> Result<()> {
    if table_has_column(conn, "messages", "thread_count")? {
        conn.execute_batch("ALTER TABLE messages DROP COLUMN thread_count;")?;
    }
    Ok(())
}

/// v38: backfill heuristic-detected calendar events for already-synced mail.
/// Detection previously ran only at sync-ingest (`persist_parsed_metadata`), so
/// plain-text meeting invites sitting in cached messages — e.g. a forwarded
/// Zoom invite with no `text/calendar` part — never reached the calendar. Re-run
/// `detect_events` over every message that has no calendar event yet and insert
/// any result at confidence >= 0.6 (the same threshold the live path uses).
///
/// Keyset-paginated by `messages.id` (not OFFSET) so inserting events mid-scan
/// can't shift a page window; each page commits in its own transaction to avoid
/// one giant write lock. Idempotent: the `NOT EXISTS` filter plus
/// `insert_detected`'s existing-row guard prevent duplicates and preserve any
/// user-set `dismissed` flag. Per-row insert errors are logged and skipped so
/// one bad row can't abort the whole backfill.
fn migrate_v38_backfill_detected_events(conn: &Connection) -> Result<()> {
    const PAGE: usize = 1000;
    type Row = (
        i64,            // messages.id (keyset cursor)
        String,         // account_id
        String,         // folder_name
        u32,            // uid
        Option<String>, // subject
        Option<String>, // from_email
        String,         // date
        Option<String>, // plain_text
    );

    let mut last_id: i64 = 0;
    loop {
        // Materialize a page first so the SELECT statement is dropped before we
        // open the write transaction below.
        let rows: Vec<Row> = {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.account_id, m.folder_name, m.uid,
                        m.subject, m.from_email, m.date, mb.plain_text
                 FROM messages m
                 LEFT JOIN message_bodies mb
                   ON mb.account_id = m.account_id
                  AND mb.folder_name = m.folder_name
                  AND mb.uid = m.uid
                 WHERE m.id > ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM calendar_events ce
                       WHERE ce.account_id = m.account_id
                         AND ce.folder_name = m.folder_name
                         AND ce.message_uid = m.uid
                   )
                 ORDER BY m.id ASC
                 LIMIT ?2",
            )?;
            let page = stmt
                .query_map(rusqlite::params![last_id, PAGE as i64], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>>>()?;
            page
        };

        if rows.is_empty() {
            break;
        }
        let page_len = rows.len();
        last_id = rows.last().map(|r| r.0).unwrap_or(last_id);

        let tx = conn.unchecked_transaction()?;
        for (_id, account_id, folder_name, uid, subject, from_email, date, plain_text) in &rows {
            let detected = cxmail_core::mail::detect_events::detect_events(
                subject.as_deref(),
                plain_text.as_deref(),
                from_email.as_deref().unwrap_or(""),
                date,
            );
            for event in &detected {
                if event.confidence >= 0.6 {
                    // `&tx` deref-coerces to `&Connection`. Log + skip on error.
                    if let Err(e) = crate::db::calendar::insert_detected(
                        &tx,
                        account_id,
                        folder_name,
                        *uid,
                        &event.summary,
                        &event.dtstart,
                        event.dtend.as_deref(),
                        event.location.as_deref(),
                        event.description.as_deref(),
                        event.confidence,
                    ) {
                        log::error!(
                            "v38 backfill: insert_detected failed for {account_id}/{folder_name}/{uid}: {e}"
                        );
                    }
                }
            }
        }
        tx.commit()?;

        if page_len < PAGE {
            break;
        }
    }

    Ok(())
}

/// v31: track whether `cid:` references in `sanitized_html` have been resolved
/// to inline `data:` URLs. Default 0 means existing cached messages will run
/// one self-heal pass on next open (see `fetch_message_body` cache HIT path).
fn migrate_v31_cid_resolved(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "message_bodies", "cid_resolved")? {
        conn.execute(
            "ALTER TABLE message_bodies ADD COLUMN cid_resolved INTEGER NOT NULL DEFAULT 0;",
            [],
        )?;
    }
    Ok(())
}

/// v39: add `message_bodies.bcc_json`, mirroring the v11 `to_json` / `cc_json`
/// columns. A saved draft's MIME carries a `Bcc:` header (build_draft_raw), but
/// nothing persisted it into the local body cache, so reopening/re-saving a
/// same-session draft silently dropped Bcc. New column is nullable; existing
/// rows stay NULL (the reopen path treats NULL as "no Bcc"). Guarded so re-runs
/// are no-ops.
fn migrate_v39_bcc_json(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "message_bodies", "bcc_json")? {
        conn.execute("ALTER TABLE message_bodies ADD COLUMN bcc_json TEXT;", [])?;
    }
    Ok(())
}

/// v40: replace the external Tantivy index with an SQLite FTS5 table kept
/// transactionally in sync by triggers. Because the triggers live in the DB
/// schema, every writer — the Tauri app, the helper daemon, and the MCP
/// binary — indexes automatically; nothing external can drift or corrupt.
///
/// Design notes (each pinned by a unit test below):
/// - `messages_fts.rowid` = `messages.id` (INTEGER PRIMARY KEY AUTOINCREMENT
///   → rowid alias, stable across VACUUM; messages are inserted with
///   INSERT OR IGNORE only, so no REPLACE rowid churn).
/// - `recursive_triggers` is never enabled anywhere, so `INSERT OR REPLACE`
///   into `message_bodies` fires only the INSERT trigger (the implicit
///   delete does NOT fire delete triggers) — the body lands in FTS with no
///   duplicate/clear race.
/// - FK CASCADE delete of `message_bodies`/`attachments` after a message
///   delete: those triggers' `messages` subquery returns NULL → `WHERE
///   rowid = NULL` no-op; the message's own delete trigger already removed
///   the FTS row.
/// - The UPDATE trigger is scoped to the indexed columns so `is_read` /
///   `is_flagged` flips don't rewrite FTS rows.
///
/// Idempotent: the virtual table is IF NOT EXISTS, triggers are dropped and
/// recreated, and the backfill (`db::search::rebuild`) clears the FTS table
/// before re-inserting.
pub(crate) fn migrate_v40_fts5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            subject, sender, recipients, body, attachment_names,
            tokenize = "porter unicode61 remove_diacritics 2",
            prefix = '2 3'
        );

        DROP TRIGGER IF EXISTS messages_fts_ai;
        DROP TRIGGER IF EXISTS messages_fts_ad;
        DROP TRIGGER IF EXISTS messages_fts_au;
        DROP TRIGGER IF EXISTS message_bodies_fts_ai;
        DROP TRIGGER IF EXISTS message_bodies_fts_au;
        DROP TRIGGER IF EXISTS message_bodies_fts_ad;
        DROP TRIGGER IF EXISTS attachments_fts_ai;
        DROP TRIGGER IF EXISTS attachments_fts_ad;

        CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, subject, sender, recipients, body, attachment_names)
            VALUES (new.id,
                    coalesce(new.subject, ''),
                    trim(coalesce(new.from_name, '') || ' ' || coalesce(new.from_email, '')),
                    trim(coalesce(new.to_list, '') || ' ' || coalesce(new.cc_list, '')),
                    '', '');
        END;

        CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
            DELETE FROM messages_fts WHERE rowid = old.id;
        END;

        CREATE TRIGGER messages_fts_au AFTER UPDATE OF subject, from_name, from_email, to_list, cc_list ON messages BEGIN
            UPDATE messages_fts SET
                subject    = coalesce(new.subject, ''),
                sender     = trim(coalesce(new.from_name, '') || ' ' || coalesce(new.from_email, '')),
                recipients = trim(coalesce(new.to_list, '') || ' ' || coalesce(new.cc_list, ''))
            WHERE rowid = new.id;
        END;

        CREATE TRIGGER message_bodies_fts_ai AFTER INSERT ON message_bodies BEGIN
            UPDATE messages_fts SET body = coalesce(new.plain_text, '')
            WHERE rowid = (SELECT id FROM messages WHERE account_id = new.account_id
                           AND folder_name = new.folder_name AND uid = new.uid);
        END;

        CREATE TRIGGER message_bodies_fts_au AFTER UPDATE OF plain_text ON message_bodies BEGIN
            UPDATE messages_fts SET body = coalesce(new.plain_text, '')
            WHERE rowid = (SELECT id FROM messages WHERE account_id = new.account_id
                           AND folder_name = new.folder_name AND uid = new.uid);
        END;

        CREATE TRIGGER message_bodies_fts_ad AFTER DELETE ON message_bodies BEGIN
            UPDATE messages_fts SET body = ''
            WHERE rowid = (SELECT id FROM messages WHERE account_id = old.account_id
                           AND folder_name = old.folder_name AND uid = old.uid);
        END;

        CREATE TRIGGER attachments_fts_ai AFTER INSERT ON attachments BEGIN
            UPDATE messages_fts SET attachment_names =
                (SELECT coalesce(group_concat(filename, ' '), '') FROM attachments
                 WHERE account_id = new.account_id AND folder_name = new.folder_name AND message_uid = new.message_uid)
            WHERE rowid = (SELECT id FROM messages WHERE account_id = new.account_id
                           AND folder_name = new.folder_name AND uid = new.message_uid);
        END;

        CREATE TRIGGER attachments_fts_ad AFTER DELETE ON attachments BEGIN
            UPDATE messages_fts SET attachment_names =
                (SELECT coalesce(group_concat(filename, ' '), '') FROM attachments
                 WHERE account_id = old.account_id AND folder_name = old.folder_name AND message_uid = old.message_uid)
            WHERE rowid = (SELECT id FROM messages WHERE account_id = old.account_id
                           AND folder_name = old.folder_name AND uid = old.message_uid);
        END;
        "#,
    )?;

    let count = crate::db::search::rebuild(conn)?;
    log::info!("v40: FTS5 backfill indexed {count} messages");
    Ok(())
}

/// v50: connection details for the generic `imap` provider.
///
/// Every default is chosen so this migration is a **no-op for the three
/// existing providers** — the rows end up describing exactly what
/// `connect_gmail` / `connect_icloud` / `connect_outlook` already hardcoded:
/// implicit TLS on IMAP 993, STARTTLS on SMTP 587 (`implicit_tls(false)`),
/// and authenticating as the account's own email address (NULL username).
///
/// `folders.special_use` records whether `folder_type` came from an RFC 6154
/// SPECIAL-USE attribute the server declared, rather than from a name guess.
/// It exists to make `db::folders::folder_of_type`'s ordering deterministic —
/// a server declaration outranks a heuristic — so it defaults to 0 for every
/// existing row, which is the truth: all of them were classified by name.
///
/// Guarded per column so a re-run cannot fail on an already-migrated DB.
fn migrate_v50_generic_imap(conn: &Connection) -> Result<()> {
    if !table_has_column(conn, "accounts", "imap_security")? {
        conn.execute_batch(
            "ALTER TABLE accounts ADD COLUMN imap_security TEXT NOT NULL DEFAULT 'implicit';",
        )?;
    }
    if !table_has_column(conn, "accounts", "smtp_security")? {
        conn.execute_batch(
            "ALTER TABLE accounts ADD COLUMN smtp_security TEXT NOT NULL DEFAULT 'starttls';",
        )?;
    }
    if !table_has_column(conn, "accounts", "imap_username")? {
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN imap_username TEXT;")?;
    }
    if !table_has_column(conn, "accounts", "smtp_username")? {
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN smtp_username TEXT;")?;
    }
    if !table_has_column(conn, "folders", "special_use")? {
        conn.execute_batch(
            "ALTER TABLE folders ADD COLUMN special_use INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    Ok(())
}

fn table_has_column(conn: &Connection, table_name: &str, column_name: &str) -> Result<bool> {
    let pragma = format!("PRAGMA table_info({})", table_name);
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for column in columns {
        if column? == column_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn parse_tz_offset(s: &str) -> i32 {
    // Parse "+0000", "-0700", "GMT", etc.
    match s {
        "GMT" | "UTC" => 0,
        s if s.len() >= 5 => {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let digits = &s[1..];
            let hours: i32 = digits[..2].parse().unwrap_or(0);
            let mins: i32 = digits[2..4].parse().unwrap_or(0);
            sign * (hours * 3600 + mins * 60)
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        initialize, migrate_v61_triage, migrate_v62_triage_tokens, migrate_v63_triage_withheld,
        get_schema_version, migrate_v22_sync_state_reconciliation_columns,
        migrate_v58_account_hidden_from_aggregates,
        migrate_v59_draft_attachment_blobs, migrate_v60_drafts,
        migrate_v32_unescape_subjects, migrate_v33_recipient_voice_and_archetypes,
        migrate_v34_backfill_to_list, migrate_v35_salvage_malformed_dates,
        migrate_v37_drop_thread_count, migrate_v38_backfill_detected_events, migrate_v39_bcc_json,
        migrate_v41_account_track_opens_enabled, migrate_v43_gcal,
        migrate_v44_recipient_voice_buckets, migrate_v50_generic_imap,
        migrate_v52_zoom_meetings, migrate_v54_repair_mojibake_snippets,
        migrate_v55_scrub_dash_claims_from_voice_profiles,
        migrate_v57_invite_notifications, set_schema_version, table_has_column,
    };
    use rusqlite::Connection;

    /// v54 repairs snippets the Latin-1 bug double-encoded, and — the assertion
    /// that matters — leaves everything else byte-for-byte alone. It runs over
    /// every stored snippet, so a transform that "fixed" legitimate accented
    /// prose would corrupt the mailbox at scale rather than repair it.
    #[test]
    fn migrate_v54_repairs_only_mojibake_snippets_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, snippet TEXT);
             INSERT INTO messages(snippet) VALUES
                 ('\u{e2}\u{80}\u{9c}A writer\u{e2}\u{80}\u{94}and, I believe'),
                 ('Caf\u{e9} au lait'),
                 ('plain ascii'),
                 ('\u{201C}already correct\u{201D}'),
                 (NULL),
                 ('');",
        )
        .unwrap();

        migrate_v54_repair_mojibake_snippets(&conn).unwrap();

        let snippet = |id: i64| -> Option<String> {
            conn.query_row("SELECT snippet FROM messages WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(
            snippet(1).as_deref(),
            Some("\u{201C}A writer\u{2014}and, I believe")
        );
        assert_eq!(snippet(2).as_deref(), Some("Caf\u{e9} au lait"));
        assert_eq!(snippet(3).as_deref(), Some("plain ascii"));
        assert_eq!(snippet(4).as_deref(), Some("\u{201C}already correct\u{201D}"));
        assert_eq!(snippet(5), None);
        assert_eq!(snippet(6).as_deref(), Some(""));

        // Re-running must be a no-op — an older binary stamps the version back
        // down, so the ladder re-runs this on the next new-build launch.
        migrate_v54_repair_mojibake_snippets(&conn).unwrap();
        assert_eq!(
            snippet(1).as_deref(),
            Some("\u{201C}A writer\u{2014}and, I believe")
        );
        assert_eq!(snippet(2).as_deref(), Some("Caf\u{e9} au lait"));
    }

    /// v55 stops stored profiles from prescribing the punctuation the pinned
    /// rule bans. Both fixtures are the REAL strings from the live DB.
    ///
    /// The assertion that matters is the third one: a profile with no dash
    /// claim must come through byte-for-byte. This runs over every stored
    /// profile, and a transform that mangled ordinary style prose would be fed
    /// into every future draft rather than caught by anything.
    #[test]
    fn migrate_v55_scrubs_only_dash_claims_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        // Schemas copied from the real ones. The first draft of this test
        // invented an `id` column on `voice_profiles_recipient`, which has a
        // COMPOSITE primary key and no such column — the test passed and the
        // migration failed against every real database.
        conn.execute_batch(
            r#"CREATE TABLE voice_profiles (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   account_id TEXT NOT NULL,
                   voice_profile_json TEXT NOT NULL,
                   voice_examples_json TEXT NOT NULL);
               CREATE TABLE voice_profiles_recipient (
                   account_id TEXT NOT NULL,
                   recipient_email TEXT NOT NULL,
                   profile_json TEXT NOT NULL,
                   PRIMARY KEY (account_id, recipient_email));
               INSERT INTO voice_profiles(account_id, voice_profile_json, voice_examples_json) VALUES
                 ('a', '{"punctuation_style":"Uses commas heavily, occasional em dashes, and frequent colon-led lists.","tone":"warm"}',
                  '[{"excerpt":"Happy to walk through it—say the word."}]'),
                 ('b', '{"punctuation_style":"Short sentences, plain words.","tone":"dry"}', '[]');
               INSERT INTO voice_profiles_recipient(account_id, recipient_email, profile_json) VALUES
                 ('a', 'dana@northwind.example', '{"punctuation_style":"Uses em-dashes, parentheses, bullets, and occasional numbered lists."}');"#,
        )
        .unwrap();

        migrate_v55_scrub_dash_claims_from_voice_profiles(&conn).unwrap();

        let account = |id: i64| -> String {
            conn.query_row(
                "SELECT voice_profile_json FROM voice_profiles WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        let recipient: String = conn
            .query_row(
                "SELECT profile_json FROM voice_profiles_recipient
                  WHERE recipient_email = 'dana@northwind.example'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert!(
            account(1).contains("Uses commas heavily, and frequent colon-led lists."),
            "claim not removed: {}",
            account(1)
        );
        assert!(
            account(1).contains("\"tone\":\"warm\""),
            "unrelated field lost: {}",
            account(1)
        );
        assert!(
            recipient.contains("Parentheses, bullets, and occasional numbered lists."),
            "claim not removed: {recipient}"
        );

        // A profile with nothing to scrub must come back untouched.
        assert_eq!(
            account(2),
            r#"{"punctuation_style":"Short sentences, plain words.","tone":"dry"}"#
        );

        // Verbatim excerpts of real sent mail are evidence, not instructions —
        // the migration must not rewrite the record of what was actually sent.
        let examples: String = conn
            .query_row(
                "SELECT voice_examples_json FROM voice_profiles WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            examples,
            r#"[{"excerpt":"Happy to walk through it—say the word."}]"#
        );

        // Idempotent: an older binary stamps the version back down, so the
        // ladder re-runs this on the next new-build launch.
        let before = (account(1), account(2), recipient.clone());
        migrate_v55_scrub_dash_claims_from_voice_profiles(&conn).unwrap();
        assert_eq!(account(1), before.0);
        assert_eq!(account(2), before.1);
    }

    /// v52 must add the Zoom side table and touch nothing else — and the second
    /// assertion is the tripwire that matters. If someone later "simplifies" the
    /// side table back into a `gcal_events.zoom_meeting_id` column, this fails
    /// immediately rather than after a sync silently nulls a real meeting id.
    /// See the migration's own docstring for the two failure modes.
    #[test]
    fn migrate_v57_invite_notifications_is_additive_idempotent_and_unlinked() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
             INSERT INTO accounts(id) VALUES ('acct');",
        )
        .unwrap();
        migrate_v43_gcal(&conn).unwrap();

        migrate_v57_invite_notifications(&conn).unwrap();
        // Idempotent: `initialize` re-runs the ladder whenever an older binary
        // stamps the version back down (gotchas #26 and #37).
        migrate_v57_invite_notifications(&conn).unwrap();

        conn.execute(
            "INSERT INTO invite_notifications
                (account_id, gcal_calendar_id, gcal_event_id, attendee_email)
             VALUES ('acct','primary','evt-1','dana@northwind.example')",
            [],
        )
        .unwrap();
        // Uniqueness is per attendee per event, so a second send refreshes one
        // row rather than appending a duplicate history entry.
        assert!(conn
            .execute(
                "INSERT INTO invite_notifications
                    (account_id, gcal_calendar_id, gcal_event_id, attendee_email)
                 VALUES ('acct','primary','evt-1','dana@northwind.example')",
                [],
            )
            .is_err());

        // No foreign keys at all. A swept mirror row or a removed account must
        // not erase the record that a person was told about a meeting — the
        // ledger row is evidence, and its event going away does not unsend mail.
        let fk_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('invite_notifications')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            fk_count, 0,
            "a CASCADE here would silently destroy delivery evidence"
        );

        // THE TRIPWIRE: this must never become a column on `gcal_events`.
        // `upsert_remote_event` rewrites that table wholesale from Google's
        // payload, which knows nothing about who we notified, so a column would
        // be nulled on the very next sync tick.
        assert!(
            !table_has_column(&conn, "gcal_events", "notified_at").unwrap(),
            "the delivery ledger cannot live on gcal_events; see gotcha #44"
        );
    }

    /// v58 adds one column with a safe default and nothing else. Idempotent
    /// because `initialize` re-runs the ladder whenever an older binary stamps
    /// the version back down — and a re-run must not reset a choice the user
    /// made in between.
    #[test]
    fn migrate_v59_draft_attachment_blobs_is_additive_idempotent_and_cascades() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE messages (account_id TEXT, folder_name TEXT, uid INTEGER,
                                    UNIQUE(account_id, folder_name, uid));
             INSERT INTO messages VALUES ('acct', 'Drafts', 1);",
        )
        .unwrap();
        migrate_v59_draft_attachment_blobs(&conn).unwrap();
        migrate_v59_draft_attachment_blobs(&conn).unwrap(); // idempotent
        conn.execute(
            "INSERT INTO draft_attachment_blobs (account_id, folder_name, message_uid, filename, content_type, data)
             VALUES ('acct', 'Drafts', 1, 'a.pdf', 'application/pdf', x'00')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM messages WHERE uid = 1", []).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM draft_attachment_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "blob rows must cascade with the messages row");
    }

    #[test]
    fn migrate_v62_triage_tokens_is_additive_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        migrate_v62_triage_tokens(&conn).unwrap(); // idempotent
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('triage_verdicts') WHERE name IN ('input_tokens','output_tokens')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn migrate_v61_triage_is_additive_idempotent_and_default_off() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        migrate_v61_triage(&conn).unwrap(); // idempotent
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('a', 'a@x.io', 'gmail', 'i', 's')",
            [],
        )
        .unwrap();
        let on: i64 = conn
            .query_row("SELECT triage_enabled FROM accounts WHERE id='a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(on, 0, "triage must default to OFF for every account");
        let fks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('triage_verdicts')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(fks > 0, "a verdict must die with its message");
    }

    #[test]
    fn migrate_v60_drafts_is_additive_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_v60_drafts(&conn).unwrap();
        migrate_v60_drafts(&conn).unwrap(); // idempotent
        conn.execute(
            "INSERT INTO drafts (draft_id, account_id, folder_name, uidvalidity, current_uid, created_at, updated_at)
             VALUES ('d', 'acct', 'Drafts', 1, 5, 'now', 'now')",
            [],
        )
        .unwrap();
        migrate_v60_drafts(&conn).unwrap(); // a re-run keeps existing rows
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM drafts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        // Deliberately NO FK to messages: a draft outlives every one of its
        // revisions, which are evicted on each save.
        let fks: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_list('drafts')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fks, 0, "drafts must not cascade with messages");
    }

    #[test]
    fn migrate_v58_account_hidden_from_aggregates_is_additive_idempotent_and_default_off() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY);
             INSERT INTO accounts(id) VALUES ('acct');",
        )
        .unwrap();

        migrate_v58_account_hidden_from_aggregates(&conn).unwrap();
        assert!(table_has_column(&conn, "accounts", "hidden_from_aggregates").unwrap());

        let hidden: i64 = conn
            .query_row(
                "SELECT hidden_from_aggregates FROM accounts WHERE id = 'acct'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hidden, 0, "existing accounts stay visible everywhere");

        conn.execute("UPDATE accounts SET hidden_from_aggregates = 1", [])
            .unwrap();
        migrate_v58_account_hidden_from_aggregates(&conn).unwrap();
        let after_rerun: i64 = conn
            .query_row(
                "SELECT hidden_from_aggregates FROM accounts WHERE id = 'acct'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_rerun, 1, "a re-run must not reset the user's choice");
    }

    #[test]
    fn migrate_v52_zoom_meetings_is_additive_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
             INSERT INTO accounts(id) VALUES ('acct');",
        )
        .unwrap();
        migrate_v43_gcal(&conn).unwrap();

        migrate_v52_zoom_meetings(&conn).unwrap();
        // Idempotent: `initialize` re-runs the ladder whenever an older binary
        // stamps the version back down (see gotchas #26 and #37).
        migrate_v52_zoom_meetings(&conn).unwrap();

        conn.execute(
            "INSERT INTO zoom_meetings
                (account_id, gcal_calendar_id, gcal_event_id, zoom_meeting_id, join_url, state)
             VALUES ('acct','primary','evt-1','869','https://z/869','active')",
            [],
        )
        .unwrap();
        // The UNIQUE index is on the remote triple, not on a local row id.
        assert!(conn
            .execute(
                "INSERT INTO zoom_meetings (account_id, gcal_calendar_id, gcal_event_id)
                 VALUES ('acct','primary','evt-1')",
                [],
            )
            .is_err());

        // No foreign keys at all — deleting a cached mirror row (or an account)
        // must not take the link with it, or the meeting is orphaned invisibly.
        let fk_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('zoom_meetings')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fk_count, 0, "a CASCADE here would erase a live Zoom meeting's id");

        // THE TRIPWIRE: the link must never live on `gcal_events`.
        assert!(
            !table_has_column(&conn, "gcal_events", "zoom_meeting_id").unwrap(),
            "`upsert_remote_event` rewrites gcal_events from Google's payload, which knows \
             nothing about Zoom — a column here is nulled on every sync tick"
        );
        assert!(!table_has_column(&conn, "gcal_events", "zoom_join_url").unwrap());
    }

    #[test]
    fn v43_adds_google_calendar_mirror_without_touching_mail_events() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
             CREATE TABLE calendar_events (
                 id INTEGER PRIMARY KEY,
                 account_id TEXT NOT NULL,
                 folder_name TEXT NOT NULL,
                 message_uid INTEGER NOT NULL,
                 dtstart TEXT NOT NULL
             );
             INSERT INTO accounts(id) VALUES ('acct');
             INSERT INTO calendar_events(id, account_id, folder_name, message_uid, dtstart)
             VALUES (1, 'acct', 'INBOX', 7, '2026-07-30T20:30:00Z');",
        )
        .unwrap();

        migrate_v43_gcal(&conn).unwrap();
        migrate_v43_gcal(&conn).unwrap();

        let mail_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM calendar_events", [], |r| r.get(0))
            .unwrap();
        let calendar_fk: String = conn
            .query_row(
                "SELECT \"table\" FROM pragma_foreign_key_list('gcal_events')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mail_count, 1);
        assert_eq!(calendar_fk, "gcal_calendars");
    }
    #[test]
    fn v41_seeds_track_opens_from_global_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL);
             CREATE TABLE tracking_config (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 is_enabled INTEGER NOT NULL DEFAULT 0,
                 service_url TEXT, api_key TEXT, last_synced TEXT);
             INSERT INTO tracking_config (id, is_enabled) VALUES (1, 1);
             INSERT INTO accounts (id, email) VALUES ('a1', 'one@example.com'), ('a2', 'two@example.com');",
        )
        .unwrap();
        assert!(!table_has_column(&conn, "accounts", "track_opens_enabled").unwrap());

        migrate_v41_account_track_opens_enabled(&conn).unwrap();
        let on_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE track_opens_enabled = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(on_count, 2, "global is_enabled=1 seeds all accounts on");

        // User flips one account off; re-running the migration must not clobber it.
        conn.execute(
            "UPDATE accounts SET track_opens_enabled = 0 WHERE id = 'a1'",
            [],
        )
        .unwrap();
        migrate_v41_account_track_opens_enabled(&conn).unwrap();
        let a1: i32 = conn
            .query_row(
                "SELECT track_opens_enabled FROM accounts WHERE id = 'a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(a1, 0, "re-run preserves the user's per-account choice");
    }

    #[test]
    fn v41_seeds_zero_when_tracking_config_row_missing() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL);
             CREATE TABLE tracking_config (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 is_enabled INTEGER NOT NULL DEFAULT 0,
                 service_url TEXT, api_key TEXT, last_synced TEXT);
             INSERT INTO accounts (id, email) VALUES ('a1', 'one@example.com');",
        )
        .unwrap();

        migrate_v41_account_track_opens_enabled(&conn).unwrap();
        let a1: i32 = conn
            .query_row(
                "SELECT track_opens_enabled FROM accounts WHERE id = 'a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            a1, 0,
            "missing tracking_config row falls back to 0 via COALESCE"
        );
    }

    #[test]
    fn v39_adds_bcc_json_column_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message_bodies (
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                to_json TEXT, cc_json TEXT, UNIQUE(account_id, folder_name, uid));",
        )
        .unwrap();
        assert!(!table_has_column(&conn, "message_bodies", "bcc_json").unwrap());

        migrate_v39_bcc_json(&conn).unwrap();
        assert!(table_has_column(&conn, "message_bodies", "bcc_json").unwrap());
        // Sibling columns survive.
        assert!(table_has_column(&conn, "message_bodies", "to_json").unwrap());
        assert!(table_has_column(&conn, "message_bodies", "cc_json").unwrap());

        // Idempotent: re-running on a table that already has the column is a no-op.
        migrate_v39_bcc_json(&conn).unwrap();
        assert!(table_has_column(&conn, "message_bodies", "bcc_json").unwrap());
    }

    #[test]
    fn v37_drops_thread_count_column_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, uid INTEGER NOT NULL, date TEXT NOT NULL DEFAULT '',
                thread_count INTEGER NOT NULL DEFAULT 0, thread_root_id TEXT,
                UNIQUE(account_id, folder_name, uid));",
        )
        .unwrap();
        assert!(table_has_column(&conn, "messages", "thread_count").unwrap());

        migrate_v37_drop_thread_count(&conn).unwrap();
        assert!(!table_has_column(&conn, "messages", "thread_count").unwrap());
        // thread_root_id (the canonical column) survives.
        assert!(table_has_column(&conn, "messages", "thread_root_id").unwrap());

        // Idempotent: running again on a column-less table is a no-op.
        migrate_v37_drop_thread_count(&conn).unwrap();
        assert!(!table_has_column(&conn, "messages", "thread_count").unwrap());
    }

    #[test]
    fn v38_backfills_detected_meeting_invites_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, uid INTEGER NOT NULL, subject TEXT,
                from_email TEXT, date TEXT NOT NULL DEFAULT '',
                UNIQUE(account_id, folder_name, uid));
             CREATE TABLE message_bodies (
                account_id TEXT NOT NULL, folder_name TEXT NOT NULL, uid INTEGER NOT NULL,
                plain_text TEXT, UNIQUE(account_id, folder_name, uid));
             CREATE TABLE calendar_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, message_uid INTEGER NOT NULL, event_uid TEXT,
                summary TEXT, description TEXT, location TEXT, dtstart TEXT NOT NULL, dtend TEXT,
                organizer_name TEXT, organizer_email TEXT, status TEXT, method TEXT,
                rsvp_status TEXT DEFAULT 'needs-action', raw_ics TEXT, source TEXT NOT NULL DEFAULT 'ics',
                confidence REAL, dismissed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();

        let body = "Sam Smith is inviting you to a scheduled Zoom meeting.\n\n\
Topic: Northwind Company Project Discussion\n\
Time: May 29, 2026 09:00 AM Central Time (US and Canada)\n\n\
Join Zoom Meeting\n\
https://us02web.zoom.us/j/87654321098?pwd=abcdEFGH\n";
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject, from_email, date)
             VALUES ('6ed99f49', 'INBOX', 225, 'Northwind Company Project Discussion',
                     'sam@harborline.example', '2026-05-28T20:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text)
             VALUES ('6ed99f49', 'INBOX', 225, ?1)",
            [body],
        )
        .unwrap();

        migrate_v38_backfill_detected_events(&conn).unwrap();

        let (count, dtstart, description, source): (i64, String, Option<String>, String) = conn
            .query_row(
                "SELECT count(*), max(dtstart), max(description), max(source)
                 FROM calendar_events WHERE message_uid = 225",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one detected event");
        assert_eq!(dtstart, "2026-05-29T14:00:00Z", "Central → UTC with DST");
        assert_eq!(
            description.as_deref(),
            Some("https://us02web.zoom.us/j/87654321098?pwd=abcdEFGH")
        );
        assert_eq!(source, "detected");

        // Idempotent: a second run inserts no duplicate.
        migrate_v38_backfill_detected_events(&conn).unwrap();
        let count2: i64 = conn
            .query_row(
                "SELECT count(*) FROM calendar_events WHERE message_uid = 225",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count2, 1, "no duplicate on re-run");
    }

    #[test]
    fn set_schema_version_keeps_a_single_row_and_collapses_bloat() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL);")
            .unwrap();
        // Simulate bloat from the old INSERT OR REPLACE behavior (BUG-05).
        for v in [30, 31, 32, 33, 34] {
            conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [v])
                .unwrap();
        }

        set_schema_version(&conn, 36).unwrap();

        let count: i64 = conn
            .query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "table should collapse to a single row");
        assert_eq!(get_schema_version(&conn).unwrap(), 36);

        // Repeated calls stay at one row.
        set_schema_version(&conn, 37).unwrap();
        let count2: i64 = conn
            .query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count2, 1);
        assert_eq!(get_schema_version(&conn).unwrap(), 37);
    }

    #[test]
    fn v35_salvages_malformed_dates_and_sentinels_unrecoverable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                date TEXT NOT NULL DEFAULT '',
                UNIQUE(account_id, folder_name, uid)
            );",
        )
        .unwrap();
        let rows: [(i64, &str); 5] = [
            (1, "2026-05-28T10:00:00+00:00"), // already ISO -> unchanged
            (2, "Wed, 25 Mar 2026 10:05:36 +0000.1731-577592"), // salvage
            (3, "Thu, 02 Apr 2026 00:30:54 +0200 . 714661948"), // salvage (non-UTC offset)
            (4, "_smtpDate . 714661948"),     // unrecoverable -> sentinel
            (5, ""),                          // empty -> unchanged
        ];
        for (uid, d) in rows {
            conn.execute(
                "INSERT INTO messages (account_id,folder_name,uid,date) VALUES ('a','INBOX',?1,?2)",
                rusqlite::params![uid, d],
            )
            .unwrap();
        }

        migrate_v35_salvage_malformed_dates(&conn).unwrap();

        let get = |uid: i64| -> String {
            conn.query_row("SELECT date FROM messages WHERE uid=?1", [uid], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(get(1), "2026-05-28T10:00:00+00:00");
        assert_eq!(get(2), "2026-03-25T10:05:36+00:00");
        assert_eq!(get(3), "2026-04-01T22:30:54+00:00");
        assert_eq!(get(4), "1970-01-01T00:00:00+00:00");
        assert_eq!(get(5), "");

        // The garbage rows no longer sort above real ISO dates.
        let top: String = conn
            .query_row(
                "SELECT date FROM messages ORDER BY date DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(top, "2026-05-28T10:00:00+00:00");

        // Idempotent.
        migrate_v35_salvage_malformed_dates(&conn).unwrap();
        assert_eq!(get(2), "2026-03-25T10:05:36+00:00");
    }

    #[test]
    fn v34_backfill_fills_empty_to_list_without_clobbering() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, uid INTEGER NOT NULL, date TEXT NOT NULL DEFAULT '',
                to_list TEXT, cc_list TEXT, UNIQUE(account_id, folder_name, uid));
             CREATE TABLE message_bodies (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL, uid INTEGER NOT NULL, to_json TEXT, cc_json TEXT,
                UNIQUE(account_id, folder_name, uid));",
        )
        .unwrap();
        // uid 1: to_list NULL, body has to_json -> filled (the BUG-02 case).
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,to_list) VALUES ('a','INBOX',1,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_bodies (account_id,folder_name,uid,to_json,cc_json) \
             VALUES ('a','INBOX',1,'[{\"email\":\"x@y.com\"}]','[]')",
            [],
        )
        .unwrap();
        // uid 2: to_list already populated -> must NOT be clobbered.
        conn.execute(
            "INSERT INTO messages (account_id,folder_name,uid,to_list) \
             VALUES ('a','INBOX',2,'[{\"email\":\"keep@me.com\"}]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_bodies (account_id,folder_name,uid,to_json) \
             VALUES ('a','INBOX',2,'[{\"email\":\"other@z.com\"}]')",
            [],
        )
        .unwrap();

        migrate_v34_backfill_to_list(&conn).unwrap();

        let t1: Option<String> = conn
            .query_row("SELECT to_list FROM messages WHERE uid=1", [], |r| r.get(0))
            .unwrap();
        let t2: Option<String> = conn
            .query_row("SELECT to_list FROM messages WHERE uid=2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(t1.as_deref(), Some(r#"[{"email":"x@y.com"}]"#));
        assert_eq!(t2.as_deref(), Some(r#"[{"email":"keep@me.com"}]"#));
    }

    #[test]
    fn v22_migration_is_idempotent_after_partial_apply() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE sync_state (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                last_uid INTEGER DEFAULT 0,
                uidvalidity INTEGER,
                last_synced TEXT,
                UNIQUE(account_id, folder_name)
            );
            ",
        )
        .unwrap();

        conn.execute(
            "ALTER TABLE sync_state ADD COLUMN last_flags_reconciled_at TEXT;",
            [],
        )
        .unwrap();

        migrate_v22_sync_state_reconciliation_columns(&conn).unwrap();
        migrate_v22_sync_state_reconciliation_columns(&conn).unwrap();

        assert!(table_has_column(&conn, "sync_state", "last_flags_reconciled_at").unwrap());
        assert!(table_has_column(&conn, "sync_state", "last_existence_reconciled_at").unwrap());
    }

    #[test]
    fn v32_unescapes_subjects_in_place_and_skips_clean_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE accounts (id TEXT PRIMARY KEY);
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_name TEXT NOT NULL,
                uid INTEGER NOT NULL,
                subject TEXT,
                date TEXT NOT NULL DEFAULT '',
                UNIQUE(account_id, folder_name, uid)
            );
            INSERT INTO accounts (id) VALUES ('a');
            ",
        )
        .unwrap();

        let escaped = r#"Shipped: \"Layla 300 Thread Count...\""#;
        let clean = r#"Shipped: "Already Clean""#;
        let null_subject: Option<&str> = None;
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject) VALUES ('a','INBOX',1,?1)",
            [escaped],
        ).unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject) VALUES ('a','INBOX',2,?1)",
            [clean],
        ).unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, subject) VALUES ('a','INBOX',3,?1)",
            rusqlite::params![null_subject],
        ).unwrap();

        migrate_v32_unescape_subjects(&conn).unwrap();

        let s1: String = conn
            .query_row("SELECT subject FROM messages WHERE uid=1", [], |r| r.get(0))
            .unwrap();
        let s2: String = conn
            .query_row("SELECT subject FROM messages WHERE uid=2", [], |r| r.get(0))
            .unwrap();
        let s3: Option<String> = conn
            .query_row("SELECT subject FROM messages WHERE uid=3", [], |r| r.get(0))
            .unwrap();

        assert_eq!(s1, r#"Shipped: "Layla 300 Thread Count...""#);
        assert_eq!(s2, clean);
        assert_eq!(s3, None);

        // Idempotent: running again is a no-op
        migrate_v32_unescape_subjects(&conn).unwrap();
        let s1b: String = conn
            .query_row("SELECT subject FROM messages WHERE uid=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(s1b, r#"Shipped: "Layla 300 Thread Count...""#);
    }

    #[test]
    fn v33_migration_adds_voice_tables_and_cascades_archetype_assignments() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            PRAGMA foreign_keys=ON;
            CREATE TABLE accounts (id TEXT PRIMARY KEY);
            CREATE TABLE ai_draft_edits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL
            );
            INSERT INTO accounts (id) VALUES ('acct');
            ",
        )
        .unwrap();

        migrate_v33_recipient_voice_and_archetypes(&conn).unwrap();
        migrate_v33_recipient_voice_and_archetypes(&conn).unwrap();

        assert!(table_has_column(&conn, "ai_draft_edits", "recipient_email").unwrap());
        assert!(table_has_column(&conn, "voice_profiles_recipient", "profile_json").unwrap());
        assert!(table_has_column(&conn, "voice_archetypes", "profile_json").unwrap());
        assert!(table_has_column(&conn, "message_archetype", "archetype_id").unwrap());

        conn.execute(
            "INSERT INTO voice_archetypes
             (account_id, archetype_id, name, description, profile_json, sample_count)
             VALUES ('acct', 'arch-1', 'Warm', '', '{}', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_archetype (account_id, folder_name, uid, archetype_id)
             VALUES ('acct', 'Sent', 1, 'arch-1')",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM voice_archetypes WHERE account_id = 'acct' AND archetype_id = 'arch-1'",
            [],
        )
        .unwrap();

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM message_archetype", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn v44_migration_adds_durable_recipient_voice_buckets() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE accounts (id TEXT PRIMARY KEY);
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
             INSERT INTO accounts (id) VALUES ('acct');",
        )
        .unwrap();

        migrate_v44_recipient_voice_buckets(&conn).unwrap();
        migrate_v44_recipient_voice_buckets(&conn).unwrap();
        assert!(table_has_column(&conn, "voice_profiles_recipient", "learning_version").unwrap());

        conn.execute(
            "INSERT INTO recipient_voice_buckets
                (account_id, recipient_email, bucket_id, centroid_json,
                 sample_count, first_message_date, last_message_date)
             VALUES ('acct', 'sam@example.com', 'structured', '[1.0,0.0]',
                     1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recipient_voice_sample_buckets
                (account_id, recipient_email, folder_name, uid, message_date,
                 bucket_id, similarity, feature_json)
             VALUES ('acct', 'sam@example.com', 'Sent', 1,
                     '2026-01-01T00:00:00Z', 'structured', 1.0, '[1.0,0.0]')",
            [],
        )
        .unwrap();

        conn.execute(
            "DELETE FROM recipient_voice_buckets
             WHERE account_id = 'acct' AND recipient_email = 'sam@example.com'
               AND bucket_id = 'structured'",
            [],
        )
        .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recipient_voice_sample_buckets",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    /// v50 must be a no-op for the three providers that predate it: every
    /// default has to describe what `connect_gmail` / `connect_icloud` /
    /// `connect_outlook` already hardcoded, or an existing account silently
    /// changes handshake on the next launch. Re-running must also be safe —
    /// an old binary re-stamps `schema_version` downward (see gotcha #26), so
    /// this migration WILL run more than once on a real machine.
    #[test]
    fn migrate_v50_is_idempotent_and_defaults_match_the_hardcoded_providers() {
        let conn = Connection::open_in_memory().unwrap();
        // A v49-shaped accounts row: no security/username columns yet.
        conn.execute_batch(
            "CREATE TABLE accounts (
                 id TEXT PRIMARY KEY,
                 email TEXT NOT NULL UNIQUE,
                 provider TEXT NOT NULL,
                 imap_host TEXT NOT NULL,
                 imap_port INTEGER NOT NULL DEFAULT 993,
                 smtp_host TEXT NOT NULL,
                 smtp_port INTEGER NOT NULL DEFAULT 587
             );
             CREATE TABLE folders (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account_id TEXT NOT NULL,
                 name TEXT NOT NULL,
                 folder_type TEXT
             );
             INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('a1', 'chris@gmail.com', 'gmail', 'imap.gmail.com', 'smtp.gmail.com');
             INSERT INTO folders (account_id, name, folder_type)
             VALUES ('a1', 'INBOX', 'inbox');",
        )
        .unwrap();

        migrate_v50_generic_imap(&conn).unwrap();
        migrate_v50_generic_imap(&conn).unwrap();

        let (imap_sec, smtp_sec, imap_user, smtp_user) = conn
            .query_row(
                "SELECT imap_security, smtp_security, imap_username, smtp_username
                 FROM accounts WHERE id = 'a1'",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .unwrap();
        // 993 implicit + 587 STARTTLS + authenticate-as-email is exactly what
        // the pre-v50 code did unconditionally.
        assert_eq!(imap_sec, "implicit");
        assert_eq!(smtp_sec, "starttls");
        assert_eq!(imap_user, None);
        assert_eq!(smtp_user, None);

        // Pre-existing rows are otherwise untouched.
        let host: String = conn
            .query_row("SELECT imap_host FROM accounts WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(host, "imap.gmail.com");

        // Folders classified before v50 were all name-guessed, so 0 is the
        // truthful default — it must not claim the server declared them.
        let special: i64 = conn
            .query_row("SELECT special_use FROM folders WHERE name = 'INBOX'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(special, 0);
    }
}
