-- The admin's record of every service it has seen (docs/SERVICES.md,
-- phase S2; D8 finding 34). A service's beacon in Redis lasts fifteen
-- seconds; this keeps it once it has gone, so the Services page can say
-- "down since". `site_id` is '' for the platform's own services, a site's
-- id for the services its link carries up.
CREATE TABLE IF NOT EXISTS service_instances (
    site_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    host TEXT NOT NULL,
    version TEXT NOT NULL,
    first_seen TIMESTAMPTZ NOT NULL,
    last_seen TIMESTAMPTZ NOT NULL,
    up BOOLEAN NOT NULL,
    down_since TIMESTAMPTZ,
    stopped BOOLEAN NOT NULL DEFAULT FALSE,
    last JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (site_id, id)
);
