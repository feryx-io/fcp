#!/usr/bin/env python3
"""Persistent faster-whisper speech recognition worker — JSONL over stdio.

Loads the whisper model once at startup, then answers recognition requests
streamed as newline-delimited JSON on stdin (fcp-media's `local` STT
provider, `crates/fcp-media/src/stt/providers/local.rs`).

Protocol:
  stdout: {"ready": true, "duration_ms": 4200}                       (once)
  stdin:  {"wav": "/tmp/x.wav", "language": "en", "prompt": "sales"}\n
  stdout: {"ok": true, "text": "sales please", "confidence": 0.82}\n
  stdout: {"ok": false, "error": "..."}\n                         (a failure)
  stderr: diagnostics

Environment:
  WHISPER_MODEL   tiny | base | small | medium | large-v3   (default base)
  WHISPER_DEVICE  auto | cpu | cuda                          (default auto)

Dependencies: pip install faster-whisper
"""

import json
import math
import os
import sys
import time


def reply(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def main():
    started = time.monotonic()
    try:
        from faster_whisper import WhisperModel
    except ImportError as e:
        reply({"ready": False, "error": f"faster-whisper is not installed: {e}"})
        return 1

    model_name = os.environ.get("WHISPER_MODEL", "base")
    device = os.environ.get("WHISPER_DEVICE", "auto")
    compute_type = "float16" if device.startswith("cuda") else "int8"
    try:
        model = WhisperModel(model_name, device=device, compute_type=compute_type)
    except Exception as e:  # noqa: BLE001 - reported to the caller
        reply({"ready": False, "error": f"loading {model_name}: {e}"})
        return 1
    print(f"whisper {model_name} loaded on {device}", file=sys.stderr)
    reply({"ready": True, "duration_ms": int((time.monotonic() - started) * 1000)})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            segments, _info = model.transcribe(
                request["wav"],
                language=request.get("language"),
                initial_prompt=request.get("prompt"),
                vad_filter=False,
                beam_size=5,
            )
            segments = list(segments)
            text = " ".join(s.text.strip() for s in segments).strip()
            confidence = None
            if segments:
                mean_logprob = sum(s.avg_logprob for s in segments) / len(segments)
                confidence = round(min(1.0, math.exp(mean_logprob)), 3)
            reply({"ok": True, "text": text, "confidence": confidence})
        except Exception as e:  # noqa: BLE001 - reported to the caller
            reply({"ok": False, "error": str(e)})
    return 0


if __name__ == "__main__":
    sys.exit(main())
