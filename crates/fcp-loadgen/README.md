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

## The soak

`fcp-soak` runs the whole stack for hours and asks whether anything it
holds kept growing (`docs/DEEP_TESTING.md` D6). It seeds its own tenant
through the admin API, then runs three loads at once: calls between
phones, a busy conference room, and a room that empties and is made
again. API traffic runs beside them. Every 30 s it samples each
`fcp-*` service's memory, descriptors, UDP sockets and threads, plus
Redis and every Postgres table. It exits non-zero on a refused,
dropped or silent call, an API 5xx, or growth that has not levelled off
by the end of the run and is not named in `EXPECTED_GROWTH`.

```bash
# Against the compose stack on this host, for four hours
fcp-soak --compose-dir deploy/docker-compose --minutes 240 --report soak.json
```

The compose project's `.env` gives it the admin token, the Postgres
password and the call manager's token. Set `retain_hours = 1` under
`[cdr]` in `configs/fcp-jobs.toml` first: the CDR stream keeps a day of
records for replay, which a run of a day or less sees only grow (#345).
The workflow does the same. `.github/workflows/soak.yml` runs
it weekly against the nightly images. `tests/soak_stack.rs` runs the
same soak against the binaries of this checkout (`FCP_SOAK_SECONDS`).

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

## How big a run one process can drive

A phone is a SIP endpoint with sockets and timers, and they all live in
the harness's process. Past roughly **100 phones on a small box** the
generator becomes the thing being measured: the same register storm,
against a node that stayed at 1 % CPU and answered every request,
measured p50 10 ms at 30 phones, 18 ms at 100, and **1525 ms at 300**
(`docs/CAPACITY_AND_SURVIVABILITY.md` §13, phase 4).

So: keep one process under about 100 phones on hardware like that, run
several processes for more, and put them on a different machine from the
node before quoting any number. A run whose latencies rise while the
node's CPU stays flat is measuring itself.

## Where the numbers come from

Real runs are manual, on hardware worth naming, and their numbers go in
`docs/CAPACITY_AND_SURVIVABILITY.md` §13 and `docs/SINGLE-NODE-SETUP.md` §12. CI runs
one small profile (`tests/ci_profile.rs`) for **behaviour** — every call
answered, every leg hearing audio, every REGISTER answered — and asserts
no rate at all: a shared runner's throughput is a number about the runner.
