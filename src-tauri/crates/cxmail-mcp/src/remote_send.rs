//! Send a reviewed draft: the one send path outside the CXMail window.
//!
//! This is **not** an MCP tool, on purpose. The MCP never delivers mail
//! (`send_email` refuses by design), and every agent on the machine can reach
//! the MCP. These two functions are reached only through the
//! `cxmail-mcp remote …` subcommands, which CXMail Remote (`remote/`) calls
//! after a person has looked at the draft on their phone:
//!
//! 1. `preview_draft` fetches the draft fresh from IMAP and returns everything
//!    that would go out (From, every To/Cc/Bcc, subject, body) plus a SHA-256
//!    of the stored message.
//! 2. `send_draft` fetches it again, refuses unless the bytes still hash to
//!    the digest the person approved, then sends through the same
//!    `smtp::send_email` the compose window uses and retires the draft.
//!
//! So what is approved is exactly what leaves: a draft edited anywhere after
//! the review (the Mac, an agent's `edit_draft`) gets a new UID or new bytes,
//! and the send refuses. Only messages in the account's Drafts folder can be
//! sent, so this can't be pointed at an inbox message.

use cxmail_core::bridge::{notify, Envelope};
use cxmail_core::error::AppError;
use cxmail_db::db;
use cxmail_email::email::{imap, parser, smtp};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub struct Addr {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct DraftPreview {
    pub account_id: String,
    pub folder: String,
    pub uid: u32,
    pub from: Addr,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub bcc: Vec<Addr>,
    pub subject: String,
    /// Ammonia-sanitised, for display only (architecture invariant 2).
    pub sanitized_html: Option<String>,
    pub plain_text: Option<String>,
    /// Non-inline attachment file names. A draft with any attachment is
    /// refused at send time in this version; listing them lets the review
    /// screen say so before the person taps Send.
    pub attachments: Vec<String>,
    pub sendable: bool,
    pub blocked_reason: Option<String>,
    /// SHA-256 of the raw stored draft; `send_draft` must be given this back.
    pub digest: String,
}

#[derive(Debug, Serialize)]
pub struct SendOutcome {
    pub message_id: String,
    /// False when the mail went out but the draft could not be removed. The
    /// mail is NOT re-sent in that case: the draft just stays in Drafts.
    pub draft_removed: bool,
    pub note: Option<String>,
}

pub fn digest(raw: &[u8]) -> String {
    Sha256::digest(raw).iter().map(|b| format!("{b:02x}")).collect()
}

fn bracket(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') {
        id.to_string()
    } else {
        format!("<{id}>")
    }
}

/// Why a parsed draft can't be sent from here, or None when it can.
pub fn blocked_reason(parsed: &parser::ParsedMessage) -> Option<String> {
    if parsed.to_list.is_empty() && parsed.cc_list.is_empty() && parsed.bcc_list.is_empty() {
        return Some("The draft has no recipients.".into());
    }
    let named: Vec<&str> = parsed
        .attachments
        .iter()
        .filter(|a| !a.is_inline)
        .map(|a| a.filename.as_deref().unwrap_or("attachment"))
        .collect();
    if !parsed.attachments.is_empty() {
        let what = if named.is_empty() {
            "inline images".to_string()
        } else {
            named.join(", ")
        };
        return Some(format!(
            "The draft carries attachments ({what}). Sending those from the phone isn't supported yet: send it from CXMail on the Mac."
        ));
    }
    if parsed.html_body.is_none() && parsed.plain_text.is_none() {
        return Some("The draft has no body.".into());
    }
    None
}

/// The draft as the compose window would send it. `from` has already been
/// checked against the account's send-as addresses.
pub fn outgoing_from_draft(
    parsed: &parser::ParsedMessage,
    from: &db::identities::SendAsAddress,
) -> Result<smtp::OutgoingEmail, String> {
    if let Some(reason) = blocked_reason(parsed) {
        return Err(reason);
    }
    let rcpt = |list: &[parser::EmailAddress]| -> Vec<smtp::EmailRecipient> {
        list.iter()
            .map(|a| smtp::EmailRecipient {
                name: a.name.clone(),
                email: a.email.clone(),
            })
            .collect()
    };
    let html_body = match (&parsed.html_body, &parsed.plain_text) {
        (Some(h), _) => h.clone(),
        (None, Some(p)) => format!("<pre>{}</pre>", html_escape(p)),
        (None, None) => unreachable!("blocked_reason refuses a bodyless draft"),
    };
    let references = if parsed.references.is_empty() {
        None
    } else {
        Some(
            parsed
                .references
                .iter()
                .map(|r| bracket(r))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };
    Ok(smtp::OutgoingEmail {
        from_email: from.email.clone(),
        from_name: from.display_name.clone(),
        to: rcpt(&parsed.to_list),
        cc: rcpt(&parsed.cc_list),
        bcc: rcpt(&parsed.bcc_list),
        subject: parsed.subject.clone().unwrap_or_default(),
        html_body,
        plain_body: parsed.plain_text.clone(),
        in_reply_to: parsed.in_reply_to.as_deref().map(bracket),
        references,
        track_opens: None,
        attachments: vec![],
    })
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn addrs(list: &[parser::EmailAddress]) -> Vec<Addr> {
    list.iter()
        .map(|a| Addr {
            name: a.name.clone(),
            email: a.email.clone(),
        })
        .collect()
}

struct Fetched {
    account: db::accounts::Account,
    raw: Vec<u8>,
    parsed: parser::ParsedMessage,
}

/// Load the account, insist `folder` is its Drafts folder, and fetch the
/// draft's raw bytes fresh from IMAP. Leaves the session open for the caller.
async fn fetch_draft(
    db_path: &str,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<(Fetched, imap::ImapSession), AppError> {
    let account = {
        let conn = rusqlite::Connection::open(db_path)?;
        let account = db::accounts::get_by_id(&conn, account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".into()))?;
        let drafts = db::folders::folder_for_account(&conn, &account.id, &account.provider, "drafts");
        if drafts != folder {
            return Err(AppError::General(format!(
                "Only drafts can be sent from here, and {folder:?} is not this account's Drafts folder ({drafts:?})."
            )));
        }
        account
    };
    let mut session = imap::connect_for_account(&account).await?;
    imap::select_folder(&mut session, folder).await?;
    if !imap::uid_exists(&mut session, folder, uid).await? {
        let _ = imap::disconnect(session, &account.email).await;
        return Err(AppError::NotFound(
            "That draft is gone: it was sent, deleted, or edited (an edit saves it under a new UID). Reopen Drafts.".into(),
        ));
    }
    let raw = imap::fetch_body(&mut session, uid).await?;
    // Gotcha #9: the parser can panic on hostile HTML.
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser::parse_message(&raw)))
        .map_err(|_| AppError::Parse("The draft could not be parsed.".into()))?;
    Ok((Fetched { account, raw, parsed }, session))
}

pub async fn preview_draft(
    db_path: &str,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<DraftPreview, AppError> {
    let (f, session) = fetch_draft(db_path, account_id, folder, uid).await?;
    let _ = imap::disconnect(session, &f.account.email).await;
    let p = &f.parsed;
    let blocked = blocked_reason(p);
    Ok(DraftPreview {
        account_id: account_id.to_string(),
        folder: folder.to_string(),
        uid,
        from: Addr {
            name: p.from_name.clone(),
            email: p.from_email.clone(),
        },
        to: addrs(&p.to_list),
        cc: addrs(&p.cc_list),
        bcc: addrs(&p.bcc_list),
        subject: p.subject.clone().unwrap_or_default(),
        sanitized_html: p.sanitized_html.clone(),
        plain_text: p.plain_text.clone(),
        attachments: p
            .attachments
            .iter()
            .filter(|a| !a.is_inline)
            .map(|a| a.filename.clone().unwrap_or_else(|| "attachment".into()))
            .collect(),
        sendable: blocked.is_none(),
        blocked_reason: blocked,
        digest: digest(&f.raw),
    })
}

pub async fn send_draft(
    db_path: &str,
    account_id: &str,
    folder: &str,
    uid: u32,
    approved_digest: &str,
) -> Result<SendOutcome, AppError> {
    let (f, mut session) = fetch_draft(db_path, account_id, folder, uid).await?;
    // Everything that can refuse happens before SMTP, and the IMAP session is
    // released on every refusal.
    let prepared = (|| {
        if digest(&f.raw) != approved_digest.trim().to_ascii_lowercase() {
            return Err(AppError::General(
                "The draft changed after you reviewed it. Nothing was sent: review it again.".into(),
            ));
        }
        let conn = rusqlite::Connection::open(db_path)?;
        let addresses = db::identities::send_as_addresses(
            &conn,
            &f.account.id,
            &f.account.email,
            f.account.display_name.as_deref(),
        )?;
        let from = db::identities::resolve_send_as(&addresses, Some(&f.parsed.from_email))?;
        outgoing_from_draft(&f.parsed, &from).map_err(AppError::General)
    })();
    let email = match prepared {
        Ok(e) => e,
        Err(e) => {
            let _ = imap::disconnect(session, &f.account.email).await;
            return Err(e);
        }
    };

    // Point of no return. A failure past here must never lead to a resend.
    let message_id = match smtp::send_email(&f.account, &email).await {
        Ok(id) => id,
        Err(e) => {
            let _ = imap::disconnect(session, &f.account.email).await;
            return Err(e);
        }
    };

    // Retire the draft the way the draft-replace path does: flag one UID and
    // expunge only that UID (never a mailbox-wide EXPUNGE).
    let removed = async {
        imap::store_flags(&mut session, uid, true, "\\Deleted").await?;
        imap::uid_expunge(&mut session, uid).await
    }
    .await;
    let _ = imap::disconnect(session, &f.account.email).await;

    notify(&Envelope::new("email-mutated", account_id, folder, uid).with_tool("remote_send_draft")).await;

    Ok(match removed {
        Ok(()) => SendOutcome {
            message_id,
            draft_removed: true,
            note: None,
        },
        Err(e) => SendOutcome {
            message_id,
            draft_removed: false,
            note: Some(format!(
                "Sent. The draft could not be removed ({e}), so delete it from Drafts yourself; it will not be sent again."
            )),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> parser::ParsedMessage {
        parser::parse_message(raw.as_bytes())
    }

    fn from() -> db::identities::SendAsAddress {
        db::identities::SendAsAddress {
            account_id: "a".into(),
            email: "me@example.com".into(),
            display_name: Some("Me".into()),
            signature_html: None,
            is_primary: true,
            identity_id: None,
            from_sent: false,
        }
    }

    const DRAFT: &str = "From: Me <me@example.com>\r\nTo: Ann <ann@example.org>\r\nCc: bob@example.org\r\nBcc: secret@example.org\r\nSubject: Re: budget\r\nIn-Reply-To: <orig@example.org>\r\nReferences: <root@example.org> <orig@example.org>\r\nMessage-ID: <d1@cxmail.app>\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Sounds good.</p>\r\n";

    #[test]
    fn digest_is_stable_and_byte_sensitive() {
        assert_eq!(digest(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_ne!(digest(DRAFT.as_bytes()), digest(DRAFT.replace("good", "great").as_bytes()));
    }

    #[test]
    fn every_recipient_survives_including_bcc() {
        let out = outgoing_from_draft(&parse(DRAFT), &from()).unwrap();
        let emails = |v: &[smtp::EmailRecipient]| v.iter().map(|r| r.email.clone()).collect::<Vec<_>>();
        assert_eq!(emails(&out.to), ["ann@example.org"]);
        assert_eq!(emails(&out.cc), ["bob@example.org"]);
        assert_eq!(emails(&out.bcc), ["secret@example.org"]);
        assert_eq!(out.to[0].name.as_deref(), Some("Ann"));
    }

    /// Bcc recipients reach the SMTP envelope (and, by
    /// `smtp::tests::send_path_never_writes_a_bcc_header`, nowhere else).
    #[test]
    fn bcc_goes_in_the_envelope() {
        let out = outgoing_from_draft(&parse(DRAFT), &from()).unwrap();
        assert!(smtp::envelope_rcpts(&out).contains(&"secret@example.org"));
    }

    #[test]
    fn threading_headers_are_bracketed() {
        let out = outgoing_from_draft(&parse(DRAFT), &from()).unwrap();
        assert_eq!(out.in_reply_to.as_deref(), Some("<orig@example.org>"));
        assert_eq!(out.references.as_deref(), Some("<root@example.org> <orig@example.org>"));
        assert_eq!(out.subject, "Re: budget");
        assert!(out.html_body.contains("Sounds good."));
    }

    #[test]
    fn from_comes_from_the_resolved_send_as_address() {
        let out = outgoing_from_draft(&parse(DRAFT), &from()).unwrap();
        assert_eq!(out.from_email, "me@example.com");
        assert_eq!(out.from_name.as_deref(), Some("Me"));
    }

    #[test]
    fn refuses_no_recipients_and_attachments() {
        let none = DRAFT
            .replace("To: Ann <ann@example.org>\r\n", "")
            .replace("Cc: bob@example.org\r\n", "")
            .replace("Bcc: secret@example.org\r\n", "");
        assert!(outgoing_from_draft(&parse(&none), &from()).unwrap_err().contains("no recipients"));

        let with_att = "From: me@example.com\r\nTo: ann@example.org\r\nSubject: x\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=plan.pdf\r\n\r\nJVBERi0=\r\n--b--\r\n";
        let err = outgoing_from_draft(&parse(with_att), &from()).unwrap_err();
        assert!(err.contains("plan.pdf"), "{err}");
    }
}
