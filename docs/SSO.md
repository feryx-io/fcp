# Single sign-on

A tenant's people sign in through the tenant's identity provider: Entra
ID, Okta, Google Workspace or any other OpenID Connect provider (phase 3
of `docs/IDENTITY_AND_DEVICES.md`), or any SAML 2.0 provider (phase 4,
§8). Everything here applies to both, except where a section says it is
one protocol's.

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
- **SAML needs the `saml` build** of `fcp-admin`, which links the xmlsec
  C library. The images are built with it. From source, install
  `libxmlsec1-dev` and `libclang-dev`, then build with
  `cargo build -p fcp-admin --features saml`.
  - A build without it keeps SAML providers, but signs nobody in through
    them: it answers 503 saying so.
  - SAML also needs an HTTPS `public_url`. The provider posts its answer
    from its own site, and the browser carries the sign-in's cookie on
    that post only when it is `SameSite=None; Secure`.

## 2. The tenant: a provider

A tenant administrator manages providers on the admin UI's **Single
Sign-On** page, or at `/api/v1/identity-providers`. Doing so needs
`identity_provider_manage`, which only a tenant admin and a super admin
hold.

1. Register an application (a "web" client) with the provider. Its
   redirect URI is the one the page shows:
   `<public_url>/api/v1/auth/sso/callback`. Its back-channel logout URI,
   where the provider supports one, is also shown (§5). The guides in §7
   walk through Entra ID, Okta and Google.
2. Add the provider here with these settings:
   - **issuer**: what the provider's ID tokens name in `iss`;
   - **client id**;
   - **client secret**: kept sealed at rest and never shown again;
   - **scopes** besides `openid`: by default `profile email`.
3. **Check** fetches the discovery document and the keys, as a sign-in
   would.

Two settings decide whose tokens count:

- **Required claims.** These are claims every token must carry, with the
  given value; any other token is refused as `claim_mismatch`.
  - Google's `hd` names the Workspace domain. Without it, any Google
    account at all could sign in.
  - Entra's `tid` names the directory.
  - A claim that is a list must contain the value.
- **The verified-email claim.** This is the claim that says the email is
  the person's, used for linking. It is `email_verified` by default, and
  `xms_edov` for Entra ID, whose tokens carry no `email_verified`.

How a sign-in becomes a user:

- **The account.** It is the provider's `sub` for the person, kept as an
  identity of the user. The same person stays the same user whatever
  name or email the provider gives them later.
- **The first sign-in** is decided in this order:
  - **Link by verified email** (on by default). It links the one user
    whose email the provider asserts as verified (the verified-email
    claim, `true`). An unverified email links nobody.
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
    `not_provisioned`, `username_taken`, `no_username`, `disabled`,
    `super_admin` or `claim_mismatch`.
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

## 5. Back-channel logout

A provider that supports OpenID Connect Back-Channel Logout 1.0 tells FCP
when someone signs out there. Keycloak does, as does Okta where the org
has it enabled; Entra ID and Google do not.
Register the provider's back-channel logout URI, which the page shows:
`<public_url>/api/v1/auth/sso/{id}/backchannel-logout`.

- **The request.** The provider POSTs a `logout_token`, verified as
  strictly as an ID token:
  - signed by its keys;
  - its `iss`;
  - `aud` is our client;
  - issued in the last five minutes, so a captured one replayed later is
    refused;
  - the logout event in `events`;
  - no `nonce`;
  - a `sub`, a `sid` or both.
- **What ends:**
  - with a `sid`, the sessions begun as that provider session (the `sid`
    the ID token carried at sign-in);
  - with only a `sub`, every session of that person begun through the
    provider.
- **The answer** is 200, with `Cache-Control: no-store`. A token that
  does not verify is answered 400 and ends nothing.
- **The effect** is at once: an ended session's access token is refused
  at its next request, and its refresh cookie no longer refreshes.

## 6. Sites

A site signs its people in through its tenant's provider itself, so
sign-in works while the link to the cloud is down
(`docs/IDENTITY_AND_DEVICES.md` Q11).

- **What syncs.** The tenant's providers and their users' identities are
  in the site's slice (`docs/HYBRID_CONTROL_PLANE.md`).
  - The client secret travels only over the link's mutual TLS, and the
    link stores it sealed with the site's own master key.
  - The site's admin reads providers, but a change is the cloud's.
- **The site's own address.** Set `[api.sso] public_url` on the site's
  admin to where browsers reach it. Register its callback,
  `<site public_url>/api/v1/auth/sso/callback`, with the provider as a
  second redirect URI of the same client.
- **What a site does not do.**
  - It does not create users at first sign-in: users come from the cloud,
    so a stranger is refused as `not_provisioned`.
  - It does not set roles from groups: the cloud is the users' system of
    record and applies them at the person's next sign-in there.
  - It does link a person by verified email. The identity it records
    there is the site's own, until the cloud's arrives.
- **When the provider is unreachable**, people sign in with a password
  only if the tenant's policy allows it. There is no other fallback.
- **Back-channel logout** reaches the URI the provider has. A provider
  takes one per client, so register the cloud's; sessions at a site end
  when they expire.

## 7. Guides

These were written against each provider's console as of 2026. Labels
move, but the settings do not.

### Entra ID (Microsoft)

1. **Entra admin center → App registrations → New registration.**
   - Supported account types: *this organizational directory only*.
   - Redirect URI: platform *Web*, the callback URI.
2. **Certificates & secrets → New client secret.** Copy the value: that
   is the client secret.
3. **Token configuration.**
   - *Add optional claim → ID → `email` and `xms_edov`*. `xms_edov` says
     the email belongs to a domain the directory has verified.
   - For roles by group: *Add groups claim → Security groups*, emitted
     as group object IDs. Over about 200 groups the token carries none;
     choose *groups assigned to the application* to stay under.
4. **In FCP:**
   - Issuer: `https://login.microsoftonline.com/<directory id>/v2.0`.
     The `common` and `organizations` endpoints are not single issuers
     and are refused.
   - Client id: the application (client) ID.
   - Verified-email claim: `xms_edov`.
   - Required claim: `tid <directory id>`.
   - Username claim: `preferred_username`, the UPN.
   - Roles by group: the group's object ID, then the role.
5. Entra ID sends no back-channel logout.

### Okta

1. **Admin console → Applications → Create App Integration.** Choose
   *OIDC*, then *Web Application*.
   - Sign-in redirect URI: the callback URI.
   - Client authentication: *Client secret*. FCP sends it as HTTP Basic,
     Okta's default.
2. **Assignments.** Assign the people or groups who may sign in.
3. **Back-channel logout.** On the app's *General* tab, where the org
   offers it, enter the back-channel logout URI.
4. **Groups.** Two routes:
   - With the org authorization server (issuer `https://<org>.okta.com`):
     the app's *Sign On → OpenID Connect ID Token → Groups claim filter*.
     Name it `groups` and match, say, `fcp-` prefixes.
   - With a custom authorization server (issuer
     `https://<org>.okta.com/oauth2/<server id>`): *Security → API →
     Authorization Servers → Claims*. Add a claim `groups`, *ID Token*,
     *Always*, value type *Groups*, with a filter.
5. **In FCP:** the issuer exactly as Okta's discovery document names it,
   the client id and secret, and roles by group name. The verified-email
   claim stays `email_verified`.

### Google Workspace

1. **Google Cloud console → APIs & Services.**
   - *OAuth consent screen*: user type *Internal*, so only your Workspace
     users are offered it.
   - *Credentials → Create credentials → OAuth client ID → Web
     application*, with the callback URI as an authorized redirect URI.
2. **In FCP:**
   - Issuer: `https://accounts.google.com`.
   - The client id and secret.
   - Required claim: `hd <your Workspace domain>`. Without it a personal
     Google account would be accepted wherever the consent screen is
     external.
   - Username claim: `email`. Google sends no `preferred_username`.
3. **Groups.** Google's ID token carries none, so roles by group do not
   apply. Set roles in FCP, or provision them by SCIM (phase 6).
4. Google sends no back-channel logout.

## 8. SAML

A SAML provider is added like any other, with protocol **SAML 2.0**. It
is described by its metadata, which every SAML provider publishes.

- **The provider side.**
  1. Paste the identity provider's metadata XML. It gives:
     - its entity id;
     - its HTTP-Redirect sign-in address;
     - the certificate it signs with.

     Metadata naming no signing certificate is refused: nothing could be
     verified against it.
  2. The page then shows what to give the identity provider:
     - FCP's entity id, `<public_url>/api/v1/auth/saml/{id}`, which
       serves FCP's metadata for import;
     - its assertion consumer service, `…/{id}/acs` (HTTP-POST).
  3. **Check** reads the metadata as a sign-in would.
- **Attributes.** The username, email, name and groups settings name
  SAML attributes, by their `Name` or `FriendlyName`. `NameID` means the
  subject itself, so a username can be the NameID.
  - The account is the NameID, which should be persistent. A transient
    NameID is refused: it would name a different person every time.
  - **Trust its email attribute** says that the provider's email is the
    person's own, for linking by email. SAML has nothing like
    `email_verified`, so it is off unless turned on.
  - Required claims and roles by group apply to attributes as they do to
    claims.
- **What is checked.** A response is taken only if all of these hold:
  - it is signed, the response or its assertion, by a certificate in the
    metadata, with RSA or ECDSA over SHA-256 or stronger;
  - it is reduced to its signed parts before anything is read, so
    nothing unsigned rides along;
  - it answers this browser's sign-in: its `InResponseTo`, and its
    RelayState matching the `fcp_saml` cookie;
  - its destination and recipient are FCP's assertion consumer service;
  - it carries an audience naming FCP's entity id;
  - it is within its time window, allowing three minutes of clock skew;
  - its assertion has not been taken before, as assertion ids are
    remembered until they expire.

  Anything else lands on `/login?sso_error=bad_assertion`. A sign-in the
  identity provider starts by itself (IdP-initiated) is refused as
  `bad_state`: there is no sign-in of this browser for it to answer.
- **The session** that follows has `method: "saml"`. Sites, the password
  policy and identities work as for OpenID Connect.

## 9. Not yet

- **Logging out at the provider.** Signing out of FCP ends the FCP
  session only; it does not send the browser to the provider's
  end-session endpoint.
- **SAML, still to come** (phase 4b):
  - single logout;
  - metadata fetched from the provider's URL;
  - encrypted assertions and signed AuthnRequests, which need FCP's own
    signing key;
  - the SAML guides for Entra ID, Okta and Google.
- **SCIM provisioning** is phase 6.
