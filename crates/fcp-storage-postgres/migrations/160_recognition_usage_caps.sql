-- Speech recognition counted and capped (docs/TENANT_RECOGNITION.md R3).
--
-- What a recognizer heard, in milliseconds of audio, beside what a voice
-- spoke; a site ships it up in the same rows.
ALTER TABLE speech_usage ADD COLUMN IF NOT EXISTS recognized_ms BIGINT NOT NULL DEFAULT 0;

-- A cap counts characters or minutes. An OpenAI account both speaks and
-- hears, so a provider may carry one of each: the unit joins every key.
-- (An admin of the release before, writing a cap during a rolling
-- upgrade, no longer finds its ON CONFLICT index and is refused until it
-- is replaced; a cap is a rare write.)
ALTER TABLE speech_caps ADD COLUMN IF NOT EXISTS unit TEXT NOT NULL DEFAULT 'characters'
    CHECK (unit IN ('characters', 'minutes'));
DROP INDEX IF EXISTS speech_caps_one;
CREATE UNIQUE INDEX IF NOT EXISTS speech_caps_one
    ON speech_caps ((COALESCE(tenant_id, '')), provider, kind, unit);

ALTER TABLE speech_cap_notices ADD COLUMN IF NOT EXISTS unit TEXT NOT NULL DEFAULT 'characters';
ALTER TABLE speech_cap_notices DROP CONSTRAINT IF EXISTS speech_cap_notices_pkey;
ALTER TABLE speech_cap_notices ADD PRIMARY KEY (tenant_id, provider, unit, month, level);

-- Sites: a minute cap's row id ends `@minutes`; `used` is in milliseconds.
ALTER TABLE site_speech_caps ADD COLUMN IF NOT EXISTS unit TEXT NOT NULL DEFAULT 'characters';
ALTER TABLE speech_cap_carried ADD COLUMN IF NOT EXISTS unit TEXT NOT NULL DEFAULT 'characters';
ALTER TABLE speech_cap_carried DROP CONSTRAINT IF EXISTS speech_cap_carried_pkey;
ALTER TABLE speech_cap_carried ADD PRIMARY KEY (tenant_id, provider, unit);

-- The platform's price per minute heard, beside its price per million
-- characters.
ALTER TABLE platform_speech ADD COLUMN IF NOT EXISTS minute_prices JSONB NOT NULL DEFAULT '{}'::jsonb;
