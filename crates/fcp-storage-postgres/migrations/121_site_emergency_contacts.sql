-- A site's own emergency contacts (docs/E911.md "On-site notification"):
-- who in that building is told about an emergency call placed from it,
-- before the tenant's contacts, and whether the tenant's are told too.
-- Each contact is {kind, target, name?, listen?}, as the tenant's are.
ALTER TABLE locations
    ADD COLUMN IF NOT EXISTS emergency_contacts JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN IF NOT EXISTS notify_tenant_contacts BOOLEAN NOT NULL DEFAULT true;

-- The one contact a site had (a name and a phone number, which nothing
-- told) becomes its first: a short number an extension to ring, a longer
-- one a mobile to text, as the tenant's free-text contacts were read.
-- emergency_contact_name and emergency_contact_phone are read by nothing
-- from v0.25.456.
UPDATE locations
   SET emergency_contacts = jsonb_build_array(
           jsonb_strip_nulls(jsonb_build_object(
               'kind', CASE WHEN length(regexp_replace(emergency_contact_phone, '[^0-9]', '', 'g')) >= 7
                            THEN 'sms' ELSE 'call' END,
               'target', btrim(emergency_contact_phone),
               'name', NULLIF(btrim(emergency_contact_name), '')
           )))
 WHERE emergency_contacts = '[]'::jsonb
   AND emergency_contact_phone IS NOT NULL
   AND regexp_replace(emergency_contact_phone, '[^0-9]', '', 'g') <> '';
