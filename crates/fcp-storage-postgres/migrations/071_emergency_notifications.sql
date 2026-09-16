-- E911 block 2 phase 5 (docs/E911_BLOCK_2.md §8): notification delivered.
--
-- A tenant's notification contacts become typed (email, sms, call,
-- webhook); the free text saved before this is read by its shape, so
-- nothing a tenant configured is lost. Every attempt to tell a contact
-- about an emergency call is a row, so a tenant can show afterwards that
-- someone was told — and a 933 test call proves the chain before it is
-- needed.

ALTER TABLE emergency_configs
    ADD COLUMN IF NOT EXISTS contacts JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Migrate the free-text contacts by shape, as NotifyContact::parse does:
-- a SIP URI or a short number is a call, an address with an @ an email, a
-- longer number an SMS, a UUID a webhook endpoint. Anything else is left
-- behind in notify_contacts, which stays as it is for one release.
UPDATE emergency_configs
SET contacts = COALESCE((
    SELECT jsonb_agg(jsonb_build_object('kind', kind, 'target', target))
    FROM (
        SELECT
            CASE
                WHEN lower(c) LIKE 'sip:%' OR lower(c) LIKE 'sips:%' OR lower(c) LIKE 'tel:%'
                    THEN 'call'
                WHEN lower(c) LIKE 'mailto:%' THEN 'email'
                WHEN c LIKE '%@%' THEN 'email'
                WHEN c ~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
                    THEN 'webhook'
                WHEN c ~ '^[+0-9()\-. ]+$'
                     AND length(regexp_replace(c, '[^0-9]', '', 'g')) >= 7 THEN 'sms'
                WHEN c ~ '^[+0-9()\-. ]+$' THEN 'call'
                ELSE NULL
            END AS kind,
            CASE WHEN lower(c) LIKE 'mailto:%' THEN substring(c from 8) ELSE c END AS target
        FROM unnest(emergency_configs.notify_contacts) AS c
        WHERE btrim(c) <> ''
    ) AS typed
    WHERE kind IS NOT NULL
), '[]'::jsonb)
WHERE contacts = '[]'::jsonb
  AND notify_contacts IS NOT NULL
  AND array_length(notify_contacts, 1) > 0;

-- One attempt at one contact. The row carries the notice itself, so a
-- retry needs nothing else, and next_retry_at schedules it.
CREATE TABLE IF NOT EXISTS emergency_notifications (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    -- The emergency call's record, when it had been written by then.
    call_record_id BIGINT REFERENCES emergency_calls(id) ON DELETE CASCADE,
    -- The call's id, which always identifies it.
    call_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    contact JSONB NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,
    -- 'delivered', 'retrying' or 'failed'.
    status TEXT NOT NULL,
    error TEXT,
    delivered_at TIMESTAMPTZ,
    next_retry_at TIMESTAMPTZ,
    -- Whether this told someone about a test call (933).
    test BOOLEAN NOT NULL DEFAULT FALSE,
    notice JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_emergency_notifications_call
    ON emergency_notifications (tenant_id, call_id, created_at);

CREATE INDEX IF NOT EXISTS idx_emergency_notifications_due
    ON emergency_notifications (next_retry_at)
    WHERE next_retry_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_emergency_notifications_delivered
    ON emergency_notifications (tenant_id, kind, target, delivered_at DESC)
    WHERE delivered_at IS NOT NULL;
