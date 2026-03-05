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
    "ko": "k",
    "ko-KR": "k",
    "zh": "z",
    "zh-CN": "z",
    "pt": "p",
    "pt-BR": "p",
    # Single-letter codes pass through
    "a": "a",
    "b": "b",
    "e": "e",
    "f": "f",
    "j": "j",
    "k": "k",
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


def main():
    print("kokoro_tts_worker: loading model...", file=sys.stderr, flush=True)
    t0 = time.monotonic()

    from kokoro import KPipeline

    pipeline = KPipeline(lang="a")  # default lang, overridden per-request
    load_ms = int((time.monotonic() - t0) * 1000)
    print(f"kokoro_tts_worker: model loaded in {load_ms}ms", file=sys.stderr, flush=True)

    # Signal readiness
    ready_msg = json.dumps({"ready": True, "load_ms": load_ms})
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

            # Generate audio segments
            # KPipeline returns generator of (graphemes, phonemes, audio) tuples
            audio_segments = []
            for _gs, _ps, audio in pipeline(text, voice=voice, lang=lang):
                if audio is not None:
                    audio_segments.append(audio)

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
