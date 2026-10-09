-- Push (docs/PUSH.md §2.1, phase P1): what a binding says about its phone.
--
-- A phone registers with its Contact's instance and reg-id (RFC 5626), kept
-- so a new binding from the same phone replaces the old; an app that can be
-- woken adds where (RFC 8599: the provider, the app at it, the device's
-- token); a binding over a flow names the node holding the flow. A device
-- shows which provider wakes it.

ALTER TABLE registrations ADD COLUMN IF NOT EXISTS instance_id TEXT;
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS reg_id INTEGER;
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS pn_provider TEXT;
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS pn_param TEXT;
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS pn_prid TEXT;
ALTER TABLE registrations ADD COLUMN IF NOT EXISTS flow_node TEXT;

ALTER TABLE devices ADD COLUMN IF NOT EXISTS push_provider TEXT;
