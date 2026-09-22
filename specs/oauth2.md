# OAuth2 Flow

## Prerequisites

1. Create a Google Cloud project at https://console.cloud.google.com
2. Enable the Gmail API
3. Create OAuth 2.0 credentials → Application type: "Desktop app"
4. Note the `client_id` and `client_secret`
5. No redirect URI configuration needed (Google allows loopback for desktop apps)

Client ID and secret are compiled into the binary (private repo, no distribution concerns).

## Flow

```
Step 1: User clicks "Add Gmail Account"
  → Frontend calls invoke('start_oauth2', { provider: 'gmail' })

Step 2: Rust generates OAuth2 authorization URL
  → Scopes: https://mail.google.com/ (full IMAP/SMTP access)
  → PKCE: Generate code_verifier (random 128 bytes, base64url) + code_challenge (SHA256 of verifier, base64url)
  → State: Random nonce (32 bytes) for CSRF protection
  → Start local HTTP server on random port (127.0.0.1:0 — OS picks available port)
  → Redirect URI: http://localhost:PORT/oauth/callback
  → Return auth URL to frontend

Step 3: Frontend opens URL in system browser
  → Use @tauri-apps/plugin-shell open() to launch default browser
  → User authenticates with Google in their browser
  → Google redirects to http://localhost:PORT/oauth/callback?code=...&state=...

Step 4: Local HTTP server receives callback
  → Verify state parameter matches what was generated
  → Exchange authorization code for tokens (with PKCE code_verifier):
    POST https://oauth2.googleapis.com/token
    {
      code,
      client_id,
      client_secret,
      redirect_uri: "http://localhost:PORT/oauth/callback",
      grant_type: "authorization_code",
      code_verifier
    }
  → Response: { access_token, refresh_token, expires_in, token_type }

Step 5: Store credentials in macOS Keychain
  → access_token  → keyring service: "cxmail", account: "gmail:{email}:access"
  → refresh_token → keyring service: "cxmail", account: "gmail:{email}:refresh"
  → expires_at    → keyring service: "cxmail", account: "gmail:{email}:expires"
  → Shut down local HTTP server

Step 6: Test IMAP connection
  → Connect to imap.gmail.com:993 with XOAUTH2 (see imap.md)
  → If successful: save account to SQLite, emit success event to frontend
  → If failed: delete stored tokens, return error

Step 7: Token refresh (automatic, before each IMAP session)
  → Check if access_token is expired (or within 5-minute buffer)
  → If expired:
    POST https://oauth2.googleapis.com/token
    {
      refresh_token,
      client_id,
      client_secret,
      grant_type: "refresh_token"
    }
  → Update access_token and expires_at in Keychain
```

## Security

- **PKCE** (Proof Key for Code Exchange): Prevents authorization code interception. Required for desktop apps.
- **Loopback redirect**: No external server. Google allows `http://localhost:PORT` for desktop app type credentials.
- **State parameter**: Random nonce verified on callback to prevent CSRF.
- **Client secret**: Compiled into binary. Google classifies desktop apps as "public clients" — the secret is not truly secret, but is still required by the token endpoint. This is fine for a private repo.
- **Token storage**: Keychain only. Never logged, never in SQLite, never sent anywhere except Google's token endpoint.
- **Access tokens**: Short-lived (1 hour). Refresh tokens are long-lived, stored in Keychain.

## Provider-Specific Notes

### Gmail
- OAuth2 with XOAUTH2 SASL for IMAP/SMTP
- Scope: `https://mail.google.com/`
- Token endpoint: `https://oauth2.googleapis.com/token`
- Auth endpoint: `https://accounts.google.com/o/oauth2/v2/auth`

### iCloud (Phase 2)
- No OAuth2. Uses app-specific passwords.
- User generates at https://appleid.apple.com → Security → App-Specific Passwords
- IMAP LOGIN with email + app-specific password
- Store password in Keychain

### Outlook (Phase 2)
- OAuth2 via Microsoft Identity Platform
- Auth endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
- Scope: `https://outlook.office365.com/IMAP.AccessAsUser.All https://outlook.office365.com/SMTP.Send offline_access`
- XOAUTH2 SASL for IMAP
