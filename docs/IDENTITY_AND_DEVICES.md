# Identity and devices

Designed 2026-10-01, after the developer surface (`docs/DEVELOPER_SURFACE.md`)
closed. This is the fourth recommendation of the 2026-09-25 feature review
against BroadSoft, CUCM, Zoom, Webex Calling and Twilio. It covers how people
and phones arrive on the platform:

- signing in with the company's identity provider (OIDC and SAML);
- users created, changed and removed by that provider (SCIM 2.0);
- a phone taken out of its box that finds its configuration (zero-touch);
- the firmware it runs.

Every product in the review sells these four. FCP has none of them. People
sign in with a password FCP stores, users are typed in by hand, and a phone
is configured by an administrator who enters its MAC and then types a URL
into it. That URL needs a header no phone can send. Under all four sits a
user who is a row keyed by its username, owning nothing the database knows
about. This document designs the fixes. Agreed 2026-10-01: the twelve
questions in §5, each settled on its recommendation.

## 1. What the code has, and what it lacks

Three surveys of the code were run on 2026-10-01, and their sharpest
findings were checked against the files. The admin API is
`crates/fcp-api-core`, served by `fcp-admin` under `/api/v1`.

### 1.1 Signing in: a password, a token, nothing else

- **One way in.** `POST /api/v1/auth/login` (`v1/auth_session.rs:133`)
  takes `{username, password, tenant_id?}`.
  - It checks a bcrypt (cost 12) hash and answers an HS256 JWT built by
    hand (`encode_jwt` :648).
  - The JWT's claims are `sub` (the **username**), `role`, `tenant_id`,
    `display_name`, `exp` and `iat`. There is no session id, issuer or
    audience.
  - Its lifetime is a fixed 24 hours (`JWT_EXPIRY_HOURS` :24).
- **The signing key is the API token.** It is
  `HMAC-SHA256("fcp-jwt-key-derivation", api.auth_token)`
  (`state.rs:431-439`), so rotating the platform's static token signs
  everyone out. The events server derives the same key
  (`fcp-events-server/src/main.rs:822`).
- **No server-side session.**
  - Logout does nothing on the server (`auth_session.rs:309`).
  - A token cannot be revoked. A disabled user is refused only because
    `validate_token` reloads the user by username on every request
    (`auth.rs:252-289`).
- **The tenant is guessed from the username.**
  - Usernames are unique per tenant: the users table's key is
    `(tenant_id, username)` (`migrations/001:39`).
  - Login searches every tenant for the username and refuses a name found
    in two (`load_user_for_login`, :88-112).
  - The UI never sends `tenant_id` (`LoginPage.tsx:27`), so such a user
    cannot sign in from the UI at all.
  - A tenant has SIP domains but no login domain, email domain or slug
    (`tenant.rs:70-135`).
- **No SSO of any kind.**
  - No OIDC, SAML, OAuth or LDAP code, configuration or crate.
  - The user has no external-identity field: no issuer, subject or IdP id.
  - Email is optional, not unique, and not used to sign in.
- **No MFA and no reset by email.** There is a password policy
  (`validate_password_strength`, `storage.rs:46`), lockout after
  `[security] max_failed_attempts`, and optional expiry.
- **The UI forgets a session on reload.** The token lives only in memory
  (`stores/auth.ts:20-26`), the UI never calls `/auth/refresh`, and there
  is no callback route an identity provider could send a browser back to.
- **Sign-in is barely audited.** The audit middleware records a login
  attempt as an anonymous POST with its status and source address
  (`audit.rs:87-166`). It does not record which user, or which tenant.
  Success, failure and lockout appear only in the logs.
- **The web password is also the SIP password.** The SIP HA1 is derived
  from it unless a separate digest password is set (`auth_session.rs:583`),
  and `password_hash` is `NOT NULL`. A user who never has a web password
  still needs SIP credentials.
- **A site signs in on its own.** `users` is journaled to sites
  (`migrations/079:57`). A site's admin checks bcrypt against its synced
  copy, which works while the link is down. Its JWT key comes from its own
  token, so a cloud session is not valid at a site. `/me/password` is a
  write a site makes that the sync does not cover.

### 1.2 Users: a row that owns nothing

- **The API addresses users by username, and a username cannot change.**
  - `GET/PUT/DELETE /api/v1/users/:username` (`v1/users.rs`).
  - `UserId` (`user-<uuid>`) exists, but `users.id` has only a non-unique
    index (`migrations/002:31`).
  - `list_users` loads every user and filters by tenant in memory
    (:249-262). There is no filter or search.
- **A deleted username comes back to life.**
  - A session's `sub` and an API token's maker are usernames.
  - Delete a user and create another with the same name: the first user's
    unexpired sessions and tokens work again, as the second user.
- **Deleting a user leaves everything it owned behind.** No migration has
  `REFERENCES users`. `delete_user` runs a bare `DELETE FROM users`
  (`fcp-storage-postgres/src/lib.rs:1088-1097`). Orphaned:
  - its devices (`owner_user_id`);
  - line bindings and extensions;
  - its voicemail mailbox and messages;
  - calling settings;
  - hoteling sessions;
  - declared locations;
  - meetings;
  - queue memberships.

  `docs/COMPLIANCE_FOLLOWUPS.md:137-139` already records this. Subject
  erasure finds the user by its live row, so it must run before the delete
  (`legal_hold.rs:250-254`).
- **Disabling is two flags that disagree.**
  - `enabled` and `account_status` (Active, Suspended, Locked) overlap.
  - SIP digest authentication reads only `enabled`
    (`registrar.rs:660`), so a Suspended user can still register.
  - Disabling a user unregisters nothing, ends no call or hoteling
    session, and publishes nothing: `UserEvent::Enabled` and `Disabled`
    exist (`fcp-events/src/types.rs:741,750`) and are never sent.
- **Creating a user creates only the row.**
  - No extension, number, mailbox or calling settings.
  - A password is required.
  - `Tenant.max_users` exists but is never enforced.
  - An invalid `account_status` silently becomes Active (`users.rs:386`).
- **There are no groups.** Departments are free text. Hunt and pickup
  groups are static TOML listing extensions (`fcp-pbx/src/config.rs`).
  Queues list agents as `TEXT[]`. Nothing holds "these users".
- **Events say little.** `user.updated` carries `changed_fields: vec![]`
  (a TODO at `users.rs:637`). Audit rows record the route, not the change.

### 1.3 Devices: provisioning no phone can use

- **A phone cannot authenticate to the provisioning URL.**
  `GET /api/v1/provisioning/{mac}` (`v1/provisioning.rs`) wants two
  secrets in headers:
  - a deployment-wide token, in `X-FCP-Provisioning-Token` or a bearer;
  - a per-device token, in `X-FCP-Device-Provisioning-Token`.

  No phone sends custom headers, and a phone out of its box knows neither
  secret. The provisioning server works for a script, not for a phone.
- **No vendor file names.**
  - Phones ask for `y000000000xx.cfg` and `<mac>.cfg` (Yealink),
    `000000000000.cfg` (Poly), `cfg<mac>.xml` (Grandstream) and
    `SEP<mac>.cnf.xml` (Cisco).
  - The server knows one route with a bare MAC, and always answers
    `text/plain`.
- **Nothing helps a phone find the server.** There is no DHCP option
  66/160 guidance and no redirection service: Yealink RPS, Poly ZTP,
  Grandstream GDMS, Cisco EDOS and Fanvil FDPS are all absent.
  - There is no claim flow either: no activation code, and no unknown MAC
    recorded so it can be assigned.
  - The admin must create the device, MAC first, before it can fetch
    anything.
  - The only mention is an unchecked roadmap line
    (`docs/ARCHITECTURE-v0.2.md:1992`).
- **The templates are thin.**
  - Rendering is `${var}` replacement with no conditionals or loops.
    `model_pattern` is never used to pick a template.
  - The server is the realm on port 5060 over UDP (`provisioning.rs:337`),
    though the platform serves SIPS.
  - No time zone, NTP, firmware URL or provisioning-URL self-reference.
  - Four vendors: no Fanvil or Snom.
- **Nothing is recorded.** A fetch logs nothing and writes nothing back:
  - no last-provisioned time;
  - no source address or User-Agent;
  - no record of a MAC nobody claimed.
- **No resync.** `check-sync` NOTIFYs exist for hoteling only
  (`fcp-sip/src/hoteling.rs:360-391`). There is no "re-read your config"
  for a device or a template, and no vendor variants (Cisco's `resync`,
  `check-sync;reboot=false`).
- **Registration is not the device's.**
  - The devices table's registration fields are never written.
  - The list view overlays live registrations, matched by AOR, then MAC
    in the User-Agent, then IP (`devices.rs:464-581`). The single view
    does not.
  - The User-Agent, which names the model and firmware, is never parsed.
- **No firmware.** There is a free-text `firmware_version` an admin types,
  and nothing else: no store, upload, serving, target version or rollout.
  `docs/ENTERPRISE_CALLING.md:1061` puts firmware out of scope.
- **The MAC is unique across the deployment** (`migrations/046:35`), and
  the provisioning token is one value for the whole deployment, not per
  tenant (`[api] provisioning_auth_token`).

## 2. The model

- **A user is an id, with identities.**
  - `users.id` becomes unique and is what everything else names.
  - Sessions and API tokens name their user by id. A re-created username
    is a new user with nothing of the old one.
  - A username can be renamed, because SCIM renames and so do people.
  - `user_identities(user_id, provider_id, subject)` links a user to
    accounts at the tenant's identity providers.
  - A user may have no web password; their SIP credentials are generated.
- **Signing in is per tenant.**
  - A tenant has login domains: `@acme.com` means Acme.
  - The login page asks who you are first, then offers what your tenant
    allows: a password, or "Continue with Acme SSO".
  - A tenant's identity providers are OIDC or SAML, each with claims and
    groups mapped to FCP's names and roles, and optional just-in-time
    creation. Secrets are sealed at rest (`fcp-crypto`).
  - A tenant's password policy may be `allowed`, `admins_only` or
    `disabled` once SSO is on. A platform super-admin and the static token
    always get in (break-glass).
- **A session is a row.**
  - `sessions` records the user, the tenant, how they signed in
    (`password`, `totp`, `oidc`, `saml`), the identity provider's session
    id, expiry and revocation. The JWT carries the session id (`sid`) and
    the user id (`sub`).
  - Logout revokes the session. Disabling a user revokes all of theirs.
    An identity provider's back-channel or single logout revokes by its
    session id.
  - The signing key is its own, with a key id, so it can rotate without
    signing everyone out.
  - The UI holds a short-lived access token in memory and a refresh token
    in an HttpOnly cookie, so a reload keeps you signed in.
- **A user has a lifecycle.**
  - **Active.**
  - **Suspended** (SCIM `active=false`). Every way in is refused: web,
    SIP, tokens. Registrations are dropped, hoteling sessions end, and
    `user.disabled` is published. Calls in progress continue.
  - **Deprovisioned.** Deleted by SCIM or an admin. The user is
    suspended, then purged after the tenant's grace period by a defined
    offboarding:
    - devices are unassigned;
    - extension and numbers are released;
    - the mailbox is kept until its retention says otherwise;
    - calling settings, tokens, hoteling, queue memberships and declared
      locations are removed;
    - meetings move to an owner the tenant names.

    Subject erasure, where asked, runs first.
- **Groups hold users.**
  - A tenant's groups have members by user id.
  - A group can carry a role, and the highest wins.
  - A queue can take its agents from a group.
  - SCIM Groups and an identity provider's group claims both land here.
- **SCIM 2.0 is the provisioner's door.** `/scim/v2/Users` and
  `/scim/v2/Groups` per tenant (RFC 7643/7644):
  - filters (`userName eq`, `externalId eq`, `emails[value eq]`), paging,
    PATCH and ETags;
  - an FCP extension schema for extension, role and location;
  - authenticated by an API token holding `scim_provision`.
- **A phone authenticates the way phones can.**
  - Each tenant gets a provisioning base URL with a secret in its path:
    `https://<host>/provision/<tenant-key>/`. HTTPS is required, and the
    key can rotate.
  - Under it, vendor file names are served with the vendor's content
    types.
  - A device may also have HTTP credentials, which a redirection service
    hands the phone with the URL.
  - Every fetch is recorded: device, time, address, User-Agent.
- **A phone finds its server by itself.**
  - The device's MAC is registered with its vendor's redirection service
    (Yealink RPS, Poly ZTP, Grandstream GDMS, Fanvil FDPS) when the device
    is created or claimed. The service points the phone at the tenant's
    URL.
  - DHCP option 66/160 is documented for networks without one.
  - A MAC nobody has claimed, fetching under a tenant's key, is recorded
    as a pending device. The admin assigns it to a user in one step.
  - MACs can be imported in bulk.
- **Templates are real.**
  - A Jinja-style engine (`minijinja`) with conditionals and loops.
  - The template is picked by vendor and model.
  - Variables cover transport (TLS where the platform serves it), port,
    time zone, NTP, the firmware URL, the provisioning URL itself, and
    BLF keys.
  - Existing `${var}` templates still render.
  - A device or template change can be pushed with a resync (the vendor's
    `check-sync` variant), per device or for every device of a template.
- **Firmware is managed.**
  - Images are uploaded per vendor and model, stored like recordings, and
    served under the provisioning URL.
  - A tenant sets a target version per model, rolled out to a pilot set
    first if it likes. The template's firmware URL follows the target.
  - The version each phone runs is read from its User-Agent, so the admin
    sees who is behind.
- **The device knows itself.** Registration state, address and User-Agent
  are kept on the device as they change. The model and firmware are parsed
  from the User-Agent, and the single view shows them.

## 3. Phases

Each phase is one PR based on main, with its admin UI and its docs.

1. **Users have ids and sessions are rows.**
   - Migration: `users.id` unique.
   - Sessions and API tokens name the user's id. Tokens made before
     carry the username and are matched once by tenant and username, then
     rewritten.
   - Rename (`PUT /users/:username` with a new `username`) and
     `/users/by-id/{id}`.
   - `sessions` with `sid`; logout and disable revoke.
   - `[api] session_signing_key` with a key id, the old derivation still
     accepted until it expires.
   - Session lifetime configurable.
   - The UI's refresh cookie, so a reload keeps you signed in.
   - Login audit naming the user and tenant, and `user.signed_in` /
     `user.sign_in_failed` events.
   - Proved:
     - a re-created username does not inherit the old user's sessions or
       tokens;
     - logout revokes;
     - a key rotation keeps sessions.
   - **Done (v0.25.308)** (`docs/API_TOKENS.md` §7).
     - `fcp_core::session`:
       - `SessionKeys`: HS256 with a key id; the derived key kept as the
         fallback and for tokens made before;
       - `LoginSessionStore`;
       - `authenticate_session`, shared by the admin API and the events
         server, which now verifies the same tokens through it.
     - Migration 090: `users.id` unique (a duplicate re-keyed first),
       `login_sessions` referencing it with `ON DELETE CASCADE`.
     - Login answers a 15-minute access token and the `fcp_refresh`
       cookie (`HttpOnly`, `SameSite=Strict`, path `/api/v1/auth`).
       `/auth/refresh` moved before the bearer layer and takes only the
       cookie. The secret turns over in one `UPDATE`; reuse after ten
       seconds revokes the session.
     - Logout revokes. A password change revokes the user's other
       sessions, a reset all of them; a disable or a suspension revokes
       all.
     - API tokens record the maker's id in `user_id`. A username there is
       matched once and rewritten. Usernames of the form `user-<uuid>`
       are refused.
     - `[api.session]` (`signing_key`, `previous_signing_keys`, `ttl_secs`,
       `access_ttl_secs`), `FCP_ADMIN_SESSION_SIGNING_KEY`, and the events
       server's `[auth] session_signing_key`.
     - The audit names a sign-in's user and tenant (`actor_kind`
       `sign_in`); `user.signed_in` and `user.sign_in_failed`; ended
       sessions swept after a month; `GET /users/by-id/{id}`.
     - The admin UI restores a session on load, refreshes a minute before
       expiry and on a 401.
     - **Moved to phase 5: renaming a user.** A user's SIP HA1 is computed
       over the username, which a rename cannot recompute without the
       password. Separating the SIP identity from the login name belongs
       with the lifecycle.
     - Proved by
       `a_session_is_a_row_refreshed_by_its_cookie_and_revoked` and
       `a_user_made_again_with_the_same_name_inherits_nothing`
       (`api_postgres.rs`), and the key rotation and forgery tests in
       `fcp_core::session`.
2. **Tenants have login domains, and passwords are optional.**
   - `tenants.login_domains`.
   - Home-realm discovery: `POST /auth/discover {login}` answers the
     tenant's ways in.
   - The login page asks who you are first.
   - `password_hash` nullable; SIP credentials generated for a user with
     none.
   - Per-tenant `password_login` policy with break-glass.
   - TOTP for password sign-in, required by role if the tenant says so,
     with recovery codes (Q4).
   - Split in two: 2a the domains, optional passwords and the policy; 2b
     TOTP.
   - **2a done (v0.25.309)** (`docs/API_TOKENS.md` §7).
     - Migration 091: `tenants.login_domains` (GIN) and `sign_in_json`;
       `Tenant::{login_domains, sign_in}`, `PasswordLogin`,
       `get_tenant_by_login_domain`.
     - Login domains are set by a platform admin only, since a domain
       decides where a person's sign-in goes, and are unique across
       tenants.
     - `POST /auth/discover`. Login takes an email: its domain's tenant,
       then the username, then the users' email
       (`find_users_by_email_in_tenant`).
     - `password` optional on user creation, with a generated SIP
       password. An empty password hash matches nothing; before, it
       answered 500.
     - The policy is enforced after the password is checked. The API
       accepts only `allowed` until phase 3 gives a tenant an identity
       provider.
   - **2b done (v0.25.310)** (`docs/API_TOKENS.md` §7).
     - `fcp_core::totp`: RFC 6238 over HMAC-SHA1, checked against the
       RFC's vectors; a step either way accepted; ten recovery codes kept
       as SHA-256.
     - Migration 092: `user_totp`, keyed by the user's id and deleted with
       them; the secret sealed like the user's other secrets; the last
       step and the recovery codes spent in single statements, so a code
       is good once.
     - A password answers a five-minute ticket (a session claim with a
       `purpose`, which `authenticate_session` refuses) for
       `/auth/login/totp`, or for `/auth/totp/{enroll,confirm}` when the
       tenant's `sign_in.require_totp` applies to a user with none.
     - `/me/totp` to set it up, turn it off and renew the codes;
       `DELETE /users/{username}/totp` to reset it.
     - Wrong codes count toward the lockout; the count is reset only by a
       whole sign-in.
     - An expired password no longer answers a token: it was a session
       the user could use without changing it.
     - The login page's code, recovery-code and set-up steps, a QR code
       (`qrcode-generator`); the settings page's two-factor card; the
       tenant form's "Two-factor sign-in".
     - Not journaled to sites: a site's users set up their own.
     - The tenant form's login domains; the login page's "Signing in to".
     - Proved by `a_tenants_people_find_it_by_their_email_domain` and
       `a_tenants_password_policy_is_kept_and_never_shuts_out_a_super_admin`.
     - Login domains are claimed without a DNS proof of ownership: the
       platform admin vouches. A TXT-record check is a later refinement.

3. **OIDC sign-in.**
   - `identity_providers` per tenant: issuer discovery, client id, and
     secret sealed at rest.
   - Authorization code with PKCE and `state`/`nonce`; ID-token
     verification against the issuer's JWKS.
   - Claim mapping (username, email, name, groups); group-to-role
     mapping; just-in-time creation if on.
   - `user_identities`.
   - The callback answers the SPA with a one-time code it exchanges for a
     session.
   - Back-channel logout.
   - The admin UI's SSO page, and guides for Entra ID, Okta and Google.
   - Proved against a stand-in identity provider in `fcp-testkit` that
     signs real tokens.
   - Split in two: 3a the sign-in (providers, the flow, mapping,
     just-in-time creation, identities, the policy unlocked, the admin
     page); 3b back-channel logout, sites (Q11) and the guides.
   - **3a done (v0.25.311)** (`docs/SSO.md`).
     - Migration 093 holds four tables:
       - `identity_providers`, with the client secret sealed and in the
         re-encrypt list;
       - `user_identities`, keyed by provider and `sub`;
       - `sso_logins` and `sso_handoffs`, each taken by one
         `DELETE … RETURNING`.
     - `fcp_core::identity` is the model and store. `fcp_api_core::oidc`
       is the relying party:
       - discovery and the JWKS are fetched through the webhook guard and
         cached, with the keys refetched once for an unknown `kid`;
       - the code is exchanged with PKCE and client_secret_basic;
       - RS256 and ES256 are verified with the `rsa` and `p256` crates;
       - `iss`, `aud`/`azp`, `exp`/`iat` and `nonce` are checked.
     - The routes: `/auth/sso/{id}/start`, the callback bound to the
       browser by an `fcp_sso` cookie, and `/auth/sso/exchange` with a
       one-time code.
     - Discovery lists the tenant's providers. Linking is by verified
       email; creation just in time; roles follow groups, the most
       privileged winning and never `super_admin`.
     - `identity_provider_manage` (tenant admin) guards
       `/identity-providers` (CRUD and `check`), alongside
       `/users/{username}/identities`.
     - `password_login` can be restricted once an enabled provider
       exists. The last one cannot be taken away while it is.
     - `[api.sso] public_url` and `allow_private_issuers` configure it.
     - The UI gains a Single Sign-On page, the login page's provider
       buttons and its landing, and the tenant form's password policy.
     - It is proved by `fcp_testkit::oidc::StandInIdp`, which signs RS256
       with a fixed test key.
   - **3b done (v0.25.312)** (`docs/SSO.md` §5–§7).
     - Back-channel logout:
       - `POST /auth/sso/{id}/backchannel-logout` takes a `logout_token`,
         verified like an ID token, issued in the last five minutes,
         with the logout event, no `nonce`, and a `sub` or `sid`;
       - it ends sessions by the provider's `sid` or by the person.
       - Migration 094 adds `login_sessions.idp_provider_id`/`idp_sid`,
         carried from the ID token through the handoff.
     - Sites (Q11):
       - `identity_providers` and `user_identities` are journaled and
         in the slice; an identity's row id is generated as
         `provider:subject`;
       - the client secret crosses the link's mutual TLS and is sealed
         with the site's key;
       - the site's admin refuses provider writes;
       - at a site, no just-in-time creation and no roles from groups.
     - Two settings: `required_claims` (Google's `hd`, Entra's `tid`) and
       `email_verified_claim` (`xms_edov` for Entra ID).
     - Guides for Entra ID, Okta and Google Workspace.
     - Phase 3 is complete.
4. **SAML sign-in.**
   - SP metadata per provider, and IdP metadata imported.
   - Signed responses and assertions verified, with audience,
     recipient, time and replay checks. Encrypted assertions are
     optional.
   - Attribute and group mapping as for OIDC; single logout.
   - Proved against a stand-in IdP that signs with a test key. The
     library is Q2.
   - Split in two: 4a the sign-in, with the build, CI and the image
     carrying xmlsec; 4b single logout, metadata by URL, encrypted
     assertions and signed requests, and the guides.
   - **4a done (v0.25.313)** (`docs/SSO.md` §8).
     - `ProviderKind::Saml`, configured by the IdP's metadata XML
       (`SamlSettings`), with `trust_email` for linking.
     - `fcp_api_core::saml` over `samael` 0.0.22 behind the `saml`
       feature. A build without it keeps every route and answers 503.
     - Checks on top of samael's:
       - a provider without a signing certificate is refused, since
         samael would verify nothing;
       - the allowed algorithms are pinned to SHA-256 and up, digests
         included;
       - an audience naming FCP is required;
       - a transient NameID is refused.
     - Routes and state: `/auth/saml/{id}` serves FCP's metadata (also
       its entity id); `/auth/saml/{id}/acs` takes the HTTP-POST.
       Migration 095 remembers assertion ids. The `fcp_saml` cookie is
       `SameSite=None; Secure` over HTTPS.
     - The image builds with `--features saml` and carries
       `libxmlsec1t64-openssl`; CI installs `libxmlsec1-dev` and runs
       the tests with the feature.
     - The tests sign real responses with xmlsec: a sign-in, then
       unsigned, tampered, wrong-key, wrong-audience, transient,
       unsolicited, wrong-cookie and replayed responses all refused.
   - **4b done (v0.25.314)** (`docs/SSO.md` §8).
     - FCP's own key per SAML provider:
       - generated with the provider; the private half is the sealed
         `client_secret` and the certificate is `saml.sp_certificate`,
         taken from no request;
       - `POST …/sp-key` rotates it;
       - AuthnRequests and LogoutResponses are signed with it, and
         FCP's metadata names it and its logout.
     - Encrypted assertions: samael decrypts them with that key, inside
       a signed response. A test encrypts with AES-128-CBC and
       RSA-OAEP.
     - Single logout (`/auth/saml/{id}/slo`):
       - HTTP-Redirect verified over the raw query, not samael's
         verifier, which re-encodes and panics on a missing or
         non-SHA-256 `SigAlg`;
       - HTTP-POST reduced to its signed parts;
       - the issuer, destination and freshness checked;
       - `NameID` and `SessionIndex` read directly, since samael maps
         `SessionIndex` as an attribute;
       - sessions ended by the provider's session or the person, and a
         signed LogoutResponse sent back.
     - Metadata by URL: `saml.idp_metadata_url`, fetched through the
       guard on save and on `POST …/refresh-metadata`.
     - SAML guides for Entra ID, Okta and Google Workspace.
     - Not done: logout started from FCP, and refreshing metadata on a
       schedule.
     - Phase 4 is complete.
5. **A user's lifecycle.**
   - One status: Active, Suspended, Deprovisioned. `enabled` is read
     from it and kept for one release.
   - Suspend refuses SIP, sessions and tokens, drops registrations, ends
     hoteling and publishes `user.disabled`.
   - Deprovision and the tenant's grace period; the offboarding purge as
     one transaction plus the file reaper; `max_users` enforced.
   - Groups (`groups`, `group_members`), roles from groups, queues from
     groups.
   - `list_users` filtered in SQL; `user.updated` with its changed
     fields.
   - Sites: the purge reaches the site's rows through the journal.
   - Split in four: 5a one status and what suspension does; 5b
     deprovisioning and the purge; 5c groups; 5d rename (moved here from
     phase 1).
   - **5a done (v0.25.315)** (`docs/USERS.md`).
     - `User::is_suspended` / `may_use_services`, and `normalize_status`
       on every write and read (migration 096).
     - The checks that read only `enabled`, and so let a Suspended user
       in, now refuse one:
       - SIP digest (registrar and presence);
       - a device whose owner is suspended;
       - the routing gate;
       - hoteling login and the effective user;
       - provisioning;
       - the voicemail PIN, which checked no user at all.
     - Locked stops only web sign-in.
     - `status_changed` (shared with SCIM later): sessions revoked,
       registrations dropped (by username, extension and owned devices'
       SIP names) and announced, hoteling ended, `user.disabled` /
       `user.enabled` (never sent before).
     - An invalid `account_status` is 400 (it used to become Active,
       which is how the UI's "Disabled" made a user active); `disabled`
       is read as suspended.
     - `user.updated` carries `changed_fields`. `unlock` announces itself
       for the credential caches.
     - `max_users` is enforced (409, and `tenant_full` for JIT).
     - `CredentialStore::list_users_in_tenant` and `count_users_in_tenant`
       are in SQL; `GET /users?q=&status=`; the callers that filtered all
       users moved to them.
     - Proved by `a_suspended_user_is_refused_everywhere_and_their_phones_go_quiet`,
       `a_tenants_users_are_listed_filtered_and_capped` (`api_postgres.rs`)
       and `a_suspended_users_phone_is_refused_and_a_locked_ones_is_not`
       (`fcp-call-manager/tests/registrations.rs`, over SIP).
   - **5b done (v0.25.316)** (`docs/USERS.md` §4).
     - `AccountStatus::Deprovisioned` with `users.deprovisioned_at`
       (migration 097); refused everywhere a suspension is; set only by a
       delete, cleared only by `POST /users/{username}/restore`.
     - The tenant's `offboarding` (`grace_days`, default 30;
       `meetings_owner`), on the tenants API and the Tenants page.
     - `fcp_core::offboarding` (`OffboardingStore::purge_user`, one
       transaction in Postgres, under `DeletionCause::Offboarding`) and
       `fcp_api_core::offboarding` (deprovision, restore, purge, the
       hourly sweep run by `fcp-admin` under the `fcp:offboarding` lock).
     - The purge: devices unassigned; lines and user extensions released
       (numbers kept); the mailbox kept closed and off its line, removed
       by the sweep once retention empties it (its foreign key to the line
       is now `ON DELETE SET NULL`); calling settings, tokens, hoteling
       sessions, queue seats (and queues' agent lists), declared
       location, registrations and presence removed; others' forwards,
       find-me steps and assistants forgetting the user
       (`CallingSettings::forget_user`); meetings to the named owner or
       nobody; the profile picture on the file queue; a `user` row in
       the deletion log; the user row last.
     - An open erasure for the user holds the purge until it completes.
     - `POST /users/{username}/purge` for a platform admin;
       `user.deprovisioned` (with `purge_after`), `user.restored`, and
       `user.deleted` at the purge.
     - Sites: the link purges what only the site holds (tokens, hoteling,
       queue seats, declared location, registrations, the mailbox) when
       the cloud's delete of the user reaches it; the synced rows follow
       through the journal.
     - Not done: deprovisioned users still count toward `max_users` until
       purged.
     - Proved by `a_deleted_user_waits_out_the_grace_period_and_is_purged_from_every_store`,
       `a_tenant_without_a_grace_period_purges_at_once_and_a_platform_admin_may_early`
       (`api_postgres.rs`) and the site half of
       `fcp-link/tests/apply.rs`.
   - **5c done (v0.25.317)** (`docs/USERS.md` §5).
     - `groups` and `group_members` (migration 098; the membership's
       journal id is `group:user`), `fcp_core::group` (`Group`,
       `GroupMember`, `GroupStore`), journaled and synced to sites, a
       membership going with its user.
     - **Deviation from Q7: roles add up instead of "the highest wins".**
       The roles are not a ladder (a user admin manages users and a
       telecom admin does not; a telecom admin manages queues and a user
       admin does not), so taking the highest would cost a member of both
       groups one of them. A user's permissions are what their own role
       and each group's role hold (`roles_have_permission`); the most
       privileged is what is shown. A group gives neither `super_admin`
       nor `department_admin`.
     - Group roles are read with the user at every request
       (`CredentialStore::group_roles_of_user`, carried by
       `SessionPrincipal` and `AuthIdentity::Session`), so membership
       changes take effect at once; API tokens' makers, the events
       server, the password policy and the TOTP requirement count them.
     - `group_read` and `group_manage` (tenant admin alone); `/groups`,
       `/groups/{id}/members`, `/users/{username}/groups`; the admin UI's
       Groups page.
     - `idp_group`: a sign-in through a provider sets the user's
       membership of the groups that follow the provider's (roles by
       group on a provider still work).
     - A queue's `agent_group_id`: the group's members may log in with
       `*50`, asked at each login.
     - Proved by `a_groups_role_is_its_members_too_and_roles_add_up`,
       `groups_that_follow_the_identity_provider_are_set_at_every_sign_in`
       (`api_postgres.rs`),
       `a_groups_member_logs_in_to_the_queue_that_takes_its_agents_from_it`
       (`fcp-call-manager/tests/calls.rs`, over SIP) and the groups half of
       `fcp-link/tests/apply.rs`.
   - **5d done (v0.25.318)** (`docs/USERS.md` §6).
     - The SIP identity is separate from the login name: `users.sip_username`
       (migration 099, one per name in a tenant), `User::sip_name()`, the
       username until a rename and the old username after.
     - Every SIP-side lookup by name asks `CredentialStore::get_user_by_sip_name`
       / `list_users_by_sip_name` (registrar digest and tenant resolution,
       hoteling, ring plans, routing's caller, emergency, the recording
       decision, queue agents), and every SIP address built for a user uses
       the SIP name (the user's own AOR fallback, executive and forward
       targets, provisioning's account, the voicemail server's routes, new
       mailboxes); HA1 is computed over it on every password change.
     - `PUT /users/{username}` with `username` renames
       (`CredentialStore::rename_user`: one transaction with meetings and
       legacy token makers); `user.renamed`; the new name checked against
       usernames, SIP names and devices' own names.
     - `put_user` renames in place by id first, so a rename reaches a site
       through its link (the upsert on `(username, tenant_id)` would have
       collided with the unique id).
     - Not done: changing the SIP name itself.
     - Proved by `a_renamed_user_keeps_their_sip_name_their_sessions_and_what_they_had`
       (`api_postgres.rs`),
       `a_renamed_users_phone_registers_calls_and_is_reached_as_before`
       (`fcp-call-manager/tests/calls.rs`, over SIP) and the rename half
       of `fcp-link/tests/apply.rs`.
     - Phase 5 is complete.
6. **SCIM 2.0.**
   - `/scim/v2/{Users,Groups,ServiceProviderConfig,ResourceTypes,Schemas}`.
   - Filters, PATCH and ETags.
   - The FCP extension schema, and `scim_provision`.
   - Proved by a SCIM test suite run against the binary: create, rename,
     deactivate, reactivate, delete and group membership, as Entra ID and
     Okta send them.
   - Split in two: 6a users (with discovery, the permission and the
     tenant's switch); 6b groups and the provider guides.
   - **6a done (v0.25.319)** (`docs/SCIM.md`).
     - `fcp_api_core::scim` at `/scim/v2`, behind the bearer layer with the
       audit and rate limits: `Users` (create, read, list with one-`eq`
       filters and paging, replace, patch, delete),
       `ServiceProviderConfig`, `ResourceTypes`, `Schemas`; SCIM errors and
       `application/scim+json`.
     - A PATCH is applied to the user's SCIM document and taken as a
       replace, so Entra ID's path operations and Okta's value objects mean
       the same; `"True"`/`"False"` taken as booleans; a part of the name
       splits the display name.
     - ETags from `updated_at`; `If-Match` answered `412`.
     - `scim_provision` (tenant admin); refused at a site.
     - `users.scim_managed`, `users.scim_external_id` (migration 100,
       unique per tenant); `UserFilter` gained exact username, external id
       and email, and leaving out the deprovisioned, in SQL.
     - Q5: `sign_in.scim` — no just-in-time creation, and single sign-on
       only for users SCIM made (`not_provisioned` otherwise).
     - A delete deprovisions (`docs/USERS.md` §4); `userName` renames (§6);
       `active` suspends and resumes (§2).
     - Proved by `entra_id_provisions_updates_and_removes_a_user`,
       `okta_provisions_replaces_and_deactivates_a_user` and
       `a_tenant_whose_users_come_from_scim_signs_in_only_those`
       (`api_postgres.rs`), with Entra ID's and Okta's own request bodies,
       and the patch and filter unit tests in `fcp_api_core::scim`.
   - **6b done (v0.25.320)** (`docs/SCIM.md` §4, §6).
     - `/scim/v2/Groups` (create, read, list with `displayName`/
       `externalId`/`id` filters and `excludedAttributes=members`, replace,
       patch, delete) over 5c's groups; `groups.scim_external_id`
       (migration 101).
     - Members by user id, set exactly by a replace and changed by a
       patch: Entra ID's `add`/`remove` with value lists, Okta's filtered
       `members[value eq "…"]` and value objects; each change announced.
     - The group's role on FCP's group extension.
     - Setup guides for Entra ID and Okta, written from their documentation.
     - Proved by `entra_id_and_okta_provision_groups_and_their_members`
       (`api_postgres.rs`).
     - Phase 6 is complete.
7. **Provisioning that phones can use.**
   - The tenant provisioning key and URL; vendor file names and content
     types; optional device HTTP credentials.
   - The fetch log on the device, and pending devices with claim.
   - MAC bulk import.
   - `minijinja` templates picked by model, with the new variables and
     `${var}` still rendering; Fanvil and Snom starters.
   - `POST /devices/{id}/resync` and `/config-templates/{id}/resync` with
     each vendor's variant.
   - Registration written to the device, and the User-Agent parsed.
   - The old header-token route kept for one release.
   - Proved by the testkit phone fetching its files as each vendor names
     them, then registering with what it got.
   - Split in two: 7a the tenant's URL (key, vendor file names, device
     credentials, networks, the fetch log); 7b templates by model with
     `minijinja`, starters, MAC import, pending devices and claim, resync,
     the registration on the device, and the end-to-end proof.
   - **7a done (v0.25.321)** (`docs/PROVISIONING.md`).
     - `fcp_core::provisioning`: the key (128 bits, sealed at rest and found
       by its SHA-256, migration 102 `provisioning_keys` on the re-encrypt
       list), `parse_file` for each vendor's names, Polycom's master file,
       `TenantProvisioning` (`require_device_credentials`, `networks`),
       `DeviceFetch` and `ProvisioningStore`.
     - `GET /provision/{key}/{file}` outside the bearer layer, HTTPS only,
       every refusal a 404 (or a 401 asking for Basic credentials: the
       device's provisioning token); `render_device` shared with the old
       route, which stays for one release.
     - `/api/v1/provisioning/key` (read and replace) and
       `/devices/{id}/fetches`; the Devices page's URL card and fetch log;
       the Tenants page's settings.
     - Proved by
       `a_phone_fetches_its_files_from_the_tenants_url_as_its_vendor_names_them`
       (`api_postgres.rs`) and the file-name unit tests.
   - 7b split again: **7b** templates by model, starters, pending phones
     and claim, MAC import; **7c** resync, the registration on the device,
     the end-to-end proof, the old route removed.
   - **7b done (v0.25.322)** (`docs/PROVISIONING.md` §5–§7).
     - `fcp_core::config_template`: `render` over minijinja (Q12), older
       `${name}` rewritten to `{{ name }}`, XML escaping by vendor, a fuel
       budget, `check_template` on save; `TemplateContext` with every
       variable (transport, time zone and offsets, NTP, the provisioning
       URL, `accounts`, `line_keys`); `pick_template` by vendor and
       `model_matches` glob; Fanvil and Snom vendors and starters, the four
       older starters rewritten in Jinja.
     - `fcp_core::provisioning`: `identify` (each vendor's User-Agent),
       `vendor_of_mac`, `vendor_of_file`, `PhoneIdentity::of_request`;
       `PendingDevice` and the store's pending methods.
     - Devices' `vendor` and `line_keys` (migration 103, with
       `pending_devices`); `[api.provisioning]` (`sip_server`, `sip_port`,
       `sip_transport`, `ntp_server`, `public_url`) — the admin's realm
       was `fcp.local`, so phones were told an unreachable server.
     - Routes: `/provisioning/pending` (list, claim, dismiss),
       `/devices/import`, `/devices/{id}/render`,
       `/config-templates/preview`; the Phone templates page, the Devices
       page's pending card, configuration dialog, vendor and line keys.
     - Proved by
       `a_pending_phone_is_claimed_and_gets_the_template_for_its_model` and
       `phones_are_imported_all_or_none_and_xml_values_are_escaped`
       (`api_postgres.rs`) and the template and identity unit tests.
   - **7c done (v0.25.323)** (`docs/PROVISIONING.md` §8–§9) — phase 7
     complete.
     - The registration on the device: `fcp_sip::device_sync`
       (`DeviceRegistrations`, run by the registrar that takes the
       REGISTER) finds the device — own credentials, the MAC in the
       User-Agent, the address its last fetch or registration came from,
       its SIP URI, the user's only device — and writes state, address,
       User-Agent, firmware and (where unset) vendor and model through
       `DeviceStore::record_device_registration`, one UPDATE, at most every
       ten minutes unchanged; the device views show `expired` without a
       live registration.
     - Resync: `/devices/{id}/resync` and `/config-templates/{id}/resync`,
       asked of a call manager (`CallSupervisor::resync_device`, the node's
       `/devices/{id}/resync`), which sends each contact the vendor's
       NOTIFY (`check-sync;reboot=false`, `resync`, `check-sync`); hoteling
       uses the same.
     - `sip_domain` (the tenant's SIP domain, which names it to the
       registrar) and the starters registering in it through `sip_server`
       as outbound proxy; a default tenant with no row served its defaults.
     - The old `/api/v1/provisioning/{mac}` route removed;
       `[api] provisioning_auth_token` accepted with a warning for a
       release.
     - Proved by
       `a_phone_fetches_its_file_registers_with_it_and_is_told_to_fetch_again`
       (`fcp-admin/tests/provisioning.rs`, the testkit phone against the
       admin and a call manager).
8. **Zero-touch.**
   - A redirection-service connector trait. Connectors for Yealink RPS
     and Poly ZTP first, then Grandstream GDMS and Fanvil FDPS (Q9).
   - A device created or claimed is registered with its vendor;
     deleted, unregistered.
   - Vendor credentials sealed per tenant.
   - DHCP option 66/160 documented.
   - Proved against stand-ins of each vendor's API; real accounts checked
     by hand.
   - Split in two: **8a** the connector trait, Yealink RPS and Poly ZTP,
     the accounts, the device lifecycle, DHCP; **8b** Grandstream GDMS and
     Fanvil FDPS.
   - **8a done (v0.25.324)** (`docs/PROVISIONING.md` §10).
     - `fcp_api_core::redirection`: `RedirectionConnector` (register,
       unregister, check, whether it carries credentials); `YealinkRps`
       over the RPS JSON API (HMAC-SHA256 signing per its guide, a
       device's own `uniqueServerUrl` and credentials, add then edit when
       it exists, delete by RPS's id); `PolyZtp` over the ZTP XML API
       (`add-sip-device` to the tenant's profile under its Reference ID,
       moved when associated elsewhere, `delete-sip-device`,
       `query-sip-device` as the check).
     - Accounts per tenant and service (`redirection_accounts`, migration
       104, the secret sealed and on the re-encrypt list); where each
       device stands (`device_redirections`); `[api.redirection]` for the
       vendors' endpoints, the operator's, not a tenant's.
     - A device registered when made, claimed, imported or first changed
       with an account on, moved with its MAC, taken off when disabled or
       deleted, on a task of its own; RPS given the device's token, or a
       new one when the tenant requires credentials; tokens now 32
       characters (RPS's limit).
     - Routes: `/provisioning/redirection` (list, put, delete, check,
       sync), `/devices/{id}/redirection` (read, register now); the
       Devices page's *Zero-touch* card and *Register with its vendor*.
     - DHCP options 66 and 160 documented.
     - Proved by
       `a_phone_is_registered_with_its_vendors_service_and_removed_with_its_device`
       (`api_postgres.rs`) against `fcp_testkit::redirection`'s stand-ins
       (RPS checking each signature); not yet run against the real
       services (Q9).
   - **8b deferred (2026-10-05).** Neither remaining vendor publishes what
     a connector needs: Grandstream's GDMS API guide covers sign-in, sites
     and adding a device (which wants the serial number on its box), not
     assigning it to an RPS profile or removing it, and the full reference
     (`doc.grandstream.dev`) does not load; Fanvil publishes no FDPS API.
     Their phones find FCP by DHCP (options 66/160) or by the vendor's
     console pointed at the tenant's URL by hand, until a vendor's
     documentation or an account to read it from arrives.
9. **Firmware.**
   - The image store (directory or S3, like recordings) and upload
     (Q10); serving under the provisioning URL.
   - Per-model target versions, and a pilot set.
   - The template's `firmware_url`.
   - Compliance from the parsed User-Agent; `device.firmware_behind`.
   - The admin UI's Firmware page.
   - Split in two: **9a** images in a directory, upload, targets with a
     pilot stage, serving, `firmware_url`, compliance and
     `device.firmware_behind`, the Firmware page; **9b** images in an S3
     bucket, and a site fetching the cloud's images over the link and
     caching them (Q11).
   - **9a done (v0.25.325)** (`docs/PROVISIONING.md` §11).
     - `fcp_core::firmware`: `FirmwareImage` (vendor, model patterns,
       version, file, size, SHA-256), `FirmwareTarget` with a
       `RolloutStage` and pilot devices, `target_for`, `FirmwareStatus`;
       `firmware_images` and `firmware_targets` (migration 105).
     - `fcp_api_core::v1::firmware`: upload streamed to `[api.firmware]
       directory` with the size limit and a hash, the platform
       administrator's (Q10); targets per tenant with counts of current,
       behind and unknown; `/provision/<key>/firmware/<image>/<file>`
       under the provisioning guards; `firmware_url`,
       `firmware_directory`, `firmware_target_version` in templates and
       Polycom's master file; the starters' firmware lines; the device
       views' `firmware_status`.
     - `device.firmware_behind`, said by the registration writer once per
       version a phone reports behind its target.
     - The Firmware page and the Devices page's Firmware column; the
       compose stack keeps images in `admin-data` and its proxy takes
       large uploads.
     - Proved by
       `firmware_is_uploaded_once_targeted_per_tenant_piloted_and_served`
       (`api_postgres.rs`) and the target and status unit tests.
   - 9b split again: **9b** images in a bucket; **9c** a site fetching the
     cloud's images over the link and caching them (Q11).
   - **9b done (v0.25.326)** (`docs/PROVISIONING.md` §11).
     - `fcp_api_core::firmware_files::FirmwareFiles` (put, open, delete)
       with `DirectoryFiles`; `fcp-admin`'s `S3Files` over the CDR
       archive's `S3Target`, which gained `get` (streamed, its own
       deadline) and `delete`; `[api.firmware.s3]`.
     - An upload staged locally, put with its SHA-256 signed and confirmed
       by size and hash before the image is listed; fetches streamed from
       the bucket through the admin; removal deletes the object.
     - Proved by `an_image_lives_in_the_bucket_and_is_served_from_it`
       (`fcp-admin/tests/firmware_s3.rs`) and
       `reads_an_object_back_and_deletes_it` (`fcp-cdr/tests/s3.rs`),
       against Garage locally; both skip without `FCP_TEST_S3_*`, as CI
       runs them.

## 4. What this does not do

- It adds no directory sync over LDAP: SCIM is the provisioning path.
  Active Directory reaches FCP through Entra ID or an IdP that speaks
  SCIM.
- It adds no device management beyond configuration and firmware: no
  remote reboot of arbitrary phones, no screen capture, no packet capture.
- It does not validate phones' factory certificates (mutual TLS with
  vendor MICs). The provisioning key, HTTPS and optional credentials are
  the defence; certificate-bound provisioning is a later project.
- It builds no softphone: clients are the review's last item.
- It adds no WebAuthn or passkeys for password users. TOTP is the second
  factor (Q4), and SSO users get theirs from their IdP.

## 5. Questions

1. **OIDC and SAML both?** *Recommended:* yes, OIDC first (phase 3), then
   SAML (phase 4). Entra ID, Okta and Google all speak OIDC. Many
   enterprises still require SAML of a vendor, and the review's products
   all offer both.
2. **Which SAML library?** *Recommended:* `samael`, behind a `saml` cargo
   feature of `fcp-admin` that the images enable. It verifies XML
   signatures through `xmlsec`, a C library the images must then carry
   (`libxmlsec1`). The alternative, verifying XML signatures with our own
   canonicalization, is where SAML implementations get their CVEs.
3. **How does the UI hold a session?** *Recommended:* an access token in
   memory, valid for 15 minutes, and a refresh token in an HttpOnly,
   `SameSite=Strict` cookie scoped to `/api/v1/auth`, rotated on use. A
   reload keeps you signed in, script cannot read the long-lived secret,
   and the API's bearer model is unchanged for everything else.
4. **MFA in this project?** *Recommended:* TOTP for password sign-in,
   which a tenant can require by role, with recovery codes; phase 2. SSO
   users get their second factor from their IdP, and WebAuthn is later.
5. **Just-in-time creation, SCIM, or both?** *Recommended:* both, per
   identity provider. JIT creates an end user on first sign-in, with the
   role its groups map to. When SCIM is configured, JIT is off and a user
   SCIM has not created cannot sign in.
6. **What does a delete do?** *Recommended:* SCIM's or an admin's delete
   suspends at once and purges after the tenant's grace period (default
   30 days), so an offboarding mistake can be undone and a mailbox
   collected. The purge is the defined offboarding of §2. The platform
   admin can purge at once.
7. **What is a group for?** *Recommended:*
   - a membership store;
   - a role by group (the highest wins);
   - a queue's agents by group.

   Hunt and pickup groups stay where they are, in config, for a later
   project.
8. **How does a phone authenticate?** *Recommended:* the tenant's key in
   the URL path, HTTPS only, plus per-device HTTP credentials where the
   redirection service can carry them (RPS and ZTP can). Factory
   certificates are later (§4). A tenant may also restrict provisioning to
   its networks.
9. **Which redirection services?** *Recommended:*
   - Yealink RPS and Poly ZTP first: the two most deployed in the
     review's market.
   - Then Grandstream GDMS and Fanvil FDPS, behind the same trait.
   - Each tested against a stand-in of its API. The real services need
     vendor accounts we do not have; they are checked by hand by whoever
     has one.
10. **Who uploads firmware?** *Recommended:* the platform admin uploads
    images, which every tenant can see. A tenant chooses its target
    versions and rollout. Images are large and vendor-licensed, and one
    copy serves everyone.
11. **Sites?** *Recommended:*
    - A site's admin talks to the tenant's identity provider itself: the
      provider configuration syncs, without its secrets, which the site
      fetches over the link and seals.
    - While the IdP is unreachable, password sign-in is the fallback,
      only if the tenant's policy allows it.
    - SCIM goes to the cloud only, and users reach the site by the
      journal.
    - A site's phones provision from the site's admin.
    - Firmware is fetched from the cloud by the link and cached at the
      site.
12. **Which template engine?** *Recommended:* `minijinja`. It is pure
    Rust, Jinja syntax that integrators already know, and sandboxed. A
    `${var}` template is rewritten to `{{var}}` when it renders, so
    nothing a tenant has breaks.
