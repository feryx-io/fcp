# fcp-loadgen

Put a node under load and write down what it did
(`docs/CAPACITY_AND_SURVIVABILITY.md` §8).

The harness drives [`fcp-testkit`]'s scripted phones — real SIP, real RTP
— many at a time, and reports **what the callers saw** beside **what the
node said about itself** over the same seconds. It is a measuring tool:
`publish = false`, in no release image, and it writes nothing to the
platform's database.

```bash
# A ramp until the node refuses or slows down
fcp-loadgen --profile call-ramp --sip 10.0.0.7:5060 --realm fcp.local \
  --users 50 --answer-extension-base 2000 \
  --cps 5 --cps-step 5 --cps-max 120 --rung-seconds 30 \
  --metrics-url http://10.0.0.7:8084/metrics --pid "$(pidof fcp-call-manager)" \
  --json ramp.json
```

## Profiles

| `--profile` | What it does |
|---|---|
| `register-storm` | Every device registers at once, then re-registers a quarter before its lifetime lapses. What a registrar meets when a subnet comes back. |
| `call-ramp` | Offered calls per second climb a ladder; the run stops at the first rung that refuses too many, answers too slowly, or drops a call. The last rung that held is the node's number. |
| `soak` | `--users` calls held up for the whole run, each with a tone both ways. What holding them costs in memory, CPU and media quality. |
| `mixed` | Calls and registrations at once, which is what a PBX actually sees. |

## Accounts

The accounts must exist already — seed them however the deployment
normally does. Callers are `{--user-prefix}{--first-user + i}`; with
`--answer-with` (default `peer`) the harness also starts a phone per
caller that registers and auto-answers, and `{callee}` in
`--destination` is that phone's extension (`--answer-extension-base + i`)
or, without one, its account name. Point `--destination` somewhere else —
a room, an IVR, a number on a trunk — to load that path instead.

## What it measures

- Offered, answered, refused and dropped calls, and calls per second.
- INVITE → first provisional, → `18x`, → `200 OK`: p50, p90, p99, max.
- REGISTER → its final response, the same way.
- Final responses by status code, so a refusal is a number with a reason.
- Per-leg RTP: loss, out-of-order, duplicates, jitter — and the worst leg
  and the silent legs, which an average hides.
- The node's `/metrics` and, when it runs on this machine, its resident
  memory and CPU (`--pid`), sampled every `--sample-seconds`.

## Budgets

Any of `--max-p99-invite-ms`, `--min-admitted-cps`, `--max-rtp-loss`,
`--max-refused-fraction`, `--max-dropped`, `--max-silent-legs` and
`--max-p99-register-ms` turns the run into a gate: the summary says which
held, and the process exits non-zero if one did not.

## Where the numbers come from

Real runs are manual, on hardware worth naming, and their numbers go in
`docs/CAPACITY_AND_SURVIVABILITY.md` §13 and `docs/DEPLOYMENT.md`. CI runs
one small profile (`tests/ci_profile.rs`) for **behaviour** — every call
answered, every leg hearing audio, every REGISTER answered — and asserts
no rate at all: a shared runner's throughput is a number about the runner.
