-- A trunk's transport_port may be unset, meaning the transport's own
-- default: 5060, or 5061 for TLS (docs/DEPLOYMENT_GAPS.md §1.8). The
-- column was NOT NULL DEFAULT 5060, so a TLS trunk created without a
-- port was stored on 5060 while the node-side default was 5061.
ALTER TABLE trunks ALTER COLUMN transport_port DROP NOT NULL;
ALTER TABLE trunks ALTER COLUMN transport_port DROP DEFAULT;
-- The TLS trunks that stored the wrong default get the right one: unset.
UPDATE trunks
   SET transport_port = NULL
 WHERE transport_port = 5060
   AND transport_protocol IN ('tls', 'wss');
