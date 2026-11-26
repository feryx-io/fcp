# Speech recognition

What FCP hears: a recording transcribed, or a call's audio recognised as
it is spoken. Built in `docs/DEVELOPER_SURFACE.md` phase 6; flows that
listen (Gather by speech, the Transcribe node) are phase 7 (§6).

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

## 6. Flows that listen

A flow's **Gather** listens for digits, speech, or either:

```json
"menu": {
  "type": "gather",
  "input": "dtmf_speech",
  "prompt": {"type": "say", "text": "Say sales or support, or press 2."},
  "num_digits": 1,
  "timeout": 5,
  "speech_timeout_ms": 800,
  "hints": ["sales", "support"],
  "language": "en-US",
  "transitions": {"2": "support"},
  "speech_transitions": {"sales": "sales", "support": "support"},
  "default": "menu",
  "on_timeout": "nothing",
  "on_error": "digits_only"
}
```

- `input`: `dtmf` (the default, and what a Gather written before speech
  reads as), `speech`, or `dtmf_speech` — whichever the caller gives
  first.
- `timeout`: seconds the caller has to begin. A caller who has begun to
  speak is waited for until their utterance ends, however long the
  timeout. A noise recognised as no words does not hold the Gather open:
  once the caller has been quiet for the end-of-speech silence and three
  seconds more, the timeout counts again.
- `speech_timeout_ms`: the silence that ends the utterance; the
  recognizer's default (800) when unset.
- `hints` and `language`: passed to the provider; `[stt] language` when
  `language` is unset.
- `speech_transitions`: a phrase or keyword to a node. A phrase matches
  when the caller's words contain it as whole words, case and punctuation
  aside ("Sales, please." matches `sales`, "wholesale" does not); the
  longest matching phrase wins. With none matching, `default`, then the
  Gather again.
- What was said is in `speech`, the confidence (0.00–1.00, empty when the
  provider gives none) in `speech_confidence`, and `gather_input` is
  `dtmf` or `speech`. A Gather that listens for speech clears `speech` and
  `speech_confidence` first, so a Branch with `exists` can tell.
- `barge_in` (the default): the caller speaking for 200 ms, or a partial
  transcript, stops the prompt, and the digits that complete the Gather
  do. Without it the prompt plays out and speech during it is not heard;
  digits pressed during it are kept, as they always were.
- `on_error`: taken when speech is asked for alone and the node cannot
  listen (no `[stt]`, or the recognizer refused); without it, the timeout
  branch. A `dtmf_speech` Gather that cannot listen goes on with digits.

A digit sent by SIP INFO (`application/dtmf-relay`) joins the Gather in
progress like one in the media.

A **Transcribe** node sends the call's live transcript as `call.transcript`
events until a Transcribe with `"action": "stop"`, the end of the flow, or
the end of the call:

```json
"transcript": {
  "type": "transcribe",
  "leg": "both",
  "language": "en-US",
  "interim_results": false,
  "next": "agent"
}
```

`leg` is `caller` (the default: the party the flow answered), `callee`
(the party a `Dial` reached) or `both`. Each event carries the call, the
tenant (so webhooks get it), the `leg`, the `text`, the `confidence` and
`is_final`; with `interim_results` the partial transcripts a streaming
provider sends come too, each revised by the next. A Transcribe that
cannot start takes `on_error`, or `next`.

A phone that stops sending during silence (silence suppression) is heard
as silence: the listener fills a leg's gaps after 60 ms, so an utterance
ends when the caller stops.

## 7. Tests

`fcp_testkit::stt::ScriptedStt` stands in for the server: it answers a
known phrase for a known tone, so a scripted phone can "say" something.
`fcp-call-manager/tests/speech.rs` leaves a message through a flow and
reads the transcription back; `fcp-tts-server/tests/transcribe.rs` sends
a file and a stream through the running server;
`fcp-media/tests/stt_providers.rs` holds Deepgram's and OpenAI's requests
and answers against stand-ins; `fcp-call-manager/tests/listen.rs` runs a
menu that is spoken to, pressed by SIP INFO and left silent, and reads a
`call.transcript` from the node's events. The real services are checked by
hand.

## 8. Text to speech: Piper by default

What FCP says — prompts, greetings, the voicemail menu, a flow's Say — is
spoken by `fcp-tts-server` (`[tts]`, `[[tts.providers]]`); the call
manager, IVR and voicemail server reach it with `tts_provider = "remote"`.
Without an engine all of it was silence, because the only local engine,
Kokoro, needs PyTorch and was left off (`docs/DEEP_TESTING.md` Q-D8a).

- **Piper** (`type = "piper"`, `fcp_media::tts::providers::piper`) is the
  default engine: the last MIT-licensed release of the C++ engine and the
  LJSpeech voice (public domain), in every TTS image
  (`scripts/fetch-piper.sh`, pinned by SHA-256). One `piper --json-input`
  process per voice, started on first use and kept; its audio returned at
  8 kHz like every provider's. About 0.2–0.5 s a sentence on a CPU.
  `binary`, `voices_dir` and `voice` (`en_US-ljspeech-medium` unless set)
  configure it; a request's voice is used if installed, else one of the
  request's language (`en-GB` → an `en_GB-…` model in `voices_dir`), else
  the default. More voices are `.onnx` models with their `.onnx.json`
  dropped into `voices_dir` — check each voice's licence (several of
  Piper's are non-commercial).
- **Kokoro** (`type = "local"`, `engine = "kokoro"`) is in the
  `fcp-tts-full` image only (`FCP_TTS=tts-full` in compose), with the
  local Whisper recogniser.
- **OpenAI**, **Google Cloud**, **ElevenLabs**, **Azure** and **Amazon
  Polly** are providers over their vendors' APIs, in either image
  (ElevenLabs, Azure and Polly since v0.25.536, #339). Each returns
  8 kHz mono WAV like Piper: ElevenLabs' 16 kHz PCM and Polly's PCM are
  resampled or wrapped, and Azure is asked for 8 kHz.

  | `type` | Keys | A voice is | With no voice |
  |---|---|---|---|
  | `elevenlabs` | `api_key`; `model` (`eleven_flash_v2_5` unless set) | an id, or a name in the account | Rachel |
  | `azure` | `api_key` (the Speech resource's key), `region` | a short name, `en-GB-SoniaNeural` | the first neural voice of the language |
  | `aws_polly` | `region`; `access_key_id` and `secret_access_key`, or neither; `engine` (`neural` unless set) | an id, `Amy` | the first voice of the language for the engine |

  ElevenLabs' v2.5 models are told the request's language; the others
  choose it from the text. Polly with no keys signs with
  `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`, else
  the machine's role: a container's credentials endpoint (ECS, EKS) or
  the EC2 instance metadata service over IMDSv2, kept until five minutes
  before they expire. Every key may be `${NAME}` from the environment
  and is masked by `--check-config`; none is logged. `endpoint` sends a
  provider somewhere other than its vendor (a proxy, a test's stand-in).
  `/v1/voices?provider=<name>&language=<tag>` lists the vendor's voices.

`default_provider` names the engine used when a request names none.

The cloud providers are tested through the server against the testkit's
stand-ins for their vendors (`fcp_testkit::tts`; `fcp-tts-server/tests/
engines.rs`), which check the key, the output format and Polly's
signature, and against the real services when `FCP_TEST_ELEVENLABS_API_KEY`,
`FCP_TEST_AZURE_SPEECH_KEY` with `FCP_TEST_AZURE_SPEECH_REGION`, or
`FCP_TEST_POLLY_REGION` (with credentials in the environment or the
machine's role) are set.

The stack's default greeting (`deploy/docker-compose/audio/
default-greeting.wav`, played by a mailbox with no greeting of its own)
was recorded by Piper through `record_a_prompt_when_asked` in the
provider's tests. The provider's own tests run the real engine when
`FCP_PIPER` and `FCP_PIPER_VOICES` name it.

## A tenant's speech

Since v0.25.648 a tenant chooses the provider, voice and language its
words are spoken with (`GET/PUT /api/v1/speech/settings`,
`docs/TENANT_SPEECH.md`). `fcp-tts-server` reads the choice from the
platform's database (`[storage] database_url`, `FCP_TTS_DATABASE_URL`) and
applies it to any request naming the tenant that leaves those unset;
without a database every tenant gets `[tts]`'s defaults, as before.
