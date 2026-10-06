# Signing in an app

How FCP's own apps (Anvil, the softphone: `docs/SOFTPHONE.md`) sign a
person in and keep them signed in. It is OAuth 2.0 for native apps
(RFC 8252) with PKCE (RFC 7636), run by the admin API for FCP's apps only:
FCP is not an identity provider for anyone else's.

## 1. In the browser

The app opens the system browser; the person signs in exactly as they do
on the web (password, a one-time code, the tenant's identity provider).

1. **The app begins.** It makes a verifier (43 to 128 characters of
   `A-Z a-z 0-9 - . _ ~`), listens on a loopback port, and opens:

   ```
   GET /api/v1/auth/app/authorize
       ?client_id=anvil
       &redirect_uri=http://127.0.0.1:53124/callback
       &code_challenge=<base64url(SHA-256(verifier))>
       &code_challenge_method=S256
       &state=<anything>
   ```

   `redirect_uri` must be plain HTTP to `127.0.0.1`, `[::1]` or
   `localhost`, on any port and path. The admin keeps the request for ten
   minutes and sends the browser to its sign-in page,
   `{public_url}/login?app=<request>` (`[api.sso] public_url`, or the same
   origin without one).
2. **The person signs in.** The page remembers the request across an
   identity provider's round trip. Once signed in it calls
   `POST /api/v1/auth/app/approve` `{request}` with the new session's token,
   and sends the browser to the answer's `redirect_url`: the app's address
   with `code` and `state`. A request is granted once. Granting needs a
   browser session signed in within the last ten minutes, so someone at a
   signed-in browser left open cannot grant an app without the password.
3. **The app redeems the code** within a minute:

   ```
   POST /api/v1/auth/app/token
   grant_type=authorization_code&code=…&code_verifier=…
   &redirect_uri=http://127.0.0.1:53124/callback&client_id=anvil
   &client_name=Alice's MacBook&install_id=<kept by the install>
   &platform=macos&version=0.2.0
   ```

   The body may be a form or JSON. A code is tried once: a wrong verifier
   or redirect spends it.

## 2. Without a browser

An app that has none (`anvil-cli`, tests) signs in as the web page does —
`POST /auth/login`, then `/auth/login/totp` if asked — and then:

```
POST /api/v1/auth/app/sessions
Authorization: Bearer <that sign-in's access token>
{"client_name": "anvil-cli", "install_id": "…", "platform": "linux"}
```

The same ten-minute rule applies, and the same password policy, lockout
and second factor, because they are the same sign-in. (The design's
"password grant", `docs/SOFTPHONE.md` §2.2, is this: one sign-in path, not
a second copy of it in the token endpoint.)

## 3. The session

Both answer the same tokens:

```json
{
  "access_token": "…",          // 15 minutes, as a web session's
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "…",         // opaque
  "refresh_expires_in": 7776000,
  "session_id": "sess-…",
  "user": { "username": "alice", … }
}
```

- **Refreshing.** `POST /auth/app/token`
  `grant_type=refresh_token&refresh_token=…` answers new tokens. The refresh
  token turns over every time: keep the newest.
  - The one before it, presented within ten seconds, is refused with 409
    (the app raced itself) and the session stands.
  - Presented later, it means two holders: the session ends.
  - A browser's refresh cookie is not taken here, nor an app's token at
    `/auth/refresh`.
- **How long.** An app session lasts while it is used: the tenant's
  `sign_in.app_session_days` (90 by default, 1 to 365) from its last
  refresh, where a browser's lasts 12 hours. Set it with the tenant
  (`PUT /api/v1/tenants/{id}`, `sign_in.app_session_days`).
- **One per install.** A second sign-in from the same `install_id` ends the
  first.
- **Ending.** `POST /auth/app/revoke` `{token: <refresh token>}` signs the
  app out (200 whether or not the token named a session, RFC 7009). Anything
  that signs a user out everywhere ends app sessions too: a password change,
  an administrator's reset, the user suspended, locked or deleted.
- **What an app may do.** Its access token is the user's, as the web
  session's is: the routes the user's roles allow, and `/me`. An app
  session cannot grant another app a session.
- **Where it shows.** It is a row in `login_sessions` naming the app
  (`app_name`, `app_install_id`, `app_platform`, `app_version`), and its
  sign-in is published as `user.signed_in` with `method: "app"`.

## 4. Errors

| Status | When |
|---|---|
| 400 | A `redirect_uri` that is not a loopback address, a method other than S256, a malformed verifier, no `client_name`/`install_id`, an unknown `grant_type` |
| 401 | A code spent, expired or not this verifier's; a refresh token that is not a live app session's; a browser session older than ten minutes asking to grant |
| 403 | An API token asking to grant |
| 404 | An app's request that has expired or was granted already |
| 409 | A refresh token replaced within the last ten seconds |
