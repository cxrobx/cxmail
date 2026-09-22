use crate::db::accounts::Account;
use crate::email::oauth2;
use crate::email::providers::{self, TlsMode};
use crate::error::AppError;
use async_imap::types::{Fetch, NameAttribute};
use futures::StreamExt;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex as StdMutex};
use std::time::Instant;
use tokio::net::TcpStream;
use tokio_native_tls::TlsConnector;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type TlsCompat = Compat<tokio_native_tls::TlsStream<TcpStream>>;
type ImapStream = futures::io::BufReader<TlsCompat>;
pub type ImapSession = async_imap::Session<ImapStream>;

// ---------------------------------------------------------------------------
// Session ledger — diagnostic instrumentation, added 2026-08-11.
//
// Sync failures ("Connection timed out for X") climbed ~5x between late July
// and Aug 11 while "IMAP LOGOUT timed out" climbed ~45x over the same window,
// and 24 of 28 failures on Aug 11 landed between 00:00-06:00 local while the
// machine was awake and idle — i.e. failures are WORST when the app is least
// used. Two explanations fit that shape and imply opposite fixes:
//
//   (a) we leak Gmail-side connection slots. A session whose LOGOUT fails or
//       times out keeps its slot until the server reaps it; Gmail caps
//       simultaneous IMAP connections PER ACCOUNT, so a leak on one account
//       eventually starves that same account's next connect.
//   (b) the network (or Gmail's throttling of nine accounts from one IP) is
//       simply worse than it was, and no code change helps.
//
// They could not be told apart because `disconnect` did not know WHICH account
// it was closing, so a drop could never be matched to the later failure it
// supposedly caused. These counters are per-account for exactly that reason,
// and they are reported inline on the timeout error itself — the failure line
// carries its own evidence rather than requiring a correlation pass over the log.
//
// CORRECTION, 2026-08-14. The first cut of this shipped a counter named
// `believed_open` (`opened - closed_clean - dropped`) and read it as live
// connections. It is not, in two independent ways, and reading it that way
// pointed the diagnosis straight at (a):
//
//   * `idle.rs` opens a session per account and reaches `disconnect` on NO
//     error path — every reconnect incremented `opened` alone. Since that loop
//     reconnects hardest on the sickest account, the bogus number correlated
//     beautifully with the real symptom, which is what made it convincing.
//   * Even a genuinely unaccounted session is not an open socket. `Drop`
//     closes the `TcpStream`; only the SERVER-side slot is in question.
//
// Measured while it claimed 14 open for chris@cxventures.io and 35 across four
// accounts: `lsof` showed 11 sockets to :993 for the whole process, all nine
// accounts, and 0 in the helper. So `abandoned` now counts the sessions that
// end without a LOGOUT attempt (see `SessionGuard`), `unaccounted` is a
// self-check on the ledger that should read 0, and `slot_suspect`
// (`dropped + abandoned`) is the only number that speaks to hypothesis (a).
// Do not reintroduce a counter that infers live sockets from arithmetic —
// count sockets, or count endings.
//
// Cheap by construction: three u64s per account behind a std Mutex, touched
// twice per IMAP session. Not wired to any UI.
// ---------------------------------------------------------------------------

/// Per-account session accounting.
///
/// **None of these counters is a socket count.** Dropping an `ImapSession`
/// closes its `TcpStream`, so our side is gone the moment the value dies by any
/// path. What the counters track is how each session ENDED, because that is
/// what predicts whether the *server* still holds a slot for it.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionLedger {
    pub opened: u64,
    /// LOGOUT completed. The server released the slot.
    pub closed_clean: u64,
    /// LOGOUT was attempted and returned an error or timed out. The socket is
    /// gone on our side; the server-side slot is NOT, until the server reaps it.
    pub dropped: u64,
    /// The session ended without LOGOUT being attempted at all — it left scope
    /// via `?`, a `return`, or a panic, and `Drop` closed the socket. The FIN
    /// releases the slot on a live path; on a path that has already gone dead
    /// (the case that produces these) it may not, so this counts alongside
    /// `dropped` as slot-suspect.
    pub abandoned: u64,
}

impl SessionLedger {
    /// Sessions opened whose ending has not been recorded — i.e. the ones
    /// currently IN USE, plus any the ledger has lost track of.
    ///
    /// Read it as a live-session gauge with a bounded resting value, never as a
    /// running total. Each account keeps one long-lived IDLE session, so at rest
    /// this reads **1 per connected account** and 0 for an account that cannot
    /// connect; it rises briefly during a sync and settles back. Measured
    /// 2026-08-14, ten minutes after the fix shipped: 8 accounts × 1, summing to
    /// 8, against exactly 8 sockets to `:993` in `lsof`.
    ///
    /// **A value that climbs without settling is the bug this replaced** — that
    /// is the ledger losing sessions, not Gmail holding them. The distinction
    /// matters because the old counter conflated the two and read 66 across nine
    /// accounts while 9 sockets were open. For "might the server still hold a
    /// slot", use [`SessionLedger::slot_suspect`].
    ///
    /// Signed on purpose: negative means double-counted closes, which is itself
    /// a bug worth seeing rather than clamping away.
    pub fn unaccounted(&self) -> i64 {
        self.opened as i64
            - self.closed_clean as i64
            - self.dropped as i64
            - self.abandoned as i64
    }

    /// Sessions that may still hold a server-side slot: LOGOUT failed, or was
    /// never attempted. This is the number the slot-starvation hypothesis is
    /// actually about.
    pub fn slot_suspect(&self) -> u64 {
        self.dropped + self.abandoned
    }
}

static LEDGER: LazyLock<StdMutex<HashMap<String, SessionLedger>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

static PROCESS_START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Anchor the uptime clock at launch. Without this the clock would start at the
/// first IMAP connection instead, which is precisely the interval a leak
/// hypothesis needs measured correctly.
pub fn note_process_start() {
    let _ = *PROCESS_START;
}

/// Whole seconds since `note_process_start()` (or since the first IMAP
/// connection, if that was never called).
pub fn uptime_secs() -> u64 {
    PROCESS_START.elapsed().as_secs()
}

fn ledger_update(email: &str, f: impl FnOnce(&mut SessionLedger)) {
    // A poisoned lock must never take a mail sync down — this is diagnostics.
    if let Ok(mut map) = LEDGER.lock() {
        f(map.entry(email.to_string()).or_default());
    }
}

/// Snapshot for one account. Returns the zero ledger for an unknown account.
pub fn ledger_for(email: &str) -> SessionLedger {
    LEDGER
        .lock()
        .ok()
        .and_then(|map| map.get(email).copied())
        .unwrap_or_default()
}

/// Every account with recorded activity, for a periodic summary line.
pub fn ledger_snapshot() -> Vec<(String, SessionLedger)> {
    LEDGER
        .lock()
        .map(|map| {
            let mut rows: Vec<_> = map.iter().map(|(k, v)| (k.clone(), *v)).collect();
            rows.sort_by(|a, b| b.1.slot_suspect().cmp(&a.1.slot_suspect()));
            rows
        })
        .unwrap_or_default()
}

// --- background connect breaker -------------------------------------------
//
// A background loop that retries a refused account on a fixed cadence is how a
// server-side throttle gets RENEWED instead of expiring. Measured 2026-08-14:
// `chris@cxventures.io` was refused at XOAUTH2 for three days while CXMail kept
// knocking ~76 times an hour, harder than any healthy account. Adding backoff
// to the IDLE loop alone only halved it — `sync_all_inboxes` runs every ~3.75
// minutes and tried every account unconditionally, so the account still took
// ~16 attempts an hour with no decay.
//
// The breaker lives here, at the connect layer, rather than in either loop:
// both of them are asking the same question ("is it worth talking to this
// account right now?"), and the failure they are reacting to is observed here.
//
// It is ADVISORY and background-only by construction — `connect_for_account`
// records outcomes but never refuses. A user clicking a message, sending, or
// saving a draft must always get a real attempt; a circuit breaker that makes
// the app feel broken to a person is worse than the throttle it is dodging.

const CONNECT_BACKOFF_BASE: std::time::Duration = std::time::Duration::from_secs(30);
const CONNECT_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Delay a background caller should wait after the Nth consecutive connect
/// failure (1-based). Pure and shared so the IDLE loop and the periodic sync
/// cannot drift onto two different schedules.
pub fn connect_backoff(consecutive_failures: u32) -> std::time::Duration {
    let doublings = consecutive_failures.saturating_sub(1).min(16);
    let secs = CONNECT_BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64 << doublings)
        .min(CONNECT_BACKOFF_MAX.as_secs());
    std::time::Duration::from_secs(secs)
}

#[derive(Default, Clone, Copy)]
struct ConnectHealth {
    consecutive_failures: u32,
    retry_after: Option<Instant>,
}

static CONNECT_HEALTH: LazyLock<StdMutex<HashMap<String, ConnectHealth>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// Record whether a connect attempt succeeded, and arm/clear the breaker.
fn note_connect_outcome(account_email: &str, ok: bool) {
    if let Ok(mut map) = CONNECT_HEALTH.lock() {
        let h = map.entry(account_email.to_string()).or_default();
        if ok {
            *h = ConnectHealth::default();
        } else {
            h.consecutive_failures = h.consecutive_failures.saturating_add(1);
            h.retry_after = Instant::now().checked_add(connect_backoff(h.consecutive_failures));
        }
    }
}

/// How long a BACKGROUND caller should hold off on this account; `None` means
/// go ahead.
///
/// **User-initiated work must not consult this.** Reading a message, sending,
/// and saving a draft all connect on a person's behalf and must always try.
pub fn background_cooldown_remaining(account_email: &str) -> Option<std::time::Duration> {
    let h = CONNECT_HEALTH.lock().ok()?.get(account_email).copied()?;
    let until = h.retry_after?;
    until.checked_duration_since(Instant::now())
}

/// Consecutive connect failures on record, for logging.
pub fn connect_failure_streak(account_email: &str) -> u32 {
    CONNECT_HEALTH
        .lock()
        .ok()
        .and_then(|m| m.get(account_email).map(|h| h.consecutive_failures))
        .unwrap_or(0)
}

/// Record a session that ended without LOGOUT being attempted.
///
/// Prefer [`SessionGuard`] — calling this by hand at each error site is the
/// arrangement that produced the hole this exists to close.
pub fn note_abandoned(account_email: &str) {
    ledger_update(account_email, |l| l.abandoned += 1);
}

/// Accounts for a session that can leave scope via `?`, `return`, or a panic.
///
/// `disconnect` can only count sessions it is handed, so any function that
/// opens one and then uses `?` before closing it leaves `opened` incremented
/// and nothing else — which is exactly how `idle.rs` made the old
/// `believed_open` climb to 14 for an account holding ~2 real sockets. A guard
/// rather than a call at each error site because the failing property was that
/// a NEW error path silently reintroduces the hole; here it cannot, since the
/// accounting rides on the scope rather than on remembering.
///
/// Disarm immediately before handing the session to [`disconnect`], which does
/// its own accounting.
pub struct SessionGuard<'a> {
    account_email: &'a str,
    armed: bool,
}

impl<'a> SessionGuard<'a> {
    /// Arm accounting for a session that has just been opened successfully.
    /// Only construct this after `connect_for_account` returns `Ok` — a failed
    /// connect never incremented `opened`, so counting it would push
    /// `unaccounted` negative.
    pub fn new(account_email: &'a str) -> Self {
        Self { account_email, armed: true }
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            note_abandoned(self.account_email);
        }
    }
}

/// XOAUTH2 authenticator for async-imap.
struct XOAuth2 {
    token: String,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _data: &[u8]) -> Self::Response {
        self.token.clone()
    }
}

/// Represents a folder from the IMAP LIST command.
#[derive(Debug, Clone, Serialize)]
pub struct ImapFolder {
    pub name: String,
    pub delimiter: Option<String>,
    pub folder_type: String,
    /// `folder_type` came from an RFC 6154 SPECIAL-USE attribute the server
    /// declared, not from guessing at the name. Persisted so
    /// `db::folders::folder_of_type` can rank a declaration above a guess.
    pub special_use: bool,
}

/// Represents a message header fetched from IMAP.
#[derive(Debug, Clone, Serialize)]
pub struct ImapMessageHeader {
    pub uid: u32,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub to_list: String,
    pub cc_list: String,
    pub date: Option<String>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub flags: Vec<String>,
    pub size: u32,
    pub is_read: bool,
    pub is_flagged: bool,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub precedence: Option<String>,
    /// `X-CXMail-Draft-Id` — the logical draft this revision belongs to
    /// (`db::drafts`, v60). Only drafts this software saved carry it.
    pub draft_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImapMessageFlags {
    pub uid: u32,
    pub is_read: bool,
    pub is_flagged: bool,
}

/// The keychain key holding a password-auth account's password.
///
/// Reproduces the literals that predated the generic provider byte-for-byte
/// (`icloud:{email}:password`, previously spelled inline here and in what is
/// now `smtp::get_smtp_transport`). Changing the shape orphans every already
/// stored credential — the keychain has no rename.
// `pub`: the Keychain key format is consumed by `commands::auth` and
// `commands::accounts`, both now in the app crate.
pub fn password_key(provider: &str, email: &str) -> String {
    format!("{}:{}:password", provider, email)
}

/// Open a TCP+TLS connection to an IMAP server and consume its greeting,
/// returning a client ready to authenticate.
///
/// Shared by every connect path so there is one place TLS is established —
/// and, critically, one place that could ever grow a plaintext branch. There
/// isn't one: `native_tls::TlsConnector::new()` keeps default certificate
/// verification, and no caller can opt out.
async fn open_imap_client(
    host: &str,
    port: u16,
    security: TlsMode,
) -> Result<async_imap::Client<ImapStream>, AppError> {
    if security == TlsMode::StartTls {
        // Deferred deliberately, not overlooked. `ImapSession` is aliased to a
        // concrete TlsStream type, so a STARTTLS path makes the stream generic
        // across ~40 signatures — and the upgrade itself has a silent-corruption
        // trap: the greeting is read through a BufReader, whose `into_inner()`
        // DISCARDS anything already buffered, exactly as the connection goes
        // secure. Every host CXMail presets is 993 implicit. See gotcha #41.
        return Err(AppError::Imap(format!(
            "IMAP STARTTLS is not supported yet — {}:{} needs implicit TLS (usually port 993)",
            host, port
        )));
    }

    let native_connector = native_tls::TlsConnector::new()
        .map_err(|e| AppError::Imap(format!("TLS connector creation failed: {}", e)))?;
    let tls = TlsConnector::from(native_connector);

    log::info!("Establishing TCP connection to {}:{}...", host, port);
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| AppError::Imap(format!("TCP connection failed: {}", e)))?;
    log::info!("TCP connected, starting TLS handshake...");

    let tls_stream = tls
        .connect(host, tcp)
        .await
        .map_err(|e| AppError::Imap(format!("TLS handshake failed: {}", e)))?;
    log::info!("TLS handshake complete, creating IMAP client...");

    // Convert tokio TLS stream to futures-io compatible via compat layer
    let compat_stream = tls_stream.compat();

    // Read server greeting manually to verify connection works
    use futures::io::AsyncBufReadExt;
    let mut buf_stream = futures::io::BufReader::new(compat_stream);
    let mut greeting = String::new();
    buf_stream
        .read_line(&mut greeting)
        .await
        .map_err(|e| AppError::Imap(format!("Failed to read greeting: {}", e)))?;
    log::info!("IMAP greeting: {}", greeting.trim());

    Ok(async_imap::Client::new(buf_stream))
}

/// Authenticate with SASL XOAUTH2 (Gmail, Outlook).
async fn connect_xoauth2(
    host: &str,
    port: u16,
    email: &str,
    access_token: &str,
) -> Result<ImapSession, AppError> {
    // Build the raw SASL XOAUTH2 string (NOT base64-encoded — async-imap encodes it)
    let xoauth2_token = format!("user={}\x01auth=Bearer {}\x01\x01", email, access_token);

    let client = open_imap_client(host, port, TlsMode::Implicit).await?;
    log::info!("IMAP client created, authenticating with XOAUTH2...");

    let auth = XOAuth2 {
        token: xoauth2_token,
    };
    let session = client
        .authenticate("XOAUTH2", auth)
        .await
        .map_err(|(e, _)| AppError::Imap(format!("XOAUTH2 auth failed for {}: {}", host, e)))?;

    log::info!("XOAUTH2 authentication successful!");
    Ok(session)
}

/// Authenticate with IMAP LOGIN — the single password-auth path, shared by
/// iCloud and every generic host.
async fn connect_login(
    host: &str,
    port: u16,
    security: TlsMode,
    username: &str,
    password: &str,
) -> Result<ImapSession, AppError> {
    let client = open_imap_client(host, port, security).await?;
    log::info!("IMAP client created, authenticating with LOGIN as {}...", username);

    let session = client
        .login(username, password)
        .await
        .map_err(|(e, _)| AppError::Imap(format!("LOGIN auth failed: {}", e)))?;

    log::info!("LOGIN authentication successful!");
    Ok(session)
}

/// Connect to Gmail IMAP with XOAUTH2 and return a session.
pub async fn connect_gmail(email: &str) -> Result<ImapSession, AppError> {
    log::info!("Connecting to Gmail IMAP for {}", email);
    let access_token = oauth2::get_valid_access_token(email).await?;
    log::info!("Got access token, connecting to imap.gmail.com:993...");
    connect_xoauth2("imap.gmail.com", 993, email, &access_token).await
}

/// List all folders from the IMAP server.
pub async fn list_folders(
    session: &mut ImapSession,
    provider: &str,
) -> Result<Vec<ImapFolder>, AppError> {
    let mut names_stream = session
        .list(None, Some("*"))
        .await
        .map_err(|e| AppError::Imap(format!("LIST failed: {}", e)))?;

    let mut folders = Vec::new();
    while let Some(item) = names_stream.next().await {
        let name = item.map_err(|e| AppError::Imap(format!("LIST item error: {}", e)))?;
        let folder_name = name.name().to_string();
        let delimiter = name.delimiter().map(|c| c.to_string());

        // Branch inside the loop rather than widening `classify_folder`: the
        // attribute slice borrows from `name`, so it cannot be carried out to
        // a caller or stored on `ImapFolder`.
        let (folder_type, special_use) = if provider == "imap" {
            let attrs = name.attributes();
            // \Noselect names are hierarchy placeholders (Dovecot, Courier).
            // They are not mailboxes — persisting one means every later
            // SELECT of it fails. Skipped only here; the three legacy
            // providers keep their existing behavior untouched.
            if attrs.contains(&NameAttribute::NoSelect) {
                log::debug!("Skipping \\Noselect placeholder folder {:?}", folder_name);
                continue;
            }
            match special_use_folder_type(attrs) {
                Some(t) => (t.to_string(), true),
                None => (
                    heuristic_folder_type(&folder_name, delimiter.as_deref()).to_string(),
                    false,
                ),
            }
        } else {
            (classify_folder(&folder_name, provider), false)
        };

        folders.push(ImapFolder {
            name: folder_name,
            delimiter,
            folder_type,
            special_use,
        });
    }

    Ok(folders)
}

/// Connect to iCloud IMAP with LOGIN auth and return a session.
pub async fn connect_icloud(email: &str) -> Result<ImapSession, AppError> {
    log::info!("Connecting to iCloud IMAP for {}", email);
    let password = crate::keychain::get_credential(&password_key("icloud", email))?.ok_or_else(
        || AppError::AuthFailed("No iCloud app-specific password found in Keychain".to_string()),
    )?;
    connect_login(
        "imap.mail.me.com",
        993,
        TlsMode::Implicit,
        email,
        &password,
    )
    .await
}

/// Connect to Outlook IMAP with XOAUTH2 and return a session.
pub async fn connect_outlook(email: &str) -> Result<ImapSession, AppError> {
    log::info!("Connecting to Outlook IMAP for {}", email);
    let access_token = oauth2::get_valid_outlook_access_token(email).await?;
    log::info!("Got Outlook access token, connecting to outlook.office365.com:993...");
    connect_xoauth2("outlook.office365.com", 993, email, &access_token).await
}

/// Connect to an arbitrary IMAP server using the settings on the account row.
///
/// This is the only connect path that reads host/port/security/username from
/// the DB — the three OAuth-era providers keep their literals, so a corrupted
/// row cannot redirect an existing Gmail account at an attacker's host.
pub async fn connect_generic(account: &Account) -> Result<ImapSession, AppError> {
    let host = providers::validate_host(&account.imap_host)?;
    let port = providers::port_from_i32(account.imap_port, "IMAP")?;
    let security = TlsMode::parse(&account.imap_security)?;
    // Called for its rejections (plaintext relay / POP3), not its answer: a
    // server may legitimately run implicit TLS on an unusual port, so a
    // disagreement between the stored mode and the port is NOT an error.
    providers::security_for_port(port)?;

    let username = account
        .imap_username
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or(&account.email);
    let password = crate::keychain::get_credential(&password_key("imap", &account.email))?
        .ok_or_else(|| {
            AppError::AuthFailed(format!(
                "No stored password for {} — reconnect the account.",
                account.email
            ))
        })?;

    log::info!("Connecting to {}:{} for {}", host, port, account.email);
    connect_login(&host, port, security, username, &password).await
}

/// Connection timeout for IMAP connect + auth.
/// Dead sockets (TCP alive but no data flow) can hang indefinitely without this.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Connect to the account's IMAP server and return an authenticated session.
/// Wraps the entire connect+auth flow in a timeout to prevent dead-socket hangs.
///
/// Takes the whole `Account` rather than `(email, provider)` because the
/// generic `imap` provider's host, port, security mode and login name live on
/// the row — there is nothing to dispatch on without it. The old
/// `connect_for_provider` was **deleted** rather than kept as a wrapper so a
/// missed call site is a compile error instead of an account that silently
/// cannot connect.
pub async fn connect_for_account(account: &Account) -> Result<ImapSession, AppError> {
    let outcome = tokio::time::timeout(CONNECT_TIMEOUT, async {
        match account.provider.as_str() {
            "gmail" => connect_gmail(&account.email).await,
            "icloud" => connect_icloud(&account.email).await,
            "outlook" => connect_outlook(&account.email).await,
            "imap" => connect_generic(account).await,
            other => Err(AppError::Imap(format!("Unsupported provider: {}", other))),
        }
    })
    .await;

    match outcome {
        Err(_elapsed) => {
            // Carry the evidence on the failure itself, rather than requiring a
            // correlation pass over the log. A leaked-slot cause shows up here
            // as a climbing `slot_suspect` for THIS account at the moment its
            // connect timed out; a network cause shows up as a timeout with it
            // flat. Read `slot_suspect`, never `unaccounted` — the latter is a
            // self-check on the ledger and says nothing about the server. See
            // the correction in the ledger comment above.
            let l = ledger_for(&account.email);
            log::warn!(
                "connect timeout for {} ({}) [uptime={}s opened={} closed_clean={} dropped={} abandoned={} slot_suspect={} unaccounted={}]",
                account.email,
                account.provider,
                uptime_secs(),
                l.opened,
                l.closed_clean,
                l.dropped,
                l.abandoned,
                l.slot_suspect(),
                l.unaccounted(),
            );
            note_connect_outcome(&account.email, false);
            Err(AppError::Imap(format!(
                "Connection timed out for {} ({})",
                account.email, account.provider
            )))
        }
        // Count only sessions that genuinely exist. Counting the inner Err too
        // (an auth failure, an unsupported provider) would inflate `opened`
        // against an ending that never happens, pushing `unaccounted` positive
        // for a session that was never created. This is also why
        // `SessionGuard::new` may only be constructed on the Ok path.
        Ok(Ok(session)) => {
            ledger_update(&account.email, |l| l.opened += 1);
            note_connect_outcome(&account.email, true);
            Ok(session)
        }
        // An inner error is still a failed connect from the breaker's point of
        // view — a rejected auth or an unreachable host earns a hold-off just as
        // a timeout does. Only a session we actually got clears it.
        Ok(Err(e)) => {
            note_connect_outcome(&account.email, false);
            Err(e)
        }
    }
}

/// Classify a Gmail folder name into a folder type.
fn classify_gmail_folder(name: &str) -> String {
    match name {
        "INBOX" => "inbox".to_string(),
        "[Gmail]/Sent Mail" => "sent".to_string(),
        "[Gmail]/Drafts" => "drafts".to_string(),
        "[Gmail]/Trash" => "trash".to_string(),
        "[Gmail]/Spam" => "spam".to_string(),
        "[Gmail]/All Mail" => "archive".to_string(),
        "[Gmail]/Starred" => "starred".to_string(),
        "[Gmail]/Important" => "important".to_string(),
        _ => "other".to_string(),
    }
}

/// Classify an iCloud folder name into a folder type.
fn classify_icloud_folder(name: &str) -> String {
    match name {
        "INBOX" => "inbox".to_string(),
        "Sent Messages" => "sent".to_string(),
        "Drafts" => "drafts".to_string(),
        "Trash" => "trash".to_string(),
        "Junk" => "spam".to_string(),
        "Archive" => "archive".to_string(),
        "Notes" => "other".to_string(),
        _ => "other".to_string(),
    }
}

/// Classify an Outlook folder name into a folder type.
fn classify_outlook_folder(name: &str) -> String {
    match name {
        "INBOX" | "Inbox" => "inbox".to_string(),
        "Sent Items" | "Sent" => "sent".to_string(),
        "Drafts" => "drafts".to_string(),
        "Deleted Items" | "Trash" => "trash".to_string(),
        "Junk Email" | "Junk" => "spam".to_string(),
        "Archive" => "archive".to_string(),
        _ => "other".to_string(),
    }
}

/// Map an RFC 6154 SPECIAL-USE attribute to a CXMail `folder_type`.
///
/// Checked in a **fixed precedence order**, not in the order the server listed
/// them, so a mailbox carrying two attributes classifies the same way on every
/// server. `\All` maps to `archive` to match `classify_gmail_folder`'s
/// treatment of `[Gmail]/All Mail`.
///
/// Applied to the generic provider ONLY. Gmail labels `[Gmail]/Starred` with
/// `\Flagged` and `[Gmail]/Important` with an extension attribute, and its
/// existing classification maps those to `starred` / `important` — which are
/// not RFC 6154 concepts. Letting SPECIAL-USE win there would silently rewrite
/// `folder_type` for accounts that already work, and six call sites read it.
fn special_use_folder_type(attrs: &[NameAttribute<'_>]) -> Option<&'static str> {
    const PRECEDENCE: &[(NameAttribute<'static>, &str)] = &[
        (NameAttribute::Sent, "sent"),
        (NameAttribute::Drafts, "drafts"),
        (NameAttribute::Trash, "trash"),
        (NameAttribute::Junk, "spam"),
        (NameAttribute::Archive, "archive"),
        (NameAttribute::All, "archive"),
        (NameAttribute::Flagged, "starred"),
    ];
    PRECEDENCE
        .iter()
        .find(|(attr, _)| attrs.contains(attr))
        .map(|(_, kind)| *kind)
}

/// Classify a folder by name when the server declared no SPECIAL-USE.
///
/// Matches the **last hierarchy segment** so `INBOX.Sent` and `INBOX/Sent`
/// work, and matches it **whole** — a substring match would make `Sent to
/// Legal` the account's Sent folder, which silently blinds follow-up nudges
/// (`db::nudges` reads "my last message in this thread" out of Sent).
fn heuristic_folder_type(name: &str, delimiter: Option<&str>) -> &'static str {
    if name.eq_ignore_ascii_case("INBOX") {
        return "inbox";
    }

    // Prefer the server's declared delimiter; fall back to trying both common
    // ones, since a plain LIST may omit it.
    let leaf = match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => name.rsplit(d).next().unwrap_or(name),
        None => name
            .rsplit(['/', '.'])
            .next()
            .unwrap_or(name),
    };
    let leaf = leaf.trim().to_ascii_lowercase();

    match leaf.as_str() {
        "inbox" => "inbox",
        "sent" | "sent items" | "sent mail" | "sent messages" | "sentmail" => "sent",
        "drafts" | "draft" => "drafts",
        "trash" | "bin" | "deleted" | "deleted items" | "deleted messages" => "trash",
        "spam" | "junk" | "junk e-mail" | "junk email" | "bulk mail" => "spam",
        "archive" | "archives" | "all mail" | "all" => "archive",
        _ => "other",
    }
}

/// Classify a generic-provider folder: server declaration first, name second.
pub fn classify_generic_folder(
    name: &str,
    delimiter: Option<&str>,
    attrs: &[NameAttribute<'_>],
) -> String {
    special_use_folder_type(attrs)
        .unwrap_or_else(|| heuristic_folder_type(name, delimiter))
        .to_string()
}

/// Classify a folder name based on provider.
pub fn classify_folder(name: &str, provider: &str) -> String {
    match provider {
        "gmail" => classify_gmail_folder(name),
        "icloud" => classify_icloud_folder(name),
        "outlook" => classify_outlook_folder(name),
        _ => {
            // Fallback: try common names
            match name {
                "INBOX" => "inbox".to_string(),
                _ => "other".to_string(),
            }
        }
    }
}

/// Select a folder and return (UIDVALIDITY, UIDNEXT).
pub async fn select_folder(
    session: &mut ImapSession,
    folder: &str,
) -> Result<(u32, u32), AppError> {
    let mailbox = session
        .select(folder)
        .await
        .map_err(|e| AppError::Imap(format!("SELECT {} failed: {}", folder, e)))?;

    let uidvalidity = mailbox
        .uid_validity
        .ok_or_else(|| AppError::Imap("No UIDVALIDITY in SELECT response".to_string()))?;
    let uidnext = mailbox
        .uid_next
        .ok_or_else(|| AppError::Imap("No UIDNEXT in SELECT response".to_string()))?;

    Ok((uidvalidity, uidnext))
}

/// Fetch message headers by UID range.
pub async fn fetch_headers(
    session: &mut ImapSession,
    uid_start: u32,
    uid_end: u32,
) -> Result<Vec<ImapMessageHeader>, AppError> {
    let range = format!("{}:{}", uid_start, uid_end);
    let mut fetch_stream = session
        .uid_fetch(
            &range,
            "(UID ENVELOPE FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID IN-REPLY-TO REFERENCES LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST PRECEDENCE X-CXMAIL-DRAFT-ID)])",
        )
        .await
        .map_err(|e| AppError::Imap(format!("FETCH headers failed: {}", e)))?;

    let mut headers = Vec::new();
    while let Some(item) = fetch_stream.next().await {
        let fetch = item.map_err(|e| AppError::Imap(format!("FETCH item error: {}", e)))?;
        if let Some(uid) = fetch.uid {
            let header = parse_fetch_to_header(uid, &fetch);
            headers.push(header);
        }
    }

    Ok(headers)
}

/// Fetch the full RFC822 body of a message by UID.
pub async fn fetch_body(session: &mut ImapSession, uid: u32) -> Result<Vec<u8>, AppError> {
    let range = format!("{}", uid);
    let mut fetch_stream = session
        .uid_fetch(&range, "BODY.PEEK[]")
        .await
        .map_err(|e| AppError::Imap(format!("FETCH body failed: {}", e)))?;

    let fetch = fetch_stream
        .next()
        .await
        .ok_or_else(|| AppError::NotFound(format!("Message UID {} not found", uid)))?
        .map_err(|e| AppError::Imap(format!("FETCH body item error: {}", e)))?;

    let body = fetch
        .body()
        .ok_or_else(|| AppError::Imap("No body in FETCH response".to_string()))?;

    log::info!(
        "imap::fetch_body: uid={} returned {} bytes",
        uid,
        body.len()
    );
    Ok(body.to_vec())
}

/// Fetch ONLY the verbatim RFC 5322 header block of one message.
///
/// Deliberately separate from `fetch_body`'s `BODY.PEEK[]`: the sync FETCH asks
/// for `BODY.PEEK[HEADER.FIELDS (…)]`, a named subset, so full headers are not
/// available for already-synced mail and widening that FETCH would pull
/// megabytes of header text across a 14.6k-message mailbox for messages nobody
/// will audit. This is the lazy alternative — one message, headers only, on the
/// first request for that message's source. `.PEEK` so it never sets `\Seen`.
pub async fn fetch_raw_headers(
    session: &mut ImapSession,
    uid: u32,
) -> Result<Vec<u8>, AppError> {
    let range = format!("{}", uid);
    let mut fetch_stream = session
        .uid_fetch(&range, "BODY.PEEK[HEADER]")
        .await
        .map_err(|e| AppError::Imap(format!("FETCH headers failed: {}", e)))?;

    let fetch = fetch_stream
        .next()
        .await
        .ok_or_else(|| AppError::NotFound(format!("Message UID {} not found", uid)))?
        .map_err(|e| AppError::Imap(format!("FETCH header item error: {}", e)))?;

    // async-imap files a `BODY[HEADER]` section under `header()`, not `body()`.
    let block = fetch
        .header()
        .or_else(|| fetch.body())
        .ok_or_else(|| AppError::Imap("No header section in FETCH response".to_string()))?;

    log::info!(
        "imap::fetch_raw_headers: uid={} returned {} bytes",
        uid,
        block.len()
    );
    Ok(block.to_vec())
}

pub async fn fetch_flags(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<ImapMessageFlags>, AppError> {
    let mut flags = Vec::new();
    for sequence in build_uid_sequences(uids, 250) {
        let mut fetch_stream = session
            .uid_fetch(&sequence, "(UID FLAGS)")
            .await
            .map_err(|e| AppError::Imap(format!("FETCH flags failed for {}: {}", sequence, e)))?;

        while let Some(item) = fetch_stream.next().await {
            let fetch =
                item.map_err(|e| AppError::Imap(format!("FETCH flags item error: {}", e)))?;
            if let Some(uid) = fetch.uid {
                let flag_list: Vec<async_imap::types::Flag> = fetch.flags().collect();
                let is_read = flag_list
                    .iter()
                    .any(|f| matches!(f, async_imap::types::Flag::Seen));
                let is_flagged = flag_list
                    .iter()
                    .any(|f| matches!(f, async_imap::types::Flag::Flagged));
                flags.push(ImapMessageFlags {
                    uid,
                    is_read,
                    is_flagged,
                });
            }
        }
    }

    Ok(flags)
}

pub async fn search_unseen(session: &mut ImapSession) -> Result<HashSet<u32>, AppError> {
    session
        .uid_search("UNSEEN")
        .await
        .map_err(|e| AppError::Imap(format!("SEARCH UNSEEN failed: {}", e)))
}

/// Run an arbitrary UID SEARCH query in the currently-selected folder.
/// Used by server-side search fallback (X-GM-RAW for Gmail, standard
/// RFC 3501 keys elsewhere — see `email::server_search`).
pub async fn search_folder(
    session: &mut ImapSession,
    query: &str,
) -> Result<Vec<u32>, AppError> {
    let hits = session
        .uid_search(query)
        .await
        .map_err(|e| AppError::Imap(format!("UID SEARCH failed: {}", e)))?;
    let mut uids: Vec<u32> = hits.into_iter().collect();
    uids.sort_unstable();
    Ok(uids)
}

/// Fetch headers for an explicit UID set (compacted into ranges). Same FETCH
/// items as `fetch_headers` so results feed the normal ingest path.
pub async fn fetch_headers_for_uids(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<ImapMessageHeader>, AppError> {
    let mut headers = Vec::new();
    for sequence in build_uid_sequences(uids, 250) {
        let mut fetch_stream = session
            .uid_fetch(
                &sequence,
                "(UID ENVELOPE FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID IN-REPLY-TO REFERENCES LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST PRECEDENCE X-CXMAIL-DRAFT-ID)])",
            )
            .await
            .map_err(|e| AppError::Imap(format!("FETCH headers for {} failed: {}", sequence, e)))?;
        while let Some(item) = fetch_stream.next().await {
            let fetch = item.map_err(|e| AppError::Imap(format!("FETCH item error: {}", e)))?;
            if let Some(uid) = fetch.uid {
                headers.push(parse_fetch_to_header(uid, &fetch));
            }
        }
    }
    Ok(headers)
}

pub async fn uid_search_range(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<HashSet<u32>, AppError> {
    let mut existing = HashSet::new();
    for sequence in build_uid_sequences(uids, 250) {
        let query = format!("UID {}", sequence);
        let result = session
            .uid_search(&query)
            .await
            .map_err(|e| AppError::Imap(format!("UID SEARCH {} failed: {}", query, e)))?;
        existing.extend(result);
    }
    Ok(existing)
}

/// Update flags on a message (e.g., mark read/unread).
///
/// **Draining a response stream is not the same as checking the response**, and
/// that distinction is the whole reason this is written with
/// `run_command_and_check_ok` rather than the obvious `uid_store`.
///
/// `Session::uid_store` hands back a stream of `Fetch` items built by
/// `parse_fetches`, which is `take_while(filter)`-terminated: `filter_sync`
/// returns `tag != command_tag`, so the **tagged completion is what ends the
/// stream and is dropped inside the library**. A tagged `NO` or `BAD` therefore
/// never becomes an item, and the earlier
/// `while let Some(_item) = stream.next().await {}` could not have seen one
/// however carefully it inspected what it drained. The call only failed when
/// `uid_store` itself failed to *dispatch*, so a STORE the server actively
/// refused — expunged UID, mailbox selected read-only, per-account rate
/// limiting — was reported to every caller as success.
///
/// `run_command_and_check_ok` is the same public API `uid_copy` / `uid_mv`
/// already use, and sends byte-identical wire traffic (`uid_store` builds
/// exactly this string). It loops to the tagged response, calls
/// `check_status_ok`, and returns `Error::No` / `Error::Bad` on a refusal.
/// Untagged `FETCH` responses the STORE emits are forwarded to
/// `handle_unilateral`, whose catch-all arm is a lossy
/// `try_send(..).ok()` — so a full unsolicited channel drops them rather than
/// blocking us. We discarded those responses before this change anyway.
///
/// One assumption worth stating because nothing enforces it: `check_done_ok`
/// checks the status of **any** `Done` it meets, including one carrying a
/// stale tag from an earlier command. Every helper in this module fully drains
/// its own responses before the next command is sent, so the buffer is clean in
/// practice — but pipelining two commands on one session would break that
/// silently.
pub async fn store_flags(
    session: &mut ImapSession,
    uid: u32,
    add: bool,
    flags: &str,
) -> Result<(), AppError> {
    let op = if add { "+FLAGS" } else { "-FLAGS" };
    let command = format!("UID STORE {} {} ({})", uid, op, flags);

    session.run_command_and_check_ok(&command).await.map_err(|e| {
        // Name the command and the UID: a refusal and a dispatch failure read
        // identically from a log line otherwise, and they have different
        // remedies (the message is gone / the mailbox is read-only, versus the
        // connection is broken).
        AppError::Imap(format!("{} failed: {}", command, e))
    })?;

    Ok(())
}

/// Move a message to another folder using UID MOVE (or COPY+DELETE fallback).
pub async fn move_message(
    session: &mut ImapSession,
    uid: u32,
    destination: &str,
) -> Result<(), AppError> {
    // Try MOVE first (RFC 6851), fall back to COPY+DELETE
    let range = format!("{}", uid);
    match session.uid_mv(&range, destination).await {
        Ok(_) => Ok(()),
        Err(_) => {
            // Fallback: COPY + mark deleted + expunge
            session
                .uid_copy(&range, destination)
                .await
                .map_err(|e| AppError::Imap(format!("COPY failed: {}", e)))?;
            // Through `store_flags`, not a second inline `uid_store` + drain:
            // this had the identical swallowed-refusal bug (see its doc comment),
            // and here the consequence is worse than a wrong flag — EXPUNGE
            // follows, so a refused `\Deleted` used to mean the copy landed, the
            // original was never marked, and the message was silently duplicated
            // rather than moved. Failing here leaves the copy behind, which is
            // visible and recoverable; the old behaviour was neither.
            store_flags(session, uid, true, "\\Deleted").await?;
            expunge(session).await?;
            Ok(())
        }
    }
}

pub async fn expunge(session: &mut ImapSession) -> Result<(), AppError> {
    let stream = session
        .expunge()
        .await
        .map_err(|e| AppError::Imap(format!("EXPUNGE failed: {}", e)))?;
    futures::pin_mut!(stream);
    while stream.as_mut().next().await.is_some() {}
    Ok(())
}

/// Expunge exactly one UID (`UID EXPUNGE`, RFC 4315 UIDPLUS).
///
/// A mailbox-wide `EXPUNGE` also purges every message *another client* has
/// marked `\Deleted` in the folder — collateral the draft-replace path has no
/// business causing. Sent through `run_command_and_check_ok`, not the library's
/// `uid_expunge` stream: draining that stream swallows a tagged NO/BAD (gotcha
/// #38), and the BAD is the one signal this needs — it is how a server without
/// UIDPLUS answers an unknown command, and the only case where falling back to
/// the mailbox-wide form is correct. A NO is a refusal and propagates.
pub async fn uid_expunge(session: &mut ImapSession, uid: u32) -> Result<(), AppError> {
    let command = format!("UID EXPUNGE {}", uid);
    match session.run_command_and_check_ok(&command).await {
        Ok(()) => Ok(()),
        Err(async_imap::error::Error::Bad(resp)) => {
            log::warn!(
                "{} answered BAD ({}) — server lacks UIDPLUS; falling back to mailbox-wide EXPUNGE",
                command,
                resp.trim()
            );
            expunge(session).await
        }
        Err(e) => Err(AppError::Imap(format!("{} failed: {}", command, e))),
    }
}

/// Is `uid` currently in `folder`? Selects the folder, then `UID SEARCH UID n`.
///
/// This is the only way to tell: a `UID STORE` / `UID FETCH` on a UID the
/// server no longer holds is silently ignored (RFC 9051 §6.4.9 — UID commands
/// skip nonexistent UIDs), so "the command succeeded" says nothing about
/// whether the message exists.
pub async fn uid_exists(session: &mut ImapSession, folder: &str, uid: u32) -> Result<bool, AppError> {
    let _ = select_folder(session, folder).await?;
    let hits = session
        .uid_search(format!("UID {}", uid))
        .await
        .map_err(|e| AppError::Imap(format!("UID SEARCH UID {} failed: {}", uid, e)))?;
    Ok(hits.contains(&uid))
}

/// Create a new mailbox on the IMAP server.
pub async fn create_folder(session: &mut ImapSession, folder_name: &str) -> Result<(), AppError> {
    session
        .create(folder_name)
        .await
        .map_err(|e| AppError::Imap(format!("CREATE failed: {}", e)))?;
    Ok(())
}

/// Delete a mailbox on the IMAP server.
pub async fn delete_folder(session: &mut ImapSession, folder_name: &str) -> Result<(), AppError> {
    session
        .delete(folder_name)
        .await
        .map_err(|e| AppError::Imap(format!("DELETE failed: {}", e)))?;
    Ok(())
}

/// Rename a mailbox on the IMAP server.
pub async fn rename_folder(
    session: &mut ImapSession,
    from: &str,
    to: &str,
) -> Result<(), AppError> {
    session
        .rename(from, to)
        .await
        .map_err(|e| AppError::Imap(format!("RENAME failed: {}", e)))?;
    Ok(())
}

/// Disconnect cleanly from the IMAP server (5s timeout — drops session on hang).
///
/// `account_email` is required rather than optional: a dropped session's cost is
/// a server-side connection slot held against **that account**, and without
/// knowing which one, a drop can never be matched to the connect failure it
/// later causes. The parameter was added by changing this signature instead of
/// adding a second labelled function, so every call site is a compile error
/// until it says which account it is closing — the same reasoning that deleted
/// `connect_for_provider` rather than wrapping it (gotcha #41).
pub async fn disconnect(mut session: ImapSession, account_email: &str) -> Result<(), AppError> {
    match tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await {
        Ok(Ok(_)) => ledger_update(account_email, |l| l.closed_clean += 1),
        Ok(Err(e)) => {
            ledger_update(account_email, |l| l.dropped += 1);
            let l = ledger_for(account_email);
            log::warn!(
                "IMAP LOGOUT error for {account_email} (session dropped): {e} [dropped={} slot_suspect={}]",
                l.dropped,
                l.slot_suspect()
            );
        }
        Err(_) => {
            ledger_update(account_email, |l| l.dropped += 1);
            let l = ledger_for(account_email);
            log::warn!(
                "IMAP LOGOUT timed out after 5s for {account_email}, dropping session [dropped={} slot_suspect={}]",
                l.dropped,
                l.slot_suspect()
            );
        }
    }
    Ok(())
}

/// After APPEND, look up the new UID by Message-ID. Gmail can take 1-2s to index;
/// retry up to 3 times with a 1s sleep between attempts. `message_id_header` must
/// already include the angle brackets, e.g. `<abc@cxmail.app>`.
pub async fn find_uid_by_message_id(
    session: &mut ImapSession,
    folder: &str,
    message_id_header: &str,
) -> Result<u32, AppError> {
    let query = format!("HEADER Message-ID \"{}\"", message_id_header);
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
        let _ = select_folder(session, folder).await?;
        let hits = session
            .uid_search(&query)
            .await
            .map_err(|e| AppError::Imap(format!("UID SEARCH after APPEND failed: {}", e)))?;
        if let Some(uid) = hits.into_iter().max() {
            return Ok(uid);
        }
    }
    Err(AppError::Imap(format!(
        "APPEND succeeded but new draft UID not found in {} after 3 attempts",
        folder
    )))
}

/// Decode RFC 2047 encoded-words (e.g., =?UTF-8?Q?Hello?= or =?UTF-8?B?SGVsbG8=?=).
fn decode_rfc2047(input: &str) -> String {
    let mut result = String::new();
    let mut remaining = input;

    while let Some(start) = remaining.find("=?") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start..];

        // Find the encoding marker: =?charset?E? where E is Q or B
        // Skip past =?
        let after_start = &remaining[2..];

        // Find charset end (first ?)
        let Some(charset_end) = after_start.find('?') else {
            result.push_str("=?");
            remaining = &remaining[2..];
            continue;
        };

        let after_charset = &after_start[charset_end + 1..];

        // Find encoding marker end (should be Q? or B?)
        let Some(enc_end) = after_charset.find('?') else {
            result.push_str("=?");
            remaining = &remaining[2..];
            continue;
        };

        let encoding = &after_charset[..enc_end];
        let after_encoding = &after_charset[enc_end + 1..];

        // Find the closing ?= marker — search from the encoded text portion
        let Some(text_end) = after_encoding.find("?=") else {
            result.push_str("=?");
            remaining = &remaining[2..];
            continue;
        };

        let encoded = &after_encoding[..text_end];

        let decoded = match encoding.to_uppercase().as_str() {
            "Q" => {
                let mut bytes = Vec::new();
                let mut chars = encoded.chars();
                while let Some(c) = chars.next() {
                    if c == '=' {
                        let hex: String = chars.by_ref().take(2).collect();
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            bytes.push(byte);
                        }
                    } else if c == '_' {
                        bytes.push(b' ');
                    } else {
                        bytes.push(c as u8);
                    }
                }
                String::from_utf8_lossy(&bytes).to_string()
            }
            "B" => {
                use base64::Engine;
                match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Err(_) => encoded.to_string(),
                }
            }
            _ => encoded.to_string(),
        };

        result.push_str(&decoded);

        // Advance past the entire =?charset?E?text?= block
        // Total length: 2 (=?) + charset + 1 (?) + encoding + 1 (?) + text + 2 (?=)
        let total_len = 2 + charset_end + 1 + enc_end + 1 + text_end + 2;
        remaining = &remaining[total_len..];

        // Skip whitespace between adjacent encoded words
        if remaining.starts_with(' ') || remaining.starts_with('\t') {
            if let Some(next_start) = remaining.find("=?") {
                let between = &remaining[..next_start];
                if between.trim().is_empty() {
                    remaining = &remaining[next_start..];
                }
            }
        }
    }

    result.push_str(remaining);
    result
}

/// Decode bytes from IMAP envelope, handling RFC 2047 encoded-words.
fn decode_envelope_str(data: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(data).ok()?;
    let decoded = decode_rfc2047(s);
    Some(decoded)
}

/// Convert IMAP envelope addresses (`to`, `cc`) into a JSON array of
/// `{name, email}` objects matching the `EmailAddress` shape used elsewhere.
/// Used by sync to populate `messages.to_list` / `messages.cc_list` so the
/// per-recipient voice queries can match against sent mail.
fn envelope_addresses_to_json(
    addresses: Option<&Vec<async_imap::imap_proto::types::Address<'_>>>,
) -> String {
    let Some(addrs) = addresses else {
        return "[]".to_string();
    };
    let items: Vec<serde_json::Value> = addrs
        .iter()
        .filter_map(|addr| {
            let mailbox = addr
                .mailbox
                .as_ref()
                .and_then(|m| std::str::from_utf8(m).ok())
                .unwrap_or("")
                .trim();
            let host = addr
                .host
                .as_ref()
                .and_then(|h| std::str::from_utf8(h).ok())
                .unwrap_or("")
                .trim();
            if mailbox.is_empty() || host.is_empty() {
                return None;
            }
            let email = format!("{}@{}", mailbox, host);
            let name = addr
                .name
                .as_ref()
                .and_then(|n| decode_envelope_str(n))
                .filter(|s| !s.trim().is_empty());
            Some(serde_json::json!({
                "name": name,
                "email": email,
            }))
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

/// Parse an async-imap Fetch response into our header struct.
fn parse_fetch_to_header(uid: u32, fetch: &Fetch) -> ImapMessageHeader {
    let envelope = fetch.envelope();

    let subject = envelope
        .as_ref()
        .and_then(|e| e.subject.as_ref())
        .and_then(|s| decode_envelope_str(s))
        .map(|s| crate::email::parser::normalize_unstructured_header(&s));

    let (from_name, from_email) = envelope
        .as_ref()
        .and_then(|e| e.from.as_ref())
        .and_then(|addrs| addrs.first())
        .map(|addr| {
            let name = addr.name.as_ref().and_then(|n| decode_envelope_str(n));
            let mailbox = addr
                .mailbox
                .as_ref()
                .and_then(|m| std::str::from_utf8(m).ok())
                .unwrap_or("");
            let host = addr
                .host
                .as_ref()
                .and_then(|h| std::str::from_utf8(h).ok())
                .unwrap_or("");
            let email = format!("{}@{}", mailbox, host);
            (name, Some(email))
        })
        .unwrap_or((None, None));

    let date = envelope
        .as_ref()
        .and_then(|e| e.date.as_ref())
        .and_then(|d| std::str::from_utf8(d).ok())
        .map(|s| normalize_date_to_iso8601(s));

    let flag_list: Vec<async_imap::types::Flag> = fetch.flags().collect();
    let flags: Vec<String> = flag_list.iter().map(|f| format!("{:?}", f)).collect();
    let is_read = flag_list
        .iter()
        .any(|f| matches!(f, async_imap::types::Flag::Seen));
    let is_flagged = flag_list
        .iter()
        .any(|f| matches!(f, async_imap::types::Flag::Flagged));

    let size = fetch.size.unwrap_or(0);

    // Extract custom headers from the header fields body
    let header_text = fetch
        .header()
        .and_then(|h| std::str::from_utf8(h).ok())
        .unwrap_or("");

    let message_id = extract_header_value(header_text, "Message-ID");
    let in_reply_to = extract_header_value(header_text, "In-Reply-To");
    let references = extract_header_value(header_text, "References");
    let list_unsubscribe = extract_header_value(header_text, "List-Unsubscribe");
    let list_unsubscribe_post = extract_header_value(header_text, "List-Unsubscribe-Post");
    let draft_id = extract_header_value(header_text, crate::email::draft_local::DRAFT_ID_HEADER)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let precedence = extract_header_value(header_text, "Precedence");

    let to_list = envelope_addresses_to_json(envelope.as_ref().and_then(|e| e.to.as_ref()));
    let cc_list = envelope_addresses_to_json(envelope.as_ref().and_then(|e| e.cc.as_ref()));

    ImapMessageHeader {
        uid,
        subject,
        from_name,
        from_email,
        to_list,
        cc_list,
        date,
        message_id,
        in_reply_to,
        references,
        flags,
        size,
        is_read,
        is_flagged,
        list_unsubscribe,
        list_unsubscribe_post,
        draft_id,
        precedence,
    }
}

// Date normalization lives in `cxmail-core` because `db::schema`'s migrations
// re-normalize stored Date headers and must not depend on this crate.
// Re-exported so `email::imap::normalize_date_to_iso8601` still resolves.
pub use cxmail_core::mail::date::{normalize_date_to_iso8601, DATE_SENTINEL};

/// Extract a header value from raw header text.
fn extract_header_value(headers: &str, name: &str) -> Option<String> {
    let search = format!("{}:", name);
    for line in headers.lines() {
        if line.to_lowercase().starts_with(&search.to_lowercase()) {
            return Some(line[search.len()..].trim().to_string());
        }
    }
    None
}

fn build_uid_sequences(uids: &[u32], chunk_size: usize) -> Vec<String> {
    if uids.is_empty() || chunk_size == 0 {
        return Vec::new();
    }

    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    sorted.chunks(chunk_size).map(build_uid_sequence).collect()
}

fn build_uid_sequence(uids: &[u32]) -> String {
    if uids.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    let mut start = uids[0];
    let mut end = uids[0];

    for &uid in &uids[1..] {
        if uid == end.saturating_add(1) {
            end = uid;
            continue;
        }

        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{}:{}", start, end));
        }

        start = uid;
        end = uid;
    }

    if start == end {
        parts.push(start.to_string());
    } else {
        parts.push(format!("{}:{}", start, end));
    }

    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::{
        background_cooldown_remaining, build_uid_sequence, build_uid_sequences,
        classify_generic_folder, connect_failure_streak, heuristic_folder_type, ledger_for,
        ledger_update, normalize_date_to_iso8601, note_connect_outcome, special_use_folder_type,
        SessionLedger, NameAttribute, DATE_SENTINEL,
    };

    // --- background connect breaker (gotcha #46) -------------------------

    #[test]
    fn an_account_with_no_history_is_never_gated() {
        // The breaker must be invisible until something actually fails —
        // gating a healthy account would delay real mail for no reason.
        assert!(background_cooldown_remaining("never-seen@example.test").is_none());
    }

    #[test]
    fn consecutive_failures_arm_a_widening_cooldown() {
        let e = "breaker-widens@example.test";
        note_connect_outcome(e, false);
        let first = background_cooldown_remaining(e).expect("armed after one failure");
        note_connect_outcome(e, false);
        let second = background_cooldown_remaining(e).expect("still armed");
        assert!(
            second > first,
            "the hold-off must grow: {second:?} should exceed {first:?}"
        );
        assert_eq!(connect_failure_streak(e), 2);
    }

    #[test]
    fn one_successful_connect_clears_the_breaker_immediately() {
        // The recovery path. If a success did not fully reset both the streak
        // and the deadline, an account that came back would stay throttled by
        // us long after the server forgave it.
        let e = "breaker-clears@example.test";
        note_connect_outcome(e, false);
        note_connect_outcome(e, false);
        assert!(background_cooldown_remaining(e).is_some());

        note_connect_outcome(e, true);
        assert!(
            background_cooldown_remaining(e).is_none(),
            "a working connect must un-gate the account at once"
        );
        assert_eq!(connect_failure_streak(e), 0);
    }

    #[test]
    fn the_breaker_is_scoped_to_one_account() {
        // Gmail's limits are per-account; one refused account must not stop the
        // other eight from syncing.
        let bad = "breaker-isolated-bad@example.test";
        let good = "breaker-isolated-good@example.test";
        note_connect_outcome(bad, false);
        assert!(background_cooldown_remaining(bad).is_some());
        assert!(background_cooldown_remaining(good).is_none());
    }

    // --- session ledger (gotcha #46 instrumentation) ---------------------

    #[test]
    fn a_clean_close_leaves_nothing_unaccounted_and_nothing_suspect() {
        let l = SessionLedger { opened: 10, closed_clean: 10, dropped: 0, abandoned: 0 };
        assert_eq!(l.unaccounted(), 0);
        assert_eq!(l.slot_suspect(), 0);
    }

    #[test]
    fn a_dropped_session_is_accounted_for_but_still_slot_suspect() {
        // The distinction the whole investigation rests on: a drop is NOT a
        // clean close. It is fully ACCOUNTED (our socket is gone and we know
        // how it ended) while remaining slot-suspect, because LOGOUT failed and
        // the server may still hold it.
        let l = SessionLedger { opened: 10, closed_clean: 7, dropped: 3, abandoned: 0 };
        assert_eq!(l.unaccounted(), 0);
        assert_eq!(l.slot_suspect(), 3);
    }

    #[test]
    fn an_abandoned_session_is_accounted_for_and_is_equally_slot_suspect() {
        // The `idle.rs` case. It must NOT show up as unaccounted — that was the
        // bug: the ledger read these as live connections and pointed the
        // diagnosis at slot starvation on evidence that was really a counting
        // hole. No LOGOUT was attempted, so it is every bit as slot-suspect as
        // a `dropped` one.
        let l = SessionLedger { opened: 10, closed_clean: 4, dropped: 1, abandoned: 5 };
        assert_eq!(l.unaccounted(), 0, "abandoned sessions are accounted, not unknown");
        assert_eq!(l.slot_suspect(), 6, "no-LOGOUT counts alongside failed-LOGOUT");
    }

    #[test]
    fn unaccounted_counts_sessions_still_in_use_not_sessions_gmail_holds() {
        // Six sessions opened and still running is what an in-progress sync
        // looks like. The number is only alarming when it fails to settle back
        // toward one-per-account — and it says nothing about the server either
        // way, which is what `slot_suspect` is for.
        let l = SessionLedger { opened: 10, closed_clean: 4, dropped: 0, abandoned: 0 };
        assert_eq!(l.unaccounted(), 6);
        assert_eq!(l.slot_suspect(), 0, "in-use sessions are not slot-suspect");
    }

    #[test]
    fn unaccounted_is_signed_so_a_double_close_is_visible_not_clamped() {
        let l = SessionLedger { opened: 1, closed_clean: 2, dropped: 0, abandoned: 0 };
        assert_eq!(l.unaccounted(), -1, "double-counted closes must be visible");
    }

    #[test]
    fn the_guard_counts_a_session_that_leaves_scope_via_an_error_path() {
        // Mutation tripwire for the actual fix: remove the guard from
        // `run_idle_session` and `abandoned` stops moving, which is precisely
        // the state that produced the misleading log line.
        let email = "guard-drops@example.test";
        ledger_update(email, |l| l.opened += 1);
        {
            let _guard = super::SessionGuard::new(email);
            // scope exits by drop, standing in for `?`
        }
        let l = ledger_for(email);
        assert_eq!(l.abandoned, 1);
        assert_eq!(l.unaccounted(), 0, "the guard is what closes the hole");
    }

    #[test]
    fn a_disarmed_guard_leaves_the_close_to_disconnect() {
        // Double-counting would push `unaccounted` negative, which is why
        // disarm-then-disconnect is the required order.
        let email = "guard-disarmed@example.test";
        ledger_update(email, |l| l.opened += 1);
        {
            let mut guard = super::SessionGuard::new(email);
            guard.disarm();
        }
        ledger_update(email, |l| l.closed_clean += 1); // stands in for `disconnect`
        let l = ledger_for(email);
        assert_eq!(l.abandoned, 0);
        assert_eq!(l.unaccounted(), 0);
    }

    #[test]
    fn the_ledger_is_per_account_so_one_account_cannot_mask_another() {
        // Per-account is the point: Gmail caps connections per account, so a
        // leak on one must not be averaged away across nine. Unique keys keep
        // this independent of other tests sharing the global ledger.
        let a = "ledger-test-a@example.com";
        let b = "ledger-test-b@example.com";
        ledger_update(a, |l| l.opened += 3);
        ledger_update(a, |l| l.dropped += 2);
        ledger_update(b, |l| l.opened += 1);

        assert_eq!(ledger_for(a).opened, 3);
        assert_eq!(ledger_for(a).dropped, 2);
        assert_eq!(ledger_for(b).opened, 1);
        assert_eq!(ledger_for(b).dropped, 0);
        assert_eq!(ledger_for("never-seen@example.com"), SessionLedger::default());
    }

    #[test]
    fn build_uid_sequence_compacts_contiguous_ranges() {
        assert_eq!(build_uid_sequence(&[1, 2, 3, 8, 10, 11, 12]), "1:3,8,10:12");
    }

    #[test]
    fn build_uid_sequences_sorts_deduplicates_and_chunks() {
        let sequences = build_uid_sequences(&[12, 11, 11, 10, 3, 2, 1, 8], 3);

        assert_eq!(sequences, vec!["1:3", "8,10:11", "12"]);
    }

    #[test]
    fn normalize_date_passes_through_iso_and_rfc2822() {
        assert_eq!(
            normalize_date_to_iso8601("2026-03-25T10:05:36+00:00"),
            "2026-03-25T10:05:36+00:00"
        );
        assert_eq!(
            normalize_date_to_iso8601("Wed, 25 Mar 2026 10:05:36 +0000"),
            "2026-03-25T10:05:36+00:00"
        );
    }

    #[test]
    fn normalize_date_salvages_trailing_junk_after_offset() {
        // Junk glued to the offset with no space (BUG-01 sample).
        assert_eq!(
            normalize_date_to_iso8601("Wed, 25 Mar 2026 10:05:36 +0000.1731-577592"),
            "2026-03-25T10:05:36+00:00"
        );
        // Junk separated from the offset by spaces, with a non-UTC offset.
        assert_eq!(
            normalize_date_to_iso8601("Thu, 02 Apr 2026 00:30:54 +0200 . 714661948"),
            "2026-04-01T22:30:54+00:00"
        );
    }

    #[test]
    fn normalize_date_unrecoverable_yields_low_sentinel() {
        // No salvageable offset: must NOT return the raw string (which would
        // sort to the top of date-DESC views).
        assert_eq!(normalize_date_to_iso8601("_smtpDate . 714661948"), DATE_SENTINEL);
        assert_eq!(normalize_date_to_iso8601("not a date at all"), DATE_SENTINEL);
    }

    // ---- Generic-provider folder classification (RFC 6154 + name fallback) ----

    #[test]
    fn special_use_covers_every_rfc_6154_attribute() {
        use NameAttribute::*;
        assert_eq!(special_use_folder_type(&[Sent]), Some("sent"));
        assert_eq!(special_use_folder_type(&[Drafts]), Some("drafts"));
        assert_eq!(special_use_folder_type(&[Trash]), Some("trash"));
        assert_eq!(special_use_folder_type(&[Junk]), Some("spam"));
        assert_eq!(special_use_folder_type(&[Archive]), Some("archive"));
        assert_eq!(special_use_folder_type(&[Flagged]), Some("starred"));
        // \All is Gmail's "All Mail" shape; classify_gmail_folder already maps
        // that name to "archive", so the attribute must agree.
        assert_eq!(special_use_folder_type(&[All]), Some("archive"));
    }

    #[test]
    fn non_special_use_attributes_declare_nothing() {
        use NameAttribute::*;
        assert_eq!(special_use_folder_type(&[]), None);
        assert_eq!(special_use_folder_type(&[Marked, Unmarked, NoInferiors]), None);
        // Gmail's \Important is a vendor extension, not RFC 6154 — it must not
        // be read as a special use.
        assert_eq!(
            special_use_folder_type(&[Extension(std::borrow::Cow::Borrowed("\\Important"))]),
            None
        );
    }

    /// Two attributes on one mailbox must classify identically no matter which
    /// order the server listed them in.
    #[test]
    fn multiple_attributes_resolve_by_fixed_precedence_not_server_order() {
        use NameAttribute::*;
        assert_eq!(special_use_folder_type(&[All, Archive]), Some("archive"));
        assert_eq!(special_use_folder_type(&[Archive, All]), Some("archive"));
        assert_eq!(special_use_folder_type(&[Marked, Sent]), Some("sent"));
        assert_eq!(special_use_folder_type(&[Sent, Marked]), Some("sent"));
        assert_eq!(special_use_folder_type(&[Junk, Trash]), Some("trash"));
        assert_eq!(special_use_folder_type(&[Trash, Junk]), Some("trash"));
    }

    #[test]
    fn heuristics_match_the_last_hierarchy_segment() {
        assert_eq!(heuristic_folder_type("INBOX", None), "inbox");
        assert_eq!(heuristic_folder_type("Sent Items", None), "sent");
        assert_eq!(heuristic_folder_type("INBOX.Sent", Some(".")), "sent");
        assert_eq!(heuristic_folder_type("INBOX/Sent Mail", Some("/")), "sent");
        assert_eq!(heuristic_folder_type("INBOX.Drafts", Some(".")), "drafts");
        assert_eq!(heuristic_folder_type("Deleted Items", None), "trash");
        assert_eq!(heuristic_folder_type("Bin", None), "trash");
        assert_eq!(heuristic_folder_type("Junk E-mail", None), "spam");
        assert_eq!(heuristic_folder_type("All Mail", None), "archive");
        assert_eq!(heuristic_folder_type("Archive", None), "archive");
    }

    #[test]
    fn heuristics_are_case_insensitive_and_survive_a_missing_delimiter() {
        assert_eq!(heuristic_folder_type("sent items", None), "sent");
        assert_eq!(heuristic_folder_type("SENT MAIL", None), "sent");
        // A plain LIST may omit the delimiter; both common ones still split.
        assert_eq!(heuristic_folder_type("INBOX.Sent", None), "sent");
        assert_eq!(heuristic_folder_type("INBOX/Trash", None), "trash");
    }

    /// The load-bearing negative. A substring match would make "Sent to Legal"
    /// the account's Sent folder — and `db::nudges` computes "my last message
    /// in this thread" out of Sent, so the follow-up lane would go quiet with
    /// no error anywhere.
    #[test]
    fn a_folder_merely_containing_a_keyword_is_not_special() {
        assert_eq!(heuristic_folder_type("Sent to Legal", None), "other");
        assert_eq!(heuristic_folder_type("Drafts of Contracts", None), "other");
        assert_eq!(heuristic_folder_type("Archived Invoices", None), "other");
        assert_eq!(heuristic_folder_type("Trashy Newsletters", None), "other");
        assert_eq!(heuristic_folder_type("INBOX.Sent to Legal", Some(".")), "other");
        assert_eq!(heuristic_folder_type("Clients", None), "other");
    }

    /// Precedence, both directions: what the server SAYS beats what the name
    /// suggests — including when the name would have said something else.
    #[test]
    fn a_server_declaration_outranks_the_name_in_both_directions() {
        use NameAttribute::*;
        // A localized name the heuristics cannot possibly know.
        assert_eq!(classify_generic_folder("Papierkorb", None, &[Trash]), "trash");
        assert_eq!(classify_generic_folder("Gesendet", None, &[Sent]), "sent");
        // ...and the reverse: a familiar name that the server says is something else.
        assert_eq!(classify_generic_folder("Trash", None, &[Archive]), "archive");
        assert_eq!(classify_generic_folder("Sent", None, &[Junk]), "spam");
        // With no attributes it falls through to the name.
        assert_eq!(classify_generic_folder("Papierkorb", None, &[]), "other");
        assert_eq!(classify_generic_folder("Trash", None, &[]), "trash");
    }
}
