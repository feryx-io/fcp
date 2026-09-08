use crate::{MeetingId, TenantId, VideoCodec};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

fn default_true() -> bool {
    true
}

/// Composite layouts a meeting may select (`video_layout`).
pub const VIDEO_LAYOUTS: [&str; 4] = ["grid", "active_speaker", "spotlight", "pip"];

/// Output resolutions a meeting may cap at (`video_max_resolution`).
pub const VIDEO_RESOLUTIONS: [&str; 4] = ["180p", "360p", "720p", "1080p"];

/// Inclusive bounds for `video_max_participants` (tiles in the composite).
pub const VIDEO_MAX_PARTICIPANTS_RANGE: std::ops::RangeInclusive<u32> = 1..=16;

/// Inclusive bounds for `video_fps`.
pub const VIDEO_FPS_RANGE: std::ops::RangeInclusive<u32> = 1..=30;

pub fn default_video_layout() -> String {
    "grid".to_string()
}

pub fn default_video_max_participants() -> u32 {
    16
}

pub fn default_video_max_resolution() -> String {
    "720p".to_string()
}

pub fn default_video_fps() -> u32 {
    15
}

pub fn default_video_codecs() -> Vec<VideoCodec> {
    vec![VideoCodec::H264, VideoCodec::Vp8]
}

/// When the passthrough fast path applies (§5.6).
pub const VIDEO_PASSTHROUGH_MODES: [&str; 3] = ["off", "auto", "on"];

pub fn default_video_passthrough() -> String {
    "auto".to_string()
}

/// Validate the per-meeting video settings against the allowed layouts,
/// resolutions and ranges. Returns a human-readable message on failure,
/// suitable for a 400 response body.
#[allow(clippy::too_many_arguments)]
pub fn validate_video_settings(
    layout: &str,
    max_participants: u32,
    max_resolution: &str,
    fps: u32,
    codecs: &[VideoCodec],
    passthrough: &str,
) -> Result<(), String> {
    if !VIDEO_PASSTHROUGH_MODES.contains(&passthrough) {
        return Err(format!(
            "video_passthrough must be one of {}",
            VIDEO_PASSTHROUGH_MODES.join(", ")
        ));
    }
    if !VIDEO_LAYOUTS.contains(&layout) {
        return Err(format!(
            "video_layout must be one of {}",
            VIDEO_LAYOUTS.join(", ")
        ));
    }
    if !VIDEO_MAX_PARTICIPANTS_RANGE.contains(&max_participants) {
        return Err(format!(
            "video_max_participants must be between {} and {}",
            VIDEO_MAX_PARTICIPANTS_RANGE.start(),
            VIDEO_MAX_PARTICIPANTS_RANGE.end()
        ));
    }
    if !VIDEO_RESOLUTIONS.contains(&max_resolution) {
        return Err(format!(
            "video_max_resolution must be one of {}",
            VIDEO_RESOLUTIONS.join(", ")
        ));
    }
    if !VIDEO_FPS_RANGE.contains(&fps) {
        return Err(format!(
            "video_fps must be between {} and {}",
            VIDEO_FPS_RANGE.start(),
            VIDEO_FPS_RANGE.end()
        ));
    }
    if codecs.is_empty() {
        return Err("video_codecs must list at least one codec".to_string());
    }
    for (i, codec) in codecs.iter().enumerate() {
        if codecs[..i].contains(codec) {
            return Err(format!("video_codecs lists {} more than once", codec));
        }
    }
    Ok(())
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

    /// Whether the room composites and delivers video (audio-only when false)
    #[serde(default)]
    pub video_enabled: bool,

    /// Composite layout: one of [`VIDEO_LAYOUTS`] (default "grid")
    #[serde(default = "default_video_layout")]
    pub video_layout: String,

    /// Maximum number of tiles in the composite, 1..=16 (default 16)
    #[serde(default = "default_video_max_participants")]
    pub video_max_participants: u32,

    /// Output resolution cap: one of [`VIDEO_RESOLUTIONS`] (default "720p")
    #[serde(default = "default_video_max_resolution")]
    pub video_max_resolution: String,

    /// Output frame rate, 1..=30 (default 15)
    #[serde(default = "default_video_fps")]
    pub video_fps: u32,

    /// Leave each participant's own tile out of the composite sent back to them
    #[serde(default)]
    pub video_exclude_self: bool,

    /// Video codecs offered, in preference order (default H264, VP8)
    #[serde(default = "default_video_codecs")]
    pub video_codecs: Vec<VideoCodec>,

    /// When a participant whose view is a single source they can already
    /// decode is sent that source's own stream rather than a composite
    /// (`docs/VIDEO_CONFERENCING.md` §5.6): `auto` (the default — only
    /// while the room is showing one participant full canvas), `on` or
    /// `off`. A forwarded picture carries no name banner or speaking
    /// border, because nothing composited it.
    #[serde(default = "default_video_passthrough")]
    pub video_passthrough: String,

    /// Creation timestamp
    pub created_at: SystemTime,

    /// Last modification timestamp
    pub updated_at: SystemTime,
}

impl Meeting {
    /// Validate this meeting's video settings; see [`validate_video_settings`].
    pub fn validate_video_settings(&self) -> Result<(), String> {
        validate_video_settings(
            &self.video_layout,
            self.video_max_participants,
            &self.video_max_resolution,
            self.video_fps,
            &self.video_codecs,
            &self.video_passthrough,
        )
    }
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
            video_enabled: true,
            video_layout: "active_speaker".to_string(),
            video_max_participants: 9,
            video_max_resolution: "1080p".to_string(),
            video_fps: 30,
            video_exclude_self: true,
            video_passthrough: default_video_passthrough(),
            video_codecs: vec![VideoCodec::Vp8, VideoCodec::H264, VideoCodec::Av1],
            created_at: UNIX_EPOCH + std::time::Duration::from_secs(1700000000),
            updated_at: UNIX_EPOCH + std::time::Duration::from_secs(1700000000),
        };

        let json = serde_json::to_string(&meeting).unwrap();
        assert!(json.contains("\"video_codecs\":[\"VP8\",\"H264\",\"AV1\"]"));
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
        assert!(parsed.video_enabled);
        assert_eq!(parsed.video_layout, "active_speaker");
        assert_eq!(parsed.video_max_participants, 9);
        assert_eq!(parsed.video_max_resolution, "1080p");
        assert_eq!(parsed.video_fps, 30);
        assert!(parsed.video_exclude_self);
        assert_eq!(
            parsed.video_codecs,
            vec![VideoCodec::Vp8, VideoCodec::H264, VideoCodec::Av1]
        );
        parsed.validate_video_settings().unwrap();
    }

    #[test]
    fn meeting_without_video_fields_deserialises_to_defaults() {
        let json = serde_json::json!({
            "id": MeetingId::new(),
            "tenant_id": TenantId::new(),
            "room_number": "8002",
            "name": "Legacy",
            "guest_pin": null,
            "host_pin": null,
            "require_guest_pin": false,
            "max_participants": 0,
            "wait_for_moderator": false,
            "enabled": true,
            "auto_record": false,
            "default_locked": false,
            "permanent": false,
            "created_at": { "secs_since_epoch": 1700000000, "nanos_since_epoch": 0 },
            "updated_at": { "secs_since_epoch": 1700000000, "nanos_since_epoch": 0 },
        });
        let parsed: Meeting = serde_json::from_value(json).unwrap();
        assert!(!parsed.video_enabled);
        assert_eq!(parsed.video_layout, "grid");
        assert_eq!(parsed.video_max_participants, 16);
        assert_eq!(parsed.video_max_resolution, "720p");
        assert_eq!(parsed.video_fps, 15);
        assert!(!parsed.video_exclude_self);
        assert_eq!(parsed.video_codecs, vec![VideoCodec::H264, VideoCodec::Vp8]);
        parsed.validate_video_settings().unwrap();
    }

    #[test]
    fn video_settings_validation_rejects_out_of_range_values() {
        let ok = |layout, tiles, res, fps, codecs: &[VideoCodec]| {
            validate_video_settings(layout, tiles, res, fps, codecs, "auto")
        };
        let codecs = default_video_codecs();
        assert!(ok("grid", 16, "720p", 15, &codecs).is_ok());
        assert!(ok("pip", 1, "180p", 1, &codecs).is_ok());
        assert!(ok("spotlight", 16, "1080p", 30, &codecs).is_ok());

        assert!(ok("mosaic", 16, "720p", 15, &codecs)
            .unwrap_err()
            .contains("video_layout"));
        assert!(ok("grid", 0, "720p", 15, &codecs)
            .unwrap_err()
            .contains("video_max_participants"));
        assert!(ok("grid", 17, "720p", 15, &codecs)
            .unwrap_err()
            .contains("video_max_participants"));
        assert!(ok("grid", 16, "4k", 15, &codecs)
            .unwrap_err()
            .contains("video_max_resolution"));
        assert!(ok("grid", 16, "720p", 0, &codecs)
            .unwrap_err()
            .contains("video_fps"));
        assert!(ok("grid", 16, "720p", 31, &codecs)
            .unwrap_err()
            .contains("video_fps"));
        assert!(ok("grid", 16, "720p", 15, &[])
            .unwrap_err()
            .contains("video_codecs"));
        assert!(ok(
            "grid",
            16,
            "720p",
            15,
            &[VideoCodec::H264, VideoCodec::H264]
        )
        .unwrap_err()
        .contains("more than once"));
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
