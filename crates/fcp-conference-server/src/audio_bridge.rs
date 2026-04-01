use bytes::Bytes;
use forge_codecs::AudioCodec;
use forge_conference::{ConferenceRoom, DtmfCommand, DtmfCommandHandler, ParticipantState};
use forge_dtmf::rfc2833::Rfc2833Detector;
use forge_dtmf::{DtmfDigit, DtmfEventType};
use forge_resampler::Resampler;
use forge_rtp::{JitterBuffer, RtpPacket, RtpSocketPair, SrtpContext};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, trace, warn, Instrument};

/// Maximum gain multiplier for participant audio (shared between DTMF and API).
pub const MAX_GAIN: f32 = 10.0;

/// Gain adjustment step per DTMF volume up/down press.
/// 0.25 gives ~4 presses to double volume from default 1.0.
const DTMF_GAIN_STEP: f32 = 0.25;

/// VAD silence threshold: RMS² below this value is considered silence (~-40 dBFS, ~100 RMS).
const VAD_SILENCE_THRESHOLD: i64 = 10_000;

/// Default telephone-event payload type (RFC 4733 standard assignment).
pub const DEFAULT_TE_PAYLOAD_TYPE: u8 = 101;

/// Audio quality statistics tracked per participant.
pub struct AudioStats {
    pub packets_received: AtomicU64,
    pub packets_decoded: AtomicU64,
    pub decode_errors: AtomicU64,
    pub packets_sent: AtomicU64,
    pub encode_errors: AtomicU64,
    pub dtmf_events: AtomicU64,
    /// Consecutive silent frames (PCM samples below threshold).
    /// Reset to 0 when voice activity is detected. Used for presence detection.
    pub silence_frames: AtomicU64,
}

impl AudioStats {
    pub fn new() -> Self {
        Self {
            packets_received: AtomicU64::new(0),
            packets_decoded: AtomicU64::new(0),
            decode_errors: AtomicU64::new(0),
            packets_sent: AtomicU64::new(0),
            encode_errors: AtomicU64::new(0),
            dtmf_events: AtomicU64::new(0),
            silence_frames: AtomicU64::new(0),
        }
    }

    /// Returns true if the participant has been silent for more than the given
    /// number of frames (at 50 frames/sec, 1500 = 30 seconds of silence).
    pub fn is_inactive(&self, threshold_frames: u64) -> bool {
        self.silence_frames.load(Ordering::Relaxed) > threshold_frames
    }

    /// Estimate MOS (Mean Opinion Score) from packet statistics.
    ///
    /// Uses a simplified E-model (ITU-T G.107) approximation:
    /// - Starts at 4.4 (excellent G.711 baseline)
    /// - Deducts for packet loss (decode_errors / packets_received)
    /// - Clamps to [1.0, 4.5]
    ///
    /// Returns None if no packets have been received yet.
    pub fn estimated_mos(&self) -> Option<f64> {
        let rx = self.packets_received.load(Ordering::Relaxed);
        if rx == 0 {
            return None;
        }
        let errors = self.decode_errors.load(Ordering::Relaxed);
        let loss_pct = (errors as f64 / rx as f64) * 100.0;

        // Simplified: MOS = 4.4 - 0.1 * loss% - 0.01 * loss%^2
        let mos = 4.4 - 0.1 * loss_pct - 0.01 * loss_pct * loss_pct;
        Some(mos.clamp(1.0, 4.5))
    }
}

/// Per-participant audio bridge task.
///
/// Runs concurrently:
/// - Receives inbound RTP packets, decodes to PCM, writes to the conference mixer
/// - Detects DTMF telephone-event packets and routes to command handler
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
    te_pt: Option<u8>,
    dtmf_clock_rate: u32,
    dtmf_handler: Option<Arc<DtmfCommandHandler>>,
    is_host: bool,
    stats: Arc<AudioStats>,
    srtp_context: Option<Arc<RwLock<SrtpContext>>>,
    mixer_sample_rate: u32,
    dtmf_muted_samples: Option<Arc<Vec<i16>>>,
    dtmf_unmuted_samples: Option<Arc<Vec<i16>>>,
    dtmf_locked_samples: Option<Arc<Vec<i16>>>,
    dtmf_unlocked_samples: Option<Arc<Vec<i16>>>,
) {
    let span = info_span!(
        "audio_bridge",
        participant = %participant_id,
        room = %room.id(),
        codec = %encoder.name(),
        srtp = srtp_context.is_some(),
    );
    async move {
    let mut seq: u16 = rand::random();
    let ssrc: u32 = rand::random();
    let mut timestamp: u32 = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    let mut jitter_buffer = JitterBuffer::new(Duration::from_millis(jitter_buffer_ms));

    // Output gain: controls how loud the conference mix sounds to THIS participant.
    // Adjusted via DTMF *7 (up) and *8 (down). Default 1.0 = unity.
    let mut output_gain: f32 = 1.0;

    // Resamplers for codec ↔ mixer rate conversion (e.g. G.722@16kHz ↔ mixer@8kHz)
    let codec_rate = decoder.native_format().sample_rate;
    let need_resample = codec_rate != mixer_sample_rate;
    let mut inbound_resampler = if need_resample {
        info!(codec_rate, mixer_sample_rate, "Resampling enabled: codec→mixer (inbound) and mixer→codec (outbound)");
        Some(Resampler::new(codec_rate, mixer_sample_rate, 1).expect("valid resampler"))
    } else {
        None
    };
    let mut outbound_resampler = if need_resample {
        Some(Resampler::new(mixer_sample_rate, codec_rate, 1).expect("valid resampler"))
    } else {
        None
    };

    // DTMF detector for telephone-event packets
    let mut dtmf_detector = if te_pt.is_some() {
        Some(Rfc2833Detector::new(dtmf_clock_rate))
    } else {
        None
    };
    let te_payload_type = te_pt.unwrap_or(DEFAULT_TE_PAYLOAD_TYPE);

    info!(
        frame_size = frame_size,
        payload_type = payload_type,
        jitter_buffer_ms = jitter_buffer_ms,
        te_pt = ?te_pt,
        dtmf_enabled = dtmf_handler.is_some(),
        is_host = is_host,
        "Audio bridge started"
    );

    // Drain stale RTP packets that accumulated in the kernel UDP buffer
    // during the welcome message / TTS playback delay. Without this,
    // hundreds of buffered packets arrive at once and flood the mixer.
    {
        let mut drained = 0u32;
        loop {
            match tokio::time::timeout(Duration::from_millis(1), rtp_socket.recv_rtp_raw()).await {
                Ok(Ok(_)) => { drained += 1; }
                _ => break,
            }
        }
        if drained > 0 {
            debug!(drained, "Drained stale RTP packets before bridge start");
        }
    }

    loop {
        tokio::select! {
            // Branch 1: Receive inbound RTP → decrypt (if SRTP) → buffer → decode → write to mixer
            result = rtp_socket.recv_rtp_raw() => {
                match result {
                    Ok((raw_data, _addr)) => {
                        stats.packets_received.fetch_add(1, Ordering::Relaxed);

                        // SRTP: decrypt if enabled (acquire and release lock before continuing)
                        let plain_data = if let Some(ref ctx) = srtp_context {
                            let result = {
                                let mut guard = ctx.write().await;
                                guard.unprotect_rtp(&raw_data)
                            }; // guard dropped here — lock released before any further awaits
                            match result {
                                Ok(d) => Bytes::from(d),
                                Err(e) => {
                                    trace!(participant = %participant_id, error = %e, "SRTP unprotect error");
                                    continue;
                                }
                            }
                        } else {
                            raw_data
                        };

                        // Parse RTP packet from (decrypted) bytes
                        let packet = match RtpPacket::parse(plain_data) {
                            Ok(p) => p,
                            Err(e) => {
                                trace!(participant = %participant_id, error = %e, "RTP parse error");
                                continue;
                            }
                        };

                        // Check if this is a telephone-event (DTMF) packet
                        if packet.header.payload_type() == te_payload_type {
                            if let Some(ref mut detector) = dtmf_detector {
                                match detector.process_with_timestamp(
                                    &packet.payload[..],
                                    packet.header.timestamp,
                                ) {
                                Ok(events) => {
                                    for event in events {
                                        if event.event_type == DtmfEventType::End {
                                            if let Some(ch) = dtmf_digit_to_char(&event.digit) {
                                                stats.dtmf_events.fetch_add(1, Ordering::Relaxed);
                                                // Route to DTMF command handler
                                                if let Some(ref handler) = dtmf_handler {
                                                    if let Some((cmd, _role)) =
                                                        handler.process_digit(&participant_id, ch)
                                                    {
                                                        let result = execute_dtmf_command(
                                                            &room,
                                                            &participant_id,
                                                            is_host,
                                                            cmd,
                                                        );
                                                        // Apply output gain adjustment
                                                        if result.gain_delta != 0.0 {
                                                            output_gain = (output_gain + result.gain_delta).clamp(0.0, MAX_GAIN);
                                                            info!(participant = %participant_id, output_gain = output_gain, "Output gain adjusted");
                                                        }
                                                        // Play feedback audio to THIS participant only
                                                        if let Some(ref fb) = result.feedback {
                                                            let fb_samples = match fb {
                                                                DtmfFeedback::Muted => &dtmf_muted_samples,
                                                                DtmfFeedback::Unmuted => &dtmf_unmuted_samples,
                                                                DtmfFeedback::Locked => &dtmf_locked_samples,
                                                                DtmfFeedback::Unlocked => &dtmf_unlocked_samples,
                                                            };
                                                            if let Some(ref pcm) = fb_samples {
                                                                // Map payload type to short codec name for create_codec_instance
                                                                let codec_name = match payload_type {
                                                                    0 => "PCMU",
                                                                    8 => "PCMA",
                                                                    9 => "G722",
                                                                    _ => "PCMU",
                                                                }.to_string();
                                                                info!(
                                                                    participant = %participant_id,
                                                                    codec = %codec_name,
                                                                    samples = pcm.len(),
                                                                    "Playing DTMF feedback audio"
                                                                );
                                                                match crate::conference_handler::create_codec_instance(&codec_name) {
                                                                    Ok(mut fb_encoder) => {
                                                                        crate::conference_handler::play_prompt_to_rtp(
                                                                            &rtp_socket, pcm, fb_encoder.as_mut(),
                                                                            payload_type, frame_size, &srtp_context,
                                                                            mixer_sample_rate,
                                                                        ).await;
                                                                        // Drain RTP that accumulated during playback
                                                                        loop {
                                                                            match tokio::time::timeout(
                                                                                Duration::from_millis(1),
                                                                                rtp_socket.recv_rtp_raw(),
                                                                            ).await {
                                                                                Ok(Ok(_)) => {}
                                                                                _ => break,
                                                                            }
                                                                        }
                                                                    }
                                                                    Err(e) => {
                                                                        warn!(error = %e, "Failed to create DTMF feedback encoder");
                                                                    }
                                                                }
                                                            } else {
                                                                warn!(participant = %participant_id, "No DTMF feedback samples loaded");
                                                            }
                                                        }
                                                        if result.should_leave {
                                                            debug!(
                                                                participant = %participant_id,
                                                                "DTMF leave command, stopping bridge"
                                                            );
                                                            return;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    trace!(participant = %participant_id, error = %e, "DTMF detection error");
                                }
                                }
                            }
                            continue; // Don't decode telephone-event as audio
                        }

                        // Push audio payload into jitter buffer
                        jitter_buffer.push(
                            packet.header.sequence_number,
                            packet.header.timestamp,
                            packet.payload.to_vec(),
                        );

                        // Drain all ready packets from the jitter buffer
                        while let Some(payload_data) = jitter_buffer.pop() {
                            match decoder.decode(&payload_data) {
                                Ok(pcm_samples) => {
                                    stats.packets_decoded.fetch_add(1, Ordering::Relaxed);

                                    // Resample from codec rate to mixer rate if needed
                                    // (e.g. G.722@16kHz → mixer@8kHz)
                                    let mixer_samples = if let Some(ref mut rs) = inbound_resampler {
                                        match rs.resample(&pcm_samples) {
                                            Ok(s) => s,
                                            Err(e) => {
                                                trace!(participant = %participant_id, error = %e, "Inbound resample error");
                                                pcm_samples
                                            }
                                        }
                                    } else {
                                        pcm_samples
                                    };

                                    // VAD-based presence: check if audio is silence
                                    // Threshold: RMS below ~100 (~-40 dBFS) is silence
                                    let rms_sq: i64 = mixer_samples.iter()
                                        .map(|&s| (s as i64) * (s as i64))
                                        .sum::<i64>()
                                        / mixer_samples.len().max(1) as i64;
                                    if rms_sq < VAD_SILENCE_THRESHOLD {
                                        stats.silence_frames.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        stats.silence_frames.store(0, Ordering::Relaxed);
                                    }

                                    if let Err(e) = room.write_audio(&participant_id, &mixer_samples) {
                                        warn!(participant = %participant_id, error = %e, "write_audio failed");
                                    }
                                }
                                Err(e) => {
                                    stats.decode_errors.fetch_add(1, Ordering::Relaxed);
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
                let mix_start = Instant::now();
                let mix_result = room.mix_for_participant(&participant_id);
                let mix_elapsed = mix_start.elapsed();
                // Deadlock detection: warn if mixing takes longer than 10ms
                // (should normally be sub-millisecond for in-memory mixing)
                if mix_elapsed > Duration::from_millis(10) {
                    warn!(
                        participant = %participant_id,
                        elapsed_ms = mix_elapsed.as_millis() as u64,
                        "Slow room mix operation — possible lock contention"
                    );
                }
                match mix_result {
                    Ok(Some(mixed_pcm)) => {
                        // Apply output gain (DTMF *7/*8 volume control)
                        let mixed_pcm = if (output_gain - 1.0).abs() > 0.01 {
                            mixed_pcm.iter().map(|&s| {
                                (s as f32 * output_gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16
                            }).collect::<Vec<i16>>()
                        } else {
                            mixed_pcm
                        };
                        // Resample from mixer rate to codec rate if needed
                        // (e.g. mixer@8kHz → G.722@16kHz)
                        let codec_pcm = if let Some(ref mut rs) = outbound_resampler {
                            match rs.resample(&mixed_pcm) {
                                Ok(s) => s,
                                Err(e) => {
                                    trace!(participant = %participant_id, error = %e, "Outbound resample error");
                                    mixed_pcm
                                }
                            }
                        } else {
                            mixed_pcm
                        };
                        match encoder.encode(&codec_pcm) {
                            Ok(encoded) => {
                                let packet = RtpPacket::build(
                                    payload_type,
                                    seq,
                                    timestamp,
                                    ssrc,
                                    Bytes::from(encoded),
                                    false,
                                );
                                // SRTP: encrypt if enabled, otherwise send plain
                                // Lock is scoped to just the crypto operation — not held during network I/O
                                let send_result = if let Some(ref ctx) = srtp_context {
                                    let packet_bytes = packet.to_bytes();
                                    let protect_result = {
                                        let mut guard = ctx.write().await;
                                        guard.protect_rtp(&packet_bytes)
                                    }; // guard dropped here — lock released before send
                                    match protect_result {
                                        Ok(protected) => rtp_socket.send_rtp_raw(&protected).await,
                                        Err(e) => {
                                            trace!(participant = %participant_id, error = %e, "SRTP protect error");
                                            Ok(0)
                                        }
                                    }
                                } else {
                                    rtp_socket.send_rtp(&packet).await
                                };
                                match send_result {
                                    Ok(_) => {
                                        stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        trace!(participant = %participant_id, error = %e, "RTP send error");
                                    }
                                }
                                seq = seq.wrapping_add(1);
                                timestamp = timestamp.wrapping_add(frame_size as u32);
                            }
                            Err(e) => {
                                stats.encode_errors.fetch_add(1, Ordering::Relaxed);
                                trace!(participant = %participant_id, error = %e, "Encode error");
                            }
                        }
                    }
                    Ok(None) => {
                        // No mix available (solo participant or nobody has enough
                        // samples). Drain our own buffered audio so it doesn't
                        // accumulate and hit the buffer limit while we wait for
                        // another participant to join.
                        let _ = room.get_all_participant_audio(frame_size, None);
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

    // Clean up DTMF state
    if let Some(ref handler) = dtmf_handler {
        handler.clear_participant(&participant_id);
    }

    let rx = stats.packets_received.load(Ordering::Relaxed);
    let tx = stats.packets_sent.load(Ordering::Relaxed);
    let errs = stats.decode_errors.load(Ordering::Relaxed) + stats.encode_errors.load(Ordering::Relaxed);
    info!(
        packets_rx = rx,
        packets_tx = tx,
        errors = errs,
        dtmf = stats.dtmf_events.load(Ordering::Relaxed),
        "Audio bridge stopped"
    );
    }.instrument(span).await
}

/// Result of executing a DTMF command
struct DtmfResult {
    /// True if the participant should leave (bridge should stop)
    should_leave: bool,
    /// Optional audio feedback to play to THIS participant only
    feedback: Option<DtmfFeedback>,
    /// Output gain adjustment (applied to the mix sent to this participant)
    gain_delta: f32,
}

enum DtmfFeedback {
    Muted,
    Unmuted,
    Locked,
    Unlocked,
}

/// Execute a DTMF command on the conference room.
fn execute_dtmf_command(
    room: &Arc<ConferenceRoom>,
    participant_id: &str,
    is_host: bool,
    command: DtmfCommand,
) -> DtmfResult {
    match command {
        DtmfCommand::MuteToggle => {
            // Toggle between Active and Muted
            let mut feedback = None;
            match room.get_participant_metadata(participant_id) {
                Ok(meta) => {
                    let new_state = match meta.state {
                        ParticipantState::Muted => ParticipantState::Active,
                        _ => ParticipantState::Muted,
                    };
                    if let Err(e) = room.set_participant_state(participant_id, new_state.clone()) {
                        warn!(error = %e, "Failed to toggle mute");
                    } else {
                        info!(participant = %participant_id, state = ?new_state, "DTMF mute toggle");
                        feedback = Some(match new_state {
                            ParticipantState::Muted => DtmfFeedback::Muted,
                            _ => DtmfFeedback::Unmuted,
                        });
                    }
                }
                Err(e) => warn!(error = %e, "Failed to get participant state for mute toggle"),
            }
            DtmfResult { should_leave: false, feedback, gain_delta: 0.0 }
        }
        DtmfCommand::VolumeUp => {
            info!(participant = %participant_id, "DTMF volume up");
            DtmfResult { should_leave: false, feedback: None, gain_delta: DTMF_GAIN_STEP }
        }
        DtmfCommand::VolumeDown => {
            info!(participant = %participant_id, "DTMF volume down");
            DtmfResult { should_leave: false, feedback: None, gain_delta: -DTMF_GAIN_STEP }
        }
        DtmfCommand::Info => {
            let count = room.participant_count();
            info!(participant = %participant_id, participant_count = count, "DTMF info request");
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::Leave => {
            info!(participant = %participant_id, "DTMF leave requested");
            DtmfResult { should_leave: true, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::MuteAll => {
            if is_host {
                for pid in room.participants() {
                    let _ = room.set_participant_state(&pid, ParticipantState::Muted);
                }
                info!(participant = %participant_id, "DTMF host: muted all");
            }
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::UnmuteAll => {
            if is_host {
                for pid in room.participants() {
                    let _ = room.set_participant_state(&pid, ParticipantState::Active);
                }
                info!(participant = %participant_id, "DTMF host: unmuted all");
            }
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::LockConference => {
            let mut feedback = None;
            if is_host {
                room.lock();
                info!(participant = %participant_id, "DTMF host: locked conference");
                feedback = Some(DtmfFeedback::Locked);
            }
            DtmfResult { should_leave: false, feedback, gain_delta: 0.0 }
        }
        DtmfCommand::UnlockConference => {
            let mut feedback = None;
            if is_host {
                room.unlock();
                info!(participant = %participant_id, "DTMF host: unlocked conference");
                feedback = Some(DtmfFeedback::Unlocked);
            }
            DtmfResult { should_leave: false, feedback, gain_delta: 0.0 }
        }
        DtmfCommand::EndConference => {
            if is_host {
                info!(participant = %participant_id, "DTMF host: ending conference");
                room.lock();
                // Remove all participants — their audio bridges will detect
                // the mix error and stop, triggering cleanup in the handler.
                for pid in room.participants() {
                    let _ = room.remove_participant(&pid);
                }
            }
            DtmfResult { should_leave: true, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::KickParticipant(participant_num) => {
            if is_host {
                let participants = room.participants();
                // Safe bounds check — participant list may have changed since DTMF was entered
                match participants.get(participant_num.wrapping_sub(1)) {
                    Some(target) => {
                        info!(participant = %participant_id, target = %target, num = participant_num, "DTMF host: kicking participant");
                        let _ = room.remove_participant(target);
                    }
                    None => {
                        debug!(participant = %participant_id, num = participant_num, count = participants.len(), "DTMF kick: invalid participant number");
                    }
                }
            }
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::DialOut(destination) => {
            if is_host {
                info!(
                    participant = %participant_id,
                    destination = %destination,
                    "DTMF host: dial-out requested"
                );
                // Signal the conference handler to initiate dial-out.
                // We can't do async SIP here, so publish via the room's
                // dial-out request channel. The handler polls this.
                room.request_dial_out(&destination);
            }
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
        DtmfCommand::Incomplete | DtmfCommand::InvalidCommand | DtmfCommand::EnterHostPin | DtmfCommand::EnterDialOut => {
            // Internal states, no action needed
            DtmfResult { should_leave: false, feedback: None, gain_delta: 0.0 }
        }
    }
}

/// Convert a DtmfDigit to its character representation.
fn dtmf_digit_to_char(digit: &DtmfDigit) -> Option<char> {
    match digit {
        DtmfDigit::Zero => Some('0'),
        DtmfDigit::One => Some('1'),
        DtmfDigit::Two => Some('2'),
        DtmfDigit::Three => Some('3'),
        DtmfDigit::Four => Some('4'),
        DtmfDigit::Five => Some('5'),
        DtmfDigit::Six => Some('6'),
        DtmfDigit::Seven => Some('7'),
        DtmfDigit::Eight => Some('8'),
        DtmfDigit::Nine => Some('9'),
        DtmfDigit::Star => Some('*'),
        DtmfDigit::Hash => Some('#'),
        DtmfDigit::A => Some('A'),
        DtmfDigit::B => Some('B'),
        DtmfDigit::C => Some('C'),
        DtmfDigit::D => Some('D'),
    }
}
