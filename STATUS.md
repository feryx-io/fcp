# FCP Status

**The single source of truth for what is done, partial, or absent.**
Update this file when feature status changes; keep other docs describing scope, not status.

- **Last reviewed:** 2026-09-01 (full code audit against ARCHITECTURE-v0.2.md)
- **Version:** v0.25.81 · 37 workspace crates
- **Submodules:** siphon-rs `v2026.08.25` · forge-media `v2026.09.01.1` (both at their repos' current release tips)
- **Health:** CI green (fmt, clippy `-D warnings`, tests, cargo-audit) · 0 Dependabot alerts · 0 future-incompat warnings

## Phase scoreboard

| Phase | Status | Notes |
|---|---|---|
| 1 — MVP foundation | ✅ Complete | Core types, siphon-rs stack, B2BUA, call-manager, REST API, Postgres, Docker |
| 2 — Programmable voice | ✅ Complete | Flow engine, IVR, voicemail, webhooks, events, admin UI, DB-backed flow store, TTS service |
| 3 — Clustering & apps | ✅ Complete* | SWIM cluster, queue, presence, conference, WebRTC, Redis. *Lua plugin runtime and gRPC API from the original Phase 3 scope were never built (see Descoped/undecided) |
| 4 — Enterprise | ⏳ In progress | Detailed audit below |
| 5 — Carrier grade | 📋 Planned | Multi-region, E911, video, compliance — data-model stubs at most |

## Phase 4 feature audit (verified against code, 2026-09-01)

| Feature | Status | Evidence |
|---|---|---|
| STIR/SHAKEN | ✅ Done | ~1,900 lines in `fcp-sbc-core`/`fcp-sbc`: ES256 sign + verify, cert cache, enforcement-pipeline wiring, `/stir-shaken` admin route, ~25 tests |
| SBC enforcement + trunks | ✅ Done | `fcp-sbc-core` / `fcp-sbc` |
| Policy-based routing + LCR | ✅ Done | `fcp-routing` (rule evaluator, ~1,000 lines) |
| Provisioning — serving layer | ✅ Done | `fcp-api-core/src/v1/provisioning.rs`: token auth (constant-time), HTTPS enforcement, template rendering, migrations 045/046 |
| TTS service | ✅ Done | `fcp-tts-server`: synthesize/voices/providers/cache routes, Redis feature-gating |
| Audio transcoding | ✅ Done | B2BUA/SBC path wired (2026-09-01): `fcp-media` rebuilds the payload-type map from each leg's negotiated SDP and arms forge-engine's per-packet transcoders on codec mismatch, in both single-session and cross-wired bridge modes; queue path already had it. Transcodable set: PCMU/PCMA/Opus/PCM (Opus needs the `opus` feature; G722/G729 pass through untranscoded). Gated by `media.enable_transcoding` (default off). `RemoteBackend` stubs now return errors instead of panicking (remote mode remains unimplemented) |
| Provisioning — vendor templates | ✅ Done (2026-09-01) | Built-in starter templates for Yealink, Polycom, Grandstream, Cisco ship in `fcp-core/src/config_template.rs`; tenants browse them at `GET /v1/config-templates/library` and instantiate a customizable copy via `POST /v1/config-templates/library/:vendor` (never overwrites an existing copy) |
| Policy engine (`fcp-policy`) | ✅ Extracted (2026-09-01) | Dedicated crate: rule model, evaluator, YAML store with hot reload. `fcp-routing` re-exports everything (no consumer changes). Extraction also fixed the ignored `TimeWindow.timezone` (rules now evaluate in their own timezone, overnight windows supported) and added per-tenant feature toggles (`require_features` on rules + `feature_toggles` in YAML). SBC admission stays in `fcp-sbc-core` (it is rate/capacity control, a different concern); number manipulation remains future (SBC transform stage) |
| Billing | ✅ Done (2026-09-01) | Dedicated `fcp-billing` crate (rating, balance admission, settlement, hooks; model stays in `fcp-core/src/billing.rs`). Rate plans + longest-prefix rates (migration 047, `/v1/rate-plans`); answered calls rated and **settled** at CDR creation (atomic debit, one charge per call). **Balance enforcement** per tenant (migration 048, `/v1/billing/balance`): 402 at admission when the first billing increment is unaffordable, mid-call cut-off when the affordable duration runs out, fail-open on storage errors. **External hooks**: `billing.*` platform events (`call_rated`, `balance_changed`, `balance_low`, `balance_depleted`, `call_rejected`, `call_cut_off`) → webhooks/WebSocket, plus the `BillingHook` trait for in-process integrations; credits/adjustments via `/v1/billing/balance/transactions`. See `docs/BILLING.md`. Not yet: multi-currency, sub-balances, invoicing |
| SBC pipeline stages 3–5 | ✅ Done (2026-09-01) | **Reachability**: classified route resolution (`RouteResolution`) — unroutable destinations now get 404 (nothing configured) vs 503 (trunks exist but unavailable) instead of generic 480/500. **Routing**: trunk selection/failover/LB already existed (`TrunkRouter` + LCR); fixed a concurrency-accounting leak that recorded every failover candidate as a started call. **Transform**: headers via HMR (pre-existing) + new per-trunk egress number transforms (`egress_number_transform`: strip prefixes / prepend, applied at route build) |
| Encryption at rest / KMS | ✅ Done (2026-09-01) | `fcp-crypto` crate: AES-256-GCM field cipher, envelope-wrapped DEKs (`data_encryption_keys`, migration 049), KEK providers `local` (env master key) and `aws-kms` (feature-gated, GenerateDataKey/Decrypt). Postgres backend seals SIP digest/HA1/provisioning passwords, trunk passwords, conference PINs and webhook secrets; legacy plaintext reads through, writes always seal. Platform-admin API: status / rotate / reencrypt at `/v1/system/encryption`. See `docs/ENCRYPTION.md`. Not yet: rewrapping DEKs under a new KEK, media-file encryption |

## Phase 5 reality check

| Item | Status |
|---|---|
| Multi-region | ❌ `fcp-cluster` is flat single-region SWIM membership |
| E911 | 🟡 Block 1 done (2026-09-02, `docs/E911.md`): emergency-number detection (tenant-configurable, `911`/`933` + `9` prefix by default) before any admission control, dispatchable-location resolution (device override → device site → user site → DID address) with ELIN callback numbers per site, emergency-capable trunks tried first, callback number as caller ID + `P-Asserted-Identity` + `Priority: emergency`, `call.emergency` event with notification contacts, `emergency_calls` audit table, admin API (config, readiness report, validate-address, per-number validation) and a pluggable `E911AddressValidator`. Not yet: PIDF-LO location by value, subnet-based location for softphones, provider-backed validation, in-platform notification delivery |
| Video conferencing | ❌ Conference server and WebRTC gateway are audio-only |
| Regulatory compliance | 🟡 Basics done (2026-09-02, `docs/COMPLIANCE.md`): per-tenant retention policy (CDRs, SIP traces, flow traces, recordings, audit events) applied hourly by fcp-admin and on demand, with recorded runs; recording-consent mode with a prompt played before API-started recordings; admin audit log written by an API middleware (every mutating request + exports, actor/tenant/route/status) with `GET /audit-log`; CSV CDR export. Not yet: consent announcement on B2BUA auto-recorded calls, legal hold, data-subject export/erasure |
| Distributed conferencing | 📋 Ready plan in `docs/TO_DO_CONFERENCE_LOADBALANCE_HA.md` (~2–3 weeks) |

## Phase 5 gate decisions (made 2026-09-02)

| Item | Decision | Rationale |
|---|---|---|
| Storage strategy | **Postgres stays the system of record. SQLite is dropped.** CockroachDB support comes as a compatibility pass on the existing backend (same wire protocol) plus a CI service, scheduled as the first step of multi-region work, not before | A second dialect backend would be ~7k lines to maintain for single-box deployments that docker-compose already covers. Only ~12 of 45 migrations use Postgres-specific features (triggers, PL/pgSQL, GIN), all supported by current CockroachDB |
| Plugin runtime (WASM/Lua) + `fcp-plugins` SDK | **Descoped** | Flows, webhooks, and the `BillingHook`/policy traits are the extension points. Revisit only on a concrete customer need |
| gRPC API (`fcp-api-grpc`) | **Descoped** | REST (~150 routes) + WebSocket cover the surface; a gRPC mirror would be permanent double maintenance |
| Distributed conferencing (cascaded mixing) | **Kept, scheduled after E911 and compliance basics** | Plan on file in `docs/TO_DO_CONFERENCE_LOADBALANCE_HA.md` (~2–3 weeks). Dispatchable-location and compliance items are legal requirements for US enterprise UC; 1000-participant conferences are a scale feature |
| Dedicated CDR-processor pipeline | **Kept, deferred** | Inline CDR creation + `fcp-jobs` post-processing is sufficient until carrier call volumes; keep on the roadmap as a Phase 5 carrier-scale item |
| Standalone proxy worker | **Descoped** | Registrar, B2BUA, and SBC binaries cover signaling; a pure proxy only serves load-distribution topologies the SBC can already front |

Vision items in `docs/ARCHITECTURE-v0.2.md` that are descoped above remain in that document as history; this table is authoritative.

## Known weak spots

- **Test coverage** outside the SIP stack is thin: the Postgres backend and the core REST handlers (users, devices, templates, provisioning) now have database-backed tests, and every binary now has a start/health/SIGTERM smoke test, but the binaries' actual call paths (SIP + media end to end) still have no automated coverage; the REST surface is covered except for TTS synthesis (needs a provider) and a porting API that does not exist yet. Workspace `TODO`/`todo!()` count is low (~45, mostly `fcp-media`'s RemoteBackend), so the debt is coverage, not stubs.
- `proc-macro-error2`/`redis 0.25` future-incompat warnings were cleared in v0.25.63.

## Next steps (agreed 2026-09-01)

1. ~~Documentation truth pass~~ (done 2026-09-01)
2. ~~Finish transcoding path~~ (done 2026-09-01: PT map + transcoder arming wired through `fcp-media`; `RemoteBackend` stubs return errors)
3. ~~Billing MVP~~ (done 2026-09-01: rate plans + rating + CDR cost stamping + usage cost) → ~~billing follow-ups~~ (done 2026-09-01: `fcp-billing` crate, balance enforcement with 402 + cut-off, `billing.*` events/hooks, ledger API)
4. ~~Ship vendor provisioning templates~~ (done 2026-09-01: built-in library + instantiate API, rather than migration seeding — future tenants get them too)
5. ~~Extract `fcp-policy`~~ (done 2026-09-01: policy engine in its own crate + timezone fix + tenant feature toggles; admission deliberately left in fcp-sbc-core as capacity control)
6. ~~SBC pipeline stages 3–5~~ (done 2026-09-01: reachability classification, routing accounting fix, per-trunk number transforms)
7. Parallel: raise the test floor — ~~containerized Postgres integration tests~~ (done 2026-09-01: `fcp-storage-postgres/tests/integration.rs` runs against a Postgres service in CI, gated on `FCP_TEST_DATABASE_URL`; first run caught two production bugs — `query_cdrs` emitted unnumbered `$` placeholders so every filtered CDR query failed, and `aggregate_usage*` could not decode `SUM(bigint)` (NUMERIC) so usage/billing summaries failed for any tenant with answered calls — both fixed in v0.25.71); → ~~API handler tests against Postgres~~ (done 2026-09-02: `fcp-api-core/tests/api_postgres.rs` drives the full Axum router — bearer middleware, platform-admin tenant override, users, devices, config templates, and the device auto-provisioning endpoint including its uniform-404 failure surface — against the same CI Postgres service; the SIP side of `ApiState` is built on a no-op transport dispatcher, which turned out to need no new harness); → ~~binary smoke tests~~ (done 2026-09-02: every service binary has `tests/smoke.rs` on the new `fcp-testkit` harness — `--version`/`--help`, malformed-config rejection, and start → `/health` 200 → SIGTERM → clean exit on free loopback ports, DB-backed ones against the CI Postgres service, `fcp-jobs` against a Redis service. First run caught five startup bugs: `fcp-registrar` panicked on every start (rustls had no `CryptoProvider` because the tree links both ring and aws-lc-rs — now `fcp-sip` installs one before building TLS configs), `fcp-sbc` could not load any config file (flattened `SbcConfig` never matched the documented `[core]` layout and sub-configs had no serde defaults, so `fcp-sbc-local.toml` failed to parse), `fcp-admin` and `fcp-api-gateway` died on SIGTERM instead of shutting down (Ctrl+C only), and `fcp-presence` had no `--version`); → ~~widen the handler suite~~ (done 2026-09-02: `api_postgres.rs` now also covers JWT login/refresh/lockout/role gating, database `fcp_…` API tokens (scoping, expiry, revocation), CDRs + usage + billing summary, rate plans + the balance ledger, trunks in Carrier and Enterprise modes, queue configs, and voicemail mailboxes — 16 tests. Findings fixed: `fcp_api_core::create_router` never attached the API token store (only fcp-admin's inlined copy did — admin now calls the shared `create_router_with`), platform tokens ignored the `x-fcp-tenant-id` override on `/cdrs`, and `POST /api-tokens` answered 200 instead of 201). → ~~remaining API gaps~~ (done 2026-09-02: 23 tests — added flows + versions, flow traces (call-visibility scoping), registrations, the presence runtime (state, watchers, auth policy, pres-rules XML) on a live `PresenceManager`, the queue runtime (agent login/pause/resume/logout, stats) on a live `QueueManager`, the self-service `/me` routes (profile, password, forwarding, voicemail) as an end-user session, and the `/events` WebSocket first-message auth over a real socket. Fixes: registrations and flow traces ignored the platform-admin `x-fcp-tenant-id` override like `/cdrs` had, `POST /flows` answered 200, and `/me/profile` spelled roles differently from login (`enduser` vs `end_user`). Phone-number porting has no API surface at all — only the `PortingStore` data model — so nothing to test there) → ~~multipart uploads~~ (done 2026-09-02: 26 tests — avatar upload/download/replace/delete with type, size, and field validation; voicemail greetings through both the admin and `/me` routes (upload, activate, audio download, expiring temporary greeting, delete); audio prompts create, upload, download, category filter, tenant scoping, delete. No product changes needed) → ~~SIP trace + debug routes~~ (done 2026-09-02: 28 tests — `/calls/:id/sip-messages` by call id, bare uuid, `?sip_call_id=`, and the CDR fallback, with tenant scoping; `/debug/*` summary, dialogs, subscriptions, calls, and rate limits against real `Dialog`/`Subscription`/`Call` objects inserted into the runtime managers, per tenant. Two bugs fixed: the SIP trace handler stripped the `call-` prefix before querying while the B2BUA stores the prefixed id, so live calls were untraceable until their CDR existed; and `/debug/dialogs` compared SIP Call-IDs against internal call ids, so tenant admins never saw any dialog — it now maps through the tenant's call legs. The trace handler also honours the `x-fcp-tenant-id` override now
8. ~~KMS design~~ (done 2026-09-01: envelope encryption in `fcp-crypto`, local + AWS KMS providers). ~~Phase 5 gate decisions~~ (made 2026-09-02: see the *Phase 5 gate decisions* table above)

## Phase 5 engineering order (agreed 2026-09-02)

1. ~~**E911** block 1~~ (done 2026-09-02, see the Phase 5 table and `docs/E911.md`). Block 2, unscheduled: PIDF-LO by value for NG911 trunks, subnet-based location, a provider-backed validator, notification delivery.
2. ~~**Compliance basics**~~ (done 2026-09-02, see the Phase 5 table and `docs/COMPLIANCE.md`). Follow-ups, unscheduled: consent announcement on auto-recorded calls, legal hold, per-subject export/erasure.
3. **Distributed conferencing**: the cascaded-mixing plan on file.
4. **Multi-region**: CockroachDB compatibility pass + CI service first, then region-aware cluster membership and event bus.
5. Video conferencing and the dedicated CDR pipeline remain unscheduled carrier-scale items.

## Historical documents

Superseded status docs live in [`docs/archive/`](docs/archive/README.md) — they describe
Dec 2025–Jan 2026 state and must not be used for planning. `ARCHITECTURE-v0.2.md` remains
the architecture *vision*; its §12 roadmap checkboxes are not maintained — this file is.
