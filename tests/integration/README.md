# Integration Tests

This directory is reserved for cross-crate, cross-service integration tests that
require real infrastructure (Postgres, Redis, a running SIP stack) rather than
in-process stubs.

## Current state

End-to-end SIP + media tests against the running binaries — the test
floor, `docs/TEST_FLOOR.md` — live next to each binary on the same
gating as the smoke tests: `crates/fcp-call-manager/tests/calls.rs`
(T1: registration, a heard call, hang-up either side, the CDR; T2: busy, decline, no answer, an unregistered callee, hold and resume, DTMF both ways, blind transfer) so far.
The scripted phone, the seed and the service runner are in
`fcp-testkit`.

Database-backed tests live next to the crates they exercise and are gated on
`FCP_TEST_DATABASE_URL` (CI runs them against a `postgres:16` service):

- `crates/fcp-storage-postgres/tests/integration.rs` — the storage layer
  (see that crate's README).
- `crates/fcp-api-core/tests/api_postgres.rs` — REST handlers through the
  full router built by `fcp_api_core::create_router` (the same builder
  fcp-admin serves): bearer middleware, platform-admin tenant override
  (`x-fcp-tenant-id`), JWT login/refresh/lockout and role gating, database
  API tokens, users, devices, config templates, device auto-provisioning,
  CDRs/usage/billing summary, rate plans and the balance ledger, trunks
  (Carrier vs Enterprise deployment modes), queue configs and the queue
  runtime (live `QueueManager`), voicemail mailboxes, flows and flow traces,
  registrations, the presence runtime (live `PresenceManager`), the
  self-service `/me` routes, the `/events` WebSocket first-message auth
  (served on a real loopback socket), the multipart uploads (avatars,
  voicemail greetings, audio prompts), SIP traces, the `/debug` runtime
  views (real dialogs, subscriptions, and calls inserted into the managers),
  the E911 admin API (config, readiness, address validation, audit
  trail), and the compliance basics (retention policy and runs, admin
  audit log, CSV export).
  The `ApiState` it needs is built on a
  no-op SIP transport dispatcher (no sockets, no SIP traffic), so no separate
  harness crate was required.

- `crates/<binary>/tests/smoke.rs` (all 14 service binaries) — built on
  `crates/fcp-testkit` (`fcp_testkit::smoke`): `--version`/`--help`, a
  malformed config file is rejected, and the service starts on free loopback
  ports, answers its health endpoint, and exits cleanly on SIGTERM. Binaries
  that need a database run against `FCP_TEST_DATABASE_URL`; `fcp-jobs` needs
  `FCP_TEST_REDIS_URL` (a `redis:7` service in CI). Both skip when unset.

Nothing runnable lives in this directory yet; cross-service tests (API + SIP
+ media) are still a gap.

## Known coverage gaps

### Device auto-provisioning (`GET /api/v1/provisioning/:mac_address`)

Covered by `crates/fcp-api-core/tests/api_postgres.rs`:

1. Missing / invalid shared provisioning token, the admin API token used in
   its place, and plain-HTTP requests all return the uniform 404.
2. Unknown MAC, wrong or missing per-device token, disabled device, and a
   device without a template return the same 404 with a byte-identical body.
   Response *timing* is not asserted (too noisy for CI); the ordering that
   makes the cases indistinguishable is locked in by the handler's structure.
3. MAC normalization collision (`AA:BB:CC:DD:EE:FF` vs `aa-bb-cc-dd-ee-ff`)
   rejects with 409 at the API layer; the DB-level unique index is covered by
   the storage suite.
4. `list_devices_by_user` keeps the `provisioning_token_hash` (regression
   guard for the silent-drop bug fixed in v0.25.49).
5. Successful provisioning returns `Cache-Control: no-store` and renders the
   owner's credentials, including the HA1 read back through the sealed
   column.

Still uncovered: TTS synthesis endpoints (they need a TTS provider) and
phone-number porting (no API exists yet; only the `PortingStore` model).
Subscriptions in `/debug` are visible platform-wide only: SUBSCRIBE dialogs
carry no tenant, so tenant-scoped callers see none.

## When adding tests here

- Cargo test targets in this directory should be gated behind a feature or env
  var so they don't run on every `cargo test --all`. Integration runs belong
  in CI with real services stood up.
- Prefer testcontainers-rs over assuming a host-installed Postgres.
