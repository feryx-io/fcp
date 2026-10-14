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

Still to come in D2:
- SIPp against the SBC;
- an attended transfer (REFER with Replaces);
- a REGISTER and INVITE flood against admission;
- baresip for media.

