# Speech spending caps

Designed 2026-11-27, agreed the same day on every recommendation. The
follow-up `docs/TENANT_SPEECH.md` §7 question 4
left for later: usage is counted and shown (S4), nothing stops it.

## 1. What there is

- **Usage per tenant, provider and hour** (`speech_usage`, migration
  155): requests, characters asked for, and `synthesized_characters`, the
  ones a provider actually spoke (the rest came from the cache and cost
  nothing). The TTS server counts in memory and adds to the table every
  15 s (`fcp-tts-server/src/usage.rs`); a site's hours come up its link
  and are set as the site's (S5). `GET /speech/usage` sums them.
- **Who pays for what.** A platform provider (one `fcp-tts-server` is
  configured with: a cloud voice on the platform's key, or a local engine
  such as Piper that costs only CPU) is the platform's bill. A tenant's
  own account (`account:<uuid>`, S3) is the tenant's bill.
- **What a platform administrator can do today:** hold a provider back
  from every tenant (`platform_speech.held_back`). Nothing between "every
  tenant, unlimited" and "no tenant".
- **What a tenant can do today:** nothing to stop its own account's bill
  running up — a flow in a loop speaking fresh text (a caller's name, a
  balance, an HTTP node's answer) synthesizes every time.
- **No speech events.** The TTS server publishes none and has no events
  token.
- **The fallback already exists.** When a tenant's chosen account cannot
  speak, the server's default speaks instead and the call is never
  silent; a request that names the account (a preview, a test) is refused
  with the reason (S3).

## 2. The model

**A cap** is a number of synthesized characters per calendar month (UTC)
on one provider for one tenant. Synthesized, because that is what a
provider bills: cached audio is free and never counts.

**Two kinds, set by whoever pays:**

| Cap | Set by | On | Default |
|---|---|---|---|
| Platform cap | Platform administrator | A platform provider, for one tenant or as the default for every tenant | None (unlimited, as today) |
| Account cap | The tenant (`speech_manage`) | One of its own accounts | None |

A platform administrator can set a default cap per platform provider
(every tenant) and override it for a tenant (higher, lower, or none).
A tenant cannot raise a platform cap; it may set a lower one of its own
on a platform provider, since a tenant reselling to its own customers
may want one.

**The server's default provider cannot be capped.** It is what speaks
when a cap is reached; capping it would leave a call silent. The API
refuses a cap on it (400), and a provider that becomes the default while
capped loses its cap's effect, with a warning on the page.

**At the cap, the default speaks.** For the rest of the month the
tenant's requests that would have gone to the capped provider go to the
server's default instead, with the default's voice, exactly as the S3
fallback does. A request naming the capped provider (a preview, an
account's test) is refused: `429`, `{"code":"speech_cap_reached"}`, the
cap and the month's count. The month turning, or the cap raised or
removed, ends it within the refresh below.

**Warnings before the wall.** At 80 % and at 100 % of a cap, once per
cap and month: `speech.cap_near` and `speech.cap_reached` events (to the
tenant's webhooks and `/me/events`; the platform's for platform caps), a
line on the Speech page, and an audit row for the platform's. Raised by
the admin's sweep, not the TTS server (§4).

**Enforcement is soft, by about a flush.** Each TTS server knows the
month's total for every capped (tenant, provider) from the database plus
what it has counted and not yet written. Several servers each add up to
15 s of the others' speech before seeing it, so a cap can be passed by a
few requests' worth. This is a spending guard, not billing; a strict
count would put Redis on every synthesis.

**Money is shown, not capped.** A platform administrator may give a
platform provider a price per million characters, and a tenant its own
account's; the Usage card then shows an estimated cost beside the
characters. Caps stay in characters: prices change, a provider's bill is
the truth, and a cap in characters means the same thing next month.

## 3. The API

- `GET /api/v1/speech/caps` — the caps that apply to the tenant: each
  provider's platform cap (read-only to the tenant), the tenant's own
  cap, the month's synthesized characters, the state (`ok`, `near`,
  `reached`) and when it resets (`prompt_read`).
- `PUT /api/v1/speech/caps/{provider}` / `DELETE …` — the tenant's own
  cap on one of its accounts or (lower only) on a platform provider
  (`speech_manage`). `{provider}` is a platform provider's name or
  `account:<uuid>`.
- `GET /api/v1/system/speech/caps`, `PUT/DELETE
  /api/v1/system/speech/caps/{provider}` — a platform provider's default
  cap for every tenant (platform only).
- One tenant's platform cap is `PUT /api/v1/speech/caps/{provider}` with
  `kind: "platform"` (null `monthly_characters`: none for this tenant) and
  `DELETE …?kind=platform`, by a platform administrator in the tenant
  picked (`x-fcp-tenant-id`), as the rest of the admin works on a tenant.
- `price_per_million` on `/system/speech` (per platform provider) and on
  an account; `estimated_cost` on `/speech/usage` rows where a price is
  known.
- The TTS server's `/v1/synthesize` refusal: `429` with
  `speech_cap_reached`, passed on unchanged by the admin's preview.

## 4. How it works

- **Stored:** `speech_caps` (tenant or none for a platform default,
  provider, `kind` platform/tenant, `monthly_characters`) and
  `speech_cap_notices` (tenant, provider, month, level) so each warning
  is raised once. Migration 157.
- **At the TTS server:** `fcp-tts-server/src/caps.rs` keeps the caps and
  each capped (tenant, provider)'s month-to-date total, refreshed every
  15 s with the usage flush (one query: the caps joined with this
  month's `speech_usage` sums, cloud and sites), plus the server's own
  counts since. The effective cap is the lowest that applies (platform
  default, tenant's platform cap, tenant's own). `synthesize` asks
  `caps.reached(tenant, provider)` after resolving the provider and
  before speaking; reached and not named, it falls back to the default
  (`fell_back`, counted as the default's usage).
- **The sweep:** the admin's minute sweep (beside the offboarding and
  service sweeps, under a Redis lock) compares the same sums with the
  caps, writes a `speech_cap_notices` row for a new 80 % or 100 %, and
  publishes the event through the admin's existing event path.
- **The page:** the Speech page's Usage card gains a Caps section (each
  provider: the month's characters against its cap as a bar, the state,
  the tenant's own cap editable); the platform's view gains default caps
  and prices per provider, and a tenant's page in System gains its
  platform caps.

## 5. Sites

Caps are journaled and synced down like settings (`SYNC_TABLES`); a
site's admin refuses writes to them (`managed`). A site's TTS server
enforces with what it can see: the site's own usage, plus the cloud's
month-to-date for the tenant carried down as a synced row
(`speech_cap_totals`, written by the cloud's sweep each minute, holding
the cloud's and every site's sums). While the link is down, the site
counts on from the last total it had: a cut site can pass a cap by what
it speaks while cut, never by what the cloud does. Warnings are raised
by the cloud's sweep only, once.

## 6. Phases

- **C1 — caps at the TTS server.** `speech_caps`, the store, the
  tenant's and the platform's API, `caps.rs`, the fallback and the 429,
  the default provider refused a cap. Proved by a TTS server test (a cap
  reached falls back to the default; named, 429; the month turning ends
  it) and the Postgres API suite (who may set what, a tenant's cap never
  above the platform's effect, the default refused).
  **Done (v0.25.660):** `fcp_core::speech::{SpeechCap, SpeechCapKind,
  effective_cap, month_of}` and the cap methods on `SpeechUsageStore`
  (`speech_caps`, migration 157; an account's caps deleted with it);
  `/speech/caps` (list with `platform_cap`, `platform_override`,
  `own_cap`, `cap`, `used`, `state`, `resets_at`; `PUT`/`DELETE
  /speech/caps/{provider}`, `kind: platform` for a platform
  administrator) and `/system/speech/caps[/{provider}]`; a cap refused on
  the default provider, on a provider the TTS server does not have, on
  another tenant's account, and the platform's on an account.
  `fcp-tts-server/src/caps.rs`, read after each usage flush and added to
  as the server synthesizes; at a cap `synthesize` falls back to the
  default (its voice, counted as its usage) and answers a named provider
  `429 speech_cap_reached`, which the admin's preview and account test
  pass on. Proved by `caps::tests`,
  `at_its_cap_a_tenant_is_spoken_for_by_the_default`, fcp-core's
  `the_lowest_cap_that_applies_is_the_cap`, and the Postgres suite's
  `speech_caps_are_set_by_whoever_pays`; the route matrix seeds a cap in
  tenant B.
- **C2 — warnings and the page.** `speech_cap_notices`, the admin's
  sweep, `speech.cap_near`/`cap_reached` in `PlatformEvent::EVENT_TYPES`,
  prices and `estimated_cost`, the Caps section and the platform's
  defaults, a Playwright spec.
  **Done (v0.25.661):** `speech_cap_notices` and `platform_speech.prices`
  (migration 158); `fcp_api_core::v1::speech::sweep_caps`, run every
  minute by the admin under the `fcp:speech-caps` lock, notes each
  (tenant, provider, month, level) once and publishes
  `PlatformEvent::Speech` — `speech.cap_near` at 80 %, `speech.cap_reached`
  at 100 % (near noted too, so never said after) — to the tenant's
  webhooks and `/me/events`. No audit row: a sweep is nobody's action, and
  the notices table is the record. Prices: `prices` on `PUT
  /system/speech` and `price_per_million` on each provider it lists, an
  account's own `price_per_million`, and `estimated_cost` on each
  `/speech/usage` row (synthesized characters at the price, four places).
  The page: a Caps card (each provider's month against its cap as a bar,
  near and reached called out, the tenant's own cap editable, a cap added
  for any provider or account but the default), the platform's cap on the
  tenant editable by a platform administrator in that card for the tenant
  picked (rather than a page of its own), the offer card's default cap
  and price per provider, the account form's price, and the Usage card's
  Estimated cost column. Proved by `speech_caps_warn_once_and_usage_is_priced`
  and `e2e/speech.spec.ts`'s caps test.
- **C3 — sites.** Caps journaled and synced, `speech_cap_totals` down,
  the site's server enforcing on both, proved by `fcp-link/tests`.
  **Done (v0.25.662):** the journal needs a tenant on every row, and the
  platform's default caps have none, so the caps themselves are not
  synced. Instead the cloud's minute sweep keeps `site_speech_caps`
  (migration 159, journaled, in `SYNC_TABLES`; one row per tenant with a
  site and capped provider, never the default): the cap that applies and
  the month's synthesized characters by source (`''` the cloud, else a
  site's id), written only when it changed and removed with its cap
  (`fcp_core::speech::SiteSpeechCap`). The link keeps each as
  `speech_cap_carried` — the cap, and what was spoken everywhere but this
  site, whose own it counts itself — and the site's TTS server reads it
  with its own caps (as the platform's on the tenant) and usage, so
  `/speech/caps` at the site shows the same. A site's admin refuses
  `speech/caps` and `system/speech/caps`, and raises no warnings (the
  cloud's sweep alone does). Proved by
  `speech_caps_are_carried_to_a_tenant_s_sites` and `fcp-link/tests/apply.rs`
  (a cap reaching the site with its own share left out, refused there,
  gone when the cloud's goes). All three phases done.

## 7. Questions

1. **What a cap counts:** synthesized characters (recommended: what a
   provider bills, cache hits free), all characters asked for, or
   requests.
2. **Who caps what:** the platform its providers (a default for every
   tenant and per tenant), the tenant its own accounts and, lower only,
   the platform's (recommended); or the platform alone.
3. **At the cap:** the server's default speaks for the rest of the month
   and a request naming the provider gets 429 (recommended), or every
   request is refused, or nothing is enforced and only warned.
4. **The period:** the calendar month in UTC (recommended), or a rolling
   30 days.
5. **Warnings:** events and a page line at 80 % and 100 %, once per cap
   and month (recommended); other thresholds configurable later.
6. **Strictness:** soft, refreshed with the 15 s flush, so several
   servers can pass a cap by a few requests (recommended), or strict
   through a Redis counter on every synthesis.
7. **Money:** caps in characters with an optional price per provider for
   an estimated cost (recommended), caps in money, or no prices.
8. **Sites:** caps synced, the cloud's month-to-date carried down and the
   site counting on while cut (recommended), or caps enforced in the
   cloud only.
