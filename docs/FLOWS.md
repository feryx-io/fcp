# Flows: the Http node and versions

What a flow developer needs about the `Http` node and about flow versions
(`docs/DEVELOPER_SURFACE.md` phase 7b). Flows that listen — Gather by
speech and the Transcribe node — are in `docs/SPEECH.md` §6.

## 1. The Http node

```json
"lookup": {
  "type": "http",
  "method": "POST",
  "url": "https://crm.example.com/calls",
  "send_variables": true,
  "headers": {"Authorization": "Bearer {{crm_token}}"},
  "timeout": 5,
  "response_variable": "crm",
  "next": "greet",
  "on_error": "fallback"
}
```

- **The request.** `url`, header values and `body` take `{{variable}}`
  interpolation. With `send_variables` the body is instead a JSON object
  of the flow's variables, every value a string, sent as
  `application/json` unless a header names another type. The runner's own
  `__`-prefixed bookkeeping is left out.
- **The response.** The body is stored in `response_variable`
  (`http_response` by default) and the status in
  `<response_variable>_status`. A JSON body is also unpacked into dotted
  variables:

  | Response | Variables |
  |---|---|
  | `{"customer": {"name": "Ada", "vip": true}}` | `crm.customer.name` = `Ada`, `crm.customer.vip` = `true` |
  | `{"items": [{"id": "a1"}]}` | `crm.items.0.id` = `a1` |
  | `{"note": null}` | `crm.note` = empty |

  Strings are stored as they are, numbers and booleans as JSON, null as
  empty. At most 256 values are unpacked, 16 levels deep. Each request
  clears the previous response's dotted values first, so a value absent
  from this response is absent, not stale. Use them as `{{crm.customer.name}}`
  in a Say, or as a Branch's `variable`.
- **Errors.** `on_error` is taken when no response came (a refused
  connection, a timeout, a body over 1 MiB) and when the status is 400 or
  above. Without `on_error`, a request that could not be made fails the
  flow, and an error status goes on to `next` as it always did; the status
  variable says which it was.

## 2. Versions

Every edit of a flow makes a new version, and every version is kept
(`flow_versions`, migration 089, written by a trigger on `flows` so the
admin API, a site's sync and a seed all record one). Enabling or
disabling a flow does not make a version.

- `GET /api/v1/flows/{id}/versions` lists them, newest first.
- `GET /api/v1/flows/{id}/versions/{version}` reads one back.
- `PUT /api/v1/flows/{id}` answers with the version it stored.
- A call loads its flow once, when it starts, and runs that version to the
  end: an edit made while calls are in the flow reaches the next call.
  Its flow trace names the version (`flow_version`), which the route above
  reads back.
- Deleting a flow deletes its versions.

## 3. Removed: `webhook_url`

A flow's `webhook_url` was stored and returned but never read by
anything. It is gone from the API and the model; a client that still
sends it has it ignored. The column stays in `flows` until a later
release drops it, so a node of the previous release reading it during a
rolling upgrade still finds it. A flow that should tell a system what it
is doing calls out with an Http node. The `flow.*` events carry no tenant
yet, so no tenant webhook receives them.

## A mailbox's call handling

A mailbox can hand the calls it would take to a flow of its tenant's:
its `flow_id` (`PUT /api/v1/voicemail/mailboxes/{id}`, the mailbox's *Call
handling* on the Voicemail page). Where the call would have reached the
mailbox — no answer, busy, nobody's phone up — the call manager runs that
flow instead. It can say something, ring someone else (Dial), send the
call to a queue, or still take a message: its Voicemail node records into
the mailbox it names. The flow is an ordinary flow, on the Flows page with
the tenant's others. An empty `flow_id` gives the call back to the
mailbox. A flow's own Dial to the same user does not loop: the plan's
voicemail answers it 480 (`FLOW_DIAL_HEADER`).

The flows a node builds for itself — the voicemail server's per-mailbox
record flow, `*98`'s access flow, hoteling's login — are not stored:
`fcp_flow::BuiltinFlows` holds them in memory, in front of the tenants'
flows, and a call's are let go when the call ends (migration 117 removed
the ones written before, `docs/DEEP_TESTING.md` D8).

