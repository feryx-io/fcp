# Live call control

Acting on a call that is up, from the API (`docs/DEVELOPER_SURFACE.md`
phase 8): hold, resume, transfer, redirect into a flow, play a prompt,
say text, hang up.

## 1. The routes

All are `POST /api/v1/calls/{call_id}/<action>`. Each needs the
`call_control` permission and acts on the caller's tenant's calls only.
The call id is the one `GET /api/v1/calls` lists. A party is `caller`
(the call's A leg: the party that placed it, or the carrier's caller on
an inbound call) or `callee`.

| Action | Body | Answer | What happens |
|---|---|---|---|
| `hold` | `{"party": "caller"}` (the default) | 200 `held` | The party hears the hold music (theirs, the tenant's, or the built-in), and the other party stops hearing them. |
| `resume` | — | 200 `resumed` | The music stops and the two hear each other again. A call not held is left as it is. |
| `transfer` | `{"target": "1003", "party": "caller"}` | 202 `transferring` | The party is connected to the target, which is dialed back through the node and routed as any call to it would be: a user's plan, a number's binding, a trunk, a flow. The other party is dropped once the target answers. |
| `redirect` | `{"flow_id": "survey", "party": "caller"}` | 202 `redirecting` | The party is moved into one of the tenant's enabled flows, and the other party is dropped. |
| `play` | `{"prompt_id": "prompt-…", "to": "both"}` | 200 `playing`, `playback_id` | One of the tenant's prompts is played, to `both` (the default), the `caller` or the `callee`. |
| `say` | `{"text": "One moment", "language": "en-GB", "voice": "…", "to": "both"}` | 200 `playing`, `playback_id` | The text is spoken by the node's text-to-speech (`[media] tts_engine` or `tts_remote_url`); 503 on a node without one. |
| `hangup` | — | 200 `ended` | Both parties are sent BYE, and the record says `Hung up via the API`. |

A transfer or a redirect runs in the background: the 202 says it has
started, and the call's events say how it went. A target that does not
answer leaves the call as it was.

The errors come in the API's one shape (`code`, `error`, `details`):

- 404: a call no node holds, or not the caller's tenant's; a flow or
  prompt that is not the tenant's.
- 409: a call not yet answered; a call whose media does not pass through
  the node (hold, play and say need it: `[b2bua.media_anchoring]`); one
  party already held when the other is asked; a transfer already under
  way.
- 400: a party other than `caller` or `callee`; a transfer target that is
  not a user, an extension or a number (a URI to elsewhere is refused:
  the target is dialed at the node).

`DELETE /api/v1/calls/{call_id}` still ends a call on the node the admin
API runs beside; `POST …/hangup` reaches the node holding the call
wherever the API runs.

## 2. Where the call is

The admin API holds no calls. Each request goes to the call manager that
`[call_manager] url` names, or else to each healthy call manager in the
cluster until one has the call, with `[call_manager] auth_token` and the
caller's tenant; the node's answer comes back as it gave it. On the node
the routes are `/calls/{id}/<action>` behind its `[http] api_token`, with
the tenant in the body as `tenant_id`.

## 3. How a redirect reaches its flow

A redirect is a transfer to the node itself: the node issues a one-time
ticket naming the flow and the tenant, and dials the party back at
`flow~<ticket>`. The dial-back's INVITE redeems the ticket and starts the
flow for it. A ticket redeems once, lasts a minute, and is 128 random
bits, so nothing but the node's own dial-back reaches a flow this way.

## 4. Proved by

- `fcp-call-manager/tests/calling.rs`, against the binary:
  - `the_api_holds_plays_into_and_hangs_up_a_call`;
  - `the_api_transfers_a_call_and_redirects_it_into_a_flow`.
- `fcp-admin/tests/call_control.rs`: the admin binary forwarding each
  control to a stand-in node, with its token, tenant and fields, and the
  node's refusal in the API's error shape.
