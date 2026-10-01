# Developer surface

Designed 2026-09-29, after the nightly stack (`docs/NIGHTLY_STACK.md`)
closed. This is the third recommendation of the 2026-09-25 feature review
against BroadSoft, CUCM, Zoom, Webex Calling and Twilio. It covers what a
developer outside the project meets first: the REST API, its reference,
API keys, webhooks, SDKs, and a flow that can listen as well as speak.

Every CPaaS in that review sells a documented API, keys a customer
manages, signed webhooks and speech input. FCP has an API of about 300
routes, and a developer can find almost none of it. It has keys that let a
read-only user mint an administrator's, webhooks that a tenant cannot
manage and that a receiver cannot verify against replay, and flows that
hear only digits. This document designs the fixes. Agreed 2026-09-29: the eleven
questions in §5, each settled on its recommendation.

## 1. What the code has, and what it lacks

Three surveys of the code were run on 2026-09-29, and each finding was
checked against the file. The admin API is `crates/fcp-api-core`, served
by `fcp-admin` under `/api/v1`.

### 1.1 API keys: a privilege escalation, and less than a key needs

- **Any admin can mint any token.** `POST /api/v1/api-tokens`
  (`v1/mod.rs:655`) sits behind `require_admin` alone (`mod.rs:795`).
  `require_admin` passes every session whose role is not `end_user`
  (`auth.rs:59-66`), and that includes `read_only`.
  - `create_api_token` copies `req.permissions` into the token unchecked
    (`v1/api_tokens.rs:172`).
  - A token whose permissions include `"admin"` or `"*"` passes every
    check (`auth.rs:94-100`, and `check_permission` at :121).
  - So a `read_only` user can mint a token with full tenant
    administration.
  - The token stays inside the user's tenant: `is_platform_admin` is false
    for every API token (`auth.rs:81-87`).
- **Permission names are free text.** The tokens page offers `admin` and
  `read` (`ApiTokensPage.tsx:20-21`), but no `Permission` is named `read`
  (`permissions.rs:137-209`). A "read-only" token is therefore refused
  everywhere `check_permission` runs. Where only `require_admin` runs, it
  is refused by `is_admin`, which wants `admin` or `*`.
- **Enforcement is thin.** `check_permission` runs in 14 of the 51 handler
  files. Every other admin route asks only `is_admin`, so a token without
  `admin` is useless outside those 14 files, and a token with it reaches
  everything.
- **Missing lifecycle pieces:**
  - no owner (`user_id` is always empty, `api_tokens.rs:170`);
  - revoke is a hard delete (the audit row survives through
    `audit.rs`);
  - no rotation;
  - no per-token or per-tenant rate limit (`fcp-api-gateway` limits per
    source address only and authenticates nothing);
  - no personal token for an end user's `/me` routes.
- **Storing the SHA-256 of a 256-bit random secret is sound.** A salt adds
  nothing against a secret with that much entropy, so this stays.

### 1.2 The API has no contract

- **No specification exists.** The repository and the lockfile hold no
  OpenAPI, utoipa, aide or schemars. The hand-written documents cover
  about 40 of the ~300 routes:
  - `docs/API_PHONE_NUMBERS.md` and `docs/API_EVENTS.md` are current.
  - `docs/API_CURL.md` and `docs/API_EXAMPLES.md` are from January.
  - `API_EXAMPLES.md` names a `crates/fcp-server` that does not exist.
- **Types are hidden.** 222 of the 311 handler functions return
  `impl IntoResponse`. Their bodies are mostly typed structs, but the
  signature does not say which. About 20 routes return `json!` or `Value`:
  the 12 proxied live-conference routes, plus `recordings.rs:289`,
  `flows.rs:360` and others.
- **Errors come in two shapes.** `ApiError` answers
  `{"error": …, "details": …}` (`error.rs:12-61`). The auth middleware
  (`auth.rs:321`, `:341`, `:367`), the `TenantContext` extractor
  (`:434-476`) and axum's own extractor rejections answer plain text.
- **Pagination comes in five forms:**
  - Most lists (users, numbers, devices, extensions, trunks, tenants)
    return a bare array with no limit.
  - Flows, recordings and flow traces return
    `{items, total, limit, offset}`.
  - CDRs add a keyset `next_cursor` (`cdrs.rs:208-221`).
  - Several routes take a `limit` alone.
  - Billing takes optional `limit` and `offset`.
- **Timestamps come in four forms:**
  - 31 `SystemTime` fields serialise as serde's
    `{"secs_since_epoch", "nanos_since_epoch"}`.
  - About 55 are epoch integers, some in seconds and some in
    milliseconds.
  - A few are `f64` seconds.
  - A few are RFC 3339 strings.
- **Ids come in two forms.** Response structs mostly carry
  `id: String` set from `to_string()`, which gives `tenant-<uuid>`. A typed
  id serialised directly is a bare UUID (`identifiers.rs:11`). Both forms
  parse.
- **Browsers cannot use two things.** CORS allows neither `PATCH` nor the
  `x-fcp-tenant-id` header (`lib.rs:123-130`), although 15 routes are
  `PATCH`.
- **The clients are hand-maintained, and they drift.**
  - The CLI's 34 model structs are copies of the server's.
  - The admin UI's 244 endpoint functions and 154 types are written by
    hand (`api/endpoints.ts`, `types/index.ts`).
  - The UI calls routes the server does not have:
    `/calls/:id/{hold,resume,transfer,mute,unmute}` (`endpoints.ts:233`)
    and `/porting/requests*` (`:608`).
  - No SDK exists.
- **One version, no deprecation.** `/api/v1` is the only version, with no
  deprecation header and no API changelog.

### 1.3 Webhooks: built, unreachable, unverifiable

- **The dispatcher is sound.** `fcp-webhooks` signs, retries with backoff,
  logs deliveries and replays them. Endpoint secrets are sealed at rest
  (`fcp-storage-postgres/src/encryption.rs:76`).
- **A tenant cannot manage webhooks.** They are managed only on
  `fcp-events-server` (`routes/webhooks.rs:28-39`), behind that server's
  single service token.
  - The request names the tenant in its body (`tenant_id`, `:47`), so a
    caller holding the token can write any tenant's endpoints.
  - The admin API has no webhook route, and neither the UI nor the CLI
    shows one.
- **The routes probably match nothing.** They are written `{id}`, but the
  crate is on axum 0.7 (`Cargo.toml:30`), whose router takes `:id`. Under
  0.7 `/v1/webhooks/{id}` matches only a literal `{id}`. Only list and
  create work.
- **The secret is the caller's.** It is taken from the request (`:49`),
  so its strength is whatever the caller typed.
- **A receiver cannot reject a replay.** `X-FCP-Signature` is
  `sha256=<hex>` over the body alone (`payload.rs:25`), with no timestamp.
  A captured delivery verifies forever.
- **Nothing guards against SSRF.** A URL of `http://169.254.169.254/…` or
  any internal address is accepted and called.
- **The compose stack ships webhooks off**
  (`[webhook] enabled` defaults to false, and `configs/fcp-events-server.toml` does not set it).
- **The catalogue is documented in prose only.** 127 event types in 20
  categories live in `fcp-events/src/types.rs` and are described in
  `docs/API_EVENTS.md`. Nothing machine-readable lists them for a
  subscriber.

### 1.4 Flows speak but cannot listen

- **No speech recognition runs anywhere in FCP.**
  - What forge-media has:
    - `forge-transcription` is a stub whose one engine returns
      `NotImplemented`.
    - `forge-ai-stream`'s Deepgram, Anthropic and ElevenLabs connectors
      are stubs.
    - Its OpenAI Realtime connector works, and the conference server uses
      it for a room's AI participant (`conference_handler.rs:2611`).
  - No FCP crate reads caller audio as samples.
- **The tap exists but is not connected.** forge-engine decodes every
  inbound packet to PCM. It hands that PCM, labelled by leg, to a media
  bridge (`forwarding.rs:449-469`, `media_bridge.rs:55-63`), but only when
  the session has a `MediaBridgeManager`. FCP never sets one:
  - `fcp-media` keeps the forge session private (`backend.rs:441-470`).
  - `MediaManager` exposes digits (`subscribe_digits`,
    `create_dtmf_collector`) but no audio.
- **Gather reads digits only** (`fcp-flow/src/schema.rs:130-169`,
  `flow_handler.rs:3645-3917`).
- **The SIP INFO path is half built.** It writes `dtmf_input` and reads
  `__gather_transitions`, `__gather_variable` and `__gather_default`
  (`flow_handler.rs:4121-4173`), but nothing ever sets those variables.
  Only the default could fire, and it reads the unset variable too.
- **Voicemail transcription is modelled but never produced.**
  - The model has it: `VoicemailMessage.transcription`, the mailbox's
    `transcription_enabled` and `include_transcription`, the columns
    (migration 006) and the API fields.
  - Nothing produces it: `fcp-voicemail/src/lib.rs:378` always writes
    `None`, and nothing reads `transcription_enabled`.
- **Other gaps a flow developer meets:**
  - The `Http` node sends only what its template interpolates. It stores
    the raw response text in one variable and treats a 500 as success
    (`engine.rs:367-410`, `:805-851`).
  - `FlowDefinition.webhook_url` is never read.
  - Postgres keeps only a flow's latest version: `update_flow` increments
    in place (`lib.rs:6784-6814`), so `/flows/:id/versions` returns one
    row and a call cannot run the version it started on.
- **What already exists and serves as the model:** TTS. The
  `TtsProvider` trait in `fcp-media/src/tts.rs:385-402` has seven
  providers, including a `remote` one pointed at `fcp-tts-server`, a
  cache, and a Python worker for the local kokoro engine.

## 2. The model

- **One contract, generated from the code.** Every admin route carries a
  `utoipa` annotation naming its request, response and error types.
  - A test writes `docs/openapi.json`, and CI fails when it differs, the
    same way `check-migrations.sh` guards migrations.
  - `fcp-admin` serves the spec at `/api/v1/openapi.json` and renders it
    at `/admin/api`.
  - The admin UI's types, the CLI's models and the SDKs come from the
    spec, not from copies.
- **One shape for everything the spec describes:**
  - errors as `{"error", "details", "code"}` from every layer;
  - times as RFC 3339;
  - ids with their prefix;
  - every list as `{"data": [...], "next_cursor"}`, with `limit` and
    `cursor`;
  - `X-Request-Id` on every response;
  - `429` with `Retry-After` from the rate limiter.
- **A key is a scoped credential with an owner.**
  - Every token names its creator and carries permissions drawn from
    `Permission`, each one held by the creator. The reserved `admin` is
    replaced by the creator's role, snapshotted.
  - A token cannot outlive its creator's account: disabling the user
    disables their tokens.
  - An end user can hold a personal token for their own `/me` routes.
  - A token can be revoked (kept, marked, refused) and rotated (a new
    secret, with the old one honoured for a grace period).
  - Tokens are rate-limited per token and per tenant.
- **Webhooks belong to the tenant.**
  - `/api/v1/webhooks` in the admin API, scoped to the caller's tenant
    and gated by a new permission.
  - The server makes the secret, shows it once and rotates it.
  - A delivery is signed over its timestamp and body:
    `X-FCP-Signature: t=<unix>,v1=<hex>`, with HMAC-SHA256 over
    `"<t>.<body>"`. A receiver rejects a stale `t`.
  - A URL must be HTTPS, and it must not resolve to a private, loopback
    or link-local address unless the platform allows it.
  - The event types are listed at `/api/v1/webhooks/event-types` and
    appear in the spec's `webhooks` section.
  - The events server keeps dispatching and loses its management routes.
- **Speech has the same shape as TTS.**
  - `fcp_media::stt` defines an `SttProvider`: a streaming session fed
    16 kHz mono PCM, yielding partial and final results with confidence,
    plus a one-shot `transcribe(file)` for recordings.
  - Its providers: Deepgram (streaming), OpenAI (a file, or a stream cut
    by VAD), a local faster-whisper worker like the kokoro one, and
    `remote`, pointed at `fcp-tts-server`, which gains `/v1/transcribe`.
  - The media manager gains `listen(session, leg)`. It attaches
    forge-engine's media bridge to that session on first use and yields
    the leg's PCM, resampled.
  - Gather listens for speech, digits or both. Voicemail transcribes a
    message when its mailbox asks.

## 3. Phases

One PR per phase. Each is proved by tests, and the specification, the
SDKs and the guide are checked in CI.

1. **Keys made safe.**
   - Permissions are validated against `Permission`, and each must be held
     by the creator. `admin` and `*` become an alias for the creator's
     role at creation.
   - Tokens are created, listed and revoked under a new
     `api_token_manage` permission (held by `tenant_admin` and above).
   - Other fields: `created_by`, `revoked_at` (revoke keeps the row), and
     rotation with a grace period (`POST /api-tokens/:id/rotate`).
   - A disabled or deleted creator's tokens are refused.
   - Personal tokens for end users, limited to `/me`.
   - The tokens page offers the real permissions, grouped.
   - `check_permission` in every admin handler that lacks one, so a
     scoped token reaches what it names and no more. This is the phase's
     largest part: a table of route to permission, checked by a test that
     walks the router.
   - Proved by a test per role that a token cannot exceed its creator,
     and a test that every admin route either checks a permission or is
     listed as admin-only.
   - **Done (v0.25.293).** What was built is described in
     `docs/API_TOKENS.md`. It differs from the plan above in these ways:
     - The route table is `route_permissions::ROUTES`. It covers every
       authenticated route, and is enforced by one middleware for sessions
       and tokens alike rather than by a check in each handler.
     - An API token cannot make another token.
     - A maker locked out by failed passwords keeps their tokens.
     - Tokens carrying `admin` or `*` from before this release still work,
       flagged `legacy_full_access`.
     - Writing the services' configuration takes `system_config`.
     - The events WebSocket takes `call_view`.
2. **One shape.**
   - JSON errors from the auth layer, the tenant extractor and axum's
     rejections (a `code` added to `ApiError`).
   - CORS gains `PATCH` and `x-fcp-tenant-id`.
   - `X-Request-Id` on every response.
   - Rate limits per token and per tenant (`[api.rate_limit]`), counted in
     Redis when the admin has one and in the process otherwise, with 429,
     `Retry-After` and `RateLimit-*` headers.
   - The normalisation agreed in Q3 and Q4: times, ids and list envelopes.
     The UI and the CLI are changed in the same PR.
   - The UI's calls to routes that do not exist are removed or wired, as
     Q10 settles.
   - Phase 2 is split into three PRs:
     - **2a** covers errors, CORS, request ids, rate limits and the dead UI
       calls. **Done (v0.25.294)**, as `docs/API_TOKENS.md` §5–6
       describes. Where the counter runs was settled by Q11: in the
       admin's Redis when it has one (`RedisStorage::count_in_window`),
       in the process otherwise, and a failed count lets the request
       through.
     - **2b** makes times RFC 3339 and ids prefixed. **Done (v0.25.295).**
       - Every response time is RFC 3339, through `fcp_api_core::time`'s
         serde helpers on the handlers' own structs. Domain structs, which
         also go into JSONB, heartbeats and events, keep their shapes, and
         the API gives them views.
       - Typed ids were already prefixed wherever a response held one.
       - Four resources have unprefixed UUID ids.
       - The UI and the CLI read the new forms, and the CLI still reads
         the old ones.
       - The test suite checks every response it sees.
     - **2c** puts every list in the `{data, next_cursor}` envelope.
       **Done (v0.25.296).**
       - `fcp_api_core::page` provides `PageQuery`, `Page`, and `paginate`
         (keyset over a stable key) for the lists read whole.
       - The audit and deletion logs page in SQL by `(time, id)`.
       - CDRs keep their store cursor inside the envelope.
       - Flows, recordings, flow traces and billing use an offset cursor.
       - The UI's `getAll` follows cursors. CDRs, recordings and flow
         traces page with a cursor stack.
       - The CLI's `list_all` follows cursors too. Its registrations list
         had quietly read nothing against the admin API.
       - The paging test found the compliance store dropping sub-seconds
         from times it read back, so a cursor skipped rows later in the
         same second.
3. **The contract.**
   - `utoipa` on every admin route, and typed responses in place of
     `impl IntoResponse` where the body is JSON.
   - The spec is generated into `docs/openapi.json` and checked in CI.
   - `/api/v1/openapi.json` is served, with a reference page at
     `/admin/api`.
   - The admin UI's `types/index.ts` is replaced by types generated with
     `openapi-typescript`, so `npx tsc -b` catches drift.
   - The CLI's models are generated or taken from `fcp-api-core`.
   - `docs/API_CURL.md` and `API_EXAMPLES.md` are deleted.
   - Phase 3 is split in two. **3a** covers the spec, its checks, where it
     is served and the reference page; **3b** covers the UI's and the CLI's
     types generated from it.
   - **3a done (v0.25.297).**
     - `utoipa` 6 is used (5 when this was written): a `#[utoipa::path]` on
       all 306 routes, plus fcp-admin's three cluster routes.
     - `ToSchema` goes on the API's types, and on the fcp-core, presence
       and cluster types it returns, behind their `openapi` features.
     - `fcp_api_core::openapi::ApiDoc` holds the spec. A test holds it to
       the route table, and another refuses two schemas with one name.
     - `docs/openapi.json` is checked by fcp-admin's
       `docs_openapi_json_is_the_spec` (regenerate it with
       `FCP_WRITE_OPENAPI=1`).
     - The spec is served at `/api/v1/openapi.json`, rendered at
       `/admin/api`.
     - `api_postgres.rs` validates every response any test receives
       against its operation's schema, which covers 198 of the 306
       operations. That check found:
       - eleven types sharing schema names;
       - proxied conference and supervision errors outside the error
         shape;
       - a 422 without a code.
   - **3b done (v0.25.298).**
     - The admin UI generates `src/api/schema.d.ts` from `docs/openapi.json`
       with `openapi-typescript` (`npm run gen:api`). 127 of its types are
       now aliases of the generated schemas, so `tsc` catches drift. It
       found:
       - `tenant_id` sent in two update bodies the server never read;
       - codecs typed as free strings;
       - the audit log typed after the config audit.
     - A new CI job, **Admin UI**, regenerates the file, fails on a diff,
       and typechecks. CI had never built the UI.
     - The CLI keeps its models. `fcp-cli/tests/contract.rs` feeds each of
       the 13 a minimal and a full instance of its schema, and so found the
       debug subscription field that 2b renamed.
4. **SDKs.**
   - TypeScript and Python clients in `sdk/`, generated from the spec,
     under a thin hand-written layer: auth, the tenant header, a pager over
     `next_cursor`, typed errors, retries on 429 and 503, and
     `verify_webhook(secret, header, body)`.
   - Versions follow the workspace.
   - CI builds both and runs each SDK's own tests against a live
     `fcp-admin` from the testkit.
   - Publishing to npm and PyPI is the owner's step (Q6).
   - **Done (v0.25.299).**
     - `sdk/typescript` (`@fcp/sdk`): `openapi-typescript` +
       `openapi-fetch`, with a retrying fetch, `ok`/`FcpError`,
       `paginate`/`all`, and `verifyWebhook`.
     - `sdk/python` (`fcp-sdk`): `openapi-python-client` 0.29.1 generates
       `fcp_api`, with no post-hooks so the output is byte-reproducible;
       `fcp_sdk` is the layer (retrying transports, sync and async, `ok`,
       `paginate`, `verify_webhook`).
     - One webhook test vector is shared by both SDKs, for phase 5's
       dispatcher.
     - `fcp-admin/tests/sdks.rs` runs both suites against a live admin
       (`FCP_SDK_TESTS=required` in CI). The **SDKs** job checks the
       generated code and the versions' major and minor.
     - Getting the Python client to generate found two contract bugs,
       both fixed with a test that guards against them:
       - 200 query parameters declared "in path" (`IntoParams` without
         `parameter_in = Query`);
       - a tuple schema no generator reads.
     - The four upload forms are now described in the contract.
5. **Webhooks.**
   - `/api/v1/webhooks`: CRUD, `test`, deliveries, replay,
     `rotate-secret` and `event-types`, tenant-scoped, under
     `webhook_manage`.
   - Server-made secrets, the timestamped signature, and the URL guard
     (`[webhooks] allow_private_targets` for a lab).
   - The events server's management routes are deleted, and its
     dispatcher is on in compose.
   - A Webhooks page in the admin UI.
   - Proved by a delivery verified with the SDK's helper, a replay
     refused as stale, and a private URL refused.
   - **Done (v0.25.300).**
     - `fcp_api_core::v1::webhooks`: ten routes under `webhook_manage`
       (tenant and telecom admins), a `whsec_` secret made by the server
       and shown once, rotation with an overlap during which both secrets
       sign (`previous_secret`, sealed at rest), tests and replays through
       a dispatcher in `ApiState`, the catalogue from
       `PlatformEvent::EVENT_TYPES` (a test keeps it equal to
       `event_type_name`).
     - `X-FCP-Signature: t=<unix>,v1=<hex>` replaces `sha256=`; each
       attempt, retry and replay is signed when it is sent.
     - `fcp_webhooks::target`: the URL checked when written (HTTPS, no
       credentials, no non-public address, the name resolved) and the
       dispatcher's resolver dropping non-public addresses at delivery,
       with redirects off; `[webhooks] allow_private_targets` on the
       admin and `[webhook] allow_private_targets` on the events server.
     - The events server's `/v1/webhooks` routes are deleted. It refuses
       to start with webhooks on and no store; in compose it opens
       Postgres under the master key with webhooks on, and it is one of
       the restore test's database services.
     - The spec's `webhooks` section describes a delivery
       (`WebhookEvent`, its `event_type` an enum of every type).
     - The admin UI's **Webhooks** page; `docs/WEBHOOKS.md`.
     - Proved by `webhooks_are_the_tenants_and_their_secret_is_shown_once`
       and `a_webhook_delivery_is_signed_logged_and_replayed`
       (`api_postgres.rs`: private and non-HTTPS URLs refused, another
       tenant blind, a delivery verified and refused ten minutes later,
       a replay signed anew, both secrets during a rotation), and by each
       SDK's live test receiving a delivery and checking it with
       `verifyWebhook` / `verify_webhook`.
6. **Speech recognition.**
   - `fcp_media::stt` with the four providers.
   - `MediaManager::listen` over forge-engine's media bridge. If the
     bridge needs anything forge-media does not yet export, it is a
     forge-media release first.
   - `/v1/transcribe` on `fcp-tts-server`, as a streaming WebSocket and a
     file upload.
   - Voicemail transcription when the mailbox asks: stored, put in the
     notification email, published as `voicemail.transcribed`.
   - Tests use a scripted provider in `fcp-testkit` that answers a known
     transcript for known audio. Real providers are proved by a manual
     check recorded in the PR.
   - **Done (v0.25.301)** (`docs/SPEECH.md`).
     - `fcp_media::stt`: `SttProvider` (one-shot `transcribe`, native
       `stream` or none), `SttService` (`transcribe_wav` for any WAV,
       `stream` cutting a provider without one into utterances by energy
       with `Endpointer`), and the four providers: `deepgram` (files and
       its WebSocket, an utterance's pieces joined), `openai`, `local`
       (`scripts/whisper_stt_worker.py`, faster-whisper) and `remote`.
     - `MediaBackend::listen` over forge-engine's `MediaBridgeManager`,
       which forge-media already exported: no forge-media release. One
       bridge per session, fanned out per leg and resampled to 16 kHz,
       detached when nobody listens or the session ends.
       `MediaManager::recognize` feeds a leg to the provider and stops
       when the stream is finished or dropped.
     - `fcp-tts-server`: `[stt.providers.<name>]`, `POST /v1/transcribe`
       and the `/v1/transcribe/stream` WebSocket; a server may now carry
       speech recognition without TTS.
     - `[stt]` on the call manager and the voicemail server. A mailbox
       with `transcription_enabled` has each message transcribed after
       the call, stored on the message, and published as
       `voicemail.transcribed`, which carries the tenant so webhooks get
       it.
     - Found on the way: the call manager's voicemail service wrote its
       messages' paths under `/var/fcp/voicemail/recordings` while its
       recorder wrote under `[media] recording_dir`, so no message it
       recorded could be played or read. Both are now `recording_dir`.
     - Not done: the email half. No node sends a voicemail notification
       email, so `include_transcription` has nothing to put the text in.
       The voicemail events other than `voicemail.transcribed` carry no
       tenant and so reach no webhook.
     - Proved by `stt_providers.rs` (Deepgram's and OpenAI's requests and
       answers against stand-ins), the listener test over real RTP in
       `backend.rs`, `fcp-tts-server/tests/transcribe.rs` and
       `fcp-call-manager/tests/speech.rs` (a message left through a flow,
       transcribed by `fcp_testkit::stt::ScriptedStt`, and one not sent
       when the mailbox does not ask).
7. **Flows that listen.**
   - Gather gains:
     - `input` (`dtmf`, `speech`, `dtmf_speech`);
     - `speech_timeout` (end-of-utterance silence);
     - `hints`;
     - `language`;
     - `speech_transitions` (a phrase or keyword to a node);
     - the variables `speech` and `speech_confidence`;
     - barge-in on speech.
   - A `Transcribe` node sends a call's live transcript as
     `call.transcript` events, if Q9 keeps it.
   - The flow builder gains the fields.
   - The flow fixes:
     - `Http` posts the flow's variables as JSON when asked, parses a JSON
       response into dotted variables, and takes `on_error` for a status
       of 400 or above.
     - The INFO path's Gather works.
     - Postgres keeps every version (`flow_versions`), and a call runs the
       version it started on.
     - `webhook_url` is deleted.
   - Split in two: 7a the listening, 7b the flow fixes other than INFO.
   - **7a done (v0.25.302)** (`docs/SPEECH.md` §6).
     - Gather's `input`, `speech_timeout_ms` (milliseconds, named for its
       unit), `hints`, `language` and `speech_transitions`, matched as
       whole words with the longest phrase winning
       (`fcp_flow::schema::match_speech`); `speech`, `speech_confidence`
       and `gather_input` set. A caller who has begun to speak is waited
       for past `timeout`; 200 ms of voice or a partial transcript stops
       the prompt.
     - `fcp_sip::flow_listen`: the Gather's digits and recognition set up
       before the prompt, and the `Transcribe` node (`start`/`stop`,
       `caller`/`callee`/`both`, `interim_results`) publishing
       `call.transcript` with the tenant, so webhooks get it.
     - The INFO path: a digit by SIP INFO joins the Gather in progress.
       It never could: `start_flow` set the call's `flow_id` on a copy and
       not in the call manager, so the INFO handler never saw a flow, and
       the variables it read were never written.
     - Found on the way: the listener heard nothing while a phone sent
       nothing, so a phone suppressing silence never ended an utterance.
       A leg that has been sending and goes quiet is now heard as silence
       after 60 ms.
     - The flow builder's fields, the Transcribe node in its palette and
       spoken transitions drawn as edges.
     - Proved by `fcp-call-manager/tests/listen.rs`: a menu spoken to,
       pressed by INFO and left silent, and the sales branch's transcript
       read from the node's events.
   - **7b done (v0.25.304)** (`docs/FLOWS.md`).
     - `Http`: `send_variables` posts the flow's variables as JSON; the
       status lands in `<response_variable>_status`; a JSON body is
       unpacked into dotted variables (256 at most, 16 deep, the previous
       response's cleared); `on_error` is taken for no response, a body
       over 1 MiB, or a status of 400 or above. Without `on_error` an
       error status still goes to `next`, as before, so no existing flow
       starts failing.
     - `flow_versions` (migration 089), written by a trigger on `flows`
       and backfilled; the version reads come from it, and
       `GET /flows/{id}/versions/{version}` is new. `PUT /flows/{id}`
       now answers with the version it stored (it answered the old one).
       A call already ran the version it started on (the flow is loaded
       once per call); now it is proved, and the version a trace names
       can be read back.
     - `webhook_url` is gone from the model, the API, the spec and the
       SDKs. The column stays until a later contraction.
     - Proved by the engine's tests against a local HTTP server, the
       tightened `flows_crud_versions_and_tenant_scoping`, and
       `a_call_runs_the_version_it_started_on` in `listen.rs`.
8. **Live call control and the guide.**
   - `/api/v1/calls/:id/{hold,resume,transfer,redirect,play,say,hangup}`
     over the node API that enterprise calling built. `redirect` moves a
     live call into a flow.
   - `docs/DEVELOPER_GUIDE.md`: a token, a call, a webhook, a flow that
     listens, in each SDK, run in CI as the SDK tests' examples.
   - Split in two: 8a the call control, 8b the guide.
   - **8a done (v0.25.305)** (`docs/CALL_CONTROL.md`).
     - The node's `/calls/:id/{hold,resume,transfer,redirect,play,say,hangup}`
       (`fcp-call-manager/src/call_control.rs`) over new
       `SipMessageHandler` methods (`fcp_sip::call_control`): hold and
       resume set the call's held leg, so the hold music and the muting
       are the phone-hold path's; transfer is the REFER path's
       `TransferRun` with `notify_referrer` off; play resolves the
       tenant's prompt; say is the node's text-to-speech; hangup BYEs
       both legs.
     - Redirect is a transfer to the node itself at `flow~<ticket>`, a
       one-time ticket naming the flow and tenant, redeemed by the
       dial-back's INVITE, which starts the flow.
     - The admin API's `/api/v1/calls/:id/…` under `call_control`,
       forwarded like supervision (`SupervisionRequest::Control`), with
       typed requests in the spec and the SDKs.
     - The admin UI's Hold, Resume, Transfer and hangup reach the node.
       Its buttons showed for states no source reports (`active`,
       `confirmed`, `held`), so they never appeared; an answered call
       (`answered` or `Established`) now offers them. Mute was never a
       route and is not one.
     - Proved by `calling.rs` (hold heard, music and silence; a prompt
       heard; say refused without TTS; hangup; a transfer answered by
       Carol with Bob dropped; a redirect running the flow for Alice) and
       `fcp-admin/tests/call_control.rs` (every control forwarded with its
       fields, token and tenant; the node's refusal in the API's shape).

## 4. What this does not do

- It adds no `/v2`: the normalisation lands in `v1` (Q3).
- It adds no AI agent in flows. The OpenAI Realtime connector stays the
  conference server's.
- It adds no marketplace, sandbox tenants or usage-based API billing.
- It does not rename `fcp-tts-server`, even though it now also
  recognises speech.

## 5. Questions

1. **Who may create a token, and with what?** *Recommended:* anyone
   holding the new `api_token_manage`, with permissions each held by the
   creator; `admin` means "the creator's role", snapshotted at creation.
   The alternative (tenant admins only, any permission) closes the hole
   but leaves the owner model out.
2. **Personal tokens for end users?** *Recommended:* yes, limited to
   `/me`. A user's own integrations (a CRM dialling through
   `/me/dial-out`) otherwise need a password-based JWT.
3. **Normalise in `v1`, or add `v2`?** *Recommended:* normalise in `v1`
   now. Today's only clients are our own UI and CLI, both changed in the
   same PR. Once the spec and the SDKs publish the shapes, every change
   becomes a version. The changes: RFC 3339 times, prefixed ids, and one
   error shape.
4. **Every list paginated?** *Recommended:* yes, with `{data, next_cursor}`
   and `limit` (default 100, maximum 1000) on every list. Keyset where the
   table can grow without bound (CDRs already, users, numbers, devices,
   extensions, audit); an offset cursor elsewhere.
5. **Which spec tool?** *Recommended:* `utoipa` 5. Its annotations sit
   beside the handler, it does not depend on the axum version (FCP is on
   0.7), and it is the most used. `aide` infers more but is tied to
   axum's types and a newer axum.
6. **Which SDKs, and published by whom?** *Recommended:* TypeScript and
   Python, generated (`openapi-typescript` + `openapi-fetch`,
   `openapi-python-client`) under a thin hand-written layer, kept in
   `sdk/`, built and tested in CI. Publishing to npm and PyPI stays the
   owner's step, like the GHCR packages.
7. **Where do webhooks live?** *Recommended:* in the admin API, tenant
   by tenant. The events server keeps only the dispatcher, and its
   management routes are deleted: they are unreachable today and trust a
   tenant id from the body.
8. **Keep the old signature header for a release?** *Recommended:* no.
   Webhooks ship off, and their routes cannot reach an endpoint by id, so
   nobody verifies the old header. The new one replaces it.
9. **Which speech providers first, and a live `Transcribe` node?**
   *Recommended:* Deepgram (streaming), OpenAI (file and VAD-cut stream),
   a local faster-whisper worker, and `remote`. The `Transcribe` node
   stays in phase 7: it costs little once `listen` exists, and it is what
   a CPaaS customer asks for second.
10. **The UI's calls to routes that do not exist?** *Recommended:* call
    hold, resume and transfer become real in phase 8. Mute is dropped, as
    the B2BUA has no mute. The porting page is deleted, as FCP has no
    porting.
11. **Where are rate limits counted?** *Recommended:* in `fcp-api-core`,
    per token and per tenant, in the admin's Redis when it has one and in
    the process otherwise. The gateway keeps its per-address limit for
    anonymous abuse.
