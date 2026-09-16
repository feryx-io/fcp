# Emergency Calling, Block 2 — Design

Status: **agreed** (2026-09-15), every recommendation in §12 taken.
All six phases are done (v0.25.176 to v0.25.181, see §11). This
document is the plan for the four items `docs/E911.md` left for later — **location by value** (PIDF-LO) for
NG911 trunks, **network-based location** for softphones and roaming
devices, **provider-backed address validation**, and **notification
delivery** inside the platform — and for the defects the survey behind it
found in block 1. `docs/E911.md` describes what exists; `STATUS.md` tracks
progress.

Read with `docs/E911.md` (location precedence, the call flow, the admin
API) and `docs/COMPLIANCE.md` (emergency call records are pseudonymised,
never deleted, on erasure).

## 1. Decisions

Proposed and agreed 2026-09-15, after surveying the emergency call path
end to end (detection, routing, the outbound INVITE, the record and event),
the SIP stack's body and header support, registrations, the WebRTC gateway,
and what the platform can already deliver (§2). The rulings on §12 are the
recommendations.

1. **Every path an emergency call can take is an emergency path.** Block 1
   detects and marks calls in the B2BUA's ordinary INVITE path. A browser
   can dial 911 through the WebRTC gateway with no identity and no
   location, a flow's dial node builds its own INVITE, and a request the
   handler sends over a reused TLS connection loses every header it adds.
   Before any location work, every one of these gets the same classification, headers,
   location and record — or refuses the call with a reason — and the test
   floor proves it for each.
2. **The location travels the way the trunk takes it.** A trunk declares
   `emergency_location`: `elin` (block 1: the callback number the carrier
   maps to an address), `pidf_lo` (the location by value in the INVITE, RFC
   6442), or both. The callback number goes out either way; nothing a trunk
   does not ask for is added to its INVITEs.
3. **A location is structured before it is sent.** A PIDF-LO civic address
   is made of RFC 5139 elements (house number, street, suffix, floor,
   unit…), not one line of text. The address model gains those fields; a
   validator fills them when it normalises; an address that was never
   structured is sent with what can be said about it and flagged, never
   withheld.
4. **Where the device is outranks where it was provisioned — when the
   platform can tell.** A tenant maps its networks (CIDR ranges) to sites.
   The source address of a device's registration and of the INVITE itself,
   and the client address a trusted edge (the SBC, the WebRTC gateway)
   vouches for, pick the site. A match is used for devices that move
   (softphones, mobile and browser clients); a desk phone keeps its
   provisioned site unless its tenant says otherwise (§12).
5. **A user off every known network says where they are.** For a caller
   no network places, the platform uses the location the user declared
   most recently (self-service, with a timestamp and an expiry), then their
   number's registered address, and the record says which it was and how
   old — RAY BAUM's Act asks for the best location available, not silence.
6. **Validation is a provider behind one trait, and it covers every
   address.** Sites, device overrides and user-declared locations are
   validated like phone-number addresses already are. A provider (an HTTP
   adapter with a documented contract, and named carrier adapters as they
   are needed) returns the structured, normalised address; editing an
   address clears its validation.
7. **Notification is delivered, recorded and tested.** Kari's Law requires
   an on-site notification; today the event carries free-form contacts and
   nothing sends anything. Contacts become typed (email, SMS, an extension
   to ring with a spoken announcement, a webhook), a notifier delivers each
   one with retries, every attempt is recorded against the emergency call,
   `notified` is set when one lands, and a `933` test call exercises the
   whole chain marked as a test.
8. **The record says what happened.** The emergency call record is written
   when the call is placed and completed when it ends: the trunk that
   answered, the SIP outcome, the duration, the location actually sent and
   how, and each notification.

## 2. What is wrong today

**Paths that are not emergency paths.**

- The B2BUA's headers for a B-leg — `P-Asserted-Identity` and `Priority:
  emergency` on an emergency call, the call-id header towards a conference
  node — are passed only on the plain INVITE path. A target with a TLS flow
  token goes out through `invite_via_flow`, which takes no extra headers.
  This is latent today: emergency and conference targets are routes, never
  flows. It becomes reachable as soon as a location body or header has to
  reach a flow target, so it is fixed with the SIP stack work of phase 2.
- The WebRTC gateway's `POST /v1/calls` sends any destination, `911`
  included, with no classification, no tenant, no user and no location.
  It never reads the browser's address.
- A flow's `Dial` node builds its INVITE directly, outside the handler's
  detection.
- The emergency routing branch does not set `route_trunks`, so the record
  cannot name the trunk that answered, and the event and record are
  written before routing with `trunk_name: None` and `outcome: "placed"`,
  never updated.

**Trunks.**

- `emergency_capable` exists twice: on the database trunk (`/v1/trunks`,
  the admin API) and on the TOML `[[trunks]]` of each node. Outbound
  routing — emergency routing included — is built only from TOML. The
  database flag, and database trunks generally, decide nothing but inbound
  admission by source address. An administrator who marks a trunk
  emergency-capable in the API changes nothing.

**Location.**

- The whole A-leg body is treated as SDP. A device that sends its own
  location (a multipart body with a PIDF-LO part, as NG911-aware clients
  do) breaks the call's SDP handling.
- No registration stores its source address, and `devices.last_ip_address`
  is filled only in an API response, never persisted. Nothing can place a
  device by network.
- `site_address()` always reports `validated: false`: readiness counts
  every site-derived location as unvalidated, and there is no way to
  validate a site or a device override.
- The address model is one line of street text; nothing is structured
  enough for a PIDF-LO civic address. Sites carry latitude and longitude
  that nothing uses.
- The SIP stack parses `Geolocation`, `Geolocation-Routing` and
  `Geolocation-Error` but cannot write them, has no multipart body builder
  or parser, and offers no INVITE that takes a custom body *and* extra
  headers.

**Validation.**

- `ApiState::with_e911_validator` is never called: every deployment runs
  the completeness check, whatever it needs.

**Notification.**

- `notify_contacts` is free text. Nothing delivers anything: there is no
  email or SMS code in the platform, nothing sends a SIP MESSAGE, and
  `mark_emergency_call_notified` is called only by a test.

**Tests.**

- No test places an emergency call over SIP. Detection, routing, the
  headers and the record are covered by unit tests and an API test against
  the store, not by a call.

## 3. Goals and non-goals

Goals:

1. An emergency call from any entry point — a registered phone, a
   softphone, a browser, a flow, through the SBC — is classified, routed to
   emergency-capable trunks, carries its callback number and priority, and
   is recorded.
2. On a trunk that takes it, the call carries a dispatchable location by
   value: a valid PIDF-LO with a structured civic address (and a geodetic
   point when known), referenced by a `Geolocation` header.
3. A device that moves is located by where it is: by the network it is on
   when the tenant has mapped it, by the user's declared location
   otherwise, with the source and age of the location on the record.
4. Every address a call can carry can be validated through the provider
   the operator uses.
5. The on-site notification Kari's Law requires is delivered by the
   platform, retried, recorded, and exercised by a test call.

Non-goals: location by reference (HELD, RFC 5985, or a `Geolocation`
header pointing at an HTTPS location server) — the platform is not a
location server; LoST (RFC 5222) PSAP lookup — the NG911 provider routes;
NENA i3 additional data (RFC 7852 `EmergencyCallData` blocks) beyond what
§5 names; device-side GPS or Wi-Fi positioning; text-to-911 (RTT, MSRP);
jurisdictions outside the US beyond configurable numbers (the model does
not stop a tenant using `112`, and civic address elements are
international, but only US validation is designed here).

## 4. Every path

- **The flow path.** `invite_via_flow` gains the extra headers and a body
  (a siphon-rs change, released by tag with phase 2's), and the B2BUA
  passes its headers — and, with §5, the location body — on both paths.
- **The WebRTC gateway.** An emergency number dialled from a browser is a
  call of a tenant's user, not of the gateway: the gateway's call request
  names the tenant and user it acts for (from the session's join or call
  token), the gateway sends the call through the call manager like every
  other call, and adds the browser's client address as a trusted edge
  address (§6). With no identity it refuses emergency numbers with a
  reason the browser can show, rather than sending an unlocated call
  (§12).
- **Flows.** A `Dial` node whose destination classifies as emergency hands
  the call to the handler's emergency routing instead of building its own
  INVITE.
- **The record.** The emergency routing branch sets `route_trunks`; the
  record is completed when the call terminates — trunk, SIP status,
  answered-at, duration, the location sent and its delivery method — and
  `call.emergency` keeps firing at placement, with a
  `call.emergency_completed` event after.
- **Trunks.** Emergency routing takes the trunks each node routes by. The
  database flag is made honest: see §12 for the choice between routing
  database trunks and removing a flag that decides nothing.
- **The A-leg body.** A multipart body is parsed: the SDP part is the SDP,
  a PIDF-LO part is a device-supplied location (§6), anything else is
  ignored and logged.

## 5. Location by value

### 5.1 The address

`E911Address` gains optional RFC 5139 civic elements, kept alongside the
existing line fields so nothing that reads them changes:

| Element | Field | Example |
|---|---|---|
| `country` | `country` | `US` |
| `A1` | `state` | `CA` |
| `A3` | `city` | `San Francisco` |
| `PRD` / `POD` | `street_predirection` / `street_postdirection` | `N` |
| `RD` | `street_name` | `Market` |
| `STS` | `street_suffix` | `St` |
| `HNO` / `HNS` | `house_number` / `house_number_suffix` | `1455` / `A` |
| `LOC` | `additional` | `Room 4B` |
| `FLR` / `UNIT` / `ROOM` | `floor` / `unit` / `room` | `4` |
| `BLD` | `building` | `East` |
| `NAM` | `name` | `Example HQ` |
| `PC` | `postal_code` | `94103` |

A site's floor and building fill `FLR` and `BLD`; latitude and longitude,
when set, become a geodetic point. A validator returns the elements; an
address a validator never structured is sent with `RD` holding the street
line and the record flagged `unstructured`.

### 5.2 The document

A PIDF-LO (RFC 4119, profiled by RFC 5491): one `presence` whose entity is
the callback number, one `dm:device` holding a `gp:geopriv` with the civic
address, and, when the site has coordinates, a second `dm:device` with a
`gml:Point` (RFC 5491 puts each location in its own element).
`gp:usage-rules` allow retransmission (the location is for routing and
dispatch). `gp:method` says how the location was found: `Manual` for a
provisioned site or number, `Wiremap` for a network match, and the
device's own method for a location the device supplied. Built with
`quick-xml`, which the workspace already has, and validated against the
schemas in tests.

### 5.3 The INVITE

For a trunk with `emergency_location` including `pidf_lo`, the outbound
INVITE carries a `multipart/mixed` body — the SDP, and the PIDF-LO with a
`Content-ID` — a `Geolocation: <cid:…>` header referencing it, and
`Geolocation-Routing: yes`. The SIP stack gains a multipart builder and
parser, header serialisers for the geolocation headers, and an INVITE that
takes a body, a content type and extra headers together (siphon-rs, by
release tag). A `Geolocation-Error` in a trunk's response is logged and
recorded; the call is not retried without the location unless the trunk
is also `elin`.

## 6. Where the device is

### 6.1 Networks

`emergency_networks (id, tenant_id, cidr, location_id, name, priority,
applies_to)`: a CIDR (v4 or v6) maps to a site; the most specific match
wins, then priority. `applies_to` is `nomadic` (softphones, mobile and
browser devices — the default) or `all`. Managed at `/v1/emergency/networks`.

### 6.2 The address a call comes from

- **Registrations** store their source address and port (the packet's, and
  the `received` a NAT'd client's Via implies) with each binding.
- **The INVITE's own source** is known to the handler (`peer`).
- **A trusted edge** — the SBC in front of phones, the WebRTC gateway in
  front of browsers — adds `X-FCP-Client-Address` with the address it saw.
  The call manager honours it only from peers listed as trusted edges (the
  cluster's nodes and `[e911] trusted_edges`), and strips it from anyone
  else.

The client address is the edge's when there is one, else the INVITE's
source, else the device's latest registration source.

### 6.3 Precedence

The resolver's order becomes:

1. The device's `e911_override`.
2. A location the device supplied in the INVITE, when the device (or its
   type) is trusted to supply one.
3. A network match for the client address, for devices it applies to.
4. The device's site, then the user's site.
5. The user's declared location, while it has not expired.
6. The E911 address on the user's number.

The record carries the source, the client address that matched, and for a
declared location its age.

### 6.4 Declared locations

A user sets "where I am" through self-service (`/v1/me/location`) or a
phone's feature code prompt: an address (validated like any other) and an
expiry (a day by default). A softphone whose network matches nothing and
whose user has no current declaration makes the readiness report and the
record say so; the call still goes out with the best location there is.

## 7. Validation

- **Every address.** `POST /v1/locations/:id/e911/validate` and
  `POST /v1/devices/:id/e911/validate` join the phone-number route; a
  validated site's address reaches readiness as validated. Editing an
  address clears its validation.
- **Providers.** `[e911.validator]` in `fcp-admin` selects `basic` (today's
  check) or `http`: a POST of the address to a URL with a bearer token,
  answered with `{valid, normalized: {…civic elements…}, issues,
  provider_reference}` — the contract a thin adapter for any MSAG-grade
  service implements. Named adapters for specific carriers are added when
  a deployment needs one (§12). Credentials are sealed with encryption at
  rest.
- **Structure.** A validator's normalised address carries the civic
  elements of §5.1; the basic validator splits the house number and street
  suffix where it can and says when it could not.

## 8. Notification

### 8.1 Contacts

`notify_contacts` becomes a list of typed contacts; existing strings are
migrated by shape (an `@` without a scheme is an email, a SIP URI an
extension call, digits an SMS number):

| Kind | Target | Delivery |
|---|---|---|
| `email` | address | SMTP |
| `sms` | E.164 number | the configured SMS provider (HTTP adapter) |
| `call` | an extension or SIP URI | a call that speaks the notice (TTS or a prompt) and repeats it until answered or a timeout |
| `webhook` | a webhook endpoint id | the existing dispatcher, with the notification as its payload |

### 8.2 The notice

What Kari's Law asks: that an emergency call was placed, from whom (user,
extension, device), the callback number, and the location sent — plus the
time, whether it was a test, and the source of the location. One text,
rendered per channel.

### 8.3 Delivery

The notifier runs in `fcp-events-server`, which already consumes the
platform's events and delivers webhooks, on `call.emergency`. Each contact
is attempted at once and retried with backoff for a bounded time; each
attempt is an `emergency_notifications (call_record_id, contact, kind,
attempt, status, error, delivered_at)` row; the call record's `notified`
is set with the first delivery. A tenant with no deliverable contact is a
readiness issue. SMTP and the SMS provider are `[notifications]` sections of
the events server; their credentials are sealed.

### 8.4 Test calls

A `933` test call delivers every notification with "TEST" in its subject
and first line, so a tenant can prove the chain end to end; the readiness
report shows the last successful test per contact.

## 9. Observability

- Metrics on the call manager: emergency calls by kind, by location source
  and delivery method (`elin`, `pidf_lo`), calls with no location, calls
  sent with an unstructured or unvalidated address, `Geolocation-Error`
  responses.
- Metrics on the events server: notifications by kind and outcome, time to
  first delivery.
- Events: `call.emergency` (placement, as today, gaining the client address
  and delivery method), `call.emergency_completed`, and
  `emergency.notification_failed` when a contact exhausts its retries.

## 10. Testing

- **The test floor** (`fcp-call-manager/tests`), with the scripted phone
  able to read the INVITE it receives: a registered phone dials `911`; a
  phone standing in for an `elin` trunk receives the callback number,
  `P-Asserted-Identity` and `Priority: emergency`; one standing in for a
  `pidf_lo` trunk receives a multipart body whose PIDF-LO validates and
  carries the site's structured address and a `Geolocation` header naming
  it; a
  softphone on a mapped network is located at that network's site; a
  `933` call is recorded as a test; the record names the trunk and the
  outcome after hangup.
- **The WebRTC gateway**: a browser session with a tenant identity dials
  `911` and the trunk receives its location by client network; one without
  is refused with a reason.
- **Notification**: a small SMTP sink and an HTTP SMS sink in `fcp-testkit`;
  an emergency call delivers the email, the SMS and a call to a notified
  extension that hears the notice; a failing contact is retried and
  recorded; `notified` is set.
- **Documents**: PIDF-LO output validated against RFC 4119/5139/5491
  schemas; multipart and geolocation headers round-tripped through the
  siphon-rs parser; property tests on address structuring.
- **Validation**: the HTTP validator against a test server; site and
  device validation through the API; editing clears it.

## 11. Phases

One PR per phase, reviewed before the next starts; siphon-rs changes land
by release tag first.

1. **Every path and the record.** The WebRTC gateway's emergency identity
   and refusal, flows' `Dial` node, A-leg multipart parsing,
   `route_trunks` and the completed record with `call.emergency_completed`,
   the trunk flag made honest (§12.1), the scripted phone's view of the
   INVITE, and the first emergency calls on the test floor.

   *Done (v0.25.176).* What shipped:

   - **One emergency path.** Classification, location, the outbound
     identity and the placement record live in `fcp_sip::emergency`, and
     the B2BUA's INVITE path and the flow `Dial` node both use them.
   - **The record.** The call manager keeps each record's id as its
     insert lands and completes the record from the call's ending (the
     `leg` a `Dial` node reports for its own leg). Migration 067 adds
     `sip_status`, `answered_at`, `ended_at` and `delivery`.
   - **The gateway.** The WebRTC gateway takes `caller` on
     `POST /v1/calls` and classifies against `[e911]`. It refuses an
     emergency call with no caller (403) or no `[sip] outbound_proxy`
     (503), and implements `outbound_proxy`, which it had parsed and
     ignored. It sends `X-FCP-Caller-Tenant` and `X-FCP-Caller-User`,
     which the call manager believes only from `[e911] trusted_edges`
     (the cluster's nodes are not implied).
   - **The trunk flag.** It is documented as not routing. `GET
     /emergency/trunks` on each call manager shows what does, and the
     call manager warns at startup with no emergency-capable trunk.
   - **Bodies.** `fcp_b2bua_core::body` reads multipart bodies for every
     request SDP read in the call manager, queue and conference server.
   - **Tests.** Two test files cover it: `fcp-call-manager/tests/emergency.rs`
     (911 and 933, failover, a flow, trusted edges) and
     `fcp-webrtc-gateway/tests/calls.rs`.

   Two defects the phase's tests found were fixed with it:

   - **Parallel emergency routes.** The B2BUA forked an emergency call to
     every emergency route at once, the ordinary LCR trunk included, so a
     second carrier could deliver the same 911 call. Routes are now tried
     one at a time.
   - **A race in the `Dial` node.** Its response `select!` could take a
     closed provisional channel, or the transaction's termination, for the
     outcome of a call that had been answered.
2. **Location by value.** Structured civic addresses (model, API,
   migration), the PIDF-LO builder, the SIP stack's multipart and
   geolocation additions and headers and bodies on the flow path,
   `emergency_location` per trunk, the multipart INVITE,
   `Geolocation-Error` handling, the test floor's `pidf_lo` trunk.

   *Done (v0.25.177, siphon-rs v2026.09.15).* What shipped:

   - **The SIP stack.**
     - sip-core has `MultipartBody` (build and parse) and writes the
       geolocation headers.
     - sip-parse reads `Geolocation-Routing`'s `yes`/`no` and RFC 6442's
       `code="…"`.
     - sip-uac's `InviteOptions` sends an INVITE with a body, extra
       headers and a per-call From together, also over a flow.
       `fcp_b2bua_core::body` now parses with sip-core.
   - **The address.**
     - `CivicElements` sits flattened in `E911Address`, as a site's `civic`
       column (migration 068) and `/v1/locations` field.
     - A site's floor and building fill `FLR` and `BLD`, and its
       coordinates are the point.
   - **The document.** `fcp_core::pidf_lo`, with quick-xml. One change
     from §5.2: the civic and geodetic locations are two `gp:geopriv`
     elements in one `dm:device`, not two devices, since one device with
     two device IDs describes nothing real.
   - **Checking the document.** It is checked against the RFC 5139, 4119
     and 4479 schemas' sequences and required elements, as tests copied
     from the RFCs, rather than by an XSD validator (none builds here).
   - **The INVITE.**
     - `fcp_sip::emergency::attempt_invite` builds each attempt from its
       trunk's `emergency_location`. Emergency calls in the B2BUA and the
       flow `Dial` node go through it.
     - The per-call From replaces the shared UAC override for them.
     - A refused location (`Geolocation-Error` or 424) is recorded. A trunk
       that also takes `elin` is retried without the body (§12.9); a
       `pidf_lo`-only trunk gives way to the next route.
   - **The flow path.** Flow INVITEs now carry the extra headers: the
     latent gap from §2 is closed.
   - **The record.** `location_structured` and `geolocation_error` are
     added, and `delivery` is the last attempt's methods.
   - **Tests.** The test floor has a `pidf_lo` trunk, the refusal paths
     and a flow carrying the location.
3. **Where the device is.** Registration source addresses, trusted edge
   addresses from the SBC and the gateway, `emergency_networks` and its
   API, device-supplied locations, declared locations, the new precedence,
   readiness per device type.

   *Done (v0.25.178).* What shipped:

   - **The address a call comes from.** A REGISTER's source is kept with
     the registration (`registrations.source_address`); an INVITE's own
     source places its caller; `X-FCP-Client-Address` carries what an edge
     saw, believed only from `[e911] trusted_edges`. An edge adds it with
     `vouch_client_address` — the WebRTC gateway always does (reading the
     browser's address, with `[http] trusted_proxies` for
     `X-Forwarded-For`), and the SBC gained `[core.e911]` for both settings.
   - **Networks.** `emergency_networks` (migration 069) with
     `/v1/emergency/networks`; `match_network` takes the most specific
     range, then the lowest priority, among those that place the device's
     kind.
   - **Device-supplied locations.** `fcp_core::pidf_lo::parse` reads a
     PIDF-LO a device sent, used only from a trusted device type
     (`trust_device_locations`) or a device with `e911_trust_location`.
   - **Declared locations.** `/v1/me/location` (get, put, delete), each
     validated, expiring after `declared_location_ttl_secs` (a day by
     default). Erasure covers them: `ExportClass::DeclaredLocations`.
   - **The precedence** of §6.3, with the client address and a
     declaration's age on the record and on `call.emergency`.
   - **Readiness** flags users whose devices all move but whom nothing
     places (`unplaced_nomadic`).
   - **Metrics** (§9): `fcp_emergency_calls_total`,
     `fcp_emergency_location_gaps_total`,
     `fcp_emergency_location_errors_total` on the call manager's
     `/metrics`.

   One thing found on the way: a device's type cannot be changed after it
   is created (`put_device` never updated it, and the API never offered
   it). Left as it is, since nothing needs it; noted here because a test
   had to make a new device rather than change one.
4. **Validation.** Site and device validation, structured normalisation in
   the basic validator, the HTTP provider with sealed credentials,
   validation cleared on edit.

   *Done (v0.25.179).* What shipped:

   - **Every address.** `POST /v1/locations/:id/e911/validate` and
     `POST /v1/devices/:id/e911/validate` join the phone-number route. A
     site keeps what the validator said (`e911_validated_at`,
     `e911_validated_by`, `e911_validation_reference`, migration 070) and
     its normalised street, city, state and civic elements are written
     back; a device's `e911_override` carries the same marks in its own
     JSON. A validated site's address reaches readiness as validated, so a
     user placed there is finally *ready*.
   - **Only a validator marks an address.** An address sent to the API
     with `"validated": true` arrives unvalidated, and editing a site
     address, a device override or a number's address clears the mark —
     re-sending the same address keeps it.
   - **Structure.** The basic validator splits `1455A N Market Street SW`
     into `HNO`/`HNS`/`PRD`/`RD`/`STS`/`POD`, reads a second line such as
     `Suite 300` or `4th Floor` into `UNIT`/`FLR`, and says in `notes`
     what it could not tell apart — a note is not an issue and does not
     make the address invalid.
   - **The HTTP provider.** `[e911.validator]` in `fcp-admin` selects
     `basic` or `http`; `HttpAddressValidator` posts the tidied,
     structured address with a bearer token and reads
     `{valid, normalized, issues, notes, provider_reference}`. A provider
     that cannot be reached is an error, not an invalid address.
     `with_e911_validator`, which nothing called, is now what wires it.
   - **Sealed credentials.** The token may be a value sealed with
     encryption at rest, in the file or in a mounted `token_file`, opened
     at startup; `POST /v1/system/encryption/seal` (platform admin) is how
     an operator produces one.
   - **Tests.** `crates/fcp-api-core/tests/e911_validator.rs` runs the
     HTTP validator against a provider that answers like one; the API test
     covers site and device validation, clearing on edit, and readiness
     turning ready once the site is validated; fcp-core's tests cover the
     street parser.
5. **Notification.** Typed contacts and their migration, the notifier in
   the events server (email, SMS, call, webhook), `emergency_notifications`,
   `notified`, test-call notices, the SMTP and SMS sinks in the testkit.

   *Done (v0.25.180).* What shipped:

   - **Contacts.** `EmergencyConfig.contacts` is a list of typed contacts
     (`email`, `sms`, `call`, `webhook`). Free text is read by its shape,
     in migration 071 and again on every read, so nothing a tenant saved
     before block 2 is lost; the API takes either form and answers with
     both (`contacts`, and `notify_contacts` as the plain strings it
     always was).
   - **The notice.** `fcp_core::e911::notify` builds one notice from the
     call — who placed it, the callback number, the location sent and its
     source, where the caller was seen, the time — and renders it per
     channel: an email subject and body, one line of SMS, spoken words
     that read numbers digit by digit, a webhook payload. A `933` call's
     notice is `[TEST]` in the subject and the first line and says no
     emergency services were contacted.
   - **The notifier.** The new `fcp-notify` crate holds the notifier and
     its channels; `fcp-events-server` runs it on `call.emergency` beside
     the webhook dispatcher, under `[notifications]`. Every contact is
     tried at once, each attempt is an `emergency_notifications` row
     (migration 071) carrying the notice itself, failures are retried with
     a doubling delay until `max_attempts` and never past
     `give_up_after_secs`, the first delivery sets the record's
     `notified`, and a contact that runs out of attempts raises
     `emergency.notification_failed`.
   - **The channels.** SMTP (lettre, STARTTLS required by default), an
     HTTP SMS provider with a documented contract, a call placed by a call
     manager (`POST /v1/emergency/notify-call`, which rings the extension,
     speaks the notice through TTS and answers with the outcome — only an
     answered call counts), and the existing webhook dispatcher
     (`deliver_once`). Credentials may be sealed with encryption at rest.
   - **Readiness.** `/emergency/readiness` gained a `notifications` block:
     every contact with what keeps it from being deliverable, when
     anything last reached it and when a test last did; a tenant with
     nobody to tell, or one that has never proved the chain with a `933`
     call, is told so. `GET /emergency/calls/:call_id/notifications` lists
     every attempt for a call.
   - **Metrics** (§9): `fcp_emergency_notifications_total` by kind and
     outcome and `fcp_emergency_notification_first_delivery_seconds` on
     the events server's new `/metrics`.
   - **Tests.** `fcp-notify` delivers to the testkit's new SMTP and HTTP
     sinks against a real database (every contact told, the record marked,
     retries and the failure event, a test call's notice); the storage
     suite proves free-text contacts read back typed; the API suite covers
     typed contacts, the attempt list and readiness; and the test floor
     places a `933` call through a call manager wired to a real events
     server and reads the email that arrives.

   One defect this found and fixed: `PlatformEvent::tenant_id()` returned
   a tenant only for `call.initiated`, so the webhook dispatcher dropped
   every `call.emergency` — a tenant could subscribe to emergency events
   and never receive one.
6. **Admin UI.** Emergency pages: configuration and contacts, readiness,
   networks, the call log with its notifications; site, device and trunk
   fields the UI does not show today (`e911_callback_number`,
   `e911_override`, `emergency_capable`, `emergency_location`).

   *Done (v0.25.181).* What shipped:

   - **The emergency pages** at `/admin/emergency` (Operations →
     Emergency), in four tabs: **Configuration** (numbers, prefixes, the
     typed contacts added and removed here, the device types trusted to
     send their own location, how long a declaration stands),
     **Readiness** (the summary tiles, whether anybody would be told —
     each contact with when anything last reached it and when a test last
     did — and every user with the location that would be dispatched and
     what keeps them from being ready), **Networks** (the ranges, their
     sites, what they place and their priority) and **Calls** (every
     emergency call, and behind *View* what was sent with it and every
     attempt to tell somebody).
   - **The fields the rest of the UI did not show.** Locations carry the
     ELIN and their validation, and validate from the row's menu; devices
     show where the phone would be dispatched to and take the override and
     the trust setting in the edit dialog, with validation from the menu;
     phone numbers say whether their address is validated and validate the
     same way; trunks carry `emergency_capable`, with the note that
     platform routing uses each node's own trunks.
     `emergency_location` stays TOML per node (§12.1), so the UI says so
     rather than pretending to edit it.
   - **One API change the UI needed:** a device's `e911_override` and
     `e911_trust_location` can be *cleared* (`null` on the PATCH, told
     apart from absent), so a phone that goes back to its site stops
     carrying the address it had somewhere else.
   - **Checked against a running admin server**, not only compiled: the
     configuration saved with typed contacts and read back, a site
     validated through the API and its normalised civic elements stored,
     readiness reporting a tenant with nobody to tell.

Block 2 is complete.

## 12. Open questions

Each with a recommendation; every recommendation was taken on 2026-09-15.

1. **Database trunks.** Outbound routing reads only each node's TOML
   trunks, so the API's `emergency_capable` decides nothing, and database
   trunks are tenant-scoped while the router is platform-wide.
   (a) Keep block 2 on the platform's trunks: the API flag is documented as
   taking effect only once tenant trunks route, readiness lists the
   emergency-capable trunks each node routes by, and **tenant trunk routing
   becomes its own project** (recommended — it changes all routing, not
   just emergency calls); or (b) make database trunks route in phase 1.
2. **Network match versus provisioned site.** A network match overrides a
   device's site only for nomadic devices (softphones, mobile, browser)
   unless a network is marked `all` (recommended), or always.
3. **Device-supplied locations.** Trust a PIDF-LO a device sends only from
   device types or devices the tenant marks as trusted, placing it above
   network and site (recommended), or never use one.
4. **Browsers.** The WebRTC gateway sends emergency calls only for sessions
   that carry a tenant user identity, through the call manager, and refuses
   them otherwise (recommended), or refuses emergency numbers from browsers
   altogether.
5. **Declared location expiry.** A day by default, tenant-configurable
   (recommended), or until changed.
6. **Which validators.** The HTTP adapter contract in block 2, named
   carrier adapters when a deployment asks (recommended), or one named
   provider now.
7. **Where notifications run.** The events server, beside webhooks
   (recommended: it already consumes events and retries deliveries), or the
   call manager that places the call (fewer moving parts, but the call path
   is the wrong place for SMTP retries), or `fcp-jobs`.
8. **Call notification.** A call that speaks the notice to an extension,
   repeated until answered (recommended: every desk phone can receive a
   call; few render SIP MESSAGE), or SIP MESSAGE.
9. **When `pidf_lo` fails.** On a trunk that is `pidf_lo` only, a
   `Geolocation-Error` or a 4xx naming the body fails over to the next
   emergency trunk (recommended), or the same trunk is retried without the
   body.

## 13. Not in scope

Location by reference (HELD) and running a location information server;
LoST and PSAP routing; RFC 7852 additional data beyond the device, service
and subscriber basics a later block may add; text-to-911; non-US address
validation; tenant trunk routing beyond the emergency flag (§12.1); device
positioning (GPS, Wi-Fi, Bluetooth beacons).
