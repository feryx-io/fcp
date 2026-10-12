-- A site's emergency call carries its caller's location up the link
-- (docs/HYBRID_CONTROL_PLANE.md §3.6): the link trunk takes the location by
-- value as well as the ELIN, so the site sends the PIDF-LO it resolved and
-- the cloud places the call there. Link trunks made before take it too; the
-- journal carries the change to their sites.
UPDATE trunks
   SET emergency_location = ARRAY['pidf_lo', 'elin']::TEXT[]
 WHERE link_end = 'cloud'
   AND emergency_location = ARRAY['elin']::TEXT[];
