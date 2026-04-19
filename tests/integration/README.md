# Integration Tests

This directory is reserved for cross-crate, cross-service integration tests that
require real infrastructure (Postgres, Redis, a running SIP stack) rather than
in-process stubs.

## Current state

No runnable integration tests exist here yet. Unit tests in the individual
crates cover most of the pure logic.

## Known coverage gaps

### Device auto-provisioning (`GET /api/v1/provisioning/:mac_address`)

Per-device and deployment provisioning tokens plus MAC uniqueness landed in
migration 046 (`046_device_provisioning_tokens.sql`). The pure auth helpers are
unit-tested in
`crates/fcp-api-core/src/v1/provisioning.rs`, but the full Axum handler path is
not. An end-to-end test should construct an `ApiState` against an ephemeral
Postgres instance and verify:

1. Missing / invalid `X-FCP-Provisioning-Token` returns 404 **without** hitting
   the devices table (locks in the shared-token gate).
2. Unknown MAC returns 404 after the shared-token check. Response timing must
   be indistinguishable from the "found but wrong per-device token" case
   (locks in the timing-attack fix in
   `provisioning.rs::auto_provision`).
3. Disabled device with correct tokens returns 404 (locks in the ordering:
   `authorize_device_provisioning_token` runs before the `enabled` check).
4. MAC normalization collision: creating a second device with the same
   normalized MAC (e.g. `AA:BB:CC:DD:EE:FF` vs `aa-bb-cc-dd-ee-ff`) rejects
   with 409 at the API layer and fails the unique index at the DB layer.
5. `list_devices_by_user` returns the `has_provisioning_token` flag correctly
   (regression guard for the silent-drop bug fixed in v0.25.49).
6. Successful provisioning returns `Cache-Control: no-store` and renders the
   template with the device's credentials.

The main blocker is that `ApiState::new()` requires a populated
`SipMessageHandler`, `CallManager`, `DialogManager`, and `SubscriptionManager`,
which in turn need a running SIP transport layer. A lightweight test harness
(e.g. a `testkit::ApiHarness` builder that stubs out the SIP layer behind
trait objects) would unlock these tests.

## When adding tests here

- Cargo test targets in this directory should be gated behind a feature or env
  var so they don't run on every `cargo test --all`. Integration runs belong
  in CI with real services stood up.
- Prefer testcontainers-rs over assuming a host-installed Postgres.
