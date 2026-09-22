//! Transport security and server presets for the generic `imap` provider.
//!
//! [`TlsMode`] lives here rather than in `email::imap` because `email::smtp`
//! needs it too and neither should own the other's vocabulary.

use crate::error::AppError;

/// How to establish TLS on a mail connection.
///
/// **There is deliberately no `Plain` / `None` variant, and adding one is not
/// a small change.** The moment this enum can describe an unencrypted
/// connection, something in the UI will eventually offer it, and CXMail's
/// TLS-enforced invariant becomes a checkbox. A server that genuinely cannot
/// do TLS is a server CXMail declines to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// TLS from the first byte — IMAPS 993, SMTPS 465.
    Implicit,
    /// Plaintext greeting, then an in-band upgrade — IMAP 143, SMTP 587.
    StartTls,
}

impl TlsMode {
    /// Parse the value stored in `accounts.imap_security` / `smtp_security`.
    ///
    /// Unknown values are an error rather than a silent fallback: guessing
    /// here means guessing the handshake, and the failure mode of guessing
    /// wrong on the permissive side is a plaintext-looking connection.
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "implicit" => Ok(TlsMode::Implicit),
            "starttls" => Ok(TlsMode::StartTls),
            other => Err(AppError::General(format!(
                "Unknown transport security {:?} — expected \"implicit\" or \"starttls\"",
                other
            ))),
        }
    }

    /// `true` when TLS is established before the greeting.
    ///
    /// This is exactly `mail_send`'s `implicit_tls` flag.
    pub fn is_implicit(self) -> bool {
        matches!(self, TlsMode::Implicit)
    }
}

/// A known provider's server settings, keyed by the email domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailPreset {
    /// Stable id, also the value the UI sends back.
    pub id: &'static str,
    pub label: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: &'static str,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: &'static str,
    /// Domains that select this preset. Matched case-insensitively against the
    /// part after `@`.
    pub domains: &'static [&'static str],
    /// Shown under the password field — every one of these hosts wants an
    /// app-specific password, not the account password, and saying so is the
    /// difference between a working setup and a mystery auth failure.
    pub hint: &'static str,
}

/// Presets for hosts worth shipping. Deliberately excluded:
///
/// - **Proton Bridge.** Its `127.0.0.1:1143` listener presents a self-signed
///   per-install certificate. Trusting it needs either a verification bypass —
///   which becomes a checkbox users point at corporate MITM proxies — or CA
///   pinning against a third-party app's internal files, in BOTH native-tls
///   (IMAP) and rustls (SMTP). If it is ever wanted, ship it as a distinct
///   `proton-bridge` provider with pinning, never as a generic trust toggle.
/// - **Anything POP3-only.** CXMail has no POP3 client.
///
/// Gmail is present on purpose: an app-password connection is not OAuth, so it
/// consumes none of the Cloud project's 100-consent lifetime cap and shows no
/// unverified-app warning. OAuth stays the default in the picker.
pub const PRESETS: &[MailPreset] = &[
    MailPreset {
        id: "gmail",
        label: "Gmail (app password)",
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_security: "implicit",
        // Byte-identical to the OAuth Gmail path in `smtp::get_smtp_transport`
        // so both take the same wire path. Pinned by a test.
        smtp_host: "smtp.gmail.com",
        smtp_port: 587,
        smtp_security: "starttls",
        domains: &["gmail.com", "googlemail.com"],
        hint: "Requires 2-Step Verification, then an App Password from myaccount.google.com.",
    },
    MailPreset {
        id: "fastmail",
        label: "Fastmail",
        imap_host: "imap.fastmail.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.fastmail.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["fastmail.com", "fastmail.fm", "fastmail.us", "messagingengine.com"],
        hint: "Create an app password under Settings → Privacy & Security → App Passwords.",
    },
    MailPreset {
        id: "icloud",
        label: "iCloud Mail",
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
        smtp_security: "starttls",
        domains: &["icloud.com", "me.com", "mac.com"],
        hint: "Requires an app-specific password from appleid.apple.com.",
    },
    MailPreset {
        id: "yahoo",
        label: "Yahoo Mail",
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["yahoo.com", "yahoo.co.uk", "ymail.com", "rocketmail.com"],
        hint: "Generate an app password under Account Security.",
    },
    MailPreset {
        id: "aol",
        label: "AOL Mail",
        imap_host: "imap.aol.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.aol.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["aol.com"],
        hint: "Generate an app password under Account Security.",
    },
    MailPreset {
        id: "zoho",
        label: "Zoho Mail",
        imap_host: "imap.zoho.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.zoho.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["zoho.com", "zohomail.com"],
        hint: "Enable IMAP in Zoho settings, then create an app-specific password.",
    },
    MailPreset {
        id: "gmx",
        label: "GMX",
        imap_host: "imap.gmx.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "mail.gmx.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["gmx.com", "gmx.net", "gmx.de", "gmx.co.uk"],
        hint: "Enable IMAP access in GMX settings first.",
    },
    MailPreset {
        id: "mailcom",
        label: "mail.com",
        imap_host: "imap.mail.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.mail.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["mail.com", "email.com", "usa.com"],
        hint: "Enable IMAP access in mail.com settings first.",
    },
    MailPreset {
        id: "yandex",
        label: "Yandex Mail",
        imap_host: "imap.yandex.com",
        imap_port: 993,
        imap_security: "implicit",
        smtp_host: "smtp.yandex.com",
        smtp_port: 465,
        smtp_security: "implicit",
        domains: &["yandex.com", "yandex.ru", "ya.ru"],
        hint: "Create an app password in Yandex ID → Security.",
    },
];

/// Look up the preset for an email address, if its domain is known.
///
/// `None` is the "Other" case — the caller must ask for server settings rather
/// than guess them.
pub fn preset_for_email(email: &str) -> Option<&'static MailPreset> {
    let domain = email.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    PRESETS
        .iter()
        .find(|p| p.domains.iter().any(|d| *d == domain))
}

/// Look up a preset by its stable id.
pub fn preset_by_id(id: &str) -> Option<&'static MailPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The security mode implied by a port, for the ports where it is unambiguous.
///
/// Returns `Ok(None)` when the port is legitimate but tells us nothing, so the
/// caller must have been given an explicit choice. **Never guesses** — picking
/// wrong here picks the wrong handshake.
pub fn security_for_port(port: u16) -> Result<Option<TlsMode>, AppError> {
    match port {
        993 | 465 => Ok(Some(TlsMode::Implicit)),
        143 | 587 => Ok(Some(TlsMode::StartTls)),
        // Port 25 is unauthenticated server-to-server relay; a client that
        // submits mail there is either sending in the clear or talking to
        // something that will refuse it.
        25 => Err(AppError::General(
            "Port 25 is for server-to-server relay and is not encrypted — use 587 (STARTTLS) or 465 (TLS) for sending."
                .to_string(),
        )),
        110 | 995 => Err(AppError::General(format!(
            "Port {} is POP3. CXMail is an IMAP client — use 993, or ask your provider for their IMAP settings.",
            port
        ))),
        _ => Ok(None),
    }
}

/// Validate a user-supplied hostname.
///
/// Rejects a pasted URL rather than trying to salvage one: `imaps://host/` in
/// a host field means the user copied the wrong thing, and silently stripping
/// the scheme hides that from them.
pub fn validate_host(host: &str) -> Result<String, AppError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(AppError::General("Server address is required.".to_string()));
    }
    if host.contains("://") {
        return Err(AppError::General(format!(
            "Enter just the server name (e.g. imap.example.com), not a URL — got {:?}.",
            host
        )));
    }
    if host.contains('/') || host.contains(' ') || host.contains('@') {
        return Err(AppError::General(format!(
            "{:?} is not a valid server name.",
            host
        )));
    }
    Ok(host.to_ascii_lowercase())
}

/// Narrow a stored `accounts.imap_port` / `smtp_port` to a real port number.
///
/// The column is `INTEGER` and surfaces as `i32`, so this is a real range
/// check and not a formality — `as u16` would wrap 65536 to 0 and 70000 to
/// 4464, both of which connect somewhere unintended instead of erroring.
pub fn port_from_i32(port: i32, label: &str) -> Result<u16, AppError> {
    match u16::try_from(port) {
        Ok(0) | Err(_) => Err(AppError::General(format!(
            "{} port {} is out of range (1-65535).",
            label, port
        ))),
        Ok(p) => Ok(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::TlsMode;

    #[test]
    fn parses_the_two_stored_spellings() {
        assert_eq!(TlsMode::parse("implicit").unwrap(), TlsMode::Implicit);
        assert_eq!(TlsMode::parse("starttls").unwrap(), TlsMode::StartTls);
    }

    #[test]
    fn rejects_anything_else_rather_than_falling_back() {
        // "none"/"plain" must NOT resolve to a working mode — a fallback here
        // is how an unencrypted connection gets shipped by accident.
        for bad in ["none", "plain", "", "TLS", "ssl"] {
            assert!(TlsMode::parse(bad).is_err(), "{bad:?} must not parse");
        }
    }

    #[test]
    fn implicit_maps_to_mail_send_implicit_tls() {
        assert!(TlsMode::Implicit.is_implicit());
        assert!(!TlsMode::StartTls.is_implicit());
    }

    #[test]
    fn preset_lookup_is_domain_based_and_case_insensitive() {
        assert_eq!(preset_for_email("chris@gmail.com").unwrap().id, "gmail");
        assert_eq!(preset_for_email("chris@googlemail.com").unwrap().id, "gmail");
        assert_eq!(preset_for_email("Chris@GMAIL.COM").unwrap().id, "gmail");
        assert_eq!(preset_for_email("a@fastmail.com").unwrap().id, "fastmail");
        assert_eq!(preset_for_email("a@fastmail.fm").unwrap().id, "fastmail");
        assert_eq!(preset_for_email("a@icloud.com").unwrap().id, "icloud");
    }

    #[test]
    fn an_unknown_domain_has_no_preset() {
        // The "Other" path. Returning a plausible guess here would send the
        // user's password to a host we invented.
        assert!(preset_for_email("chris@cxventures.io").is_none());
        assert!(preset_for_email("chris@notgmail.com").is_none());
        assert!(preset_for_email("no-at-sign").is_none());
        assert!(preset_for_email("trailing@").is_none());
    }

    /// An app-password Gmail account and an OAuth Gmail account must take the
    /// SAME SMTP wire path — otherwise "it sends over OAuth but not over app
    /// password" becomes a bug with no obvious cause. These literals are
    /// copied from `smtp::get_smtp_transport`'s "gmail" arm.
    #[test]
    fn gmail_preset_smtp_matches_the_oauth_gmail_path() {
        let gmail = preset_by_id("gmail").unwrap();
        assert_eq!(gmail.smtp_host, "smtp.gmail.com");
        assert_eq!(gmail.smtp_port, 587);
        assert_eq!(gmail.smtp_security, "starttls");
        assert_eq!(gmail.imap_host, "imap.gmail.com");
        assert_eq!(gmail.imap_port, 993);
    }

    #[test]
    fn every_preset_declares_a_parseable_security_mode() {
        for p in PRESETS {
            TlsMode::parse(p.imap_security)
                .unwrap_or_else(|_| panic!("{} imap_security", p.id));
            TlsMode::parse(p.smtp_security)
                .unwrap_or_else(|_| panic!("{} smtp_security", p.id));
            // A preset whose port and declared security disagree would connect
            // with the wrong handshake and look like a dead server.
            assert_eq!(
                security_for_port(p.imap_port).unwrap(),
                Some(TlsMode::parse(p.imap_security).unwrap()),
                "{} imap port/security disagree",
                p.id
            );
            assert_eq!(
                security_for_port(p.smtp_port).unwrap(),
                Some(TlsMode::parse(p.smtp_security).unwrap()),
                "{} smtp port/security disagree",
                p.id
            );
        }
    }

    #[test]
    fn preset_ids_and_domains_are_unique() {
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_domains = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(seen_ids.insert(p.id), "duplicate preset id {}", p.id);
            for d in p.domains {
                // A domain claimed twice makes preset_for_email order-dependent.
                assert!(seen_domains.insert(*d), "domain {d} claimed twice");
                assert_eq!(*d, d.to_ascii_lowercase(), "domain {d} must be lowercase");
            }
        }
    }

    #[test]
    fn proton_bridge_is_not_a_preset() {
        // Its self-signed per-install cert cannot be trusted without either a
        // verification bypass or dual-stack CA pinning. Keep it out.
        for p in PRESETS {
            assert!(!p.imap_host.contains("127.0.0.1"), "{} is a local bridge", p.id);
            assert!(!p.id.contains("proton"), "proton must not be a preset");
        }
    }

    #[test]
    fn ports_map_to_security_only_where_unambiguous() {
        assert_eq!(security_for_port(993).unwrap(), Some(TlsMode::Implicit));
        assert_eq!(security_for_port(465).unwrap(), Some(TlsMode::Implicit));
        assert_eq!(security_for_port(143).unwrap(), Some(TlsMode::StartTls));
        assert_eq!(security_for_port(587).unwrap(), Some(TlsMode::StartTls));
        // Legitimate but uninformative — the caller must have been told.
        assert_eq!(security_for_port(2525).unwrap(), None);
    }

    #[test]
    fn plaintext_and_pop3_ports_are_refused_by_name() {
        let p25 = security_for_port(25).unwrap_err().to_string();
        assert!(p25.contains("relay"), "{p25}");
        for pop in [110u16, 995] {
            let msg = security_for_port(pop).unwrap_err().to_string();
            assert!(msg.contains("POP3"), "port {pop}: {msg}");
        }
    }

    #[test]
    fn host_validation_rejects_urls_and_junk() {
        assert_eq!(validate_host("  IMAP.Example.COM ").unwrap(), "imap.example.com");
        for bad in ["", "   ", "imaps://imap.example.com", "https://x.com",
                    "imap.example.com/path", "imap example com", "user@imap.example.com"] {
            assert!(validate_host(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    /// `accounts.imap_port` is i32; `as u16` would wrap silently and connect
    /// to a port nobody chose.
    #[test]
    fn port_narrowing_rejects_out_of_range_instead_of_wrapping() {
        assert_eq!(port_from_i32(993, "IMAP").unwrap(), 993);
        assert_eq!(port_from_i32(65535, "IMAP").unwrap(), 65535);
        for bad in [0, -1, 65536, 70000, i32::MAX, i32::MIN] {
            assert!(port_from_i32(bad, "IMAP").is_err(), "{bad} must be rejected");
        }
        // The specific wrap that `as u16` would have produced.
        assert_eq!(70000i32 as u16, 4464);
    }

    /// The TS mirror in `src/lib/mailPresets.ts` is duplicated deliberately
    /// (same call as gotcha #36), which means it can drift. This parses that
    /// file and fails when it does — a prefilled-wrong-host form is a support
    /// ticket that looks like a broken server.
    #[test]
    fn ts_mirror_matches_the_rust_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/lib/mailPresets.ts");
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

        // Pull `key: "value"` / `key: 123` pairs out of each object literal.
        fn field<'a>(block: &'a str, key: &str) -> &'a str {
            let needle = format!("{}:", key);
            let start = block
                .find(&needle)
                .unwrap_or_else(|| panic!("missing {key} in TS preset block:\n{block}"))
                + needle.len();
            let rest = block[start..].trim_start();
            let rest = rest.strip_prefix('"').unwrap_or(rest);
            let end = rest.find(['"', ',']).unwrap_or(rest.len());
            rest[..end].trim()
        }

        // Split on the `id:` marker so each chunk is one preset object.
        let blocks: Vec<&str> = src.split("    id: \"").skip(1).collect();
        assert_eq!(
            blocks.len(),
            PRESETS.len(),
            "TS mirror has {} presets, Rust has {}",
            blocks.len(),
            PRESETS.len()
        );

        for (block, rust) in blocks.iter().zip(PRESETS.iter()) {
            let id = block[..block.find('"').unwrap()].to_string();
            assert_eq!(id, rust.id, "preset order/id differs");
            for (key, expected) in [
                ("imap_host", rust.imap_host.to_string()),
                ("imap_port", rust.imap_port.to_string()),
                ("imap_security", rust.imap_security.to_string()),
                ("smtp_host", rust.smtp_host.to_string()),
                ("smtp_port", rust.smtp_port.to_string()),
                ("smtp_security", rust.smtp_security.to_string()),
            ] {
                assert_eq!(field(block, key), expected, "{id}.{key} differs between TS and Rust");
            }
            for domain in rust.domains {
                assert!(
                    block.contains(&format!("\"{domain}\"")),
                    "{id} is missing domain {domain} in the TS mirror"
                );
            }
        }
    }
}
