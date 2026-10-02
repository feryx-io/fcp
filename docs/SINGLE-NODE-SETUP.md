# Running FCP on one machine

Every FCP service on one host, from the source tree, the way a developer
or an evaluator runs it. Written against the code and the root
`fcp-*-local.toml` files (v0.25.248, `docs/SHIP_AND_OPERATE.md` phase 5);
every port, key, flag and command here was taken from a binary's
`--help`, its `--check-config` output or its router, not from memory.
For the same stack in containers see `deploy/docker-compose/README.md`;
for several nodes, `docs/CLUSTER_OPERATIONS_RUNBOOK.md`; for several
regions, `docs/MULTI_REGION_DEPLOYMENT.md`.

## 1. What runs

Fourteen binaries. Their names are not all their crates' names.

| Binary | Crate | What it is | Local config |
|---|---|---|---|
| `fcp-admin` | fcp-admin | The management API (`/api/v1`) and the admin UI (`/admin/`) | `fcp-admin-local.toml` |
| `fcp-api-gateway` | fcp-api-gateway | The public face of the admin: caching, rate limiting, one origin | `fcp-api-gateway-local.toml` |
| `fcp-events-server` | fcp-events-server | Event streaming: every service posts to it, clients subscribe | `fcp-events-server-local.toml` |
| `fcp-call-manager` | fcp-call-manager | Registrar, B2BUA, routing, PBX features: where phones register and calls are placed | `fcp-call-manager-local.toml` |
| `fcp-conference` | fcp-conference-server | The conference bridge | `fcp-conference-local.toml` |
| `fcp-ivr` | fcp-ivr | Call flows (IVR) | `fcp-ivr-local.toml` |
| `fcp-voicemail` | fcp-voicemail-server | Voicemail | `fcp-voicemail-local.toml` |
| `fcp-presence` | fcp-presence-server | Presence and BLF | `fcp-presence-local.toml` |
| `fcp-media` | fcp-media-server | The standalone media server | `fcp-media-local.toml` |
| `fcp-webrtc-gw` | fcp-webrtc-gateway | Browsers into conference rooms | `fcp-webrtc-gw-local.toml` |
| `fcp-tts` | fcp-tts-server | Text to speech for the call manager and IVR | `fcp-tts-local.toml` |
| `fcp-jobs` | fcp-jobs | The CDR processor and the job workers | `fcp-jobs-local.toml` |
| `fcp-sbc` | fcp-sbc | The session border controller | `fcp-sbc-local.toml` |
| `fcp-registrar` | fcp-registrar | A standalone registrar (optional; the call manager has one) | none |

A minimal PBX is Postgres, Redis, `fcp-events-server`, `fcp-admin` and
`fcp-call-manager`. Everything else is added for what it does.

## 2. Prerequisites

- **Rust**, the stable toolchain (`rust-toolchain.toml`). The code uses
  features from Rust 1.82; older toolchains do not build it.
- **PostgreSQL** (16 is what CI and the compose stack use). Every
  binary that opens Postgres runs the migrations itself at startup;
  there is no separate migrate step. Create the database and user the
  local files name:

  ```sql
  CREATE USER fcp WITH PASSWORD 'fcp_password';
  CREATE DATABASE fcp OWNER fcp;
  ```

- **Redis** 7, on `localhost:6379`. Cluster membership, the event bus,
  the CDR stream, the gateway's cache and rate limiter and the job
  queue use it. A single node runs without it, with those features off.
- **Git submodules** (`git submodule update --init --recursive`): the
  SIP and media stacks live under `external/` and nothing builds
  without them.
- For `fcp-tts`, a Python environment with the Kokoro engine
  (`.venv/bin/python3` in the local file). Without it the TTS server
  refuses to start, and the call manager and IVR run without speech.

## 3. Build

```bash
cargo build --release
ls target/release/ | grep '^fcp-'
```

The binaries are the first column of the table above.

## 4. The rules every binary enforces at startup

These are what most old setup instructions got wrong. A binary that
breaks one prints the rule and exits.

**A non-loopback HTTP bind needs a token.**

| Binary | The key | The environment variable |
|---|---|---|
| `fcp-admin` | `[api] auth_enabled = true` and `auth_token` | `FCP_ADMIN_AUTH_TOKEN` |
| `fcp-events-server` | `[auth] token` | `FCP_EVENTS_AUTH_TOKEN` |
| `fcp-call-manager` | `[http] api_token` | `FCP_CALL_MANAGER_API_TOKEN` |
| `fcp-conference` | `[http] auth_enabled = true` and `auth_token` | `FCP_CONFERENCE_AUTH_TOKEN` |
| `fcp-ivr`, `fcp-voicemail`, `fcp-presence`, `fcp-registrar` | `[http] api_token` | `FCP_IVR_API_TOKEN`, `FCP_VOICEMAIL_API_TOKEN`, `FCP_PRESENCE_API_TOKEN`, `FCP_REGISTRAR_API_TOKEN` |
| `fcp-sbc` | the `--health-api-token` flag | `FCP_SBC_HEALTH_API_TOKEN` |
| `fcp-media`, `fcp-tts` | `[auth] tokens` — required whatever the bind (`fcp-tts` alternatively `allow_unauthenticated`) | `FCP_MEDIA_AUTH_TOKENS`, `FCP_TTS_AUTH_TOKENS` (comma-separated) |
| `fcp-webrtc-gw` | `[auth] tokens`; unset, a random token is generated and logged | `FCP_WEBRTC_GW_AUTH_TOKENS` |

**A non-loopback SIP bind needs digest authentication:**
`[registrar] auth_enabled = true` on the call manager and registrar,
`[sip] auth_enabled = true` on the presence server. All default to on.

**The admin's session key** (`[api.session] signing_key`,
`FCP_ADMIN_SESSION_SIGNING_KEY`) signs a signed-in user's access tokens.
It must be at least 32 characters and differ from `auth_token`. Unset, it
is derived from `auth_token`. The events server takes the same key as
`[auth] session_signing_key` (`FCP_EVENTS_SESSION_SIGNING_KEY`) so the
admin UI's live views work (`docs/API_TOKENS.md` §7).

**Single sign-on** (`[api.sso] public_url`) is where browsers reach the
admin. Tenants' identity providers send people back there. Unset, nobody
signs in through a provider (`docs/SSO.md`).

**The admin's provisioning token** (`[api] provisioning_auth_token`,
what phones fetch their configuration with) must be at least 32
characters and differ from `auth_token`; `provisioning_require_https`
defaults to true, so on plain HTTP set it false.

**An unknown key is refused.** Every configuration section refuses a
key it does not have, naming it, so a typo or a key from an old
version stops the node rather than being silently ignored.

**Check a file before you start it:**

```bash
target/release/fcp-call-manager --check-config --config fcp-call-manager-local.toml
```

Every binary takes `--check-config`. It loads the file, applies the
environment, runs the rules above, prints the effective configuration
with every secret masked and exits 0 (the node would start), 1 (a rule
refuses it, printed) or 2 (it does not parse, printed). Run it without
`--config` to see a binary's defaults.

**Secrets from the environment.** Every token and every database URL
has an environment variable in the binary's `FCP_<BINARY>_*` style
(`--help` lists them; the table above has the tokens). A value in the
environment wins over the file.

**Any string by reference.** Every string in every binary's file may
name an environment variable as `${NAME}`, anywhere in the string —
`password = "${SMTP_PASSWORD}"`, `hosts = ["${CARRIER}:5060"]`,
`database_url = "postgresql://fcp:${DB_PASSWORD}@db:5432/fcp"` — and it
is resolved when the file is loaded, so a credential nested in a list or
an optional provider, which no per-field variable reaches, still stays
out of the file. A name the environment does not set refuses the file at
startup, naming the key and the variable (`--check-config` exits 1);
`$${…}` writes a literal `${…}`. The effective configuration
`--check-config` prints shows what was resolved, masked where the key
is a secret.

## 5. Addresses and ports

The root local files share one plan. The realm is `fcp.local`; every
HTTP bind is `0.0.0.0` with a token set; the developer's LAN address
`192.168.1.225` appears as `local_ip` and in URLs — **replace it with
your host's address** before phones or browsers on other machines will
reach you (or pass `--public-ip` / `FCP_<BINARY>_PUBLIC_IP` to the call
manager, IVR, voicemail, conference, media and WebRTC servers).

| Service | SIP | HTTP | RTP |
|---|---|---|---|
| `fcp-call-manager` | 5060 udp/tcp | 8084 | 20000–21999 |
| `fcp-ivr` | 5070 | 8086 | 22000–23999 |
| `fcp-voicemail` | 5072 | 8088 | 24000–25999 |
| `fcp-conference` | 5074 | 8090 | 26000–27999 |
| `fcp-presence` | 5076 | 8092 | — |
| `fcp-webrtc-gw` | 5077 | 8095 | 34000–35999 |
| `fcp-sbc` | `--bind` (the local file leaves it) | `--health-bind` | 28000–29999 |
| `fcp-media` | — | 8083 | 30000–31999 |
| `fcp-admin` | — | 8080 | — |
| `fcp-events-server` | — | 8081 | — |
| `fcp-jobs` | — | 8082 | — |
| `fcp-tts` | — | 8093 | — |
| `fcp-api-gateway` | — | 8000 | — |
| Postgres / Redis | — | 5432 / 6379 | — |

The binaries' own defaults are not this plan: every SIP service
defaults to `0.0.0.0:5060` (the presence server to `127.0.0.1:5060`)
and RTP `10000–20000`, and the HTTP defaults are `127.0.0.1:8080`
(admin *and* presence), `:8081` events, `:8082` jobs (on `0.0.0.0`),
`:8083` registrar, `:8084` call manager, `:8086` IVR, `:8087`
voicemail, `:8090` conference, `0.0.0.0:8080` media and WebRTC,
`0.0.0.0:8000` gateway. Two services on their defaults collide; use the
files.

## 6. What connects to what

Every local file already says this; here is the map.

- **Events.** The events server takes `[auth] token`
  (`events-local-secret` in the files); every producer posts to
  `http://localhost:8081/ingest` with the same value as its `[events]
  auth_token`: the call manager, conference server, IVR, voicemail
  server and admin.
- **The admin** calls the call manager (`[call_manager] url`,
  `auth_token` = the call manager's `api_token`), the conference server
  (`[conference] server_url`, `auth_token`, and `join_secret`, which
  must equal the conference server's `[conference] join_secret` — the
  admin mints join tokens, the conference server verifies them) and
  tells browsers where the WebRTC gateway is (`gateway_url`).
- **The call manager** sends conference calls to `[b2bua]
  conference_server_uri` (`sip:conference@<host>:5074`) and, with
  `[cluster] enabled`, places them on the least loaded node instead.
  The conference server lists the call manager's address in `[sip]
  trusted_edges` so the call manager may name the record each leg
  belongs to.
- **Speech.** The call manager and IVR ask `fcp-tts` at
  `tts_remote_url` (`http://localhost:8093`) with `tts_remote_token` =
  one of the TTS server's `[auth] tokens`.
- **The gateway** proxies `/api/v1/*` and `/metrics` to the admin and
  passes the caller's `Authorization` header through; the events and
  jobs backends it names are only health-probed. `GET /gateway/status`
  shows each backend's last probe.
- **Records.** With `[cdr] sink = "stream"` a node ships its records
  to the Redis stream and `fcp-jobs` (`[cdr] enabled = true`) stores
  them; the local files leave the default, `local`, where each node
  stores its own.
- **Region.** `[cluster] region` on every binary (or `FCP_REGION`)
  labels the node for placement across regions; one machine needs
  none.

## 7. Start

Postgres and Redis first, then the events server (everything posts to
it, and it reads tenants' webhooks from Postgres: `docs/WEBHOOKS.md`),
then the rest in any order. From the repository root:

```bash
target/release/fcp-events-server --config fcp-events-server-local.toml &
target/release/fcp-admin          --config fcp-admin-local.toml &
target/release/fcp-call-manager   --config fcp-call-manager-local.toml &
target/release/fcp-conference     --config fcp-conference-local.toml &
target/release/fcp-ivr            --config fcp-ivr-local.toml &
target/release/fcp-voicemail      --config fcp-voicemail-local.toml &
target/release/fcp-presence       --config fcp-presence-local.toml &
target/release/fcp-media          --config fcp-media-local.toml &
target/release/fcp-webrtc-gw      --config fcp-webrtc-gw-local.toml &
target/release/fcp-tts            --config fcp-tts-local.toml &
target/release/fcp-jobs           --config fcp-jobs-local.toml &
target/release/fcp-api-gateway    --config fcp-api-gateway-local.toml &
target/release/fcp-sbc --config fcp-sbc-local.toml \
    --bind 0.0.0.0:5078 --health-bind 127.0.0.1:8094 &
```

The SBC's SIP and health binds are flags (`FCP_SBC_BIND`,
`FCP_SBC_HEALTH_BIND`); a non-loopback health bind needs
`--health-api-token`.

What each logs when it is up: the admin `API server listening on …`,
the events server `WebSocket server listening`, the call manager
`fcp-call-manager ready — listening for SIP messages` (and `Cluster
membership started` when clustered), the conference server
`fcp-conference ready — listening for SIP INVITE messages`.

## 8. Check

`/health` needs no token anywhere and says the process is alive;
`/ready` needs none and says whether to send the node work (503 while
it drains).

```bash
for p in 8080/api/v1 8081 8082 8084 8086 8088 8090 8092 8093 8095 8000; do
  printf '%-14s ' "$p"; curl -s "http://localhost:$p/health"; echo
done
curl -s http://localhost:8083/health          # fcp-media
```

Everything else on a service's HTTP port wants its token:

```bash
curl -s -H 'Authorization: Bearer svc-local-secret' http://localhost:8084/status | jq .
curl -s -H 'Authorization: Bearer events-local-secret' http://localhost:8081/status | jq .
curl -s http://localhost:8000/gateway/status | jq .backends
```

`svc-local-secret` is the call manager's, conference server's, IVR's,
voicemail server's, presence server's and WebRTC gateway's token in the
local files; the admin's is `fcp-local-dev-token-2026`.

## 9. The first tenant, user and phone

A fresh database has no tenant. With the admin's token:

```bash
A='Authorization: Bearer fcp-local-dev-token-2026'
J='Content-Type: application/json'
# A tenant; note its id.
curl -s -H "$A" -H "$J" -X POST http://localhost:8080/api/v1/tenants \
  -d '{"name":"Acme"}' | jq .
# A user in it; the SIP password is what the phone registers with.
curl -s -H "$A" -H "$J" -X POST http://localhost:8080/api/v1/users \
  -d '{"username":"alice","password":"alice-pw","display_name":"Alice","tenant_id":"<tenant id>"}' | jq .
# An extension that rings that user; the target names the user by id.
curl -s -H "$A" -H "$J" -X POST http://localhost:8080/api/v1/extensions \
  -d '{"number":"1001","target":{"type":"user","id":"<user id>"},"tenant_id":"<tenant id>"}' | jq .
```

Or do the same in the admin UI at `http://localhost:8080/admin/`
(through the gateway at `:8000` only the API is proxied, not the UI).

A phone then registers at the host's address, port 5060, realm
`fcp.local`, user `alice`, password `alice-pw`. Registrations are
listed at `GET /registrations` on the call manager, with its token.

## 10. Stop

`SIGTERM` (or `POST /drain` with the token, then `SIGTERM`): each
routing binary stops taking new work, keeps what it carries until its
`drain_wait_secs`, tells the cluster, and exits; the call manager logs
`Shutdown complete`. The order does not matter.

## 11. Keeping it

Where every binary writes its state, what a backup must hold and what an
upgrade does: `docs/BACKUP_AND_RESTORE.md`.

## 12. When something is wrong

- **It exits at once with a rule.** Read the rule; `--check-config`
  shows the effective configuration it was judged on.
- **`unknown field`.** A key the binary does not have. `--check-config`
  without `--config` prints every key it does.
- **Phones do not register.** The SIP bind is `0.0.0.0` in the files,
  but `local_ip` is what the node puts in its own headers and SDP: set
  it (or `--public-ip`) to the address phones reach. Digest
  authentication is on; the user must exist with that password, in the
  realm `fcp.local`.
- **Calls have no audio.** The RTP range (§5) must be reachable from
  the phone, and `local_ip` / `public_ip` must be an address it can
  send to.
- **A producer's events never arrive.** Its `[events] auth_token`
  must equal the events server's `[auth] token`; the events server
  answers 401 otherwise, silently to the producer.
- **The admin's live views are empty.** Its `[events] auth_token`,
  `[call_manager] auth_token` and `[conference] auth_token` must match
  those services' tokens.
- **Two services on their defaults collide.** Use the files (§5).

## 13. What one node carries

Measured 2026-09-21 with `fcp-loadgen` (`docs/CAPACITY_AND_SURVIVABILITY.md`
§8) against a release-built call manager. **Read the machine before the
numbers**: a 12-core Azure VM with 2 GB of RAM running Postgres, the node
and the load generator at once. Every figure below is a floor — the run
ended before the node did.

| | Measured |
|---|---|
| Calls per second | **20/s sustained**, every rung from 2 to 20 held; the ramp ran out of time, not headroom |
| INVITE → `200 OK` | p50 **4 ms**, p99 **7 ms** at 20 calls/s |
| Media (B2BUA relay, 4419 legs) | **0.00 % loss**, 0.37 ms mean jitter |
| Memory per concurrent call | **≈0.3 MiB** (34 MiB idle → 64 MiB with 100 calls up) |
| CPU at 20 calls/s | 3 % of twelve cores |
| Registration storm | 300 devices, 1200 registrations, **all answered**, none refused |

Hold music (`[media] hold_music`, on by default,
`docs/ENTERPRISE_CALLING.md` §3.3) anchors every call through the node,
so every call takes an RTP port pair from `[media] rtp_port_start` to
`rtp_port_end`: size that range for the concurrent calls you expect, or
set `hold_music = false` to let calls without another reason to be
anchored send their media directly — holds are then passed to the far
phone and held callers hear silence.

What to take from it when sizing: a node's resident memory is roughly
`35 MiB + 0.3 MiB × concurrent calls` before recordings and conferences,
the signalling path costs single-digit milliseconds, and the relay does
not lose packets. What **not** to take from it: a ceiling. Nothing here
found one, and a real one needs a dedicated box with the load generator
on a different machine.

To measure your own hardware:

```bash
# On a machine that is not the node:
fcp-loadgen --profile call-ramp --sip <node>:5060 --realm <realm>   --users 50 --answer-extension-base 2000   --cps 5 --cps-step 5 --cps-max 200 --rung-seconds 30   --metrics-url http://<node>:8084/metrics --json ramp.json
```

Keep a single harness process under about 100 phones: past that the
generator becomes the bottleneck and measures itself (see the storm table
in §13 of the design document).
