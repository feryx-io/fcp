#!/usr/bin/env python3
"""Persistent Kokoro TTS worker — JSONL-over-stdio protocol.

Loads the Kokoro model once at startup, then processes synthesis requests
streamed as newline-delimited JSON on stdin, writing results to stdout.

Protocol:
  stdin:  {"text": "Hello", "voice": "af_heart", "lang": "a", "output": "/tmp/out.wav"}\n
  stdout: {"ok": true, "duration_ms": 850}\n
  stderr: diagnostic/progress messages (ignored by caller)

Dependencies: pip install kokoro soundfile numpy
System:       espeak-ng (for phonemization)
"""

import json
import os
import sys
import time

import numpy as np
import soundfile as sf

# Language mapping: BCP-47 → Kokoro language code
LANG_MAP = {
    "en-US": "a",
    "en-GB": "b",
    "es": "e",
    "es-ES": "e",
    "fr": "f",
    "fr-FR": "f",
    "ja": "j",
    "ja-JP": "j",
    "hi": "h",
    "hi-IN": "h",
    "it": "i",
    "it-IT": "i",
    "zh": "z",
    "zh-CN": "z",
    "pt": "p",
    "pt-BR": "p",
    # Single-letter codes pass through
    "a": "a",
    "b": "b",
    "e": "e",
    "f": "f",
    "h": "h",
    "i": "i",
    "j": "j",
    "z": "z",
    "p": "p",
}


def resample_24k_to_8k(audio: np.ndarray) -> np.ndarray:
    """Resample 24 kHz audio to 8 kHz using decimation by 3.

    Applies a simple low-pass FIR anti-alias filter before downsampling
    to avoid aliasing artifacts.
    """
    # Design a simple windowed-sinc low-pass filter
    # Cutoff at 4 kHz (Nyquist of 8 kHz target) relative to 24 kHz sample rate
    num_taps = 63
    cutoff = 4000.0 / 24000.0  # normalized cutoff
    n = np.arange(num_taps)
    center = (num_taps - 1) / 2.0
    # sinc filter
    h = np.sinc(2 * cutoff * (n - center))
    # Hamming window
    h *= np.hamming(num_taps)
    # Normalize
    h /= h.sum()

    # Apply filter
    filtered = np.convolve(audio, h, mode="same")

    # Decimate by 3 (24000 / 3 = 8000)
    return filtered[::3]


def resolve_device(requested: str = "auto") -> str:
    """Resolve the compute device for the model.

    Args:
        requested: "auto", "cuda", "cuda:0", "cuda:1", "cpu"

    Returns:
        The actual device string to pass to PyTorch.
    """
    import torch

    if requested == "auto":
        device = "cuda" if torch.cuda.is_available() else "cpu"
    else:
        device = requested

    if device.startswith("cuda") and not torch.cuda.is_available():
        print(f"kokoro_tts_worker: CUDA requested but not available, falling back to CPU", file=sys.stderr, flush=True)
        device = "cpu"

    if device.startswith("cuda"):
        idx = 0
        if ":" in device:
            idx = int(device.split(":")[1])
        gpu_name = torch.cuda.get_device_name(idx)
        gpu_mem = torch.cuda.get_device_properties(idx).total_mem / (1024 ** 3)
        print(f"kokoro_tts_worker: using GPU {idx}: {gpu_name} ({gpu_mem:.1f} GB)", file=sys.stderr, flush=True)
    else:
        print(f"kokoro_tts_worker: using CPU", file=sys.stderr, flush=True)

    return device


def main():
    # Read device preference from environment (set by fcp-tts service config)
    device_pref = os.environ.get("KOKORO_DEVICE", "auto")
    device = resolve_device(device_pref)

    print(f"kokoro_tts_worker: loading model on {device}...", file=sys.stderr, flush=True)
    t0 = time.monotonic()

    from kokoro import KPipeline

    # Cache pipelines per language code (each has its own G2P)
    pipelines = {}
    default_pipeline = KPipeline(lang_code="a", repo_id="hexgrad/Kokoro-82M")
    # Move model to the requested device
    if device != "cpu":
        default_pipeline.model = default_pipeline.model.to(device)
    pipelines["a"] = default_pipeline
    load_ms = int((time.monotonic() - t0) * 1000)
    print(f"kokoro_tts_worker: model loaded in {load_ms}ms on {device}", file=sys.stderr, flush=True)

    def get_pipeline(lang_code):
        if lang_code not in pipelines:
            print(f"kokoro_tts_worker: creating pipeline for lang={lang_code}", file=sys.stderr, flush=True)
            # Share the model across pipelines to save memory
            pipelines[lang_code] = KPipeline(lang_code=lang_code, repo_id="hexgrad/Kokoro-82M", model=default_pipeline.model)
        return pipelines[lang_code]

    # Signal readiness with device info
    ready_msg = json.dumps({"ready": True, "load_ms": load_ms, "device": device})
    sys.stdout.write(ready_msg + "\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            resp = {"ok": False, "error": f"Invalid JSON: {e}"}
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()
            continue

        text = req.get("text", "")
        voice = req.get("voice", "af_heart")
        lang_input = req.get("lang", "a")
        output_path = req.get("output", "/tmp/kokoro_out.wav")

        # Map language code
        lang = LANG_MAP.get(lang_input, lang_input)

        try:
            t_start = time.monotonic()

            pipeline = get_pipeline(lang)

            # Redirect stdout to stderr during synthesis to prevent any
            # library output (huggingface_hub downloads, tqdm, etc.) from
            # corrupting the JSONL protocol stream.
            real_stdout = sys.stdout
            sys.stdout = sys.stderr

            try:
                # Generate audio segments
                # KPipeline returns generator of Result objects with .audio property
                audio_segments = []
                for result in pipeline(text, voice=voice):
                    if result.audio is not None:
                        audio_segments.append(result.audio.numpy())
            finally:
                sys.stdout = real_stdout

            if not audio_segments:
                resp = {"ok": False, "error": "No audio generated"}
                sys.stdout.write(json.dumps(resp) + "\n")
                sys.stdout.flush()
                continue

            # Concatenate all segments
            full_audio = np.concatenate(audio_segments)

            # Resample from 24 kHz to 8 kHz
            audio_8k = resample_24k_to_8k(full_audio)

            # Convert to int16
            audio_int16 = np.clip(audio_8k * 32767, -32768, 32767).astype(np.int16)

            # Write 8 kHz 16-bit PCM mono WAV
            sf.write(output_path, audio_int16, 8000, subtype="PCM_16")

            duration_ms = int((time.monotonic() - t_start) * 1000)

            resp = {"ok": True, "duration_ms": duration_ms}

        except Exception as e:
            resp = {"ok": False, "error": str(e)}

        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
