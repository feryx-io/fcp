# FCP Status

**The single source of truth for what is done, partial, or absent.**
Update this file when feature status changes; keep other docs describing scope, not status.

- **Last reviewed:** 2026-09-01 (full code audit against ARCHITECTURE-v0.2.md)
- **Version:** v0.25.63 · 33 workspace crates
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
| Provisioning — vendor templates | ❌ Missing | No shipped Polycom/Yealink/Cisco/Grandstream templates |
| Policy engine (`fcp-policy`) | ❌ Not extracted | Rules live inline in `fcp-routing` + SBC admission config; no dedicated crate or DSL |
| Billing (`fcp-billing`) | ❌ Absent | Only read-only `GET /billing/summary` usage report; CDRs carry `cost: None // TODO` |
| SBC pipeline stages 3–5 | ❌ Planned | Reachability, trunk routing/failover, header & number transform |
| Encryption at rest / KMS | ❌ Absent | No KMS/cipher code in the storage layer |

## Phase 5 reality check

| Item | Status |
|---|---|
| Multi-region | ❌ `fcp-cluster` is flat single-region SWIM membership |
| E911 | ❌ `E911Address` data model + CRUD only; no validation service, PSAP, or emergency routing |
| Video conferencing | ❌ Conference server and WebRTC gateway are audio-only |
| Regulatory compliance | ❌ Not started |
| Distributed conferencing | 📋 Ready plan in `docs/TO_DO_CONFERENCE_LOADBALANCE_HA.md` (~2–3 weeks) |

## Descoped / undecided (vision items with zero code)

Each needs a deliberate keep-or-descope decision:

- Plugin runtime (WASM/Lua) + `fcp-plugins` SDK
- gRPC API (`fcp-api-grpc`)
- Dedicated CDR-processor pipeline (currently inline + `fcp-jobs`)
- Standalone proxy worker (registrar/B2BUA/SBC exist; proxy does not)
- SQLite and CockroachDB storage backends (workspace is Postgres + Redis only; this also gates the multi-region storage strategy)

## Known weak spots

- **Test coverage** outside the SIP stack is thin: storage backends and most binaries have little to no automated coverage. Workspace `TODO`/`todo!()` count is low (~45, mostly `fcp-media`'s RemoteBackend), so the debt is coverage, not stubs.
- `proc-macro-error2`/`redis 0.25` future-incompat warnings were cleared in v0.25.63.

## Next steps (agreed 2026-09-01)

1. ~~Documentation truth pass~~ (done 2026-09-01)
2. ~~Finish transcoding path~~ (done 2026-09-01: PT map + transcoder arming wired through `fcp-media`; `RemoteBackend` stubs return errors)
3. Billing MVP: stamp CDR `cost`, per-tenant rate tables, build on existing usage aggregation
4. Ship vendor provisioning templates (seed via migration)
5. Extract `fcp-policy` (evaluator + admission behind one crate, YAML rule format)
6. SBC pipeline stages 3–5
7. Parallel: raise the test floor (containerized Postgres integration tests, binary smoke tests)
8. Phase 5 gate decisions: storage strategy, plugin/gRPC keep-or-descope, schedule distributed conferencing, KMS design

## Historical documents

Superseded status docs live in [`docs/archive/`](docs/archive/README.md) — they describe
Dec 2025–Jan 2026 state and must not be used for planning. `ARCHITECTURE-v0.2.md` remains
the architecture *vision*; its §12 roadmap checkboxes are not maintained — this file is.
