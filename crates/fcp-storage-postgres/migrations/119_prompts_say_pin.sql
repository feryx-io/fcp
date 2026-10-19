-- The system prompts said "PIN", which text to speech spells out as three
-- letters (docs/DEEP_TESTING.md D8). The built-in texts now say "pin"; a
-- prompt still holding an old built-in text takes the new one, and audio
-- made from the old text is let go so the call manager speaks it again.
UPDATE prompts SET tts_text = 'Enter your pin, followed by pound.', audio_path = NULL, updated_at = NOW()
WHERE is_system AND name = 'vm_enter_pin' AND tts_text = 'Enter your PIN followed by pound.';

UPDATE prompts SET tts_text = 'Incorrect pin. Please try again.', audio_path = NULL, updated_at = NOW()
WHERE is_system AND name = 'vm_incorrect_pin' AND tts_text = 'Incorrect PIN. Please try again.';

UPDATE prompts SET tts_text = 'Please enter your conference pin, followed by the pound key.', audio_path = NULL, updated_at = NOW()
WHERE is_system AND name = 'conf_pin_prompt' AND tts_text = 'Please enter your conference PIN, followed by the pound key.';

UPDATE prompts SET tts_text = 'Invalid pin. Please try again.', audio_path = NULL, updated_at = NOW()
WHERE is_system AND name = 'conf_invalid_pin' AND tts_text = 'Invalid PIN. Please try again.';
