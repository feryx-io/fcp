use bytes::Bytes;
use forge_codecs::AudioCodec;
use forge_conference::ConferenceRoom;
use forge_rtp::{JitterBuffer, RtpPacket, RtpSocketPair};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace, warn};

/// Per-participant audio bridge task.
///
/// Runs concurrently:
/// - Receives inbound RTP packets, decodes to PCM, writes to the conference mixer
/// - Every 20ms, reads the mix-minus output, encodes, and sends as RTP
///
/// This bypasses ForwardingEngine (designed for 1:1 B2BUA bridging) and
/// works directly with the forge-conference mixer.
pub async fn run_audio_bridge(
    room: Arc<ConferenceRoom>,
    participant_id: String,
    rtp_socket: Arc<RtpSocketPair>,
    mut encoder: Box<dyn AudioCodec>,
    mut decoder: Box<dyn AudioCodec>,
    payload_type: u8,
    frame_size: usize,
    jitter_buffer_ms: u64,
) {
    let mut seq: u16 = rand::random();
    let ssrc: u32 = rand::random();
    let mut timestamp: u32 = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    let mut jitter_buffer = JitterBuffer::new(Duration::from_millis(jitter_buffer_ms));

    debug!(
        participant = %participant_id,
        room = room.id(),
        frame_size = frame_size,
        payload_type = payload_type,
        codec = encoder.name(),
        jitter_buffer_ms = jitter_buffer_ms,
        "Audio bridge started"
    );

    loop {
        tokio::select! {
            // Branch 1: Receive inbound RTP → buffer → decode ready packets → write to mixer
            result = rtp_socket.recv_rtp() => {
                match result {
                    Ok((packet, _addr)) => {
                        // Push raw payload into jitter buffer
                        jitter_buffer.push(
                            packet.header.sequence_number,
                            packet.header.timestamp,
                            packet.payload.to_vec(),
                        );

                        // Drain all ready packets from the jitter buffer
                        while let Some(payload_data) = jitter_buffer.pop() {
                            match decoder.decode(&payload_data) {
                                Ok(pcm_samples) => {
                                    let _ = room.write_audio(&participant_id, &pcm_samples);
                                }
                                Err(e) => {
                                    trace!(participant = %participant_id, error = %e, "Decode error");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        debug!(
                            participant = %participant_id,
                            error = %e,
                            "RTP recv error, stopping bridge"
                        );
                        break;
                    }
                }
            }

            // Branch 2: 20ms tick → read mix-minus → encode → send RTP
            _ = interval.tick() => {
                match room.mix_for_participant(&participant_id) {
                    Ok(Some(mixed_pcm)) => {
                        match encoder.encode(&mixed_pcm) {
                            Ok(encoded) => {
                                let packet = RtpPacket::build(
                                    payload_type,
                                    seq,
                                    timestamp,
                                    ssrc,
                                    Bytes::from(encoded),
                                    false,
                                );
                                if let Err(e) = rtp_socket.send_rtp(&packet).await {
                                    trace!(participant = %participant_id, error = %e, "RTP send error");
                                }
                                seq = seq.wrapping_add(1);
                                timestamp = timestamp.wrapping_add(frame_size as u32);
                            }
                            Err(e) => {
                                trace!(participant = %participant_id, error = %e, "Encode error");
                            }
                        }
                    }
                    Ok(None) => {
                        // Silence — no audio to mix yet, still advance timestamp
                        timestamp = timestamp.wrapping_add(frame_size as u32);
                    }
                    Err(e) => {
                        // Participant was removed from room
                        warn!(
                            participant = %participant_id,
                            error = %e,
                            "Mix failed, participant likely removed"
                        );
                        break;
                    }
                }
            }
        }
    }

    debug!(
        participant = %participant_id,
        "Audio bridge stopped"
    );
}
