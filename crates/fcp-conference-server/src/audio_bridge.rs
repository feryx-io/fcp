use forge_conference::ConferenceRoom;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace};

/// Per-participant audio bridge task.
///
/// Runs at 20ms intervals, reading incoming RTP packets from the participant,
/// decoding them to PCM, writing to the conference mixer, then reading the
/// mix-minus output and encoding back to RTP for send.
///
/// This bypasses ForwardingEngine (designed for 1:1 B2BUA bridging) and
/// works directly with the forge-conference mixer.
pub async fn run_audio_bridge(
    room: Arc<ConferenceRoom>,
    participant_id: String,
    frame_size: usize,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(20));

    debug!(
        participant = %participant_id,
        room = room.id(),
        frame_size = frame_size,
        "Audio bridge started"
    );

    loop {
        interval.tick().await;

        // Read mix-minus audio for this participant
        match room.mix_for_participant(&participant_id) {
            Ok(Some(_mixed)) => {
                // In a full implementation, we would:
                // 1. Encode the PCM samples to the negotiated codec (PCMU/PCMA/G.722)
                // 2. Send as RTP packet via the participant's forge-rtp session
                //
                // For now, the mixer is running and producing output.
                // RTP send/receive integration will be wired when the SIP INVITE
                // handler establishes the actual RTP session.
                trace!(participant = %participant_id, "Mixed audio frame ready");
            }
            Ok(None) => {
                // No audio available yet (silence)
            }
            Err(e) => {
                // Participant was removed from room
                debug!(
                    participant = %participant_id,
                    error = %e,
                    "Mix failed, participant likely removed"
                );
                break;
            }
        }
    }

    debug!(
        participant = %participant_id,
        "Audio bridge stopped"
    );
}
