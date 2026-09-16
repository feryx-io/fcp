-- E911 block 2 phase 2 (docs/E911_BLOCK_2.md §5): location by value.
--
-- A site's street address as RFC 5139 civic elements (house number, street
-- name and suffix, directions, unit, room, …), kept beside the line fields
-- people edit; phone number and device addresses already store JSON and
-- gain the same elements there. An emergency call's record says whether the
-- location it carried was structured, and the Geolocation-Error a trunk
-- returned, if any.

ALTER TABLE locations ADD COLUMN IF NOT EXISTS civic JSONB;

ALTER TABLE emergency_calls
    ADD COLUMN IF NOT EXISTS location_structured BOOLEAN,
    ADD COLUMN IF NOT EXISTS geolocation_error TEXT;
