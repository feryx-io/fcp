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
about. This document designs the fixes. The questions in §5 are open.

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
4. **SAML sign-in.**
   - SP metadata per provider, and IdP metadata imported.
   - Signed responses and assertions verified, with audience,
     recipient, time and replay checks. Encrypted assertions are
     optional.
   - Attribute and group mapping as for OIDC; single logout.
   - Proved against a stand-in IdP that signs with a test key. The
     library is Q2.
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
6. **SCIM 2.0.**
   - `/scim/v2/{Users,Groups,ServiceProviderConfig,ResourceTypes,Schemas}`.
   - Filters, PATCH and ETags.
   - The FCP extension schema, and `scim_provision`.
   - Proved by a SCIM test suite run against the binary: create, rename,
     deactivate, reactivate, delete and group membership, as Entra ID and
     Okta send them.
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
8. **Zero-touch.**
   - A redirection-service connector trait. Connectors for Yealink RPS
     and Poly ZTP first, then Grandstream GDMS and Fanvil FDPS (Q9).
   - A device created or claimed is registered with its vendor;
     deleted, unregistered.
   - Vendor credentials sealed per tenant.
   - DHCP option 66/160 documented.
   - Proved against stand-ins of each vendor's API; real accounts checked
     by hand.
9. **Firmware.**
   - The image store (directory or S3, like recordings) and upload
     (Q10); serving under the provisioning URL.
   - Per-model target versions, and a pilot set.
   - The template's `firmware_url`.
   - Compliance from the parsed User-Agent; `device.firmware_behind`.
   - The admin UI's Firmware page.

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
