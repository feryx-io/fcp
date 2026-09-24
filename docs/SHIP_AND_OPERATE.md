# Ship and operate

Status: **agreed** (2026-09-23), survey done, all four questions in §3
settled by the user; **phases 1–5 done** (v0.25.244–248). Chosen when the deployment gaps project closed
(`docs/DEPLOYMENT_GAPS.md`), the last of the four candidates named on
2026-09-20.

The question this project asks is the simplest one a platform can be
asked: **does what we ship start, and does what we tell an operator
work?** The survey answered both with no. Nothing in the repository
starts a shipped configuration or checks a documented command, so every
project since edge security that added a startup rule — a token on a
public bind, digest authentication on a public SIP bind — left the
packaging and the documents where they were. They rotted in step with
the code's improvement.

Read with `docs/DEPLOYMENT_GAPS.md` (the same method: each claim
checked against the code), `docs/EDGE_SECURITY.md` (where the startup
rules came from) and `docs/MULTI_REGION_DEPLOYMENT.md` (the one
deployment document written against the code, and the model for the
rest).

## 1. The survey

Three checks: every shipped configuration started against its binary;
every packaging file read against the binaries it packages; every
operator document read against the routes, keys, flags and defaults
the code has today. File references are to the repository root.

### 1.1 What we ship does not start

**Run, not read.** Each compose configuration was given to its binary
on this machine. Five of nine are refused before they touch the
network:

| Configuration | The binary says |
|---|---|
| `configs/fcp-admin.toml` | `api.auth_enabled must be true when binding the admin API to a non-loopback address (0.0.0.0:8080)` |
| `configs/fcp-call-manager.toml` | `http.api_token must be set when binding the call-manager HTTP API to a non-loopback address (0.0.0.0:8084)` |
| `configs/fcp-conference-server.toml` | `http.auth_enabled must be true when binding the conference HTTP API to a non-loopback address (0.0.0.0:8090)` |
| `configs/fcp-conference-server.gpu.toml` | the same |
| `configs/fcp-events-server.toml` | `Refusing to bind fcp-events-server to 0.0.0.0:8081 without auth.token configured` |
| `configs/fcp-media-server.toml` | a panic creating `/var/lib/fcp/media/recordings` — fine in its container, but the first thing an operator running it by hand sees |

The registrar's, gateway's, jobs' and WebRTC gateway's are accepted
(the registrar's was refused too until v0.25.237 fixed it in passing).
Every root `fcp-*-local.toml` is accepted: the local files were kept
current with the code, the shipped ones were not.

**Read.** `deploy/docker-compose/docker-compose.yml` cannot bring the
stack up even with those five fixed:

- It needs an external volume, `fcp_shared-data` (`:277`), that only the
  *root* `docker-compose.yml` creates, and only under that name when
  run from a directory called `fcp`. On any other machine `up` stops
  before the first container.
- **There is no Postgres service.** The admin, call manager and
  conference server all point at `postgres:5432`
  (`configs/fcp-admin.toml:15`, `fcp-call-manager.toml:72`,
  `fcp-conference-server.toml:22`); nothing answers. The root compose's
  Postgres has a different user and password and a different network.
- The call manager and the conference server both run on the host
  network and both bind `0.0.0.0:5060`. UDP binds with `SO_REUSEADDR`
  (`crates/fcp-sip/src/listener.rs:145`) so both succeed and one
  silently takes the other's traffic; the second TCP listener fails in
  a task and logs a line.
- The media server, on the host network, binds `0.0.0.0:8080` — the
  admin's published port. Whichever starts second loses, and the media
  server's own health check may be answered by the admin.
- The call manager's config names TLS certificate paths under
  `./certs`, a directory that does not exist; Docker mounts an empty
  one and the listener fails to start.
- The call manager, conference server and WebRTC gateway wait for the
  health of services that never become healthy.

And the wiring between the services that would start is wrong:

- The call manager posts events to `/ingest` with no `[events]
  auth_token`; the events server, once it has the token it must have,
  answers 401 to every one. The admin's events queries likewise.
- The admin's call manager URL is a compose hostname the call manager,
  on the host network, does not have; its `auth_token` is commented out
  while the call manager now requires one.
- The admin's conference and gateway URLs, the conference `join_secret`
  and `trusted_edges`, and the call manager's `conference_server_uri`
  are all unset, so live room controls, join tokens, conference CDRs
  under the call's id and placement to the conference server do not
  work, while the WebRTC gateway ships with its public join page on.
- Prometheus scrapes five metrics endpoints that need a token, with no
  token, and none of the three that do not.
- Every test script under `deploy/docker-compose/scripts/` but one calls
  routes that now need a token, without one; `integration-test.sh`
  parses `docker compose ps --format json` as an array, which Compose
  no longer prints.

A second, older packaging — the root `docker-compose.yml`,
`Dockerfile` and `config/fcp-docker.toml`, the call manager alone — fails
the same way (SIP auth off on a public bind, no `api_token`, the wrong
database credentials, a health check on a port the binary does not
serve).

**What the packaging never covers:** no Postgres, no database creation
or migrate step (migrations run inside every binary that opens Postgres,
which is right, but nobody says so); no compose service for the
registrar, and no Dockerfile at all for the SBC, IVR, voicemail,
presence or TTS servers; every token inline in a TOML with no
environment override for most of them (the events server's, the call
manager's, the conference server's), so a secret store has nothing to
inject; no TLS material for anything; encryption at rest never turned
on (`FCP_ENCRYPTION_PROVIDER` unset everywhere); the admin UI's `dist/`
gitignored, so an image from a clean checkout ships the placeholder
page; a committed 6 MB binary, `deploy/fcp-events-server`, that nothing
references.

**CI** builds no image, validates no compose file, and parses no
shipped TOML. Its toolchain is `stable`; the Dockerfiles pin
`rust:1.92`. Clippy runs `--all-features`, tests do not, so the video
features the NVIDIA image needs are linted and never tested.
`deny.toml` exists; no job runs it.

### 1.2 Stale keys are ignored, silently

No configuration struct outside the SBC's `[admission]` refuses an
unknown key. So:

- `configs/fcp-call-manager.toml` sets `[media] enable_transcoding` and
  `enable_dtmf`, which are not fields (`crates/fcp-call-manager/src/config.rs:545-598`).
- `configs/fcp-admin.toml` sets `[api] cors_enabled`; not a field.
- `fcp-admin-local.toml` puts `prompts_dir` after the `[logging]`
  header, so it is `logging.prompts_dir`, dropped.
- `fcp-sbc-local.toml` puts `redis_url` under `[cluster]` (it is
  top-level: `crates/fcp-sbc/src/config.rs:21`) — the node logs
  "cluster.enabled = true but redis_url is not set; cluster disabled" —
  and `[core.nat] public_ip` where the field is `external_ip`
  (`crates/fcp-sbc-core/src/nat.rs:66`). `docs/SINGLE-NODE-SETUP.md`
  copies both mistakes.
- `crates/fcp-admin/fcp-admin.toml.example` configures `storage_type`,
  `sqlite_path`, `postgres_url`, `mysql_url`, `mongodb_url`; the field
  is `database_url` and the others were never read. The IVR's and
  voicemail server's examples set `backend = "sqlite"`.
- The runbook's `cluster_secret` on the call manager
  (`docs/CLUSTER_OPERATIONS_RUNBOOK.md:37`) is parsed
  (`config.rs:708`) and read by nothing; `/internal/*` is authenticated
  by `[http] api_token` (`health.rs:2218-2226`).
- `fcp.toml` at the root is read by no binary.

Every one of these looked right, started, and did something other than
what its author meant.

### 1.3 The documents do not describe the code

`docs/SINGLE-NODE-SETUP.md`, `docs/CLUSTER_OPERATIONS_RUNBOOK.md`,
`deploy/docker-compose/README.md`, the root `README.md` and the four
crate READMEs that exist (admin, gateway, events server, jobs) were
checked line by line. None gives a working setup as written. The
patterns:

- **The token rules are in none of them.** Every binary refuses a
  non-loopback HTTP bind without its token (`[http] api_token` on the
  call manager, registrar, IVR, voicemail and presence servers;
  `api.auth_enabled` + `auth_token` on the admin; `[auth] token` on the
  events server; `http.auth_enabled` + `auth_token` on the conference
  server; `--health-api-token` on the SBC; `auth.tokens` on the media
  and TTS servers), and the call manager and presence server refuse a
  public SIP bind without digest authentication. The single-node guide
  binds everything to `0.0.0.0` with none of them.
- **Every operator route in the runbook is called without a token**
  (`/status`, `/drain`, `/calls`, `/registrations`, `/metrics`), and it
  says every routing binary serves all of them; the SBC and presence
  server have no `/status`, the registrar, IVR and voicemail servers no
  `/metrics`, `/calls` or `/registrations`.
- **Binary names, ports and defaults are wrong.** The guide lists
  `fcp-conference-server`, `fcp-media-server`, `fcp-presence-server`,
  `fcp-voicemail-server`; the binaries are `fcp-conference`,
  `fcp-media`, `fcp-presence`, `fcp-voicemail` (and `fcp-webrtc-gw`,
  `fcp-tts`, `fcp-registrar` are not listed at all). Its port plan gives
  the presence server no SIP port (it has one, defaulting to the call
  manager's 5060), gives the SBC 5076 (the presence local file's), the
  media server 8083 (the registrar's), and RTP ranges that overlap.
- **Features are described that do not exist and roadmapped that do.**
  The root README configures `[sip] bind_address`, `[storage]
  postgres_url`, `[media] rtp_port_range` and a `[voicemail]` section
  the call manager has never had; says the API uses basic auth, that
  clustering uses SWIM, that Rust 1.74 suffices (the code needs 1.82);
  and lists rate limiting, JWT, RBAC, STIR/SHAKEN, encryption at rest,
  billing, multi-region, E911, video and compliance as future. The
  admin README documents SQLite, MySQL and MongoDB backends and links
  an `fcp-server` crate. The compose README describes four services,
  SQLite, `deploy/kubernetes/` and a `cdr_processing` job type; none
  exists.
- **The example event and job payloads are invalid** (a tenant id that
  is not a UUID; a job id that is not a UUID; `created_at` as epoch
  seconds where RFC 3339 is read).
- Five more deployment documents in `docs/` — `DEPLOYMENT.md`
  (Kubernetes manifests for files that do not exist),
  `DEPLOYMENT_BINARIES.md` (a February design), `QUICKSTART.md`
  (SQLite), `INSTALL_DOCKER.md`, `TESTING.md` (January) — predate most
  of the platform and contradict it.

What the documents leave out that an operator needs: the token rules
above; `[events] auth_token` on every producer; `[cluster] region` /
`FCP_REGION`; the conference server's `[cluster] secret` and
`join_secret`; the admin's provisioning-token rules (32 characters,
distinct from `auth_token`); the five `FCP_*_PUBLIC_IP` variables; and
which HTTP binds default to loopback (most) and which to `0.0.0.0` (the
media and WebRTC gateway servers).

### 1.4 What is sound

- **The Dockerfiles build the right packages and copy the right
  binaries**, expose the right ports, and every health check they name
  is a route that exists and needs no token.
- **The root `fcp-*-local.toml` files all start** (the TTS server's
  fails only for want of its Python provider). They are the one set of
  configurations that was kept current; the guide should point at
  them rather than paraphrase them.
- **The binaries' own validation is good.** Every failure in §1.1 was
  caught at startup with a message naming the key. The rules are right;
  the packaging never met them.
- **`docs/MULTI_REGION_DEPLOYMENT.md`** was written against the code
  a day ago and needed no correction.

## 2. Why this happened, and the fix that stops it

Three projects — edge security, capacity and survivability, the
deployment gaps — each added a rule a binary enforces at startup, and
each was right to. None of them touched `deploy/` or the operator
documents, because nothing failed when they did not: no test starts a
shipped configuration, no CI job parses one, and the binaries accept
any key at all, so a configuration cannot be wrong in a way that shows.

The structural fixes, which the phases below build in before they fix
the content:

1. **Unknown keys are refused** (`#[serde(deny_unknown_fields)]` on
   every configuration section of every binary). This is the change
   that would have caught every item in §1.2 the day it was written.
   It is a behaviour change: a deployment carrying a key the code
   stopped reading will refuse to start until the key is removed — with
   a message naming it, which is the point.
2. **Every binary can check a configuration without running it**: a
   `--check-config` flag that loads, validates and exits, and prints
   the effective configuration with secrets masked. That is what CI
   runs over every shipped TOML, and what an operator runs before a
   restart.
3. **CI starts what we ship**: `docker compose config` on both compose
   files, `--check-config` over every configuration in `deploy/` and at
   the root, the images built (nightly, since a full build is long),
   and one smoke of the compose stack against the testkit's Postgres
   and Redis. A packaging change that breaks startup fails the PR.

## 3. Questions

Four, settled by the user on 2026-09-23.

**Q1. Refuse unknown configuration keys?** — *Yes, everywhere.* The alternative is a warning, which is what a log line
already is for the keys the code does read and ignore, and nobody read
those. A deployment that has junk in its file learns so at the next
restart, from a message that names the key. The seven configurations
that carry stale keys today are fixed in the same phase.

**Q2. The root packaging — `docker-compose.yml`, `Dockerfile`,
`config/fcp-docker.toml`?** — *Delete it.* It packages the
call manager alone, predates `deploy/docker-compose/`, and fails in the
same ways; keeping two packagings is how one of them rots. The
Postgres service and the volume it alone provided move into
`deploy/docker-compose/`.

**Q3. Where do the compose stack's secrets come from?** — *Environment
overrides on every binary, and a `.env` the compose file fills them
from.* Today most tokens can only be written into a TOML,
which is why the shipped TOMLs have none. An override per token
(`FCP_EVENTS_SERVER_AUTH_TOKEN`, `FCP_CALL_MANAGER_API_TOKEN`,
`FCP_CONFERENCE_AUTH_TOKEN`, and so on, in each binary's existing
`FCP_<BINARY>_*` style) lets the compose stack, Kubernetes or a secret
store inject them, and lets the shipped TOMLs stay free of secrets. The
alternative — literal `change-me` tokens in the shipped TOMLs — starts
the demo stack and teaches the wrong habit.

**Q4. The operator documents?** — *Rewrite the five against the code,
delete the four crate READMEs and the five dead deployment documents.* One place for each question — setting up a node
(`SINGLE-NODE-SETUP.md`), running a cluster (the runbook), running the
compose stack (its README), and the root README as the map — each
written the way `MULTI_REGION_DEPLOYMENT.md` was, from the code. The
crate READMEs duplicate the setup guide with older facts and would rot
again; the five dead documents go to `docs/archive/` with the rest.

Decided without asking: the committed binary `deploy/fcp-events-server`
is removed; the hard-coded developer LAN address `192.168.1.225` leaves
every shipped file in favour of `HOST_IP` from `.env`; the runbook's
dead `cluster_secret` key becomes a refused key like any other and the
runbook says `/internal/*` is authenticated by `[http] api_token`, which
therefore must agree across a cluster's call managers.

## 4. Phases

One PR each, in order. The structural phases come first so the content
phases are checked by them.

| Phase | What |
|---|---|
| 1 ✅ | **A configuration cannot lie** (§1.2, Q1). `deny_unknown_fields` on every configuration section of every binary; `--check-config` on every binary (load, validate, print the effective configuration with secrets masked, exit 0 or 1 with the message); every shipped and root TOML brought through it, which fixes the seven stale-key files and the dead `cluster_secret`. A CI step runs `--check-config` over every TOML under `deploy/` and at the root. Result: done as written, on every binary's own configuration sections and on the sections the shared crates own (`fcp-sbc-core`'s, `fcp-pbx`'s, `fcp-queue`'s, `fcp-presence`'s, `fcp-cdr`'s node config, `fcp-sip`'s edge config); a `[[trunks]]` entry is the one place a stray key is still ignored, since `fcp_core::Trunk` is also the stored model. `--check-config` exits 0 when the node would start, 1 when a startup rule refuses the file, 2 when it does not parse, and prints the effective configuration with every secret masked — by key, and the password of any URL. The CI step is each binary's smoke suite: `fcp_testkit::smoke::assert_configs_check` runs the files that binary ships, the root local file and the compose file(s) must parse, and the ones a node must accept are accepted; the five compose files the token rules refuse are held at "parses" until phase 3. The seven files with stale keys were refused as predicted and are fixed; the call manager's `cluster_secret` is gone. |
| 2 ✅ | **Secrets from the environment** (Q3). An environment override for every token and secret a binary reads from its TOML and does not yet take from the environment — the events server's `[auth] token`, the call manager's `[http] api_token` and `[events] auth_token`, the conference server's `[http] auth_token`, `[conference] join_secret` and `[cluster] secret`, the registrar's, IVR's, voicemail's and presence server's `[http] api_token`, the TTS server's and media server's `auth.tokens`, the SBC's already there — each in that binary's `FCP_<BINARY>_*` style, documented in `--help`, and masked by `--check-config`. Result: done as written, as clap arguments with `env` (so `--help` lists them and the value is never echoed), applied after the file is loaded so the environment wins: `FCP_CALL_MANAGER_{DATABASE_URL,API_TOKEN,EVENTS_AUTH_TOKEN,TTS_REMOTE_TOKEN}`, `FCP_REGISTRAR_{DATABASE_URL,API_TOKEN}`, `FCP_ADMIN_{DATABASE_URL,EVENTS_AUTH_TOKEN,CALL_MANAGER_AUTH_TOKEN,CONFERENCE_AUTH_TOKEN,CONFERENCE_JOIN_SECRET}` beside its existing two, `FCP_EVENTS_{AUTH_TOKEN,WEBHOOK_DATABASE_URL}`, `FCP_CONFERENCE_{DATABASE_URL,AUTH_TOKEN,EVENTS_AUTH_TOKEN,JOIN_SECRET,CLUSTER_SECRET}`, `FCP_IVR_{DATABASE_URL,API_TOKEN,EVENTS_AUTH_TOKEN,TTS_REMOTE_TOKEN}`, `FCP_VOICEMAIL_{DATABASE_URL,API_TOKEN,EVENTS_AUTH_TOKEN}`, `FCP_PRESENCE_{DATABASE_URL,API_TOKEN}`, `FCP_MEDIA_{DATABASE_URL,AUTH_TOKENS}`, `FCP_WEBRTC_GW_AUTH_TOKENS`, `FCP_TTS_AUTH_TOKENS` (the token lists comma-separated), `FCP_JOBS_DATABASE_URL` (the CDR processor's and archive's), `FCP_SBC_DATABASE_URL`. Database URLs are included because a password in one is a secret. Not covered: the events server's notification providers (SMTP password, SMS token, notify-call token), the gateway's TURN passwords and a call manager webhook's signing secret, which are nested lists or optional providers; they stay in the file. Proved per binary by a test that a public bind refused without its token is accepted with the token in the environment, shown masked and never printed. |
| 3 ✅ | **The stack comes up** (§1.1, Q2). `deploy/docker-compose/` gets a Postgres service and its own volumes; the root packaging is deleted; a port plan with no collisions (the conference server off 5060, the media server off 8080, RTP ranges disjoint); the host-network services reach the bridge services through published ports on `HOST_IP`; the tokens come from `.env` through phase 2's overrides; TLS for the call manager either generated by a one-shot service or off; the wiring set — events tokens on every producer, the admin's call manager, conference and gateway URLs and tokens, `trusted_edges`, `conference_server_uri`, `join_secret`; a registrar service; Prometheus scraping every metrics endpoint with the token it needs and Grafana pointed at the right host; the scripts sending tokens and `integration-test.sh` reading Compose's output as it is; the stray binary removed. A CI job runs `docker compose config` on both compose files on every PR and builds the images nightly. Result: done as written, with these choices. Postgres 16 runs as `postgres` (user `fcp`, database `fcp`, password from `.env`); the host-network services reach it, Redis and the events server through the ports those publish on the host (`127.0.0.1:5432`, `:6379`, `:8081`), and the admin reaches the call manager and conference server through `host.docker.internal`. The port plan: SIP 5060 call manager, 5074 conference, 5077 WebRTC, 5062 registrar; HTTP 8080 admin, 8081 events, 8082 jobs, 8083 registrar, 8084 call manager, 8085 media, 8090 conference, 8095 WebRTC, 8000 gateway; RTP 10000–10999, 20000–20999, 30000–30999, 34000–35999. `.env` holds every secret under the binaries' own `FCP_*` names and the compose file hands the shared ones on (the events token to every producer, the call manager's and conference server's tokens and the join secret to the admin), so the TOMLs carry none. TLS is off on the call manager rather than pointed at certificates that do not exist. Prometheus's configuration is written from the compose file (`configs: content:`) so its bearer tokens come from the same `.env`, and it scrapes every metrics endpoint including the three that need no token. The CDR stream pipeline is on: the call manager and conference server ship to Redis and `fcp-jobs` stores. The registrar is a `registrar` profile on SIP 5062, since the call manager holds 5060. The stray binary, the root packaging and its Prometheus and Grafana files are gone; the developer's LAN address is `HOST_IP` in `.env` alone. The scripts source `.env` and send each service its token; `integration-test.sh` reads Compose's output whether it is one object per line or an array. Each binary's smoke suite now checks its compose configuration *with the stack's `.env`*, so the five that phase 1 held at "parses" must start. CI's `packaging` job runs `docker compose config` on the stack alone, with the GPU overlay and with the registrar profile on every pull request; `images.yml` builds all nine images nightly and on demand. Not done here: no Docker on the development machine, so the stack was proved by the configurations and `docker compose config` in CI, not by bringing it up — that is phase 4. |
| 4 ✅ | **The stack is proved** (§1.1). The testkit starts the compose stack's binaries the way the compose file wires them — same configurations, same tokens, same ports, against the test Postgres and Redis — registers a phone through the call manager, places a call into the conference server, reads it back from the admin API through the gateway and sees its event on the events server. The compose file and the test share the configurations, so a change to one is checked by the other. Runs in CI. Result: done as written. `fcp_testkit::compose::ComposeStack` reads `docker-compose.yml` and `.env` — each service's Dockerfile (hence binary), mounted configuration, interpolated environment and health check — and `crates/fcp-call-manager/tests/compose_stack.rs` starts the events server, the admin, the gateway, the call manager and the conference server from exactly those, on the compose file's own ports, against the test's Postgres and Redis; the only things it changes are the compose network's names (to the loopback), the containers' paths (to a temp dir), the database and Redis URLs and `HOST_IP`. A phone registers at the call manager, joins a room on the conference server, the events server lists the call under the token the compose file gave the call manager, the gateway reads it back through the admin, and the gateway reports its backends healthy. Found on the first run: the shipped `routing.yaml` listed no users, and the YAML routing store knows only the users it lists, so the compose call manager answered every INVITE with **500** — the stack could not have carried a call. The file is gone from the stack (the tenants' routing rules live in the database); that a caller missing from a YAML routing store fails with 500 rather than being refused is a defect noted for later. |
| 5 ✅ | **The single-node guide and the runbook** (§1.3, Q4). `docs/SINGLE-NODE-SETUP.md` rewritten from the root local files: every binary by its real name, the token rules, the real defaults, a port plan that holds, `FCP_REGION`, the conference secrets, what `--check-config` says before a start. `docs/CLUSTER_OPERATIONS_RUNBOOK.md` rewritten against the routes that exist, per binary, with the token on every call; the dead `cluster_secret` gone; the log lines as they are. Both checked the way `MULTI_REGION_DEPLOYMENT.md` was. Result: both rewritten from scratch. The facts came from the binaries, not from the old text: every binary's `--help` (the flags and the `FCP_*` variables), its `--check-config` without a file (the defaults — which showed that two services on their defaults collide, the admin and presence server both on `127.0.0.1:8080`), its router (the routes that exist and which layer they sit under), and the source for the log lines quoted. The guide names the fourteen binaries by their real names, states the seven startup rules with the key and the variable for each, gives the port plan the root local files share (their RTP ranges made disjoint in the same change, the call manager's having overlapped four others), draws the map of what connects to what and with which token, shows the first tenant, user and extension through the API with the request shapes the code reads, and ends with the failures an operator meets and what each means. The runbook says what a cluster is (Redis heartbeats, no gossip, no leader), lists every binary's routes and its token — the SBC and presence server have no `/status`, `fcp-jobs` has no token, the SBC's `/metrics` is JSON — and gives the drain, rolling restart and partition procedures as the code does them; the dead `cluster_secret` is gone and `/internal/*` is said to use `api_token`. |
| 6 | **The READMEs** (§1.3, Q4). The root `README.md` rewritten as the map — what the platform is, what is done, how to build, where each question is answered — with the roadmap section replaced by a pointer to `STATUS.md`; the compose README rewritten against phase 3's stack; the four crate READMEs deleted; `DEPLOYMENT.md`, `DEPLOYMENT_BINARIES.md`, `QUICKSTART.md`, `INSTALL_DOCKER.md` and `TESTING.md` archived. Every command in every remaining document run once. |

## 5. Testing

- Phase 1: a unit test per binary that a configuration with one
  unknown key is refused naming the key; `--check-config` on every
  shipped TOML in CI, the assertion being the exit code.
- Phase 2: a unit test per binary that the environment override wins
  over the TOML and that `--check-config` masks it.
- Phase 3: `docker compose config` in CI; the images built nightly.
- Phase 4: the compose-shaped floor test, the phone-to-conference call
  and its readback; it is the test that stops phase 3 rotting.
- Phases 5 and 6: every command in the documents run by hand once
  against a node started from the root local files, and the token,
  route and default claims cross-checked against `--check-config`'s
  output and each binary's router.
