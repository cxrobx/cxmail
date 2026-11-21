//! Send a reviewed draft: the one send path outside the CXMail window.
//!
//! This is **not** an MCP tool, on purpose. The MCP never delivers mail
//! (`send_email` refuses by design), and every agent on the machine can reach
//! the MCP. These functions are reached only through the
//! `cxmail-mcp remote …` subcommands, which CXMail Remote (`remote/`) calls
//! after a person has looked at the draft on their phone:
//!
//! 1. `preview_draft` fetches the draft fresh from IMAP and returns everything
//!    that would go out (From, every To/Cc/Bcc, subject, body, attachment names) plus a SHA-256
//!    of the stored message.
//! 2. `send_draft` fetches it again, refuses unless the bytes still hash to
//!    the digest the person approved, then sends the original MIME body using
//!    the compose window's SMTP transport and retires the draft.
//! 3. `preview_attachment` checks the same digest before returning one file.
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
    pub edit_text: String,
    pub edit_tail_html: Option<String>,
    /// Attachment names shown before the person taps Send.
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
    uidvalidity: u32,
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
    let (uidvalidity, _) = imap::select_folder(&mut session, folder).await?;
    if !selected_uid_exists(&mut session, uid).await? {
        let _ = imap::disconnect(session, &account.email).await;
        return Err(AppError::NotFound(
            "That draft is gone: it was sent, deleted, or edited (an edit saves it under a new UID). Reopen Drafts.".into(),
        ));
    }
    let raw = imap::fetch_body(&mut session, uid).await?;
    // Gotcha #9: the parser can panic on hostile HTML.
    let parsed = std::panic::catch_unwind(|| parser::parse_message_without_attachment_text(&raw))
        .map_err(|_| AppError::Parse("The draft could not be parsed.".into()))?;
    Ok((Fetched { account, raw, parsed, uidvalidity }, session))
}

async fn selected_uid_exists(session: &mut imap::ImapSession, uid: u32) -> Result<bool, AppError> {
    session.uid_search(format!("UID {uid}")).await
        .map(|uids| uids.contains(&uid))
        .map_err(|error| AppError::Imap(format!("Draft UID lookup failed: {error}")))
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
    let blocked = blocked_reason(p).or_else(|| {
        parser::reviewed_draft_mime(&f.raw).err().map(str::to_owned)
    });
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
        edit_text: parser::editable_draft_text(p),
        edit_tail_html: parser::editable_draft_tail(p).map(str::to_owned),
        attachments: parser::draft_attachment_names(&f.raw),
        sendable: blocked.is_none(),
        blocked_reason: blocked,
        digest: digest(&f.raw),
    })
}

pub fn compose_identity(
    db_path: &str,
    account_id: &str,
    reply: Option<(&str, u32)>,
) -> Result<serde_json::Value, AppError> {
    let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    crate::mcp::server::compose_identity(&conn, account_id, reply)
}

const CHANGED_DRAFT: &str = "The draft changed after you reviewed it. Review the draft again.";

pub async fn preview_attachment(
    db_path: &str,
    account_id: &str,
    folder: &str,
    uid: u32,
    approved_digest: &str,
    index: usize,
) -> Result<Vec<u8>, AppError> {
    let (f, session) = fetch_draft(db_path, account_id, folder, uid).await?;
    let _ = imap::disconnect(session, &f.account.email).await;
    if digest(&f.raw) != approved_digest {
        return Err(AppError::General(CHANGED_DRAFT.into()));
    }
    parser::reviewed_draft_attachment(&f.raw, index, 25 * 1024 * 1024)
        .map_err(|reason| AppError::General(reason.into()))
}

#[derive(serde::Deserialize)]
pub struct DraftEdit {
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body: String,
    /// Files added on the phone, appended after the draft's existing parts.
    #[serde(default)]
    files: Vec<NewDraftFile>,
}

#[derive(serde::Deserialize)]
pub struct NewDraftFile {
    name: String,
    /// Standard base64 of the file bytes.
    data: String,
}

const MAX_NEW_FILES: usize = 10;
const MAX_NEW_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_NEW_FILES_TOTAL: usize = 40 * 1024 * 1024;

impl DraftEdit {
    /// Decoded `(name, content type, bytes)` for each new file, or why they were refused.
    fn decoded_files(&self) -> Result<Vec<(String, String, Vec<u8>)>, AppError> {
        use base64::Engine;
        let refused = |why: &str| AppError::General(format!("The new attachment was refused: {why}."));
        if self.files.len() > MAX_NEW_FILES {
            return Err(refused("too many files"));
        }
        let mut total = 0;
        let mut out = Vec::with_capacity(self.files.len());
        for file in &self.files {
            let name = file.name.trim();
            if name.is_empty() || name.len() > 200 || name.chars().any(|c| c.is_control() || "/\\\"".contains(c)) {
                return Err(refused("its name is not a plain file name"));
            }
            if file.data.len() > MAX_NEW_FILE_BYTES / 3 * 4 + 4 {
                return Err(refused("each file must be under 25 MB"));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(file.data.as_bytes())
                .map_err(|_| refused("it is not valid base64"))?;
            total += bytes.len();
            if bytes.is_empty() || bytes.len() > MAX_NEW_FILE_BYTES || total > MAX_NEW_FILES_TOTAL {
                return Err(refused("files must be under 25 MB each and 40 MB together"));
            }
            let content_type = cxmail_email::email::attachment_file::guess_content_type(name);
            out.push((name.to_string(), content_type, bytes));
        }
        Ok(out)
    }
}

impl DraftEdit {
    fn validate(&self) -> Result<(), AppError> {
        let invalid = [&self.to, &self.cc, &self.bcc].into_iter().any(|list| {
            list.len() > 50 || list.iter().any(|a| {
                let Some((local, domain)) = a.split_once('@') else { return true; };
                a.len() >= 320 || local.is_empty() || !domain.contains('.') || domain.contains('@')
                    || a.chars().any(|c| c.is_control() || c.is_whitespace() || "<>,;".contains(c))
            })
        });
        if invalid || self.to.is_empty() || self.subject.len() > 998
            || self.subject.chars().any(char::is_control) || self.body.len() > 256 * 1024
            || self.body.contains('\0')
        {
            return Err(AppError::General("Check the draft recipients, subject and body.".into()));
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct DraftEditOutcome {
    pub uid: u32,
    pub old_removed: bool,
}

fn build_edited_draft(
    f: &Fetched,
    edit: &DraftEdit,
    draft_id: &str,
    new_id: &str,
) -> Result<Vec<u8>, AppError> {
    use mail_builder::{headers::raw::Raw, mime::MimePart, MessageBuilder};
    let mime = if edit.body == parser::editable_draft_text(&f.parsed) {
        let (mut headers, body) = parser::reviewed_draft_mime(&f.raw)
            .map_err(|e| AppError::Parse(e.into()))?;
        headers.extend_from_slice(body);
        headers
    } else {
        parser::replace_draft_prose(&f.raw, &f.parsed, &edit.body)
            .map_err(|e| AppError::Parse(e.into()))?
    };
    let mime = parser::append_draft_files(
        &mime,
        &edit.decoded_files()?,
        &format!("cxmail-{}", uuid::Uuid::new_v4().simple()),
    )
    .map_err(|e| AppError::Parse(e.into()))?;
    let p = &f.parsed;
    let to = draft_recipients(&edit.to, &p.to_list);
    let cc = draft_recipients(&edit.cc, &p.cc_list);
    let bcc = draft_recipients(&edit.bcc, &p.bcc_list);
    let mut message = MessageBuilder::new()
        .message_id(new_id)
        .header(cxmail_email::email::draft_local::DRAFT_ID_HEADER, Raw::new(draft_id))
        .subject(&edit.subject)
        .to(smtp::to_address_list(&to));
    if let Some(name) = &p.from_name {
        message = message.from((name.as_str(), p.from_email.as_str()));
    } else {
        message = message.from(p.from_email.as_str());
    }
    if !cc.is_empty() { message = message.cc(smtp::to_address_list(&cc)); }
    if !bcc.is_empty() { message = message.bcc(smtp::to_address_list(&bcc)); }
    if let Some(id) = &p.in_reply_to {
        message = message.in_reply_to(smtp::strip_message_id_brackets(id));
    }
    let references = p.references.iter().map(|id| bracket(id)).collect::<Vec<_>>().join(" ");
    if !references.is_empty() {
        message = message.header("References", Raw::new(references));
    }
    Ok(message.body(MimePart::raw(mime)).write_to_vec()?)
}

fn draft_recipients(values: &[String], original: &[parser::EmailAddress]) -> Vec<smtp::EmailRecipient> {
    values.iter().map(|email| smtp::EmailRecipient {
        email: email.clone(),
        name: original.iter().find(|r| r.email.eq_ignore_ascii_case(email)).and_then(|r| r.name.clone()),
    }).collect()
}

pub async fn edit_reviewed_draft(
    db_path: &str,
    account_id: &str,
    folder: &str,
    uid: u32,
    expected_digest: &str,
    edit: DraftEdit,
) -> Result<DraftEditOutcome, AppError> {
    use cxmail_email::email::{draft_local, draft_revisions};
    edit.validate()?;
    // Refuse bad files before any IMAP work.
    edit.decoded_files()?;
    let conn = rusqlite::Connection::open(db_path)?;
    let known = db::drafts::find_by_uid(&conn, account_id, folder, uid)?;
    let (f, mut session) = fetch_draft(db_path, account_id, folder, uid).await?;
    let mut claim = None;
    let result = async {
        if digest(&f.raw) != expected_digest {
            return Err(AppError::General(CHANGED_DRAFT.into()));
        }
        let scan = draft_revisions::scan(&mut session, folder).await?;
        let id = scan.draft_id_of(uid).map(str::to_owned)
            .or_else(|| known.as_ref().map(|row| row.draft_id.clone()))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // A draft identifier is a generated UUID. Foreign headers don't get
        // to inject an arbitrary value into the revision header we write.
        if uuid::Uuid::parse_str(&id).is_err() {
            return Err(AppError::General("This draft identifier could not be validated.".into()));
        }
        let revisions = scan.revisions(&id);
        if matches!(draft_revisions::resolve(revisions, uid, false), draft_revisions::Resolution::Moved { .. })
            || known.as_ref().is_some_and(|row| row.uidvalidity != scan.uidvalidity || row.draft_id != id)
            || scan.uidvalidity != f.uidvalidity
            || !selected_uid_exists(&mut session, uid).await?
        {
            return Err(AppError::General(CHANGED_DRAFT.into()));
        }
        let token = uuid::Uuid::new_v4().to_string();
        match db::drafts::claim(&conn, &id, uid, &token)? {
            db::drafts::ClaimOutcome::Claimed => claim = Some((id.clone(), token)),
            db::drafts::ClaimOutcome::Missing => {}
            _ => return Err(AppError::General(CHANGED_DRAFT.into())),
        }
        let new_id = format!("{}@cxmail.app", uuid::Uuid::new_v4());
        let raw = build_edited_draft(&f, &edit, &id, &new_id)?;
        session.append(folder, Some("(\\Seen \\Draft)"), None, &raw)
            .await.map_err(|e| AppError::Imap(format!("Saving draft failed: {e}")))?;
        let new_uid = imap::find_uid_by_message_id(&mut session, folder, &bracket(&new_id)).await?;
        let retired = draft_revisions::retire(
            &mut session, folder, &draft_revisions::superseded(revisions, uid, new_uid),
        ).await;
        let pointed = if let Some((_, token)) = &claim {
            db::drafts::advance(&conn, &id, token, scan.uidvalidity, new_uid)
        } else {
            db::drafts::link(&conn, &id, account_id, folder, scan.uidvalidity, new_uid)
        };
        if let Err(e) = pointed {
            log::warn!("Could not advance phone-edited draft {id}: {e}");
        }
        if let Err(e) = db::messages::delete_uids(&conn, account_id, folder, &retired) {
            log::warn!("Could not evict superseded phone-edited draft rows: {e}");
        }
        if let Err(e) = draft_local::persist_local_draft(&conn, account_id, folder, new_uid, &raw) {
            log::warn!("Could not cache phone-edited draft {id}: {e}");
        }
        Ok(DraftEditOutcome { uid: new_uid, old_removed: retired.contains(&uid) })
    }.await;
    if let Some((id, token)) = claim {
        let _ = db::drafts::release(&conn, &id, &token);
    }
    let _ = imap::disconnect(session, &f.account.email).await;
    if let Ok(saved) = &result {
        notify(&Envelope::new("draft-updated", account_id, folder, saved.uid)
            .with_old_uid(uid).with_tool("remote_edit_draft")).await;
    }
    result
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
    let message_id = match smtp::send_reviewed_draft(&f.account, &email, &f.raw).await {
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

    fn edit_with(files: serde_json::Value) -> Result<DraftEdit, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "to": ["ann@example.org"], "cc": [], "bcc": [], "subject": "s", "body": "b", "files": files,
        }))
    }

    #[test]
    fn phone_files_are_decoded_and_typed() {
        let edit = edit_with(serde_json::json!([{"name": "Quote v2.pdf", "data": "JVBERi0x"}])).unwrap();
        let files = edit.decoded_files().unwrap();
        assert_eq!(files, [("Quote v2.pdf".to_string(), "application/pdf".to_string(), b"%PDF-1".to_vec())]);
        // Older phone builds send no `files` key at all.
        let plain: DraftEdit = serde_json::from_value(serde_json::json!({
            "to": ["ann@example.org"], "cc": [], "bcc": [], "subject": "s", "body": "b",
        })).unwrap();
        assert!(plain.decoded_files().unwrap().is_empty());
    }

    #[test]
    fn phone_files_refuse_paths_bad_bytes_and_excess() {
        for bad in [
            serde_json::json!([{"name": "../id_ed25519", "data": "YQ=="}]),
            serde_json::json!([{"name": "a\u{7}.txt", "data": "YQ=="}]),
            serde_json::json!([{"name": " ", "data": "YQ=="}]),
            serde_json::json!([{"name": "a.txt", "data": "%%%"}]),
            serde_json::json!([{"name": "a.txt", "data": ""}]),
            serde_json::Value::Array(vec![serde_json::json!({"name": "a.txt", "data": "YQ=="}); MAX_NEW_FILES + 1]),
        ] {
            let edit = edit_with(bad.clone()).unwrap();
            assert!(edit.decoded_files().is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn refuses_no_recipients() {
        let none = DRAFT
            .replace("To: Ann <ann@example.org>\r\n", "")
            .replace("Cc: bob@example.org\r\n", "")
            .replace("Bcc: secret@example.org\r\n", "");
        assert!(outgoing_from_draft(&parse(&none), &from()).unwrap_err().contains("no recipients"));
    }
}
