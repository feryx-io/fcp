#!/bin/bash
# Runs on every boot as root. Each step is skipped once done, so a box
# whose disk already has everything boots in seconds; a fresh disk takes
# ten to fifteen minutes (the driver install reboots once).
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
LOG=/var/log/gpu-box-setup.log
exec >>"$LOG" 2>&1
echo "== gpu-box startup $(date -Is)"

# 1. The NVIDIA driver and CUDA toolkit, Google's way. The installer
#    reboots the machine when it needs to; this script runs again after.
if ! command -v nvidia-smi >/dev/null 2>&1; then
  echo "-- installing the GPU driver"
  apt-get update -q
  apt-get install -y -q python3 curl
  curl -fsSL -o /opt/install_gpu_driver.py \
    https://raw.githubusercontent.com/GoogleCloudPlatform/compute-gpu-installation/main/linux/install_gpu_driver.py
  python3 /opt/install_gpu_driver.py
  # A driver install ends in a reboot request; make sure of it.
  reboot
fi
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader || true

# 1b. The CUDA toolkit, which Google's installer leaves out: nvcc and
#     libnpp for FFmpeg's CUDA filters. The toolkit alone, never the
#     `cuda` meta-package, which would pull a driver over Google's. 12.4
#     is the newest the 550 driver runs.
if [ ! -x /usr/local/cuda/bin/nvcc ]; then
  echo "-- installing the CUDA toolkit"
  curl -fsSL -o /tmp/cuda-keyring.deb \
    https://developer.download.nvidia.com/compute/cuda/repos/debian12/x86_64/cuda-keyring_1.1-1_all.deb
  dpkg -i /tmp/cuda-keyring.deb
  apt-get update -q
  apt-get install -y -q --no-install-recommends cuda-toolkit-12-4
fi
/usr/local/cuda/bin/nvcc --version | tail -1 || true

# 2. What FCP and forge-media build with, plus what FFmpeg with NVENC
#    needs. Idempotent: apt skips what is there.
if [ ! -f /opt/gpu-box/.deps ]; then
  echo "-- installing build dependencies"
  apt-get update -q
  apt-get install -y -q \
    build-essential clang cmake pkg-config nasm yasm git curl unzip \
    libssl-dev libclang-dev libopus-dev libvpx-dev \
    libdav1d-dev libnuma-dev python3-pip rsync
  mkdir -p /opt/gpu-box && touch /opt/gpu-box/.deps
fi

# 3. FFmpeg with NVENC/NVDEC (the `hwaccel` feature's device contexts),
#    built once from source under /opt/ffmpeg-nv. The nv-codec-headers
#    are MIT and give ffmpeg its NVIDIA entry points.
if [ ! -x /opt/ffmpeg-nv/bin/ffmpeg ]; then
  echo "-- building FFmpeg with NVENC/NVDEC"
  export PATH=/usr/local/cuda/bin:$PATH
  rm -rf /opt/src && mkdir -p /opt/src && cd /opt/src
  # The headers' SDK must be one the driver speaks: the 550 driver has
  # NVENC API 12.2, and the headers' main branch is already past it.
  git clone --depth 1 --branch n12.2.72.0 \
    https://git.videolan.org/git/ffmpeg/nv-codec-headers.git
  make -C nv-codec-headers install PREFIX=/usr/local
  git clone --depth 1 --branch n7.1 https://git.ffmpeg.org/ffmpeg.git
  cd ffmpeg
  # The build that ships (design §15.7, decision 4): LGPL, the CUDA
  # filters compiled by clang (`cuda-llvm`, no nvcc, no toolkit needed
  # at build time), no libnpp. What is measured here is what runs
  # there. Debian 12's clang 14 knows sm_86 at most; the L4 (sm_89)
  # runs that PTX through the driver's JIT.
  ./configure --prefix=/opt/ffmpeg-nv --enable-shared \
    --enable-ffnvcodec --enable-cuda-llvm --nvccflags="--cuda-gpu-arch=sm_86 -O2" \
    --enable-nvenc --enable-nvdec --enable-cuvid \
    --enable-libvpx --enable-libopus --enable-libdav1d \
    --disable-doc --disable-programs --enable-ffmpeg
  make -j"$(nproc)"
  make install
  echo /opt/ffmpeg-nv/lib > /etc/ld.so.conf.d/ffmpeg-nv.conf && ldconfig
  /opt/ffmpeg-nv/bin/ffmpeg -hide_banner -encoders | grep -E "nvenc" || true
fi

# 4. Rust for every OS Login user is theirs to install (rustup is per
#    user); a profile snippet points at the FFmpeg build so pkg-config
#    and the linker find it.
cat >/etc/profile.d/gpu-box.sh <<'PROFILE'
export PATH=/opt/ffmpeg-nv/bin:/usr/local/cuda/bin:$HOME/.cargo/bin:$PATH
export PKG_CONFIG_PATH=/opt/ffmpeg-nv/lib/pkgconfig:${PKG_CONFIG_PATH:-}
export LD_LIBRARY_PATH=/opt/ffmpeg-nv/lib:/usr/local/cuda/lib64:${LD_LIBRARY_PATH:-}
PROFILE
echo "== gpu-box ready $(date -Is)"
