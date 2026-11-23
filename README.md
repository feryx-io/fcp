# Ferrous Communications Platform

FCP is a multi-tenant, API-first unified communications platform written
in Rust: a registrar and B2BUA with PBX features, a session border
controller, conferencing with video and a WebRTC gateway, IVR, voicemail,
presence, a programmable call-flow engine, billing, compliance, E911 and
multi-region operation, on a SIP stack and a media stack of its own. It
is an alternative to FreeSWITCH, Kamailio and Asterisk with memory safety,
async I/O and observability built in.

> **Status: pre-release.** FCP is under active development: its APIs,
> configuration and database schema still change between releases, and it
> is not yet recommended for production use.

**What is done, partial or absent is in [STATUS.md](STATUS.md)**, the
maintained source of truth. This file is the map.

## What runs

Fourteen binaries, each a service. Their names are not all their crates'
names; the full table with each one's configuration file is in
[docs/SINGLE-NODE-SETUP.md](docs/SINGLE-NODE-SETUP.md) §1.

| Binary | What it is |
|---|---|
| `fcp-call-manager` | Registrar, B2BUA, routing, PBX features: where phones register and calls are placed |
| `fcp-sbc` | The session border controller: trunks, admission, topology hiding, STIR/SHAKEN |
| `fcp-admin` | The management API (`/api/v1`) and the admin UI (`/admin/`) |
| `fcp-api-gateway` | The public face of the admin: caching, rate limiting, one origin |
| `fcp-events-server` | Event streaming: every service posts to it, clients subscribe, webhooks fire |
| `fcp-conference` | The conference bridge, audio and video, clustered |
| `fcp-webrtc-gw` | Browsers into conference rooms |
| `fcp-ivr`, `fcp-voicemail`, `fcp-presence` | Call flows, voicemail, presence and BLF |
| `fcp-media`, `fcp-tts` | The standalone media server; text to speech |
| `fcp-jobs` | The CDR processor and the job workers |
| `fcp-registrar` | A standalone registrar, for deployments that keep registration off the call manager |

Under `external/` are the two stacks they are built on, each its own
public repository: [siphon-rs](https://github.com/feryx-io/siphon-rs),
the SIP stack, and [forge-media](https://github.com/feryx-io/forge-media),
the media stack. Clone with `--recursive`, or run
`git submodule update --init --recursive`; nothing builds without them.

## Build and test

Rust stable (`rust-toolchain.toml`); the code needs Rust 1.82 or later.

```bash
git clone --recursive https://github.com/feryx-io/fcp.git
cd fcp
cargo build --release          # every binary, under target/release/
cargo test --workspace         # unit tests and every test that needs no database
```

The database-backed suites and the end-to-end tests against the running
binaries run when `FCP_TEST_DATABASE_URL` (and, for some,
`FCP_TEST_REDIS_URL`) is set; CI runs them against service containers.
What CI gates every pull request on:

```bash
# Format: the workspace members only (`--all` would also reformat the submodules)
pkgs=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name | "-p \(.)"' | tr '\n' ' ')
cargo fmt $pkgs -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets \
  --features fcp-conference-server/recording-webm,fcp-conference-server/recording-mp4,fcp-conference-server/video-openh264
```

`docs/TEST_FLOOR.md` describes the end-to-end harness (a scripted SIP
phone with media, and every binary started as a process).

## Run it

- **One machine, from source:** [docs/SINGLE-NODE-SETUP.md](docs/SINGLE-NODE-SETUP.md)
  — the port plan, the rules every binary enforces at startup, what
  connects to what, the first tenant, user and phone.
- **One machine, in containers:** [deploy/docker-compose/README.md](deploy/docker-compose/README.md)
  — `docker compose up -d --build` brings up Postgres, Redis and the
  stack; its configurations are the ones a CI test starts.
- **Several nodes:** [docs/CLUSTER_OPERATIONS_RUNBOOK.md](docs/CLUSTER_OPERATIONS_RUNBOOK.md)
  — what a cluster is, every binary's routes and token, draining and
  rolling restarts.
- **Several regions:** [docs/MULTI_REGION_DEPLOYMENT.md](docs/MULTI_REGION_DEPLOYMENT.md).
- **Video on a GPU:** [docs/GPU.md](docs/GPU.md).

Three things about configuration that every binary shares:

- **`--check-config`** loads a file, applies the environment, runs the
  startup rules and prints the effective configuration with every secret
  masked; exit 0 means the node would start. Run it before a restart,
  and without `--config` to see a binary's defaults.
- **A non-loopback HTTP bind needs the binary's token, and a public SIP
  bind needs digest authentication.** The guide's §4 has the key and the
  environment variable for each. An unknown key is refused, naming it.
- **Every secret has an environment variable** in the binary's
  `FCP_<BINARY>_*` style (`--help` lists them), which wins over the file.
- **Any string in a file may name one as `${NAME}`**, resolved when the
  file is loaded; a name the environment lacks refuses the file. That is
  how a credential nested in a list stays out of the file.

## Where each question is answered

The design documents are the record of what was built and why; each
ends with what it left out. `CLAUDE.md` indexes them all; the ones an
operator or integrator reaches for:

| Question | Document |
|---|---|
| The architecture and the roadmap it came from | `ARCHITECTURE-v0.2.md` |
| Trunks, carriers, tenant trunks, routing rules | `docs/TENANT_TRUNKS.md` |
| What a node can carry, admission, draining | `docs/CAPACITY_AND_SURVIVABILITY.md` |
| The edge: authentication, address checks, rate limits | `docs/EDGE_SECURITY.md` |
| Conferencing across nodes, and video | `docs/DISTRIBUTED_CONFERENCING.md`, `docs/VIDEO_CONFERENCING.md`, `docs/WEBRTC.md` |
| CDRs: the record, the pipeline, the archive | `docs/CDR_PIPELINE.md` |
| Billing, encryption at rest, compliance, E911 | `docs/BILLING.md`, `docs/ENCRYPTION.md`, `docs/COMPLIANCE.md`, `docs/E911.md` |
| Events, webhooks and the WebSocket stream | `docs/API_EVENTS.md` |
| The packaging and the documents, checked against the code | `docs/SHIP_AND_OPERATE.md`, `docs/DEPLOYMENT_GAPS.md` |
| The SIP stack | `external/siphon-rs/CLAUDE.md` |

## Contributing

`docs/CONTRIBUTING.md`. In short: design first for anything larger than
a fix, one pull request per phase, and the three CI gates above before
opening it. `CLAUDE.md` is the guide to the codebase for people and for
the assistant that helps maintain it.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT), at your option. Unless you state otherwise,
any contribution you intentionally submit for inclusion in FCP, as defined
in the Apache-2.0 license, is dual licensed as above, without any
additional terms or conditions.

## Support

Issues and discussion: https://github.com/feryx-io/fcp/issues
