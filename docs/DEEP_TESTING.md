# Deep testing

*Drafted 2026-10-13, after the last open FCP gap (#303) was merged. Agreed 2026-10-13: all phases, every question on its recommendation. Q9, which real-world accounts and devices exist, is still open; it is needed only for D8.*

The features are built. This project looks for the bugs that the tests
written alongside each feature could not find, and fixes them.

Those tests prove what their authors meant: the scenario the feature was
designed for, driven by the same team's tools. This project goes after the
other kinds of bugs:
- features that are each right alone but wrong together;
- clients that are not our own;
- routes that nothing calls;
- failures in the middle of a call;
- hours of running, not seconds;
- the admin UI as a person uses it.

## 1. What exists

- **About 2,700 tests across 41 crates.** These include:
  - the end-to-end test floor (`docs/TEST_FLOOR.md`): a scripted SIP phone
    with media against the real binaries, on one or two nodes;
  - the Postgres-backed API suite, which checks every response it sees
    against the OpenAPI contract;
  - the SDKs' suites against a live admin.
- **Every pull request** runs fmt, clippy with every feature, the whole
  suite with Postgres and Redis service containers, the SDKs' generated
  code, the admin UI's type check, and the compose file.
- **Every night**, the fourteen images are built and the stack is started
  from them. A phone calls through it, then a backup and a restore run.
- **Every week**, the security audit runs and the submodules' fuzzing is
  checked.
- **Load:** `fcp-loadgen` measured what one node carries
  (`docs/CAPACITY_AND_SURVIVABILITY.md` §8). It is run by hand.

## 2. What the survey found

### 2.1 Our phone tests our server

The testkit's `Phone` is built on siphon-rs, the same SIP stack as every
FCP binary. A parser or dialog bug common to both sides passes every test.

No independent SIP implementation is ever pointed at FCP:
- not SIPp;
- not baresip or pjsua;
- not a hardware phone.

Interop is the class of bug a customer meets first. Every device fault so
far (Polycom's master file, Yealink's NOTIFY, Grandstream's names) was
found by reading vendor documents, not by a device.

### 2.2 Routes nothing calls

The OpenAPI contract has 412 operations. Searching every test, the SDKs'
examples and their suites by path finds about 65 that are never named.
That is an estimate, and §5's D1 makes it exact: measured, it is 87 (§8, D1). The estimate's groups:

| Group | Never called |
|---|---|
| Live video controls (pin, spotlight, content layout) and meetings by id | 12 |
| `/me/voicemail` messages and greetings, mailbox greetings | 10 |
| The admin API's live call control (`/calls/{id}/hold` … `hangup`); only the node's own routes are tested | 7 |
| Presence watchers, policy, pres-rules | 7 |
| Encryption rotate, re-encrypt, rewrap, seal | 4 |
| Phone number purchase, activate, release, available | 4 |
| Extensions by number, trunk test, clone, stats | 6 |
| Sign-up, expired-password change, recovery codes, token rotation under `/me` | 4 |
| Others: dashboard, usage by number, config audit and validate, queue stats, prompt synthesis, tenant deletion status, digest password, identity unlink, emergency notify-call | 11 |

A route that has never run can return a 500, reach another tenant, or
not do what it says.

### 2.3 Features tested one at a time

The enterprise calling phases (`docs/ENTERPRISE_CALLING.md`) each proved
their own feature. Their combinations were rarely tested. Examples:
- DND with an executive's screening;
- find-me reaching a hoteling guest;
- a shared line appearance with call waiting off;
- a forward crossing a class of service that refuses it;
- a schedule's after-hours target being a pushed app;
- a parked call whose parker is suspended.

There are eleven features, a ring plan executed by four paths, and the
B2BUA's held leg. Interactions are where they disagree.

### 2.4 Failures happen between tests

What is already proved:
- survivability of a site with its link cut (`fcp-link/tests/survivability.rs`);
- a drained node (`capacity.rs`);
- a Redis reconnect (multi-region phase 3).

What is not:
- Postgres going away, or failing over to a replica, in the middle of a
  call, a REGISTER storm or a CDR write;
- a node killed with `SIGKILL` rather than drained;
- a conference trunk dropped mid-mix;
- the events server or `fcp-jobs` dying with work in flight.

### 2.5 Nothing runs for long

The longest test runs for minutes. A leak of memory, file descriptors,
RTP ports, Redis keys, Postgres connections or rows in a table nobody
reaps shows up after hours, so we would not see one.

### 2.6 Tenant isolation is checked where someone thought of it

`route_permissions::ROUTES` enforces a permission on every route. Whether
tenant A's administrator can read or change tenant B's object by its id
is only tested route by route, where an author remembered to. Nothing
probes every route with another tenant's ids.

### 2.7 The admin UI only has to compile

`npx tsc -b` is the UI's only gate. No page has been opened by a test, so
a broken form, a wrong field name or a page that throws is invisible.

### 2.8 Hostile SIP reaches parsers, not binaries

siphon-rs and forge-media fuzz their parsers nightly. Nothing sends a
running SBC any of these:
- malformed or oversized messages;
- out-of-order transactions;
- a CANCEL racing a 200;
- a REGISTER or INVITE flood.

### 2.9 The nightly's own health

- **Security audit:** failed on 2026-10-12 on three `quinn-proto`
  advisories. The lock now has 0.11.18, which fixes them; the next weekly
  run should confirm.
- **Live stack:** failed three nights running (2026-10-07 to 10-09) with
  `fcp-admin` unhealthy at start, then passed from 10-10. The cause was
  never written down.

### 2.10 The real world

The following have only ever been tested against stand-ins:
- hardware phones;
- a real carrier trunk;
- Entra ID, Okta and Google as identity providers;
- APNs and FCM;
- browsers on the WebRTC gateway;
- Anvil on a phone.

The stand-ins are faithful to the documents, not to the services.

## 3. Goals and non-goals

**Goals:**
- Find the bugs in §2's areas and fix them.
- Leave behind tests that keep them found, running in CI or the nightly,
  so the next feature is tested the deep way by default.

**Non-goals:**
- New features.
- A coverage percentage as a target.
- Performance tuning, beyond fixing a leak or a collapse the soak finds.

## 4. How the session runs

- **Each phase** (§5) starts by writing its tests, then fixes what they
  find.
- **A bug found** gets a failing test first, then the fix, in the phase's
  PR when it is small. A bug too large for that becomes a GitHub issue
  labelled `deep-testing` (Q2), with the failing test marked `#[ignore]`
  and naming the issue.
- **This document keeps the findings**: a table per phase listing what
  was found, its severity, and the PR or issue that dealt with it.
- **Severity:**
  - *S1*: a call dropped or misrouted, data reaching another tenant, a
    crash, an emergency call affected;
  - *S2*: a feature not doing what it says;
  - *S3*: a wrong answer from an edge route, or a cosmetic fault.

## 5. Phases

**D0. The nightly green and watched.**
- Find the 10-07 to 10-09 stack failure from its logs, or show that it
  was fixed.
- Confirm the audit passes.
- Make a failed scheduled run visible, not something found by chance
  (Q8).

**D1. Measure.**
- Record every route the Postgres API suite and the SDK suites call, by
  instrumenting the test client rather than searching the text.
- Fail CI when a route in the contract is never called, with an explicit
  allowlist for the ones that need a provider (Q5).
- Produce a weekly line-coverage report with `cargo llvm-cov`, published
  as an artifact and not used as a gate. It points the later phases at
  the code no test reaches.

**D2. Clients that are not ours.**
- **SIPp scenarios** in CI against the call manager and the SBC:
  - registration with digest, wrong and stale nonces;
  - calls answered, rejected, cancelled, and CANCEL racing 200;
  - re-INVITE and UPDATE;
  - PRACK;
  - REFER, blind and attended;
  - session timers;
  - malformed and oversized messages;
  - out-of-dialog strays;
  - a REGISTER and INVITE flood against admission.
- **baresip** (Q4) as a second phone with real media for the scenarios
  that need audio:
  - codecs PCMU, PCMA, G722 and Opus, with transcoding between them;
  - DTMF RFC 2833 and INFO;
  - SRTP SDES;
  - TLS;
  - hold with music;
  - a conference room.

**D3. Every route, every tenant, every role.**
- A test for each route §2.2 lists, against its documented behaviour.
- A generated probe over `ROUTES`: each route called with tenant B's ids
  by tenant A's administrator. The expected answer is 404 or 403, never
  2xx.
- A role matrix: each route called with a token of each built-in role,
  checked against the permission table.
- Done in parts: D3a the generated probes (the tenant probe and the role
  matrix), D3b a test for each route §2.2 lists, in two PRs (D3b1 the
  stores' routes, D3b2 sign-in, calls, conferences, voicemail and the
  platform's).

**D4. Feature interactions.**
- A table-driven call suite in `fcp-call-manager/tests/interactions.rs`.
- Each row is a combination of a caller, a callee's settings (forwards,
  DND, find-me, call waiting, executive, shared line, hoteling, class of
  service, schedule, push app), and an event (busy, no answer, hold,
  transfer, park, suspend).
- Each row also states the expected outcome, written from
  `ENTERPRISE_CALLING.md` before it is run.
- Rows are chosen pairwise over the features, not exhaustively.

**D5. Faults.**
- With calls up and REGISTERs flowing, take each of these away and bring
  it back:
  - Postgres (stopped, and failed over to a replica);
  - Redis;
  - a call manager killed with `SIGKILL`;
  - a conference node holding half a room;
  - the events server;
  - `fcp-jobs`.
- After each, check:
  - which calls lived;
  - what callers heard;
  - that registrations, CDRs and events caught up;
  - that nothing needs a restart by hand.
- **Upgrade with data:** the previous release's images, a populated
  database, then the current images over it.

**D6. Soak.**
- `fcp-loadgen` against the compose stack for hours (Q6): steady calls,
  registrations, conferences and API traffic.
- A sampler records each process's RSS, open file descriptors, RTP ports
  in use, Redis keys, Postgres connections and the row counts of every
  table that should stay flat.
- A run fails on growth that does not level off.

**D7. The admin UI as a person uses it.**
- Playwright (Q7) against a live admin with a seeded tenant: sign in, then
  open every page in the navigation with no console errors.
- Run each page's create, edit and delete for the main objects (users,
  devices, numbers, trunks, flows, queues, schedules, prompts, webhooks,
  identity providers), checked against the API afterwards.
- In CI beside the type check.

**D8. The real world, by hand.**
- A checklist we work through together (Q9) with whatever exists:
  - hardware phones provisioned zero-touch;
  - a carrier trunk with inbound and outbound calls, emergency test
    calls (`933`) included;
  - Entra ID or Okta sign-in, SCIM and sign-out;
  - Anvil woken by a real push on iOS and Android;
  - Chrome, Firefox and Safari in a WebRTC room with video and screen
    share.
- Each finding becomes a test against a stand-in where one can be made.

## 6. Order

D0 and D1 come first: they are small, and D1 decides where D3 looks.

D2 to D7 are independent and can go in any order. The proposed order is
by expected yield:
1. D2 (interop);
2. D4 (interactions);
3. D3 (routes and tenants);
4. D5 (faults);
5. D6 (soak);
6. D7 (UI).

D8 runs whenever the accounts and devices are at hand.

## 7. Questions

**Q1. Scope.** All of D0–D8, or a subset first?
*Recommendation:* all, in §6's order, D8 as available. The automated
phases each leave a suite behind.

**Q2. Where findings live.**
*Recommendation:* the tables in this document, plus GitHub issues
labelled `deep-testing` for what is not fixed in the phase that found it.

**Q3. Fix as found, or find everything first?**
*Recommendation:* fix as found within each phase. An S1 is fixed at once
in its own PR whatever the phase.

**Q4. Which independent clients?**
*Recommendation:* SIPp for protocol scripts and hostile traffic, and
baresip for media. Both are packaged for Debian and Ubuntu runners.
pjsua is a fallback where baresip lacks a feature.

**Q5. Should an uncalled route fail CI?**
*Recommendation:* yes, with an allowlist that names why each entry is
there. Line coverage is reported, not gated.

**Q6. Where and how long does the soak run?**
*Recommendation:* weekly in Actions for four hours (the job limit is
six), plus one 24-hour run by hand on a small cloud VM before calling D6
done. The GPU box is not needed.

**Q7. UI tests: Playwright or nothing?**
*Recommendation:* Playwright, with Chromium only in CI.

**Q8. How a failed nightly reaches us.**
*Recommendation:* the scheduled workflows open, or comment on, a GitHub
issue when they fail and close it when they pass.

**Q9. What real-world things are available for D8?**
This is for you to say: which hardware phones, a carrier account with a
test number, an Entra ID or Okta tenant, the Apple and Firebase accounts
for push, and devices for Anvil.
*Answered 2026-10-18:* Poly phones, a Twilio Elastic SIP trunk with a
number, the Apple and Firebase accounts with iOS and Android devices, on
a GCE VM with a public address under `d8.feryx.io`. No identity tenant:
SSO and SCIM stay with their stand-ins.

**Q10. Anvil.** Does the session include Anvil's own client code, or only
FCP?
*Recommendation:* FCP's server side here, with Anvil only as a client in
D8. Anvil's own deep testing would be its own pass.

## 8. Findings

### D0. The nightly green and watched

| Finding | Severity | Dealt with |
|---|---|---|
| The weekly audit failed on three `quinn-proto` advisories (RUSTSEC-2026-0338, -0340, -0343; a remote peer can panic the QUIC stack) | S2 | Already fixed: the lock has 0.11.18. This PR's audit job confirms it. |
| The live stack failed 2026-10-07 to 10-09 because `fcp-admin` overflowed a runtime worker's stack at startup, and restarted until compose gave up. It passed again from 10-10 with no change aimed at it. The cause: the images were built with Rust 1.92, which CI never uses. utoipa's generated `openapi()` built by 1.92 in release with SAML needs about 2.1 MiB of stack (53 KiB under 1.98), and the admin built it inside a spawned task on a 2 MiB worker, so startup sat on the edge. | S1 (the admin does not start) | The spec is built on a thread of its own with a 64 MiB stack (`api_docs::spec_json`), tested from a 128 KiB caller. The sixteen Dockerfiles moved to `rust:1.98`, the toolchain CI tests. Only main moves the `nightly` tag, so the Images workflow can run on a branch. |
| A failed scheduled run told nobody | S2 | `scripts/scheduled-run-report.sh`, the last job of the CI and Images workflows, opens an issue labelled `nightly-failure`, comments on it while the run fails, and closes it when it passes. |

### D1. Measured

| Finding | Severity | Dealt with |
|---|---|---|
| The survey's ~65 uncalled operations were 87. Searching the tests for a path's text counts a list `GET` as called when a `POST` to the same path is, and a route as called when a test only names it in a refusal. Measured by the server, 325 of 412 operations are called. Uncalled operations include the list endpoints of rate plans, extensions, locations, line bindings, config templates and voicemail mailboxes. | S3 (a measure, not a bug) | `fcp_api_core::route_log` logs every request that reaches an `/api/v1` route when `FCP_ROUTE_LOG` names a file: in the Postgres suite's in-process router, and in every admin binary another suite starts. `scripts/check-routes-called.py` fails CI on an operation that is neither called nor in `scripts/routes-uncalled.txt`, and on one listed there that is called. All 87 are listed, each with its reason, for D3. |
| Five operations are reached only to be refused (401, 403): `PATCH /routing/rules/{id}`, `POST /me/totp/disable`, `POST /calls/{id}/whisper`, and `GET` and `DELETE /calls/{id}/supervisor`. Their handlers have never run. | S3 | Counted as uncalled and listed for D3. |
| Line coverage was never measured. | S3 | `.github/workflows/coverage.yml` runs the workspace's tests under `cargo llvm-cov` each Sunday. It publishes the HTML report and a per-crate table, the most uncovered first. It is a report, not a gate. |
| The first run (2026-10-13, dispatched on main): 135,477 of 187,529 lines, 72.2%. The least covered: `fcp-cli` 3.8% (the terminal UI), `fcp-jobs` 49.0%, `fcp-presence-server` 50.8%, `fcp-voicemail-server` 53.4%, `fcp-events` 54.4%, `fcp-call-manager` 55.5%, `fcp-conference-server` 57.9%. The most uncovered lines: `fcp-sip` (8,918), `fcp-cli` (7,792), `fcp-api-core` (7,191), `fcp-conference-server` (6,916). | — (a measure) | Points D2 and D4 at the call path. Its artifact put `coverage.json` under a `..` path that `gh run download` refuses; both files now live in `target/llvm-cov/`. |

### D2. Clients that are not ours: SIPp against the call manager

`crates/fcp-call-manager/tests/sipp.rs` runs SIPp (`fcp_testkit::sipp`) against the call manager with `auth_invites` on. Both ends of each call are SIPp, so no siphon-rs code is on the client side. The scenarios are XML in `tests/sipp/`, written by `tests/sipp/scenarios.py`. CI installs `sip-tester` and sets `FCP_SIPP_TESTS=required`. The tests cover:
- registration with digest, a forged nonce and a wrong password;
- calls answered, refused busy, cancelled while ringing, cancelled before they are set up, and cancelled as the callee answers;
- re-INVITE;
- requests for dialogs that do not exist;
- messages no parser should accept.

| Finding | Severity | Dealt with |
|---|---|---|
| A CANCEL that crossed the callee's 200 left the callee in an answered call with nobody on the other end. The node took its 200 and ACKed it, then sent it a CANCEL and never a BYE. There were three windows:<br>1. The CANCEL handler removed the leg's dialog and CANCELled a transaction that had its 2xx.<br>2. A CANCEL arriving after the forking task chose its winner, but before it stored it on the call, found no leg to end. The forking task's wake-up had already passed.<br>3. The helper that ends a leg read the dialog the moment its CANCEL was answered. The callee's 200 can come after the CANCEL's own 200, and was missed. | S2 | Fixed in three parts:<br>1. The CANCEL handler marks the call `caller_cancelled` and takes the forking task's wake-up and chosen leg in one `update_call`. The forking task stores its winner in an update that reads the mark, and hangs up the winner itself if the mark is set.<br>2. `cancel_or_bye_fork_leg_with` no longer CANCELs an answered leg. It waits for the INVITE's outcome (siphon-rs `CallHandle::invite_outcome`, new in sip-uac 0.8.0) up to Timer B, and BYEs a 2xx.<br>3. siphon-rs reports a 2xx only once it is ACKed, so the BYE never overtakes the ACK, and `cancel()` refuses an INVITE that has its final response.<br>The 0 ms answer is tested five times a run. Every round goes the way the bug did, and the callee is now hung up. |
| A caller who hung up as soon as the node had the INVITE was put through. The node sends 100 first, then authenticates and routes. A CANCEL in that time matched no call, was answered 200 and forgotten. The callee rang and could answer a caller who had gone. A CANCEL after the call was made but before forking ended the call while the INVITE handler went on to ring the callee. | S2 | Fixed: a CANCEL that finds no call is kept by its INVITE's Via branch (RFC 3261 §9.2) for up to 32 s, at most 10,000. The INVITE handler finds it right after creating the call and answers 487. The forking step stores its wake-up in the update that reads `caller_cancelled`, and rings no one if it is set. `a_cancel_before_the_call_is_set_up_rings_no_one`. |
| A CANCEL that arrived after the caller had been answered tore the call down. The node hung up the callee and ended the call, and then left the caller's BYE unanswered. RFC 3261 §9.2: a CANCEL for a request with its final response has no effect. | S2 | Fixed: the CANCEL handler leaves a call it finds established alone, decided in the same update. The forking task marks a call established before it relays the 200, so the caller has the answer or will get it, and hangs up with BYE. The unanswered BYE was seen once, in that torn-down call, and not again in 20 runs after the fix. |
| `registrar.auth_invites` is off by default, and the shipped compose configurations set it off explicitly. `EDGE_SECURITY.md` §2b, agreed as Q1, said: off for one release with a warning, then on. The warning has said "the next release" since v0.25.224. With it off, an INVITE from loopback or from a registered device's address is put through without proving who placed it. | S2 | Fixed in v0.25.397: on by default and in the shipped compose configurations; the single-node file trusts loopback for its siblings; a node that turns it off is warned at every start; the upgrade note is `BACKUP_AND_RESTORE.md` §5.4.1. |
| Turning `auth_invites` on broke queues and a flow's Dial. They ring by INVITing the node's own SIP address, with no user's password, so they were challenged and failed. Before, the loopback bypass let them through. In compose the node's own address is the host's, not loopback, so queues would have stopped working there. | S2 (hidden by the off default) | Fixed: `fcp_sip::own_leg`. A random per-process token in `X-FCP-Own-Leg` is added by `SipUserAgentClient` only to INVITEs addressed to the node itself. An INVITE carrying it is admitted as the node's own leg; anything else carrying the header has it stripped at ingress (`claims`). The address alone proves nothing, since a UDP source can be forged. |
| The test phone counted a call as answered only after the INVITE's provisional stream closed. After an authenticated retry, the challenged first attempt holds that stream open until Timer D, so the phone waited a full second. A far end that answered and hung up within that second looked like a call that ended unanswered. | S3 (tests only) | Fixed: `Phone` waits for `CallHandle::invite_outcome`, which reports a 2xx once it is ACKed. |
| `[security] max_invite_cps_per_source` counts a challenged call twice: the challenged attempt and the retry. | — (by design) | Kept and documented on the setting. Not counting INVITEs that carry credentials would mean checking them first, a database read for every packet of a flood. |
| A BYE for a dialog that does not exist was answered 200. In-dialog requests for no dialog (INFO, UPDATE, PRACK and the rest) were answered 500, from the error a missing dialog raised. RFC 3261 §12.2.2: 481. | S3 | Fixed: `handle_bye` answers 481 when neither a dialog nor a call matches. `lookup_call_for_in_dialog` raises `NoSuchDialog`, which the request loop answers 481. `requests_for_no_dialog_are_refused`. |
| The digest's `uri` is never compared with the Request-URI (RFC 3261 §22.4, RFC 7616 §3.4.6). SIPp signs `sip:127.0.0.1:<port>` for a `REGISTER sip:e2e.local` and is accepted. The nonce's replay check limits it to a man in the middle. | S3 | #307: an exact match would refuse real clients, SIPp among them. |
| A CANCEL for no INVITE at all is answered 200, not 481 (RFC 3261 §9.2 SHOULD); it is kept like an early one. | S3 | Left: telling it from an INVITE still being set up needs the INVITE's server transaction looked up by branch. |
| Nine kinds of broken message, sent at the node, all left it answering: no SIP at all, a bad Request-URI without a Via, a header without a colon, a Content-Length too long and negative, seventy Vias, a 30 KB header, a stray response, a CSeq that is not a number. | — (passed) | `messages_no_parser_should_accept_are_survived`. |

### D2b. Requests inside a call: UPDATE, PRACK, REFER, session timers

The same suite, `fcp-call-manager/tests/sipp.rs`, with SIPp on both ends:
- UPDATE with a new offer in an answered call (RFC 3311);
- a callee that rings reliably (RFC 3262), to callers with and without
  100rel;
- a blind transfer by the caller (REFER, RFC 3515), carried out by the
  node, with a third SIPp as the target;
- session timers (RFC 4028) under `[b2bua.session_timer] enabled`: an
  interval under Min-SE, a session the node refreshes, and one the caller
  was to refresh and never does.

| Finding | Severity | Dealt with |
|---|---|---|
| Session expiry hung up one leg. When a session expired, the node sent a BYE on the leg whose timer ran out and ended the call record, but never hung up the other party. That party stayed in a call with nobody, and the call's media session and recording were never closed. | S2 | Fixed: an expired session ends the call through the hangup every other path uses (`hangup_call_by_id_saying`): BYE on both legs with `Reason: SIP;cause=408`, then the media closed. The session timer listener reaches the handler through `serving`, a copy the first request sets once the builders are done. `a_session_nobody_refreshes_is_ended`. |
| A callee that needed 100rel could not be called. The node's INVITE never said `Supported: 100rel`, so a compliant callee never rang reliably, and one that requires it refuses the call (421). Had a callee rung reliably anyway, the node copied its `RSeq` and `Require: 100rel` to a caller that never offered 100rel. It also forwarded a caller's PRACK, which PRACKs the callee a second time, since siphon-rs's INVITE transaction already does. The callee's 481 to that second PRACK tore the dialog down. | S2 | Fixed, one leg at a time: siphon-rs's `IntegratedUAC` adds `100rel` to every INVITE's Supported, since its transaction PRACKs every reliable provisional. The node relays a callee's ringing to the caller as an ordinary provisional (no RSeq, no `Require: 100rel`), and answers a PRACK 481, since it sent nothing reliably. `the_node_pracks_a_callee_that_rings_reliably`. |
| A caller that *requires* 100rel still gets ordinary provisionals. RFC 3262 obliges the node to send them reliably, and neither siphon-rs's server transaction nor the node can. | S3 | #310. |
| An INVITE asking for a session shorter than the Min-SE was put through, not refused 422. siphon-rs's `validate_session_timer` read the header with the typed parser, which refuses an interval under 90 s and returns nothing. "Nothing" was taken for "no header", so the one case 422 exists for was never refused. | S3 | Fixed in siphon-rs: the interval is read as a number before any range check. `a_session_shorter_than_min_se_is_refused_422`. |
| Every session timer was answered `1800;refresher=uas`, whatever the caller asked. RFC 4028 §9: the answer keeps the request's interval (it may shorten it, never below Min-SE) and its refresher. A caller that asked to refresh every 90 s was told the node would, every 900 s. | S3 | Fixed: `session_timer_answer`. The caller's interval is kept within the node's own and either side's Min-SE, and the caller's refresher is kept when it names one. `a_session_the_node_refreshes_is_refreshed` checks the answer, and that the node refreshes within it. |
| The side that does not refresh hung up at the moment the session expired. RFC 4028 §10 recommends the BYE slightly before: the minimum of 32 s and a third of the interval earlier. | S4 | Fixed in siphon-rs (`non_refresher_expiry`). The test wants the BYE within 75 s of a 90 s session, and it comes at 60. |
| UPDATE is relayed and answered, and a blind transfer is carried out: the referrer gets 202, a NOTIFY with 200 and a BYE; the target is dialled back through the node (with its own-leg token) and the transferred party moved to it. | — (passed) | `sipp_updates_an_answered_call`, `sipp_transfers_a_call_blind`. |

### D2c. The SBC, an attended transfer, floods

`crates/fcp-sbc/tests/sipp.rs` puts the SBC between two SIPp trunks: a
carrier at `127.0.0.2` (a second one at `127.0.0.4`) and a SIPp callee
standing in for the core at `127.0.0.3`. The SBC runs without a database,
with its trunks from configuration. The carrier's scenarios are the call
manager's callers without the challenge (`trunk()` in `scenarios.py`).
The tests cover:
- calls answered, refused busy, cancelled while ringing, re-INVITEd and
  UPDATEd;
- requests for dialogs that do not exist;
- messages no parser should accept;
- an INVITE flood from one carrier address, with the request limits as
  shipped.

The call manager's suite gained:
- an attended transfer (REFER with Replaces). One SIPp caller holds both
  calls (`-m 2`), and the second call hands its dialog to the first
  through SIPp globals;
- a REGISTER flood and an INVITE flood from `127.0.0.1` at 200 a second,
  while phones at `127.0.0.2` and `127.0.0.3` register and call
  (`Sipp::start_at`).

| Finding | Severity | Dealt with |
|---|---|---|
| siphon-rs's transport drops everything over 200 packets a second from one IP address (UDP, and frames on TCP, TLS and WebSocket), before the message is parsed. No FCP binary configures that limit, and `[e911] trusted_edges` does not exempt anyone from it. Measured: 1,200 OPTIONS at 400 a second from a trusted edge, 581 answered. The SBC in front of a call manager, a call manager in front of a conference server, and a carrier trunk all hit it at about 30 calls a second, with nothing in FCP's own counters. | S2 | Fixed in v0.25.401 (siphon-rs `set_ingress_rate_exemption`, sip-transport 0.6.1). Every binary sets the rate from `[sip] max_messages_per_source` (`[core.security]` on the SBC; default 200, `0` for none). Exempt: `trusted_sources`, trusted edges, trunk sources, bound sites and live cluster members. The shipped single-node and compose files trust loopback. The tests are `trusted_sources_are_not_limited_per_source` (1,200 of 1,200 answered, against 584 from an address not listed), `the_per_source_limit_is_configurable`, and the SBC's `a_trunk_source_is_not_limited_per_source`. |
| Inbound packets are each handled on a task of their own (`fcp-sip/src/listener.rs`), so two messages of one dialog that arrive back to back can be handled in either order, over TCP too. Seen: a callee's 180 and 200, sent together, handled 200 first, and the 180 dropped. Any pair could be swapped: INFO digits, an ACK and a BYE, a NOTIFY and the next one. | S3 | #313. The CANCEL race's caller now accepts a 200 with no 180 before it, which UDP allows anyway. |
| An INVITE carrying a To tag that names no dialog was taken as a new call. At the SBC it rang the core, and the 180 sent back to the carrier carried two To tags (`tag=no-such-dialog;tag=…`). The call manager challenged it instead, but would have done the same for a caller with credentials. | S3 | Fixed: the shared handler answers 481 (RFC 3261 §12.2.2, the node not recreating the dialog). `strays` requires the 481 at both binaries. |
| `[admission] max_register_cps_per_source` defaults to unlimited on the call manager, while `[security] max_invite_cps_per_source` defaults to 20. `EDGE_SECURITY.md` says REGISTER floods are bounded; they are only when an operator sets the key. With it set to 10, a flood of 400 REGISTERs in 2 s had 370 refused 503, and a phone at another address registered in the middle of it. | S3 | #314: the default is a decision about admission's "unset is unlimited" rule (`CAPACITY_AND_SURVIVABILITY.md`). |
| Every refusal at the edge is stateful. An INVITE refused 503 by `max_invite_cps_per_source`, or challenged 401, holds a server transaction, and retransmits its answer about 11 times over 32 s unless it is ACKed. A flood from a spoofed address, which never ACKs, is answered roughly ten times over. | S4 | #315: a refusal at the edge could be sent statelessly. |
| The CANCEL race's caller sometimes ACKed its answer to an empty Request-URI. A CANCEL shares its INVITE's branch, so SIPp cannot tell the CANCEL's 200 from the INVITE's by transaction. When the INVITE's 200 came first, the CANCEL's 200 landed on the receive that records the route set, and blanked it. | — (tests only) | The scenario keeps each 200's Contact and To and picks the INVITE's by its CSeq method. |
| A caller who hung up while joining a conference stayed in the room. The conference node answers at step 9 of its INVITE handling and stores the participant's session at step 19, after the welcome message. A BYE in between found no session, so the shared handler answered it 481, and the join then completed. This made the drain test fail on CI (#318), and a caller hanging up the moment they were answered reproduced it every time. | S2 | Fixed: the call is `joining` from its 200 until its session is stored; a BYE then is answered 200 and the session removed once it exists. `a_caller_who_hangs_up_on_the_answer_leaves_nobody_behind`. |
| rustls 0.23.45 had four advisories, published 2026-10-14 (RUSTSEC-2026-0346 to -0349), which failed every PR's audit. | S2 | rustls 0.23.46. |
| The SIPp tests' `no_calls_up` read a `count` field that the call manager's `/calls` does not have, so it passed whatever was up. Two assertions in `calls.rs` read the same field. | S3 (tests only) | Fixed: all three count the `calls` array. Every SIPp test passes with the real check. |
| Attended transfer: the referrer gets 202, a NOTIFY and a BYE on both calls. The node dials the target back with the Replaces, which takes the referrer's leg of the second call. The transferred party and the target are left on calls that do not include the referrer, until the target hangs up and both are hung up. | — (passed) | `sipp_transfers_a_call_attended`. |
| Floods: the call manager refused 370 of 400 REGISTERs (limit 10) and most of 400 INVITEs (limit 20), while a call from another address went through. The SBC refused the flood 429 while the other carrier's call reached the core. Nothing the floods sent was left up. | — (passed) | `a_flood_from_one_source_costs_that_source`, `a_flood_from_one_carrier_costs_that_carrier`. |
| The SBC carries calls answered, refused, cancelled, re-INVITEd and UPDATEd between two SIPp trunks. The core sees a dialog of the SBC's own, with nothing of the carrier's Call-ID or tags, and the SBC survives the nine broken messages. | — (passed) | `crates/fcp-sbc/tests/sipp.rs`. |

### D2d. baresip: real media from a phone that is not ours

`fcp_testkit::baresip` runs baresip, a softphone that shares no code with
FCP, and drives it over its `ctrl_tcp` module:
- its microphone is a WAV file of a tone;
- what it hears is the `sndfile` filter's dump of what it decoded, with
  ALSA's `null` device as the player, since baresip decodes only as a
  player pulls and Ubuntu's 1.0 has no file player. The test reads the dump
  at 8 kHz mono and listens with the testkit's tone detector;
- its media stays on loopback;
- codecs, SRTP, DTMF mode, TLS and auto-answer are set per account.

`crates/fcp-call-manager/tests/baresip.rs` puts the call manager between
two baresip phones. In each call both hear the other's tone and neither
hears its own. The tests cover:
- PCMU, and G.722 at both ends;
- Opus at one end and PCMA at the other, transcoded;
- DTMF as RFC 4733 events and as SIP INFO;
- both phones registered over TLS with SDES SRTP required;
- hold with the node's music, then resume.

`rooms.rs` puts two baresip phones in a conference room. CI installs
`baresip-core` and sets `FCP_BARESIP_TESTS=required`.

| Finding | Severity | Dealt with |
|---|---|---|
| Apart from the two fixed below, two baresip phones hear each other through the node in every case listed above. With PCMU, G.722, RFC 4733 and INFO digits, SDES SRTP over TLS, hold music and resume, and a room, each phone hears exactly the other's tone. | — (passed) | `crates/fcp-call-manager/tests/baresip.rs`, `two_baresip_phones_in_a_room_hear_each_other`. |
| An Opus-only softphone could not call a G.711-only phone: the node answered 488. With no trunk codec list, the node offered the callee only the caller's preferred codec, plus whichever of PCMU and PCMA the caller had also offered. A callee with none of those refused, though the node can transcode Opus to G.711. The offer is built by `filter_sdp_codecs`, which only keeps codecs. So the trunk branch's "no overlap: offer the trunk's codecs and transcode" filtered the caller's offer down to codecs it never had, leaving no audio codec. | S2 | Fixed: `fcp_media::sdp::ensure_audio_codecs` adds the chosen codecs a caller lacks (G.711, G.722 and G.729 at their static payload types, Opus at a free dynamic one) before the filter. An anchored call whose caller's codec is transcodable is offered PCMU and PCMA as well. `opus_and_pcma_are_transcoded`. |
| A transcoder built for a codec pair the legs no longer have was kept. The media session starts by assuming PCMU; with an Opus caller it armed Opus↔µ-law transcoders at once. The callee's answer then said PCMA, and "transcoding armed" was logged, but `ensure_transcoder_*` build only where there is none. The callee's A-law went on being decoded as µ-law, and the caller heard nothing of them. | S2 | Fixed in `sync_transcoding_state`: a transcoder whose pair or payload-type map differs from the legs' is dropped before arming. Without the fix, `opus_and_pcma_are_transcoded` fails every time ("Alice does not hear Bob"). |
| A phone that requires SRTP but registered over UDP gets a call it cannot accept: the node offers SRTP only to devices whose signalling is encrypted, so the phone answers 488. | — (by design) | SDES puts the key in the SDP, which over UDP anyone on the path reads (`docs/SOFTPHONE.md` A5). The test registers both phones over TLS. |
| baresip 1.1 resumes a held call and then answers the `resume` command with "Invalid argument". | — (not ours) | The test judges resume by what the held party hears. |


### D3a. Every tenant, every role: the generated probes

`crates/fcp-api-core/tests/api_postgres/route_matrix.rs` probes every route
in `route_permissions::ROUTES`. A route added to the table is probed with
no new test. Both probes collect every disagreement and fail once with the
whole list.

**Another tenant's ids.** `no_route_reaches_another_tenants_objects`
seeds tenant B with one of everything that has an id in a path: user,
group, site, network, device, trunk, number, line, extension, mailbox,
message, flow, prompt, template, class of service, schedule, LCR route,
routing rule, queue, rate plan and rate, hybrid site, meeting, hold,
identity provider, subject request, webhook, firmware target, brand asset,
pending phone, hoteling session, API token, CDR, SIP trace, recording,
emergency call and notification, calling settings and a presence policy.
Then tenant A's administrator calls every route naming an object with B's
ids. The rules:
- nothing answers 2xx or 5xx (503, "not configured here", is allowed);
- a list asked for B with the platform's tenant header shows none of B's
  ids or names;
- B's objects read the same afterwards, compared as B sees them;
- the live conference routes reach the conference server naming tenant
  A, whatever the request says.

Three kinds of answer are allowed 2xx, each proved:
- a sub-list filtered by tenant (a call's emergency notifications, its SIP
  messages) must be empty, and B has rows there;
- a presence policy is kept per tenant by AOR, which no tenant owns, so
  A's write lands in A's namespace, and B's view is compared;
- a route's echo of what the request itself carried is not a leak.

A route whose parameter tenant B has no value for fails the test, and the
routes left out are named with the reason (`/me`, redirection services
and template starters by name, the runtime queues). D3b1 put the services'
configuration back in: it is kept per tenant too.

**Every role.** `every_role_reaches_what_the_table_gives_it_*` runs once
per deployment mode. A user of each of the seven built-in roles calls
every gated route with ids that name nothing:
- a role the table refuses must get 403;
- a role the table admits must get anything but 401 or 403, unless
  `handler_refuses` names the handler's own further check;
- nothing may answer 5xx.

That is 2,352 checks per mode. The probes' requests carry
`route_log::PROBE_HEADER`, so they count toward no route's coverage: a
route is called by a test of what it does (D3b).

| Finding | Severity | Dealt with |
|---|---|---|
| No route gives tenant A's administrator anything of tenant B's. Across 269 probes with B's ids or B's tenant header, every answer was a refusal, a 404, an empty filtered list or A's own namespace, and B's objects were unchanged. | — (passed) | `no_route_reaches_another_tenants_objects`. |
| The end-user role held `user_read`, `device_read`, `phone_number_read` and `call_view` ("only their own"), but every admin route refuses an end user (`require_admin`), and their own objects come through `/me`. The grants could never be used. They told the CLI's menus and anything reading the model that an end user could list the tenant's users, devices, numbers and calls. | S3 | Fixed: the end user holds none of them, which matches `docs/API_TOKENS.md` ("None; an end user reaches `/me`"). |
| At a service provider the telecom admin held `tenant_create`, `tenant_update` and `tenant_delete`, though the role is tenant-scoped and the tenant handlers refuse anyone below the platform administrator. | S3 | Fixed: it keeps `tenant_read` there, as `read_only` does. |
| `GET /config…` with no config store, and `POST /prompts/{id}/synthesize` with no TTS service, answered 500, where an unconfigured dependency answers 503 elsewhere (`/calls/…` without a call manager). | S3 | Fixed: both answer 503. |
| `GET /voicemail/messages/{id}` returns a plain UUID as its id. This is correct, since a message id has no prefixed type, but the shape check did not know it, because no test had called the route. | — (by design) | Listed with the unprefixed resources in `check_shape` and `docs/API_TOKENS.md`. |
| Two handlers ask more than the table: `GET /tenants/{id}/deletion` (table: `tenant_read`) and `POST /users/{username}/purge` before the grace period (table: `user_delete`) refuse anyone below the platform administrator. | — (by design) | Named in `handler_refuses`. |
| The role probe sends `{}` as a write's body, so a write whose body fails to parse answers 422 before a handler's own further check runs. The probe proves the table, not the handlers' extra checks. | — (limit) | D3b's per-route tests. |

### D3b1. The stores' routes, one by one

D3b1 tests 38 of the 87 routes D1 found uncalled, against what each says
it does: numbers, extensions, lines and sites
(`tests/api_postgres/numbers.rs`); routing rules, least-cost routes,
trunks, rate plans and classes of service (`routing.rs`); queues, groups
and presence (`queues_presence.rs`); the services' configuration and phone
templates (`config.rs`). Each test also checks what the route must refuse,
another tenant's call included. The suite's app now has the config store,
so `/config` answers rather than 503. `scripts/routes-uncalled.txt` holds
the 49 left for D3b2.

D3a's probe puts another tenant's id in the path. Five of the cross-tenant
bugs below were out of its reach:
- an id in a request body;
- a record found by a name the tenant chose;
- a key the manager scopes by address alone;
- a tenant named in the query.

| Finding | Severity | Dealt with |
|---|---|---|
| Any administrator of any tenant could read the platform's service configuration. `GET /config…` took `system_view_logs`, which tenant admins, telecom admins and read-only users hold, and the nodes seed that configuration with their own credentials: the media server's API tokens (`media/auth`), the conference server's AI key (`conference/ai`), the events server's Redis URL. `?tenant_id=` also read another tenant's entries, for any session that was not an end user. | S1 | Fixed: reading `/config` takes `system_config`, as writing does, which only a super admin holds. A session that does not see every tenant reads its own tenant's entries, whatever tenant it names. `docs/API_TOKENS.md` says so. |
| An extension's target was not checked to be the tenant's. Tenant A's extension could dial B's user, device or number by id, and the call manager rings whatever the id names, in any tenant. Its site was not checked either. | S1 | Fixed: create and update refuse a target or site outside the tenant (400). |
| A line's `flow_id` was not checked. The call manager loads a flow by id alone, so A's IVR number could run B's flow. | S1 | Fixed: a line takes only a flow of its own tenant's, and an empty value clears it. |
| A trunk's stats counted every call record carrying its name, in any tenant. Trunk names are the tenant's own choice, so two tenants with a trunk called `carrier` saw each other's volume, ASR and ACD. | S1 | Fixed: `TrunkStore::get_trunk_stats` takes the tenant, and the SQL filters on it. |
| A number given back with `/release` kept what its tenant set on it: the emergency address and callback, caller name, site, hybrid site, allow and block lists, tags, recording. The next tenant to buy it from the pool got all of it, A's site id included. A number still bound to a line could also be released, and its line kept routing the next owner's calls to A's people. | S1 | Fixed: release refuses a number with a line binding (409) and clears the tenant's fields, keeping the carrier's. |
| `DELETE /presence/{aor}/watchers/{id}` removed the watcher from any tenant watching that address: the handler used the manager's unscoped removal. It also removed it twice over, so the second, eventful removal found nothing: no `presence.watcher_removed` went out and the watcher gauge was not updated. | S2 | Fixed: one removal, scoped to the tenant, which counts and announces. The comment promising a `terminated` NOTIFY is gone, since none is sent. |
| A trunk's average call duration and post-dial delay were always 0. Postgres's `AVG` of an integer column is `numeric`, and the read as `f64` failed into an `unwrap_or(0.0)`. | S2 | Fixed: the averages are cast to `float8`, and a failed read is an error, not 0. |
| A site could be deleted while users, devices, extensions, numbers or emergency networks named it. Each was left pointing at nothing, and an emergency call from there lost the address the tenant had given. | S2 | Fixed: `DELETE /locations/{id}` answers 409, naming what still uses the site. |
| A phone template a device named could be deleted. The device's next fetch was given no configuration: a named template that is gone is not replaced by one picked by vendor. | S2 | Fixed: 409 while a device names it. |
| A queue's strategy was any string. The node reads one it does not know as round robin, so a typo, or `ring_all` (which the suite used), changed how calls were handed out without a word. `PATCH /queue-configs/{id}` also took an empty name or extension, which create refuses. | S3 | Fixed: create and update share one check: a name, an extension, and one of the seven strategies the node and the UI know. |
| `POST /config/{service}/{section}/validate` checks only that the value is a non-empty object. It does not know a section's fields, since those are defined in each service's crate. | S4 | Left as it is: the test pins what it does. A schema per section is a feature of its own. |
| `/phone-numbers/available` and `/purchase` are the platform's pool, but only `/release` puts a number in it: no route adds one, and nothing documents the pool. | S4 | Tested as built. Noted for the number-provider work. |

### D3b2. Sign-in, calls, conferences, voicemail and the platform

D3b2 tests the other 48 of D1's uncalled routes and removes one: sign-in
and the caller's own account (`tests/api_postgres/sign_in.rs`); calls
through the admin API, against an in-process call manager
(`calls.rs`); meetings, the caller's own and live video
(`conferences.rs`); voicemail and prompts, with a TTS stand-in on loopback
(`voicemail.rs`); the dashboard, usage by number, notification calls,
redirection accounts and encryption at rest (`platform.rs`). Where a route
is the API's front for a node, the node's side is proved against the
binary: the call view and recording in `fcp-call-manager/tests/calls.rs`,
the notification call end to end in `emergency.rs`, message waiting in
`mwi.rs`, the admin's forwarding in `fcp-admin/tests/call_control.rs`.
`scripts/routes-uncalled.txt` is empty: every operation in the contract is
called by a test.

Four of the findings are not in the route a test called but in what it
fronts: the call routes, the notification call and message waiting were
never connected to the node that does the work, and nothing had followed
a request that far.

| Finding | Severity | Dealt with |
|---|---|---|
| Anyone who could make or change users could give any role. A tenant administrator could make a `super_admin` — a platform administrator over every tenant — and a user administrator could set a tenant administrator's password, or SIP password, and sign in as them, or promote themselves. | S1 | Fixed: a role is given, and a user holding one is acted on (role, password, SIP password, second factor, identity, unlock, delete, restore), only by someone whose roles hold everything that role does, and `super_admin` only by the platform (`fcp_core::permissions::roles_may_assign`, `docs/USERS.md` §8). A group's role is held to the same rule. |
| The E911 call channel never placed a call. The events server posted to `/api/v1/emergency/notify-call` on the call manager, which has no such route; the admin API's route tried to place the call itself, and the admin has no SIP client. An extension was looked up at the node's own address, not in the tenant. `fcp-notify`'s test spoke only to a stand-in. | S1 | Fixed: the call manager's `POST /emergency/notify-call` rings one of the tenant's users or extensions at a registered phone and speaks the notice; `fcp-notify` posts there; the admin's route puts the call to a call manager. Proved end to end: a test call makes the events server ring the contact's phone, which hears the notice, and the attempt is recorded as delivered. |
| Message waiting never followed a new message. The call manager tells subscribed phones on `voicemail.mwi_updated`, which nothing published: the voicemail flow publishes `message_recorded`, and the API published nothing when a message was heard, saved or deleted. `mwi.rs` passed because it published the event itself. | S2 | Fixed: the call manager recounts on any voicemail event naming a mailbox (left, played, deleted, or `mwi_updated`), the recording event names the mailbox by id, and the API announces the counts after a status change or a delete. |
| Voicemail PINs were kept in plain text and compared with `==`, though the migration's own comment says "hashed PIN". Nothing checked them: `PUT /me/voicemail` took any string. | S2 | Fixed: four to eight digits, kept as a bcrypt hash, as a user's PIN is; a mailbox written before keeps working with its plain PIN, compared in constant time, until it is next set. |
| `GET` and `DELETE /calls/{id}` and the recording routes looked only in the admin process, which holds no calls: every one answered 404 in a real deployment. The call times were Rust debug output, not RFC 3339. | S2 | Fixed: put to the call manager holding the call, as live call control is; the node gained `GET /calls/{id}`, and its recording routes are scoped to the tenant named. |
| `POST /calls` (click-to-call) could never place a call from the admin, and on a node it dialed whatever SIP address it was given — outside trunks, class of service and billing — with both legs offered one fixed RTP port and nothing bridging them. | S2 | Removed, with its code; nothing used it. A routed click-to-call is a feature of its own. |
| Changing a spoken prompt's text with no TTS service, or when synthesis failed, dropped its audio and answered 200: calls then played silence. | S2 | Fixed: the new words are spoken before anything is saved; otherwise 503 and the prompt is as it was. |
| A temporary greeting's lifetime overflowed: `expires_in_hours` was multiplied and added unchecked, and a large one panicked the handler. A missing TTS service was a 500. | S3 | Fixed: one to 8,760 hours, greeting text at most 2,000 characters; 503 without TTS or when it fails. |
| A meeting's `owner_id` was any string. A meeting could not have a PIN cleared: `null` in a PATCH meant "unchanged". An owner's PATCH dropped fields it may not change without a word. | S3 | Fixed: the owner is one of the tenant's users (by username or id, kept as the username `/me/conferences` matches); `null` clears; an owner's unknown field is refused. |
| The dashboard swallowed every storage error into a zero, counted calls only in the admin process (none), and counted today's calls over a capped list. | S3 | Fixed: errors are errors, active calls come from where the call list finds them, and today's calls are counted in SQL. |
| Usage by number kept the hundred busiest numbers, then paged over them. | S3 | Fixed: every number, paged. |
| Public sign-up did not check the username as every other way of making a user does: a CRLF or a `/` went in. | S3 | Fixed: the same check. |
| A content layout was passed to the conference server unchecked. Leaving supervision answered 200 where the contract said 204. Another user's voicemail message was 403, telling it existed. A SIP password was removed under `user_delete`, not `user_set_password`. | S4 | Fixed each. |
| Under password expiry, a user an administrator made has never reset their password, so their first sign-in asks for a change. | — | As designed; pinned by the test. |

### D4. Feature interactions

`fcp-call-manager/tests/interactions.rs` is a table of 37 rows. Each row
gives a callee's settings, an event and the outcome: the phones that ring
first, how the caller's call ends, and the phones that never ring. Each
row runs on a call manager and a database of its own, all with the same
cast: Alice calls Bob, and Carol, Dave and Erin are the others. Where a
row needs them, it also has a hoteling host, two desks on Bob's shared
line, a sleeping app woken through the APNs stand-in, and a voicemail
server.

The settings are DND, call waiting, the four forwards, find-me, an
executive with an assistant, a shared line, hoteling, a class of service,
a user's after-hours schedule, a closed number and a push app. The events
are a plain call, busy, no answer, already on a call, hold, park, a blind
transfer, and a suspension after a park. The paths are an extension, a
number, a shared line and a flow's `Dial`.

The rows cover every pair §2.3 named. Each outcome was written from
`ENTERPRISE_CALLING.md` before the row was run, and each row cites the
section it comes from. Where the design is silent, the row says what was
chosen: two users forwarding to each other end in 482.

The harness was checked against itself: five rows given a wrong outcome
must fail. One did not, because it listed the answering phone among
those that never ring, so the runner now refuses a row that does that.

| Finding | Severity | Dealt with |
|---|---|---|
| A flow's `Dial` to a user ran a copy of the ring plan of its own. It rang the first registered phone only, with no call waiting, find-me, executive screening or push. A forward's target was dialed back into the node without the node's own-leg token, so since `auth_invites` became the default (D2a) every such forward was challenged and failed. Under several tenants it would also have landed in the default tenant. | S2 | Fixed: the leg goes back into the node at the user's extension, carrying the own-leg token, the flow's call (its tenant) and `X-FCP-Flow-Dial`. The node runs the whole plan, and its voicemail treatment answers 480, so the flow keeps its `busy` and `no_answer` branches (`ENTERPRISE_CALLING.md` §3.1.1). |
| Two users forwarding to each other put calls into the node without end. Each hop was a new call with its own media session: 19 calls in 0.1 s, stopped only when the node's own rate limit (20 calls a second from its address) refused its own leg with 503. Forwards on no answer did the same, one ring time at a time. | S2 | Fixed: a call whose forwards reach a user they already reached is answered 482 Loop Detected, as is a chain of more than ten forwards. |
| A forward to a user was cut off at the forwarding user's ring time, which had run out by then. The forward's leg kept the old timeout, so Carol rang only as long as Bob had. When the two times matched, her own voicemail or forward never applied. | S2 | Fixed: a forward placed into the node waits for the new call's own plan, with a ten-minute backstop. |

### D5a. Faults at the call manager

`fcp-call-manager/tests/faults.rs` takes a call manager's stores away
while Alice talks to Bob, with Carol and Dave registered, and brings them
back. The node reaches Postgres and Redis through
`fcp_testkit::cut::CutLink`, which fails a link two ways:
- **refused**: every connection is reset and new ones are refused, as
  when the store's process has gone;
- **silent**: connections stay open and new ones are accepted, but
  nothing crosses, as across a partition.

Five tests:
- **Postgres away**, refused and silent;
- **Redis away**, refused and silent, with Bob taking one call at a
  time, so whether the cluster still counts his ended call is something
  a caller hears;
- **a killed node**: two call managers in a cluster, one killed with
  `SIGKILL` while it carries Alice's call to Bob, and started again on
  its spool.

After each, the test checks:
- that the call up lived, heard both ways, and that its BYE crossed;
- that a REGISTER and a new call got a final answer within 8 s;
- that the call's record was written;
- that the node rejoined the cluster;
- that the events raised while Redis was away reached the stream;
- that Carol could register and call again, with no restart.

The conference node, the events server and `fcp-jobs` are D5b
(below). A replica failover and an upgrade over a populated database are
D5c.

| Finding | Severity | Dealt with |
|---|---|---|
| With Postgres away, a call manager answered no REGISTER and no new INVITE. Refused, each request waited for the pool's 30 s acquire timeout, and the phone had given up at 32 s with no answer. Silent, a query on an open connection waited until the network came back, however long that was. Calls already up were unaffected. | S2 | Fixed: `fcp_storage_postgres::primary_health::PrimaryHealth` asks the primary `SELECT 1` every second, and two missed answers mark it down. While it is down, the handler's store gate answers a REGISTER or a new INVITE at once with `503 Service Unavailable - store unreachable` and `Retry-After: 5`. A REGISTER already under way is cut off at five seconds with the same 503. The pool's acquire timeout is `[storage] acquire_timeout_secs` (5). `/ready` is 503 while the database is down, and `/status` and `/metrics` say so (`fcp_database_up`, `fcp_database_outages_total`). |
| A call manager killed mid-call kept its users busy, and its tenants' and trunks' places taken, for three hours. Every call takes a lease in Redis, per user (for call waiting and `max_calls`), per tenant and per trunk. Only the node carrying the call refreshes or releases it, and the lease stood for three hours (`DEFAULT_LEASE_TTL_SECS`). Measured: Bob, who takes one call at a time, was still answered 486 Busy on the surviving node 13 s after the dead node's call to him ended with it. A shared line's appearance stood for half an hour. A release that failed while Redis was away stood just as long. | S2 | Fixed: `[cluster] lease_ttl_secs` (60) is the lease time for all four, refreshed every quarter of it, and the settling tick runs at least that often. Bob is reachable as soon as the lease lapses: within `lease_ttl_secs` of the node's death, and 2 s after Redis returned in the test. |
| A call up when its node was killed had no record, ever: its talk time was never billed and nobody could find the call. | S2 | Fixed: every ten seconds, the node keeps each answered call's record as it would be written then, in `<spool_dir>/open/<call>.json` (`fcp_cdr::OpenRecords`). The call's end writes the real record and removes the file. A node that starts and finds files left writes them as answered and ended by `node_failed`. A call's record now has the call's own id as its `cdr_id`, so a node killed between the two writes is billed once. |
| An emergency call cannot be put through while the database is away: the caller, the tenant, the emergency numbers and the location are all read from it. | S2 | #326. It is refused with the same 503 as any other call. That is faster than the 32 s wait it had before, but it is still not a call placed. |
| Redis away, refused or silent: calls up and new calls carried on, and every `call.ended` raised meanwhile reached the stream once Redis was back. | — | No change needed; the tests keep it so. |

### D5b. Faults beside the call manager

Three tests kill the processes that work beside the call managers, each
with work in hand:
- **a conference node** holding half a room
  (`fcp-conference-server/tests/cluster.rs`,
  `callers_on_a_killed_node_are_moved_to_another`): Alice on one node
  and Bob on the other, both through a call manager. Alice's node is
  killed with `SIGKILL`; she must hear Bob again, Bob her, and Carol,
  who calls in afterwards, must land on the node that is left and hear
  Bob;
- **the events server** (`fcp-events-server/tests/faults.rs`): a
  tenant's webhook endpoint in the test, which either answers or holds
  every request open, and an emergency contact that is that endpoint.
  Events raised while the server is down must arrive when it starts.
  Events it has taken off the stream, with every request held open,
  must arrive after it is killed and started again, each under one
  `event_id`, and the emergency call's notice with them;
- **`fcp-jobs`** (`fcp-jobs/tests/faults.rs`): five records on the CDR
  stream while the processor's database is silent, so it holds the batch
  on its insert. It is killed; the next processor must store each record
  once, charge each call once and announce each record.

| Finding | Severity | Dealt with |
|---|---|---|
| A caller on a killed conference node was left on dead air for as long as they would hold. The call manager kept the call up, billed it and counted its user busy: a conference call's media does not pass through the call manager, so nothing timed out, and nothing watched the node go. The docs said its callers were "gone with it". The rest of the room was unharmed: the other node hung up its trunk within 15 s, and a new caller was placed there. | S2 | Fixed: the call manager checks its conference calls every second. A call whose node has left the member list is moved into the room on a live node by the migration code with no ticket (a new B-leg with the caller's identity, the caller re-INVITEd) and nothing sent to the dead node. If no node will take it, the call ends with `Reason: SIP;cause=503;text="conference node gone"` and `node_failed`. Measured: moved within a second of the node leaving the member list. The caller joins afresh: a PIN room asks again. |
| An events server killed with events in hand lost them. It acknowledged each batch on the Redis stream as soon as it had passed it to its in-process bus, and the webhooks and the emergency notifier took events from there. So an event taken and not yet delivered was gone, with no delivery row for the retry processor to find, and so was the on-site notice of an emergency call (Kari's Law). Each start used a new consumer name and claimed nothing, so anything delivered to a dead consumer and not acknowledged stayed pending for good. The in-process bus also drops what a slow follower falls behind on. | S1 | Fixed: with Redis as the bus, the stream consumer hands each event to `WebhookDispatcher::take` and the notifier before it acknowledges it. `take` writes the delivery rows first (`WebhookStore::log_delivery_once`), with ids and the payload's `event_id` derived from the stream entry, and then sends. The notifier skips contacts already tried for the call. Entries a consumer held past `[event_bus] claim_idle_secs` (60) are claimed. Measured before the fix: none of three webhooks or the notice arrived after the kill. |
| An event handed over twice could reach a receiver twice under two `event_id`s, so the receiver could not tell it was one event. | S3 | Fixed with the above: the id is derived from the stream entry. |
| A webhook or notice sent but not yet recorded when the events server dies is sent again when it starts. | — | As designed: delivery is at least once (`docs/WEBHOOKS.md`). |
| Events raised while the events server was down reached it when it started: the consumer group keeps them. `fcp-jobs` killed holding a batch: the next processor stored, charged and announced each record once. | — | No change needed; the tests keep it so. |

### D5c. Failover and upgrade

**The primary fails over to its standby.** `fcp-call-manager/tests/faults.rs`
runs a Postgres primary and a streaming standby of its own
(`fcp_testkit::pg_pair::PgPair`, from the server's binaries; CI's runner
image ships them). The node is given the primary by an address the
failover moves, as a VIP, a DNS name or a pooler does: a `CutLink` the
test retargets. Two tests:
- **the primary fails over mid-call**
  (`the_primary_fails_over_to_its_standby_mid_call`): Alice talks to Bob,
  with the node reading its configuration from the standby. The primary
  is crashed (`pg_ctl stop -m immediate`), the standby promoted and the
  address moved. The call must live and its BYE cross. While there is no
  primary, a REGISTER must be answered 503. Afterwards, Carol must
  register and call Dave, a call to Alice must find the registration
  made before the failover, and every call's record must be written;
- **a standby at the primary's address**
  (`a_standby_behind_the_primary_address`): the address moves to the
  standby before it is promoted, as when a failover moves it early, or
  an old primary comes back as a standby while a node still resolves the
  old address. A REGISTER must be answered 503 and `/ready` must say so.
  A call must still be put through, and its record written once the
  standby is promoted.

**An upgrade over a populated database.** The nightly's stack job
(`.github/workflows/images.yml`) starts the stack on the previous
release: the last good scheduled run on main, its images and its
compose project. `compose_live.rs` (`fresh`) makes the tenant and its
users, registers two phones with sealed passwords and carries a call.
The stack is then upgraded in place by `BACKUP_AND_RESTORE.md` §5.3:
backed up, this release's compose project, pull, `up -d`. A call through
the upgraded stack (`upgraded`) must find the tenant, the users and the
earlier call's record, and the call manager must run this release. The
backup, drop and restore that followed already continue from there.

| Finding | Severity | Dealt with |
|---|---|---|
| A call manager whose primary's address led to a standby answered every REGISTER `500 Internal Server Error`, and `/ready` said it was ready. The probe asked `SELECT 1`, which a standby answers, and the write was then refused with `cannot execute INSERT in a read-only transaction`. A phone may not retry a 500, and a balancer kept sending registrations to the node. | S2 | Fixed: the probe asks `SELECT pg_is_in_recovery()`. A standby is up and not writable (`database.writable`, `fcp_database_writable`). A REGISTER is answered `503 Service Unavailable - store read-only` with `Retry-After: 5`, and `/ready` is 503. New calls are still placed: they only read, and their records wait in the spool. |
| A REGISTER whose write the store refused, in the second before the probe noticed, was answered 500. | S3 | Fixed: the registrar answers a store failure `503 Service Unavailable - store error` with `Retry-After: 5`. |
| The upgrade, first run on this branch (Images run 38987345367): from 0.25.403 to 0.25.409, the tenant, users, registrations and the earlier record were all found, and the call manager ran 0.25.409. No migration landed between the two, so that night proved the procedure and not a schema change; each night crosses whatever migrations landed since the last good one. | — | No change needed; the nightly keeps it so. |
| The primary failed over mid-call: the call lived, REGISTERs were refused 503 until the promotion, and then registrations, calls and records carried on with no restart. | — | No change needed; the test keeps it so. |

### D6. Soak

**What runs.** `fcp-soak` (`crates/fcp-loadgen/src/soak`) seeds a tenant
through the admin API and puts the whole stack under three loads at
once, from `fcp_loadgen::run`:
- calls between phones, each answered by its own phone, with the phones
  re-registering underneath;
- a conference room three callers keep busy;
- a room whose two callers leave together, so it empties and is made
  again every round.

Beside them, an administrator and a user use the API every few seconds:
- lists of users, records, rooms, calls and nodes;
- an extension made and deleted;
- do-not-disturb on and off;
- a sign-in and a sign-out.

**What is watched.** A sampler (`soak::stack`) records, every 30 s:
- each `fcp-*` service's resident memory, open descriptors, UDP sockets
  (the RTP ports a media path holds) and threads. A container is read
  with `docker exec`, a local process from `/proc`;
- Redis's keys, memory and clients;
- Postgres's connections and every table's live rows.

**How growth is judged.** `soak::growth` leaves the warm-up out (a fifth
of the run, at most 30 minutes) and cuts the rest into four windows.
A series grows when its last window's median is above its first by
more than its tolerance, and still above its third. A pool that filled
early stands still at the end; a leak does not. What grows by design is
named in `EXPECTED_GROWTH`, each entry with its reason: a call record
per call, an audit row per write, SIP traces kept until a tenant's
retention policy says. A run fails on:
- a load that was refused, dropped or silent;
- an API answer of 5xx;
- any other growth.

**Where it runs.**
- Weekly, by `.github/workflows/soak.yml`: four hours against the
  nightly images on a GitHub runner.
- On a workstation, against the binaries of the checkout:
  `FCP_SOAK_SECONDS=3600 cargo test -p fcp-loadgen --test soak_stack`.
  `FCP_SOAK_CALLERS=0` or `FCP_SOAK_ROOMS=0` leaves one load, to tell
  whose a growth is.
- In CI, the same test for 90 s, as a check that the soak itself works.
- Once by hand for 24 hours on a small cloud VM, before D6 is called
  done (Q6).

**What a run shows.** With the fixes below, an hour on a workstation
(v0.25.412) offered 1,801 calls and 840 room joins, all answered, none
dropped or silent, and 7,956 API requests, none failed. Every process's
memory, descriptors, sockets and threads, Postgres's connections and
Redis stood still after the warm-up. What grew was the audit log, call
records, sign-in sessions, SIP traces and the site journal, each named
in `EXPECTED_GROWTH`.

**The 24-hour run (2026-10-18/19, v0.25.412, a GCE e2 VM).** 43,201 calls
and 14,381 room joins offered, all answered, none dropped or silent; 95,640
registrations; 183,600 API requests, none failed. The verdict failed on two
growths. The call manager's memory rose linearly, 96.6 → 149.8 MB, about
1.2 KB a call, with nothing it counts left over: a leak to find with a
heap profile (#344). Redis rose 20 → 89 MB, all of it the CDR stream's
24-hour replay window, which a 24-hour run cannot see level off (#345: the
soak runs it at an hour; ~1.5 KB a record). And forge-rtp logged a warning
for every refused latch packet — 44 million lines, a 6.7 GB log
(forge-media #175).

| Finding | Severity | Dealt with |
|---|---|---|
| A conference room was a number, not a tenant's number. The conference server keyed its live rooms — the mixer, the session index, recordings, DTMF handlers, the floor, speaker and content claims — by room number alone. Two tenants that both numbered a meeting 8801 shared one mix on a node, and a tenant's control request (mute, kick, lock, record, video) acted on whichever 8801 was live, since `control_tenant` preferred the live room's tenant to the one the admin API named. It went unseen because every SIP caller was the default tenant's: the call manager's leg into a room named no tenant, and the conference server, reached at an address, refused any other tenant's caller (403) once there was more than one tenant. The soak's tenant, made through the API, could not join a room at all. | S1 (two tenants' calls mixed; one tenant controlling another's room) | Fixed in v0.25.410. A live room is a `RoomKey` (tenant and number), and `RoomBridge` is the only way to the mixer's rooms, so a lookup by number alone does not compile. The call manager names the caller's tenant and user on its legs into a room, and the conference server believes them from a trusted edge (`resolve_tenant`). Every HTTP route that names a room takes the tenant from the header the admin API and the peers already send; `/conferences` lists each room with its tenant. `rooms.rs` `two_tenants_with_one_room_number_meet_apart`. |
| Callers in a room that emptied and was made again often heard nothing: the soak's two-caller room was silent on two calls in three. Two races. forge-conference's get-or-create checks and then inserts, so two callers making a room at once could each make it, the second replacing the first's. And the sweep that removes an emptied room checked it was empty, read the meeting, then removed it; a caller who joined during the read was left in a room no one else could reach. | S2 (silence in a conference) | Fixed in v0.25.410: `RoomBridge` makes and removes rooms one at a time, a caller holds a `Joining` from finding the room until it is in, and the sweep removes a room only if it is still empty and no one is joining (`delete_room_if_idle`). `rooms.rs` `callers_meet_in_a_room_made_again_round_after_round`; the check-then-insert is also forge-media's to fix. |
| The call manager held two more UDP sockets for every call it had ever anchored: 114 after 57 calls, still open after a quiet minute, some ports bound twice. fcp-media ended a session by stopping its forwarding, which returned the ports to the pool. But forge-engine's `SessionManager` kept the session in its map, with its sockets. The next call was given the "free" port and bound it beside the old socket (`SO_REUSEADDR`), so the kernel chose which socket a packet reached. Descriptors grew without bound. | S1 (descriptors exhausted in time; packets to a dead call's socket) | Fixed in v0.25.411 (#331): a session ends through `SessionManager::stop_session`. `a_terminated_session_lets_its_sockets_go`. |
| The usage rollups never rebuilt on schedule. `fcp_cdr::maintenance` took the session advisory lock `fcp:cdr-rollups` on one pooled connection. The rebuild then waited for the transaction lock of the same name on another connection, held by its own scheduler, for as long as the process lived. Each `fcp-admin` and `fcp-jobs` held a transaction open throughout, which also keeps vacuum from removing dead rows. The tests called the rebuild directly and never took the scheduler's lock. | S2 (usage figures stale; a transaction open for ever) | Fixed in v0.25.412: the scheduler's lock has its own name (`fcp:cdr-rollup-schedule`). `the_scheduled_rollup_rebuild_finishes` times out without the change. |
| A caller could be answered on a port the node could not bind. The conference server and the call manager's queue took a port pair from their pool, put it in the answer, sent the 200 OK, and only then bound it. The pool knows only the node's own pairs: when another process on the host held one, the bind failed after the answer, and the caller sat in a call that no room or queue held, with no BYE. Dialling out and trunks between nodes made their offer the same way. Found when a CI run's test node met a port held by another test. | S2 (an answered caller in dead air) | Fixed in v0.25.412: `fcp_media::bound_port_pair` binds a pair before it is offered or answered on, passing over pairs held outside the node. It is used by the conference server's joins, dial-out and trunks both ways, and the queue's answer. `rooms.rs` `callers_are_answered_on_ports_the_node_could_bind` holds 46 of the node's 50 pairs and fails without the change. |


### D7. The admin UI in a browser

`fcp-admin/tests/ui.rs` starts an admin on a database of its own, with
authentication on, serving the UI it embeds. It makes a super admin, a
tenant admin and an end user through the API. It then runs the Playwright
suite in `crates/fcp-admin-ui/ui/e2e` against the admin, in Chromium.
The suite is told where the admin is, and who may sign in, through
`FCP_UI_*` variables.

**Every page** (`pages.spec.ts`):
- The super admin signs in through the form and opens every page in the
  navigation by its link, then Settings.
- The end user opens every page of their portal.
- A page fails when it throws, logs a console error, or shows the error
  boundary.
- It also fails on any API answer of 400 or more, other than the refused
  session refresh before sign-in and five reads that answer 404 for
  "nothing yet" (`NOTHING_YET` in `fixtures.ts`).
- And it fails on any button without a name a screen reader can say.

**The main objects** (`crud.spec.ts`), as a tenant administrator: users,
devices, numbers, queues, flows, schedules, prompts, webhooks and
identity providers. Trunks are done as the super admin, since in an
enterprise (the default deployment mode) trunks are the platform's.
- Each object is made through its form, changed (and, for users, devices
  and numbers, has a field emptied), and deleted.
- Each step is checked through the API.

**Where it runs.** CI's `Admin UI in a browser` job builds the UI, then
the admin, installs Chromium, and runs the test with
`FCP_UI_TESTS=required`; the traces of a failure are kept. Locally the
test skips when the admin embeds no UI or Chromium is missing.

| Finding | Severity | Dealt with |
|---|---|---|
| The admin image served no UI. `Dockerfile.admin` built the binary without building the UI first, so the binary embedded the placeholder page and `/admin/` answered "Admin UI not built". Every deployment from our images had no admin UI; the stack test only asked the admin's API. | S2 (the admin UI absent wherever our images run) | Fixed in v0.25.413: the image builds the UI in a Node stage and embeds it. `compose_live.rs` checks `/admin/` on the restored stack, which always runs this release's images. |
| The UI had not built since 2026-10-06. A security update moved Tailwind from v3 to v4, whose PostCSS plugin is a separate package, and `vite build` failed. CI only type-checked the UI. | S2 (no UI could be built) | Fixed in v0.25.413: `@tailwindcss/postcss`, and the stylesheet imports Tailwind and keeps `tailwind.config.ts` through `@config`. The new CI job builds the UI before anything else. |
| No flow could be made from the UI. A new flow was sent with a greeting whose `next` was `null`, which the flow schema refuses (a node's next is a node). The error was not shown either: the dialog stayed open. It had been so since the flow builder was written. | S2 (flows could be made only through the API) | Fixed in v0.25.413: a new flow is a greeting and a hangup; the dialog shows a refusal, and a refused delete says why. |
| An emptied field was never cleared. The user, device and number edit forms left an empty field out of the request, so the old value stayed. The API, sent an empty string, stored `""` for a user's e-mail, phone, mobile, department, job title, location and emergency contact, and a device's model, firmware, SIP URI and location. | S3 | Fixed in v0.25.413: the forms send the emptied field, and the API clears a field given an empty string, as it already did for an extension. `crud.spec.ts` empties a user's e-mail, a device's model and a number's caller name. |
| The trunk and queue forms failed silently. A refused save left the dialog open with no message; a trunk without a name was not explained. | S3 | Fixed in v0.25.413: both forms and their delete dialogs show why. |
| About forty buttons had no name a screen reader could say: every row's actions menu, the trunk, queue and schedule icons, the table's pager, the calling settings' switches, Branding's uploads, and the flow builder's back button and name field. | S3 (accessibility) | Fixed in v0.25.413; `pages.spec.ts` now fails on a button without a name on any page. |
| A user without a mailbox saw the API's raw error on their voicemail page, not "No voicemail mailbox is configured". | S3 | Fixed in v0.25.413: the client reads the 404 as no mailbox, as it already did for a brand. |
| A schedule was deleted at one click, without the confirmation every other page asks for. | S3 | Fixed in v0.25.413. |
| The UI shows every page and button to every role, and the API refuses what the role may not do. A tenant administrator in an enterprise is offered "Add Trunk" and refused. | S3 | Not fixed: the UI knows nothing of permissions, and teaching it is a change of its own (#333). A refused save now at least says "Insufficient permissions". |

### D8. The real world, by hand

The D8 server is the compose stack from the nightly images on a GCE VM
with a static address and a Let's Encrypt certificate for `d8.feryx.io`
(`deploy/gcp-d8`). It is up for the sessions and removed after them.

The checklist, worked through together:
1. **Poly phones.** Provisioned from the tenant's URL, then by Poly ZTP
   if an account exists. Register over UDP and TLS, call each other, hold
   and resume, blind and attended transfer, park, voicemail and its
   light, BLF, a shared line, resync from the admin.
2. **Twilio.** A tenant trunk to the Elastic SIP trunk's termination
   URI, its number on the tenant. Inbound to a user, a flow and a queue;
   outbound with the line's caller ID; DTMF both ways; a long call; `933`.
3. **Anvil and push.** Sign in on iOS and Android, register, call a Poly
   phone and the PSTN. Asleep, woken by a real APNs and FCM push for a
   call from a phone and from Twilio; a cancelled call's push.
4. **Browsers.** Chrome, Firefox and Safari in one WebRTC room with
   video and screen share, with a Poly phone dialled in.

**Session 1 (2026-10-18).** A Poly VVX 250 (UC Software 6.4.7) and a
Twilio Elastic SIP trunk with one number. The VVX was provisioned from the
tenant's URL over HTTPS and registered; a call from a mobile through
Twilio rang it and was answered with audio both ways; a call from the VVX
to the mobile left through Twilio with the line's number as its caller
ID; voicemail took a message and the VVX's light came on. Each of these
needed fixes first — the table — and the menu that plays messages is
silent without text to speech (Q-D8a below).

**Session 2 (2026-10-19).** The fixes of session 1 retested on the images
they shipped in, with every workaround removed: the VVX registered as
`alice@d8.feryx.io`; a call from the mobile through Twilio rang it with
ringback and audio both ways; a call out left Twilio in E.164
(`send_e164`) with the line's number; voicemail answered with Piper's
greeting, kept the message, and the VVX's lamp came on and went out
through Redis — with the admin's deletes and with `*98`'s; `*98` took the
PIN, played the message and deleted it, its prompts clean in G.722 after
the resampler was replaced (forge-media v2026.10.19). The rows below from
the stored-TOML finding on came out of session 2. forge-media #174 (the
transcoder's G.722 mapping) is open.

| Finding | Severity | Dealt with |
|---|---|---|
| A trunk host given as a name with no port was resolved and its **address** put in the Request-URI. Twilio, which knows a trunk by the domain there, refused every outbound call (403). | S2 (no outbound call to a carrier that names trunks by domain) | Fixed in v0.25.414 with siphon-rs's `InviteOptions::destination` (sip-uac 0.9.0): the name stays, the address the router chose is where the INVITE goes (TLS checks the name); emergency attempts too. Test: `trunk_router` `a_route_to_a_resolved_name_keeps_the_name`, siphon-rs `an_invite_goes_to_its_destination_and_keeps_the_name`. |
| The default tenant had no row, so nothing could claim a SIP domain for a single-tenant deployment: a phone provisioned to register as `alice@d8.feryx.io` was refused ("unknown tenant domain"), and provisioning's fallback told it to. Tests never met it: the testkit's `Seed` writes the default tenant's row by hand. | S2 (phones cannot register by name) | Fixed in v0.25.415: the default tenant reads as its defaults, its first update stores it, it is listed on the Tenants page and never deleted. Test: `the_default_tenant_takes_a_sip_domain_and_is_never_deleted`. |
| Three response builders copied only the first `Via`: the simple responses (480, 486, 503…), the relayed provisional and the digest challenge. A carrier's INVITE arrives through its edge with two; its callers heard no ringback, and a refused call kept ringing at the caller's end. The relayed 180 also carried the callee's Contact (a phone's private address, to the carrier) and the callee's Record-Route. | S2 (no ringback and no refusal for any call through a proxy) | Fixed in v0.25.416. Test: `sipp.rs` `a_caller_behind_a_proxy_hears_it_ring_and_be_refused`, failing on the old code at the 401 and the 486. |
| The voicemail server never made its directories, and the recorder refuses one that does not exist: in the compose stack every caller heard the beep and was hung up on. No test had left a message with the real server; every other reaches voicemail through a stub. | S2 (no voicemail in the shipped stack) | Fixed in v0.25.417: made at startup. Test: `fcp-voicemail-server/tests/messages.rs`, the first to leave a message with the real server. |
| On a cloud VM whose public address is NAT at the provider's edge (GCE, an AWS elastic IP), the services cannot reach one another at the address they advertise: the caller's BYE never reached voicemail, which recorded for five minutes. | S2 (calls between services break on a cloud host) | The host's fix — the address on the loopback — documented in the compose README and done by `deploy/gcp-d8`; v0.25.418 has every SIP service warn at startup when its address does not come back (`fcp_sip::self_reach`). Advertising private addresses to siblings is #335. |
| A call that ended unanswered (refused 480, busy, a carrier's 403) kept its media session and two RTP ports for as long as the node ran: about 500 such calls exhaust the call manager's ports. The soak's calls are all answered. | S2 (port exhaustion) | Fixed in v0.25.419: `CallManager::set_on_unanswered_media`, ended by the SIP handler in every binary. Test: `calling.rs` `a_call_refused_before_it_rang_leaves_no_media_session`. |
| A number bound to a user rang only the devices listed on the binding; one with none — what the API makes — rang nobody, and Twilio's call was refused 480 with the phone registered. | S2 (a user's number does not ring) | Fixed in v0.25.420. Test: `trunks.rs` `a_carriers_call_to_a_users_number_rings_the_users_phone`. |
| A user's calls out presented their SIP name unless `outbound_caller_id` was set on the line, which the API does not set. Twilio refuses it. | S3 | Fixed in v0.25.421: the line's number. Test: `a_line_with_no_caller_id_set_presents_its_number`. |
| Numbers went to a trunk as dialled; Twilio wants E.164, and a prepended `+` doubled one already there. A star code that is no feature (`*9`, sent by the VVX's own digit map) went to Twilio as a number. | S3 | Fixed in v0.25.422: `Trunk::send_e164` (migration 116, API, form); a stray star code is 404. Test: `an_e164_trunk_gets_e164_and_a_stray_star_code_reaches_no_carrier`. |
| The Polycom and Yealink starters set up no message waiting, and the VVX's default digit map sent `*9` before `*98` was typed. | S3 | Fixed in v0.25.423. |
| The compose stack set no `[api.provisioning] sip_server`: phones were told to register to `fcp.local`. | S3 | Fixed in v0.25.423: `HOST_IP`. |
| The provisioning key's whole URL ignored `[api.provisioning] public_url`. | S3 | Fixed in v0.25.414; `provisioning.rs` checks it. |
| The menu that plays voicemail (`*98`) speaks only through text to speech, which the compose stack leaves off (its image carries PyTorch); with it off every prompt is silence, as is the default "leave a message". | S2 (the shipped stack's prompts are silence) | Fixed in v0.25.425 (Q-D8a, answered: on by default): Piper is the default engine, in a small `fcp-tts` image that every stack runs; Kokoro and local Whisper are `fcp-tts-full` (`FCP_TTS=tts-full`); the images carry a default greeting spoken by Piper (`docs/SPEECH.md` §8). |
| Built-in flows were stored: every `*98` and hoteling login wrote a flow row that was never removed, and each mailbox's flow belonged to no tenant, so the Flows page never showed it and no one could edit it. | S3 (a row per voicemail check, forever) | Fixed in v0.25.427: `fcp_flow::BuiltinFlows` holds them in memory, let go at the call's end; migration 117 removes the rows. A mailbox can hand its calls to a flow of the tenant's (`flow_id`, *Call handling*): `docs/FLOWS.md`. Tests: `calling.rs` `voicemail_access_leaves_no_flow_behind`, `a_mailbox_that_hands_its_calls_to_a_flow_runs_it`. |
| The compose proxy (nginx) looked its backends up once, at start: a restarted admin came back on another address and every HTTPS request was 502 until the proxy restarted too. | S3 | Fixed in v0.25.428: Docker's resolver, looked up per request; proved on the D8 server by restarting the admin under it. |
| The operational TOML sections (`[media]`, `[voicemail]`, `[b2bua]`…) are stored at a service's first boot and read from the database after; nothing reconciled them with the file, so a key set later — the default greeting the images now carry, a changed `public_ip` — was never read, in all eight binaries that store them. Callers heard silence before the beep. | S2 (a TOML edited after the first boot is silently ignored) | Fixed in v0.25.429: `fcp_core::reconcile_with_seed` settles each key three ways against the TOML last seeded (`config_settings.seed`, migration 118, backfilled from the audit log): a key the file changed takes the file's value, every other keeps the stored one, so an edit through the API stands until the file changes that key. Tests: `config_store` unit tests, `fcp-voicemail-server/tests/messages.rs` `a_key_added_to_the_toml_later_is_read_and_an_api_edit_stands`. |
| A message left lit no lamp until the phone subscribed again: the call manager tells message-waiting subscribers on `voicemail.*` events from its own bus, and the compose stack ran the events server's bus in memory, so what the voicemail server posted to `/ingest` never reached it (the call manager even warned that `use_redis` was off). | S2 (no message waiting in the shipped stack) | Fixed in v0.25.429: the compose stack's events go through Redis (`[event_bus] bus_type = "redis"`, the call manager's `use_redis`). |
| `*98` looked for the caller's mailbox by their extension (`1001`); a mailbox made through the API is named by the username (`alice`), so every PIN was "incorrect". | S2 (no one can hear their messages) | Fixed in v0.25.430: the calling user's own mailbox, by user. Test: `mwi.rs` `the_voicemail_menu_opens_the_callers_mailbox_and_puts_the_lamp_out`. |
| In the compose stack `*98` runs on the call manager, which could not see the voicemail server's volume and accepted only files under its own `recording_dir`: messages would play as silence and could not be deleted. | S2 | Fixed in v0.25.430: `voicemail-data` mounted on the call manager, `[media] voicemail_recordings_dir` (`VoicemailConfig::message_dirs`). Same test. |
| A node on the Redis bus never heard its own events: it published to Redis alone and its subscriber skips its own. Leaving `*98` changed no lamp; with the menu's update now announced, it would still have reached only other nodes. | S2 (message waiting, and any listener of the node's own events, on every clustered node) | Fixed in v0.25.430: `fcp_events::LocalAndCluster` publishes to the node's own bus as well; leaving the menu announces `voicemail.mwi_updated`. Same test, failing on the old bus. |
| The prompts said "PIN", which Piper spells out letter by letter. | S4 | Fixed in v0.25.430: "pin"; migration 119 updates the stored system prompts still holding the old text and lets their audio be made again. |
| The voicemail server logs the events token in the WebSocket URL it connects to. | S3 (a secret in logs) | #340. |
| The events server's `consumer_name` defaults to a random name per start, so with the TOML now reconciled its stored section is rewritten at every start. | S4 | #341. |
| With the codec agreed, `*98` on the VVX still sounded off: 8 kHz prompts played to a G.722 leg were resampled a 20 ms frame at a time, by straight lines, each frame alone — a step at every frame's edge (50 a second) under a metallic voice; a 1 kHz tone measured 24.5 dB SNR. forge-resampler, used by every transcoder, was linear as well. | S2 (every prompt to a wideband phone, and every transcoded call, distorted) | Fixed in v0.25.432 with forge-media #173: a band-limited (Kaiser-windowed sinc), streaming resampler, playout keeping one per playback across frames (> 50 dB), `resample_all` for whole buffers (Piper, queue recordings, speech recognition, conference prompts). |
| The voicemail menu read "You have 0 new messages … Press 1 for new messages", and "1 new messages". | S4 | Fixed in v0.25.432: counts in English, only the choices there is something behind. |
| The voicemail server warned "MWI notifier not configured" on every message, though message waiting goes by events. | S4 | Fixed in v0.25.434: debug. |
| Session 3: an attended transfer to anything the node answers itself — the VVX's Transfer button to a menu — was refused: the consultation call has the flow, not a second party, and `Replaces` answered 481 "no party to join". | S2 (no attended transfer to an IVR, voicemail or a menu) | Fixed in v0.25.436: the transferee takes the consulting phone's place on the media session's A side, and the flow carries on with them. Test: `calling.rs` `an_attended_transfer_to_a_menu_hands_the_caller_to_it` (fails on the old code). |
| Queues are off unless `[queue] enabled`, which the compose stack never set: a queue made on the admin API or its page was never answered, and nothing said so. With it on, the node read its queues once, at start — a queue made later was refused until the node was signalled. | S2 (queues do not work in the shipped stack) | Fixed in v0.25.436: the compose and site stacks turn queues on; the node reads them again every `[queue] store_sync_secs` (30) and on SIGHUP through one `sync_from_store`; a node with queues off warns at start when the API has some. Test: `calling.rs` `a_queue_made_after_start_is_answered`. |
| Session 3, hold and park: hold and resume were clean both ways, but a call the VVX placed and then parked with `*70` gave the mobile choppy music. A capture showed two streams to Twilio from one port, the music and the mobile's own audio: the parked party moves to the A slot, and the B slot the parker left still named it, so everything it said was relayed straight back. | S2 (choppy hold music for anyone a caller parks) | Fixed in v0.25.437: `MediaManager::move_peer` carries the parked party's address, latch and SRTP keys to A and leaves B with no peer, and `release_leg` empties B when the callee parks. Test: `calling.rs` `a_caller_parking_with_the_code_leaves_the_parked_party_only_the_music` (299 packets in 3 s on the old code, about 150 with the fix). |
| `*70` answered 200 with no SDP and "Parked Orbit 701" as its reason phrase, then forgot the call: the VVX sat on a silent call until hung up by hand, and never learned the orbit. | S3 | Fixed in v0.25.437: `*70` answers into an inline flow that says "Parked on 7 0 1." and hangs up. Same test. |
| A flow's Queue node never queued anyone: it set `__queue_*` variables nothing read and went straight to `next_on_answer`, so the D8 menu's "press 2 to wait in the test queue" said goodbye. | S2 (no flow can put a caller in a queue) | Fixed in v0.25.438: the node dials the caller into the queue's extension at the node, as a Dial to a user does; the queue is found by name or extension among the node's queues (`QueueHandler::extension_of`, TOML and database alike); `next_on_answer` after that call, `next_on_timeout` when it is not answered, `next_on_error` for a queue the node does not have. Test: `calls.rs` `a_flows_queue_node_puts_the_caller_in_the_queue`. |
| A Gather's timeout ran from the start of its prompt, not its end. The D8 menu's prompt ("…Press 3 to test your keypad.") is about as long as its 8 s timeout: a caller who listened to the end heard "Goodbye." No test had a prompt longer than its timeout. | S2 (callers who wait for the menu to finish are hung up on) | Fixed in v0.25.439: `Listening::wait` keeps the deadline itself, from the prompt's end (or the moment the caller speaks over it); the digit collector keeps no time; digits pressed by the deadline still count as partial input. Test: `calling.rs` `a_menus_timeout_runs_from_the_end_of_its_prompt` (a 4 s prompt, a 2 s timeout, 3 pressed after the prompt; and a 3 pressed during it). |
| A menu could not ask again: a Gather asked once, then took `on_timeout` or `default`; "repeat the greeting three times, then go to the operator" needed three copies of the node (a Set Variable cannot count). Every IVR has it. | S3 | Added in v0.25.440: `max_attempts`, `no_input_prompt`, `invalid_prompt` and `on_max_attempts` on the Gather (`docs/FLOWS.md`, *Menus*), in the flow builder with 3 attempts and two spoken retry prompts for a new menu; unset asks once, as before. Test: `calling.rs` `a_menu_asks_again_then_gives_up_where_it_is_told`. |
| The VVX's offer names telephone-event 127; a node-answered call's answer is the session's own SDP, which always says 101 (`generate_negotiated_sdp`: "forge-engine hardcodes this PT"), while the A leg's detector is told 127 from the offer. RFC 3264 asks the answer to keep the offer's number. Digits were still heard on the D8 menu, so the VVX sends what it offered. | S4 | Open. |
| Session 3, keypad: the menu read the caller's keys back as a number — "You entered {{keys}}" with 5551234 said "five million, five hundred…". A Say had no way to read digits as digits. | S4 | Fixed in v0.25.441: a `{{name\|digits}}` filter reads one key at a time (`*` star, `#` pound), in the engine and in a Gather's retry prompts (`docs/FLOWS.md`). |
| Session 3, park again: `*70` on a call the VVX had *answered* (a Twilio call to the DID) was refused 480, busy on the phone. The parker was looked for by name on each leg's From/To, and the leg toward a callee is To the number dialed (the DID, or `1001` on an internal call), not to who answered; this morning's park test had the parker as the caller. | S2 (a callee cannot park) | Fixed in v0.25.442: `CallManager::find_established_by_party` also matches a leg's dialog remote target, the phone that answered, as the executive push already did. Test: `calling.rs` `a_callee_parking_with_the_code_parks_the_caller`. |
| Session 3, queue: the menu put the caller in the queue, who then heard two minutes of silence and was hung up. No agent was logged in (the queue's one agent was the caller), and a queue plays music and announcements only from `queue_audio_dir`, which the shipped stack does not set. | S2 (a queue caller hears dead air) | Fixed in v0.25.442: a queue with no music of its own plays the node's hold music (`MohCache::with_default`; an empty `moh_class`, as the API stores it, is the default). Position announcements still need `queue_audio_dir`. Test: `calls.rs` `a_waiting_caller_hears_music_until_an_agent_logs_in_by_name`. |
| `*50` matched the dialer's SIP name against the queue's agents, which the admin lists by extension: the VVX (`alice`) could never log in to a queue that named `1001`. | S2 (agents by extension cannot log in from a phone) | Fixed in v0.25.442: the dialer's user is found and their name and extension both match. Same test. |
| Session 3, park from the callee: the VVX heard "Parked on 7 0 1." but its own call stayed on hold. The node's BYE to it took CSeq 2, the number its PRACK for the VVX's reliable 180 had already used: siphon-rs built the PRACK from a clone of the dialog, so the dialog the 200 confirmed began again at the INVITE's CSeq. The VVX answered the BYE 500 and kept the call. Every BYE or re-INVITE the node sent toward a phone that rang reliably did the same. | S2 (the node cannot hang up or re-INVITE a phone that rang with 100rel) | Fixed in v0.25.443 (siphon-rs `fix/prack-cseq`, sip-uac 0.9.1): the PRACK takes its CSeq from the stored dialog and stores it back. Test: `sipp.rs` `the_node_pracks_a_callee_that_rings_reliably` now checks the BYE's CSeq is past the PRACK's (2 and 2 before). |
| Session 3, `*50` from the VVX: the agent was logged in, but the phone stayed on a silent call. Every feature code that succeeded (`*50`, `*51`, `*89`, `*63`, the calling codes `*56`–`*79`) answered a bare 200 with no SDP and never sent a BYE. Making them speak and hang up, as park does, then showed a flow that answers and hangs up at once sent its BYE before the caller's ACK; a phone that took the BYE first never saw the call answered (RFC 3261 §15). | S3 (the phone is left on a call after every feature code) | Fixed in v0.25.444: a feature code done is a short inline flow (`confirm_feature`: "Agent logged in.", "Do not disturb on." …), and a flow's BYE waits for the ACK of its 200 OK, at most 64·T1 (`fcp_sip::ack_wait`). Tests: `calling.rs` `dial_code` and `feature_codes_change_the_callers_own_settings`, `calls.rs` `a_waiting_caller_hears_music_until_an_agent_logs_in_by_name` check the node ends the call. |
| Session 3, the queue: the agent hung up and the caller stayed on the line until they hung up too. The agent's BYE reached the queue on its agent leg, and the queue matched only the caller's Call-ID. Under it, the queue cleared `ringing_agent` when the legs connected, the field that, at the end of a session, sends the agent to wrap-up and names the agent on the record: an agent who took a call stayed on it for good and was never rung again. | S2 (an agent's hangup leaves the caller on, and the agent takes one call per login) | Fixed in v0.25.445: a BYE on a session's agent leg ends the session and sends the caller a BYE; the connected agent stays on the session. Test: `calls.rs` `an_agent_hanging_up_ends_the_callers_call_and_takes_the_next` (two callers, the agent hanging up on each; each half of the fix fails one round). |
| An agent who hangs up the moment they answer a queue call gets 481 to their BYE: the node's leg to them is not yet the call's. Found writing the test above. The window was wider than the moment: the queue saved the agent's dialog only once the legs were connected, after the connect prompt and a recorded queue's announcement. An agent's BYE there found no session (481 on the VVX), and the caller was then connected to no one. A caller hanging up there left the agent on the line and `OnCall` for good, so they were never rung again. | S2 (an agent lost to the queue) | Fixed in v0.25.449: the agent's dialog, ports and answer time are the session's from the 200, and a session ending after the answer sends the agent to wrap-up. Test: `calls.rs` `a_hangup_during_the_announcement_ends_both_parties` (fails on the old code). On the VVX on 2026-10-22: two calls from the menu, the agent hanging up each (the second 2.6 s after answering); the mobile was hung up each time and the agent rung again 4 s later. |
| Session 3, keypad again: the digits a Gather collected kept the key that finished them, so `{{keys}}` was `5551212#`: the keypad test read the pound back, and no transition could name the digits alone. A feature code's PIN prompt already took it off. | S3 | Fixed in v0.25.450: a Gather's digits leave out its `finish_on_key`. Test: `listen.rs` `a_finish_key_is_not_part_of_the_digits` (4, 2, # took the default branch on the old code). |
| Session 3, 933 readiness: a caller with no dispatchable location went out with no callback number. The callback chain (site ELINs, the number's `e911_callback`, the number) ran only once an address was found, so an unlocated call's From was the phone's SIP name (`alice@…`) with no `P-Asserted-Identity`: nothing a PSAP can call back, and nothing a carrier keeping addresses by number (Twilio, the D8 trunk) can look up. | S2 (an unlocated emergency caller cannot be called back) | Fixed in v0.25.451: `e911::callback_for_user` runs the chain without an address. Test: `emergency.rs` `an_unlocated_caller_goes_out_as_their_number` (From `carol@e2e.local` on the old code). |
| That fix overflowed the stack: the site's call manager died (`tokio-rt-worker has overflowed its stack`) on the INVITE `fcp-link`'s `calls_cross_a_sites_link_both_ways` sends up a site's link. Every INVITE is classified for emergency numbers, and the larger location lookup's future, held inline on a deep INVITE path, was one too many for a debug build's worker. | S1 (a node down on a call; release builds have more room but no margin was measured) | Fixed in v0.25.452: the classification and the lookups are boxed (`Box::pin`) at every call. Found by the link suite before the release reached a site; the D8 server ran v0.25.451 for about an hour. |
| Session 3, 933: a test call only ever went to a carrier, whose test service (where there is one) reads back what the carrier holds for the callback number. With several carriers that proves one carrier's records, costs per call, and says nothing about this platform's half: which address it found and why, the callback number, the trunks and how each takes the location. Agreed 2026-10-22 to answer it here by default. | Feature | Done in v0.25.453: `test_call_mode` per tenant (`local`, the default, or `carrier`), migration 120, the API and the Emergency page; in `local` the call manager reads back the address and its source, the callback number, the routes in order with each one's delivery, and the contacts told, and the record says `read_back`; a flow's `Dial` to `933` likewise (`docs/E911.md` "Test calls"). Tests: `emergency.rs` `a_test_call_is_read_back_here_and_no_carrier_is_called`, `a_flow_dialing_the_test_number_is_read_back`. |
| Session 3, 933 on the VVX (2026-10-22): the readback worked (`outcome=read_back`, callback +14255852628, "no location was found"), but it named the trunks to the caller ("First, trunk twilio, with the callback number"), and that no trunk was marked for emergency calls. Which carriers a tenant uses, and in what order, is the administrator's business, not the caller's, and with several carriers it reads as noise. The call ending itself after "Goodbye" was not seen; the caller hung up first (the flow ends with `Hangup`). | S4 (wording) | Fixed in v0.25.454: the readback says only whether a real call could be placed; the routes stay on the readiness page. Tests: `a_test_call_hears_what_a_real_call_would_carry` (no trunk or carrier named), `a_test_call_hears_what_is_missing`. |
| Session 3, 933 again on the VVX (2026-10-23, v0.25.454): the readback sounded right and the node hung up after "Goodbye", but the caller heard 3.7 s of silence after the answer. The whole readback was one prompt, synthesized after the answer (Piper, 43.69 → 47.43 in the log), and its text differs per caller, so no cache could hold it. | S3 (dead air on an emergency test call) | Fixed in v0.25.455: the readback is a prompt per sentence or two (`test_readback` returns the parts, an inline flow says each in turn); the opening is the same for everyone and is warmed when the call manager starts (`emergency::warm_test_opening`), and the rest is synthesized while it plays (`TtsService::warm`). Two requests for the same text now share one synthesis instead of rewriting the cached file under a playback. Tests: `emergency.rs` `a_test_call_hears_the_readback_at_the_answer` (a 2 s voice: heard within 1.5 s and over in 7.4 s; without the warm-up nothing is heard in time), `tts.rs` `a_warmed_text_is_synthesized_once`. |
| Session 3, 933 (2026-10-23): "No emergency contacts are set" — the D8 tenant had none, and the D8 events server ran no notifier. Asked for: contacts per site as well as per tenant, several per site, and security able to listen in. A site had one `emergency_contact_name`/`_phone` that nothing ever told; the compose stack shipped no `[notifications]`, so no compose deployment told anybody. | Feature (and S2: Kari's Law notification off in every compose deployment) | Done in v0.25.456: a site's `emergency_contacts` and `notify_tenant_contacts` (migration 121, the old pair made its first contact), told first by the notifier (`contacts_for_site`, the event's `location_id`); `listen` on a call contact, offered after the notice (1 listen, 2 listen and speak), joined by a re-INVITE as the call's supervisor (`Supervisions::join_dialog`); every emergency call anchored; a 933 readback waiting a minute for a listener; readiness per site; the Locations form and the Emergency page; compose ships the call channel (`FCP_EVENTS_NOTIFY_CALL_TOKEN`). Tests: `emergency.rs` `a_sites_contact_is_told_and_listens_in`, `a_contact_listens_to_a_live_call_without_being_heard`; `delivery.rs` `a_sites_contacts_are_told_and_the_tenants_unless_it_says_not`; `numbers.rs` `a_site_keeps_its_own_emergency_contacts`; the UI's `a location is made with its emergency contacts`. The D8 events server got the call channel by hand first. |
| Session 3, Anvil v0.2.1 on Windows (2026-10-23): signed in as bob, a call to the VVX connected with no audio either way. Anvil named `127.0.0.1` in its SDP and in its REGISTER's Contact (an FCP account carries no STUN server or bind address, and loopback was its default); the call manager sent bob's audio to its own loopback and would have sent calls to bob there too. FCP had no NAT handling for phones at all: a UDP phone naming a LAN address was unreachable, and only a site's link latched its RTP. | S1 (no audio and no incoming calls for any phone behind NAT naming a local address) | Fixed in v0.25.457: `fcp_sip::nat` — a UDP phone behind NAT without a flow is rung where it registered from (`reachable_contact` in the call lookups; the registrar keeps and echoes the Contact as sent), its requests' Contact is made the address it was seen at (`fix_contact` in `handle_inbound`, not for REGISTER), a caller behind NAT is anchored (`caller_behind_nat`) and each leg behind NAT has its RTP learned from the address its SIP came from (`latch_leg`). Anvil (v0.2.2, anvil #35) names the address of its route to the registrar. Tests: `calls.rs` `phones_behind_nat_ring_hear_each_other_and_hang_up` (the testkit's `Phone::behind_nat`; each of the three parts fails it when taken out), `nat::tests`, `decision.rs` `a_caller_behind_nat_is_anchored`. |
| Session 3, Anvil v0.2.2 and the VVX behind one router with its SIP ALG off (2026-10-24): no audio either way. Both phones named their LAN addresses, so both legs learned their peers from the first packet from the shared public address (v0.25.457) — and the session had one RTP port for both legs, so forge-engine told a packet's leg by its source alone. The VVX's packet came first and was taken as leg A's: relaying still reached the right phones, but transcoding ran backwards (the VVX's G.711 decoded as Opus, passed through raw, dropped by Anvil). Every SBC and media relay gives each leg its own port. | S1 (no audio for two phones behind one public address, with different codecs; music and keys to the wrong phone with the same codec) | Fixed in v0.25.459 (forge-media 2026-10-24, forge-engine 0.8.0): a socket pair per leg — a packet's leg is the socket it arrived on, media to a leg leaves from its own socket, park moves each port with its phone (`swap_leg_sockets`); a B leg's first encrypted packet was decrypted with A's keys and dropped, RTCP's leg was guessed by port + 1 — both gone with it. Design `docs/NAT_AND_TLS.md` (phases N1, N2). Tests: `calls.rs` `two_phones_behind_one_address_hear_each_other` and `…_with_an_alg_…` (two phones on 127.0.0.2, PCMA and PCMU, tones both ways, hold music to the held phone), forge-engine `two_phones_behind_one_address_each_hear_the_other_from_their_own_port`, `a_swap_moves_each_port_with_its_phone`, `an_encrypted_leg_is_learned_from_its_first_packet`. |
| Session 3, the same two phones with the router's SIP ALG on: the VVX heard Anvil, Anvil heard nothing. The ALG rewrote both phones' Contact and SDP to the public address; the node sent valid Opus to exactly the address and port Anvil sent from. | S1? | Not explained from the server: Anvil sent no RTCP and kept no log. Anvil v0.2.3 (anvil #36) counts what it receives and drops, and writes `anvil.log`; the next ALG-on call says where the audio stops. TLS by default (`docs/NAT_AND_TLS.md` T1–T2) takes signalling out of an ALG's reach. |
| Session 3: a transcoded packet kept the sender's RTP timestamp, so G.711's 160 ticks per 20 ms reached an Opus peer whose clock wants 960 (and Opus's 960 reached the VVX as 120 ms per packet). Anvil and the VVX played it; a phone timing playout by it would not. | S3 | Fixed in v0.25.459 (forge-engine 0.8.0): the timestamp is moved to the receiver's codec clock, exactly (`TimestampMap`). Tests: forge-engine `a_transcoded_timestamp_*`, fcp-media `an_opus_leg_is_transcoded_to_a_pcmu_leg` (960 between packets). |
| Session 3: Anvil 0.2.2 on Windows could not sign in — "FCP could not be reached: error sending request" — while a browser on the same machine fetched the same URL. Bitdefender's firewall blocked the upgraded, unsigned executable; the proxy log had no request from it. Anvil kept only reqwest's top line, not the cause. | S4 (Anvil) | Not FCP's. To do in Anvil: the error's source chain in the message (`anvil-fcp/src/lib.rs:63`). |
| Session 3: after reinstalling Anvil, bob had two devices and two bindings, the old one naming `127.0.0.1`. Anvil's uninstaller deletes `session.json`, which holds the install id, so the reinstall signed in as a new install with a new device and SIP account; the old one was never told. Anvil also sends no REGISTER with `Expires: 0` when it quits. | S3 (Anvil; an orphaned device with live SIP credentials) | Open. Deleted by hand on D8. |
| Session 3: the Devices tab showed no firmware for Anvil, though its REGISTER says `Anvil/0.2.2`; the User-Agent reader knew only the provisioned vendors. | S4 | Fixed in v0.25.460: `provisioning::identify` reads `Anvil/<release>` as model Anvil, firmware the release, written at each registration. Test: `provisioning::tests` (identify). |
| Session 3, Anvil v0.2.9 and the VVX (2026-10-26): the VVX held and resumed; on resume Anvil heard white noise. The VVX had answered PCMU but offered G.722 first in its resume. Under hold music the node answers a hold and its resume itself (`answer_hold_locally`): it answered in the offer's first codec, G.722, and updated the media session only when the phone's address moved, so the session went on decoding the VVX's G.722 as PCMU into Anvil's Opus. Holding from Anvil was fine (its offer's first codec is its call's). | S2 (noise for the rest of a transcoded call after a resume from a phone that reorders its codecs) | Fixed in v0.25.463: the node answers a hold or resume in the codec the leg uses (`hold::keeping_codec`, the first codec of the leg's SDP, kept with telephone-event), and moves the session to the answer's codec when the offer no longer has it. Tests: `calls.rs` `a_resume_offering_another_codec_first_keeps_the_call_s_codec` (fails without the fix: answered PCMA), `hold::tests`. |
| Session 3, park from Anvil v0.2.10 (2026-10-26): parked, but "Parked on 7 0 1." was never heard. The node played it into the `*70` call, and Anvil's log showed all 89 packets received and played — into a new output stream on a Sonos speaker, opened as the held call's closed: a network speaker wakes for a second or two and swallowed the whole prompt. On the PC's own speakers it was heard. A phone also had no way to show the orbit. | S3 (the orbit lost on a slow speaker; nothing to show) | Fixed in v0.25.464: `*70`'s answer carries `X-FCP-Parked: <orbit>` for a phone to show; Anvil v0.2.11 shows it and keeps one output stream across calls. Test: `calling.rs` `a_caller_parking_with_the_code_leaves_the_parked_party_only_the_music` checks the header. |
| Session 3, finding 15 (2026-10-26): the VVX called Bob, Anvil declined, and the VVX heard ringing on, then a fast busy instead of voicemail. FCP took a 603 from one of Bob's phones as one refusal among the fork and kept ringing his others — stale Anvil bindings (finding 17) — to the ring time; RFC 3261 §16.7 makes a 6xx the end of the fork. With no mailbox (finding 16) the busy treatment was a 486. | S2 (a decline rings on; the caller waits for nothing) | Fixed in v0.25.465: a 6xx ends the race, the other phones are CANCELled and the busy treatment runs at once. Test: `calling.rs` `a_decline_on_one_phone_stops_the_others_and_is_busy` (fails without the fix). |
| Session 3, finding 16: Bob, with extension 1002 and no line, could not be given a mailbox: the API required a line binding, though calls find a mailbox by its user and it answers the user's extension; the line was only stored. | S2 (no voicemail for a user without a line) | Fixed in v0.25.465: `line_binding_id` optional on `POST /voicemail/mailboxes`, the admin UI's Line field "None — the user's extension". Test: `api_postgres.rs` `voicemail_mailboxes_crud`. |
| Session 3, finding 17: a signed-out Anvil stayed on the Devices page, with several registrations, some expired. Signing out ended the app session and left the install's device and its bindings; Anvil sent no REGISTER with `Expires: 0` when it stopped. | S3 (stale devices and bindings; calls forked to dead contacts) | Fixed in v0.25.465: `/auth/app/revoke` removes the install's device (`remove_device`: sessions, registrations, the device); Anvil v0.2.12 un-registers when it stops. Test: `api_postgres.rs` `an_app_signs_in_through_the_browser_and_keeps_its_session`. |
| Session 3, finding 20 (2026-10-26): a user should be able to record a call from their app, when an administrator lets them, with the recording announcement played. FCP's recording routes were administrators' only (`call_control`), and an app was not told the id of a call it placed. | S3 (a new feature) | Fixed in v0.25.466 (`docs/RECORD_AND_CONTACTS.md` §2): `media_preferences.record_on_demand`, set on the user dialog beside "Record every call" (`PUT /users` now takes `media_preferences`); `POST /me/calls/{call_id}/recording/{start,stop}` put to the node with `for_user`; the node checks the user is on the call, records with the source `on_demand` through the consent announcement, and stops only what that user started; the caller's 200 OK names the call (`X-FCP-Call-Id`); `may_record` on `/me/softphone`. Tests: `calls.rs` `a_party_records_their_own_call_and_only_they_stop_it`, `api_postgres` `a_user_records_their_own_call_once_an_administrator_lets_them`. Anvil's button is v0.2.14. |
| Session 3, finding 21 (2026-10-26): a "More" menu with Create contact, for contacts kept on the device and on the server. FCP kept no contacts of a user's own, only the tenant's directory. | S3 (a new feature) | Fixed in v0.25.467 (`docs/RECORD_AND_CONTACTS.md` §3): `personal_contacts` (migration 122) and `/me/contacts` (made, paged and searched by name, changed, deleted to a tombstone); subject export and erasure; synced both ways with the user's site — down by the journal, up by the link's shipping, each side taking only a strictly newer copy, so an echo is let go. Tests: `api_postgres` `a_user_keeps_their_own_contacts`, `fcp-link/tests/contacts.rs`. Anvil v0.2.14 (anvil #48) has the More menu and contacts on the device; Anvil's use of the server's contacts is phase 4. |
| Session 3, finding 34 (2026-10-29): the Cluster page listed one node, `cm-docker-01`, while eighteen containers ran. It lists cluster members, and membership is opt-in (`[cluster] enabled`), set only in the call manager's compose config; jobs, the API gateway, the registrar, the link and the admin cannot join at all; a process that stops simply disappears when its key expires; and `/ready` checks only draining. Asked for: a services view of every running process with its health, under a System menu with the cluster. | S3 (an operator cannot see what runs or what is down) | Designed in `docs/SERVICES.md`: a service beacon from every process, apart from membership; the admin's kept record with up/down events; a System section with a Services page; phases S1–S3; S1 done in v0.25.476 (every binary writes `fcp:service:<id>`; `/ready` answers from its required checks); S2 in v0.25.477 (the admin's record, up/down events, sites' services, `/api/v1/system/services`, metrics); S3 in v0.25.478 (the System menu, the Services page, the dashboard tile; the cluster routes made platform-only). |
| Session 3, finding 35 (2026-10-30): no way to manage parking or set a park range. The range is each call manager's TOML (`[pbx.parking]`, 701–720), shared by every tenant: tenant B's `*71701` takes tenant A's call. Each node allocates orbits from its own map and writes `park:{orbit}` with a plain SET, so two nodes parking at once take the same orbit and the second replaces the first. | S2 | Designed in `docs/PARKING.md` (agreed: lots per tenant, each with its own range). Phase 1 v0.25.483: lots per tenant, slots claimed per tenant with `SET NX`. Phase 2 v0.25.484: dialed slots, directed park, timeout fallback, parked calls listed and sent on. Phase 3 v0.25.485: the Parking page, slot lamps, park keys as lamps. Phase 4 v0.25.486: lots synced to sites, sites' parked calls on the cloud's board — all four phases done. |
| Session 3, finding 36: the audit log could be listed but not exported. | S3 | v0.25.481: `GET /audit-log/export` (CSV, JSON Lines, the listing's filters, `until` added), the page's export buttons. |
| Session 3, findings 37–38: Alice called Bob at 1002; declined, or unanswered until the ring time, voicemail never answered. The node forwarded `sip:1002@voicemail;reason=…`, and the voicemail server routed a mailbox by its extension (Bob's is `bob`), its owner's SIP name and the extensions dialing a *number* — never an extension dialing a *user*. Its voicemail record was `internal_error` in 0 s. It also routed only the default tenant's mailboxes at startup. | S2 | v0.25.480: forwarded by the mailbox's id; the server routes every name a mailbox is reached by, for every tenant. Test: `a_mailbox_with_no_line_answers_its_id_and_its_owners_extension`, which fails before the fix as on the D8 server. |
| Session 3, finding 38a: when the call timed out, Bob's Anvil still showed the incoming call. The node CANCELs every unanswered leg on the fork's timeout, on the INVITE's own transport; the capture of that minute had rotated out. | S3 | To repeat on v0.25.480 with the capture running. |
| Session 3, finding 39: Bob called Alice; voicemail answered, no message lamp on her VVX. The rolling capture showed ~40,000 `message-summary` SUBSCRIBEs a day from the VVX, every one 401 `stale=true`: a refresh inside a subscription the node lost on restart, challenged as new, and the VVX resent it on its old nonce for ever. | S2 | v0.25.480: a refresh for no subscription here is 481 (RFC 6665 §4.2.1.2), so the phone subscribes afresh; mwi.rs tests it. |
| Session 3, finding 40 (Anvil): a line's label, e.g. `1001 - Bob`. | S3 | Designed in `docs/DEVICE_LINES.md` (agreed: the admin's label, a user's rename only where the tenant allows it). Phase 1a v0.25.487: labels, the rename switch, `/me/lines/{line}/label`. |
| Session 3, finding 42 (Anvil): 8–10 unique or shared lines. | S3 | Designed in `docs/DEVICE_LINES.md` (agreed: lines as on a desk phone — own, shared, on behalf — up to ten). FCP's side v0.25.487–488 (lines, labels, `X-FCP-Line`, MWI per line); Anvil's next. |
| Session 3, finding 43 (Anvil): packets sent and received and quality figures during a call. | S3 | Designed in `docs/CALL_DIAGNOSTICS.md`. |
| Session 3, finding 44 (Anvil): a packet capture of a call, off by default, an admin enabling it. | S3 | Designed in `docs/CALL_DIAGNOSTICS.md` (agreed: an admin allows, the user captures, kept locally and uploaded to FCP). |
| The VVX uploads its `app.log` every minute to the provisioning URL and is refused (`not_a_device_file`). | S4 | To look at. |
| The VVX answers the presence NOTIFYs to its SUBSCRIBE with 500 Internal Server Error. | S3? | To look at. |
| The rolling SIP capture on the D8 server (`d8-sipcap`) had stopped on 2026-10-18 at 23:27 and its transient unit was gone. | — | Restarted 2026-10-20 (`systemd-run --unit d8-sipcap -p Restart=always`). |
| A trunk's health is probed by the conference, IVR and voicemail servers as well as the call manager. | S4 | #336. |
| The voicemail menu's flow runs on after the caller hangs up, asking for digits from a session that is gone. The D8 test menu does the same (2026-10-22: three rounds of "Session not found" after the hangup). | S3 | #337. |
| The VVX never answers the PRACK the node sends for its reliable 180 (retransmitted for the whole ring). The capture showed why: the PRACK's Via was `SIP/2.0/UDP placeholder`, the helper's stand-in, never filled in for the PRACK siphon-rs's INVITE transaction sends itself, so the VVX answered it to a host that does not exist. | S3 | Fixed in v0.25.446 (siphon-rs `fix/prack-cseq` 7c8d7fb): the PRACK takes the INVITE's Via with its own branch. Test: `sipp.rs` `the_node_pracks_a_callee_that_rings_reliably` checks the PRACK SIPp receives names no placeholder. On the VVX on 2026-10-22 it answers the PRACK 200 in about 60 ms. The flow's `Dial` node had the same fault more widely: its INVITE, ACK, PRACK and BYE are built by the helper and handed to the transaction manager as they are, so every leg a flow dialed (to a phone, a trunk, or back into the node for a queue) said `placeholder`; peers that answer to the source address hid it. Fixed in v0.25.448 (`flow_handler::with_own_via`). Test: `emergency.rs` `a_flow_dialing_911_places_an_emergency_call` checks the INVITE the carrier receives. |
| A VVX offers G.722 first. The node answering its own flow (`*98`, `*88`, a mailbox's flow) sent its configured codecs in its own order as the answer — PCMU first — while the media session took the offer's first, G.722, and encoded the prompts in it: the VVX decoded G.722 as PCMU, and every prompt was static. The 200 OK also carried no Contact. | S2 (every node-answered prompt is noise on a wideband phone) | Fixed in v0.25.431: the answer is the media session's own (the offer's order, the codec it sends in first), and the 200 OK has a Contact. Test: `sipp.rs` `a_wideband_phone_is_answered_in_the_codec_the_node_sends`, failing on the old answer. |

**Q-D8a. Text to speech in the default stack.** Voicemail retrieval,
the default greeting, IVR prompts and hoteling's prompts are spoken by
text to speech; the compose stack runs `fcp-tts` only under its `tts`
profile because the image carries PyTorch (several GB). Off, they are
silence. Ship it on, ship a small engine (Piper) as the default, or ship
recorded prompts for what is fixed and accept silence for what is not?
*Answered 2026-10-19:* on by default, with Piper the default engine and
what exists today (Kokoro, the cloud voices) enabled by settings.
