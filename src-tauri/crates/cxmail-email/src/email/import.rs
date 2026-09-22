use crate::db;
use crate::email::parser;
use crate::error::AppError;
use rusqlite::Connection;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Import messages from a Thunderbird mbox file into the local database.
pub fn import_mbox(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    mbox_path: &Path,
) -> Result<u32, AppError> {
    let file = std::fs::File::open(mbox_path)?;
    let reader = BufReader::new(file);

    let mut count = 0u32;
    let mut current_message: Vec<u8> = Vec::new();
    let mut in_message = false;
    let mut uid_counter = 1u32;

    // Get highest existing UID to avoid conflicts
    let max_uid: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE account_id = ?1 AND folder_name = ?2",
            rusqlite::params![account_id, folder_name],
            |row| row.get(0),
        )
        .unwrap_or(0);
    uid_counter = max_uid + 1;

    for line in reader.lines() {
        let line = line.map_err(|e| AppError::General(format!("Read error: {}", e)))?;

        if line.starts_with("From ") {
            // New message boundary
            if in_message && !current_message.is_empty() {
                import_single_message(
                    conn,
                    account_id,
                    folder_name,
                    uid_counter,
                    &current_message,
                )?;
                count += 1;
                uid_counter += 1;
            }
            current_message.clear();
            in_message = true;
        } else if in_message {
            current_message.extend_from_slice(line.as_bytes());
            current_message.push(b'\n');
        }
    }

    // Don't forget the last message
    if in_message && !current_message.is_empty() {
        import_single_message(conn, account_id, folder_name, uid_counter, &current_message)?;
        count += 1;
    }

    log::info!("Imported {} messages from {:?}", count, mbox_path);
    Ok(count)
}

fn import_single_message(
    conn: &Connection,
    account_id: &str,
    folder_name: &str,
    uid: u32,
    raw: &[u8],
) -> Result<(), AppError> {
    let parsed = parser::parse_message(raw);

    // mail-parser strips angle brackets from Message-IDs and yields References
    // as a Vec<String>. Re-bracket and join with whitespace so the column
    // shape matches what the IMAP sync path stores (raw RFC 5322 References).
    // This keeps `compute_thread_root_id` and the legacy
    // `reference_ids LIKE '%<id>%'` matchers working for imported messages.
    //
    // `message_id` / `in_reply_to` get the same treatment (they used to go in
    // bare) so imported mail is repliable-to by exact match — gotcha #30.
    use crate::email::message_id::{normalize_message_id, normalize_reference_chain};
    let references_joined: Option<String> = if parsed.references.is_empty() {
        None
    } else {
        Some(normalize_reference_chain(&parsed.references.join(" ")))
            .filter(|chain| !chain.is_empty())
    };
    let message_id_norm = parsed.message_id.as_deref().and_then(normalize_message_id);
    let in_reply_to_norm = parsed.in_reply_to.as_deref().and_then(normalize_message_id);

    let batch = vec![(
        uid,
        parsed.subject.as_deref(),
        parsed.from_name.as_deref(),
        Some(parsed.from_email.as_str()),
        parsed.date.as_deref().unwrap_or("1970-01-01T00:00:00Z"),
        parsed.snippet.as_deref(),
        "[]",
        message_id_norm.as_deref(),
        in_reply_to_norm.as_deref(),
        references_joined.as_deref(),
        false,
        false,
        !parsed.attachments.is_empty(),
        parsed.size_bytes as i64,
        None::<&str>,
        None::<&str>,
        "[]",
        "[]",
    )];

    db::messages::insert_batch(conn, account_id, folder_name, &batch)?;

    // Also store the body
    db::messages::insert_body(
        conn,
        account_id,
        folder_name,
        uid,
        parsed.plain_text.as_deref(),
        parsed.html_body.as_deref(),
        parsed.sanitized_html.as_deref(),
        true,
    )?;
    db::messages::insert_attachments(conn, account_id, folder_name, uid, &parsed.attachments)?;

    Ok(())
}
