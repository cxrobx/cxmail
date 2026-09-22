# OAuth release requirements

CXMail is a desktop public client. Both providers use Authorization Code with
PKCE and a loopback redirect. Microsoft ships with no client secret. Google
does NOT — despite the client being registered as a Desktop app, Google's
token endpoint still requires `client_secret` on every exchange and refresh
(verified live, twice; see the comment on `GOOGLE_CLIENT_SECRET` in
`secrets.rs`). There is no secret-less configuration available for a native
Google OAuth client; embedding it is Google's own sanctioned model for this
client type ("installed apps cannot keep secrets"), not a gap to close before
launch.

## Google

1. The production Desktop OAuth client already exists in the CXMail Google
   Cloud project (`cxmail desktop`). Do not create a second one to try to
   drop the secret — that path is a dead end, confirmed empirically.
2. Configure the consent screen, support email, product privacy-policy URL, and
   verified `cxventures.io` domain.
3. Enable the Google Calendar API and add
   `https://www.googleapis.com/auth/calendar.events` to the consent screen.
   Calendar is an incremental, opt-in grant stored separately from mail.
4. While the consent screen remains in Testing, add every Workspace identity
   used for release testing (including Artist Advisory and CX Ventures) as a
   test user.
5. Request verification for `https://mail.google.com/`, `calendar.events`,
   and the user-email scope. Gmail IMAP/SMTP access requires the restricted
   mail scope; Calendar event writes use the narrower sensitive scope.
6. Complete Google's restricted-scope security assessment if requested.
7. Add the public client ID to GitHub as `CXMAIL_GOOGLE_CLIENT_ID`.
8. Test with an account that is not listed as an OAuth test user before launch.

## Microsoft

1. Register a multitenant app and enable **Allow public client flows**.
2. Add the Mobile and desktop application loopback redirect
   `http://localhost`.
3. Grant delegated `offline_access`, `openid`, `email`, `IMAP.AccessAsUser.All`,
   and `SMTP.Send` permissions.
4. Add the application ID to GitHub as `CXMAIL_MS_CLIENT_ID`.
5. Verify sign-in, refresh, IMAP sync, and SMTP send with both consumer Outlook
   and Microsoft 365 organizational accounts.

OAuth provider approval is an external launch gate; code completion alone does
not make an unverified Gmail consent screen suitable for paying customers.
