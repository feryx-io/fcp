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
That is an estimate, and §5's D1 makes it exact. The groups:

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
