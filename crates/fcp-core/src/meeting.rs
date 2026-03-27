use crate::{MeetingId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

fn default_true() -> bool {
    true
}

/// Persistent meeting configuration for conference rooms.
///
/// A `Meeting` is the database-backed configuration for a conference room.
/// It survives restarts and defines how the room behaves when participants connect.
/// The runtime `ConferenceRoom` (from forge-conference) is ephemeral and only
/// exists while participants are connected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    /// Unique meeting identifier
    pub id: MeetingId,

    /// Tenant for multi-tenant isolation
    pub tenant_id: TenantId,

    /// Room number — the SIP Request-URI user part (e.g. "8001")
    pub room_number: String,

    /// Human-readable meeting name
    pub name: String,

    /// PIN required for guest participants (None = no PIN)
    pub guest_pin: Option<String>,

    /// PIN that grants host/moderator privileges (None = no host PIN)
    pub host_pin: Option<String>,

    /// Whether guests must enter a PIN to join
    pub require_guest_pin: bool,

    /// Maximum number of participants (0 = unlimited)
    pub max_participants: usize,

    /// Hold non-host participants until a host joins
    pub wait_for_moderator: bool,

    /// Whether this meeting is active and accepting calls
    pub enabled: bool,

    /// Automatically start recording when the first participant joins
    pub auto_record: bool,

    /// Room starts in locked state (no new participants until unlocked by host)
    pub default_locked: bool,

    /// Room persists in memory even when all participants leave
    pub permanent: bool,

    /// Optional owner username — user who can manage this meeting via self-service
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,

    /// Per-meeting override: path to PIN prompt WAV file (None = use global config)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_prompt_sound: Option<String>,

    /// Per-meeting override: path to invalid PIN WAV file (None = use global config)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_pin_sound: Option<String>,

    /// Per-meeting override: path to MOH WAV file (None = use global config)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moh_sound: Option<String>,

    /// TTS text to synthesize for the PIN prompt (used when no sound file is configured)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_prompt_tts: Option<String>,

    /// TTS text to synthesize for invalid PIN message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_pin_tts: Option<String>,

    /// TTS text for the welcome message when joining the conference.
    /// Supports {count} placeholder for current participant count.
    /// None = use default ("You have joined the conference. There are N participants.")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome_tts: Option<String>,

    /// Play a tone when a participant joins
    #[serde(default = "default_true")]
    pub play_entry_tone: bool,

    /// Play a tone when a participant leaves
    #[serde(default = "default_true")]
    pub play_exit_tone: bool,

    /// Override path for join tone WAV file (None = use server default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_sound: Option<String>,

    /// Override path for exit tone WAV file (None = use server default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_sound: Option<String>,

    /// Creation timestamp
    pub created_at: SystemTime,

    /// Last modification timestamp
    pub updated_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn meeting_serde_roundtrip() {
        let meeting = Meeting {
            id: MeetingId::new(),
            tenant_id: TenantId::new(),
            room_number: "8001".to_string(),
            name: "Team Standup".to_string(),
            guest_pin: Some("1234".to_string()),
            host_pin: Some("9999".to_string()),
            require_guest_pin: true,
            max_participants: 50,
            wait_for_moderator: false,
            enabled: true,
            auto_record: false,
            default_locked: false,
            permanent: false,
            owner_id: Some("alice".to_string()),
            pin_prompt_sound: None,
            invalid_pin_sound: None,
            moh_sound: None,
            pin_prompt_tts: None,
            invalid_pin_tts: None,
            welcome_tts: None,
            play_entry_tone: true,
            play_exit_tone: true,
            join_sound: Some("/sounds/join.wav".to_string()),
            exit_sound: None,
            created_at: UNIX_EPOCH + std::time::Duration::from_secs(1700000000),
            updated_at: UNIX_EPOCH + std::time::Duration::from_secs(1700000000),
        };

        let json = serde_json::to_string(&meeting).unwrap();
        let parsed: Meeting = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.room_number, "8001");
        assert_eq!(parsed.name, "Team Standup");
        assert_eq!(parsed.guest_pin, Some("1234".to_string()));
        assert!(parsed.require_guest_pin);
        assert_eq!(parsed.max_participants, 50);
        assert!(parsed.play_entry_tone);
        assert!(parsed.play_exit_tone);
        assert_eq!(parsed.join_sound, Some("/sounds/join.wav".to_string()));
        assert_eq!(parsed.exit_sound, None);
    }

    #[test]
    fn meeting_id_generation_and_parsing() {
        let id = MeetingId::new();
        let s = id.to_string();
        assert!(s.starts_with("mtg-"));
        let parsed: MeetingId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }
}
