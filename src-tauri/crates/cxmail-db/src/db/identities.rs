use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: Option<i64>,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub signature_html: Option<String>,
    pub is_default: bool,
}

pub fn insert(conn: &Connection, identity: &Identity) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO identities (account_id, email, display_name, signature_html, is_default) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![identity.account_id, identity.email, identity.display_name, identity.signature_html, identity.is_default as i32],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_by_account(conn: &Connection, account_id: &str) -> Result<Vec<Identity>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email, display_name, signature_html, is_default FROM identities WHERE account_id = ?1 ORDER BY is_default DESC",
    )?;
    let identities = stmt
        .query_map(params![account_id], |row| {
            Ok(Identity {
                id: row.get(0)?,
                account_id: row.get(1)?,
                email: row.get(2)?,
                display_name: row.get(3)?,
                signature_html: row.get(4)?,
                is_default: row.get::<_, i32>(5)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(identities)
}

pub fn update(conn: &Connection, identity: &Identity) -> Result<(), AppError> {
    conn.execute(
        "UPDATE identities SET email = ?2, display_name = ?3, signature_html = ?4, is_default = ?5 WHERE id = ?1",
        params![identity.id, identity.email, identity.display_name, identity.signature_html, identity.is_default as i32],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM identities WHERE id = ?1", params![id])?;
    Ok(())
}

// ─── Send-as addresses ────────────────────────────────────────────────────
//
// An account can legitimately send from more than one address: iCloud and
// Gmail both let a mailbox own alias addresses, and mail that arrived at an
// alias should be answered FROM that alias. Neither IMAP nor SMTP can
// enumerate them — IMAP is a mailbox-access protocol with no identity
// extension. (Gmail's `users.settings.sendAs.list` does accept the plain
// `https://mail.google.com/` grant CXMail holds, but it covers Gmail alone and
// is not wired; iCloud has no public API at all.) So an alias comes from one of
// two places, and this module is where "which addresses may this account send
// as" is answered:
//
//  * **configured** — one `identities` row per alias, entered by the user;
//  * **found in Sent** — any `From:` on the account's own Sent folder. That is
//    proof, not a guess: the provider's submission server accepted it (Gmail
//    even rewrites a From it will not send as), and nobody but the account can
//    put mail there. Derived at read time so it never goes stale, and
//    suppressed per address by a `send_as_hidden` tombstone (v64) when the user
//    removes it.
//
// Incoming-mail evidence (`Delivered-To`, iCloud's `Original-recipient`) is
// NOT proof — a sender can write any header — so it only ever SUGGESTS
// (`suggest_aliases`); a person confirms.

/// One address an account may put in a `From:` header.
///
/// The account's own address is always present as the primary — it is the
/// address SMTP authenticates as, and the fallback whenever nothing else
/// resolves. Every `identities` row for the account is an additional entry,
/// except a row that merely restates the primary address: that one is folded
/// INTO the primary, contributing its display name and signature. That fold is
/// what keeps the pre-alias world working, where the single `identities` row
/// existed only to hold the account's signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendAsAddress {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub signature_html: Option<String>,
    /// True for the account's own address. SMTP always authenticates as the
    /// account; a non-primary `From` is a send-as, which the provider may
    /// refuse if the alias is not registered on its side.
    pub is_primary: bool,
    /// `identities.id`, or `None` for an implicit primary with no row.
    pub identity_id: Option<i64>,
    /// True when the address was found on this account's own Sent mail rather
    /// than configured. `identity_id` is `None` for these.
    #[serde(default)]
    pub from_sent: bool,
}

/// Lowercase + trim — the comparison form for an email address, matching the
/// `norm()` convention the frontend already uses for reply-all self-filtering.
/// Real mail needs it: 109 messages in the live mailbox are addressed to the
/// same alias in two different casings.
pub fn normalize_addr(addr: &str) -> String {
    addr.trim().to_ascii_lowercase()
}

/// Every address `account_id` may send as, primary first, deduped
/// case-insensitively on the address.
///
/// `account_email` / `account_display_name` come from the `accounts` row; they
/// are passed rather than re-queried so callers that already hold the account
/// don't pay for a second lookup.
pub fn send_as_addresses(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    account_display_name: Option<&str>,
) -> Result<Vec<SendAsAddress>, AppError> {
    let rows = list_by_account(conn, account_id)?;
    let mut out = build_send_as(account_id, account_email, account_display_name, &rows);
    let hidden = hidden_send_as(conn, account_id);
    let sent = sent_from_addresses(conn, account_id);
    append_sent_detected(&mut out, &sent, &hidden);
    Ok(out)
}

/// The pure half of the Sent-folder detection: append each address found on
/// Sent mail that is neither already listed nor hidden. Configured rows win —
/// they carry the user's display name and signature.
pub fn append_sent_detected(out: &mut Vec<SendAsAddress>, sent: &[String], hidden: &[String]) {
    let Some(account_id) = out.first().map(|a| a.account_id.clone()) else {
        return;
    };
    for addr in sent {
        let key = normalize_addr(addr);
        if key.is_empty() || !key.contains('@') {
            continue;
        }
        if hidden.contains(&key) || out.iter().any(|a| normalize_addr(&a.email) == key) {
            continue;
        }
        out.push(SendAsAddress {
            account_id: account_id.clone(),
            email: addr.trim().to_string(),
            display_name: None,
            signature_html: None,
            is_primary: false,
            identity_id: None,
            from_sent: true,
        });
    }
}

/// Distinct `From:` addresses on this account's Sent folder, most-used first,
/// in the spelling first seen. Empty when the folder list has not synced — the
/// account then simply lists what is configured.
///
/// The Sent folder and nothing else: a Drafts row was never submitted, and a
/// Gmail All Mail row is a mirror that also holds everyone else's mail.
pub fn sent_from_addresses(conn: &Connection, account_id: &str) -> Vec<String> {
    let Some(sent) = crate::db::folders::sent_folder_for_account(conn, account_id) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT MIN(trim(from_email)) FROM messages
         WHERE account_id = ?1 AND folder_name = ?2
           AND from_email IS NOT NULL AND trim(from_email) != ''
         GROUP BY lower(trim(from_email))
         ORDER BY COUNT(*) DESC, 1",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![account_id, sent], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Normalized addresses the user removed for this account. Tolerates a
/// pre-v64 database (the MCP migrates non-fatally): no table, nothing hidden.
pub fn hidden_send_as(conn: &Connection, account_id: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare("SELECT email FROM send_as_hidden WHERE account_id = ?1")
    else {
        return Vec::new();
    };
    stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Remove an alias from the account's send-as list: delete its configured row
/// (if any) AND tombstone the address, so one that is also on Sent mail does
/// not come straight back as "found in Sent". Removing the primary is refused.
pub fn remove_send_as(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    email: &str,
) -> Result<(), AppError> {
    let key = normalize_addr(email);
    if key.is_empty() {
        return Err(AppError::General("No address given.".to_string()));
    }
    if key == normalize_addr(account_email) {
        return Err(AppError::General(
            "The account's own address cannot be removed.".to_string(),
        ));
    }
    conn.execute(
        "DELETE FROM identities WHERE account_id = ?1 AND lower(trim(email)) = ?2",
        params![account_id, key],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO send_as_hidden (account_id, email) VALUES (?1, ?2)",
        params![account_id, key],
    )?;
    Ok(())
}

/// Adding an address the user once removed brings it back: the tombstone only
/// ever stood for "I don't want this one".
pub fn unhide_send_as(conn: &Connection, account_id: &str, email: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM send_as_hidden WHERE account_id = ?1 AND email = ?2",
        params![account_id, normalize_addr(email)],
    )?;
    Ok(())
}

/// The pure half of [`send_as_addresses`] — no DB, so the folding and dedupe
/// rules are testable on their own.
pub fn build_send_as(
    account_id: &str,
    account_email: &str,
    account_display_name: Option<&str>,
    identities: &[Identity],
) -> Vec<SendAsAddress> {
    let primary_key = normalize_addr(account_email);
    let mut primary = SendAsAddress {
        account_id: account_id.to_string(),
        email: account_email.to_string(),
        display_name: account_display_name.map(str::to_string),
        signature_html: None,
        is_primary: true,
        identity_id: None,
        from_sent: false,
    };
    let mut aliases: Vec<SendAsAddress> = Vec::new();
    let mut seen: Vec<String> = vec![primary_key.clone()];

    for id in identities {
        let key = normalize_addr(&id.email);
        if key.is_empty() {
            continue;
        }
        if key == primary_key {
            // A row restating the account's own address is the primary's
            // settings, not a second address.
            if id.display_name.is_some() {
                primary.display_name = id.display_name.clone();
            }
            if id.signature_html.is_some() {
                primary.signature_html = id.signature_html.clone();
            }
            primary.identity_id = id.id;
            continue;
        }
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        aliases.push(SendAsAddress {
            account_id: account_id.to_string(),
            email: id.email.trim().to_string(),
            display_name: id.display_name.clone(),
            signature_html: id.signature_html.clone(),
            is_primary: false,
            identity_id: id.id,
            from_sent: false,
        });
    }

    let mut out = Vec::with_capacity(aliases.len() + 1);
    out.push(primary);
    out.extend(aliases);
    out
}

/// Pick the address to reply FROM, given the recipients the original message
/// carried. `recipients` is walked in order, so the caller decides precedence —
/// [`recipients_for_send_as_match`] hands over To, then Cc, then the
/// delivery headers.
///
/// First match wins on the RECIPIENT, not on the address list: a message
/// addressed to both the primary and an alias was addressed to the primary
/// too, and answering from it is never surprising. `None` means nothing
/// matched, and the caller falls back to the primary.
pub fn match_send_as<'a>(
    addresses: &'a [SendAsAddress],
    recipients: &[String],
) -> Option<&'a SendAsAddress> {
    for r in recipients {
        let key = normalize_addr(r);
        if key.is_empty() {
            continue;
        }
        if let Some(hit) = addresses.iter().find(|a| normalize_addr(&a.email) == key) {
            return Some(hit);
        }
    }
    None
}

/// Resolve a requested `From` address against what the account may send as.
///
/// `None` / blank means "the account's own address" — the pre-alias behaviour
/// every existing caller relies on. A requested address that is not a
/// configured send-as is REFUSED rather than silently rewritten: a From header
/// the user did not configure is either a mistake or a spoof, and both deserve
/// to be visible. Matching is case-insensitive but the CONFIGURED spelling is
/// what gets returned, so the header carries the address as the user entered
/// it.
pub fn resolve_send_as(
    addresses: &[SendAsAddress],
    requested: Option<&str>,
) -> Result<SendAsAddress, AppError> {
    let primary = addresses
        .iter()
        .find(|a| a.is_primary)
        .or_else(|| addresses.first())
        .ok_or_else(|| AppError::NotFound("Account has no send-as address".to_string()))?;
    let requested = requested.map(str::trim).filter(|r| !r.is_empty());
    let Some(requested) = requested else {
        return Ok(primary.clone());
    };
    let key = normalize_addr(requested);
    addresses
        .iter()
        .find(|a| normalize_addr(&a.email) == key)
        .cloned()
        .ok_or_else(|| {
            AppError::General(format!(
                "{} is not a configured send-as address for {}. Configured: {}. \
                 Add it under the account's Send-as addresses first.",
                requested,
                primary.email,
                addresses
                    .iter()
                    .map(|a| a.email.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

/// [`send_as_addresses`] + [`resolve_send_as`] for an account row — the one
/// call every WRITE path makes before it puts an address in a `From:` header.
/// Keeping it here rather than at each call site is gotcha #36's rule: the UI
/// draft save, the UI send, the scheduled-send worker and the two MCP draft
/// handlers must agree about what this account may send as.
pub fn resolve_for_account(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    account_display_name: Option<&str>,
    requested: Option<&str>,
) -> Result<SendAsAddress, AppError> {
    let addresses = send_as_addresses(conn, account_id, account_email, account_display_name)?;
    resolve_send_as(&addresses, requested)
}

/// The address to reply FROM for one stored message: the send-as the original
/// was addressed to, or the primary when it was addressed to none of them.
///
/// Reads the cached row only — no IMAP (see
/// [`crate::db::messages::recipients_for_send_as_match`]).
pub fn reply_from_for_message(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    account_display_name: Option<&str>,
    folder_name: &str,
    uid: u32,
) -> Result<SendAsAddress, AppError> {
    let addresses = send_as_addresses(conn, account_id, account_email, account_display_name)?;
    let recipients =
        crate::db::messages::recipients_for_send_as_match(conn, account_id, folder_name, uid)?;
    let hit = match_send_as(&addresses, &recipients).cloned();
    match hit {
        Some(a) => Ok(a),
        None => resolve_send_as(&addresses, None),
    }
}

/// Addresses this account's mail was DELIVERED to that it cannot yet send as —
/// candidate aliases, most-delivered first.
///
/// The evidence is the delivery headers the receiving server writes
/// (`Delivered-To`, `X-Original-To`, iCloud's `Original-recipient`), from the
/// `messages.delivered_to` column sync fills (v64) and from any full header
/// block cached in `message_headers`. NOT the `To:`/`Cc:` lists: those name
/// every co-recipient, and on a shared domain (`gmail.com`, `icloud.com`) a
/// same-domain filter offered strangers as your aliases.
///
/// A suggestion is never applied on its own: a sender can write any header, so
/// only a person may turn one into a send-as. Addresses the user removed are
/// not offered again.
pub fn suggest_aliases(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    limit: usize,
) -> Result<Vec<(String, u32)>, AppError> {
    let mut skip: Vec<String> = send_as_addresses(conn, account_id, account_email, None)?
        .iter()
        .map(|a| normalize_addr(&a.email))
        .collect();
    skip.extend(hidden_send_as(conn, account_id));

    // (normalized key, first-seen spelling, count)
    let mut tally: Vec<(String, String, u32)> = Vec::new();
    let mut count = |addr: String| {
        let key = normalize_addr(&addr);
        if key.is_empty() || !key.contains('@') || skip.contains(&key) {
            return;
        }
        // `me+tag@domain` is a sub-address of an address already listed, not
        // a second one — on the live mailbox, dozens of `cxrobx+aa-e2e-…`
        // test signups would otherwise crowd out the real candidates.
        if skip.contains(&strip_plus_tag(&key)) {
            return;
        }
        match tally.iter_mut().find(|(k, _, _)| *k == key) {
            Some(entry) => entry.2 += 1,
            None => tally.push((key, addr, 1)),
        }
    };

    // One count per message: a message carries the same address in both the
    // column and its cached header block, and must not vote twice.
    let mut stmt = conn.prepare(
        "SELECT m.delivered_to, h.raw_headers FROM messages m
         LEFT JOIN message_headers h
           ON h.account_id = m.account_id AND h.folder_name = m.folder_name AND h.uid = m.uid
         WHERE m.account_id = ?1
           AND (m.delivered_to IS NOT NULL OR h.raw_headers IS NOT NULL)
         ORDER BY m.date DESC LIMIT 5000",
    )?;
    let rows = stmt.query_map(params![account_id], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (column, block) = row?;
        let mut seen: Vec<String> = Vec::new();
        let mut per_message = column.as_deref().map(extract_addresses).unwrap_or_default();
        if let Some(block) = block.as_deref() {
            per_message.extend(delivery_header_addresses(block));
        }
        for addr in per_message {
            let key = normalize_addr(&addr);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            count(addr);
        }
    }
    tally.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    Ok(tally
        .into_iter()
        .take(limit)
        .map(|(_, spelling, count)| (spelling, count))
        .collect())
}

/// `local+tag@domain` → `local@domain`; anything else unchanged.
fn strip_plus_tag(addr: &str) -> String {
    match addr.split_once('@') {
        Some((local, domain)) => match local.split_once('+') {
            Some((base, _)) if !base.is_empty() => format!("{base}@{domain}"),
            _ => addr.to_string(),
        },
        None => addr.to_string(),
    }
}

/// Addresses on the delivery headers of a raw header block, in the order the
/// headers appear. `Delivered-To` (Gmail, Postfix), `X-Original-To` (Postfix)
/// and `Envelope-To` (Exim) each name the address the message was actually
/// delivered to, which is the only signal left when a mailing list or a
/// forwarder rewrote `To`.
///
/// Folding-aware, per gotcha #37: a continuation line belongs to the header
/// above it.
pub fn delivery_header_addresses(raw_headers: &str) -> Vec<String> {
    // `original-recipient` is iCloud's (RFC 3798's `rfc822;addr` form): an
    // iCloud message delivered on an alias carries the alias there even when
    // its `To:` names a list or someone else entirely.
    const WANTED: [&str; 4] = ["delivered-to", "x-original-to", "envelope-to", "original-recipient"];
    let mut out: Vec<String> = Vec::new();
    let mut keeping = false;
    let mut current = String::new();
    let flush = |buf: &mut String, out: &mut Vec<String>| {
        for addr in extract_addresses(buf) {
            if !out.iter().any(|e| normalize_addr(e) == normalize_addr(&addr)) {
                out.push(addr);
            }
        }
        buf.clear();
    };
    for line in raw_headers.lines() {
        let is_continuation = line.starts_with(' ') || line.starts_with('\t');
        if is_continuation {
            if keeping {
                current.push(' ');
                current.push_str(line.trim());
            }
            continue;
        }
        if keeping {
            flush(&mut current, &mut out);
        }
        keeping = match line.split_once(':') {
            Some((name, value)) => {
                let hit = WANTED.contains(&name.trim().to_ascii_lowercase().as_str());
                if hit {
                    current.push_str(value.trim());
                }
                hit
            }
            None => false,
        };
    }
    if keeping {
        flush(&mut current, &mut out);
    }
    out
}

/// Pull bare addresses out of a header value. Handles `a@b`, `<a@b>` and
/// `Name <a@b>`, comma-separated.
pub(crate) fn extract_addresses(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let addr = match (part.rfind('<'), part.rfind('>')) {
            (Some(lt), Some(gt)) if gt > lt => &part[lt + 1..gt],
            _ => part,
        };
        // `rfc822;addr` — the address-type prefix `Original-recipient` carries.
        let addr = addr.rsplit(';').next().unwrap_or(addr).trim();
        if addr.contains('@') && !addr.contains(' ') {
            out.push(addr.to_string());
        }
    }
    out
}

#[cfg(test)]
mod send_as_tests {
    use super::*;

    fn ident(id: i64, email: &str, name: Option<&str>, sig: Option<&str>) -> Identity {
        Identity {
            id: Some(id),
            account_id: "acct".to_string(),
            email: email.to_string(),
            display_name: name.map(str::to_string),
            signature_html: sig.map(str::to_string),
            is_default: false,
        }
    }

    fn icloud() -> Vec<SendAsAddress> {
        build_send_as(
            "acct",
            "sidalias@icloud.com",
            Some("iCloud"),
            &[ident(1, "rileyprime@icloud.com", Some("Christopher"), None)],
        )
    }

    #[test]
    fn the_account_address_is_always_first_and_primary() {
        let list = icloud();
        assert_eq!(list.len(), 2);
        assert!(list[0].is_primary);
        assert_eq!(list[0].email, "sidalias@icloud.com");
        assert!(!list[1].is_primary);
        assert_eq!(list[1].email, "rileyprime@icloud.com");
    }

    /// The pre-alias world: the ONE `identities` row an account had existed to
    /// hold its signature and restated the account's own address. Folding it
    /// into the primary — rather than listing it as a second address — is what
    /// keeps that account showing one From entry, with its signature.
    #[test]
    fn an_identity_restating_the_account_address_folds_into_the_primary() {
        let list = build_send_as(
            "acct",
            "chris@cxventures.io",
            Some("CX Ventures"),
            &[ident(
                7,
                "chris@cxventures.io",
                Some("Christopher Robinson"),
                Some("<p>sig</p>"),
            )],
        );
        assert_eq!(list.len(), 1, "not a second address: {list:?}");
        assert!(list[0].is_primary);
        assert_eq!(list[0].display_name.as_deref(), Some("Christopher Robinson"));
        assert_eq!(list[0].signature_html.as_deref(), Some("<p>sig</p>"));
        assert_eq!(list[0].identity_id, Some(7));
    }

    #[test]
    fn folding_is_case_insensitive_and_duplicate_aliases_collapse() {
        let list = build_send_as(
            "acct",
            "sidalias@icloud.com",
            None,
            &[
                ident(1, "SidAlias@iCloud.com", None, Some("<p>s</p>")),
                ident(2, "Alias@icloud.com", None, None),
                ident(3, "alias@ICLOUD.com", None, None),
            ],
        );
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list[0].signature_html.as_deref(), Some("<p>s</p>"));
        assert_eq!(list[1].email, "Alias@icloud.com");
    }

    /// The whole point of the feature: a reply to mail that arrived at the
    /// alias goes out FROM the alias.
    #[test]
    fn a_reply_defaults_to_the_alias_the_original_was_addressed_to() {
        let list = icloud();
        let recipients = vec!["rileyprime@icloud.com".to_string()];
        let hit = match_send_as(&list, &recipients).expect("alias must match");
        assert_eq!(hit.email, "rileyprime@icloud.com");
        assert!(!hit.is_primary);
    }

    /// Real mail addresses the same alias in more than one casing — 109
    /// messages in the live mailbox, spelled both `rileyprime@` and
    /// `RileyPrime@`. An exact compare answers "primary" for half of
    /// them.
    #[test]
    fn the_match_is_case_insensitive_and_returns_the_configured_spelling() {
        let list = icloud();
        let hit = match_send_as(&list, &["RileyPrime@iCloud.com".to_string()])
            .expect("case must not decide this");
        assert_eq!(hit.email, "rileyprime@icloud.com");
    }

    #[test]
    fn mail_to_the_primary_stays_on_the_primary() {
        let list = icloud();
        let hit = match_send_as(&list, &["sidalias@icloud.com".to_string()]).unwrap();
        assert!(hit.is_primary);
    }

    /// Precedence is the RECIPIENT order the caller hands over (To, then Cc,
    /// then the delivery headers), not the order of the address list. A
    /// message addressed to you at both addresses was addressed to your
    /// primary, and replying from it surprises nobody.
    #[test]
    fn the_first_matching_recipient_wins_not_the_first_matching_alias() {
        let list = icloud();
        let recipients = vec![
            "sidalias@icloud.com".to_string(),
            "rileyprime@icloud.com".to_string(),
        ];
        assert!(match_send_as(&list, &recipients).unwrap().is_primary);

        let reversed = vec![
            "rileyprime@icloud.com".to_string(),
            "sidalias@icloud.com".to_string(),
        ];
        assert!(!match_send_as(&list, &reversed).unwrap().is_primary);
    }

    #[test]
    fn a_stranger_on_the_recipient_list_matches_nothing() {
        let list = icloud();
        assert!(match_send_as(&list, &["someone@example.com".to_string()]).is_none());
        assert!(match_send_as(&list, &[]).is_none());
    }

    #[test]
    fn an_alias_of_another_account_is_not_a_match() {
        let list = icloud();
        // The Gmail account's alias must not pull an iCloud reply onto it.
        assert!(match_send_as(&list, &["chris@cxventures.io".to_string()]).is_none());
    }

    #[test]
    fn resolve_defaults_to_the_primary_when_nothing_is_requested() {
        let list = icloud();
        assert!(resolve_send_as(&list, None).unwrap().is_primary);
        assert!(resolve_send_as(&list, Some("")).unwrap().is_primary);
        assert!(resolve_send_as(&list, Some("   ")).unwrap().is_primary);
    }

    #[test]
    fn resolve_accepts_a_configured_alias_in_any_casing_and_returns_the_configured_spelling() {
        let list = icloud();
        let got = resolve_send_as(&list, Some(" RileyPrime@ICLOUD.com ")).unwrap();
        assert_eq!(got.email, "rileyprime@icloud.com");
        assert!(!got.is_primary);
    }

    /// A From the user never configured is refused, not quietly rewritten to
    /// the primary: it is either a caller bug or a spoof, and both should be
    /// visible. The message has to name the remedy (#57's convention).
    #[test]
    fn resolve_refuses_an_unconfigured_address_and_names_the_remedy() {
        let list = icloud();
        let err = resolve_send_as(&list, Some("ceo@example.com")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ceo@example.com"), "{msg}");
        assert!(msg.contains("rileyprime@icloud.com"), "lists what IS allowed: {msg}");
        assert!(msg.to_lowercase().contains("send-as"), "{msg}");
    }

    #[test]
    fn delivery_headers_are_read_folding_aware_and_deduped() {
        let block = "Delivered-To: rileyprime@icloud.com\r\n\
                     Received: from mx.example\r\n\
                     \tby icloud.com\r\n\
                     X-Original-To: Christopher Robinson\r\n \t<rileyprime@icloud.com>\r\n\
                     Envelope-To: other@example.com\r\n\
                     Subject: hi\r\n";
        let got = delivery_header_addresses(block);
        assert_eq!(
            got,
            vec![
                "rileyprime@icloud.com".to_string(),
                "other@example.com".to_string()
            ],
            "folded value must be read whole, and the repeat must collapse: {got:?}"
        );
    }

    #[test]
    fn a_header_block_with_no_delivery_headers_yields_nothing() {
        assert!(delivery_header_addresses("Subject: hi\r\nTo: a@b.com\r\n").is_empty());
    }

    /// The delivery-header leg is the only one that survives a forwarder
    /// rewriting `To`, which is exactly when To/Cc cannot answer.
    #[test]
    fn a_delivery_header_decides_when_to_and_cc_name_nobody_we_are() {
        let list = icloud();
        let mut recipients = vec!["list@discuss.example".to_string()];
        recipients.extend(delivery_header_addresses(
            "Delivered-To: rileyprime@icloud.com\r\n",
        ));
        assert_eq!(
            match_send_as(&list, &recipients).unwrap().email,
            "rileyprime@icloud.com"
        );
    }
}

/// Detection from Sent mail, the `send_as_hidden` tombstone, and delivery-header
/// suggestions — against a real schema, on the shapes seen in the live mailbox.
#[cfg(test)]
mod detection_tests {
    use super::*;

    const PRIMARY: &str = "sidalias@icloud.com";
    const ALIAS: &str = "rileyprime@icloud.com";

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::initialize(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host)
             VALUES ('acct', ?1, 'icloud', 'h', 'h')",
            params![PRIMARY],
        )
        .unwrap();
        for (name, kind) in [("Sent Messages", "sent"), ("Drafts", "drafts"), ("INBOX", "inbox")] {
            conn.execute(
                "INSERT INTO folders (account_id, name, folder_type) VALUES ('acct', ?1, ?2)",
                params![name, kind],
            )
            .unwrap();
        }
        conn
    }

    fn msg(conn: &Connection, folder: &str, uid: u32, from: &str, to: &str, delivered: Option<&str>) {
        conn.execute(
            "INSERT INTO messages (account_id, folder_name, uid, from_email, to_list, date, delivered_to)
             VALUES ('acct', ?1, ?2, ?3, ?4, '2026-09-01T00:00:00Z', ?5)",
            params![folder, uid, from, format!(r#"[{{"name":null,"email":"{to}"}}]"#), delivered],
        )
        .unwrap();
    }

    fn list(conn: &Connection) -> Vec<SendAsAddress> {
        send_as_addresses(conn, "acct", PRIMARY, None).unwrap()
    }

    /// The live case: uids 78/79 in iCloud's Sent carry `From:` the alias. That
    /// alone makes it a send-as — nothing typed, nothing confirmed.
    #[test]
    fn an_address_on_sent_mail_is_a_send_as_without_being_configured() {
        let conn = db();
        msg(&conn, "Sent Messages", 78, "rileyprime@icloud.com", "a@x.com", None);
        msg(&conn, "Sent Messages", 79, "RileyPrime@iCloud.com", "a@x.com", None);
        msg(&conn, "Sent Messages", 80, PRIMARY, "a@x.com", None);
        let got = list(&conn);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].is_primary);
        assert_eq!(normalize_addr(&got[1].email), ALIAS);
        assert!(got[1].from_sent && got[1].identity_id.is_none());
        // And it resolves as a From, which is what lets send/draft accept it.
        assert!(resolve_send_as(&got, Some(ALIAS)).is_ok());
    }

    /// Only the Sent folder is proof. A draft was never submitted, and an
    /// inbox message's From is somebody else.
    #[test]
    fn drafts_and_received_mail_prove_nothing() {
        let conn = db();
        msg(&conn, "Drafts", 1, "spoof@icloud.com", "a@x.com", None);
        msg(&conn, "INBOX", 2, "friend@icloud.com", PRIMARY, None);
        assert_eq!(list(&conn).len(), 1);
    }

    /// A configured row wins over the same address on Sent mail — it carries
    /// the user's display name and signature.
    #[test]
    fn a_configured_alias_is_not_listed_twice() {
        let conn = db();
        msg(&conn, "Sent Messages", 78, ALIAS, "a@x.com", None);
        insert(
            &conn,
            &Identity {
                id: None,
                account_id: "acct".into(),
                email: ALIAS.into(),
                display_name: Some("Christopher".into()),
                signature_html: None,
                is_default: false,
            },
        )
        .unwrap();
        let got = list(&conn);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(!got[1].from_sent);
        assert_eq!(got[1].display_name.as_deref(), Some("Christopher"));
    }

    /// Removing a found-in-Sent alias must stick — without the tombstone it is
    /// back on the next read — and re-adding it undoes the removal.
    #[test]
    fn removing_an_alias_keeps_it_removed_until_it_is_added_back() {
        let conn = db();
        msg(&conn, "Sent Messages", 78, ALIAS, "a@x.com", None);
        remove_send_as(&conn, "acct", PRIMARY, "RileyPrime@icloud.com").unwrap();
        assert_eq!(list(&conn).len(), 1);
        assert!(resolve_send_as(&list(&conn), Some(ALIAS)).is_err());

        unhide_send_as(&conn, "acct", ALIAS).unwrap();
        assert_eq!(list(&conn).len(), 2);

        assert!(
            remove_send_as(&conn, "acct", PRIMARY, "SidAlias@icloud.com").is_err(),
            "the primary is not removable"
        );
    }

    /// iCloud's `Original-recipient: rfc822;addr`, verbatim from uid 1156.
    #[test]
    fn icloud_original_recipient_is_read_without_its_address_type() {
        let block = "Return-path: <bounces@em.example>\r\n\
                     Original-recipient: rfc822;rileyprime@icloud.com\r\n\
                     To: Christopher Robinson <rileyprime@icloud.com>\r\n";
        assert_eq!(delivery_header_addresses(block), vec![ALIAS.to_string()]);
    }

    /// The reply default reads sync's `delivered_to` column: a message whose
    /// `To:` is a list still answers from the alias it was delivered on.
    #[test]
    fn the_reply_default_uses_the_delivered_to_column() {
        let conn = db();
        msg(&conn, "Sent Messages", 78, ALIAS, "a@x.com", None);
        msg(&conn, "INBOX", 5, "list@lists.example", "members@lists.example", Some(ALIAS));
        let hit = reply_from_for_message(&conn, "acct", PRIMARY, None, "INBOX", 5).unwrap();
        assert_eq!(normalize_addr(&hit.email), ALIAS);
    }

    /// Suggestions come from delivery evidence only. The regression this
    /// guards: a same-domain `To:` filter offered every other `@gmail.com`
    /// co-recipient on `cxrobx@gmail.com` as an alias.
    #[test]
    fn suggestions_come_from_delivery_headers_not_from_co_recipients() {
        let conn = db();
        for uid in 10..15 {
            msg(&conn, "INBOX", uid, "x@y.com", "stranger@icloud.com", Some(PRIMARY));
        }
        msg(&conn, "INBOX", 20, "x@y.com", "list@y.com", Some("other@icloud.com"));
        msg(&conn, "INBOX", 21, "x@y.com", "list@y.com", Some("other@icloud.com, sidalias@icloud.com"));
        msg(&conn, "INBOX", 22, "x@y.com", "list@y.com", Some("gone@icloud.com"));
        msg(&conn, "INBOX", 23, "x@y.com", "list@y.com", Some("SidAlias+signup@icloud.com"));
        remove_send_as(&conn, "acct", PRIMARY, "gone@icloud.com").unwrap();

        let got = suggest_aliases(&conn, "acct", PRIMARY, 8).unwrap();
        assert_eq!(got, vec![("other@icloud.com".to_string(), 2)], "{got:?}");
    }
}
