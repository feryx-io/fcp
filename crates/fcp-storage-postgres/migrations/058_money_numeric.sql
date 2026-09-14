-- Money is exact (docs/CDR_PIPELINE.md §14.3). Migration 047 turned
-- cdrs.cost into DOUBLE PRECISION because sqlx could not decode NUMERIC into
-- f64, and 047/048 created the rate and ledger columns the same way; sums
-- of millions of float charges drift. Every money column becomes
-- NUMERIC(18,6), existing values rounded to six places (a float such as
-- 0.02 casts to 0.02). Ratings are computed to four places, so six leaves
-- room for rates and adjustments without rounding them.

ALTER TABLE cdrs
    ALTER COLUMN cost TYPE NUMERIC(18,6) USING round(cost::numeric, 6);

ALTER TABLE rate_plans
    ALTER COLUMN default_rate_per_minute TYPE NUMERIC(18,6)
        USING round(default_rate_per_minute::numeric, 6),
    ALTER COLUMN connect_fee TYPE NUMERIC(18,6) USING round(connect_fee::numeric, 6);

ALTER TABLE rates
    ALTER COLUMN rate_per_minute TYPE NUMERIC(18,6) USING round(rate_per_minute::numeric, 6),
    ALTER COLUMN connect_fee TYPE NUMERIC(18,6) USING round(connect_fee::numeric, 6);

ALTER TABLE tenant_balances
    ALTER COLUMN balance TYPE NUMERIC(18,6) USING round(balance::numeric, 6),
    ALTER COLUMN credit_limit TYPE NUMERIC(18,6) USING round(credit_limit::numeric, 6),
    ALTER COLUMN low_balance_threshold TYPE NUMERIC(18,6)
        USING round(low_balance_threshold::numeric, 6);

ALTER TABLE balance_transactions
    ALTER COLUMN amount TYPE NUMERIC(18,6) USING round(amount::numeric, 6),
    ALTER COLUMN balance_after TYPE NUMERIC(18,6) USING round(balance_after::numeric, 6);
