# Push: ringing a sleeping phone

Designed 2026-10-09. A phone app cannot keep a SIP connection open while it
sleeps: iOS suspends it within seconds of leaving the screen, and Android
stops its network in Doze. Its registration outlives the connection, so a
call to it reaches a binding with no way in. The platforms' answer is a
push: Apple's push service (APNs, VoIP pushes through PushKit) and Google's
(FCM) wake the app, which re-registers and takes the call. RFC 8599 says
how a SIP server and a SIP app agree on that: the app puts its push
address in the REGISTER, and the server, when a request for it cannot be
delivered, pushes and waits for the app to come back.

This document designs that for FCP and for Anvil, FCP's softphone
(`docs/SOFTPHONE.md`), whose mobile app (Anvil's U4) works while it is
open. It is Anvil's U5 (`thevoiceguy/anvil` `docs/APP.md`). Agreed
2026-10-09 on every recommendation in §6.

Progress: phase P1 done (v0.25.377). A REGISTER's Contact keeps its
`+sip.instance` and `reg-id` for every phone, and a new binding from an
instance replaces its others (announced `registration.unregistered`,
reason `replaced`); `pn-provider`, `pn-param` and `pn-prid` are stored
apart from the contact (migration 111), refused with 555 and
`Feature-Caps` when this node has no key for them, answered with
`Feature-Caps: *;+sip.pns="…"` and the phone's own Contact when it has;
a push binding lives up to `[push] max_expires`, past siphon-rs's day
(the row holds the expiry, the 200 OK is built from rows); bindings over a
flow name their node (`flow_node`); a device's `push_provider` in both
device views and the UI. `[push]` is read by the call manager and the
standalone registrar from the file only (never written to the stored
sections), and validated at start.

Phase P2 done (v0.25.378): `fcp-push` sends — APNs over HTTP/2 with a
provider token (ES256, made again every fifty minutes, a refused one
dropped), FCM's HTTP v1 with the service account's access token (RS256
assertion, kept until five minutes before it lapses); once, within five
seconds; `apns-expiration: 0`, FCM's TTL thirty seconds; `payload =
"minimal"` honoured. A token the provider no longer knows (APNs 410,
`BadDeviceToken`, `DeviceTokenNotForTopic`; FCM `UNREGISTERED`,
`NOT_FOUND`) takes the push address off its binding, which stays, and is
told as `device.push_failed`. `fcp_push_total{provider,kind,outcome}` on
the call manager's `/metrics`. Sealed keys (`enc:v1:…`) are opened at
start. `POST /api/v1/devices/{id}/push-test` (`device_update`) reaches the
first call manager with `[push]` (one without answers 503 and is passed
over), which sends a `test` push to each of the device's bindings that can
be woken and says what each did; the Devices page offers it. Stand-ins:
`fcp_testkit::push::{StandInApns, StandInFcm}`, each checking the
provider's authentication and headers. Each `[[push.apns]]` and
`[[push.fcm]]` takes an `endpoint`, for a stand-in.

## 1. What FCP has, and what it lacks

Nothing of push exists in FCP or siphon-rs: no `pn-*` parameter, no
`Feature-Caps`, no 555, no APNs or FCM client.

### 1.1 Registering

- REGISTER reaches `FcpRegistrar::handle_register_from`
  (`fcp-sip/src/registrar.rs:1086`) on the call manager, the standalone
  registrar and the SBC (site links only).
- Only the first Contact is read, and only its `<…>`:
  `extract_contact_uri` (`registrar.rs:1458`) drops every Contact header
  parameter — `+sip.instance`, `reg-id`, `q`, `expires`. URI parameters
  inside the brackets survive (`normalize_contact_for_matching` removes
  only `rinstance` and `gr`), so a `pn-*` parameter would be kept, but as
  part of an opaque string and of the binding's key.
- siphon-rs's `Binding` has `instance_id` and `reg_id`; FCP never fills
  them, and `fcp_core::Registration` (`storage.rs:727`) has no field for
  them.
- Expiry is clamped to 60–86400 seconds without a 423
  (`registrar.rs:1561`).
- The 200 OK carries Contact, Service-Route and `X-FCP-Provisioning-Url`
  (`create_ok_response`, `registrar.rs:1664`); no Supported and no
  Feature-Caps.
- A binding is linked to a device after it is made (`note_device`,
  `device_sync.rs`); an app install's device is the one F1b made
  (`app_installs`, migration 108). Nothing stores a push token.

### 1.2 Flows

- A TLS REGISTER mints a flow token (`handler.rs:10965`); the flow is an
  entry in the node's in-memory `FlowRegistry` (`flow.rs:46`). Nothing
  records which node holds it.
- A call to a contact whose flow is gone — the app asleep, or the flow on
  another node — logs "Flow token not found or connection closed, falling
  back to normal INVITE" and sends the INVITE to the contact's address
  (`handler.rs:7257`). Behind NAT, and always for a sleeping app, that
  fails after five seconds.

### 1.3 Ringing

- `ring_plan::contacts_for_user` (`ring_plan.rs:280`) reads the user's
  devices' bindings once, when the INVITE arrives; `plan` (`:56`) answers
  "unreachable" when there are none (`:81`), and the find-me steps or the
  no-answer treatment take over.
- `handle_parallel_forking` (`handler.rs:7877`) races a fixed set of legs
  against the caller's CANCEL and the ring timeout (`:8175`). A fork in
  progress cannot gain a leg.
- The call lives on the node that took the INVITE (`CallManager.calls`);
  a REGISTER on any node publishes `registration.registered` on the event
  bus (`registrar.rs:1428`), which every node receives.

### 1.4 Outbound services and secrets

- reqwest with rustls in the call manager, the API and `fcp-notify`;
  fcp-sip has no HTTP client. APNs needs HTTP/2 (reqwest's `http2`).
- Secrets in configuration are sealed with
  `POST /api/v1/system/encryption/seal` and opened at start
  (`[e911.validator]`, `fcp-admin/src/main.rs:898`), or come from an
  environment variable or a file. Per-tenant service accounts sealed in a
  column follow `RedirectionAccount` (`provisioning.rs:229`).
- `fcp-notify`'s channel trait and retry policy are E911's, built for
  minutes; a call's push has seconds.

### 1.5 Anvil

- siphon-rs's UAC rebuilds the Contact as `<sip:user@addr>` and drops URI
  parameters (`sip-uac/src/integrated.rs`, the REGISTER rewrite); Anvil
  never enables outbound, so it sends no `+sip.instance` or `reg-id`.
- The phone starts only from Dart (`start_phone`); there is no PushKit,
  no `voip` background mode and no FirebaseMessagingService.

## 2. The model

### 2.1 Registering with a push address (RFC 8599)

An app that can be woken registers with its push address in the Contact
URI, and its instance:

```
Contact: <sip:app-3f9c@10.0.0.7:52311;transport=tls;pn-provider=apns;
          pn-param=ABCDE12345.com.thevoiceguy.anvil.voip;
          pn-prid=8f2c…a91>;+sip.instance="<urn:uuid:…>";reg-id=1;expires=604800
```

- **Provider.** `pn-provider` is `apns` or `fcm`. A provider FCP has no
  credentials for is refused with **555 Push Notification Service Not
  Supported**, its `Feature-Caps` naming the ones it has, as RFC 8599
  §5.2 says; the app then registers without push.
- **Parameter.** `pn-param` names the app at the provider: the APNs topic
  (team, bundle, `.voip`) or the FCM project. It selects which of FCP's
  credentials send the push (§2.3); a topic FCP has no key for is a 555.
- **Token.** `pn-prid` is the device's token. It is stored with the
  binding, never logged whole, and shown truncated.
- **Answer.** The 200 OK carries `Feature-Caps: *;+sip.pns="apns"` (or
  `"fcm"`): FCP will push for this binding.
- **The binding outlives the connection.** A push binding may ask for a
  longer expiry than an ordinary one, up to `[push] max_expires`
  (default seven days): the app re-registers whenever it runs, and a push
  wakes it to re-register when a call needs it. The registrar keeps the
  Contact's `+sip.instance` and `reg-id` for every binding (§1.1's loss is
  fixed for all phones), and a push binding is matched by its instance,
  so the woken app's new address replaces the old binding rather than
  adding one.
- **Stored.** `registrations` gains `instance_id`, `reg_id`, `pn_provider`,
  `pn_param`, `pn_prid` and `flow_node` (§2.4); the device's view
  (`/me/devices`, the admin's devices) shows whether it can be woken and
  when it last was.

### 2.2 Delivering a call

When a call rings a user (`ring_by_plan`):

1. Each binding is **reachable** (a live flow on this node, or a contact
   FCP can send to directly: UDP or a desk phone) or **wakeable** (a push
   binding without a live flow).
2. Reachable bindings ring at once, as today. For each wakeable binding
   FCP sends a push (§2.3) and records a **wait**: this call, this binding's
   instance, until the ring timeout.
3. The caller hears ringing (180) as soon as anything rings or a push has
   gone, even with no leg yet: "no contacts" is no longer "unreachable"
   while a wait is open.
4. The woken app re-registers. The REGISTER, on whichever node, publishes
   `registration.registered` with the instance; the call's node matches it
   to its wait and adds the new binding to the fork in progress. The fork
   gains a fourth `select!` arm, new legs, which join the race and the
   CANCEL loop.
5. The ring timeout still governs: an app that does not come back in time
   is a no-answer like any other, and the user's forwards and voicemail
   follow. A push that fails at once (§2.3) closes its wait at once, so a
   user whose only phone cannot be woken goes straight to their
   unreachable treatment.
6. **The call ends before the app is in**: caller hangs up, another device
   answers, a forward. FCP sends a **cancel push** to every binding it
   woke, naming the call, so the app can take its ring off the screen.
   The app ends a ring it has shown but no INVITE has followed within
   twenty seconds of registering, in case a cancel is lost.

### 2.3 Pushing

A push is small and says enough for the phone's call screen, because iOS
requires the app to show the call before it can do anything else:

```json
{"type": "call", "call": "<FCP call id>", "from": "1002",
 "name": "Ann Lee", "tenant": "<tenant id>", "sent": 1791547200}
```

`type` is `call`, `cancel` or `test` (an administrator's, which the app
shows and ends at once). `[push] payload = "minimal"` leaves out
`from` and `name` (§6 Q3).

- **APNs.** HTTP/2 to `api.push.apple.com` (or the sandbox), token
  authentication: an ES256 JWT from the team's `.p8` key, made again every
  50 minutes. `apns-push-type: voip`, `apns-topic` the `pn-param`,
  priority 10, `apns-expiration: 0` (now or never), `apns-collapse-id` the
  call.
- **FCM.** The HTTP v1 API, `projects/<project>/messages:send`, with an
  OAuth 2.0 access token from the project's service account (an RS256 JWT
  exchanged at Google's token endpoint, kept until it expires). A data
  message, `android.priority: high`, a TTL of the ring time.
- **Credentials.** Platform configuration on the call manager:

  ```toml
  [push]
  max_expires = 604800

  [[push.apns]]
  team_id = "ABCDE12345"
  key_id = "XYZ987"
  key = "enc:v1:…"            # the .p8, sealed; or key_file, or ${FCP_PUSH_APNS_KEY}
  topics = ["com.thevoiceguy.anvil.voip"]
  sandbox = false

  [[push.fcm]]
  project_id = "anvil-12345"
  service_account = "enc:v1:…" # the JSON, sealed; or service_account_file
  ```

  These are the app publisher's keys (Anvil's), not a tenant's (§6 Q2).
- **Failures.** A push is sent once, with a five-second timeout; there is
  no time to retry. A token the provider says is gone (APNs 410
  `Unregistered` or 400 `BadDeviceToken`; FCM `UNREGISTERED` or
  `NOT_FOUND`) removes the push binding, and is told as
  `device.push_failed`. Every push is counted by provider and outcome, and
  each wait by outcome (registered in time, timed out, cancelled) with the
  time from push to REGISTER.
- **The sender** is a crate of its own, `fcp-push`, used by the call
  manager; an administrator can send a test push to a device
  (`POST /api/v1/devices/{id}/push-test`, through the node).

### 2.4 Clusters and sites

- **A flow's node.** Every registration records the node holding its flow
  (`flow_node`). A call on another node reaches it through that node,
  which sends the INVITE down the flow (§1.2's fallback is wrong for
  every TLS phone in a cluster, not only for apps). Without this, a woken
  app that re-registers on another node than the call's could not be
  reached.
- **Waits across nodes** need nothing new: the registration event reaches
  every node, and only the call's node has the wait.
- **Sites.** A site's phones register at the site, and its call manager
  rings them. A site does not hold the publisher's push keys: it asks the
  cloud to push, over its link (§6 Q4). A site cut off from the cloud
  cannot wake a sleeping app; its awake phones ring as before.

### 2.5 Anvil's side

- siphon-rs: Contact URI parameters that survive the REGISTER rewrite
  (a `contact_uri_params` builder option); Anvil enables outbound
  (`+sip.instance`, `reg-id`).
- The app registers its push address when it has one: the PushKit VoIP
  token on iOS, the FCM token on Android, with the topic or project it was
  built with; a 555 means "this server cannot push", and it registers
  without.
- **iOS:** PushKit and the `voip` background mode. Each VoIP push reports
  a call to CallKit at once (Apple's rule), from the payload; the app
  starts the phone without the UI, registers, and the INVITE that follows
  is the call already on the screen. A cancel push is reported and ended
  at once.
- **Android:** a `FirebaseMessagingService`. A call push adds the incoming
  call to Telecom and starts the call's service (U4b), then the phone,
  which registers.
- Starting the phone without the UI needs an entry point native code can
  call (today only Dart can). Phases A1–A3 below.
- Apple Developer and Firebase accounts, the app's APNs key and FCM
  service account, are the owner's to make; until then everything is
  proved against stand-ins.

## 3. How it is proved

- **Stand-ins.** `fcp_testkit::push` serves an APNs-shaped HTTP/2 endpoint
  and an FCM-shaped one, checking the JWT, the topic and the headers, and
  recording each push; either can answer "unregistered".
- **The scripted phone** (`fcp_testkit::Phone`) registers over TLS with a
  push address, drops its connection, and, told of a push by the stand-in,
  registers again and answers. Tests in `fcp-call-manager/tests/push.rs`:
  a call rings an app that was asleep; a desk phone and a sleeping app
  ring together and the desk phone answers, cancelling the app; the app
  never comes back and the call goes to voicemail; a dead token removes
  the binding; a 555 for a provider FCP has no keys for.
- **Two nodes** (`fcp_testkit::cluster`): the app re-registers on the other
  node and the call reaches it.
- **Anvil** against the stand-ins in its own tests, then on devices once
  the accounts exist.

## 4. Phases

### Phase P1: registering with a push address (fcp, siphon-rs)
- Contact header parameters kept for every binding (`+sip.instance`,
  `reg-id`, `expires` per Contact); bindings matched by instance.
- `pn-provider`, `pn-param`, `pn-prid` parsed and stored (a migration on
  `registrations`); 555 with Feature-Caps for a provider or topic FCP
  cannot push to; `Feature-Caps` in the 200 OK; `[push] max_expires`.
- `flow_node` recorded. The device views show the push address.
- siphon-rs: `Feature-Caps` and 555 as types, where they belong.

### Phase P2: senders (fcp)
- `fcp-push`: APNs (HTTP/2, token auth) and FCM (HTTP v1, service
  account), the `[push]` configuration with sealed keys, failures and
  metrics, `device.push_failed`, the test push.
- `fcp_testkit::push` stand-ins.

### Phase P3: ringing a sleeping app (fcp)
- Wakeable bindings at ring time, waits, the fork's new-legs arm, ringing
  for the caller, cancel pushes, the unreachable treatment when nothing
  can be woken; `fcp-call-manager/tests/push.rs`.

### Phase P4: clusters and sites (fcp)
- A call reaching a flow on another node; the cluster test; a site asking
  the cloud to push over its link.

### Phase A1: registering with a push address (anvil, siphon-rs)
- siphon-rs `contact_uri_params`; Anvil's outbound instance; the push
  address in the account; 555 handled; tested against FCP and its
  stand-ins.

### Phase A2: Android (anvil)
- FCM, the call push to Telecom and the call's service, the phone started
  without the UI, the cancel push; the emulator test against a stand-in.

### Phase A3: iOS (anvil)
- PushKit, the `voip` background mode, CallKit from the payload, the phone
  started without the UI; then TestFlight and a Play test track, with the
  owner's accounts.

## 5. What this does not do

- **Pushes for anything but a call.** No push for a message, a voicemail
  or a lamp: the app refreshes those when it opens.
- **Refresh pushes** (RFC 8599's `+sip.pnsreg`, asking a sleeping app to
  re-register before its binding expires): the long expiry of §2.1
  covers a phone used at least once a week (§6 Q1).
- **Web push** (`pn-provider=webpush`) for a browser softphone; FCP has no
  browser softphone (`docs/SOFTPHONE.md` §5).
- **Per-tenant apps.** One publisher's keys for every tenant; a tenant's
  own branded build with its own keys comes later (§6 Q2).

## 6. Questions

1. **How a push binding lives.** *Recommendation:* a long expiry for push
   bindings (`[push] max_expires`, seven days), no refresh pushes. iOS
   forbids a silent VoIP push (each must show a call), and ordinary
   background pushes are throttled and not guaranteed, so refresh pushes
   would work on Android only. An app unused for a week stops ringing
   until it is opened.
2. **Whose keys.** *Recommendation:* the platform's, in the call manager's
   configuration, keyed by topic and project: the keys belong to whoever
   publishes the app (Anvil's own Apple team and Firebase project). A
   tenant with its own branded build would bring its own keys, stored per
   tenant like the redirection accounts; later, with branded builds.
3. **What a push says.** *Recommendation:* the caller's name and number
   (CallKit must show the call before the app has done anything, so
   without them the screen says only "Anvil"), with
   `[push] payload = "minimal"` for an operator who keeps them off
   Apple's and Google's servers.
4. **Sites.** *Recommendation:* a site asks the cloud to push, over its
   link; the publisher's keys stay in the cloud. The alternative, the
   keys synced to every site, puts them on customers' machines.
5. **How long to wait.** *Recommendation:* the user's ring time, as for
   any other device, with other devices ringing meanwhile; no separate
   push timeout.
6. **A flow on another node** (§2.4). *Recommendation:* in this project
   (P4): a woken app needs it, and so does every TLS phone in a cluster.
7. **Cancelling on iOS.** *Recommendation:* a cancel is a VoIP push too;
   the app reports it to CallKit and ends it at once, which Apple allows,
   and ends any ring with no INVITE twenty seconds after registering.
8. **Where the push address comes from.** *Recommendation:* the REGISTER
   (RFC 8599), not an API call: the binding, its expiry and its node are
   the registrar's, and a desk phone or another softphone that speaks
   RFC 8599 gets the same.
