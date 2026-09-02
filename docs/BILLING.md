# Billing

FCP's billing subsystem rates answered calls against per-tenant rate plans,
maintains an optional prepaid/credit balance per tenant, enforces that balance
at call admission and mid-call, and delivers every billing outcome to external
systems as `billing.*` platform events (webhooks / WebSocket).

## Components

| Piece | Where | Role |
|---|---|---|
| Data model | `fcp-core/src/billing.rs` | `RatePlan`, `Rate`, `TenantBalance`, `BalanceTransaction` — shared with the storage contract (`CdrStore`) |
| Engine | `fcp-billing` | Rating, balance admission math, settlement, external hooks (`BillingEngine`, `BillingHook`) |
| Persistence | `fcp-storage-postgres`, migrations 047/048 | `rate_plans`, `rates`, `tenant_balances`, `balance_transactions` |
| Call path | `fcp-sip` handler + `fcp-b2bua-core` call manager | 402 at admission, cut-off watchdog, settlement at CDR creation |
| Admin API | `fcp-api-core` `/v1/rate-plans`, `/v1/billing/*` | Plan/rate CRUD, balance settings, ledger, credits |
| Events | `fcp-events` `BillingEvent` | `billing.call_rated`, `billing.balance_changed`, `billing.balance_low`, `billing.balance_depleted`, `billing.call_rejected`, `billing.call_cut_off` |

## Rating

A tenant has at most one *default* rate plan (currency, default per-minute
rate, connect fee, billing increment) with longest-prefix-match rates. At CDR
creation the answered duration is rounded up to the billing increment and
priced; cost and currency are stamped onto the CDR and summed into
`GET /v1/usage` / `GET /v1/billing/summary`.

Destinations without dialable digits (`sip:alice@…`, `queue:support`) are not
billable. Internal extensions (`sip:2001@…`) *are* rated, at the plan default
rate — set it to 0 for free internal calling.

## Balances and enforcement

`PUT /v1/billing/balance` creates the tenant's balance row (at zero) and sets:

| Field | Meaning |
|---|---|
| `currency` | ISO 4217 code; should match the rate plan (settlement warns if not) |
| `credit_limit` | How far below zero the balance may go (0 = strictly prepaid) |
| `enforce` | Turn admission control and mid-call cut-off on/off. Off keeps the ledger without ever blocking a call |
| `low_balance_threshold` | Emit `billing.balance_low` when the balance crosses below it |

With `enforce = true`:

1. **Admission** — before routing, the SIP handler prices the *first billing
   increment* (connect fee + one increment at the matched rate) and rejects
   the INVITE with **402 Payment Required** when `balance + credit_limit`
   cannot cover it. Zero-rated destinations always pass. A
   `billing.call_rejected` event is emitted.
2. **Cut-off** — an admitted call carries the affordable duration
   (whole billing increments the available funds cover after the connect
   fee). A watchdog hangs the call up (BYE on both legs) when the answered
   duration reaches it and emits `billing.call_cut_off`.
3. **Settlement** — at call end the rated cost is debited atomically
   (one `call_charge` ledger entry per call; a retried CDR cannot
   double-charge). `billing.balance_changed` is emitted, plus
   `billing.balance_low` / `billing.balance_depleted` when the debit crosses
   those lines.

Storage failures during admission **fail open** (the call is admitted and an
error is logged): a billing outage must not take signaling down. Settlement is
still attempted at call end.

Balance enforcement is per-tenant: the platform admits calls for tenants with
no balance row or with `enforce = false` exactly as before.

## Credits and adjustments

```
POST /v1/billing/balance/transactions
{ "kind": "top_up", "amount": 25.00, "currency": "USD",
  "description": "card payment", "external_ref": "pay_01H…" }
```

`kind` is `top_up` (positive) or `adjustment` (either sign; refunds,
write-offs). `call_charge` entries are written only by settlement. The response
carries `balance_after`. `GET /v1/billing/balance/transactions?limit&offset`
lists the ledger newest first.

## External billing-system hooks

Every outcome is a `BillingEvent` delivered through the `BillingHook` trait.
The platform default (`EventBusHook`) publishes it on the event bus, so an
external system integrates by:

- **Webhook** — subscribe an endpoint to `billing.*` (or specific types) via
  `/v1/webhooks`; payloads are HMAC-signed and retried like every other event.
- **WebSocket** — `/v1/events?event_types=billing`.
- **In-process** — implement `fcp_billing::BillingHook` and install it with
  `BillingEngine::with_hook` / `CompositeHook` (e.g. a metering sidecar).

Inbound direction (payments landing in FCP) is the transactions endpoint
above; `external_ref` ties the ledger entry to the remote record.

## Not yet

- Multi-currency conversion (balance and plan are expected to share a currency).
- Per-user / per-extension sub-balances.
- Invoice generation; the ledger is the source for an external invoicing system.
