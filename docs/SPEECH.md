# Speech recognition

What FCP hears: a recording transcribed, or a call's audio recognised as
it is spoken. Built in `docs/DEVELOPER_SURFACE.md` phase 6; flows that
listen (Gather by speech) are phase 7.

## 1. The shape

Speech recognition is shaped like text-to-speech (`fcp_media::tts`):

- `fcp_media::stt::SttProvider` recognises 16 kHz mono PCM, one-shot
  (`transcribe`) or streaming (`stream`: partial and final transcripts,
  each with the provider's confidence when it gives one).
- `fcp_media::stt::SttService` wraps one provider with a default
  language. `transcribe_wav` reads a WAV of any rate or channel count.
  `stream` uses the provider's own stream, or cuts the audio into
  utterances by energy (`Endpointer`: -40 dBFS, the end-of-utterance
  silence `speech_timeout_ms`, 800 ms by default) and transcribes each.
- `MediaManager::listen(call, leg)` yields one leg's audio as 16 kHz
  frames, over forge-engine's media bridge, until the session ends.
  `MediaManager::recognize(call, leg, options)` feeds it to the
  configured provider.

## 2. Providers

| `type` | How | Stream | Keys |
|---|---|---|---|
| `deepgram` | `POST /v1/listen` for a file; the `/v1/listen` WebSocket for a stream, utterances ended by Deepgram's `endpointing` | its own | `api_key`, `model` (`nova-3`), `base_url` |
| `openai` | `POST /v1/audio/transcriptions` (a WAV) | cut into utterances | `api_key`, `model` (`whisper-1`), `base_url` |
| `local` | faster-whisper in `scripts/whisper_stt_worker.py`, started once and kept (like the kokoro TTS worker); `FCP_WHISPER_WORKER` names the script | cut into utterances | `python_path`, `model` (`base`), `device` (`auto`) |
| `remote` | the shared `fcp-tts-server`'s `/v1/transcribe` | its own | `url`, `auth_token`, `provider` |

Hints (`hints`) become Deepgram's `keyterm` (Nova-3) or `keywords`, and
OpenAI's and whisper's `prompt`. Whisper takes a two-letter language
(`en-US` is sent as `en`); OpenAI gives no confidence.

A key can be written `${NAME}` and read from the environment when the
file is loaded (`docs/PRODUCTION_STACK.md` phase 2); `--check-config`
masks it.

## 3. Configuration

### A node: the call manager and the voicemail server

```toml
[stt]
language = "en-US"

[stt.provider]
type = "remote"
url = "http://fcp-tts:8093"
auth_token = "${FCP_TTS_AUTH_TOKENS}"
```

Unset, nothing is recognised: a mailbox asking for transcription gets
none, and nothing fails.

### The shared server: `fcp-tts-server`

Named providers, as its TTS providers are; a request names one with
`?provider=`, or gets `default_provider` (required when there are
several). A server may carry TTS, speech recognition or both.

```toml
[stt]
default_provider = "deepgram"
language = "en-US"

[stt.providers.deepgram]
type = "deepgram"
api_key = "${DEEPGRAM_API_KEY}"

[stt.providers.whisper]
type = "local"
model = "small"
```

The image (`deploy/docker-compose/Dockerfile.tts`) carries faster-whisper
and the worker; the model is fetched from the Hugging Face hub on first
use.

## 4. The server's routes

Both behind the server's bearer token.

- `POST /v1/transcribe?language=&hints=a,b&provider=` — a WAV in the
  body (up to 64 MB), one transcript out:
  `{"text": "…", "confidence": 0.93, "is_final": true}`. 400 for a body
  that is not a WAV or an unknown provider, 503 with no `[stt.providers]`,
  502 when the provider fails.
- `GET /v1/transcribe/stream?language=&hints=&provider=&interim_results=&speech_timeout_ms=`
  — a WebSocket. Send binary messages of 16 kHz mono little-endian PCM and
  a text `{"type":"finish"}` when the audio ends; transcripts come back as
  text messages of the JSON above, and the server closes after the last.

## 5. Voicemail transcription

A mailbox with `transcription_enabled` (the API's mailbox field) has each
new message transcribed after the caller hangs up, on the node that
recorded it:

- the text is stored on the message (`transcription`, in the API and
  `/me/voicemail`);
- `voicemail.transcribed` is published with the tenant, the mailbox, the
  message, the text and the confidence, so the tenant's webhooks receive
  it (`docs/WEBHOOKS.md`).

A mailbox's `include_transcription` (the text in a notification email) has
nothing to act on yet: no voicemail notification email is sent by any
node.

## 6. Tests

`fcp_testkit::stt::ScriptedStt` stands in for the server: it answers a
known phrase for a known tone, so a scripted phone can "say" something.
`fcp-call-manager/tests/speech.rs` leaves a message through a flow and
reads the transcription back; `fcp-tts-server/tests/transcribe.rs` sends
a file and a stream through the running server;
`fcp-media/tests/stt_providers.rs` holds Deepgram's and OpenAI's requests
and answers against stand-ins. The real services are checked by hand.
