// Google OAuth client — registered as a "Desktop app" (installed application)
// client, not "Web application" (a prior version of this comment was wrong
// about that). Despite the Desktop type, Google's token endpoint still
// REQUIRES client_secret on both the authorization-code exchange and every
// refresh — verified live, twice, independently: a secret-less request is
// rejected regardless of PKCE. Google's docs mark the field "Optional" for
// installed apps, but the actual no-secret exemption applies only to
// Android/iOS/Chrome-app client types, never Desktop. Do not re-attempt this
// by registering a *new* Desktop client; that premise already regressed once
// in 54c93ad and was independently re-verified as a dead end.
//
// The ID and secret are deliberately NOT in this file. The source is public
// (source available), and Google's per-project OAuth user cap — 100 users, lifetime,
// unresettable — belongs to the signed release build's project. A source
// build carrying the same client would spend a slot on every self-builder's
// Gmail sign-in. So:
//   - the signed release build gets both from CI (CXMAIL_GOOGLE_CLIENT_ID /
//     CXMAIL_GOOGLE_CLIENT_SECRET — see .github/workflows/release.yml);
//   - a local dev build gets them from the Keychain via scripts/dev-signed.sh;
//   - a build without them has no Gmail sign-in (`require_client_id` reports
//     "not configured"); Gmail then connects with an app password (provider
//     `imap`), or the builder exports their own Google client before building.
// Google treats a Desktop client's secret as non-confidential, so the CI
// injection is about the user cap, not secrecy.
pub const GOOGLE_CLIENT_ID: &str = match option_env!("CXMAIL_GOOGLE_CLIENT_ID") {
    Some(value) => value,
    None => "",
};
pub const GOOGLE_CLIENT_SECRET: &str = match option_env!("CXMAIL_GOOGLE_CLIENT_SECRET") {
    Some(value) => value,
    None => "",
};
pub const GOOGLE_AUTH_URI: &str = "https://accounts.google.com/o/oauth2/auth";
pub const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

// Microsoft (Outlook) — public client (PKCE, NO secret), registered in the
// chriscxventures.onmicrosoft.com tenant as "CXMail": multitenant + personal
// Microsoft accounts, "Allow public client flows" on, loopback redirect
// `http://localhost` under publicClient (NOT web — a web redirect would make
// Azure demand a client secret this app does not carry). Delegated Graph scopes:
// offline_access, openid, email, IMAP.AccessAsUser.All, SMTP.Send.
//
// The default is the real production ID, mirroring GOOGLE_CLIENT_ID above, so
// local and dev builds can add Outlook accounts. Defaulting to "" instead meant
// every build without the CI env var reported Outlook as "not configured" —
// invisible until someone tried to add an account. Unlike Google's, this is a
// public identifier with no accompanying secret, so shipping it in source is safe.
// CI may still override via CXMAIL_MS_CLIENT_ID.
pub const MS_CLIENT_ID: &str = match option_env!("CXMAIL_MS_CLIENT_ID") {
    Some(value) => value,
    None => "be24a6c6-2d4d-46ed-b466-b1cb253f2224",
};
pub const MS_AUTH_URI: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
pub const MS_TOKEN_URI: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

pub fn require_client_id(provider: &str) -> Result<&'static str, crate::error::AppError> {
    let value = match provider {
        "gmail" => GOOGLE_CLIENT_ID,
        "outlook" => MS_CLIENT_ID,
        _ => "",
    };
    // Google's token endpoint rejects an exchange without the secret (see the
    // header comment), so an ID without its secret is "not configured" too —
    // better reported here than as a failed exchange after the consent screen.
    let secret_present = match provider {
        "gmail" => !GOOGLE_CLIENT_SECRET.trim().is_empty(),
        _ => true,
    };
    if value.trim().is_empty() || !secret_present {
        Err(crate::error::AppError::OAuth2(format!(
            "{provider} OAuth is not configured in this build"
        )))
    } else {
        Ok(value)
    }
}
