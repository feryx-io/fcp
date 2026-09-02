-- Billing follow-up: prepaid/credit balances and the balance ledger.
-- Settlement (at CDR creation) debits the rated cost of each answered call;
-- top-ups and adjustments arrive through the admin API or an external
-- billing system. Balance enforcement (402 at admission, mid-call cut-off)
-- is opt-in per tenant via tenant_balances.enforce.

CREATE TABLE IF NOT EXISTS tenant_balances (
    tenant_id TEXT PRIMARY KEY,
    balance DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'USD',
    credit_limit DOUBLE PRECISION NOT NULL DEFAULT 0
        CHECK (credit_limit >= 0),
    enforce BOOLEAN NOT NULL DEFAULT FALSE,
    low_balance_threshold DOUBLE PRECISION,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS balance_transactions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('top_up', 'call_charge', 'adjustment')),
    amount DOUBLE PRECISION NOT NULL,
    balance_after DOUBLE PRECISION NOT NULL,
    currency TEXT NOT NULL,
    call_id TEXT,
    description TEXT,
    external_ref TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_balance_transactions_tenant_created
    ON balance_transactions (tenant_id, created_at DESC);

-- One settlement per call: a retried CDR insert must not double-charge.
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_transactions_call_charge
    ON balance_transactions (call_id) WHERE kind = 'call_charge';
