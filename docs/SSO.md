# Single sign-on

A tenant's people sign in through the tenant's identity provider: Entra
ID, Okta, Google Workspace or any other OpenID Connect provider. This is
phase 3 of `docs/IDENTITY_AND_DEVICES.md`. SAML is phase 4.

## 1. The operator: `[api.sso]`

On `fcp-admin`:

```toml
[api.sso]
# Where browsers reach this admin. A provider sends them back to
# <public_url>/api/v1/auth/sso/callback, the redirect URI each tenant
# registers with its provider.
public_url = "https://pbx.example.com"
# Let an issuer be a private or loopback address over plain HTTP: a lab's
# own provider. Leave off in production.
allow_private_issuers = false
```

- **Without `public_url`**, providers can be configured, but nobody
  signs in through one. The login page offers none, and
  `/auth/sso/*/start` answers 503.
- **Issuer checks.** An issuer must be HTTPS and public. The admin
  fetches the issuer's discovery document and keys through the webhook
  guard (`docs/WEBHOOKS.md`), so a tenant cannot point the admin at the
  operator's own network.

## 2. The tenant: a provider

A tenant administrator manages providers on the admin UI's **Single
Sign-On** page, or at `/api/v1/identity-providers`. Doing so needs
`identity_provider_manage`, which only a tenant admin and a super admin
hold.

1. Register an application (a "web" client) with the provider. Its
   redirect URI is the one the page shows:
   `<public_url>/api/v1/auth/sso/callback`.
2. Add the provider here with these settings:
   - **issuer**: what the provider's ID tokens name in `iss`;
   - **client id**;
   - **client secret**: kept sealed at rest and never shown again;
   - **scopes** besides `openid`: by default `profile email`.
3. **Check** fetches the discovery document and the keys, as a sign-in
   would.

How a sign-in becomes a user:

- **The account.** It is the provider's `sub` for the person, kept as an
  identity of the user. The same person stays the same user whatever
  name or email the provider gives them later.
- **The first sign-in** is decided in this order:
  - **Link by verified email** (on by default). It links the one user
    whose email the provider asserts with `email_verified: true`. An
    unverified email links nobody.
  - **Create users** (just in time; off by default). It makes an end
    user with no password, named by the username claim
    (`preferred_username`), or the part of it before the `@`, or else
    the email's. A generated SIP password gives them a phone.
  - **Otherwise** the sign-in is refused as `not_provisioned`.
- **Roles by group.** Each line is a group the provider asserts and the
  role its members get.
  - With any set, a user's role follows their groups at every sign-in.
    The most privileged role wins, and a user in no mapped group is an
    end user.
  - If the groups claim is missing altogether, the role is left alone.
  - No provider grants `super_admin`.
- **A platform super admin never signs in through a tenant's provider.**
  It is refused as `super_admin`. A tenant admin configures the provider,
  and a provider that could assert a super admin's email would make them
  one. Super admins sign in with a password.
- **The claim names**: `preferred_username`, `email`, `name` and
  `groups` by default, each changeable.
- **Disabled, locked or suspended users** are refused here as at the
  password.

## 3. Signing in

- **The login page** asks for a username or email.
  - An email whose domain is one of the tenant's login domains offers a
    button for each enabled provider: "Continue with ‹name›".
  - The password field remains unless the tenant has kept passwords from
    people.
- **The flow.**
  - The button goes to `/api/v1/auth/sso/{id}/start`. The admin
    remembers the sign-in (its state, nonce and PKCE verifier), sets a
    cookie naming it (`fcp_sso`, HttpOnly, SameSite=Lax) and sends the
    browser to the provider.
  - On the way back, `/api/v1/auth/sso/callback` refuses a state that is
    not the one this browser's cookie names, or that is over ten minutes
    old.
  - It then trades the code for the ID token (client_secret_basic, with
    the PKCE verifier) and verifies the token:
    - RS256 or ES256 against the issuer's keys, nothing else;
    - `iss`;
    - `aud`, and `azp` when there are several audiences;
    - `exp` and `iat`, within a minute;
    - the `nonce`.
  - It then finds the user, and sends the browser to `/login?sso_code=…`.
    The page exchanges the code once, within two minutes, at
    `/api/v1/auth/sso/exchange` for a session like any other: the access
    token and the refresh cookie (`docs/API_TOKENS.md` §7).
  - A failure lands on `/login?sso_error=<reason>`.
- **What is recorded.**
  - `user.signed_in` with `method: "oidc"`.
  - `user.sign_in_failed` with the reason: `bad_state`, `expired`,
    `provider_refused`, `provider_unavailable`, `bad_id_token`,
    `not_provisioned`, `username_taken`, `no_username`, `disabled` or
    `super_admin`.
  - `user.created` for a user made at sign-in.
- **A tenant's password policy.** `sign_in.password_login` can be
  `admins_only` or `disabled` once the tenant has an enabled provider.
  - The last enabled provider of such a tenant cannot be disabled or
    deleted. Allow passwords first.
  - A platform super admin always signs in with a password.

## 4. A user's identities

- `GET /api/v1/users/{username}/identities` lists the accounts linked to
  a user (`user_read`).
- `DELETE /api/v1/users/{username}/identities/{provider_id}` unlinks one
  (`user_update`). The next sign-in there links or creates afresh.
- Deleting a provider deletes the identities at it.

## 5. Not yet

These come in phase 3b:

- **Back-channel logout.** A provider's logout does not yet end FCP
  sessions.
- **Sites.** A site does not yet sign its people in through the tenant's
  provider.
- **Step-by-step guides** for Entra ID, Okta and Google.

SAML is phase 4, and SCIM provisioning is phase 6.
