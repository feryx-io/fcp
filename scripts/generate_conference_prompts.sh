#!/bin/bash
# Generate default conference audio prompts using espeak-ng.
# These are the built-in defaults; users can override per-meeting via
# the Meeting API (pin_prompt_sound, invalid_pin_sound, moh_sound fields)
# or by setting pin_prompt_tts / invalid_pin_tts for TTS synthesis.

set -e

ANNOUNCEMENTS_DIR="${1:-/tmp/fcp/conference/announcements}"
mkdir -p "$ANNOUNCEMENTS_DIR"

VOICE="${VOICE:-en-us}"
SPEED="${SPEED:-150}"

echo "Generating conference prompts in $ANNOUNCEMENTS_DIR ..."

espeak-ng -v "$VOICE" -s "$SPEED" -w "$ANNOUNCEMENTS_DIR/pin_prompt.wav" \
  "Please enter your conference PIN, followed by the pound key."

espeak-ng -v "$VOICE" -s "$SPEED" -w "$ANNOUNCEMENTS_DIR/invalid_pin.wav" \
  "Invalid PIN. Please try again."

espeak-ng -v "$VOICE" -s 140 -w "$ANNOUNCEMENTS_DIR/moh.wav" \
  "Please wait. The conference will begin when the moderator joins."

echo "Done:"
ls -lh "$ANNOUNCEMENTS_DIR"/*.wav
