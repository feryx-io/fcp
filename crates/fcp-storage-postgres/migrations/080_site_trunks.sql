-- Calls across a site's link (docs/HYBRID_CONTROL_PLANE.md §3.6).
--
-- A trunk may run at a site: a carrier of the site's own, or the site's
-- link trunk, which the cloud makes at enrollment as its own end
-- ('cloud') and the link turns into the site's end ('site') as it
-- applies it there. A number may be answered at a site: the cloud sends
-- a call for it down that site's registration.
ALTER TABLE trunks ADD COLUMN IF NOT EXISTS site_id TEXT;
ALTER TABLE trunks ADD COLUMN IF NOT EXISTS link_end TEXT;
CREATE INDEX IF NOT EXISTS idx_trunks_site ON trunks(site_id) WHERE site_id IS NOT NULL;
ALTER TABLE phone_numbers ADD COLUMN IF NOT EXISTS site_id TEXT;
