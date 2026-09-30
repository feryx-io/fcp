# Webhooks

A webhook is an HTTPS URL of yours that FCP POSTs your tenant's events to,
each one signed with a secret only you and FCP hold. This document is for
the developer receiving them and for the operator running them. The design
is `docs/DEVELOPER_SURFACE.md` §3, phase 5.

## 1. Making one

Webhooks belong to a tenant and are managed on the admin API under
`/api/v1/webhooks`, by a caller holding `webhook_manage` (tenant admins and
above, or an API token granted it: `docs/API_TOKENS.md`). The admin UI's
**Webhooks** page does the same.

```bash
curl -s -X POST https://fcp.example.com/api/v1/webhooks \
  -H "Authorization: Bearer $FCP_TOKEN" -H 'Content-Type: application/json' \
  -d '{"url": "https://crm.example.com/fcp/events", "events": ["call.*", "voicemail.new_message"]}'
```

```json
{
  "webhook": { "id": "whk-…", "url": "https://crm.example.com/fcp/events",
               "events": ["call.*", "voicemail.new_message"], "enabled": true,
               "secret_prefix": "whsec_AbC", "…": "…" },
  "secret": "whsec_…"
}
```

The server makes the secret (`whsec_` and 32 random bytes). **It is shown
this once**; afterwards only its first characters are, as `secret_prefix`.
It is sealed at rest like every other secret (`docs/ENCRYPTION.md`).

| Route | What it does |
|---|---|
| `GET /api/v1/webhooks` | the tenant's webhooks, paged |
| `POST /api/v1/webhooks` | make one: `url`, `events`, `enabled`, `description` |
| `GET /api/v1/webhooks/event-types` | every event type, with its category |
| `GET /api/v1/webhooks/{id}` | one, without its secret |
| `PATCH /api/v1/webhooks/{id}` | change `url`, `events`, `enabled` or `description` (empty clears it) |
| `DELETE /api/v1/webhooks/{id}` | delete it and its delivery log |
| `POST /api/v1/webhooks/{id}/rotate-secret` | a new secret, the old one signing beside it for `overlap_seconds` |
| `POST /api/v1/webhooks/{id}/test` | send a `webhook.test` delivery now and say what the receiver answered |
| `GET /api/v1/webhooks/{id}/deliveries` | the latest deliveries, newest first |
| `POST /api/v1/webhooks/{id}/deliveries/{delivery_id}/replay` | send a logged delivery again now |

The request and response schemas are in the contract (`docs/openapi.json`,
served at `/api/v1/openapi.json`), and both SDKs have them
(`sdk/README.md`).

### Events

`events` filters what is delivered:

- a name from `/api/v1/webhooks/event-types`, such as `call.answered`;
- a category, such as `call.*` or `billing.*`;
- `*`, or an empty list, for every event.

An unknown name is refused. The catalogue is generated from
`PlatformEvent::EVENT_TYPES`, and the events themselves are described in
`docs/API_EVENTS.md`.

### Where a webhook may point

A URL must be HTTPS, without credentials in it, and must reach only public
addresses. Refused:

- loopback, private (RFC 1918), link-local and the cloud metadata service
  (`169.254.169.254`);
- shared (`100.64.0.0/10`), documentation, benchmarking and reserved
  ranges, and their IPv6 forms (unique local, link local, mapped, NAT64);
- `localhost` and any `*.localhost` name.

A name is resolved when the webhook is made or changed, and refused if any
address it resolves to is one of these. At delivery the dispatcher's
resolver drops such addresses again, so a name re-pointed at a private
address later (DNS rebinding) is not reached either. Redirects are not
followed.

A lab can allow private and plain-HTTP targets — see §4. Never set that
where tenants you do not trust make webhooks.

## 2. Receiving one

Each delivery is a `POST` with a JSON body:

```json
{
  "event_id": "4b1c…",
  "event_type": "call.answered",
  "timestamp": "2026-09-29T14:03:11Z",
  "tenant_id": "tenant-…",
  "data": { "…": "the event, as docs/API_EVENTS.md describes it" }
}
```

and these headers:

| Header | Value |
|---|---|
| `X-FCP-Event` | the event type |
| `X-FCP-Signature` | `t=<unix seconds>,v1=<hex>` |

`event_id` is unique per delivery; a replay carries the same one, so a
receiver may drop an id it has seen. The spec's `webhooks` section
describes the body (`WebhookEvent`).

### Verifying

`v1` is the hex HMAC-SHA256, under the webhook's secret, of the string
`"<t>.<body>"`, where `body` is the raw request body. A receiver:

1. reads the body as bytes, before parsing it;
2. splits the header on `,` into `t` and one or more `v1`;
3. refuses the delivery if `t` is more than five minutes from its clock,
   so a captured delivery cannot be replayed;
4. accepts it if any `v1` equals its own HMAC, compared in constant time.

The SDKs do exactly this:

```ts
import { verifyWebhook } from '@fcp/sdk'
if (!verifyWebhook(secret, req.headers['x-fcp-signature'], rawBody)) return res.status(400).end()
```

```python
from fcp_sdk import verify_webhook
if not verify_webhook(secret, request.headers["X-FCP-Signature"], request.get_data()):
    abort(400)
```

Both take a tolerance (`toleranceSecs` / `tolerance_secs`, default 300).
A test vector every implementation shares: secret `whsec_test_vector`,
`t=1790000000`, body `{"event":"call.started"}` gives
`v1=008976dcc4d8a2d92dbfe2c643bdb8f498825fa20a43baa8d0e7a0358ffa159e`.

Answer with any 2xx. Anything else, or no answer within the timeout (ten
seconds), is retried with exponential backoff: 30 seconds, doubling, at
most an hour apart, up to five attempts. Each attempt is re-signed with
the time it is sent.

### Rotating the secret

`POST /api/v1/webhooks/{id}/rotate-secret` returns a new secret, shown once.
For `overlap_seconds` (default a day, at most a week) every delivery
carries two `v1` values, one under each secret, so a receiver keeps
verifying while you deploy the new one. `previous_secret_valid_until` says
when the old one stops. `{"overlap_seconds": 0}` retires it at once — for a
secret you think has leaked.

### Testing and replaying

`POST …/test` sends a `webhook.test` delivery (or `{"event_type": "…"}`, any
listed type) with a small body, and answers what the receiver said:
`delivered`, `status_code`, `error` and the start of the response body. It
is logged with the other deliveries. A disabled webhook is not tested
(409).

`POST …/deliveries/{delivery_id}/replay` sends a logged delivery again now,
with the same body, signed with the time it is sent, and records the
attempt on that delivery. Use it after fixing a receiver that was failing.

## 3. Who delivers

The events server delivers: it takes every event the nodes post to it,
finds the tenant's enabled webhooks whose filter matches, and sends one
delivery each, retrying failures. The admin API manages webhooks and sends
tests and replays itself. Both read the same Postgres table
(`webhook_endpoints`) and log to `webhook_delivery_logs`.

Events relayed from another region reach WebSocket clients there but not
webhooks: a webhook fires once, in the region where the event happened
(`docs/MULTI_REGION.md`).

## 4. Configuration

The events server:

```toml
[webhook]
enabled = true                 # off unless set
database_url = "postgresql://…" # or FCP_EVENTS_WEBHOOK_DATABASE_URL
request_timeout_secs = 10
max_retries = 5
base_retry_delay_secs = 30
max_retry_delay_secs = 3600
max_concurrent = 50
retry_interval_secs = 60
allow_private_targets = false  # true only in a lab
```

With `enabled`, the events server refuses to start if the database cannot
be opened, rather than run with webhooks silently off. It opens webhook
secrets with the master key, so it needs the same encryption settings as
every database service (`FCP_ENCRYPTION_PROVIDER`, `FCP_MASTER_KEY_FILE`:
`docs/ENCRYPTION.md`).

The admin:

```toml
[webhooks]
allow_private_targets = false  # true only in a lab; set it as the events server's is
request_timeout_secs = 10      # for tests and replays
```

The compose stack ships webhooks on (`deploy/docker-compose/configs/fcp-events-server.toml`),
with private targets refused.

## 5. What changed from before

Before this release webhooks were managed on the events server's
`/v1/webhooks`, behind its service token, with the tenant named in the
request body and a caller-chosen secret; the signature was
`X-FCP-Signature: sha256=<hex>` over the body alone. Those routes are
deleted, and the old header is gone: every delivery carries the form
above — so does the call manager's own `[webhook]` forwarding, which also
drops its `X-Webhook-Signature`. A webhook made before keeps its secret and is signed the new way.
