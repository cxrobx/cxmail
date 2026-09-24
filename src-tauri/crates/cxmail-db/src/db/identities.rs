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
// extension, and the provider APIs that do know (Gmail's
// `users.settings.sendAs.list`, scope `gmail.settings.basic`; iCloud's private
// web API) are outside CXMail's grant, which is plain `https://mail.google.com/`
// for IMAP/SMTP. So aliases are **user-entered**, one row per alias in
// `identities`, and this module is where "which addresses may this account
// send as" is answered.

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
    Ok(build_send_as(
        account_id,
        account_email,
        account_display_name,
        &rows,
    ))
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

/// Addresses this account has received mail at that are not already a
/// configured send-as — candidate aliases, offered so the user picks from
/// their own mailbox instead of typing an address from memory.
///
/// Counted over the account's own cached `to_list` / `cc_list`, most-received
/// first. A suggestion is never applied on its own: the user still has to add
/// it, because only they know which of these is an alias of THIS mailbox and
/// which is a list they happen to be on.
pub fn suggest_aliases(
    conn: &Connection,
    account_id: &str,
    account_email: &str,
    limit: usize,
) -> Result<Vec<(String, u32)>, AppError> {
    let existing: Vec<String> = send_as_addresses(conn, account_id, account_email, None)?
        .iter()
        .map(|a| normalize_addr(&a.email))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT to_list, cc_list FROM messages
         WHERE account_id = ?1 AND to_list IS NOT NULL AND to_list != ''
         ORDER BY date DESC LIMIT 4000",
    )?;
    let rows = stmt.query_map(params![account_id], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    // (normalized key, first-seen spelling, count) — the spelling is kept so
    // the suggestion reads the way the sender wrote it.
    let mut tally: Vec<(String, String, u32)> = Vec::new();
    let domain = account_email.rsplit_once('@').map(|(_, d)| normalize_addr(d));
    for row in rows {
        let (to_json, cc_json) = row?;
        for json in [to_json, cc_json].into_iter().flatten() {
            for addr in crate::db::messages::addresses_from_json_pub(&json) {
                let key = normalize_addr(&addr);
                if key.is_empty() || existing.contains(&key) {
                    continue;
                }
                // Only same-domain addresses: an alias of this mailbox lives on
                // the mailbox's own domain, and everything else is every
                // correspondent the account has ever been cc'd with.
                match (&domain, key.rsplit_once('@')) {
                    (Some(d), Some((_, kd))) if kd == d => {}
                    _ => continue,
                }
                match tally.iter_mut().find(|(k, _, _)| *k == key) {
                    Some(entry) => entry.2 += 1,
                    None => tally.push((key, addr, 1)),
                }
            }
        }
    }
    tally.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    Ok(tally
        .into_iter()
        .take(limit)
        .map(|(_, spelling, count)| (spelling, count))
        .collect())
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
    const WANTED: [&str; 3] = ["delivered-to", "x-original-to", "envelope-to"];
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
fn extract_addresses(value: &str) -> Vec<String> {
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
        let addr = addr.trim();
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
