-- A user's first and last name (#343), filled by administrators, the user,
-- SCIM (`name.givenName`/`name.familyName`) and single sign-on; the
-- display name, unset, reads as the two.
ALTER TABLE users ADD COLUMN IF NOT EXISTS first_name TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_name TEXT;

-- Which greeting a mailbox's callers hear: `standard` (the node's default
-- greeting), `personalized` (spoken once by TTS with the owner's name and
-- kept as a `personalized` greeting) or `own` (the greetings the owner
-- recorded, as before). NULL is the default: personalized when the owner
-- has a first and last name, standard otherwise. `greeting_name` is the
-- name a personalized greeting says: `full_name`, `display_name` or
-- `recorded` (the mailbox's name greeting spliced into the sentence).
ALTER TABLE voicemail_mailboxes ADD COLUMN IF NOT EXISTS greeting_choice TEXT
    CHECK (greeting_choice IN ('standard', 'personalized', 'own'));
ALTER TABLE voicemail_mailboxes ADD COLUMN IF NOT EXISTS greeting_name TEXT NOT NULL DEFAULT 'full_name'
    CHECK (greeting_name IN ('full_name', 'display_name', 'recorded'));

-- A mailbox that already has a greeting of its own keeps playing it.
UPDATE voicemail_mailboxes mb SET greeting_choice = 'own'
 WHERE greeting_choice IS NULL
   AND EXISTS (SELECT 1 FROM voicemail_greetings g
                WHERE g.mailbox_id = mb.id
                  AND g.greeting_type IN ('unavailable', 'busy', 'temporary'));

-- The personalized greeting, and what it was made from, so it is made
-- again only when the name or the choice changes.
ALTER TABLE voicemail_greetings DROP CONSTRAINT IF EXISTS voicemail_greetings_greeting_type_check;
ALTER TABLE voicemail_greetings ADD CONSTRAINT voicemail_greetings_greeting_type_check
    CHECK (greeting_type IN ('unavailable', 'busy', 'name', 'temporary', 'personalized'));
ALTER TABLE voicemail_greetings ADD COLUMN IF NOT EXISTS made_from TEXT;
