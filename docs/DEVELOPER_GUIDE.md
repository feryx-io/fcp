# Developer guide

Building on FCP through its API: a token for your integration, the events
your service receives, a call flow that listens, and a call controlled
while it is up — each in TypeScript and in Python. The design is
`docs/DEVELOPER_SURFACE.md`.

The examples below are files in the repository: `sdk/typescript/examples/`
and `sdk/python/examples/`. CI runs every one of them against a live admin
and call manager (`crates/fcp-admin/tests/sdks.rs`), and a test fails if
this page quotes one differently from its file.

## 1. Before you start

- **The API.** Everything is under `https://<admin>/api/v1`. The contract
  is served at `/api/v1/openapi.json` and rendered at `/admin/api`; it is
  also `docs/openapi.json` in the repository.
- **A token.** Make your first token in the admin UI's API Tokens page
  (`/admin/tokens`; an end user's own at `/user/api-tokens`), or with
  `POST /api/v1/api-tokens` while signed in. A token holds
  only permissions its maker holds, answers to its maker, and is shown once
  (`docs/API_TOKENS.md`).
- **An SDK.**
  - TypeScript (Node 20 or later): `@fcp/sdk`, built from `sdk/typescript`
    with `npm ci && npm run build`.
  - Python (3.10 or later): `fcp-sdk`, from `sdk/python` with
    `uv pip install ./sdk/python` or `pip install ./sdk/python`.

  Both give you the token header, retries on 429 and 503, the API's errors
  as `FcpError` (with `status`, `code` and the request id), list paging,
  and webhook verification (`sdk/README.md`).
- **Running an example yourself:** set `FCP_URL` and `FCP_TOKEN` and run
  the file — `node examples/scoped_token.js` in `sdk/typescript` after a
  build, or `uv run python examples/scoped_token.py` in `sdk/python`.

## 2. A token for your integration

An integration gets a token of its own, holding what it needs and nothing
more: here, a CRM sync that reads users. Revoke it, or rotate its secret
with a grace period, from `/api/v1/api-tokens/{id}`.

<!-- example: sdk/typescript/examples/scoped_token.js -->
```js
// A scoped API token (docs/DEVELOPER_GUIDE.md §2): made with your own
// token, holding only what it names, and used by the integration it is for.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({ url, token }) {
  const fcp = createFcpClient({ baseUrl: url, token })

  // The secret is shown once: store it where the integration reads it.
  const made = await ok(fcp.POST('/api/v1/api-tokens', {
    body: { name: 'crm-sync', permissions: ['user_read'] },
  }))

  const crm = createFcpClient({ baseUrl: url, token: made.secret })
  const users = await ok(crm.GET('/api/v1/users', { params: { query: { limit: 50 } } }))
  return { token: made.token.id, users: users.data.map((u) => u.username) }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
```

<!-- example: sdk/python/examples/scoped_token.py -->
```python
"""A scoped API token (docs/DEVELOPER_GUIDE.md §2): made with your own
token, holding only what it names, and used by the integration it is for."""

import os

from fcp_api.api.api_tokens import api_tokens_create_api_token
from fcp_api.api.users import users_list_users
from fcp_api.models import CreateApiTokenRequest
from fcp_sdk import client, ok


def run(url: str, token: str) -> dict:
    fcp = client(url, token)

    # The secret is shown once: store it where the integration reads it.
    made = ok(
        api_tokens_create_api_token.sync_detailed(
            client=fcp,
            body=CreateApiTokenRequest(name="crm-sync", permissions=["user_read"]),
        )
    )

    crm = client(url, made.secret)
    users = ok(users_list_users.sync_detailed(client=crm, limit=50))
    return {"token": made.token.id, "users": [u.username for u in users.data]}


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
```

## 3. Events, by webhook

Your service receives the tenant's events as signed HTTP POSTs: a call
starting (`call.initiated`), its live transcript, emergency calls,
voicemail transcriptions, queues, conferences, recordings, records and
billing. Some events carry no tenant yet and so reach no tenant's webhook:
a call's later steps (`call.answered`, `call.ended`), media, IVR and flow
events, and voicemail's other events; a call's record (`cdr.written`)
says how it ended. A webhook names
a URL (HTTPS, and public, unless the platform allows private targets) and
the events it wants (`call.*`, or exact types from
`GET /api/v1/webhooks/event-types`). Each delivery carries
`X-FCP-Signature: t=<unix>,v1=<hex>`; check it before believing the body,
and refuse it once it is five minutes old (`docs/WEBHOOKS.md`).

<!-- example: sdk/typescript/examples/webhook_receiver.js -->
```js
// A webhook (docs/DEVELOPER_GUIDE.md §3): the tenant's events posted to
// your URL, each signed, and checked with verifyWebhook before it is
// believed.
import { createServer } from 'node:http'
import { createFcpClient, ok, verifyWebhook } from '../dist/index.js'

export async function run({ url, token, listen = { host: '127.0.0.1', port: 0 } }) {
  const fcp = createFcpClient({ baseUrl: url, token })
  const received = []
  let secret

  // Your receiver: refuse anything whose signature does not verify, or
  // that is older than five minutes (a replay).
  const server = createServer((req, res) => {
    let body = ''
    req.on('data', (chunk) => { body += chunk })
    req.on('end', () => {
      if (!verifyWebhook(secret, req.headers['x-fcp-signature'], body)) {
        res.statusCode = 401
        return res.end()
      }
      received.push(JSON.parse(body))
      res.end('ok')
    })
  })
  await new Promise((resolve) => server.listen(listen.port, listen.host, resolve))

  try {
    const made = await ok(fcp.POST('/api/v1/webhooks', {
      body: {
        url: `http://${listen.host}:${server.address().port}/fcp`,
        events: ['call.*', 'voicemail.transcribed'],
      },
    }))
    secret = made.secret // shown once, like a token's

    // Ask for a test delivery, signed like every other.
    const id = made.webhook.id
    const outcome = await ok(fcp.POST('/api/v1/webhooks/{id}/test', {
      params: { path: { id } },
      body: {},
    }))
    return { webhook: id, delivered: outcome.delivered, received: received.map((e) => e.event_type) }
  } finally {
    server.close()
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
```

<!-- example: sdk/python/examples/webhook_receiver.py -->
```python
"""A webhook (docs/DEVELOPER_GUIDE.md §3): the tenant's events posted to
your URL, each signed, and checked with verify_webhook before it is
believed."""

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

from fcp_api.api.webhooks import webhooks_create_webhook, webhooks_test_webhook
from fcp_api.models import CreateWebhookRequest, TestWebhookRequest
from fcp_sdk import client, ok, verify_webhook


def run(url: str, token: str, host: str = "127.0.0.1", port: int = 0) -> dict:
    fcp = client(url, token)
    received = []
    secret = {}

    # Your receiver: refuse anything whose signature does not verify, or
    # that is older than five minutes (a replay).
    class Receiver(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            if not verify_webhook(secret["value"], self.headers["X-FCP-Signature"], body):
                self.send_response(401)
                self.end_headers()
                return
            received.append(json.loads(body))
            self.send_response(200)
            self.end_headers()

        def log_message(self, *args):
            pass

    server = HTTPServer((host, port), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        made = ok(
            webhooks_create_webhook.sync_detailed(
                client=fcp,
                body=CreateWebhookRequest(
                    url=f"http://{host}:{server.server_port}/fcp",
                    events=["call.*", "voicemail.transcribed"],
                ),
            )
        )
        secret["value"] = made.secret  # shown once, like a token's

        # Ask for a test delivery, signed like every other.
        outcome = ok(
            webhooks_test_webhook.sync_detailed(made.webhook.id, client=fcp, body=TestWebhookRequest())
        )
        return {
            "webhook": made.webhook.id,
            "delivered": outcome.delivered,
            "received": [event["event_type"] for event in received],
        }
    finally:
        server.shutdown()


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
```

A delivery that fails is retried, logged under
`/api/v1/webhooks/{id}/deliveries`, and can be replayed.

## 4. A flow that listens

A flow is what a number does with a call: a graph of nodes from `answer`
(`docs/FLOWS.md`). This one answers, asks the caller what they want — by
voice or by keypad (`docs/SPEECH.md` §6) — posts what it heard to your
service, and routes on the answer; then a number is bound to it.

<!-- example: sdk/typescript/examples/listening_flow.js -->
```js
// A flow that listens (docs/DEVELOPER_GUIDE.md §4): a menu answered by
// speech or digits, the caller's words posted to your service, and a
// number that runs it.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({
  url, token, flowId = 'front-desk', number = '+15125550170', crm = 'https://crm.example.com',
}) {
  const fcp = createFcpClient({ baseUrl: url, token })

  const flow = await ok(fcp.POST('/api/v1/flows', {
    body: {
      id: flowId,
      name: 'Front desk',
      initial_node: 'answer',
      nodes: {
        answer: { type: 'answer', next: 'menu' },
        menu: {
          type: 'gather',
          input: 'dtmf_speech',
          prompt: { type: 'say', text: 'Say sales or support, or press 1 or 2.' },
          num_digits: 1,
          timeout: 5,
          hints: ['sales', 'support'],
          transitions: { 1: 'sales', 2: 'support' },
          speech_transitions: { sales: 'sales', support: 'support' },
          on_timeout: 'menu',
        },
        sales: {
          type: 'http',
          method: 'POST',
          url: `${crm}/leads`,
          send_variables: true, // caller, speech, speech_confidence, …
          response_variable: 'crm',
          next: 'to-sales',
          on_error: 'to-sales',
        },
        'to-sales': { type: 'dial', destination: '{{crm.owner_extension}}', next: 'bye', on_error: 'support' },
        support: { type: 'queue', queue_name: 'support', next_on_answer: 'bye' },
        bye: { type: 'hangup' },
      },
    },
  }))

  // A number whose calls run the flow.
  const phone = await ok(fcp.POST('/api/v1/phone-numbers', { body: { e164: number } }))
  await ok(fcp.POST('/api/v1/line-bindings', {
    body: { phone_number_id: phone.id, binding_type: 'ivr', flow_id: flow.id },
  }))
  return { flow: flow.id, version: flow.version, number: phone.e164 }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN }))
}
```

<!-- example: sdk/python/examples/listening_flow.py -->
```python
"""A flow that listens (docs/DEVELOPER_GUIDE.md §4): a menu answered by
speech or digits, the caller's words posted to your service, and a number
that runs it."""

import os

from fcp_api.api.flows import flows_create_flow
from fcp_api.api.line_bindings import line_bindings_create_line_binding
from fcp_api.api.phone_numbers import phone_numbers_create_phone_number
from fcp_api.models import CreateFlowRequest, CreateLineBindingRequest, CreatePhoneNumberRequest
from fcp_sdk import client, ok


def run(
    url: str,
    token: str,
    flow_id: str = "front-desk",
    number: str = "+15125550170",
    crm: str = "https://crm.example.com",
) -> dict:
    fcp = client(url, token)

    flow = ok(
        flows_create_flow.sync_detailed(
            client=fcp,
            body=CreateFlowRequest(
                id=flow_id,
                name="Front desk",
                initial_node="answer",
                nodes={
                    "answer": {"type": "answer", "next": "menu"},
                    "menu": {
                        "type": "gather",
                        "input": "dtmf_speech",
                        "prompt": {"type": "say", "text": "Say sales or support, or press 1 or 2."},
                        "num_digits": 1,
                        "timeout": 5,
                        "hints": ["sales", "support"],
                        "transitions": {"1": "sales", "2": "support"},
                        "speech_transitions": {"sales": "sales", "support": "support"},
                        "on_timeout": "menu",
                    },
                    "sales": {
                        "type": "http",
                        "method": "POST",
                        "url": f"{crm}/leads",
                        "send_variables": True,  # caller, speech, speech_confidence, …
                        "response_variable": "crm",
                        "next": "to-sales",
                        "on_error": "to-sales",
                    },
                    "to-sales": {
                        "type": "dial",
                        "destination": "{{crm.owner_extension}}",
                        "next": "bye",
                        "on_error": "support",
                    },
                    "support": {"type": "queue", "queue_name": "support", "next_on_answer": "bye"},
                    "bye": {"type": "hangup"},
                },
            ),
        )
    )

    # A number whose calls run the flow.
    phone = ok(
        phone_numbers_create_phone_number.sync_detailed(
            client=fcp, body=CreatePhoneNumberRequest(e164=number)
        )
    )
    ok(
        line_bindings_create_line_binding.sync_detailed(
            client=fcp,
            body=CreateLineBindingRequest(phone_number_id=phone.id, binding_type="ivr", flow_id=flow.id),
        )
    )
    return {"flow": flow.id, "version": flow.version, "number": phone.e164}


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
```

- **Speech** needs a recognizer on the call manager (`[stt]`,
  `docs/SPEECH.md`); without one a `dtmf_speech` Gather takes digits only.
- **The Http node** sends `send_variables` as JSON and unpacks a JSON
  answer into dotted variables, so `{"owner_extension": "1004"}` becomes
  `{{crm.owner_extension}}`. A status of 400 or above, or no answer, takes
  `on_error`.
- **Versions.** Every edit makes a version; a call runs the version it
  started on, and `/api/v1/flows/{id}/versions/{version}` reads any one
  back.
- **A transcript.** A `transcribe` node sends the call's live transcript
  to your webhook as `call.transcript` events.

## 5. A call, while it is up

A call's id comes with its events: the `call_id` of the `call.initiated`
delivery your webhook receives. With it, your service can
hold and resume a party, transfer it, redirect it into a flow, play a
prompt or speak text to it, and hang up (`docs/CALL_CONTROL.md`). The
token needs `call_control`.

<!-- example: sdk/typescript/examples/live_call.js -->
```js
// A call, live (docs/DEVELOPER_GUIDE.md §5): put the caller on hold, take
// them back, and end the call. The call's id comes from its events: the
// `call_id` of the `call.initiated` webhook delivery.
import { createFcpClient, ok } from '../dist/index.js'

export async function run({ url, token, callId }) {
  const fcp = createFcpClient({ baseUrl: url, token })
  const call = { params: { path: { call_id: callId } } }

  // The caller hears the hold music; the other party no longer hears them.
  const held = await ok(fcp.POST('/api/v1/calls/{call_id}/hold', { ...call, body: { party: 'caller' } }))
  await new Promise((resolve) => setTimeout(resolve, 1000))
  const resumed = await ok(fcp.POST('/api/v1/calls/{call_id}/resume', call))

  // Elsewhere: transfer, redirect into a flow, play a prompt, say text —
  // each one POST to /api/v1/calls/{call_id}/<action> (docs/CALL_CONTROL.md).
  const ended = await ok(fcp.POST('/api/v1/calls/{call_id}/hangup', call))
  return [held.status, resumed.status, ended.status]
}

if (import.meta.url === `file://${process.argv[1]}`) {
  console.log(await run({ url: process.env.FCP_URL, token: process.env.FCP_TOKEN, callId: process.argv[2] }))
}
```

<!-- example: sdk/python/examples/live_call.py -->
```python
"""A call, live (docs/DEVELOPER_GUIDE.md §5): put the caller on hold, take
them back, and end the call. The call's id comes from its events: the
``call_id`` of the ``call.initiated`` webhook delivery."""

import os
import sys
import time

from fcp_api.api.call_control import call_control_hangup, call_control_hold, call_control_resume
from fcp_api.models import HoldRequest
from fcp_sdk import client, ok


def run(url: str, token: str, call_id: str) -> list:
    fcp = client(url, token)

    # The caller hears the hold music; the other party no longer hears them.
    held = ok(call_control_hold.sync_detailed(call_id, client=fcp, body=HoldRequest(party="caller")))
    time.sleep(1)
    resumed = ok(call_control_resume.sync_detailed(call_id, client=fcp))

    # Elsewhere: transfer, redirect into a flow, play a prompt, say text —
    # each one POST to /api/v1/calls/{call_id}/<action> (docs/CALL_CONTROL.md).
    ended = ok(call_control_hangup.sync_detailed(call_id, client=fcp))
    return [held.status, resumed.status, ended.status]


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"], sys.argv[1]))
```

Hold, play and say need the call's media to pass through the node
(`[b2bua.media_anchoring]`); a transfer or a redirect answers 202 and runs
in the background, and the call's events say how it went.

## 6. When something is refused

Every error, from every route, has one shape:

```json
{"error": "Not Found", "code": "not_found", "details": "Call not found: call-…"}
```

with an `X-Request-Id` header naming the request in the logs. The SDKs
raise it as `FcpError`, and retry a 429 or a 503 after the `Retry-After` it
carries.

- Lists are `{data, next_cursor, total}`: pass `limit` (at most 1000) and
  the `cursor` you were given. The SDKs' `paginate` reads every page.
- Times in responses are RFC 3339, and ids carry their prefix (`call-…`,
  `tenant-…`); an id is accepted with or without it.
