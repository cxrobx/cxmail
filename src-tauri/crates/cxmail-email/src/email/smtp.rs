use crate::db::accounts::Account;
use crate::email::imap;
use crate::email::oauth2;
use crate::email::providers;
use crate::error::AppError;
use crate::keychain;
use base64::Engine;
use mail_builder::MessageBuilder;
use mail_send::{Credentials, SmtpClientBuilder};
use serde::{Deserialize, Serialize};

/// Connect + auth timeout for SMTP. `mail-send`'s own default is 3600s.
const SMTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Auth mechanism to use against the SMTP server. Gmail/Outlook reject OAuth2
/// access tokens sent via AUTH PLAIN/LOGIN ("535 5.7.8 Username and Password
/// not accepted"); they must be wrapped in SASL XOAUTH2.
enum SmtpAuth {
    Plain { username: String, password: String },
    XOauth2 { username: String, token: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailRecipient {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    pub filename: String,
    pub content_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingEmail {
    pub from_email: String,
    pub from_name: Option<String>,
    pub to: Vec<EmailRecipient>,
    pub cc: Vec<EmailRecipient>,
    pub bcc: Vec<EmailRecipient>,
    pub subject: String,
    pub html_body: String,
    pub plain_body: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub track_opens: Option<bool>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachment>,
}

/// Convert recipients into a single `Address::List`. Used everywhere we hand
/// off to `mail_builder::MessageBuilder` so the `To:` / `Cc:` / `Bcc:` fields
/// each emit exactly one header line. See the comment in
/// `send_email_with_message_id` for why the per-recipient loop variant is wrong.
// `pub`: shared by the Tauri compose path (`commands::compose`) and the MCP
// send path (`mcp::server`), both of which are now other crates.
pub fn to_address_list(
    recipients: &[EmailRecipient],
) -> Vec<mail_builder::headers::address::Address<'_>> {
    recipients
        .iter()
        .map(|r| match &r.name {
            Some(name) => (name.as_str(), r.email.as_str()).into(),
            None => r.email.as_str().into(),
        })
        .collect()
}

/// Strip the surrounding angle brackets from a Message-ID before handing it to
/// mail-builder's `.in_reply_to()` (or `.message_id()`), which wrap the value in
/// `<` `>` themselves. Stored Message-IDs keep their brackets (the parser does
/// not strip them), so passing one straight through yields a malformed `<<id>>`
/// `In-Reply-To` header. Trimming all leading `<` / trailing `>` is also
/// defensive against an already-doubled value.
// `pub` for the same reason as `to_address_list` above.
pub fn strip_message_id_brackets(id: &str) -> &str {
    id.trim().trim_start_matches('<').trim_end_matches('>')
}

/// Send an email via SMTP for the given account. Returns the generated Message-ID.
pub async fn send_email(account: &Account, email: &OutgoingEmail) -> Result<String, AppError> {
    send_email_with_message_id(account, email, None).await
}

/// Send an email via SMTP, optionally forcing a pre-generated Message-ID.
pub async fn send_email_with_message_id(
    account: &Account,
    email: &OutgoingEmail,
    message_id: Option<&str>,
) -> Result<String, AppError> {
    let transport = get_smtp_transport(account).await?;
    let SmtpTransport {
        host,
        port,
        implicit_tls,
        auth,
    } = transport;

    log::info!("Sending email via {}:{} as {}", host, port, email.from_email);

    // Generate a stable Message-ID before building the message. The public API
    // uses the bracketed form `<id@host>` (matches what shows up in IMAP threading
    // headers and what callers store). Internally, mail-builder's MessageBuilder
    // wraps the value in `<>` itself — passing a bracketed value would produce
    // `<<id@host>>` in the actual header and break thread correlation.
    let msg_id = message_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("<{}@cxmail.app>", uuid::Uuid::new_v4()));
    let msg_id = if msg_id.starts_with('<') && msg_id.ends_with('>') {
        msg_id
    } else {
        format!("<{}>", msg_id)
    };
    let bare_message_id = msg_id
        .trim_start_matches('<')
        .trim_end_matches('>');

    let mut message = MessageBuilder::new();
    message = message.message_id(bare_message_id);

    // From
    if let Some(name) = &email.from_name {
        message = message.from((name.as_str(), email.from_email.as_str()));
    } else {
        message = message.from(email.from_email.as_str());
    }

    // mail-builder 0.3.2's `.to()` / `.cc()` / `.bcc()` push a fresh header
    // each call, so a per-recipient loop produces multiple `To:` lines and
    // Gmail rejects with `550 5.7.1 multiple To headers`. Build one
    // `Address::List` per field instead.
    if !email.to.is_empty() {
        message = message.to(to_address_list(&email.to));
    }
    if !email.cc.is_empty() {
        message = message.cc(to_address_list(&email.cc));
    }
    if !email.bcc.is_empty() {
        message = message.bcc(to_address_list(&email.bcc));
    }

    message = message.subject(&email.subject);
    let styled_html = super::inline_styles::apply_inline_font_styles(&email.html_body);
    message = message.html_body(&styled_html);

    if let Some(plain) = &email.plain_body {
        message = message.text_body(plain);
    }

    if let Some(reply_to) = &email.in_reply_to {
        // Pass the bare ID — mail-builder re-wraps in `<` `>`. See strip_message_id_brackets.
        message = message.in_reply_to(strip_message_id_brackets(reply_to));
    }

    if let Some(refs) = &email.references {
        message = message.header("References", mail_builder::headers::raw::Raw::new(refs.as_str()));
    }

    // Attachments
    for att in &email.attachments {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&att.data_base64)
            .map_err(|e| AppError::General(format!("Invalid base64 attachment: {}", e)))?;
        message = message.attachment(&att.content_type, &att.filename, decoded);
    }

    // `mail-send`'s builder default is a 3600-SECOND timeout, so a dead socket
    // parks a send for an hour with no signal. Everything else in CXMail that
    // touches a mail server is bounded (IMAP connect is 15s); match that order.
    let builder = SmtpClientBuilder::new(host, port)
        .implicit_tls(implicit_tls)
        .timeout(SMTP_TIMEOUT);
    let builder = match auth {
        SmtpAuth::Plain { username, password } => {
            builder.credentials(Credentials::new(username, password))
        }
        SmtpAuth::XOauth2 { username, token } => {
            builder.credentials(Credentials::new_xoauth2(username, token))
        }
    };
    // `connect()`, never `connect_plain()`: with implicit_tls(false) mail-send
    // returns Err(MissingStartTls) when the EHLO response has no STARTTLS, so
    // the TLS-enforced invariant holds on the SMTP side for free.
    builder
        .connect()
        .await
        .map_err(|e| AppError::Imap(format!("SMTP connection failed: {}", e)))?
        .send(message)
        .await
        .map_err(|e| AppError::Imap(format!("SMTP send failed: {}", e)))?;

    log::info!("Email sent successfully with Message-ID: {}", msg_id);
    Ok(msg_id)
}

/// Open and immediately close an authenticated SMTP session.
///
/// `mail-send`'s `connect()` runs the full EHLO → STARTTLS → AUTH handshake and
/// sends nothing, so this is a complete credential check rather than a probe —
/// which is what makes it safe to run during account setup.
pub async fn verify_transport(account: &Account) -> Result<(), AppError> {
    let SmtpTransport {
        host,
        port,
        implicit_tls,
        auth,
    } = get_smtp_transport(account).await?;

    let builder = SmtpClientBuilder::new(host, port)
        .implicit_tls(implicit_tls)
        .timeout(SMTP_TIMEOUT);
    let builder = match auth {
        SmtpAuth::Plain { username, password } => {
            builder.credentials(Credentials::new(username, password))
        }
        SmtpAuth::XOauth2 { username, token } => {
            builder.credentials(Credentials::new_xoauth2(username, token))
        }
    };

    let client = builder
        .connect()
        .await
        .map_err(|e| AppError::Imap(format!("{}", e)))?;
    let _ = client.quit().await;
    Ok(())
}

/// Everything needed to open an authenticated SMTP session.
struct SmtpTransport {
    host: String,
    port: u16,
    /// TLS before the greeting (465) vs STARTTLS upgrade (587).
    implicit_tls: bool,
    auth: SmtpAuth,
}

/// Resolve the account's SMTP transport, refreshing OAuth tokens as needed.
///
/// Takes the whole `Account` for the same reason `imap::connect_for_account`
/// does: the generic provider's host/port/security/login live on the row.
async fn get_smtp_transport(account: &Account) -> Result<SmtpTransport, AppError> {
    let email = account.email.as_str();
    match account.provider.as_str() {
        "gmail" => {
            let token = oauth2::get_valid_access_token(email).await?;
            Ok(SmtpTransport {
                host: "smtp.gmail.com".to_string(),
                port: 587,
                implicit_tls: false,
                auth: SmtpAuth::XOauth2 { username: email.to_string(), token },
            })
        }
        "outlook" => {
            let token = oauth2::get_valid_outlook_access_token(email).await?;
            Ok(SmtpTransport {
                host: "smtp.office365.com".to_string(),
                port: 587,
                implicit_tls: false,
                auth: SmtpAuth::XOauth2 { username: email.to_string(), token },
            })
        }
        "icloud" => {
            let password = keychain::get_credential(&imap::password_key("icloud", email))?
                .ok_or_else(|| AppError::AuthFailed("No iCloud password found".to_string()))?;
            Ok(SmtpTransport {
                host: "smtp.mail.me.com".to_string(),
                port: 587,
                implicit_tls: false,
                auth: SmtpAuth::Plain { username: email.to_string(), password },
            })
        }
        "imap" => {
            let host = providers::validate_host(&account.smtp_host)?;
            let port = providers::port_from_i32(account.smtp_port, "SMTP")?;
            let security = providers::TlsMode::parse(&account.smtp_security)?;
            // Called for its rejections (port 25 relay, POP3 ports), not its
            // answer — a host may legitimately run implicit TLS on an odd port.
            providers::security_for_port(port)?;

            // One password serves both transports (the setup form collects
            // one), and an SMTP login defaults to the IMAP one before the
            // email — servers that want a non-email login almost always want
            // the same login on both.
            let username = account
                .smtp_username
                .as_deref()
                .or(account.imap_username.as_deref())
                .filter(|u| !u.trim().is_empty())
                .unwrap_or(email);
            let password = keychain::get_credential(&imap::password_key("imap", email))?
                .ok_or_else(|| {
                    AppError::AuthFailed(format!(
                        "No stored password for {} — reconnect the account.",
                        email
                    ))
                })?;
            Ok(SmtpTransport {
                host,
                port,
                implicit_tls: security.is_implicit(),
                auth: SmtpAuth::Plain { username: username.to_string(), password },
            })
        }
        other => Err(AppError::General(format!("Unsupported SMTP provider: {}", other))),
    }
}

#[cfg(test)]
mod tests {
    use super::strip_message_id_brackets;

    #[test]
    fn strips_single_brackets() {
        // The normal case: a parser-stored Message-ID arrives bracketed.
        assert_eq!(strip_message_id_brackets("<abc@example.com>"), "abc@example.com");
    }

    #[test]
    fn strips_doubled_brackets() {
        // Defensive: an already-malformed `<<id>>` is normalized too.
        assert_eq!(strip_message_id_brackets("<<abc@example.com>>"), "abc@example.com");
    }

    #[test]
    fn leaves_bare_id_untouched() {
        assert_eq!(strip_message_id_brackets("abc@example.com"), "abc@example.com");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(strip_message_id_brackets("  <abc@example.com>  "), "abc@example.com");
    }
}
