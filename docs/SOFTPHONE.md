# The softphone

Designed 2026-10-05. FCP serves desk phones, browsers joining conferences and
API clients, but it has no softphone of its own and nothing a softphone app
could sign in to. Anvil (`thevoiceguy/anvil`) is a cross-platform softphone
in Rust, built on the same SIP and media stacks as FCP: siphon-rs and
forge-media. This document designs how Anvil becomes FCP's own client:

- a person signs in with their FCP account (password, TOTP, SSO);
- the app is enrolled as one of their devices, with SIP credentials of its own;
- it fetches its configuration and the tenant's branding;
- it uses FCP's calling features: voicemail, BLF and presence, transfer and
  park, forwarding and DND.

Agreed 2026-10-05: the first target is Anvil as an FCP client on the
desktop, with mobile push later. Work in the anvil repo is a branch and PR
per phase, merged by the owner, as in FCP. The questions in §6 are open.

Progress: phase F4a done (v0.25.358): `/me/calls` and `/me/directory`
(§2.7); `/me/events` is F4b. Phase F3 done (v0.25.359): the tenant's brand
(`docs/BRANDING.md`). Phase F1a done (v0.25.334): app sign-in and app sessions
(`docs/APP_SIGN_IN.md`); the password grant of §2.2 became
`POST /auth/app/sessions`, one sign-in path rather than a copy of it.

## 1. What FCP has, and what it lacks

Two surveys ran on 2026-10-05, one of FCP and one of Anvil, and their
sharpest findings were checked against the files.

### 1.1 Signing in: built for a browser

- **Password and TOTP work for any client.** `POST /api/v1/auth/login`
  answers `{token, expires_in, user, mfa_required, mfa_token, …}`
  (`v1/auth_session.rs:82`), and `/auth/login/totp` finishes a TOTP sign-in.
- **Refresh is a cookie.** `fcp_refresh` is `HttpOnly; SameSite=Strict`, on
  path `/api/v1/auth` (`auth_session.rs:35`, `:363`). `POST /auth/refresh`
  reads only that cookie (`:1289`), and a session ends at 12 hours however
  often it is refreshed (`docs/API_TOKENS.md`). An app would have to replay
  a browser's cookie, and the user would sign in twice a day.
- **SSO lands on the web UI.** The redirect URI is fixed at
  `{public_url}/api/v1/auth/sso/callback` (`state.rs:214`). The callback
  sends the browser to `{public_url}/login?sso_code=…` (`v1/sso.rs:587`),
  and `/auth/sso/exchange` trades the code for a session. PKCE runs between
  FCP and the identity provider, not between the client and FCP. There is
  no loopback redirect, no custom URI scheme and no client-supplied
  `redirect_uri`, so a native app can finish SSO only by scraping a webview.
  SAML has the same shape.
- **Personal API tokens reach `/me`,** but the user would have to make one
  and paste it into the app, and a token is not tied to a device.

### 1.2 SIP credentials: only for desk phones

- **No `/me` route returns a SIP account.** `/me` has profile, password,
  TOTP, voicemail (messages, audio, greetings), location, calling, executive,
  hoteling, conferences and API tokens (`v1/mod.rs:132`). None of them
  returns a SIP server, transport, realm, username or password.
- **Devices can have their own credentials.** A device has
  `DeviceType::Softphone` (`fcp-core/src/device.rs:400`), its own
  `sip_username` and HA1 (`device.rs:46`), and the "may act as" rule that
  lets a device register and call as its owner (`fcp_sip::hoteling`,
  `docs/ENTERPRISE_CALLING.md` phase 8). Only an administrator can create
  one.
- **Provisioning is keyed by a MAC.** `GET /provision/{key}/{file}` renders
  a template with the SIP settings (`docs/PROVISIONING.md`), and finds the
  device by the MAC in the file name. A softphone has no MAC to give.
- **A user's SIP hash comes from their web password** unless an
  administrator stores a separate SIP password
  (`docs/DIGEST_CREDENTIALS.md`). A softphone that registers as the user
  would have to hold the web password.

### 1.3 Calling features: already on SIP

FCP's SIP side already has what a softphone uses:

- **MWI.** The node sends unsolicited NOTIFYs with
  `application/simple-message-summary` to every registered contact of a
  line's devices (`fcp-sip/src/mwi.rs`). A SUBSCRIBE to `message-summary` is
  refused with 489.
- **Presence and BLF.** SUBSCRIBE to `presence`, `dialog` and
  `presence.winfo`, and PUBLISH (`fcp-sip/src/presence.rs:712`).
- **Shared lines.** BroadSoft `call-info` and `line-seize`
  (`fcp-sip/src/sca.rs`).
- **Feature codes.** Park and retrieve `*70`/`*71`, pickup `*8`/`*9`, DND,
  forwarding, call waiting, find-me, hoteling, voicemail `*98`
  (`fcp-pbx/src/feature_codes.rs`). The same settings are also on
  `/me/calling`.
- **SRTP.** An anchored leg that offers SDES `a=crypto` is answered with
  SAVP and a fresh key (`fcp-media/src/backend.rs:2167`).
- **Transfer.** The node carries out REFER, blind and attended
  (`[b2bua.transfer] execute`).

### 1.4 What an app would also want

- **Call history.** CDRs are an administrator's (`route_permissions.rs:312`);
  there is no `/me/calls`.
- **A directory.** `/users` needs `user_read`; there is no `/me/directory`.
- **Live events for a user.** The `/events` WebSocket is an administrator's.
- **Branding.** Nothing per tenant: no app name, logo or colors. The
  REGISTER 200 OK carries Contact, Service-Route and Content-Length only
  (`fcp-sip/src/registrar.rs:1640`). FCP serves no `.well-known` document.
- **Push and ICE.** No APNs, FCM or RFC 8599, and no ICE on SIP legs: both
  for mobile, later (§5).

### 1.5 Anvil today

Anvil is at `0e9710e`; its last commit was 2026-04-20. About 6,300 lines.

- **What works.** Calls from `anvil-cli`, both ways: REGISTER with digest,
  INVITE with SDP offer and answer, G.711, G.722 and Opus, RFC 2833 DTMF,
  hold by re-INVITE, TLS, one STUN lookup and capture-side AGC
  (`docs/ROADMAP.md` phases 1 and 2). Its C interface for mobile hosts is
  five steps into phase 3 (`anvil-ffi`: start, register, call, answer,
  reject, hang up, hold, DTMF, one event callback).
- **It does not build against today's stacks.** Its siphon-rs and
  forge-media dependencies are path deps on sibling checkouts
  (`../siphon-rs`, `../forge-media`). Against siphon-rs v2026.09.30 and
  forge-media v2026.09.29, five calls break:
  - `local_uri()` now returns a `Result`;
  - `CallHandle::invite_request()` is now async;
  - `on_bye`/`on_cancel` take a `TransportContext`;
  - `MediaDescription::direction()` was renamed;
  - siphon-rs now retries an INVITE's 401/407 itself, so Anvil's own retry
    (`lib.rs:703`) would send twice.

  It has no CI and no test against a registrar.
- **Registration does not last.** One REGISTER at 3600 seconds
  (`lib.rs:268`), with no refresh, no keepalive and no handling of a network
  change. `register_expires`, `user_agent`, `provisioning_url`, the media
  config and the brand config are read by nothing. `outbound_proxy` sets the
  advertised address instead.
- **It answers most requests with 405.** It handles INVITE, re-INVITE, BYE,
  CANCEL and OPTIONS, and refuses NOTIFY, REFER and SUBSCRIBE
  (`lib.rs:605`), so FCP's MWI NOTIFYs are refused.
- **Missing features.** No CANCEL of its own outgoing call, no transfer
  (`transfer()` is a stub), no SRTP (the config's `SrtpMode` is read by
  nothing), no TCP and no session timers. One account per instance.
- **Branding is designed but stubbed.** The model and traits are in
  `anvil_core::brand`, and the wire contract is `docs/BRANDING.md`: a
  provisioning URL found from settings, then an `X-FCP-Provisioning-Url`
  header on the REGISTER 200 OK, then
  `https://{host}/.well-known/fcp-provisioning`, then `GET …/brand/v1` with
  an ETag. `anvil-brand`'s provider and cache are stubs.
- **What current siphon-rs offers that Anvil does not use:** `register_as`
  (several accounts), `subscribe`, `refer`, `cancel`, RFC 5626 outbound
  (`enable_outbound`), `contact_uri`, a dialog manager, session timers and
  keepalives.

## 2. The model

### 2.1 The app is a device

Each install of Anvil that signs in becomes one of the user's devices:
`DeviceType::Softphone`, owned by the user, with a SIP username and
password of its own that FCP makes. It registers and calls as its owner
through the "may act as" rule desk phones with their own credentials
already use. So:

- the web password never reaches SIP, and the app never stores it;
- removing the device (by the user or an administrator) stops its SIP
  access, ends its app session and drops its registrations, as suspending
  a user does (`docs/USERS.md`);
- every registration and call is on a device the admin sees, with its last
  registration and user agent, as provisioning already writes
  (`fcp_sip::device_sync`);
- a user's laptop and phone are two devices, both ringing for their line.

### 2.2 Signing in: OAuth 2.0 for native apps

FCP becomes an authorization server for its own apps (RFC 8252 on RFC 6749
and RFC 7636). This is its own client only, not an identity provider for
others.

1. **Authorize.** The app opens the system browser at
   `{public_url}/api/v1/auth/app/authorize` with `code_challenge` (S256), a
   `state` and its `redirect_uri`:
   - on a desktop, a loopback address on any port (`http://127.0.0.1:{port}/…`,
     RFC 8252 §7.3);
   - on mobile (later), a claimed `https` link or a private-use scheme.
2. **Sign in.** The page is the admin UI's sign-in: discovery by email,
   password, TOTP, or the tenant's SSO provider. That flow is unchanged; it
   ends by redirecting to the app's `redirect_uri` with a one-time code.
3. **Token.** The app posts the code and its `code_verifier` to
   `/api/v1/auth/app/token`, with a name for itself (`"Alice's MacBook"`,
   its platform and version). FCP:
   - creates the softphone device, or reuses the one this install enrolled
     before, named by an install id the app keeps;
   - answers an access token (the existing session JWT, 15 minutes) and a
     refresh token.

A **password grant** (`grant_type=password`, plus TOTP) is kept for
`anvil-cli` and tests, refused where the tenant's `sign_in` policy restricts
passwords to break-glass.

### 2.3 App sessions

An app session is a `login_sessions` row (`fcp_core::session`) of kind
`app`, tied to its device:

- the refresh token is opaque, stored as a hash, and rotated on every use;
  a reused old token ends the session (refresh-token reuse detection);
- it lasts while used: 90 days from the last refresh by default, set per
  tenant (`sign_in.app_session_days`);
- it ends when the device is removed, the user is suspended or
  deprovisioned, or the user signs out of the app (`POST /auth/app/revoke`);
- it is listed at `/me/devices` and `/users/{username}/devices`, with a
  remove button in both UIs.

The access token carries the device id, so the node, the admin API and the
audit log can say which app did what.

### 2.4 The client configuration

`GET /api/v1/me/softphone` (with the app's access token) answers everything
the app needs, with an ETag:

- **the SIP account:** the AOR in the tenant's domain, the registrar and
  outbound proxy with port and transport (from `[api.provisioning]`, as for
  phones), the realm, the device's SIP username, the registration interval,
  SRTP policy and codec order;
- **its lines:** the user's own line, and the shared lines they may use
  (as provisioning's `accounts` already says);
- **features:** the tenant's feature codes, the voicemail number, whether
  call waiting is on, BLF entries (the device's `line_keys`, or the user's
  speed dials);
- **the branding** URL (§2.5).

The SIP password is answered once, by the token endpoint at enrollment, and
again only by `POST /me/softphone/rotate-password`. It is stored as the
device's HA1, in the realm the registrars use (`sip_realm`, v0.25.329), and
never shown again.

### 2.5 Branding

Anvil's `docs/BRANDING.md` v1 becomes FCP's contract, as written:

- **A tenant brand.** App name, colors (light and dark), logo, dark logo,
  icon, ringtones, support/privacy/terms links, dial-plan hints. Made at
  `/api/v1/branding` (`branding_manage`), with assets uploaded and stored
  like prompts and firmware, and a Branding page in the admin UI.
- **Served** at `{public_url}/brand/v1?aor=…` with the app's access token
  (`Bearer`), an ETag and 304. A tenant without a brand answers 404, and
  the app shows its default theme. Assets are served from
  `{public_url}/brand/v1/assets/{sha256}`, public by hash.
- **Discovered three ways:** `/me/softphone` names it; the REGISTER 200 OK
  carries `X-FCP-Provisioning-Url` (a call manager's `[registrar]
  provisioning_url`, which compose sets); and FCP serves
  `/.well-known/fcp-provisioning`.

The `Basic` credential mode (SIP credentials over HTTP) is dropped from the
contract: the app always has a bearer token.

### 2.6 Finding the server

A person types their email address. The app asks
`https://{email domain}/.well-known/fcp-provisioning`, then
`https://fcp.{email domain}/…`, then asks for the server's address. The
document names FCP's public URL. `POST /auth/discover` then says which
tenant the address belongs to and how it signs in (password, SSO), as it
already does for the web.

A tenant whose email domain is not one FCP serves gives its users a link or
QR code: `fcp://{public_url}?tenant=…`, opened by the app.

### 2.7 What a user sees through the API

The app uses `/me` for what SIP does not carry:

- **voicemail:** already there (`/me/voicemail/*`: list, play, delete,
  greetings);
- **settings:** already there (`/me/calling`: DND, forwarding, call
  waiting, find-me);
- **call history:** `GET /me/calls`, the user's CDRs (both directions,
  their devices and lines), paged like every list;
- **a directory:** `GET /me/directory`, the tenant's users and extensions
  with each one's presence, searchable, controlled by a tenant setting
  (`directory.visible_to_users`, on by default);
- **live events:** `GET /me/events`, a WebSocket of the user's own events:
  their calls, voicemail counts, presence of the people they watch,
  settings changed elsewhere. The events server's filter already matches on
  tenant; it gains a user filter.

### 2.8 Anvil's side

Anvil keeps its rule that `anvil-core` knows SIP and media, not FCP:

- **`anvil-core`** gains what any SIP server's client needs, using
  siphon-rs's own APIs:
  - registration that lasts: refresh before expiry, RFC 5626 outbound with
    CRLF keepalives over TCP and TLS, re-register on a network change;
  - CANCEL;
  - SUBSCRIBE and NOTIFY: MWI accepted unsolicited or subscribed, `dialog`
    for BLF, `presence`;
  - REFER, blind and attended (`Replaces`);
  - SDES-SRTP through forge-sdp's `sdes` and forge-rtp's `srtp`;
  - TCP, several accounts (`register_as`), session timers.
- **A new crate, `anvil-fcp`**, holds what is FCP's:
  - discovery and the native sign-in (system browser, loopback listener,
    PKCE);
  - the token store, behind an `AccountStore` trait the host supplies
    (Keychain, Keystore, the desktop's secret service);
  - `/me/softphone` into an `AccountConfig`, and refresh;
  - branding: `anvil-brand`'s HTTP provider and cache filled in;
  - the `/me` API (calls, directory, voicemail, settings, events), as typed
    calls the UI uses.
- **`anvil-cli`** becomes a usable FCP client:
  - `anvil-cli login https://fcp.example.com`;
  - a config file;
  - interactive commands: call, answer, transfer, park, voicemail.

  It stays the reference client until a UI is chosen.
- **`anvil-ffi`** exposes the new calls and events as the work lands.

## 3. How it is proved

- **FCP's side** is proved in FCP's tests, as every phase is: the
  authorization flow against a stand-in browser, `/me/softphone`, branding
  and `/me/*` in the Postgres suite and against the shape check and the
  OpenAPI contract. The SIP side is proved with `fcp_testkit::Phone`
  registering with a softphone device's credentials.
- **Anvil's side** is proved against FCP itself. Anvil's CI starts FCP's
  compose stack from the nightly GHCR images (`docs/NIGHTLY_STACK.md`),
  signs in with `anvil-fcp`, and runs calls between two Anvil instances and
  an `fcp_testkit` phone: register, call both ways, MWI after a voicemail,
  BLF, blind and attended transfer, park, SRTP. Tests that need no FCP run
  against siphond.

## 4. Phases

Each phase is one PR. An FCP phase and the Anvil phase that uses it can be
open together. Anvil phases do not merge before the FCP phase they depend
on.

### Phase A0: Anvil on today's stacks (anvil)

- siphon-rs and forge-media as git dependencies pinned to release tags,
  with the one `[patch]` for `sip-sdp` (as FCP does). The stale patch
  block goes, and the MSRV is raised to what the stacks need (Q9).
- The five breaks fixed; Anvil's own INVITE auth retry deleted in favour of
  siphon-rs's; `contact_uri` set.
- Registration that lasts (refresh, outbound keepalives over TLS), CANCEL,
  TCP; the config fields that do nothing wired or deleted
  (`outbound_proxy` as a real outbound proxy).
- CI: fmt, clippy `-D warnings`, tests, and an integration test against
  siphond (register, call both ways, cancel, refresh).

### Phase F1: app enrollment and sessions (fcp)

- `/auth/app/authorize` and `/auth/app/token` (code + PKCE, loopback
  redirect, password grant for the CLI), the admin UI's sign-in handing the
  code back.
- App sessions: rotated refresh tokens with reuse detection, the tenant's
  `app_session_days`, revoke.
- A softphone device made per install with its own SIP credentials;
  `/me/devices` and `/users/{username}/devices`; removing a device ends its
  session and registrations.
- Docs: `docs/SOFTPHONE.md` grows the operator's and developer's chapters;
  `docs/API_TOKENS.md` and `docs/SSO.md` say how an app signs in.

### Phase F2: configuration and discovery (fcp)

- `GET /me/softphone` and `POST /me/softphone/rotate-password`.
- `/.well-known/fcp-provisioning`; `X-FCP-Provisioning-Url` on the REGISTER
  200 OK (`[registrar] provisioning_url`, set in compose).
- The UI's device page shows a softphone's platform, version and last sign-in.

### Phase A1: Anvil signs in to FCP (anvil)

- `anvil-fcp`: discovery, the browser sign-in with a loopback listener,
  `AccountStore`, `/me/softphone` into the account, refresh, rotation.
- `anvil-cli login`, the config file, interactive commands.
- The cross-repo test: Anvil's CI on FCP's nightly images, signing in and
  calling an `fcp_testkit` phone.

### Phase F3: branding (fcp)

Done in v0.25.359 (`docs/BRANDING.md`). One change from the plan: assets are
kept in Postgres with the brand, not in a directory or a bucket as firmware
is. They are small, so every admin node sees them without shared storage and
backups hold them; a site's link fetches the bytes by hash.

- The tenant brand (migration, `fcp_core::branding`), assets stored like
  prompts and firmware (directory or S3), `/api/v1/branding`
  (`branding_manage`), the Branding page.
- `/brand/v1` with ETag and 304, `/brand/v1/assets/{sha256}`.
- Synced to sites (`docs/HYBRID_CONTROL_PLANE.md`), like the rest of a
  tenant's configuration.

### Phase A2: branding in Anvil (anvil)

- `anvil-brand`'s HTTP provider and filesystem cache, discovery in the
  order `docs/BRANDING.md` gives, `Event::BrandUpdated`, the FFI event.

### Phase A3: calling features over SIP (anvil, with FCP fixes)

- MWI (unsolicited and subscribed), BLF by `dialog`, presence, blind and
  attended transfer, park and pickup by feature code, call waiting
  (a second incoming call while one is up), DND and forwarding through
  `/me/calling`.
- Whatever FCP gets wrong against a real client is fixed in FCP in the same
  phase. A known example: a SUBSCRIBE to `message-summary` gets 489.
  Fixed in v0.25.358 (`fcp_sip::mwi_subscriptions`):
  - The SUBSCRIBE is authenticated like an INVITE, for the subscriber's own
    mailbox: the user their `From` names, or the owner of a device acting
    for them.
  - It is answered 200, and the counts follow at once in a NOTIFY.
  - Each `voicemail.mwi_updated` sends another NOTIFY from whichever node
    holds the subscription.
  - A refresh or an end is handled inside the dialog. A subscriber with no
    mailbox gets 404.
  Presence and busy lamps worked only in the node's default tenant; fixed
  in v0.25.360 (tenant per request, colleagues may watch without a policy,
  the rung user's lamp lit, 489 not 500 for an unknown package).

### Phase F4: what a user sees (fcp)

F4a done in v0.25.358:

- `GET /me/calls` finds the user's records the way a subject export does.
  The user is expanded to their username, extension, numbers, line
  bindings and addresses on the tenant's domains. Candidates come through
  a GIN index on the parties' canonical keys (migration 110), then
  `SubjectMatcher` keeps only the user's own. Results are newest first,
  keyset-paged, with `missed=true` for missed calls only.
- `GET /me/directory` lists the tenant's active users and the extensions
  that ring no one person, each with its presence, and takes `q` to search.
  It answers 403 while the tenant's `directory.visible_to_users` is off.
  The setting is on the tenant API and the admin UI's tenant form.

F4b, `/me/events`, follows.

- `/me/calls`, `/me/directory` (with `directory.visible_to_users`),
  `/me/events` (the events server filtering by user), all in the contract
  and the SDKs.

### Phase A4: the app's data (anvil)

- `anvil-fcp`'s typed `/me` calls: call history, directory with presence,
  voicemail, settings, live events; in the CLI and the FFI.

### Phase A5: secure media and the proof (anvil, with FCP fixes)

- SDES-SRTP in Anvil (`SrtpMode` read), on by default over TLS;
  FCP offering SRTP toward a softphone device that registered over TLS
  (today only trunks are offered it).
- The cross-repo suite completed (§3); `docs/SOFTPHONE.md`'s user guide;
  Anvil's README and roadmap brought up to date.

## 5. What this does not do

- **Mobile push and the background.** RFC 8599 push parameters in REGISTER,
  an APNs and FCM sender in FCP, and holding an INVITE until the woken app
  registers are the next project, with Anvil's phase 4. They need Apple and
  Google developer accounts to prove on devices.
- **ICE** for SIP legs. FCP latches RTP, which serves a desktop behind most
  NATs. Strict NATs and mobile networks come with the mobile project.
- **A UI.** Anvil's roadmap puts the UI last (Flutter recommended). This
  project ends with a CLI and a C interface that a UI can be built on.
- **Video, conferencing inside the app, recording.** Anvil's non-goals;
  conferences are joined by dialing in or through the WebRTC join page.
- **A browser softphone.** The WebRTC gateway places outbound calls and
  joins conferences, but it neither registers nor rings a browser. That is
  a separate project.
- **FCP as an identity provider for other apps.** The authorization
  endpoint serves FCP's own apps (a fixed client id), not arbitrary
  third-party clients.

## 6. Questions

Each has a recommendation.

1. **Where the design lives.** In FCP (`docs/SOFTPHONE.md`), with Anvil's
   roadmap pointing to it, or in Anvil? *Recommendation:* in FCP. Most of
   the server work is FCP's, and FCP keeps its designs in `docs/`. Anvil's
   `ROADMAP.md` and `BRANDING.md` link to it.
2. **The app's identity.** A device with SIP credentials of its own, or the
   user's SIP password? *Recommendation:* a device of its own (§2.1). It
   keeps the web password out of SIP and makes the app removable.
3. **Native sign-in.** OAuth 2.0 for native apps, with code and PKCE and
   the system browser running the existing sign-in, or an in-app password
   form plus a webview for SSO? *Recommendation:* the system browser (§2.2).
   It is the only way SSO and MFA work unchanged, and RFC 8252 requires it.
   Keep a password grant for the CLI.
4. **App session lifetime.** *Recommendation:* rotated refresh tokens, 90
   days from last use, set per tenant, ended by device removal, suspension
   or sign-out (§2.3).
5. **The branding contract.** Adopt Anvil's `BRANDING.md` v1 as written?
   *Recommendation:* yes, with bearer-only authentication and FCP-hosted
   assets by hash (§2.5).
6. **Finding the server.** *Recommendation:* the email domain's
   `.well-known` document, then asking for the address, plus an
   `fcp://` link or QR code (§2.6).
7. **Where FCP-specific code lives in Anvil.** *Recommendation:* a new
   `anvil-fcp` crate. `anvil-core` stays a standards SIP client that would
   work against any server (§2.8).
8. **Testing across the two repos.** *Recommendation:* Anvil's CI runs
   FCP's stack from the nightly GHCR images (§3). FCP's CI stays free of
   Anvil.
9. **Anvil's MSRV.** It says 1.75. *Recommendation:* raise it to what
   current siphon-rs and forge-media need, checked in Anvil's CI.
10. **A user's directory.** *Recommendation:* every user sees the tenant's
    directory by default, and a tenant can turn it off
    (`directory.visible_to_users`).
11. **Two devices ringing.** *Recommendation:* a softphone is another of
    the user's devices, so all their registered devices ring (forked, as
    today). The user can choose in the app not to ring the desktop while
    their desk phone is registered, as a device setting.
