# Speech per tenant (D8 finding 67)

Proposed 2026-11-25. Status: **design, awaiting the questions in §7.**

> 67. There is no TTS config page for FCP admin. This should be added and
> be per tenant so TTS settings and providers can be configured.

## 1. What there is

- **One TTS server, configured in a file.** `fcp-tts-server` builds its
  providers once at startup from `[[tts.providers]]` (Piper, Kokoro,
  OpenAI, Google Cloud, ElevenLabs, Azure, Polly), with the vendors' keys
  in the TOML (`${NAME}` from the environment). `POST /v1/synthesize`
  takes `text`, `language`, `voice`, `provider` and `tenant_id`; the
  tenant is logged and nothing else (`docs/fcp-tts_plan.md`'s usage
  tracking was never built). `GET /v1/voices` and `GET /v1/providers`
  exist. The cache is keyed by provider, text, language and voice.
- **Every caller is fixed at startup, and none passes a tenant.** The call
  manager speaks through the server with `tenant_id: None` and language
  hard-coded `en-US` (its prompt pre-generation too); the IVR takes
  `[flows] tts_language`/`tts_voice`; the voicemail server's personalized
  greeting uses the default voice and language; the conference server
  builds its own provider. Since v0.25.640 the admin can speak through
  the server (`[tts] remote_url`), for prompts and their preview.
- **What tenants and users have.** A tenant has no language or voice. A
  user has `language` (default `en-US`). A prompt has `language` and
  `tts_voice`; a flow's `Say` node `voice` and `language`.
  `config_settings` is per tenant and holds nothing about speech.
- **The admin UI's voices are a hard-coded Kokoro list** (Prompts, the
  flow builder), whatever the server actually speaks with.
- **Secrets per tenant are sealed** (`docs/ENCRYPTION.md`): identity
  providers' client secrets, RTMP keys, redirection accounts — written
  once, shown as `has_secret`, never returned.

## 2. The model

**A tenant's speech settings** (`tenant_speech`): the provider used when
nothing says otherwise, the default voice, the default language, and a
voice per language where it differs. Unset, the platform's defaults
(the server's `default_provider` and voice) apply, as today.

**Providers a tenant may use:**

- **The platform's**, those `fcp-tts-server` is configured with, as far
  as the platform administrator offers them to tenants (all by default;
  a costly cloud voice can be held back).
- **The tenant's own accounts** (bring your own key): a type (OpenAI,
  ElevenLabs, Azure, Polly, Google), its key sealed at rest like an
  identity provider's secret, a name, and a test that speaks a sentence.
  Their voices are the tenant's: a cloned ElevenLabs voice stays theirs.

**Resolved at the TTS server.** A request names its tenant (callers pass
the call's tenant at last); the server reads the tenant's settings and
accounts from Postgres (cached, refreshed on change), picks the provider
and voice the request did not name, and keeps a provider per tenant
account. The server gains `[storage] database_url` and the master key to
unseal; without them it behaves as today. Callers stay simple: they pass
a tenant and, where they know it, a language (the user's, for a prompt
spoken to them).

**The cache stays shared where it may.** A platform provider's audio is
cached as today, across tenants (the same words, voice and language are
the same sound). A tenant account's is cached under that account alone.

**Usage.** The server counts characters per tenant and provider by hour
(the plan's `/v1/usage`, at last), shown on the page; a cap may follow.

## 3. The API

- `GET/PUT /api/v1/speech/settings` — the tenant's defaults
  (`speech_manage`).
- `GET /api/v1/speech/providers` — what the tenant may use, platform's
  and own; `POST/PUT/DELETE /api/v1/speech/accounts[/{id}]` for its own
  (key write-only, `has_key`), `POST …/{id}/test`.
- `GET /api/v1/speech/voices?provider=&language=` — from the server, so
  the UI's voice lists are the real ones.
- `POST /api/v1/speech/preview` — speaks a sentence with given settings.
- `GET /api/v1/speech/usage?from=&to=`.
- Platform: `GET/PUT /api/v1/system/speech` — which of the server's
  providers tenants are offered.

## 4. The page

**Speech** in a tenant's settings: default provider, voice and language
with a play button; voices per language; the tenant's own accounts (add,
test, remove); usage by month. The Prompts page and the flow builder take
their voice lists from `/speech/voices` instead of the hard-coded list.
The platform's view adds which providers tenants are offered.

## 5. Sites

Settings and accounts sync to a site like identity providers: the key
sealed again with the site's key; usage comes up with the site's records.

## 6. Phases

- **S1 — the tenant reaches the server.** `TtsService::synthesize` takes
  the tenant (and language) per request; the call manager, IVR,
  voicemail, conference server and admin pass the call's or prompt's
  tenant; `en-US` stops being hard-coded where a user's language is
  known.
- **S2 — settings.** `tenant_speech`, the API, the server resolving
  defaults per tenant, `/speech/voices`, preview, the Speech page, the
  real voice lists in Prompts and the flow builder.
- **S3 — the tenant's own accounts.** Sealed keys, the server building a
  provider per account, the cache per account, test, the platform's
  offer list.
- **S4 — usage,** counted at the server, shown on the page.
- **S5 — sites.**

STT (`docs/SPEECH.md` §1–7) follows the same model afterwards: a
tenant's recognizer and its own key.

## 7. Questions

1. **Bring your own key:** tenants may add their own provider accounts
   (recommended), or choose only among the platform's.
2. **Where a tenant's choice is resolved:** at the TTS server, which
   reads the tenant's settings (recommended), or by every caller, which
   would each need the settings and the keys.
3. **The platform's providers offered to tenants:** all by default, the
   platform administrator holding some back (recommended), or none
   unless offered.
4. **Usage:** counted and shown now, caps later (recommended), or caps
   now.
5. **Language:** the tenant's default, overridden by the user's language
   for what is spoken to that user — their voicemail, their menus
   (recommended) — or the tenant's alone.
6. **STT:** after TTS, the same model (recommended), or together.
7. **Sites:** synced, keys sealed again for the site (recommended), or
   the cloud's TTS only.
