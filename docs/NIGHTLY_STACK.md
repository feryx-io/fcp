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

`.github/workflows/images.yml` builds all fourteen images every night,
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
| 3 | **A test's Redis is its own** (§1.3). `private_redis_url` flushes the database it hands out — test binaries run one at a time, and within one binary the indexes are distinct — so a leak from an earlier binary reaches nobody; the helper says so, and the pipeline test's own deletion goes. |
| 4 | **What the nightly finds, written down.** The first runs' findings fixed or recorded; `docs/TEST_FLOOR.md` gets the live stack as its top layer; the compose README and the runbook point at the images. |

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

## 5. Not in scope

The TTS image in the live stack (its PyTorch provider; the profile
stays off, as the stack test leaves it). The GPU overlay. A multi-node
stack in CI — the floor's two-node harness covers that over binaries. A
fuzz target for FCP's own parsers (FCP parses configuration and JSON;
the wire formats are the submodules').
