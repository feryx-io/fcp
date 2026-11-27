-- What each tenant's speech cost (docs/TENANT_SPEECH.md S4): requests and
-- characters per tenant, provider and hour, as the TTS server counted
-- them. `synthesized_characters` is what a provider spoke (and bills for);
-- the rest came from the cache.
CREATE TABLE IF NOT EXISTS speech_usage (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    hour TIMESTAMPTZ NOT NULL,
    requests BIGINT NOT NULL DEFAULT 0,
    characters BIGINT NOT NULL DEFAULT 0,
    synthesized_characters BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, hour, provider)
);
