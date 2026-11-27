# Speech recognition per tenant

Designed 2026-11-27, agreed the same day on every recommendation.
`docs/TENANT_SPEECH.md` gave each tenant its own text
to speech (a provider, voices, a language, its own vendor accounts, usage,
caps, sites) and said recognition would follow the same model afterwards.
This is that follow-up.

## 1. What there is

- **One recognizer per node or server.** A call manager or voicemail
  server has `[stt.provider]` (`docs/SPEECH.md` §3): Deepgram, OpenAI, a
  local whisper, or `remote` (the shared `fcp-tts-server`). The shared
  server has named `[stt.providers]` and a `default_provider`. Every tenant
  is heard by the same one.
- **No caller names a tenant.** A flow's Gather by speech and its
  Transcribe node (`fcp_sip::flow_listen`) call
  `MediaManager::recognize(call, leg, options)`; voicemail transcription
  (`fcp_voicemail`, a mailbox with `transcription_enabled`) calls
  `SttService::transcribe_wav`. Both know the tenant (the call's, the
  mailbox's); neither passes it on. The `remote` provider's
  `/v1/transcribe` and `/v1/transcribe/stream` take `language`, `hints`
  and `provider`, nothing else.
- **The language is the flow's or the node's.** A Gather may name one;
  otherwise the node's `[stt] language`. A tenant's speech language
  (`tenant_speech.language`) is used for what is spoken to it, never for
  what is heard from it.
- **What the tenant's speech already has** (`docs/TENANT_SPEECH.md`,
  `docs/SPEECH_CAPS.md`): settings read by the TTS server from Postgres,
  the platform's providers with some held back, the tenant's own vendor
  accounts with sealed keys (`speech_accounts`: OpenAI, ElevenLabs,
  Azure, Polly, Google), usage per tenant, provider and hour
  (`speech_usage`), caps with warnings, and all of it at a tenant's sites.
  OpenAI's key speaks and hears; Deepgram, the recognizer most deployments
  use, is not an account kind.
- **Nothing is counted.** Recognition costs by the second of audio and no
  second is counted anywhere.

## 2. The model

Recognition takes the speech model as it is, adding what hearing needs.

**Resolved at the shared server, as speech is.** The `remote` provider
sends the tenant (`tenant_id` on both routes); the server reads the
tenant's settings and accounts as it does for synthesis and picks the
recognizer and language the request did not name. A node with its own
`[stt.provider]` (not `remote`) hears every tenant with it, as today: a
deployment that wants tenants to choose points its nodes at the shared
server, as it does for speech.

**The tenant's settings gain a recognizer.** `tenant_speech` gains
`recognizer` (one of the server's `[stt.providers]` or an account of the
tenant's; unset is the server's default) and `recognition_language`
(unset: the tenant's speech `language`, then the server's). A Gather or
Transcribe node naming a language still wins.

**One list of accounts, by what each can do.** `deepgram` becomes an
account kind (recognition only). OpenAI's key does both, so an OpenAI
account is choosable as a voice provider, a recognizer, or both; the other
kinds are speech only until FCP has recognizers for them (Azure and
Google both have one; neither is a provider in `fcp_media::stt` yet).
`/speech/providers` says which each provider can do; the server's
`[stt.providers]` are listed beside its voice providers.

**The platform's offer covers recognizers.** `platform_speech.held_back`
names recognizers as well as voice providers; a held-back recognizer
chosen by a tenant is the default's business, as a held-back voice is.

**Callers pass the tenant.** `MediaManager::recognize` and
`SttService::transcribe_wav` take the tenant; flow listening passes the
call's, voicemail the mailbox's. Only the `remote` provider sends it.

**When the tenant's recognizer cannot hear** (its account gone, its key
refused, held back), the server's default hears instead, so a Gather by
speech never goes deaf. A request naming the recognizer is refused with
the reason, as for speech.

**Usage in seconds.** The server counts each tenant's recognition per
recognizer and hour as `recognized_ms` (and requests), in the same
`speech_usage` rows (a new column), so shipping from sites and the usage
API carry it as they carry characters. The Usage card shows minutes heard
beside characters spoken; a price per minute gives an estimated cost.

**Caps in minutes.** A cap names what it counts: `characters` (as today)
or `minutes`. The same who-caps-what (the platform on its recognizers by
default and per tenant, the tenant on its accounts, the lowest applying),
the same warnings, the same carrying to sites. At a cap the server's
default recognizer hears instead; a cap on the default recognizer is
refused. A stream is judged when it starts: one already open runs to its
end.

**Sites.** `tenant_speech` and `speech_accounts` already sync; the new
fields and kind ride along. A site's TTS server hears with the synced
choice; its usage ships up in `speech_usage`; its caps arrive as
`site_speech_caps` with the new unit.

## 3. The API

- `GET/PUT /api/v1/speech/settings`: `recognizer` and
  `recognition_language` beside `provider` and `language`.
- `GET /api/v1/speech/providers`: each with `speaks` and `hears`.
- `POST /api/v1/speech/accounts`: `kind: "deepgram"`; `POST …/{id}/test`
  for a recognition-only account transcribes a short built-in recording
  and returns the text.
- `POST /api/v1/speech/recognize` — a WAV in, the tenant's recognizer's
  transcript out (a preview, as `/speech/preview` speaks one).
- `GET /api/v1/speech/usage`: `recognized_seconds` on each row;
  `estimated_cost` adding minutes at `price_per_minute`.
- Caps: `unit` (`characters` or `minutes`) on `/speech/caps` and
  `/system/speech/caps`; `price_per_minute` beside `price_per_million` on
  `/system/speech` and on an account.
- The TTS server: `tenant_id` on `/v1/transcribe` and
  `/v1/transcribe/stream`, 429 `speech_cap_reached` for a named recognizer
  at its cap.

## 4. The page

The Speech page gains a **Recognition** card: the recognizer (the
platform's offered ones and the tenant's accounts that hear), the
language, and a "try it" that records a few seconds in the browser and
shows what was heard. Accounts show what each does; Deepgram is added to
the vendors. Usage shows minutes heard; Caps take a unit. The platform's
offer lists recognizers with their default caps and prices.

## 5. Phases

- **R1 — the tenant reaches the server.** `recognize` and
  `transcribe_wav` take the tenant, flow listening and voicemail pass it,
  the `remote` provider sends `tenant_id`, the server resolves the
  tenant's recognition language (the speech language until R2's field).
  Proved by a flow and a voicemail heard in the tenant's language through
  the shared server.
- **R2 — choosing a recognizer.** `recognizer` and
  `recognition_language` (migration), `deepgram` accounts, `speaks`/
  `hears`, held-back recognizers, the server keeping a recognizer per
  tenant account (as it keeps a voice per account), the fallback, the
  test and `/speech/recognize`, the Recognition card.
- **R3 — usage and caps in minutes.** `recognized_ms`, the cap's unit,
  `price_per_minute`, the server counting and enforcing, the page.
- **R4 — sites.** The new fields and kind synced (already journaled
  tables), usage shipped, caps carried with their unit; proved by
  `fcp-link/tests`.

## 6. Questions

1. **Where a tenant's choice is resolved:** at the shared server, nodes
   with their own provider hearing every tenant alike (recommended), or
   every node reading the tenant's settings and keys.
2. **Settings:** a `recognizer` and `recognition_language` in the tenant's
   speech settings, the language defaulting to the speech language
   (recommended), or recognition settings of their own.
3. **Accounts:** one list, each kind doing what its vendor does (OpenAI
   both, Deepgram hearing, the rest speaking) (recommended), or separate
   recognition accounts.
4. **Vendors:** Deepgram and OpenAI now, Azure and Google recognizers in
   `fcp_media::stt` later (recommended), or all four now.
5. **When the tenant's recognizer cannot hear:** the default hears
   (recommended), or the recognition fails and the Gather falls back to
   digits.
6. **Usage:** seconds in the same `speech_usage` rows (recommended), or a
   table of its own.
7. **Caps:** a unit on the existing caps, minutes for recognizers, judged
   when a stream starts (recommended), or no recognition caps.
8. **Sites:** as speech — synced, shipped, carried (recommended), or the
   cloud's recognizer only.
9. **A preview:** `/speech/recognize` and the page's "try it" recording in
   the browser (recommended), or the account test alone.
