# Video on a GPU

How a conference node runs its video rooms on an NVIDIA GPU: what to
build, what the machine needs, what the node does with the device, and
what to read off it. The design is `docs/VIDEO_CONFERENCING.md` §15.7
(phase 8); the development box for it is `docs/GPU_BOX.md`.

## What a GPU node does

A node whose `[conference] video_device` names a GPU places every video
room on it end to end: each camera is decoded on NVDEC into device
memory, the composite is drawn there by a filter graph (`scale_cuda`
and `overlay_cuda`, with the chrome — rings, bars, avatars, name bands —
painted by the same code as the host compositor's and uploaded when it
changes), and each flavor is encoded on NVENC. Nothing but RTP crosses
the bus for a steady room.

What the device lacks runs on the host, one copy per frame:

| Stage | On the device | Falls back to the host when |
|---|---|---|
| Decode | H.264, HEVC, VP8, VP9, AV1 (NVDEC) | the codec has no hardware decoder; the host decodes and uploads each frame |
| Composite | the filter graph | the graph does not build; the host composes from downloaded tiles |
| Encode | H.264, HEVC, AV1 (NVENC) | the codec is VP8 or VP9 — a browser that offered only VP8, a WebM recording; the canvas is downloaded once a tick for all such flavors |

The room's status says where each stage runs (`GET
/conferences/{room}/video`: `device` on the room, each output, each
flavor and each source; the admin UI's Live Video dialog shows the
device and what fell back), and
`forge_conference_video_frames_moved_total{room_id,to}` counts the
copies.

## Building

The `video-hwaccel` feature of `fcp-conference-server` pulls in
`forge-video-hw`, whose bindings are generated against the FFmpeg the
build machine has. That FFmpeg must be 7.x with `--enable-ffnvcodec`,
`--enable-nvenc`, `--enable-nvdec`, `--enable-cuvid` and
`--enable-cuda-llvm` (the CUDA filters compiled by clang; no nvcc, no
CUDA toolkit at build time, no libnpp — LGPL, design decision 4). The
`nv-codec-headers` must match the NVENC SDK the driver speaks: 12.2
(`n12.2.72.0`) for the 550 driver series.

```bash
cargo build --release -p fcp-conference-server \
  --features video-hwaccel,video-vpx,video-openh264,video-dav1d
```

`video-all` is the host's native bindings and does not include
`video-hwaccel`. `deploy/docker-compose/Dockerfile.conference.nvidia`
builds the FFmpeg and the binary the way described;
`deploy/gcp-gpu/scripts/startup.sh` does the same on the development
box.

## Running

The machine needs the NVIDIA driver (550 or later) and, in a container,
the [NVIDIA container toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/),
which injects `libcuda`, `libnvcuvid` and `libnvidia-encode` when the
container asks for `NVIDIA_DRIVER_CAPABILITIES=compute,video,utility`
(the image sets it). No CUDA toolkit is needed at run time.

```toml
[conference]
video_device = "cuda:0"     # the first NVIDIA device; "host" is the default
```

With Compose:

```bash
cd deploy/docker-compose
docker compose -f docker-compose.yml -f docker-compose.gpu.yml up -d fcp-conference
```

The override builds the NVIDIA image, reserves one GPU for the container
and mounts `configs/fcp-conference-server.gpu.toml`, which names the
device.

A device that does not open at start — no driver, a build without the
feature, the wrong device name — leaves the node on the host with a
warning in the log; nothing else changes. `video_device` is validated at
start: `host`, or `backend:address` with a backend of `cuda`, `vaapi`
or `qsv` (only CUDA is built and measured; the others compile).

## Recording

A room on a GPU records to **MP4** on NVENC (design §15.9): with the
`recording-mp4` feature — the NVIDIA image has it — and
`recording_format = "auto"` (the default) or `"mp4"`, the recording's
flavor is H.264 encoded on the device like any subscriber's, and
`forge-mp4` writes it fragmented, so the file plays to its last
fragment if the node dies and opens in every player. `recording_format
= "webm"` or `recording_video_codec = "vp8"` still records, but the
composite is then VP8 encoded on the host.

## What the node prices and advertises

At start the node prices each device it has (design §9.3, §15.7):

- the **host** by running each codec on its codec pool for a share of
  `video_benchmark_seconds`, as before;
- the **GPU by saturation** (decision 7): each stage of each codec is
  run with 1, 2, 4, 8 streams of 720p30 until the summed frame rate
  stops growing, and the cost constant is what makes that many
  real-time streams fill the device. The composite is timed on the
  device's compositor with a nine-tile 720p grid.

A GPU is three engines that saturate separately — NVENC, NVDEC and
the compute cores that run the composite — so its budget is three:
`encode`, `decode` and `compose`, each of `1000 × video_budget_headroom`
work units, and a stage's units are its share of that engine's
saturated throughput. On an L4, sixty real-time 720p30 H.264 encodes
saturate NVENC, so one 720p15 H.264 encode costs nine units of the
encode engine's 800. The composite is priced the same way, by
saturation: several graphs run at once until the summed tick rate stops
growing — on an L4 one graph already renders nine 720p tiles 1,737
times a second, so a 720p15 composite costs ten units of the compose
engine's 800. A join is admitted only when every engine it touches has room,
and a share, which is a decode alone, is admitted while the compose
engine is out.

Each step of the saturation run lasts at least a second after a
one-second warm-up whatever `video_benchmark_seconds` says (short steps
overshoot: a six-second run without the warm-up read 109 real-time
H.264 encodes where long steps measure 62), so a GPU node's benchmark
takes about eighty seconds for three encoders, five decoders and the
composite, and the log says what it took.

On the wire, a GPU's budget line carries `left` and `total` of its
tightest engine — what a node from before engines were advertised
reads — and `engines` with the three; placement weighs the encode
engine's `left`, and the admin cluster page shows all three.

A join is priced on the device each of its stages would run on — the
decode and the composite on the GPU, the encode on the GPU when it has
the codec and on the host otherwise — and admitted only when it fits on
every device it touches; a refused join is a `503` before the 180, as
on a host node, and holds nothing. Every few seconds the rooms are
re-priced from what each stage actually runs on.

In cluster membership the node advertises **two budgets**: the GPU's,
with the codecs the GPU *encodes* (decision 5) and its engines, and the
host's beside it. Placement prefers a device only when it encodes the
meeting's first codec, so a VP8-first meeting is not sent to a GPU node
for a budget it would spend on that node's host; an H.264-first meeting
takes the GPU whose encode engine has the most left. `GET /status`
reports every device under `video_capacity.devices` (total, committed,
reserved, left, the engines, the codecs it encodes and decodes, the
cost table and where it came from) with the room device's figures at
the top; the admin cluster page shows one budget line per device with
its engines.

## Testing on hardware

The tests that need the GPU skip without one. On the development box:

```bash
deploy/gcp-gpu/gpu-box up
deploy/gcp-gpu/gpu-box test          # forge-video-hw, then the conference server on cuda:0
deploy/gcp-gpu/gpu-box down
```

`gpu-box test` syncs the checkout, runs forge-media's `forge-video-hw`
tests (codecs, parity of the device compositor against the host, the
composite's cost, a room on the device) and the conference server's
tests built with `video-hwaccel` and `FCP_TEST_VIDEO_DEVICE=cuda:0`,
which adds the node test: the device opens as the rooms' backend, the
saturation benchmark prices it, and the budgets come out as described.

## Numbers (NVIDIA L4, driver 550, FFmpeg 7.1)

| | |
|---|---|
| H.264 encode, real-time 720p30 streams at saturation | ~62 |
| AV1 encode | ~79 |
| HEVC encode | ~57 |
| Decode (H.264 / AV1 / HEVC) | ~234 / 243 / 285 |
| Composite, nine 720p tiles, steady state | 0.58 ms a tick, 1,737 ticks/s on one graph (host: 3.9 ms) |
| A 720p15 H.264 join, in GPU work units of 800 per engine | 9 encode, 2 decode, 10 compose |
| Device against host compositor, luma PSNR over the golden scenes | 39.5 dB or better |

The composite's steady-state cost is far below its first render's (the
graph build and the chrome planes' uploads), which is why the saturation
run warms up before it counts; the compose engine holds about
fifty-eight rooms' 30 fps composites of nine tiles.

## What is not done

VAAPI and AMF beyond compiling; a room moving between devices;
more than one GPU per node (`cuda:1` names a second device, but a
node's rooms are on one); GPU-side RTP or SRTP; a GPU runner in CI.
