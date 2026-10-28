-- D8 findings 27 and 31.
--
-- A user is listed in the tenant's directory (/me/directory) unless an
-- administrator, or the user where the tenant lets them
-- (tenants.directory_json users_may_unlist), takes them out. An unlisted
-- user still rings on their extension and is seen by administrators.
ALTER TABLE users ADD COLUMN IF NOT EXISTS directory_listed BOOLEAN NOT NULL DEFAULT TRUE;

-- The ringtone a user's apps ring with for a call from one of a contact's
-- numbers, by the app's id for it; NULL rings with the app's own choice.
ALTER TABLE personal_contacts ADD COLUMN IF NOT EXISTS ringtone TEXT;
