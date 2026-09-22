//! Mail server autodiscovery.
//!
//! Answers "where does this email address get its mail?" so a user with a
//! custom domain doesn't have to know their own IMAP hostname. Tiers, in order,
//! first hit wins:
//!
//! 0. **Curated presets** (`providers::PRESETS`) — nine providers we hand-wrote,
//!    because they carry app-password `hint` text the ISPDB has no field for.
//! 1. **Bundled ISPDB** — a vendored, unmodified MPL-2.0 snapshot of the
//!    Thunderbird autoconfig database (`resources/ispdb`, embedded by
//!    `build.rs`). Deliberately NOT fetched from `autoconfig.thunderbird.net`
//!    at runtime: see that directory's README. Local, offline, instant.
//! 2. **ISP-hosted autoconfig** — the user's OWN domain, over HTTPS only.
//! 3. **DNS SRV (RFC 6186)**, then **MX** → retry tiers 1–2 against the mail
//!    operator's domain, which is what resolves "my custom domain is hosted by
//!    someone else".
//!
//! Every tier is either local or talks only to the user's own domain. Nothing
//! here contacts a third party.
//!
//! **Discovery never connects and never sees the password.** It returns
//! settings for the setup form to display; the user sees the hostname before
//! any credential is sent. That is the mitigation for tier 3, where DNS is
//! spoofable without DNSSEC.

use crate::email::providers;
use serde::Serialize;

/// The vendored ISPDB, as `&[(domain, xml)]` sorted by domain.
mod bundled {
    include!(concat!(env!("OUT_DIR"), "/ispdb_table.rs"));
}

/// Server settings discovered for an address. Mirrors what the setup form
/// needs, and maps 1:1 onto `ImapAccountSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredConfig {
    pub display_name: Option<String>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: &'static str,
    /// `None` = authenticate as the email address itself.
    pub imap_username: Option<String>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: &'static str,
    pub smtp_username: Option<String>,
    /// Which tier answered — surfaced in the UI so the user knows whether this
    /// came from us, from their own domain, or from DNS.
    pub source: &'static str,
    /// Preset hint text, when tier 0 answered.
    pub hint: Option<String>,
}

/// The domain part of an address, lowercased.
pub fn domain_of(email: &str) -> Option<String> {
    let (_, domain) = email.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

fn local_part_of(email: &str) -> Option<&str> {
    email.rsplit_once('@').map(|(local, _)| local)
}

/// Resolve a Mozilla `<username>` template.
///
/// `%EMAILADDRESS%` means "log in with the whole address", which is our
/// `None` (the connect path already falls back to `account.email`). Storing
/// the address literally would work but would silently stop tracking the
/// account's email if it were ever corrected.
fn resolve_username(template: &str, email: &str) -> Option<String> {
    match template.trim() {
        "%EMAILADDRESS%" | "" => None,
        "%EMAILLOCALPART%" => local_part_of(email).map(str::to_string),
        // Tier 2 passes `?emailaddress=`, and some providers (verified against
        // Migadu) substitute the template SERVER-side — so the response holds
        // the literal address rather than `%EMAILADDRESS%`. Treat that as the
        // same thing, or we store a frozen copy of the address that stops
        // tracking the account if the email is ever corrected.
        literal if literal.eq_ignore_ascii_case(email.trim()) => None,
        literal => Some(literal.to_string()),
    }
}

/// Map Mozilla's `socketType` onto our stored security value.
///
/// `plain` returns `None` — an unencrypted server is one CXMail declines to
/// talk to, and `TlsMode` cannot even represent it.
fn socket_type_to_security(socket_type: &str) -> Option<&'static str> {
    match socket_type.trim().to_ascii_uppercase().as_str() {
        "SSL" => Some("implicit"),
        "STARTTLS" => Some("starttls"),
        _ => None,
    }
}

/// Is this an auth mechanism we can actually perform?
///
/// We do IMAP `LOGIN` / SMTP `AUTH PLAIN`, i.e. the password crosses an
/// already-encrypted channel. `password-encrypted` means CRAM-MD5 or similar,
/// which we do not implement; `OAuth2` is handled by the dedicated providers,
/// not the generic path. Accepting either here would produce settings that
/// look right and fail at login.
fn auth_supported(auth: &str) -> bool {
    matches!(
        auth.trim().to_ascii_lowercase().as_str(),
        "password-cleartext" | "plain" | ""
    )
}

/// Pick the best usable server entry from a `clientConfig` document.
///
/// Prefers implicit TLS over STARTTLS, because CXMail cannot do IMAP STARTTLS
/// yet (gotcha #41) — and the schema explicitly allows several entries, so a
/// provider offering both must resolve to the one we can use. Measured against
/// the bundled snapshot: 116 of 120 IMAP providers offer implicit TLS, so this
/// preference costs almost nothing.
fn pick_server<'a>(
    doc: &'a roxmltree::Document<'a>,
    tag: &str,
    want_type: &str,
    email: &str,
) -> Option<(String, u16, &'static str, Option<String>)> {
    let mut best: Option<(String, u16, &'static str, Option<String>)> = None;

    for node in doc.descendants().filter(|n| n.has_tag_name(tag)) {
        // Skip POP3 outright — CXMail is an IMAP client.
        if node.attribute("type").map(str::to_ascii_lowercase).as_deref() != Some(want_type) {
            continue;
        }
        let child = |name: &str| {
            node.children()
                .find(|c| c.has_tag_name(name))
                .and_then(|c| c.text())
                .map(str::trim)
        };

        let Some(host) = child("hostname").filter(|h| !h.is_empty()) else {
            continue;
        };
        let Some(port) = child("port").and_then(|p| p.parse::<u16>().ok()).filter(|p| *p != 0)
        else {
            continue;
        };
        let Some(security) = child("socketType").and_then(socket_type_to_security) else {
            continue; // plain, or unrecognised — not usable
        };
        // A server may list several <authentication> options; usable if ANY is.
        let auths: Vec<&str> = node
            .children()
            .filter(|c| c.has_tag_name("authentication"))
            .filter_map(|c| c.text())
            .collect();
        if !auths.is_empty() && !auths.iter().any(|a| auth_supported(a)) {
            continue;
        }
        let username = child("username")
            .and_then(|t| resolve_username(t, email));

        let candidate = (host.to_ascii_lowercase(), port, security, username);
        // First usable wins, unless a later entry upgrades STARTTLS -> implicit.
        match &best {
            None => best = Some(candidate),
            Some((_, _, "starttls", _)) if security == "implicit" => best = Some(candidate),
            _ => {}
        }
    }
    best
}

/// Parse a Mozilla `clientConfig` document into settings we can act on.
///
/// Returns `None` when the document has no IMAP server we can use — a
/// POP3-only provider, or one that is plaintext-only.
pub fn parse_client_config(
    xml: &str,
    email: &str,
    source: &'static str,
) -> Option<DiscoveredConfig> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let (imap_host, imap_port, imap_security, imap_username) =
        pick_server(&doc, "incomingServer", "imap", email)?;
    let (smtp_host, smtp_port, smtp_security, smtp_username) =
        pick_server(&doc, "outgoingServer", "smtp", email)?;

    let display_name = doc
        .descendants()
        .find(|n| n.has_tag_name("displayName"))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(DiscoveredConfig {
        display_name,
        imap_host,
        imap_port,
        imap_security,
        imap_username,
        smtp_host,
        smtp_port,
        smtp_security,
        smtp_username,
        source,
        hint: None,
    })
}

/// Every domain the bundled snapshot answers for, mapped to its XML.
///
/// Built once. Indexes BOTH the filename and each `<domain>` element inside —
/// one file routinely covers several domains (`gmx.net.xml` also serves
/// gmx.de and gmx.com), so filename-only lookup would silently lose most of
/// the coverage we just vendored.
fn bundled_index() -> &'static std::collections::HashMap<String, &'static str> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<String, &'static str>> =
        std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        for (file_domain, xml) in bundled::ISPDB {
            map.insert((*file_domain).to_string(), *xml);
            if let Ok(doc) = roxmltree::Document::parse(xml) {
                for node in doc.descendants().filter(|n| n.has_tag_name("domain")) {
                    if let Some(d) = node.text().map(|t| t.trim().to_ascii_lowercase()) {
                        if !d.is_empty() {
                            map.entry(d).or_insert(*xml);
                        }
                    }
                }
            }
        }
        map
    })
}

/// Look up a domain in the bundled ISPDB snapshot.
pub fn lookup_bundled(domain: &str) -> Option<&'static str> {
    bundled_index().get(&domain.to_ascii_lowercase()).copied()
}

/// Tiers 0 and 1 — entirely local, no network, always safe to call.
pub fn discover_offline(email: &str) -> Option<DiscoveredConfig> {
    let domain = domain_of(email)?;

    // Tier 0: our curated table. Preferred over ISPDB because it carries the
    // app-password hint, which is the difference between a working setup and
    // a mystery auth failure.
    if let Some(p) = providers::preset_for_email(email) {
        return Some(DiscoveredConfig {
            display_name: Some(p.label.to_string()),
            imap_host: p.imap_host.to_string(),
            imap_port: p.imap_port,
            imap_security: match p.imap_security {
                "implicit" => "implicit",
                _ => "starttls",
            },
            imap_username: None,
            smtp_host: p.smtp_host.to_string(),
            smtp_port: p.smtp_port,
            smtp_security: match p.smtp_security {
                "implicit" => "implicit",
                _ => "starttls",
            },
            smtp_username: None,
            source: "preset",
            hint: Some(p.hint.to_string()),
        });
    }

    // Tier 1: the vendored snapshot.
    lookup_bundled(&domain).and_then(|xml| parse_client_config(xml, email, "ispdb"))
}

/// Per-request budget. Account setup is interactive, so a hung DNS server or a
/// black-holed host must fall through quickly rather than stall the form.
const TIER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Tier 2 — autoconfig published by the user's OWN domain.
///
/// **HTTPS only.** Thunderbird historically also tried plain HTTP here, which
/// lets anyone on the path hand back a config pointing at their own server and
/// collect the user's password. There is no http fallback, deliberately.
///
/// The address is passed as a query parameter because some providers vary the
/// response by mailbox — and it goes only to the user's own mail provider,
/// which already knows it.
async fn fetch_isp_hosted(domain: &str, email: &str) -> Option<DiscoveredConfig> {
    let client = reqwest::Client::builder()
        .timeout(TIER_TIMEOUT)
        .user_agent("CXMail")
        .build()
        .ok()?;
    let encoded = urlencoding::encode(email);
    let urls = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={encoded}"),
        format!(
            "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={encoded}"
        ),
    ];
    for url in urls {
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(body) = resp.text().await else { continue };
        if let Some(cfg) = parse_client_config(&body, email, "autoconfig") {
            log::info!("autoconfig: {} resolved via {}", domain, url);
            return Some(cfg);
        }
    }
    None
}

fn resolver() -> Option<hickory_resolver::TokioResolver> {
    hickory_resolver::TokioResolver::builder_tokio()
        .map(|b| b.build())
        .map_err(|e| log::warn!("autoconfig: DNS resolver unavailable: {e}"))
        .ok()
}

/// Tier 3a — RFC 6186 SRV records.
///
/// `_imaps._tcp` is implicit TLS and usable; `_imap._tcp` is the STARTTLS/plain
/// variant and is deliberately ignored, since we cannot do IMAP STARTTLS.
/// A target of `.` means "this service is explicitly not offered" (RFC 6186 §6)
/// and must not be treated as a hostname.
async fn lookup_srv(domain: &str) -> Option<DiscoveredConfig> {
    let resolver = resolver()?;

    async fn one(
        resolver: &hickory_resolver::TokioResolver,
        name: String,
    ) -> Option<(String, u16)> {
        let lookup = tokio::time::timeout(TIER_TIMEOUT, resolver.srv_lookup(name))
            .await
            .ok()?
            .ok()?;
        // Lowest priority value wins, per RFC 2782.
        let mut best: Option<(u16, String, u16)> = None;
        for srv in lookup.iter() {
            if srv.target().is_root() {
                continue; // "." = service not provided
            }
            let host = srv.target().to_string().trim_end_matches('.').to_ascii_lowercase();
            if host.is_empty() || srv.port() == 0 {
                continue;
            }
            if best.as_ref().is_none_or(|(p, _, _)| srv.priority() < *p) {
                best = Some((srv.priority(), host, srv.port()));
            }
        }
        best.map(|(_, h, p)| (h, p))
    }

    let (imap_host, imap_port) = one(&resolver, format!("_imaps._tcp.{domain}")).await?;

    // Submission: prefer implicit (RFC 8314 _submissions) then STARTTLS.
    let (smtp_host, smtp_port, smtp_security) =
        match one(&resolver, format!("_submissions._tcp.{domain}")).await {
            Some((h, p)) => (h, p, "implicit"),
            None => {
                let (h, p) = one(&resolver, format!("_submission._tcp.{domain}")).await?;
                (h, p, "starttls")
            }
        };

    log::info!("autoconfig: {domain} resolved via SRV -> {imap_host}:{imap_port}");
    Some(DiscoveredConfig {
        display_name: None,
        imap_host,
        imap_port,
        imap_security: "implicit",
        imap_username: None,
        smtp_host,
        smtp_port,
        smtp_security,
        smtp_username: None,
        source: "srv",
        hint: None,
    })
}

/// Progressively shorter suffixes of a hostname, longest first, down to two
/// labels: `aspmx.l.google.com` -> `l.google.com` -> `google.com`.
///
/// Used to turn an MX hostname into something the ISPDB might know. This is
/// deliberately not "strip one label" — that yields `l.google.com`, which no
/// database has — and deliberately not a Public Suffix List, which would be a
/// whole dependency and a data file to keep current for one lookup.
fn domain_suffixes(host: &str) -> Vec<String> {
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    (0..labels.len().saturating_sub(1))
        .map(|i| labels[i..].join("."))
        .filter(|d| d.matches('.').count() >= 1)
        .collect()
}

/// Tier 3b — MX, then retry the earlier tiers against the mail operator.
///
/// This is what resolves "my custom domain is hosted by Fastmail/Migadu":
/// `cxventures.io` has no autoconfig of its own, but its MX points somewhere
/// that does.
async fn lookup_via_mx(domain: &str, email: &str) -> Option<DiscoveredConfig> {
    let resolver = resolver()?;
    let lookup = tokio::time::timeout(TIER_TIMEOUT, resolver.mx_lookup(domain))
        .await
        .ok()?
        .ok()?;

    let mut hosts: Vec<(u16, String)> = lookup
        .iter()
        .map(|mx| {
            (
                mx.preference(),
                mx.exchange().to_string().trim_end_matches('.').to_ascii_lowercase(),
            )
        })
        .filter(|(_, h)| !h.is_empty())
        .collect();
    hosts.sort_by_key(|(pref, _)| *pref);

    for (_, host) in hosts.iter().take(3) {
        for candidate in domain_suffixes(host) {
            // Skip the domain we already tried, or we just redo tier 1 for nothing.
            if candidate == domain {
                continue;
            }
            if let Some(xml) = lookup_bundled(&candidate) {
                if let Some(mut cfg) = parse_client_config(xml, email, "mx") {
                    log::info!("autoconfig: {domain} resolved via MX {host} -> {candidate}");
                    cfg.source = "mx";
                    return Some(cfg);
                }
            }
        }
        // The operator may publish autoconfig even if the ISPDB doesn't know it.
        if let Some(base) = domain_suffixes(host).last() {
            if base != domain {
                if let Some(mut cfg) = fetch_isp_hosted(base, email).await {
                    cfg.source = "mx";
                    return Some(cfg);
                }
            }
        }
    }
    None
}

/// Full cascade. Offline tiers first, so a known domain never touches the
/// network at all.
pub async fn discover(email: &str) -> Option<DiscoveredConfig> {
    let domain = domain_of(email)?;

    if let Some(cfg) = discover_offline(email) {
        return Some(cfg);
    }
    if let Some(cfg) = fetch_isp_hosted(&domain, email).await {
        return Some(cfg);
    }
    if let Some(cfg) = lookup_srv(&domain).await {
        return Some(cfg);
    }
    lookup_via_mx(&domain, email).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const FASTMAIL_STYLE: &str = r#"
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <domain>example.net</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn parses_a_standard_client_config() {
        let c = parse_client_config(FASTMAIL_STYLE, "chris@example.com", "ispdb").unwrap();
        assert_eq!(c.imap_host, "imap.example.com");
        assert_eq!(c.imap_port, 993);
        assert_eq!(c.imap_security, "implicit");
        assert_eq!(c.smtp_host, "smtp.example.com");
        assert_eq!(c.smtp_port, 465);
        assert_eq!(c.display_name.as_deref(), Some("Example Mail"));
        // %EMAILADDRESS% means "log in as the address" = our None.
        assert_eq!(c.imap_username, None);
    }

    /// Some providers substitute %EMAILADDRESS% server-side when tier 2 passes
    /// `?emailaddress=`. A literal that just IS the address must normalise to
    /// None, not be frozen into the account row. Verified live against Migadu.
    #[test]
    fn a_server_substituted_address_is_not_stored_as_a_literal_username() {
        let xml = FASTMAIL_STYLE.replace("%EMAILADDRESS%", "chris@example.com");
        let c = parse_client_config(&xml, "chris@example.com", "autoconfig").unwrap();
        assert_eq!(c.imap_username, None);
        assert_eq!(c.smtp_username, None);

        // Case differences are still the same address.
        let xml = FASTMAIL_STYLE.replace("%EMAILADDRESS%", "Chris@Example.COM");
        let c = parse_client_config(&xml, "chris@example.com", "autoconfig").unwrap();
        assert_eq!(c.imap_username, None);

        // ...but a genuinely different login must survive.
        let xml = FASTMAIL_STYLE.replace("%EMAILADDRESS%", "chris.mailbox");
        let c = parse_client_config(&xml, "chris@example.com", "autoconfig").unwrap();
        assert_eq!(c.imap_username.as_deref(), Some("chris.mailbox"));
    }

    #[test]
    fn email_localpart_template_becomes_a_real_username() {
        let xml = FASTMAIL_STYLE.replace("%EMAILADDRESS%", "%EMAILLOCALPART%");
        let c = parse_client_config(&xml, "chris@example.com", "ispdb").unwrap();
        assert_eq!(c.imap_username.as_deref(), Some("chris"));
        assert_eq!(c.smtp_username.as_deref(), Some("chris"));
    }

    /// The schema allows several entries. We cannot do IMAP STARTTLS, so a
    /// provider offering both must resolve to the SSL one regardless of order.
    #[test]
    fn implicit_tls_is_preferred_over_starttls_in_either_order() {
        let starttls = r#"
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname><port>143</port>
      <socketType>STARTTLS</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>"#;
        let ssl = r#"
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname><port>993</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>"#;
        for (a, b) in [(starttls, ssl), (ssl, starttls)] {
            let xml = FASTMAIL_STYLE.replace(
                "<incomingServer type=\"imap\">",
                &format!("{a}{b}<incomingServer type=\"imap\">"),
            );
            let c = parse_client_config(&xml, "chris@example.com", "ispdb").unwrap();
            assert_eq!(c.imap_port, 993, "must pick the SSL entry");
            assert_eq!(c.imap_security, "implicit");
        }
    }

    #[test]
    fn a_starttls_only_provider_still_parses_but_says_so() {
        let xml = FASTMAIL_STYLE
            .replace("<socketType>SSL</socketType>\n      <authentication>password-cleartext</authentication>\n      <username>%EMAILADDRESS%</username>\n    </incomingServer>",
                     "<socketType>STARTTLS</socketType>\n      <authentication>password-cleartext</authentication>\n      <username>%EMAILADDRESS%</username>\n    </incomingServer>");
        let c = parse_client_config(&xml, "chris@example.com", "ispdb").unwrap();
        // Surfaced honestly; the connect path rejects it with a clear message
        // rather than the parser pretending the provider is unusable.
        assert_eq!(c.imap_security, "starttls");
    }

    /// A plaintext server is one CXMail refuses to talk to. It must not be
    /// downgraded into a "working" config.
    #[test]
    fn plaintext_only_is_not_discoverable() {
        let xml = FASTMAIL_STYLE.replace("<socketType>SSL</socketType>", "<socketType>plain</socketType>");
        assert!(parse_client_config(&xml, "chris@example.com", "ispdb").is_none());
    }

    #[test]
    fn pop3_only_providers_yield_nothing() {
        let xml = FASTMAIL_STYLE.replace("incomingServer type=\"imap\"", "incomingServer type=\"pop3\"");
        assert!(parse_client_config(&xml, "chris@example.com", "ispdb").is_none());
    }

    /// CRAM-MD5 etc. would produce settings that look right and fail at login.
    #[test]
    fn unsupported_auth_mechanisms_are_refused() {
        let xml = FASTMAIL_STYLE
            .replace("<authentication>password-cleartext</authentication>", "<authentication>password-encrypted</authentication>");
        assert!(parse_client_config(&xml, "chris@example.com", "ispdb").is_none());
    }

    #[test]
    fn domain_extraction_is_forgiving_about_case_and_trailing_dots() {
        assert_eq!(domain_of("Chris@Example.COM.").as_deref(), Some("example.com"));
        assert_eq!(domain_of("no-at-sign"), None);
        assert_eq!(domain_of("trailing@"), None);
    }

    // ---- against the real vendored snapshot ----

    #[test]
    fn the_bundled_snapshot_is_actually_embedded() {
        // A build.rs that silently produced an empty table would make every
        // lookup miss and look like "this domain just isn't covered".
        assert!(
            bundled::ISPDB.len() > 100,
            "expected the vendored ISPDB, got {} entries",
            bundled::ISPDB.len()
        );
    }

    #[test]
    fn a_real_provider_resolves_from_the_snapshot() {
        let c = discover_offline("someone@gmx.net").expect("gmx.net is in the ISPDB");
        assert!(c.imap_host.contains("gmx"), "got {}", c.imap_host);
        assert_eq!(c.imap_security, "implicit");
        assert_eq!(c.source, "preset", "gmx is also one of our curated presets");
    }

    /// One file covers several domains via <domain> elements. Indexing only by
    /// filename would throw most of the vendored coverage away.
    #[test]
    fn alias_domains_inside_a_config_are_indexed_too() {
        let idx = bundled_index();
        assert!(
            idx.len() > bundled::ISPDB.len(),
            "alias domains should expand the index beyond the file count ({} vs {})",
            idx.len(),
            bundled::ISPDB.len()
        );
    }

    #[test]
    fn curated_presets_win_over_the_ispdb() {
        // Both know gmail.com; ours carries the app-password hint.
        let c = discover_offline("chris@gmail.com").unwrap();
        assert_eq!(c.source, "preset");
        assert!(c.hint.is_some(), "the preset hint is why tier 0 exists");
    }

    #[test]
    fn an_unknown_domain_discovers_nothing_offline() {
        assert!(discover_offline("chris@cxventures.io").is_none());
    }

    /// Every config we ship must either parse into usable settings or be
    /// deliberately unusable (POP3-only / plaintext). A panic or a malformed
    /// file would be a build-time landmine.
    #[test]
    fn every_bundled_config_parses_without_panicking() {
        let mut usable = 0;
        for (domain, xml) in bundled::ISPDB {
            assert!(
                roxmltree::Document::parse(xml).is_ok(),
                "{domain} is not well-formed XML"
            );
            if parse_client_config(xml, &format!("user@{domain}"), "ispdb").is_some() {
                usable += 1;
            }
        }
        // Measured at vendoring time: 116 of 120 IMAP providers are usable.
        assert!(usable > 100, "only {usable} bundled configs are usable");
    }

    // ---- MX suffix walking (tier 3b) ----

    /// Turning an MX hostname into something the ISPDB knows is the whole
    /// point of tier 3b. "Strip one label" is the obvious approach and it is
    /// wrong: aspmx.l.google.com -> l.google.com, which no database has.
    #[test]
    fn mx_suffixes_walk_down_to_a_registrable_looking_domain() {
        assert_eq!(
            domain_suffixes("aspmx.l.google.com"),
            vec!["aspmx.l.google.com", "l.google.com", "google.com"]
        );
        assert_eq!(
            domain_suffixes("in1-smtp.messagingengine.com"),
            vec!["in1-smtp.messagingengine.com", "messagingengine.com"]
        );
        assert_eq!(domain_suffixes("mx.example.com"), vec!["mx.example.com", "example.com"]);
    }

    #[test]
    fn mx_suffixes_never_yield_a_bare_tld() {
        // "com" must never be looked up — it would match nothing at best and
        // the wrong provider at worst.
        for host in ["example.com", "a.b.c.d.example.com", "weird..example.com"] {
            for s in domain_suffixes(host) {
                assert!(s.contains('.'), "{s:?} from {host:?} is not a domain");
                assert_ne!(s, "com");
            }
        }
        assert!(domain_suffixes("localhost").is_empty());
        assert!(domain_suffixes("").is_empty());
    }

    /// A real MX chain end-to-end through the bundled snapshot: a custom
    /// domain hosted by a provider the ISPDB knows should resolve, without any
    /// autoconfig published by the custom domain itself.
    #[test]
    fn an_mx_suffix_can_hit_the_bundled_snapshot() {
        // gmx is in the snapshot; a hypothetical MX at mx01.gmx.net walks to it.
        let hit = domain_suffixes("mx01.gmx.net")
            .into_iter()
            .find_map(|d| lookup_bundled(&d));
        assert!(hit.is_some(), "walking mx01.gmx.net should reach gmx.net");
    }

    /// The offline tiers must never touch the network — that is what makes
    /// `discover_offline` safe to call on every keystroke in the setup form.
    #[tokio::test]
    async fn a_preset_domain_short_circuits_before_any_network_tier() {
        // If this ever hit the network it would take seconds, not microseconds.
        let start = std::time::Instant::now();
        let c = discover("chris@gmail.com").await.unwrap();
        assert_eq!(c.source, "preset");
        assert!(
            start.elapsed() < std::time::Duration::from_millis(500),
            "preset lookup took {:?} — did it fall through to a network tier?",
            start.elapsed()
        );
    }
}
