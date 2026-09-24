# The FCP stack on one Docker host

`docker-compose.yml` here brings up Postgres, Redis and every FCP service
on one machine (v0.25.251, `docs/SHIP_AND_OPERATE.md` phases 3 and 4,
`docs/PRODUCTION_STACK.md` phase 1). The
same configurations, environment and tokens are what a CI test starts
the binaries from (`crates/fcp-call-manager/tests/compose_stack.rs`), so
a change to this stack is checked by a registration and a call.

## Start

```bash
git clone --recursive https://github.com/thevoiceguy/fcp.git   # the submodules are built into the images
cd fcp/deploy/docker-compose
$EDITOR .env                          # HOST_IP, and every secret (all say change-me)
docker compose up -d --build          # a Rust release build per service the first time
docker compose ps                     # every service healthy
bash scripts/integration-test.sh      # waits for health, then exercises each service
```

`.env` is read by the compose file. `HOST_IP` is the address phones,
browsers and carriers reach the host on; the SIP and RTP services put it
in their SDP. Every `FCP_*` value is a secret handed to the binary of that
prefix through the environment override every binary has, and the
compose file passes the shared ones on (the events token to every
producer, the call manager's and conference server's tokens and the join
secret to the admin). No configuration under `configs/` carries a
secret. To keep your own values out of git, keep a copy elsewhere and
pass `--env-file`.

A value with no variable of its own — an SMTP password under the events
server's `[notifications]`, a TURN server's password, a webhook's
signing secret — is written as `${NAME}` in the configuration and given
to the service in its `environment:`; every binary resolves such a
reference when it loads its file and refuses to start if the name is not
set. Nothing under `configs/` uses one yet; the stack test proves the
mechanism on the call manager's node id.

Options:

```bash
docker compose --profile registrar up -d                       # also the standalone registrar
docker compose --profile tts up -d                             # also text to speech (a large image: PyTorch)
docker compose -f docker-compose.yml -f docker-compose.gpu.yml up -d   # the conference server on an NVIDIA GPU (docs/GPU.md)
```

## What runs, and where

Two kinds of service. The ones that carry SIP and RTP — the call
manager, the SBC, the conference server, the IVR, voicemail and presence
servers, the media server and the WebRTC gateway — run on the host's
network, because Docker's bridge cannot forward RTP port ranges usefully. They reach the bridge services through the ports
those publish on the host (`127.0.0.1:5432`, `:6379`, `:8081`), and the
bridge services reach them through `host.docker.internal`.

| Service | Network | Ports on the host |
|---|---|---|
| `postgres` (user `fcp`, database `fcp`) | bridge | 5432 |
| `redis` | bridge | 6379 |
| `fcp-admin` — the API and the admin UI at `/admin/` | bridge | 8080 |
| `fcp-api-gateway` — the public face of the admin | bridge | 8000 |
| `fcp-events-server` | bridge | 8081 |
| `fcp-jobs` — the CDR processor and job workers | bridge | 8082 |
| `fcp-call-manager` | host | SIP 5060, HTTP 8084, RTP 10000–10999 |
| `fcp-sbc` — carriers point here; its one trunk is the call manager | host | SIP 5078, health 8094, RTP 28000–28999 |
| `fcp-conference` | host | SIP 5074, HTTP 8090, RTP 20000–20999 |
| `fcp-ivr` | host | SIP 5070, HTTP 8086, RTP 22000–22999 |
| `fcp-voicemail` — the call manager sends unanswered calls here | host | SIP 5072, HTTP 8087, RTP 24000–24999 |
| `fcp-presence` | host | SIP 5076, HTTP 8092 |
| `fcp-media` | host | HTTP 8085, RTP 30000–30999 |
| `fcp-webrtc-gw` | host | HTTP 8095, SIP 5077, RTP 34000–35999 |
| `fcp-registrar` (profile `registrar`) | host | SIP 5062, HTTP 8083 |
| `fcp-tts` (profile `tts`) — speech for the call manager and IVR | bridge | 8093 |
| `prometheus` | bridge | 9090 |
| `grafana` (admin/admin; anonymous viewers) | bridge | 3000 |

Migrations run inside every binary that opens Postgres, at its startup;
there is no migrate step. Records go from the call manager and the
conference server to the Redis stream, and `fcp-jobs` stores them
(`docs/CDR_PIPELINE.md`). Prometheus gets its configuration from the
compose file itself, so its bearer tokens come from the same `.env`; it
scrapes every metrics endpoint. The admin reaches the call manager and
the conference server with their tokens, and mints join tokens with the
secret the conference server verifies them with.

The TTS server is off unless its profile is up, because its image carries
PyTorch (the Kokoro engine); without it the call manager's and IVR's
prompts are silent. The image builds nightly like the rest.

Not in this stack yet: TLS anywhere, encryption at rest, and clustering —
one node of each (`docs/PRODUCTION_STACK.md` has the order).

## Use it

Health needs no token; everything else on a service's port needs the
token from `.env`:

```bash
. ./.env
curl -s http://localhost:8080/api/v1/health
curl -s -H "Authorization: Bearer $FCP_ADMIN_AUTH_TOKEN" http://localhost:8080/api/v1/tenants
curl -s -H "Authorization: Bearer $FCP_CALL_MANAGER_API_TOKEN" http://localhost:8084/status | jq .
curl -s -H "Authorization: Bearer $FCP_EVENTS_AUTH_TOKEN" http://localhost:8081/calls
curl -s http://localhost:8000/gateway/status | jq .backends
```

The first tenant, user and extension are created through the API or the
admin UI as `docs/SINGLE-NODE-SETUP.md` §9 shows; a phone then registers
at `HOST_IP:5060`, realm `fcp.local`. Browsers join rooms through the
WebRTC gateway's join page at `http://HOST_IP:8095/join`
(`docs/WEBRTC.md`). `scripts/test-*.sh` exercise each service with the
right token.

## Change it

Every configuration under `configs/` is checked by its binary's smoke
test with this `.env` (`--check-config` must accept it), the compose
file is validated by CI's packaging job on every pull request (alone,
with the GPU overlay, with the registrar and tts profiles), the fourteen images are
built nightly, and the stack test starts every service (the TTS server
apart), waits for each to be healthy and carries a call.
Check a file by hand:

```bash
set -a; . ./.env; set +a
../../target/release/fcp-call-manager --check-config --config configs/fcp-call-manager.toml
```

Rebuild after a code change with `docker compose up -d --build`; `docker
compose logs -f <service>` follows a service; `docker compose down -v`
removes the volumes too, the database included.

## When it does not come up

- **A service exits at once.** `docker compose logs <service>`: the
  binary prints the rule it broke, and `--check-config` reproduces it.
- **`postgres` is unhealthy.** Its volume holds a database from a
  different `POSTGRES_PASSWORD`; `docker compose down -v` and start
  again.
- **The call manager is up but phones do not register.** `HOST_IP` must
  be the address the phone reaches; the user must exist in the realm
  `fcp.local`.
- **A port is in use.** The plan above is fixed; stop what holds the
  port or change the service's configuration and the compose file
  together (the stack test uses the same ports).
- **The images will not build.** The submodules must be checked out
  (`git submodule update --init --recursive`); the nightly `images`
  workflow builds every one and says which broke.
