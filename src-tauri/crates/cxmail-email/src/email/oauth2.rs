use crate::db;
use crate::error::AppError;
use crate::keychain;
use crate::secrets;
use crate::LockExt;
use chrono::Utc;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use cxmail_core::AppCtx;

/// Held in memory during the OAuth2 flow (between start and callback).
struct PendingOAuth {
    pkce_verifier: PkceCodeVerifier,
    csrf_state: CsrfToken,
    redirect_port: u16,
}

static PENDING: Mutex<Option<PendingOAuth>> = Mutex::new(None);

struct PendingGcalOAuth {
    oauth: PendingOAuth,
    email: String,
}

static PENDING_GCAL: Mutex<Option<PendingGcalOAuth>> = Mutex::new(None);

/// Write access to events on every calendar. Sensitive scope; covers every
/// `events.*` call CXMail makes (list/get/insert/patch/delete).
pub(crate) const GCAL_EVENTS_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";

/// Read access to the calendar *list*. Non-sensitive, and NOT implied by
/// `calendar.events` — `calendarList.list` accepts only calendar,
/// calendar.calendarlist, calendar.calendarlist.readonly, or calendar.readonly.
/// Without it the very first call of every sync 403s.
pub(crate) const GCAL_CALENDARLIST_SCOPE: &str =
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly";

/// Which of the scopes CXMail requires are absent from a space-separated grant.
///
/// Matching is whitespace-tokenised on purpose: a substring test for
/// "calendar.events" is also satisfied by `calendar.events.readonly`, which
/// cannot write an event, so the app would report itself connected and then
/// fail on the first insert.
pub(crate) fn missing_gcal_scopes(granted: &str) -> Vec<&'static str> {
    let granted: Vec<&str> = granted.split_whitespace().collect();
    [GCAL_EVENTS_SCOPE, GCAL_CALENDARLIST_SCOPE]
        .into_iter()
        .filter(|required| !granted.contains(required))
        .collect()
}

/// Build an HTTP client with a sensible timeout for OAuth token operations.
/// Prevents indefinite hangs when the network is unavailable after restart.
fn oauth_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client")
}

/// Start the OAuth2 flow. Returns the authorization URL to open in a browser.
/// `ctx` is used to emit events back to the frontend when auth completes, and
/// to reach the shared database once an account exists.
pub fn start_gmail_oauth(ctx: AppCtx) -> Result<(String, u16), AppError> {
    secrets::require_client_id("gmail")?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::OAuth2(format!("Failed to bind loopback server: {}", e)))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::OAuth2(format!("Failed to get local addr: {}", e)))?
        .port();

    let redirect_url = format!("http://localhost:{}", port);

    let client = build_oauth_client(Some(&redirect_url))?;

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("https://mail.google.com/".to_string()))
        .add_scope(Scope::new(
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
        ))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let mut pending = PENDING.safe_lock();
    *pending = Some(PendingOAuth {
        pkce_verifier,
        csrf_state,
        redirect_port: port,
    });

    // Spawn a thread to wait for the callback
    std::thread::spawn(
        move || match wait_for_callback(listener, ctx.clone()) {
            Ok(()) => {
                log::info!("OAuth callback completed successfully");
            }
            Err(e) => {
                log::error!("OAuth callback error: {}", e);
                ctx.emit("oauth-error", serde_json::json!(e.to_string()));
            }
        },
    );

    Ok((auth_url.to_string(), port))
}

fn build_oauth_client(redirect_url: Option<&str>) -> Result<BasicClient, AppError> {
    let mut client = BasicClient::new(
        ClientId::new(secrets::GOOGLE_CLIENT_ID.to_string()),
        None,
        AuthUrl::new(secrets::GOOGLE_AUTH_URI.to_string())
            .map_err(|e| AppError::OAuth2(format!("Invalid auth URL: {}", e)))?,
        Some(
            TokenUrl::new(secrets::GOOGLE_TOKEN_URI.to_string())
                .map_err(|e| AppError::OAuth2(format!("Invalid token URL: {}", e)))?,
        ),
    );
    if let Some(url) = redirect_url {
        client = client.set_redirect_uri(
            RedirectUrl::new(url.to_string())
                .map_err(|e| AppError::OAuth2(format!("Invalid redirect URL: {}", e)))?,
        );
    }
    Ok(client)
}

fn wait_for_callback(listener: TcpListener, ctx: AppCtx) -> Result<(), AppError> {
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| AppError::OAuth2(format!("Failed to accept connection: {}", e)))?;

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| AppError::OAuth2(format!("Failed to read request: {}", e)))?;

    let url_part = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();

    // Send response to browser
    let response_body = r#"<html><body style="font-family:-apple-system,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;background:#1e1e1e;color:#fff;"><div style="text-align:center"><h2>CXMail</h2><p style="color:#999">Authentication successful! You can close this tab.</p></div></body></html>"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());

    // Extract code and state
    let query = url_part.split('?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;

    for param in query.split('&') {
        let mut kv = param.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("code"), Some(v)) => code = Some(url_decode(v)),
            (Some("state"), Some(v)) => state = Some(url_decode(v)),
            _ => {}
        }
    }

    if let (Some(code), Some(state)) = (code, state) {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| AppError::OAuth2(format!("Failed to create runtime: {}", e)))?;
        rt.block_on(exchange_code_and_create_account(code, state, ctx))?;
    } else {
        log::error!("OAuth callback missing code or state: {}", url_part);
        return Err(AppError::OAuth2(
            "Missing code or state in callback".to_string(),
        ));
    }

    Ok(())
}

async fn exchange_code_and_create_account(
    code: String,
    state: String,
    ctx: AppCtx,
) -> Result<(), AppError> {
    let pending = {
        let mut lock = PENDING.safe_lock();
        lock.take()
    };

    let pending = pending.ok_or_else(|| AppError::OAuth2("No pending OAuth flow".to_string()))?;

    // Verify CSRF state
    if state != *pending.csrf_state.secret() {
        return Err(AppError::OAuth2("CSRF state mismatch".to_string()));
    }

    let redirect_url = format!("http://localhost:{}", pending.redirect_port);

    // Use direct reqwest call for better error visibility
    let http_client = oauth_http_client();
    let token_response = http_client
        .post(secrets::GOOGLE_TOKEN_URI)
        .form(&[
            ("code", code.as_str()),
            ("client_id", secrets::GOOGLE_CLIENT_ID),
            ("client_secret", secrets::GOOGLE_CLIENT_SECRET),
            ("redirect_uri", redirect_url.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", pending.pkce_verifier.secret()),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Token request failed: {}", e)))?;

    let status = token_response.status();
    let body_text = token_response.text().await.unwrap_or_default();

    if !status.is_success() {
        log::error!("Token exchange failed ({}): {}", status, body_text);
        return Err(AppError::OAuth2(format!(
            "Token exchange failed ({}): {}",
            status, body_text
        )));
    }

    let token_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::OAuth2(format!("Failed to parse token response: {}", e)))?;

    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No access_token in response".to_string()))?
        .to_string();
    let refresh_token = token_json["refresh_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No refresh_token in response".to_string()))?
        .to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    // Fetch email from Google userinfo
    let email = fetch_gmail_email(&access_token).await?;

    // Store tokens in Keychain
    let key_prefix = format!("gmail:{}", email);
    keychain::store_credential(&format!("{}:access", key_prefix), &access_token)?;
    keychain::store_credential(&format!("{}:refresh", key_prefix), &refresh_token)?;
    keychain::store_credential(&format!("{}:expires", key_prefix), &expires_at.to_string())?;

    log::info!("OAuth2 tokens stored for {}", email);

    // Create account in database
    let account_id = uuid::Uuid::new_v4().to_string();
    let account = db::accounts::Account {
        id: account_id.clone(),
        email: email.clone(),
        display_name: None,
        provider: "gmail".to_string(),
        imap_host: "imap.gmail.com".to_string(),
        imap_port: 993,
        smtp_host: "smtp.gmail.com".to_string(),
        smtp_port: 587,
        imap_security: "implicit".to_string(),
        smtp_security: "starttls".to_string(),
        imap_username: None,
        smtp_username: None,
        color: Some("#0a84ff".to_string()),
        is_active: true,
        sort_order: 0,
        group_name: None,
        notify_enabled: true,
        track_opens_enabled: false,
        hidden_from_aggregates: false,
        triage_enabled: false,
    };

    let effective_id = {
        let conn = ctx.db().safe_lock();
        db::accounts::upsert(&conn, &account)?
    };

    log::info!("Account upserted for {} (id={})", email, effective_id);

    // Emit with the EFFECTIVE id — on a re-auth of an already-connected
    // email this is the existing row's id, not the fresh uuid generated
    // above, so the frontend store's upsert (by id) targets the right row.
    let mut emitted_account = account.clone();
    emitted_account.id = effective_id;
    ctx.emit_value("account-added", &emitted_account);

    Ok(())
}

async fn fetch_gmail_email(access_token: &str) -> Result<String, AppError> {
    let client = oauth_http_client();
    let resp = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Failed to fetch userinfo: {}", e)))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::OAuth2(format!("Failed to parse userinfo: {}", e)))?;

    body["email"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::OAuth2("No email in userinfo response".to_string()))
}

/// Refresh an access token using the stored refresh token.
pub async fn refresh_access_token(email: &str) -> Result<String, AppError> {
    log::info!("Refreshing access token for {}", email);
    let key_prefix = format!("gmail:{}", email);
    let refresh_token = keychain::get_credential(&format!("{}:refresh", key_prefix))?
        .ok_or_else(|| AppError::OAuth2("No refresh token found".to_string()))?;

    let http_client = oauth_http_client();
    let resp = http_client
        .post(secrets::GOOGLE_TOKEN_URI)
        .form(&[
            ("refresh_token", refresh_token.as_str()),
            ("client_id", secrets::GOOGLE_CLIENT_ID),
            ("client_secret", secrets::GOOGLE_CLIENT_SECRET),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Token refresh request failed: {}", e)))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        log::error!("Token refresh failed ({}): {}", status, body_text);
        if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if matches!(
                err_json["error"].as_str(),
                Some("invalid_grant") | Some("invalid_client")
            ) {
                return Err(AppError::ReauthRequired(email.to_string()));
            }
        }
        return Err(AppError::OAuth2(format!(
            "Token refresh failed ({}): {}",
            status, body_text
        )));
    }

    let token_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::OAuth2(format!("Failed to parse refresh response: {}", e)))?;

    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No access_token in refresh response".to_string()))?
        .to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    keychain::store_credential(&format!("{}:access", key_prefix), &access_token)?;
    keychain::store_credential(&format!("{}:expires", key_prefix), &expires_at.to_string())?;

    log::info!("Access token refreshed for {}", email);
    Ok(access_token)
}

/// Get a valid access token, refreshing if expired.
pub async fn get_valid_access_token(email: &str) -> Result<String, AppError> {
    log::info!("Getting valid access token for {}", email);
    let key_prefix = format!("gmail:{}", email);

    let access_token = keychain::get_credential(&format!("{}:access", key_prefix))?
        .ok_or_else(|| AppError::OAuth2("No access token found".to_string()))?;

    let expires_at_str = keychain::get_credential(&format!("{}:expires", key_prefix))?
        .unwrap_or_else(|| "0".to_string());
    let expires_at: i64 = expires_at_str.parse().unwrap_or(0);

    let now = chrono::Utc::now().timestamp();
    log::info!(
        "Token expires_at={}, now={}, diff={}s",
        expires_at,
        now,
        expires_at - now
    );
    if now >= expires_at - 300 {
        return refresh_access_token(email).await;
    }

    Ok(access_token)
}

// ─── Google Calendar incremental OAuth2 ──────────────────────────────

/// Start a separate, incremental Calendar grant. Mail credentials are never
/// read or overwritten by this flow.
pub fn start_gcal_oauth(
    ctx: AppCtx,
    email: String,
) -> Result<(String, u16), AppError> {
    secrets::require_client_id("gmail")?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::OAuth2(format!("Failed to bind Calendar callback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::OAuth2(format!("Failed to get Calendar callback address: {e}")))?
        .port();
    let redirect_url = format!("http://localhost:{port}");
    let client = build_oauth_client(Some(&redirect_url))?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(GCAL_EVENTS_SCOPE.to_string()))
        // calendar.events does NOT authorize calendarList.list, which is the
        // first call every sync makes (gcal_sync::sync_account). Per Google's
        // discovery doc that method needs one of calendar / calendar.calendarlist
        // / calendar.calendarlist.readonly / calendar.readonly. This is the
        // least-privileged of those, and it is a non-sensitive scope, so it adds
        // nothing to the verification burden.
        .add_scope(Scope::new(GCAL_CALENDARLIST_SCOPE.to_string()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .add_extra_param("include_granted_scopes", "true")
        .add_extra_param("login_hint", email.clone())
        .set_pkce_challenge(pkce_challenge)
        .url();

    *PENDING_GCAL.safe_lock() = Some(PendingGcalOAuth {
        oauth: PendingOAuth {
            pkce_verifier,
            csrf_state,
            redirect_port: port,
        },
        email,
    });

    std::thread::spawn(move || {
        if let Err(error) = wait_for_gcal_callback(listener, ctx.clone()) {
            log::error!("Google Calendar OAuth callback error: {error}");
            ctx.emit("calendar-oauth-error", serde_json::json!(error.to_string()));
        }
    });
    Ok((auth_url.to_string(), port))
}

fn wait_for_gcal_callback(
    listener: TcpListener,
    ctx: AppCtx,
) -> Result<(), AppError> {
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| AppError::OAuth2(format!("Failed to accept Calendar callback: {e}")))?;
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| AppError::OAuth2(format!("Failed to read Calendar callback: {e}")))?;
    let url_part = request_line.split_whitespace().nth(1).unwrap_or("");
    let body = r#"<html><body style="font-family:-apple-system,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;background:#1e1e1e;color:#fff"><div style="text-align:center"><h2>CXMail Calendar</h2><p style="color:#999">Calendar connected. You can close this tab.</p></div></body></html>"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());

    let query = url_part.split('?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for param in query.split('&') {
        let mut pair = param.splitn(2, '=');
        match (pair.next(), pair.next()) {
            (Some("code"), Some(value)) => code = Some(url_decode(value)),
            (Some("state"), Some(value)) => state = Some(url_decode(value)),
            _ => {}
        }
    }
    let (code, state) = match (code, state) {
        (Some(code), Some(state)) => (code, state),
        _ => {
            return Err(AppError::OAuth2(
                "Calendar callback missing code or state".to_string(),
            ))
        }
    };
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| AppError::OAuth2(format!("Failed to create OAuth runtime: {e}")))?;
    runtime.block_on(exchange_gcal_code(code, state, ctx))
}

async fn exchange_gcal_code(
    code: String,
    state: String,
    ctx: AppCtx,
) -> Result<(), AppError> {
    let pending = PENDING_GCAL
        .safe_lock()
        .take()
        .ok_or_else(|| AppError::OAuth2("No pending Calendar OAuth flow".to_string()))?;
    if state != *pending.oauth.csrf_state.secret() {
        return Err(AppError::OAuth2(
            "Calendar OAuth CSRF state mismatch".to_string(),
        ));
    }
    let redirect_url = format!("http://localhost:{}", pending.oauth.redirect_port);
    let response = oauth_http_client()
        .post(secrets::GOOGLE_TOKEN_URI)
        .form(&[
            ("code", code.as_str()),
            ("client_id", secrets::GOOGLE_CLIENT_ID),
            ("client_secret", secrets::GOOGLE_CLIENT_SECRET),
            ("redirect_uri", redirect_url.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", pending.oauth.pkce_verifier.secret()),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Calendar token request failed: {e}")))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::OAuth2(format!(
            "Calendar token exchange failed ({status}): {body}"
        )));
    }
    let token: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::OAuth2(format!("Invalid Calendar token response: {e}")))?;
    let access = token["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("Calendar response omitted access_token".to_string()))?;
    let refresh = token["refresh_token"].as_str().ok_or_else(|| {
        AppError::OAuth2(
            "Google omitted the Calendar refresh token; reconnect with consent".to_string(),
        )
    })?;
    let expires = Utc::now().timestamp() + token["expires_in"].as_i64().unwrap_or(3600);
    // Never assume the scope we asked for is the scope we got. Google silently
    // drops any scope that is not registered on the consent screen's Data Access
    // list, and it may omit `scope` from the response entirely. The previous
    // `unwrap_or(".../calendar.events")` fabricated the very grant this value
    // exists to verify: a scope-less token was stored as fully connected, so
    // `is_gcal_connected` returned true and every sync tick marched into a
    // guaranteed 403 that only ever surfaced as a log warning.
    let scopes = token["scope"].as_str().unwrap_or_default();
    let missing = missing_gcal_scopes(scopes);
    if !missing.is_empty() {
        return Err(AppError::OAuth2(format!(
            "Google did not grant the scopes CXMail needs: {}. Granted: {}. \
             Add the missing scopes under Data Access in the Google Cloud console \
             (project `cxmail`), press Save, then connect again.",
            missing.join(", "),
            if scopes.is_empty() { "none" } else { scopes },
        )));
    }
    let prefix = format!("gmail:{}:calendar", pending.email);
    keychain::store_credential(&format!("{prefix}:access"), access)?;
    keychain::store_credential(&format!("{prefix}:refresh"), refresh)?;
    keychain::store_credential(&format!("{prefix}:expires"), &expires.to_string())?;
    keychain::store_credential(&format!("{prefix}:scopes"), scopes)?;
    ctx.emit_value("calendar-connected", &pending.email);
    Ok(())
}

pub fn is_gcal_connected(email: &str) -> Result<bool, AppError> {
    let prefix = format!("gmail:{email}:calendar");
    let refresh = keychain::get_credential(&format!("{prefix}:refresh"))?;
    let scopes = keychain::get_credential(&format!("{prefix}:scopes"))?;
    // Split on whitespace rather than substring-matching: `contains("calendar.events")`
    // is also satisfied by `calendar.events.readonly`, which cannot write an event.
    Ok(refresh.is_some()
        && scopes
            .as_deref()
            .is_some_and(|value| missing_gcal_scopes(value).is_empty()))
}

/// Mark a Calendar grant unusable after the API rejects it for scope reasons.
///
/// Dropping the `:scopes` key makes [`is_gcal_connected`] report false, which
/// takes the account out of the sync loop and flips the UI back to "Connect".
/// The refresh token is deliberately left in place — the grant is not revoked,
/// it is under-scoped, and reconnecting is what fixes it.
pub fn invalidate_gcal_connection(email: &str) -> Result<(), AppError> {
    keychain::delete_credential(&format!("gmail:{email}:calendar:scopes"))
}


pub async fn refresh_gcal_access_token(email: &str) -> Result<String, AppError> {
    let prefix = format!("gmail:{email}:calendar");
    let refresh = keychain::get_credential(&format!("{prefix}:refresh"))?
        .ok_or_else(|| AppError::OAuth2("No Calendar refresh token found".to_string()))?;
    let response = oauth_http_client()
        .post(secrets::GOOGLE_TOKEN_URI)
        .form(&[
            ("refresh_token", refresh.as_str()),
            ("client_id", secrets::GOOGLE_CLIENT_ID),
            ("client_secret", secrets::GOOGLE_CLIENT_SECRET),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Calendar token refresh failed: {e}")))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        if body.contains("invalid_grant") || body.contains("invalid_client") {
            return Err(AppError::ReauthRequired(email.to_string()));
        }
        return Err(AppError::OAuth2(format!(
            "Calendar token refresh failed ({status}): {body}"
        )));
    }
    let token: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::OAuth2(format!("Invalid Calendar refresh response: {e}")))?;
    let access = token["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("Calendar refresh omitted access_token".to_string()))?
        .to_string();
    let expires = Utc::now().timestamp() + token["expires_in"].as_i64().unwrap_or(3600);
    keychain::store_credential(&format!("{prefix}:access"), &access)?;
    keychain::store_credential(&format!("{prefix}:expires"), &expires.to_string())?;
    Ok(access)
}

pub async fn get_valid_gcal_access_token(email: &str) -> Result<String, AppError> {
    let prefix = format!("gmail:{email}:calendar");
    let access = keychain::get_credential(&format!("{prefix}:access"))?
        .ok_or_else(|| AppError::OAuth2("Calendar is not connected".to_string()))?;
    let expires = keychain::get_credential(&format!("{prefix}:expires"))?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if Utc::now().timestamp() >= expires - 300 {
        return refresh_gcal_access_token(email).await;
    }
    Ok(access)
}

// ─── Outlook (Microsoft) OAuth2 ───────────────────────────────────────

/// Start the Outlook OAuth2 flow. Same PKCE+loopback pattern as Gmail.
pub fn start_outlook_oauth(ctx: AppCtx) -> Result<(String, u16), AppError> {
    secrets::require_client_id("outlook")?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::OAuth2(format!("Failed to bind loopback server: {}", e)))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::OAuth2(format!("Failed to get local addr: {}", e)))?
        .port();

    let redirect_url = format!("http://localhost:{}", port);

    let client = build_ms_oauth_client(Some(&redirect_url))?;

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(
            "https://outlook.office365.com/IMAP.AccessAsUser.All".to_string(),
        ))
        .add_scope(Scope::new(
            "https://outlook.office365.com/SMTP.Send".to_string(),
        ))
        .add_scope(Scope::new("offline_access".to_string()))
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let mut pending = PENDING_OUTLOOK.safe_lock();
    *pending = Some(PendingOAuth {
        pkce_verifier,
        csrf_state,
        redirect_port: port,
    });

    std::thread::spawn(
        move || match wait_for_outlook_callback(listener, ctx.clone()) {
            Ok(()) => {
                log::info!("Outlook OAuth callback completed successfully");
            }
            Err(e) => {
                log::error!("Outlook OAuth callback error: {}", e);
                ctx.emit("oauth-error", serde_json::json!(e.to_string()));
            }
        },
    );

    Ok((auth_url.to_string(), port))
}

static PENDING_OUTLOOK: Mutex<Option<PendingOAuth>> = Mutex::new(None);

fn build_ms_oauth_client(redirect_url: Option<&str>) -> Result<BasicClient, AppError> {
    let mut client = BasicClient::new(
        ClientId::new(secrets::MS_CLIENT_ID.to_string()),
        None,
        AuthUrl::new(secrets::MS_AUTH_URI.to_string())
            .map_err(|e| AppError::OAuth2(format!("Invalid MS auth URL: {}", e)))?,
        Some(
            TokenUrl::new(secrets::MS_TOKEN_URI.to_string())
                .map_err(|e| AppError::OAuth2(format!("Invalid MS token URL: {}", e)))?,
        ),
    );
    if let Some(url) = redirect_url {
        client = client.set_redirect_uri(
            RedirectUrl::new(url.to_string())
                .map_err(|e| AppError::OAuth2(format!("Invalid redirect URL: {}", e)))?,
        );
    }
    Ok(client)
}

fn wait_for_outlook_callback(
    listener: TcpListener,
    ctx: AppCtx,
) -> Result<(), AppError> {
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| AppError::OAuth2(format!("Failed to accept connection: {}", e)))?;

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| AppError::OAuth2(format!("Failed to read request: {}", e)))?;

    let url_part = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();

    let response_body = r#"<html><body style="font-family:-apple-system,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;background:#1e1e1e;color:#fff;"><div style="text-align:center"><h2>CXMail</h2><p style="color:#999">Authentication successful! You can close this tab.</p></div></body></html>"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());

    let query = url_part.split('?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;

    for param in query.split('&') {
        let mut kv = param.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("code"), Some(v)) => code = Some(url_decode(v)),
            (Some("state"), Some(v)) => state = Some(url_decode(v)),
            _ => {}
        }
    }

    if let (Some(code), Some(state)) = (code, state) {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| AppError::OAuth2(format!("Failed to create runtime: {}", e)))?;
        rt.block_on(exchange_outlook_code_and_create_account(
            code, state, ctx,
        ))?;
    } else {
        log::error!("Outlook OAuth callback missing code or state: {}", url_part);
        return Err(AppError::OAuth2(
            "Missing code or state in callback".to_string(),
        ));
    }

    Ok(())
}

async fn exchange_outlook_code_and_create_account(
    code: String,
    state: String,
    ctx: AppCtx,
) -> Result<(), AppError> {
    let pending = {
        let mut lock = PENDING_OUTLOOK.safe_lock();
        lock.take()
    };

    let pending =
        pending.ok_or_else(|| AppError::OAuth2("No pending Outlook OAuth flow".to_string()))?;

    if state != *pending.csrf_state.secret() {
        return Err(AppError::OAuth2("CSRF state mismatch".to_string()));
    }

    let redirect_url = format!("http://localhost:{}", pending.redirect_port);

    let http_client = oauth_http_client();
    let token_response = http_client
        .post(secrets::MS_TOKEN_URI)
        .form(&[
            ("code", code.as_str()),
            ("client_id", secrets::MS_CLIENT_ID),
            ("redirect_uri", redirect_url.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", pending.pkce_verifier.secret()),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("MS token request failed: {}", e)))?;

    let status = token_response.status();
    let body_text = token_response.text().await.unwrap_or_default();

    if !status.is_success() {
        log::error!("MS token exchange failed ({}): {}", status, body_text);
        return Err(AppError::OAuth2(format!(
            "Token exchange failed ({}): {}",
            status, body_text
        )));
    }

    let token_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::OAuth2(format!("Failed to parse token response: {}", e)))?;

    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No access_token in response".to_string()))?
        .to_string();
    let refresh_token = token_json["refresh_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No refresh_token in response".to_string()))?
        .to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    // Fetch email from Microsoft Graph
    let email = fetch_outlook_email(&access_token).await?;

    // Store tokens in Keychain
    let key_prefix = format!("outlook:{}", email);
    keychain::store_credential(&format!("{}:access", key_prefix), &access_token)?;
    keychain::store_credential(&format!("{}:refresh", key_prefix), &refresh_token)?;
    keychain::store_credential(&format!("{}:expires", key_prefix), &expires_at.to_string())?;

    log::info!("Outlook OAuth2 tokens stored for {}", email);

    // Create account in database
    let account_id = uuid::Uuid::new_v4().to_string();
    let account = db::accounts::Account {
        id: account_id.clone(),
        email: email.clone(),
        display_name: None,
        provider: "outlook".to_string(),
        imap_host: "outlook.office365.com".to_string(),
        imap_port: 993,
        smtp_host: "smtp.office365.com".to_string(),
        smtp_port: 587,
        imap_security: "implicit".to_string(),
        smtp_security: "starttls".to_string(),
        imap_username: None,
        smtp_username: None,
        color: Some("#0078d4".to_string()),
        is_active: true,
        sort_order: 0,
        group_name: None,
        notify_enabled: true,
        track_opens_enabled: false,
        hidden_from_aggregates: false,
        triage_enabled: false,
    };

    let effective_id = {
        let conn = ctx.db().safe_lock();
        db::accounts::upsert(&conn, &account)?
    };

    log::info!(
        "Outlook account upserted for {} (id={})",
        email,
        effective_id
    );

    let mut emitted_account = account.clone();
    emitted_account.id = effective_id;
    ctx.emit_value("account-added", &emitted_account);

    Ok(())
}

async fn fetch_outlook_email(access_token: &str) -> Result<String, AppError> {
    let client = oauth_http_client();
    let resp = client
        .get("https://graph.microsoft.com/v1.0/me")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("Failed to fetch MS userinfo: {}", e)))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::OAuth2(format!("Failed to parse MS userinfo: {}", e)))?;

    // Microsoft Graph returns mail or userPrincipalName
    body["mail"]
        .as_str()
        .or_else(|| body["userPrincipalName"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::OAuth2("No email in MS userinfo response".to_string()))
}

/// Refresh an Outlook access token using the stored refresh token.
pub async fn refresh_outlook_access_token(email: &str) -> Result<String, AppError> {
    log::info!("Refreshing Outlook access token for {}", email);
    let key_prefix = format!("outlook:{}", email);
    let refresh_token = keychain::get_credential(&format!("{}:refresh", key_prefix))?
        .ok_or_else(|| AppError::OAuth2("No Outlook refresh token found".to_string()))?;

    let http_client = oauth_http_client();
    let resp = http_client
        .post(secrets::MS_TOKEN_URI)
        .form(&[
            ("refresh_token", refresh_token.as_str()),
            ("client_id", secrets::MS_CLIENT_ID),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| AppError::OAuth2(format!("MS token refresh request failed: {}", e)))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        log::error!("MS token refresh failed ({}): {}", status, body_text);
        return Err(AppError::OAuth2(format!(
            "MS token refresh failed ({}): {}",
            status, body_text
        )));
    }

    let token_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::OAuth2(format!("Failed to parse MS refresh response: {}", e)))?;

    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| AppError::OAuth2("No access_token in MS refresh response".to_string()))?
        .to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    // Update stored refresh token if Microsoft rotated it
    if let Some(new_refresh) = token_json["refresh_token"].as_str() {
        keychain::store_credential(&format!("{}:refresh", key_prefix), new_refresh)?;
    }

    keychain::store_credential(&format!("{}:access", key_prefix), &access_token)?;
    keychain::store_credential(&format!("{}:expires", key_prefix), &expires_at.to_string())?;

    log::info!("Outlook access token refreshed for {}", email);
    Ok(access_token)
}

/// Get a valid Outlook access token, refreshing if expired.
pub async fn get_valid_outlook_access_token(email: &str) -> Result<String, AppError> {
    log::info!("Getting valid Outlook access token for {}", email);
    let key_prefix = format!("outlook:{}", email);

    let access_token = keychain::get_credential(&format!("{}:access", key_prefix))?
        .ok_or_else(|| AppError::OAuth2("No Outlook access token found".to_string()))?;

    let expires_at_str = keychain::get_credential(&format!("{}:expires", key_prefix))?
        .unwrap_or_else(|| "0".to_string());
    let expires_at: i64 = expires_at_str.parse().unwrap_or(0);

    let now = chrono::Utc::now().timestamp();
    if now >= expires_at - 300 {
        return refresh_outlook_access_token(email).await;
    }

    Ok(access_token)
}

/// Decode a URL-encoded string (e.g., %2F → /, %3D → =).
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod gcal_scope_tests {
    use super::*;

    #[test]
    fn a_full_grant_is_missing_nothing() {
        let granted = format!("{GCAL_EVENTS_SCOPE} {GCAL_CALENDARLIST_SCOPE} openid");
        assert!(missing_gcal_scopes(&granted).is_empty());
    }

    #[test]
    fn an_empty_grant_reports_both_scopes_missing() {
        // Google may omit `scope` from the token response entirely. The old code
        // defaulted that case to "calendar.events was granted", which is what let
        // a useless token be stored as a working connection.
        assert_eq!(
            missing_gcal_scopes(""),
            vec![GCAL_EVENTS_SCOPE, GCAL_CALENDARLIST_SCOPE]
        );
    }

    #[test]
    fn events_alone_still_misses_calendarlist() {
        // The exact failure seen in production: calendar.events is granted, so the
        // app looked connected, but calendarList.list — the first call of every
        // sync — 403s because that method does not accept calendar.events.
        assert_eq!(
            missing_gcal_scopes(GCAL_EVENTS_SCOPE),
            vec![GCAL_CALENDARLIST_SCOPE]
        );
    }

    #[test]
    fn readonly_events_does_not_satisfy_the_write_scope() {
        // A substring check would wrongly accept this: it contains the literal
        // "calendar.events" but cannot create an event.
        let granted = format!("{GCAL_EVENTS_SCOPE}.readonly {GCAL_CALENDARLIST_SCOPE}");
        assert_eq!(missing_gcal_scopes(&granted), vec![GCAL_EVENTS_SCOPE]);
    }
}

#[cfg(test)]
mod emission_tests {
    use crate::db::accounts::Account;
    use cxmail_core::events::to_payload;

    /// The frontend listens with `listen<Account>("account-added")` and
    /// `listen<string>("oauth-error")`. Before the crate split these payloads
    /// went to `AppHandle::emit` as the typed value directly; they now go
    /// through `serde_json::Value` so `cxmail-email` need not link Tauri.
    ///
    /// That is only a no-op if the round trip is lossless — and `to_payload`
    /// falls back to `null` when serialization fails, which would reach the UI
    /// as a silently empty event rather than an error. Pin both.
    #[test]
    fn to_payload_round_trips_the_account_added_shape() {
        // Spelled out rather than `..Default::default()` — Account has no
        // Default, and a new field added here should make this test fail to
        // compile rather than quietly go untested.
        let account = Account {
            id: "acct-1".to_string(),
            email: "chris@cxventures.io".to_string(),
            display_name: Some("Chris".to_string()),
            provider: "gmail".to_string(),
            imap_host: "imap.gmail.com".to_string(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 465,
            imap_security: "implicit".to_string(),
            smtp_security: "implicit".to_string(),
            imap_username: None,
            smtp_username: None,
            color: None,
            is_active: true,
            sort_order: 0,
            group_name: None,
            notify_enabled: true,
            track_opens_enabled: false,
            hidden_from_aggregates: false,
            triage_enabled: false,
        };

        let direct: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&account).expect("account serializes"))
                .unwrap();
        let via_sink = to_payload(&account);

        // Compared as VALUES, not as strings. `serde_json::to_value` builds a
        // BTreeMap, so the sink emits keys alphabetically where a direct
        // `AppHandle::emit` emitted them in declaration order. JSON objects are
        // unordered and the frontend reads `event.payload.id` by name
        // (`AccountSetup.tsx` -> `addAccount`), so that reordering is invisible
        // — but a dropped or renamed field would not be, and this catches it.
        assert_eq!(direct, via_sink, "the event sink must not reshape the payload");
        assert!(
            !via_sink.is_null(),
            "to_payload falls back to null on failure, which would reach the UI \
             as an empty account-added rather than an error"
        );
        assert_eq!(
            via_sink.as_object().expect("an object").len(),
            20,
            "field count changed — check the frontend Account type still matches"
        );
    }

    #[test]
    fn to_payload_round_trips_a_bare_string_payload() {
        // `oauth-error`, `calendar-oauth-error` and `calendar-connected` all
        // carry a bare string.
        let message = "Calendar OAuth CSRF state mismatch".to_string();
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            serde_json::to_string(&to_payload(&message)).unwrap(),
        );
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            serde_json::to_string(&serde_json::json!(message.clone())).unwrap(),
        );
    }
}
