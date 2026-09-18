-- Tenant trunks phase 4 (docs/TENANT_TRUNKS.md §7): the trunk a call
-- arrives on decides its tenant.
--
-- Where a carrier's calls come from: addresses and CIDR ranges, since a
-- carrier often sends from addresses it does not receive on. Empty means
-- the trunk's hosts.
ALTER TABLE trunks
    ADD COLUMN IF NOT EXISTS inbound_sources TEXT[] NOT NULL DEFAULT '{}';

-- Inbound admission now honours a trunk's direction, and nothing did
-- before: a trunk left at the API's old default of 'upstream' was admitting
-- its carrier's calls. Keep it doing so, and make a new trunk carry calls
-- both ways unless it says otherwise, as a [[trunks]] entry does.
UPDATE trunks SET direction = 'bidirectional' WHERE direction = 'upstream';
ALTER TABLE trunks ALTER COLUMN direction SET DEFAULT 'bidirectional';
