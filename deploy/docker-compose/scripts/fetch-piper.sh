#!/bin/sh
# Piper and its default voice into the TTS images (docs/SPEECH.md):
#   fetch-piper.sh <piper dir> <voices dir>
# The release is the last of the MIT-licensed C++ engine (rhasspy/piper,
# archived 2025-10); the voice is LJSpeech, public domain. Both pinned by
# SHA-256.
set -eu
PIPER_DIR=$1 VOICES_DIR=$2
RELEASE=https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz
VOICE=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/ljspeech/medium/en_US-ljspeech-medium
# GitHub and Hugging Face answer a 5xx now and then; a nightly image
# build should not fail on one.
fetch() { curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 "$@"; }

fetch -o /tmp/piper.tgz "$RELEASE"
echo "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992  /tmp/piper.tgz" | sha256sum -c -
mkdir -p "$PIPER_DIR" "$VOICES_DIR"
tar xzf /tmp/piper.tgz -C "$PIPER_DIR" --strip-components=1
rm /tmp/piper.tgz

fetch -o "$VOICES_DIR/en_US-ljspeech-medium.onnx" "$VOICE.onnx"
fetch -o "$VOICES_DIR/en_US-ljspeech-medium.onnx.json" "$VOICE.onnx.json"
cd "$VOICES_DIR"
sha256sum -c - <<SUMS
6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22  en_US-ljspeech-medium.onnx
141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860  en_US-ljspeech-medium.onnx.json
SUMS
"$PIPER_DIR/piper" --help >/dev/null 2>&1
