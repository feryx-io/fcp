# The nightly proves the shipped stack

Agreed 2026-09-25, after backup, restore and upgrade closed
(`docs/BACKUP_RESTORE_UPGRADE.md`). Nothing on the roadmap is left; what
the tree records as open is operational, and one item has been red
since it was added.
This is the survey, the phases and the questions, settled as the
project was chosen.

## 1. What the survey found

### 1.1 Fuzzing has never run in CI

siphon-rs and forge-media each fuzz their parsers nightly
(`.github/workflows/fuzz.yml` in both: cargo-fuzz on a nightly
toolchain, a cached corpus seeded from the crates' own tests, every
target for ten minutes, crash artifacts uploaded). forge-media's
workflow has run every night since 2026-09-11 and siphon-rs's since
2026-09-23, and not one run has fuzzed anything: every target fails to
build, first with

```
error: sanitizer is incompatible with statically linked libc, disable it using `-C target-feature=-crt-static`
```

and, with that flag given, with `can't find crate for core`. The build
command in the log says why: `cargo build --target
x86_64-unknown-linux-musl`. cargo-fuzz builds for the triple it was
itself built for, and the prebuilt binary the install action fetches is
a musl one, so without an explicit `--target` every run went for musl —
which the toolchain has no standard library for, and which links libc
statically, where a sanitizer is refused. The fuzz crates were written
and run on developers' machines, where cargo-fuzz was built from source
for the host. Two things made the outage invisible: the per-target loop
counts a target that did not *build* as a target that *crashed*
(forge-media's run ends with `fuzzing found crashes in:` and every one
of its eleven targets), and nothing in FCP's own CI looks at the
submodules' nightly runs — FCP is green while the parser hardening it
relies on (`docs/VIDEO_CONFERENCING.md` phase 6, siphon-rs's `fuzz/`)
has never run where it was meant to.

### 1.2 The images are built nightly and never run

`.github/workflows/images.yml` builds all fifteen images (the link's since hybrid phase 7) every night,
one runner each, and stops: the images are discarded with the runners.
What proves the compose stack is `crates/fcp-call-manager/tests/compose_stack.rs`,
which starts the *binaries* with the compose file's configurations,
environment and tokens against the test's Postgres and Redis. It cannot
see what only the containers have: each image's entrypoint and
filesystem, `depends_on` and the health checks, the `certs` service
running as a container, host networking for the SIP and media services,
the proxy, and `up --build` ordering. `backup.sh` and `restore.sh` were
proved live on 2026-09-25 — by hand, on a developer's box, with three
containers and a local binary. No automated run has ever brought the
built images up, taken a call, and restored a backup.

### 1.3 Tests share one Redis

`fcp_testkit::cluster::private_redis_url` hands each caller a Redis
database index — a random base per process plus a counter, modulo
fifteen — so two test binaries land on the same index about one run in
fifteen, and every node writes under the one prefix `fcp:`, so there is
no namespace to give a test either. On 2026-09-25 the pipeline test in
`fcp-jobs` stored and charged records another binary's call manager had
left on `fcp:cdrs` (fixed at the source, v0.25.262, and the test now
deletes the stream first). Nothing stops the next such leak.

### 1.4 What is sound

The two fuzz workflows are well shaped (corpus cache, seeds from the
crates' tests, a loop that keeps going after one target fails, artifacts
kept). `images.yml` already builds every image with the submodules.
`compose_stack.rs` and the testkit's phone (`fcp_testkit::Phone`, SIP and
SIPS, media, the admin API helpers) can drive a running stack as well as
one they started. `backup.sh` and `restore.sh` work; the live run found
nothing to change.

## 2. Phases

One PR each, in order. Each is checked by CI: a workflow change by the
run it triggers, a test by running it.

| Phase | What |
|---|---|
| 1 | ✅ v0.25.265 — **Fuzzing running and watched** (§1.1). Both fuzz steps name the host triple (`--target x86_64-unknown-linux-gnu`); each target is built before it is run, and a target that does not build is reported as such, apart from one that crashes. FCP's weekly CI gets a `submodule-nightlies` job: the latest scheduled Fuzz run of each submodule, from the GitHub API, must have succeeded and be less than two days old. |
| 2 | ✅ v0.25.266 — **The images published, then run** (§1.2). `images.yml` pushes each image to `ghcr.io/thevoiceguy/fcp-<name>:nightly` (and `:<sha>`); the compose file names `image: ghcr.io/thevoiceguy/fcp-<name>:${FCP_IMAGE_TAG:-local}` beside each `build:`, so a checkout still builds and CI pulls. A `stack` job after the matrix pulls the fourteen, brings the stack up with the shipped `.env` and `--no-build`, waits for every health check, and runs `crates/fcp-call-manager/tests/compose_live.rs` (gated on `FCP_LIVE_STACK`): a phone registers over SIP and SIPS, calls a room, is on the admin's node list; then `backup.sh`, `down -v`, `restore.sh`, `up -d --no-build`, and the phone registers again with the sealed password. Operators get pullable images: `FCP_IMAGE_TAG=nightly docker compose up -d --no-build`. |
| 3 | ✅ v0.25.287 — **A test's Redis is its own** (§1.3). `private_redis_url` flushes the database it hands out — test binaries run one at a time, and within one binary the indexes are distinct — so a leak from an earlier binary reaches nobody; the helper says so, and the pipeline test's own deletion goes. As built: `emptied` flushes the database before `private_redis_url` returns it, a binary asking for a sixteenth is refused, and `cdr_pipeline.rs` no longer deletes `fcp:cdrs` itself; `private_redis_tests` proves two acquisitions differ and a database left dirty is handed out empty. |
| 4 | ✅ v0.25.288 — **What the nightly finds, written down.** The first runs' findings fixed or recorded; `docs/TEST_FLOOR.md` gets the live stack as its top layer; the compose README and the runbook point at the images. As built: §6 below; the four findings since phase 2 merged fixed (the restore's Postgres race, the site step's `jq`, the compose health checks, forge-bfcp's parser in forge-media #168) and one recorded for the owner (the packages are private). |

## 3. Questions

Five, settled on 2026-09-25, each on its recommendation.

**Q1. The fuzz builds: name the target, or build cargo-fuzz from
source?** — *Name the target.* One flag on two commands; building
cargo-fuzz from source on every run costs minutes for the same result,
and a musl-built cargo-fuzz is fine once told what to build for.

**Q2. Where do the images go?** — *GHCR, under the repository.* The
repository is public, so the packages are; `GITHUB_TOKEN` with
`packages: write` pushes them, no secret to keep. `nightly` moves, the
sha tag does not.

**Q3. Does the stack job build or pull?** — *Pull.* The matrix builds
the fourteen in parallel on fourteen runners; one runner building them
in sequence would take hours. The stack job pulls what the matrix
pushed a few minutes before.

**Q4. The live test: cargo, or a shell script?** — *Cargo, in the
testkit.* The phone, TLS, the admin API helpers and the assertions
exist; a script would reinvent them in curl and sipp. It is gated on an
environment variable so `--all-targets` skips it everywhere but the
stack job.

**Q5. Redis isolation: flush, or a key prefix per test?** — *Flush.* A
prefix would touch every binary's configuration and the compose files;
a flush at acquisition changes one helper and leaves the nodes as they
are.

## 4. Testing

- Phase 1: a `workflow_dispatch` run of each fuzz workflow on its branch
  (twenty seconds per target) before it merges; FCP's new job run by
  hand once against the fixed nightlies.
- Phase 2: the stack job itself, run by hand from the branch; the live
  test also runs on a developer's Docker host against locally built
  images.
- Phase 3: a test in the testkit that two acquisitions in one process
  get different indexes and that an acquisition finds its index empty.
- Phase 4: the documents match what the job does.

## 6. What the nightly found

**Before phase 2 merged** (the branch's own runs, 2026-09-25): seven
fixes, all in the phase 2 PR — the proxy's health check probing IPv6,
an id the live test read in the wrong shape, the setup guide's extension
shape, an INVITE dialled at the node's address naming no tenant (a
registered caller's tenant now comes from its registration), a user
created through the API with no device ringing nowhere (their own
address is rung), a dead registration's CANCEL holding up the winner's
200 (the losers' CANCELs spawned), and every service racing to create
the first data key on an empty ring (an advisory lock, and
`start_period` on the slower health checks).

**After it merged**, one scheduled run a night:

| Night | What failed | What it was | Where fixed |
|---|---|---|---|
| 2026-09-26 | every job, unstarted | the account's Actions spending limit | the owner's billing — nothing in the code |
| 2026-09-27 | the stack job, *Back up, drop, restore* | `restore.sh` waited for Postgres with `pg_isready` on the socket: on an empty volume the image's temporary init server answers there, then restarts — the loop passed and the check after it did not | v0.25.288: `pg_isready -h 127.0.0.1` in `restore.sh` and in both compose files' Postgres health checks |
| 2026-09-27, -28 | the stack job, *A site for the live tenant* | the step's `jq` read `(.tenants // .)[]`, which is an error when the admin API answers a bare array — so the site stack has never run in the nightly | v0.25.288: `if type == "array" then . else .tenants end` |
| 2026-09-27, -28 | forge-media's Fuzz, `bfcp_parse` | a grouped BFCP attribute whose last child's padding was cut short was read, but could not be written back within its one-octet length | forge-media #168 (forge-bfcp 0.1.1), the input kept as a regression test |

| 2026-09-28 (the phase 4 branch's own run) | the stack job, *The site, enrolled and in step* — the first time the site stack ran | the testkit took a health check's in-container URL as the host's: the site's admin (8080 inside, 18080 published) was checked on 8080 — the *cloud's* admin, a false pass — and the site's link, which published nothing, was refused | v0.25.289: `fcp_testkit::compose` maps a health URL through the service's published ports; the site's link publishes its health on the host's loopback (`127.0.0.1:18096`) |

| 2026-09-29 (the site fix's own run) | the stack job, *The site, enrolled and in step*, "the site caught up with the cloud" | a product bug: the link's sync client had the link's 10 s request timeout while asking the cloud to hold the change feed 25 s, so every quiet stretch ended as a timeout and a standing `sync_error` (the tests' feed wait was 5 s, under the timeout, so none ever saw it) | v0.25.290: the sync client's timeout is the request timeout plus the feed wait; the test harness's link gets a 3 s timeout and `apply.rs` sits out a whole idle poll |

| 2026-09-29 (the sync fix's own run) | the stack job, *Back up the site* | `backup.sh` found the checkout's `Cargo.toml` relative to `COMPOSE_DIR`, a level too shallow for the site project, after its fresh-container version query failed | v0.25.290: the running admin is asked first (the stack is up), and the checkout is found from the script's own place |

siphon-rs's nightly fuzzing has been green every night since phase 1.

**Open, for the owner:** the GHCR packages are private — an anonymous
pull is refused (401) — so `FCP_IMAGE_TAG=nightly docker compose pull`
works only after `docker login ghcr.io`. Making the fourteen
`fcp-<service>` packages public (package settings, *Change visibility*)
is what the compose README's "Pull instead of build" assumes.

## 5. Not in scope

The TTS image in the live stack (its PyTorch provider; the profile
stays off, as the stack test leaves it). The GPU overlay. A multi-node
stack in CI — the floor's two-node harness covers that over binaries. A
fuzz target for FCP's own parsers (FCP parses configuration and JSON;
the wire formats are the submodules').
