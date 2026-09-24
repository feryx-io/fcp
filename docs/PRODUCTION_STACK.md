# The production-shaped stack

Status: **agreed** (2026-09-24), all four questions in §3 settled by the
user.
Chosen when ship-and-operate closed (`docs/SHIP_AND_OPERATE.md`): the
leftovers that project found by evidence and left, made into a stack an
operator could run for real.

Ship-and-operate ended with a compose stack that comes up and carries a
call, checked by CI. It also ended with a list: five binaries with no
image, no TLS anywhere, encryption at rest never on, credentials in
nested lists that no environment variable reaches, and a routing store
that answers a stranger with a 500. Each is small; together they are
the difference between a stack that demonstrates and one that could be
pointed at a carrier. This project is those, in order of what unblocks
what.

Read with `docs/SHIP_AND_OPERATE.md` (where each item was found),
`deploy/docker-compose/README.md` (the stack as it is),
`docs/ENCRYPTION.md` (what turning encryption on means) and
`docs/TENANT_TRUNKS.md` (what an SBC in front of the call manager is
for).

## 1. What is missing, and what the code already has

### 1.1 Five binaries have no image

`fcp-sbc`, `fcp-ivr`, `fcp-voicemail`, `fcp-presence` and `fcp-tts`
have no Dockerfile and no compose service. The nine Dockerfiles that
exist are one shape (`rust:1.92` builder, `debian:trixie-slim` runtime,
`ca-certificates` and `curl`), and four of the five need nothing more.
The SBC's SIP and health binds are flags, which its `FCP_SBC_BIND` and
`FCP_SBC_HEALTH_BIND` variables can set; it has no `[[core.trunks]]`
in its local file, so a stack that runs it must give it a trunk to the
call manager to be more than a healthy process.

`fcp-tts` is the exception. Its `local` provider spawns
`scripts/kokoro_tts_worker.py`, whose dependencies are `kokoro
soundfile numpy` — and `kokoro` brings PyTorch, which is gigabytes. The
server refuses to start with no provider that initialises, so an image
without Python is an image that exits.

### 1.2 No TLS

What each binary can do itself:

| Binary | TLS it serves |
|---|---|
| `fcp-call-manager`, `fcp-registrar` | SIPS (`sips_bind`) and WSS (`wss_bind`) from `tls_cert_path`/`tls_key_path` |
| `fcp-conference` | `tls_bind` for SIP/TLS, with mutual TLS for its inter-node trunks (`tls_client_*`) |
| `fcp-ivr`, `fcp-voicemail`, `fcp-presence` | WSS, from `tls_cert_path`/`tls_key_path` |
| `fcp-sbc` | SIP/TLS from `[core] tls_bind`, `tls_cert`, `tls_key` |
| `fcp-media`, `fcp-webrtc-gw` | HTTPS on their API, `[tls] enabled`, `cert`, `key` (the gateway builds its own listener) |
| `fcp-admin`, `fcp-api-gateway`, `fcp-events-server`, `fcp-jobs`, `fcp-tts` | **nothing** — plain HTTP only |

So the admin, which requires HTTPS for provisioning by default
(`provisioning_require_https`, which the compose stack turns off),
cannot provide it; it must sit behind something that does. The
compose stack ships no certificate and no way to make one; the testkit
has a CA of its own (`fcp_testkit::certs::ClusterCa`, rcgen) for the
conference trunks in tests.

### 1.3 Encryption at rest is never on

`docs/ENCRYPTION.md`: every binary that opens Postgres reads
`FCP_ENCRYPTION_PROVIDER` (`local` or `aws-kms`) and, for `local`,
`FCP_MASTER_KEY` or `FCP_MASTER_KEY_FILE`; on first start with a
provider the backend creates the initial DEK, new writes are sealed,
and `POST /api/v1/system/encryption/reencrypt` seals what was stored
before. The compose stack sets none of it, so every credential the
platform stores — SIP passwords, trunk passwords, API tokens — sits in
the database in clear.

### 1.4 Some credentials no variable reaches

Phase 2 of ship-and-operate gave every top-level token and database URL
an environment variable. Left in the file, because they are optional
providers or lists: the events server's SMTP password, SMS token and
notify-call token (`[notifications] smtp/sms/call`), a WebRTC TURN
server's password (`[[turn_servers]]`), a call manager webhook's signing
secret (`[[webhooks]]`). A per-field variable for each is possible but
does not scale; the pattern that does is a reference in the file —
`password = "${SMTP_PASSWORD}"` — resolved from the environment when
the file is loaded. No binary does that today.

### 1.5 A stranger gets a 500

The call manager's `[routing] rules_file` engine (`fcp-policy`'s YAML
store) evaluates every INVITE. It maps the caller to a tenant by
`find_user` in the YAML file's `users:` list and nothing else
(`crates/fcp-policy/src/storage/yaml.rs:163`); a caller not listed is
an error, and `evaluate_routing_policy`'s error becomes a 500
(`crates/fcp-sip/src/handler.rs:2650`). Ship-and-operate found it
because the shipped `routing.yaml` listed nobody, and removed the file
from the stack. The engine remains, and the defect with it: the users
and their tenants are in the database, where every other lookup finds
them, and an unknown caller is a refusal, not a failure.

### 1.6 What is sound

The stack itself: it comes up, carries a call, and is checked by CI on
every pull request (`docker compose config`, the configuration checks,
the stack test) and nightly (the images). Every addition below is
checked the same three ways, so a phase that breaks the stack fails
before it merges.

## 2. Phases

One PR each, in order. Each extends the compose file, its `.env`, the
stack test and the compose README; each new configuration is added to
its binary's configuration-check test.

| Phase | What |
|---|---|
| 1 | ✅ v0.25.251 — **Every binary has an image and a service** (§1.1). Dockerfiles for the SBC, IVR, voicemail, presence and TTS servers in the existing shape; compose services for each — the SBC on the host's network (SIP 5078, health 8094, its own RTP block) with one trunk to the call manager so a carrier can be pointed at it, the IVR, voicemail and presence servers on the host's network on the guide's ports, the TTS server on the bridge; the port plan extended, no collisions; tokens through the variables phase 2 of ship-and-operate made; `images.yml` builds fourteen; the stack test starts every service and checks each is healthy, and the compose README's table grows. The TTS image installs `kokoro soundfile numpy` on a Python base and runs under a `tts` profile (Q1). |
| 2 | ✅ v0.25.252 — **Secrets by reference** (§1.4, Q2). Every string in every binary's configuration may name an environment variable, `${NAME}`, resolved when the file is loaded, in `fcp_core::config` so every binary gets it the same way; a name the environment lacks refuses the file at startup, naming it; `--check-config` resolves and masks them like any secret. The per-field variables stay. The compose stack moves nothing to it yet — nothing in the stack needs a nested credential — but the stack test proves a `${…}` in a configuration is resolved from the environment. |
| 3 | ✅ v0.25.253 — **TLS in the stack** (§1.2, Q3). A one-shot `certs` service makes a CA and a certificate per service into a volume (`openssl` in Alpine; idempotent, so a restart keeps them). SIPS and WSS on the call manager and registrar, SIP/TLS on the conference server and SBC, HTTPS on the media and WebRTC servers, from those files. A reverse proxy in front of the admin, gateway and events server with the same CA, so provisioning can require HTTPS as it does by default; the admin's `gateway_url` and the WebRTC gateway's CORS origins become `https://`. The stack test registers over SIPS, and the CA is where the compose README says to find it for a phone. Plain SIP and HTTP stay listening beside them. |
| 4 | ✅ v0.25.254 — **Encryption at rest on** (§1.3). The master key is generated once into the same volume by the `certs` service (`openssl rand -base64 32`), every binary that opens Postgres gets `FCP_ENCRYPTION_PROVIDER=local` and `FCP_MASTER_KEY_FILE`, the compose README says how to run the one-time `reencrypt` and that the key is part of the backup; the stack test asserts `GET /api/v1/system/encryption` reports `local` and a DEK. |
| 5 | ✅ v0.25.255 — **A stranger is refused** (§1.5, Q4). The routing engine takes its users and their tenants from the database — a `RoutingStore` whose `find_user` asks the user store, with the YAML file supplying policies only — and a caller with no user is a `403` with a reason, as a policy's `Deny` already is, never a 500. The shipped compose stack gets a `routing.yaml` back, with a policy and no `users:`, and the stack test's call goes through it. |

Folded into this design's PR, not a phase: `STATUS.md`'s scoreboard
still said Phase 5 was "Planned — data-model stubs at most" and its
table still led three rows with 🟡 and 📐 for work its own later
entries record as done; the scoreboard and those rows now say what the
rest of the file says.

## 3. Questions

Four, settled by the user on 2026-09-24.

**Q1. The TTS image: opt-in, or in the stack?** — *Built, and run under
a `tts` profile.* The image is several gigabytes because
`kokoro` brings PyTorch; the stack without it is a stack whose call
manager and IVR run without speech, which they do today. An operator
who wants prompts spoken runs `--profile tts`; the nightly build still
builds the image, so it cannot rot.

**Q2. Secrets by reference: `${NAME}` in any configuration string?** —
*Yes, everywhere, beside the per-field variables.* One
mechanism reaches every credential including the nested ones, it is
what Kubernetes and Compose users expect, and a missing name refuses
the file the way an unknown key does. The alternative — a variable per
nested field — leaves the next nested credential unreachable again.

**Q3. HTTPS for the admin, gateway and events server: a reverse proxy
in the stack, or TLS in those binaries?** — *A reverse proxy.* Only the WebRTC gateway serves HTTPS itself today; adding a
TLS listener to four more binaries is four more places to configure
certificates and rotate them, where one proxy does it once and is what
a production deployment puts there anyway. The SIP-side TLS stays in
the binaries, which already have it.

**Q4. The routing engine's users: from the database, or retire the
engine?** — *From the database.* The YAML file is the right
home for a node's policies and the wrong one for its users, which the
database has; with users looked up there and a stranger refused with
403, the engine is a policy layer the compose stack can ship on. Whether
it should be retired in favour of the database rules
(`docs/TENANT_TRUNKS.md` §12) is a question for after it stops failing.

## 4. Testing

- Phase 1: every new configuration in its binary's configuration-check
  test with the stack's `.env`; the stack test starts every service and
  checks its health; `images.yml` builds fourteen.
- Phase 2: unit tests in `fcp-core` for `${NAME}` resolution, escaping
  and a missing name; a smoke test per binary that a `${…}` in its file
  is resolved and masked.
- Phase 3: the stack test registers a phone over SIPS with the stack's
  CA and reads the admin through the proxy over HTTPS. *As built:* the
  proxy is a container (nginx), which the test floor cannot run, so the
  stack test reads the WebRTC gateway and the media server over HTTPS
  under the same CA instead — the listeners the stack's own binaries
  serve — and the proxy is checked by `docker compose config` on every
  pull request and by the stack's own health check. The media server's
  `[tls]` needed a `bind` of its own: it had none, so HTTPS would have
  tried the HTTP address.
- Phase 4: the stack test reads `GET /api/v1/system/encryption`.
- Phase 5: unit tests for the database-backed user lookup and the
  refusal; the stack test's call passes through a shipped
  `routing.yaml`; a caller with no user gets 403 in the floor. *As
  built:* a phone's INVITE is digest-authenticated before the engine
  sees it, so a caller with no user can only reach the engine through a
  trusted edge naming one (`X-FCP-Caller-User`) — which is what the
  floor test does — or on a trunk, where a carrier's call is evaluated
  as the trunk's tenant's with no roles rather than refused, since a
  carrier holds no user's name.
