# Speech per tenant (D8 finding 67)

Proposed 2026-11-25; every question in §7 agreed on its recommendation the same day.

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
  **Done (v0.25.643):** `TtsService::synthesize_for(text, language,
  voice, tenant)` and `TtsProvider::synthesize_for` (the remote provider
  sends `tenant_id` per request; others ignore it), the client's cache
  keyed by tenant, `warm` and `MediaManager::play_tts_for` taking one.
  Passed: a tenant's own flows' Say and Gather (`FlowContext.tenant_id`,
  the call's; a flow of no tenant, the platform's or one made inline,
  speaks as the platform),
  the flow and call handlers' prompts and spoken text, `say_into_call`,
  find-me's confirm prompt, prompts made and spoken again through the API, a
  user's typed greeting, a mailbox's personalized greeting, and the
  conference server's consent, dial-out, PIN and welcome speech. Still
  without a tenant: the call manager's and a queue's recording-consent
  announcement (their plan carries none), the emergency test readback
  (the platform's words, warmed once at startup for every tenant's
  first test), the call manager's system
  prompt pre-generation and warm-up (the platform's, rightly). `en-US`
  stays where no user is at hand; a user's language arrives with S2's
  settings. Proved by `fcp-media/tests/tts_tenant.rs`.
- **S2 — settings.** `tenant_speech`, the API, the server resolving
  defaults per tenant, `/speech/voices`, preview, the Speech page, the
  real voice lists in Prompts and the flow builder.
  **S2a done (v0.25.648):** `fcp_core::speech::TenantSpeech` (provider,
  voice, language, a voice per language — `es` names every Spanish) in
  `tenant_speech` (migration 153); `GET/PUT/DELETE /api/v1/speech/settings`
  under `prompt_read`/`prompt_manage` rather than a permission of its own
  (speech is what the tenant's prompts sound like); `fcp-tts-server`
  reading them from `[storage] database_url` (`FCP_TTS_DATABASE_URL`,
  cached 30 s) and filling what a request leaves unset — the provider (one
  it has), the language, then the voice (the tenant's only with the
  tenant's provider) — before its own defaults; the client leaving a
  tenant's language and voice unset when the caller did, so the server's
  choice is the tenant's. The compose TTS server reads the platform's
  database. Proved by `fcp-tts-server` `a_tenants_settings_fill_what_a_request_leaves_unset`
  and `api_postgres` `a_tenants_speech_settings_are_kept_and_cleared`.
  **S2b done (v0.25.653):** `GET /speech/providers` and
  `GET /speech/voices?provider=&language=` (the TTS server's, through the
  admin's `[tts] remote_url`; empty where the admin has no TTS service) and
  `POST /speech/preview` (a sentence spoken now as WAV, with what is given
  and the tenant's settings for the rest; `TtsService::preview`, uncached,
  the remote provider naming the provider per request). Operations ›
  Speech: provider, language and voice, voices by language, a preview of
  any choice before it is saved, back to the platform's defaults. Prompts
  and the flow builder list the TTS service's voices (`useVoices`) instead
  of the hard-coded Kokoro list. Proved by `api_postgres`
  `speech_providers_voices_and_a_preview_come_from_the_tts_server` and
  `e2e/speech.spec.ts`.
- **S3 done (v0.25.654) — the tenant's own accounts.** `speech_accounts`
  (migration 154: OpenAI, ElevenLabs, Azure, Polly, Google; the key in
  `secret`, sealed at rest and in the re-encrypt list),
  `fcp_core::speech::SpeechAccountStore`. `GET/POST /speech/accounts`,
  `PUT/DELETE /speech/accounts/{id}` (`speech-<uuid>`; the key write-only,
  `has_key`; a change without a key keeps it, another vendor needs its
  own; at most 20; deleting one clears a choice of it) and
  `POST …/{id}/test` (a sentence spoken through it). An account is the
  provider `account:<uuid>` in the settings, previews and voice lists,
  refused for any tenant but its own. The TTS server reads accounts with
  the deployment's master key (`FCP_MASTER_KEY…`, now on the compose
  service), builds a service per (tenant, account) read again every 30 s
  and rebuilt when the account changed, its audio cached under the account
  alone; a choice of an account that cannot speak is spoken by the
  default, a request naming it (a preview, a test) is told why. The
  platform's offer: `GET/PUT /system/speech` (platform administrators;
  `platform_speech.held_back`, the default never held back), a held-back
  provider left out of a tenant's list, refused in its settings and
  previews, and spoken by the default for a tenant that chose it before.
  The Speech page gains Your accounts (add, test, edit, delete) and, for a
  platform administrator, Offered to tenants. Proved by
  `a_tenants_speech_account_keeps_its_key_sealed` (Postgres),
  `a_tenants_own_account_speaks_for_it_alone` and
  `a_provider_held_back_from_tenants_is_not_theirs_to_choose` (the TTS
  server, against `StandInTts`), `a_tenants_speech_account_is_made_chosen_and_deleted`
  and `the_platform_holds_a_provider_back_from_tenants` (`api_postgres`),
  `e2e/speech.spec.ts`.
- **S4 done (v0.25.655) — usage.** The TTS server counts each tenant's
  requests and characters per provider (the one that spoke: an account as
  `account:<uuid>`, the default where it stepped in) and hour, telling
  what the provider spoke (`TtsService::synthesize_counted`) from what the
  cache served, in memory, added to `speech_usage` (migration 155) every
  15 s and kept through a failed write (`fcp-tts-server/src/usage.rs`).
  `GET /speech/usage?from=&to=&by=month|day` (`prompt_read`; twelve months
  by default, at most 800 days) sums the hours by UTC period and provider,
  an account named by its name. The Speech page's Usage card shows the
  last twelve months. A default stepping in for an account that cannot
  speak no longer sends it the account's voice. Proved by
  `counts_are_summed_per_tenant_and_provider_and_kept_through_a_failure`
  and `speech_usage_is_read_by_month_or_day_and_provider`. Caps are for
  later (§7 question 4).
- **S5 done (v0.25.656) — sites.** `tenant_speech` and `speech_accounts`
  are journaled (migration 156) and in `SYNC_TABLES`: the snapshot and
  the feed carry them down, an account's key opened for the link's mutual
  TLS and sealed again with the site's own key as the link writes it. The
  site's admin refuses writes to `speech/settings` and `speech/accounts`
  (`fcp_api_core::managed`), a preview or an account's test being the
  site's own. Up: `speech_usage` gains `site_id` and `ship_seq`; the link
  ships the site's own hours (`ShipCursors.speech_usage`) and the cloud
  sets each as the site's (`put_site_speech_usage`), so an hour shipped
  again as it fills counts once, and `/speech/usage` sums them with the
  cloud's. Proved by `fcp-link/tests/apply.rs` (the speech and account
  reaching the site, refused there, tested there) and `records.rs` (an
  hour shipped and shipped again). All five phases done.

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
