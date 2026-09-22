use crate::db;
use crate::email::{autoconfig, imap, oauth2, providers, smtp};
use crate::error::AppError;
use crate::keychain;
use crate::AppState;
use crate::LockExt;
use tauri::Emitter;
use tauri::Manager;

/// The OAuth flows want an [`cxmail_core::AppCtx`], not an `AppHandle` — they
/// live in `cxmail-email`, which does not link Tauri. Failing here means state
/// is not managed yet, which cannot happen from an IPC command.
fn ctx(app: &tauri::AppHandle) -> Result<cxmail_core::AppCtx, AppError> {
    crate::app_ctx(app).ok_or_else(|| AppError::General("App state unavailable".to_string()))
}

/// Start the OAuth2 flow for a provider. Returns the auth URL.
#[tauri::command]
pub async fn start_oauth2(app: tauri::AppHandle, provider: String) -> Result<String, AppError> {
    match provider.as_str() {
        "gmail" => {
            let (auth_url, _port) = oauth2::start_gmail_oauth(ctx(&app)?)?;

            // Open the auth URL in the default browser
            #[allow(deprecated)]
            tauri_plugin_shell::ShellExt::shell(&app)
                .open(&auth_url, None)
                .map_err(|e| AppError::OAuth2(format!("Failed to open browser: {}", e)))?;

            Ok(auth_url)
        }
        "outlook" => {
            let (auth_url, _port) = oauth2::start_outlook_oauth(ctx(&app)?)?;

            #[allow(deprecated)]
            tauri_plugin_shell::ShellExt::shell(&app)
                .open(&auth_url, None)
                .map_err(|e| AppError::OAuth2(format!("Failed to open browser: {}", e)))?;

            Ok(auth_url)
        }
        _ => Err(AppError::OAuth2(format!(
            "Unsupported provider: {}",
            provider
        ))),
    }
}

#[tauri::command]
pub async fn start_calendar_oauth(
    app: tauri::AppHandle,
    email: String,
) -> Result<String, AppError> {
    let normalized = email.trim().to_ascii_lowercase();
    if normalized.is_empty() || !normalized.contains('@') {
        return Err(AppError::OAuth2(
            "A valid account email is required".to_string(),
        ));
    }
    {
        let state = app.state::<AppState>();
        let conn = state.db.safe_lock();
        let account = db::accounts::list(&conn)?
            .into_iter()
            .find(|account| account.email.eq_ignore_ascii_case(&normalized))
            .ok_or_else(|| AppError::NotFound("Gmail account not found".to_string()))?;
        if account.provider != "gmail" {
            return Err(AppError::OAuth2(
                "Google Calendar is available only for Gmail accounts".to_string(),
            ));
        }
    }
    let (auth_url, _) = oauth2::start_gcal_oauth(ctx(&app)?, normalized)?;
    #[allow(deprecated)]
    tauri_plugin_shell::ShellExt::shell(&app)
        .open(&auth_url, None)
        .map_err(|e| AppError::OAuth2(format!("Failed to open browser: {e}")))?;
    Ok(auth_url)
}

#[tauri::command]
pub async fn calendar_connection_status(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<bool, AppError> {
    let email = {
        let conn = state.db.safe_lock();
        let account = db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
        if account.provider != "gmail" {
            return Ok(false);
        }
        account.email
    };
    oauth2::is_gcal_connected(&email)
}

/// Discover server settings for an email address, without connecting.
///
/// Safe to call speculatively as the user types: it never sends a credential,
/// never opens a mail connection, and the offline tiers answer known domains
/// with no network access at all. Returns `None` when nothing is found, which
/// is the signal for the form to ask for settings manually.
#[tauri::command]
pub async fn discover_mail_config(
    email: String,
) -> Result<Option<autoconfig::DiscoveredConfig>, AppError> {
    let email = email.trim().to_string();
    if !email.contains('@') {
        return Ok(None);
    }
    Ok(autoconfig::discover(&email).await)
}

/// Server settings for a generic IMAP account, as the setup form sends them.
///
/// `preset_id` is a convenience: when present and known, its host/port/security
/// fill in anything the form left blank, so the two-field preset path and the
/// full manual path are one command.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImapAccountSettings {
    pub preset_id: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i32>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub smtp_security: Option<String>,
    pub imap_username: Option<String>,
    pub smtp_username: Option<String>,
}

/// Blank-to-`None`, so an empty form field never becomes an empty hostname.
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Resolve the settings the form sent into a complete `Account`, filling gaps
/// from the preset (explicit form values always win).
fn build_imap_account(
    email: &str,
    settings: ImapAccountSettings,
) -> Result<db::accounts::Account, AppError> {
    let preset = settings
        .preset_id
        .as_deref()
        .and_then(providers::preset_by_id)
        .or_else(|| providers::preset_for_email(email));

    let imap_host = non_empty(settings.imap_host)
        .or_else(|| preset.map(|p| p.imap_host.to_string()))
        .ok_or_else(|| AppError::General("IMAP server address is required.".to_string()))?;
    let smtp_host = non_empty(settings.smtp_host)
        .or_else(|| preset.map(|p| p.smtp_host.to_string()))
        .ok_or_else(|| AppError::General("SMTP server address is required.".to_string()))?;
    let imap_host = providers::validate_host(&imap_host)?;
    let smtp_host = providers::validate_host(&smtp_host)?;

    let imap_port = settings
        .imap_port
        .or_else(|| preset.map(|p| p.imap_port as i32))
        .unwrap_or(993);
    let smtp_port = settings
        .smtp_port
        .or_else(|| preset.map(|p| p.smtp_port as i32))
        .unwrap_or(587);
    // Range-check now rather than at connect time, so a bad port is a setup
    // error the user can see next to the field they typed it in.
    let imap_port_u16 = providers::port_from_i32(imap_port, "IMAP")?;
    let smtp_port_u16 = providers::port_from_i32(smtp_port, "SMTP")?;

    // Explicit choice → preset → whatever the port unambiguously implies. If
    // none of those answer, refuse: guessing picks the handshake.
    let imap_security = match non_empty(settings.imap_security) {
        Some(s) => {
            providers::TlsMode::parse(&s)?;
            s
        }
        None => resolve_security(preset.map(|p| p.imap_security), imap_port_u16, "IMAP")?,
    };
    let smtp_security = match non_empty(settings.smtp_security) {
        Some(s) => {
            providers::TlsMode::parse(&s)?;
            s
        }
        None => resolve_security(preset.map(|p| p.smtp_security), smtp_port_u16, "SMTP")?,
    };

    Ok(db::accounts::Account {
        id: uuid::Uuid::new_v4().to_string(),
        email: email.to_string(),
        display_name: None,
        provider: "imap".to_string(),
        imap_host,
        imap_port,
        smtp_host,
        smtp_port,
        imap_security,
        smtp_security,
        imap_username: non_empty(settings.imap_username),
        smtp_username: non_empty(settings.smtp_username),
        color: Some("#8e8e93".to_string()),
        is_active: true,
        sort_order: 0,
        group_name: None,
        notify_enabled: true,
        track_opens_enabled: false,
        hidden_from_aggregates: false,
        triage_enabled: false,
    })
}

fn resolve_security(
    preset: Option<&'static str>,
    port: u16,
    label: &str,
) -> Result<String, AppError> {
    if let Some(s) = preset {
        return Ok(s.to_string());
    }
    match providers::security_for_port(port)? {
        Some(providers::TlsMode::Implicit) => Ok("implicit".to_string()),
        Some(providers::TlsMode::StartTls) => Ok("starttls".to_string()),
        None => Err(AppError::General(format!(
            "Choose the {} encryption for port {} — CXMail will not guess it.",
            label, port
        ))),
    }
}

/// Add a generic IMAP/SMTP account using email + password.
///
/// Validates **both** transports before saving, and deletes the stored
/// credential if either fails. `add_icloud_account` checks IMAP only, which
/// can "verify" an account that cannot send — the two stacks do not share a
/// trust store (IMAP is native-tls, SMTP is rustls + webpki-roots), so an IMAP
/// success is not evidence about SMTP.
#[tauri::command]
pub async fn add_imap_account(
    app: tauri::AppHandle,
    email: String,
    password: String,
    settings: ImapAccountSettings,
) -> Result<(), AppError> {
    let email = email.trim().to_string();
    if !email.contains('@') {
        return Err(AppError::General(format!("{:?} is not an email address.", email)));
    }
    if password.is_empty() {
        return Err(AppError::General("Password is required.".to_string()));
    }
    log::info!("Adding generic IMAP account for {}", email);

    // Build the account BEFORE storing anything: a settings error must not
    // leave a password in the keychain for an account that was never created.
    let account = build_imap_account(&email, settings)?;
    let key = imap::password_key("imap", &email);

    keychain::store_credential(&key, &password)?;

    let verify = verify_imap_account(&account).await;
    if let Err(e) = verify {
        let _ = keychain::delete_credential(&key);
        return Err(e);
    }

    let effective_id = {
        let state = app.state::<AppState>();
        let conn = state.db.safe_lock();
        db::accounts::upsert(&conn, &account)?
    };

    log::info!("IMAP account upserted for {} (id={})", email, effective_id);

    // Emit with the EFFECTIVE id — see the Gmail/Outlook exchange functions
    // in email/oauth2.rs for why this matters on a re-auth.
    let mut emitted_account = account.clone();
    emitted_account.id = effective_id;
    let _ = app.emit("account-added", &emitted_account);

    Ok(())
}

/// Prove the credentials work on BOTH transports before the account is saved.
async fn verify_imap_account(account: &db::accounts::Account) -> Result<(), AppError> {
    match imap::connect_for_account(account).await {
        Ok(session) => {
            let _ = imap::disconnect(session, &account.email).await;
            log::info!("IMAP credentials verified for {}", account.email);
        }
        Err(e) => {
            return Err(AppError::AuthFailed(format!(
                "IMAP connection to {}:{} failed: {}. Check the server address, port and password.",
                account.imap_host, account.imap_port, e
            )));
        }
    }

    // `connect()` performs EHLO + STARTTLS + AUTH and stops there — no message
    // is sent — so this is a complete credential check, not a probe.
    smtp::verify_transport(account).await.map_err(|e| {
        AppError::AuthFailed(format!(
            "SMTP connection to {}:{} failed: {}. IMAP worked, so the password is right — check the outgoing server settings.",
            account.smtp_host, account.smtp_port, e
        ))
    })
}

/// Add an iCloud account using email + app-specific password.
/// Validates credentials by attempting an IMAP connection before saving.
#[tauri::command]
pub async fn add_icloud_account(
    app: tauri::AppHandle,
    email: String,
    password: String,
) -> Result<(), AppError> {
    log::info!("Adding iCloud account for {}", email);

    // Store password in Keychain first so connect_icloud can find it
    keychain::store_credential(&format!("icloud:{}:password", email), &password)?;

    // Validate credentials by connecting to IMAP
    match imap::connect_icloud(&email).await {
        Ok(session) => {
            let _ = imap::disconnect(session, &email).await;
            log::info!("iCloud IMAP credentials verified for {}", email);
        }
        Err(e) => {
            // Clean up the stored password on failure
            let _ = keychain::delete_credential(&format!("icloud:{}:password", email));
            return Err(AppError::AuthFailed(format!(
                "iCloud authentication failed: {}. Check your email and app-specific password.",
                e
            )));
        }
    }

    // Create account in database
    let account_id = uuid::Uuid::new_v4().to_string();
    let account = db::accounts::Account {
        id: account_id.clone(),
        email: email.clone(),
        display_name: None,
        provider: "icloud".to_string(),
        imap_host: "imap.mail.me.com".to_string(),
        imap_port: 993,
        smtp_host: "smtp.mail.me.com".to_string(),
        smtp_port: 587,
        imap_security: "implicit".to_string(),
        smtp_security: "starttls".to_string(),
        imap_username: None,
        smtp_username: None,
        color: Some("#a0aec0".to_string()),
        is_active: true,
        sort_order: 0,
        group_name: None,
        notify_enabled: true,
        track_opens_enabled: false,
        hidden_from_aggregates: false,
        triage_enabled: false,
    };

    let effective_id = {
        let state = app.state::<AppState>();
        let conn = state.db.safe_lock();
        db::accounts::upsert(&conn, &account)?
    };

    log::info!(
        "iCloud account upserted for {} (id={})",
        email,
        effective_id
    );

    // Emit with the EFFECTIVE id — see the Gmail/Outlook exchange functions
    // in email/oauth2.rs for why this matters on a re-auth.
    let mut emitted_account = account.clone();
    emitted_account.id = effective_id;
    let _ = app.emit("account-added", &emitted_account);

    Ok(())
}

#[cfg(test)]
mod imap_account_tests {
    use super::*;

    fn blank() -> ImapAccountSettings {
        ImapAccountSettings {
            preset_id: None,
            imap_host: None,
            imap_port: None,
            imap_security: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            imap_username: None,
            smtp_username: None,
        }
    }

    /// The two-field path: the user types an address and a password, nothing
    /// else. Everything below has to come from the preset.
    #[test]
    fn a_known_domain_needs_no_server_settings_at_all() {
        let a = build_imap_account("chris@fastmail.com", blank()).unwrap();
        assert_eq!(a.provider, "imap");
        assert_eq!(a.imap_host, "imap.fastmail.com");
        assert_eq!(a.imap_port, 993);
        assert_eq!(a.imap_security, "implicit");
        assert_eq!(a.smtp_host, "smtp.fastmail.com");
        assert_eq!(a.smtp_port, 465);
        assert_eq!(a.smtp_security, "implicit");
        // No custom login — authenticate as the address itself.
        assert_eq!(a.imap_username, None);
        assert_eq!(a.smtp_username, None);
    }

    /// An app-password Gmail account must land on exactly the settings the
    /// OAuth path uses, or "sends over OAuth, not over app password".
    #[test]
    fn gmail_by_app_password_matches_the_oauth_gmail_settings() {
        let a = build_imap_account("chris@gmail.com", blank()).unwrap();
        assert_eq!(a.imap_host, "imap.gmail.com");
        assert_eq!(a.imap_port, 993);
        assert_eq!(a.smtp_host, "smtp.gmail.com");
        assert_eq!(a.smtp_port, 587);
        assert_eq!(a.smtp_security, "starttls");
        // Still the generic provider — the OAuth code paths must not claim it.
        assert_eq!(a.provider, "imap");
    }

    #[test]
    fn an_explicit_preset_id_beats_the_domain_guess() {
        let s = ImapAccountSettings { preset_id: Some("zoho".into()), ..blank() };
        // A Zoho-hosted custom domain: the address says nothing, the choice does.
        let a = build_imap_account("chris@cxventures.io", s).unwrap();
        assert_eq!(a.imap_host, "imap.zoho.com");
    }

    #[test]
    fn typed_settings_beat_the_preset() {
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(993),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            imap_username: Some("chris.login".into()),
            ..blank()
        };
        // gmail.com would otherwise preset every one of these.
        let a = build_imap_account("chris@gmail.com", s).unwrap();
        assert_eq!(a.imap_host, "mail.example.com");
        assert_eq!(a.smtp_host, "smtp.example.com");
        assert_eq!(a.imap_username, Some("chris.login".to_string()));
        // Security still resolves from the ports, which are unambiguous.
        assert_eq!(a.imap_security, "implicit");
        assert_eq!(a.smtp_security, "starttls");
    }

    #[test]
    fn an_unknown_domain_with_no_settings_is_refused_not_guessed() {
        let err = build_imap_account("chris@cxventures.io", blank()).unwrap_err();
        assert!(err.to_string().contains("IMAP server address is required"), "{err}");
    }

    /// Blank form fields must not become empty hostnames or empty logins —
    /// an empty username would authenticate as "" instead of the address.
    #[test]
    fn blank_strings_are_treated_as_absent() {
        let s = ImapAccountSettings {
            imap_host: Some("   ".into()),
            imap_username: Some("".into()),
            smtp_username: Some("  ".into()),
            ..blank()
        };
        let a = build_imap_account("chris@fastmail.com", s).unwrap();
        assert_eq!(a.imap_host, "imap.fastmail.com");
        assert_eq!(a.imap_username, None);
        assert_eq!(a.smtp_username, None);
    }

    #[test]
    fn plaintext_and_pop3_ports_are_refused_at_setup() {
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(110),
            smtp_host: Some("smtp.example.com".into()),
            ..blank()
        };
        let err = build_imap_account("chris@example.com", s).unwrap_err();
        assert!(err.to_string().contains("POP3"), "{err}");

        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(993),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(25),
            ..blank()
        };
        let err = build_imap_account("chris@example.com", s).unwrap_err();
        assert!(err.to_string().contains("relay"), "{err}");
    }

    /// An unusual port carries no meaning, and there is no preset to fall back
    /// on — refusing beats picking a handshake on the user's behalf.
    #[test]
    fn an_ambiguous_port_demands_an_explicit_security_choice() {
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(9930),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            ..blank()
        };
        let err = build_imap_account("chris@example.com", s).unwrap_err();
        assert!(err.to_string().contains("Choose the IMAP encryption"), "{err}");

        // ...and supplying it resolves the account.
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(9930),
            imap_security: Some("implicit".into()),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            ..blank()
        };
        let a = build_imap_account("chris@example.com", s).unwrap();
        assert_eq!(a.imap_security, "implicit");
    }

    #[test]
    fn a_bogus_security_value_is_rejected_rather_than_stored() {
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_security: Some("none".into()),
            smtp_host: Some("smtp.example.com".into()),
            ..blank()
        };
        // "none" must never reach the accounts row — TlsMode cannot express it.
        assert!(build_imap_account("chris@example.com", s).is_err());
    }

    #[test]
    fn a_pasted_url_in_the_host_field_is_rejected() {
        let s = ImapAccountSettings {
            imap_host: Some("imaps://mail.example.com".into()),
            smtp_host: Some("smtp.example.com".into()),
            ..blank()
        };
        let err = build_imap_account("chris@example.com", s).unwrap_err();
        assert!(err.to_string().contains("not a URL"), "{err}");
    }

    #[test]
    fn out_of_range_ports_are_refused_instead_of_wrapping() {
        let s = ImapAccountSettings {
            imap_host: Some("mail.example.com".into()),
            imap_port: Some(70000),
            smtp_host: Some("smtp.example.com".into()),
            ..blank()
        };
        let err = build_imap_account("chris@example.com", s).unwrap_err();
        assert!(err.to_string().contains("out of range"), "{err}");
    }
}
