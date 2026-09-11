# The GPU box

Phase 8 of `VIDEO_CONFERENCING.md` (the GPU backend) needs a machine
with a GPU, and the work is bursty: a spike to measure NVENC/NVDEC
against §9.4's CPU numbers, then development in sessions. The box is
therefore rented by the session: `deploy/gcp-gpu` is a Terraform module
for one Spot VM with an NVIDIA L4 on GCP, brought up in a minute and
torn down after, with everything that takes time to install kept on a
persistent disk that outlives the VM.

## What it is

| Piece | Choice | Why |
|---|---|---|
| VM | `g2-standard-4` (4 vCPU, 16 GB, one L4), Spot | The L4's NVENC encodes H.264, HEVC and AV1 and its NVDEC decodes VP8, VP9 and AV1: the design's codec set. A T4 has no AV1 encode. Spot is about a third of the on-demand price; preemption costs nothing since all state is on the disk |
| Disk | 200 GB `pd-balanced` boot disk, its own resource, never destroyed by Terraform | Holds Debian 12, the driver and CUDA, FFmpeg built with NVENC/NVDEC, the checkout and the cargo target directory. Tearing the VM down keeps it; the next `up` boots in seconds |
| Access | SSH through IAP only (`gcloud compute ssh --tunnel-through-iap`), OS Login | No public port; the ephemeral external address is for outbound package and crate fetches |
| Region | `us-central1` (zone `a`) | Offers the L4 in three zones; the project's quota allows one L4 in every region already |

Rough cost: about 0.25–0.35 USD an hour while up on Spot (0.70 on
demand), plus about 8 USD a month for the disk whether up or down.

## Using it

```bash
gcloud auth login                                # once
gcloud auth application-default login            # once, for Terraform
deploy/gcp-gpu/gpu-box up                        # create the VM; first time: 10–15 min of setup on the box
deploy/gcp-gpu/gpu-box log                       # watch the driver and FFmpeg install
deploy/gcp-gpu/gpu-box ssh                       # a shell
deploy/gcp-gpu/gpu-box sync                      # rsync this checkout to ~/fcp on the box
deploy/gcp-gpu/gpu-box down                      # destroy the VM, keep the disk
deploy/gcp-gpu/gpu-box status
```

The first boot of a fresh disk installs the NVIDIA driver (Google's
installer; it reboots once), the CUDA 12.4 toolkit from NVIDIA's apt
repository (the toolkit only, never the `cuda` meta-package, which would
replace the driver), the build dependencies, and FFmpeg 7.1 from source
with `nvenc`, `nvdec`, `cuvid`, `scale_cuda` and `libnpp` under
`/opt/ffmpeg-nv`. The `nv-codec-headers` (MIT) are pinned to the NVENC
SDK the 550 driver speaks, 12.2; a newer driver could take newer
headers. Proven 2026-09-11: `h264_nvenc`, `hevc_nvenc` and `av1_nvenc`
each encode 720p30 at about 170 fps on synthetic input, and a CUVID
decode feeds `scale_cuda` without leaving the device. `/etc/profile.d/
gpu-box.sh` puts it on `PATH`, `PKG_CONFIG_PATH` and
`LD_LIBRARY_PATH`. Rust is per user: `curl https://sh.rustup.rs -sSf | sh`
on the box, once, on the disk. Every later boot skips what is there.

`up` and `down` are `terraform apply -var up=true|false`; the disk is
`prevent_destroy`, so `terraform destroy` refuses until the block is
removed — delete the disk by hand when the work is over
(`gcloud compute disks delete fcp-gpu --zone us-central1-a`).

Terraform state stays local in `deploy/gcp-gpu` (ignored by git); one
person runs this, and losing the state costs an `import` of two
resources, not the box.

## Running phase 8's tests there

CI has no GPU: the `hwaccel` feature's tests run on the box.
`gpu-box sync` then, on the box, `cargo test -p forge-video-codecs
--features hwaccel` (the feature and its tests are phase 8's to write).
Nothing GPU-specific is checked into CI's path; a build without the
feature never touches CUDA.

## Not done

- A static external address, a private network with Cloud NAT, or a
  service account with fewer than the default scopes: this is a
  development box for one person, reachable only through IAP.
- Automatic shutdown when idle. Spot preemption is the only thing that
  stops a forgotten box; `gpu-box down` is the habit.
