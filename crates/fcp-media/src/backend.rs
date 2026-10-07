//! Media backend abstraction
//!
//! Provides a unified interface for both embedded and remote media backends.

use crate::{adapters, Error, MediaSession, MediaSessionConfig, Result};
use async_trait::async_trait;
use fcp_core::{CallId, DialogId};
use serde::Serialize;
use std::net::IpAddr;
use std::sync::Arc;

/// Information about an active RTP session (for REST API)
#[derive(Debug, Clone, Serialize)]
pub struct RtpSessionInfo {
    /// FCP Call ID
    pub call_id: String,
    /// Dialog ID
    pub dialog_id: String,
    /// Session state
    pub state: String,
    /// A-leg (caller) information
    pub a_leg: RtpLegInfo,
    /// B-leg (callee) information
    pub b_leg: RtpLegInfo,
    /// Whether transcoding is active
    pub transcoding_active: bool,
    /// Local RTP port
    pub local_rtp_port: u16,
    /// Session duration in seconds
    pub duration_secs: u64,
}

/// Information about one leg of an RTP session
#[derive(Debug, Clone, Serialize)]
pub struct RtpLegInfo {
    /// Remote endpoint (IP:port)
    pub remote_addr: Option<String>,
    /// Codec name (e.g., "G722", "PCMU")
    pub codec: String,
    /// Codec clock rate (Hz)
    pub clock_rate: u32,
    /// RTP payload type
    pub payload_type: u8,
    /// Packets received
    pub packets_received: u64,
    /// Packets sent
    pub packets_sent: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Bytes sent
    pub bytes_sent: u64,
}

/// One leg of a two-legged session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    A,
    B,
}

/// Media backend trait
///
/// This trait abstracts over different deployment models:
/// - EmbeddedBackend: forge-media in-process
/// - RemoteBackend: forge-media as HTTP service
#[async_trait]
pub trait MediaBackend: Send + Sync {
    /// Create a new media session
    async fn create_session(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
    ) -> Result<Arc<MediaSession>>;

    /// Get an existing session
    async fn get_session(&self, call_id: &CallId) -> Result<Arc<MediaSession>>;

    /// Update session with SDP offer/answer
    async fn update_session(&self, call_id: &CallId, remote_sdp: String) -> Result<String>; // Returns local SDP

    /// Set the B-leg remote endpoint from SDP.
    ///
    /// Used by B2BUA when the second call leg answers. This sets participant_b's
    /// remote address so the ForwardingEngine knows where to send RTP packets
    /// destined for the B-leg endpoint.
    async fn update_session_b_leg(&self, call_id: &CallId, remote_sdp: String) -> Result<()>;

    /// As [`MediaBackend::update_session_b_leg`], with the SRTP offer this
    /// node made on that leg (`docs/TENANT_TRUNKS.md` §6).
    ///
    /// In SDES each side states the key it sends with, so the key this node
    /// encrypts with is the one it advertised — generating a fresh one on
    /// the answer would encrypt to a key the far end never saw. A backend
    /// that cannot take an offer falls back to the plain update.
    async fn update_session_b_leg_offered(
        &self,
        call_id: &CallId,
        remote_sdp: String,
        _offer: Option<crate::sdp::SrtpOffer>,
    ) -> Result<()> {
        self.update_session_b_leg(call_id, remote_sdp).await
    }

    /// Put session on hold
    /// Learn `leg`'s RTP address from its first packet — one from
    /// `source`, and from nowhere else — instead of from its SDP, which a
    /// peer behind NAT states wrong (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    /// A backend that cannot leaves the leg as its SDP said.
    async fn latch_leg(&self, call_id: &CallId, leg: Leg, source: IpAddr) -> Result<()> {
        let _ = (call_id, leg, source);
        Ok(())
    }

    async fn hold_session(&self, call_id: &CallId) -> Result<()>;

    /// Resume session from hold
    async fn resume_session(&self, call_id: &CallId) -> Result<()>;

    /// Terminate a media session
    async fn terminate_session(&self, call_id: &CallId) -> Result<()>;

    /// Get session statistics
    async fn get_stats(&self, call_id: &CallId) -> Result<crate::session::MediaSessionStats>;

    /// Send a DTMF digit to one leg, as RFC 2833 telephone-event at the
    /// payload type that leg negotiated.
    async fn send_dtmf(
        &self,
        call_id: &CallId,
        leg: MediaLeg,
        digit: char,
        duration_ms: u32,
    ) -> Result<()>;

    /// Whether telephone-event the other leg sends is relayed to `leg`.
    /// Off for a leg that did not negotiate it; the digits are still
    /// detected ([`MediaBackend::subscribe_digits`]).
    fn set_rfc2833_relay_to(&self, _call_id: &CallId, _leg: MediaLeg, _relay: bool) -> Result<()> {
        Err(Error::Backend(format!(
            "the {} backend cannot steer RFC 2833 relay",
            self.backend_name()
        )))
    }

    /// Every digit either leg sends, with the leg it came from, until the
    /// session ends. `None` when this backend cannot say which leg.
    fn subscribe_digits(
        &self,
        _call_id: &CallId,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<LegDigit>> {
        None
    }

    /// One leg's audio as this node hears it, as 16 kHz mono PCM frames
    /// ([`crate::stt::STT_SAMPLE_RATE`]) of about 20 ms, until the session
    /// ends or the receiver is dropped. A listener a second behind loses
    /// frames rather than holding the call's media up.
    async fn listen(
        &self,
        _call_id: &CallId,
        _leg: MediaLeg,
    ) -> Result<tokio::sync::mpsc::Receiver<Vec<i16>>> {
        Err(Error::Backend(format!(
            "the {} backend cannot listen to a call",
            self.backend_name()
        )))
    }

    /// Create a DTMF collector wired to the media backend's event stream.
    ///
    /// The returned collector automatically receives DTMF events for the
    /// given call from the media engine.
    fn create_dtmf_collector(
        &self,
        _call_id: &CallId,
        max_digits: usize,
        finish_on_key: Option<char>,
    ) -> crate::dtmf_collector::DtmfCollector {
        // Default: disconnected collector (no DTMF events will arrive)
        crate::dtmf_collector::DtmfCollector::new(max_digits, finish_on_key)
    }

    /// Start recording
    async fn start_recording(&self, call_id: &CallId, path: String) -> Result<()>;

    /// Stop recording
    async fn stop_recording(&self, call_id: &CallId) -> Result<()>;

    /// Play an audio file into the call
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call to inject audio into
    /// * `file_path` - Path to the audio file (WAV, MP3, etc.)
    /// * `target` - Which participant(s) should hear the audio
    ///
    /// # Returns
    ///
    /// A playback ID that can be used to stop the playback
    async fn play_file(
        &self,
        call_id: &CallId,
        file_path: String,
        target: AudioTarget,
    ) -> Result<String>;

    /// Add a supervisor to the call's media: a port of their own, returned
    /// as the SDP to offer their phone (`docs/ENTERPRISE_CALLING.md` §3.11).
    async fn add_party(&self, _call_id: &CallId, _mode: Supervision) -> Result<String> {
        Err(Error::Backend(
            "this media backend does not take a supervisor".to_string(),
        ))
    }

    /// The supervisor's SDP (their answer, or their offer): where they are
    /// and which of PCMU and PCMA they send. Returns the answer to it.
    async fn connect_party(&self, _call_id: &CallId, _remote_sdp: String) -> Result<String> {
        Err(Error::Backend(
            "this media backend does not take a supervisor".to_string(),
        ))
    }

    /// Change how the supervisor takes part.
    async fn set_party_mode(&self, _call_id: &CallId, _mode: Supervision) -> Result<()> {
        Err(Error::Backend(
            "this media backend does not take a supervisor".to_string(),
        ))
    }

    /// Remove the supervisor; the legs are relayed as before.
    async fn remove_party(&self, _call_id: &CallId) -> Result<()> {
        Err(Error::Backend(
            "this media backend does not take a supervisor".to_string(),
        ))
    }

    /// As [`MediaBackend::play_file`], starting the file again at its end
    /// until stopped (hold music, `docs/ENTERPRISE_CALLING.md` §3.3).
    async fn play_file_looped(
        &self,
        _call_id: &CallId,
        _file_path: String,
        _target: AudioTarget,
    ) -> Result<String> {
        Err(Error::Backend(
            "this media backend does not loop playback".to_string(),
        ))
    }

    /// Play text-to-speech into the call
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call to inject audio into
    /// * `text` - Text to convert to speech
    /// * `language` - Language code (e.g., "en-US")
    /// * `voice` - Voice identifier (provider-specific)
    /// * `target` - Which participant(s) should hear the audio
    ///
    /// # Returns
    ///
    /// A playback ID that can be used to stop the playback
    async fn play_tts(
        &self,
        call_id: &CallId,
        text: String,
        language: String,
        voice: Option<String>,
        target: AudioTarget,
    ) -> Result<String>;

    /// Play a generated tone into the call
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call to inject audio into
    /// * `frequency` - Tone frequency in Hz (e.g., 1000.0 for 1kHz beep)
    /// * `duration_ms` - Duration in milliseconds
    /// * `amplitude` - Amplitude from 0.0 to 1.0
    /// * `target` - Which participant(s) should hear the tone
    ///
    /// # Returns
    ///
    /// A playback ID that can be used to stop or wait for the tone
    async fn play_tone(
        &self,
        call_id: &CallId,
        frequency: f32,
        duration_ms: u32,
        amplitude: f32,
        target: AudioTarget,
    ) -> Result<String>;

    /// Stop an active playback
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call with active playback
    /// * `playback_id` - ID returned from play_file or play_tts
    async fn stop_playback(&self, call_id: &CallId, playback_id: String) -> Result<()>;

    /// Wait for a playback to complete
    ///
    /// # Arguments
    ///
    /// * `playback_id` - ID returned from play_file or play_tts
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when playback completes successfully, or an error if playback failed
    async fn wait_playback_completion(&self, playback_id: String) -> Result<()>;

    /// Bridge two calls together for B2BUA operation
    ///
    /// # Arguments
    ///
    /// * `a_leg_call_id` - Call ID of the A-leg (incoming call)
    /// * `b_leg_call_id` - Call ID of the B-leg (outbound call)
    ///
    /// # Description
    ///
    /// This method configures the media engine to forward RTP packets bidirectionally
    /// between the two call legs. Audio from A-leg is sent to B-leg, and vice versa.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when bridging is established, or an error if either session
    /// doesn't exist or bridging fails.
    async fn bridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()>;

    /// Unbridge two calls
    ///
    /// # Arguments
    ///
    /// * `a_leg_call_id` - Call ID of the A-leg
    /// * `b_leg_call_id` - Call ID of the B-leg
    ///
    /// # Description
    ///
    /// Removes the media bridge between two calls. After unbridging, each call
    /// returns to its independent state (silence or can be bridged to other calls).
    async fn unbridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()>;

    /// The pool the backend's sessions take their RTP ports from, when it
    /// has one on this node. Anything else on the node that opens RTP
    /// sockets in the same range allocates from it, so two pools never
    /// hand out one port.
    fn port_pool(&self) -> Option<Arc<forge_rtp::PortPool>> {
        None
    }

    /// Get backend type name (for logging/metrics)
    fn backend_name(&self) -> &'static str;

    /// Health check
    async fn health_check(&self) -> Result<bool>;

    /// List all active RTP sessions with their details
    async fn list_sessions(&self) -> Vec<RtpSessionInfo>;
}

/// How a supervisor takes part in a call's media
/// (`docs/ENTERPRISE_CALLING.md` §3.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Supervision {
    /// Hears both legs, heard by nobody.
    Monitor,
    /// Hears both legs, heard by this one alone.
    Whisper(MediaLeg),
    /// Hears and is heard by both legs.
    Barge,
}

impl Supervision {
    /// `monitor`, `whisper` (to the callee) or `barge`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Whisper(_) => "whisper",
            Self::Barge => "barge",
        }
    }
}

/// One leg of a two-party media session: A the caller, B the callee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaLeg {
    /// The caller's leg.
    A,
    /// The callee's leg.
    B,
}

impl MediaLeg {
    /// The other leg.
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// A digit one leg sent, once it has ended — RFC 2833 or in-band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegDigit {
    /// The leg the digit came from.
    pub leg: MediaLeg,
    /// `0`-`9`, `*`, `#` or `A`-`D`.
    pub digit: char,
    /// How long it was held, when the sender said.
    pub duration_ms: Option<u32>,
    /// Heard in the audio rather than sent as telephone-event — already
    /// in what the other leg hears.
    pub in_band: bool,
}

/// Audio injection target
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTarget {
    /// Inject audio to participant A (caller)
    ParticipantA,
    /// Inject audio to participant B (callee)
    ParticipantB,
    /// Inject audio to both participants
    Both,
}

/// Embedded backend - forge-media runs in-process
#[cfg(feature = "embedded")]
pub mod embedded {
    use super::*;
    use crate::MediaSessionStats;
    use dashmap::DashMap;
    use forge_core::{EventBus, ParticipantId};
    use forge_engine::{
        session::{DtmfConfig, ParticipantCodecConfig, TranscodingConfig},
        AudioTarget as ForgeAudioTarget, MediaSession as ForgeMediaSession, MixMode,
        PlaybackHandle, PlaybackManager, PlaybackOptions, SessionManager, SessionManagerConfig,
    };
    use forge_injection::FileSource;
    use forge_rtp::PortPoolConfig;
    use std::time::Duration;
    use tracing::{debug, error, info, warn};

    pub struct EmbeddedBackend {
        /// forge-media session manager
        session_manager: Arc<SessionManager>,

        /// Audio playback manager
        playback_manager: Arc<PlaybackManager>,

        /// Mapping of FCP CallId to forge-media session and participant IDs
        /// This lets us look up forge sessions using FCP call IDs
        sessions: DashMap<fcp_core::CallId, SessionInfo>,

        /// Active playback handles for waiting on completion
        playback_handles: DashMap<String, PlaybackHandle>,

        /// IP address to advertise in SDP (for RTP media)
        advertise_ip: String,

        /// TTS service for text-to-speech synthesis (None if not configured)
        tts_service: Option<Arc<crate::tts::TtsService>>,

        /// Forge event bus for subscribing to DTMF and other media events
        event_bus: Arc<EventBus>,

        /// The media bridge every listened-to session is given: forge
        /// hands it each leg's decoded audio.
        bridge: Arc<forge_engine::MediaBridgeManager>,

        /// Per forge session, the listeners its audio is fanned out to.
        listeners: Arc<DashMap<forge_core::CallId, Arc<Listeners>>>,
    }

    /// How many frames a listener may fall behind: a second.
    const LISTEN_QUEUE: usize = 50;

    /// Each listener: the leg it hears and where its frames go.
    type Subscribers = Vec<(MediaLeg, tokio::sync::mpsc::Sender<Vec<i16>>)>;

    /// The listeners of one session; `None` once its pump has stopped.
    struct Listeners {
        subscribers: std::sync::Mutex<Option<Subscribers>>,
    }

    impl Listeners {
        fn lock(&self) -> std::sync::MutexGuard<'_, Option<Subscribers>> {
            self.subscribers.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// Add a listener; false when the pump has stopped.
        fn add(&self, leg: MediaLeg, tx: tokio::sync::mpsc::Sender<Vec<i16>>) -> bool {
            match self.lock().as_mut() {
                Some(list) => {
                    list.push((leg, tx));
                    true
                }
                None => false,
            }
        }
    }

    /// A listened leg's 20 ms of silence, at 16 kHz.
    const SILENT_FRAME: usize = crate::stt::STT_SAMPLE_RATE as usize / 50;

    /// How long a leg that has been sending may go quiet before its gap is
    /// heard as silence. A phone suppressing silence sends nothing, and a
    /// listener timing the pause after an utterance must still hear one.
    const GAP_BEFORE_SILENCE: Duration = Duration::from_millis(60);

    /// Carry a session's audio from its bridge to its listeners, each leg
    /// resampled to 16 kHz, until the session ends or nobody listens. A
    /// leg that has started sending and then stops is heard as silence.
    async fn pump_listeners(
        mut handle: forge_engine::MediaBridgeHandle,
        fanout: Arc<Listeners>,
        listeners: Arc<DashMap<forge_core::CallId, Arc<Listeners>>>,
    ) {
        let mut resamplers: [Option<(u32, forge_resampler::Resampler)>; 2] = [None, None];
        // Per leg, until when its audio has been delivered.
        let mut heard_until: [Option<tokio::time::Instant>; 2] = [None, None];
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let frame = tokio::select! {
                frame = handle.recv_frame() => match frame {
                    Some(frame) => frame,
                    None => break,
                },
                now = tick.tick() => {
                    let mut gaps = Vec::new();
                    for (slot, leg) in [(0, MediaLeg::A), (1, MediaLeg::B)] {
                        let Some(until) = heard_until[slot].as_mut() else { continue };
                        while now.saturating_duration_since(*until) >= GAP_BEFORE_SILENCE {
                            *until += Duration::from_millis(20);
                            gaps.push(leg);
                        }
                    }
                    if !gaps.is_empty() && !deliver_silence(&fanout, &gaps) {
                        break;
                    }
                    continue;
                }
            };
            let (leg, slot) = match frame.leg {
                forge_engine::ParticipantLabel::A => (MediaLeg::A, 0),
                forge_engine::ParticipantLabel::B => (MediaLeg::B, 1),
            };
            let pcm = if frame.sample_rate == crate::stt::STT_SAMPLE_RATE {
                frame.samples
            } else {
                let resampler = match &mut resamplers[slot] {
                    Some((rate, r)) if *rate == frame.sample_rate => r,
                    other => {
                        match forge_resampler::Resampler::new(
                            frame.sample_rate,
                            crate::stt::STT_SAMPLE_RATE,
                            1,
                        ) {
                            Ok(r) => &mut other.insert((frame.sample_rate, r)).1,
                            Err(e) => {
                                warn!(rate = frame.sample_rate, error = %e, "cannot resample a listened leg");
                                continue;
                            }
                        }
                    }
                };
                match resampler.resample(&frame.samples) {
                    Ok(pcm) => pcm,
                    Err(_) => continue,
                }
            };
            let now = tokio::time::Instant::now();
            let duration = Duration::from_micros(
                pcm.len() as u64 * 1_000_000 / crate::stt::STT_SAMPLE_RATE as u64,
            );
            heard_until[slot] = Some(match heard_until[slot] {
                Some(until) if until > now => until + duration,
                _ => now + duration,
            });
            if !deliver(&fanout, leg, &pcm) {
                break;
            }
        }
        *fanout.lock() = None;
        listeners.remove_if(handle.call_id(), |_, v| Arc::ptr_eq(v, &fanout));
        // The handle detaches the bridge as it drops.
    }

    /// Hand `pcm` to `leg`'s listeners; false once nobody listens.
    fn deliver(fanout: &Listeners, leg: MediaLeg, pcm: &[i16]) -> bool {
        let mut subscribers = fanout.lock();
        let Some(list) = subscribers.as_mut() else {
            return false;
        };
        list.retain(|(_, tx)| !tx.is_closed());
        if list.is_empty() {
            return false;
        }
        for (_, tx) in list.iter().filter(|(l, _)| *l == leg) {
            let _ = tx.try_send(pcm.to_vec());
        }
        true
    }

    /// A frame of silence to each leg in `gaps`, once per entry.
    fn deliver_silence(fanout: &Listeners, gaps: &[MediaLeg]) -> bool {
        let silence = [0i16; SILENT_FRAME];
        gaps.iter().all(|leg| deliver(fanout, *leg, &silence))
    }

    /// Information about a forge-media session
    struct SessionInfo {
        /// forge-media session reference
        forge_session: Arc<ForgeMediaSession>,
        /// Forge CallId (converted from FCP CallId)
        forge_call_id: forge_core::CallId,
        /// Participant A ID
        _participant_a: ParticipantId,
        /// Participant B ID
        _participant_b: ParticipantId,
        /// FCP dialog ID
        dialog_id: fcp_core::DialogId,
        /// Cached FCP MediaSession wrapper (preserves SDP state across calls)
        fcp_session: Arc<MediaSession>,
        /// Per leg, the address its RTP is learned from rather than read
        /// off its SDP (a peer behind NAT).
        latched: std::sync::Mutex<[Option<IpAddr>; 2]>,
    }

    impl SessionInfo {
        fn latched(&self, leg: Leg) -> Option<IpAddr> {
            let held = self.latched.lock().unwrap_or_else(|e| e.into_inner());
            held[match leg {
                Leg::A => 0,
                Leg::B => 1,
            }]
        }
    }

    /// The engine's form of a supervision mode.
    fn forge_supervision(mode: Supervision) -> forge_engine::SupervisionMode {
        match mode {
            Supervision::Monitor => forge_engine::SupervisionMode::Monitor,
            Supervision::Whisper(MediaLeg::A) => {
                forge_engine::SupervisionMode::Whisper(forge_engine::CallParty::A)
            }
            Supervision::Whisper(MediaLeg::B) => {
                forge_engine::SupervisionMode::Whisper(forge_engine::CallParty::B)
            }
            Supervision::Barge => forge_engine::SupervisionMode::Barge,
        }
    }

    /// The SDP a supervisor's phone is offered, or answered: this node's
    /// port for them and the G.711 codecs given.
    fn party_sdp(advertise_ip: &str, port: u16, codecs: &[(u8, &str)]) -> String {
        let session = rand::random::<u32>();
        let family = if advertise_ip.contains(':') {
            "IP6"
        } else {
            "IP4"
        };
        let pts: Vec<String> = codecs.iter().map(|(pt, _)| pt.to_string()).collect();
        let mut sdp = format!(
            "v=0\r\no=fcp {session} {session} IN {family} {advertise_ip}\r\ns=supervision\r\n\
             c=IN {family} {advertise_ip}\r\nt=0 0\r\nm=audio {port} RTP/AVP {}\r\n",
            pts.join(" ")
        );
        for (pt, name) in codecs {
            sdp.push_str(&format!("a=rtpmap:{pt} {name}/8000\r\n"));
        }
        sdp.push_str("a=ptime:20\r\na=sendrecv\r\n");
        sdp
    }

    /// Parse the remote audio RTP endpoint from SDP: the first active audio
    /// section's port with its own `c=` line, or the session-level one.
    fn parse_remote_endpoint_from_sdp(
        sdp: &str,
    ) -> std::result::Result<std::net::SocketAddr, String> {
        crate::sdp::remote_audio_endpoint(sdp)
            .ok_or_else(|| "Failed to parse IP or port from SDP".to_string())
    }

    impl EmbeddedBackend {
        pub async fn new(config: &crate::config::MediaConfig) -> Result<Self> {
            info!("Initializing embedded media backend");

            // Create forge-media configuration
            let port_pool_config = PortPoolConfig {
                min_port: config.rtp_port_start,
                max_port: config.rtp_port_end,
            };

            let session_config = forge_engine::MediaSessionConfig {
                socket_config: forge_rtp::RtpSocketConfig::default(),
                session_timeout: Duration::from_secs(300), // 5 minutes
                dtmf_config: DtmfConfig {
                    enable_rfc2833: config.enable_dtmf,
                    enable_inband: config.enable_dtmf,
                    enable_dedup: true,
                    opus_payload_type: Some(111),
                },
                transcoding_config: TranscodingConfig {
                    enable_transcoding: config.enable_transcoding,
                    ..Default::default()
                },
                // vad_config, media_stats_interval and min_free_port_pairs were added in
                // forge-engine 0.5; their defaults preserve the previous behaviour.
                ..Default::default()
            };

            let manager_config = SessionManagerConfig {
                port_pool_config,
                session_config,
                cleanup_interval: Duration::from_secs(30),
            };

            // Create event bus for forge-media events
            let event_bus = Arc::new(EventBus::new());

            // Initialize SessionManager (returns Arc<Self>)
            let session_manager = SessionManager::new(manager_config, Some(event_bus.clone()));

            // Initialize PlaybackManager for audio injection (with session_manager reference)
            let playback_manager = Arc::new(PlaybackManager::new_with_session_manager(
                session_manager.clone(),
            ));

            // Determine the IP address to advertise in SDP
            let advertise_ip = if let Some(ref public_ip) = config.public_ip {
                info!("Using configured public IP for SDP: {}", public_ip);
                public_ip.clone()
            } else {
                warn!("No public_ip configured in media config - using 127.0.0.1 (this will NOT work for remote clients!)");
                warn!("Configure media.public_ip in fcp.toml to fix this");
                "127.0.0.1".to_string()
            };

            // Use shared TTS service if injected, otherwise create one from config
            let tts_service = if let Some(shared_tts) = config.tts_service.clone() {
                info!(provider = %shared_tts.provider_name(), "Using shared TTS service");
                Some(shared_tts)
            } else {
                match config.tts.clone() {
                    Some(tts_config) => match crate::tts::TtsService::new(tts_config).await {
                        Ok(svc) => {
                            info!(provider = %svc.provider_name(), "TTS service initialized");
                            Some(Arc::new(svc))
                        }
                        Err(e) => {
                            warn!(error = %e, "TTS service initialization failed, TTS will be unavailable");
                            None
                        }
                    },
                    None => None,
                }
            };

            // Warm up TTS worker eagerly so the first live synthesis doesn't
            // stall while loading the model (Kokoro cold-start can take 30-90s).
            if let Some(ref tts) = tts_service {
                info!("Warming up TTS worker (model load)…");
                let tts_clone = Arc::clone(tts);
                tokio::spawn(async move {
                    match tts_clone.synthesize("warmup", Some("en-US"), None).await {
                        Ok(_) => info!("TTS worker warm-up complete"),
                        Err(e) => warn!(error = %e, "TTS warm-up failed (non-fatal)"),
                    }
                });
            }

            info!(
                tts = tts_service.is_some(),
                "Embedded media backend initialized, advertise_ip: {}", advertise_ip
            );

            Ok(Self {
                session_manager,
                playback_manager,
                sessions: DashMap::new(),
                playback_handles: DashMap::new(),
                advertise_ip,
                tts_service,
                event_bus,
                bridge: Arc::new(forge_engine::MediaBridgeManager::new()),
                listeners: Arc::new(DashMap::new()),
            })
        }
    }

    fn forge_leg(leg: MediaLeg) -> forge_engine::ParticipantLabel {
        match leg {
            MediaLeg::A => forge_engine::ParticipantLabel::A,
            MediaLeg::B => forge_engine::ParticipantLabel::B,
        }
    }

    impl EmbeddedBackend {
        /// A file into the call: a prompt once, or hold music looped.
        async fn start_file_playback(
            &self,
            call_id: &fcp_core::CallId,
            file_path: String,
            target: AudioTarget,
            looped: bool,
        ) -> Result<String> {
            debug!(?call_id, path = %file_path, target = ?target, looped, "Starting audio playback");
            // Get session info to retrieve forge call_id
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let forge_call_id = session_info.forge_call_id.clone();
            drop(session_info); // Release lock

            // Create FileSource for the audio file
            let source = Box::new(
                FileSource::new(&file_path)
                    .map_err(|e| Error::ForgeMedia(format!("Failed to open audio file: {}", e)))?,
            );

            // Convert FCP AudioTarget to forge AudioTarget
            let forge_target = match target {
                AudioTarget::ParticipantA => ForgeAudioTarget::ParticipantA,
                AudioTarget::ParticipantB => ForgeAudioTarget::ParticipantB,
                AudioTarget::Both => ForgeAudioTarget::Both,
            };

            // Start playback
            let handle = self
                .playback_manager
                .start_playback_with(forge_call_id, source, {
                    let options = PlaybackOptions::new(forge_target, MixMode::Replace);
                    if looped {
                        options.looped()
                    } else {
                        options
                    }
                })
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start playback: {}", e)))?;

            let playback_id = handle.id().to_string();

            // Store handle so we can wait for completion later
            self.playback_handles.insert(playback_id.clone(), handle);

            info!(?call_id, playback_id = %playback_id, "Audio playback started");

            Ok(playback_id)
        }
    }

    #[async_trait]
    impl MediaBackend for EmbeddedBackend {
        async fn latch_leg(&self, call_id: &CallId, leg: Leg, source: IpAddr) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            {
                let mut held = session_info
                    .latched
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                held[match leg {
                    Leg::A => 0,
                    Leg::B => 1,
                }] = Some(source);
            }
            // Whatever an SDP already pinned is undone: the first packet
            // from `source` says where the peer really is.
            let participant = match leg {
                Leg::A => session_info.forge_session.participant_a(),
                Leg::B => session_info.forge_session.participant_b(),
            };
            let mut p = participant.write().await;
            p.remote_addr = None;
            p.latch_allowed_ips = Some(std::iter::once(source).collect());
            info!(?call_id, ?leg, %source, "Leg's RTP address will be learned from its first packet");
            Ok(())
        }

        async fn create_session(
            &self,
            call_id: fcp_core::CallId,
            dialog_id: fcp_core::DialogId,
            config: MediaSessionConfig,
        ) -> Result<Arc<MediaSession>> {
            debug!(?call_id, ?dialog_id, "Creating embedded media session");

            // Check if session already exists
            if self.sessions.contains_key(&call_id) {
                return Err(Error::InvalidState(format!(
                    "Session already exists for call {}",
                    call_id
                )));
            }

            // Convert FCP CallId to forge CallId
            let forge_call_id = adapters::fcp_to_forge_call_id(&call_id);

            // Create participant IDs for both legs
            let (participant_a, participant_b) = adapters::create_participant_ids(&call_id);

            // Determine codec configuration
            let codec_config = if let Some(first_codec) = config.preferred_codecs.first() {
                match first_codec.to_uppercase().as_str() {
                    "PCMU" => ParticipantCodecConfig {
                        payload_type: 0,
                        codec: forge_core::AudioCodec::PCMU,
                        clock_rate: 8000,
                    },
                    "PCMA" => ParticipantCodecConfig {
                        payload_type: 8,
                        codec: forge_core::AudioCodec::PCMA,
                        clock_rate: 8000,
                    },
                    "G722" => ParticipantCodecConfig {
                        payload_type: 9,
                        codec: forge_core::AudioCodec::G722,
                        clock_rate: 16000,
                    },
                    "OPUS" | "opus" => ParticipantCodecConfig {
                        payload_type: 111,
                        codec: forge_core::AudioCodec::Opus,
                        clock_rate: 48000,
                    },
                    _ => ParticipantCodecConfig::default(), // Default to PCMU
                }
            } else {
                ParticipantCodecConfig::default()
            };

            // Create forge-media session
            // For now, both participants use the same codec
            // TODO: Handle different codecs per participant
            let forge_session = self
                .session_manager
                .create_session_with_codecs(
                    forge_call_id.clone(),
                    participant_a.clone(),
                    participant_b.clone(),
                    codec_config.clone(),
                    codec_config,
                    None, // SDP will be set later
                    None, // from_tag will be set later
                    None, // to_tag will be set later
                    None, // Use default config
                )
                .await
                .map_err(|e| Error::ForgeMedia(e.to_string()))?;

            // Enable RFC 2833 relay if configured
            if config.enable_relay_rfc2833 {
                forge_session.set_relay_rfc2833(true);
                debug!(?call_id, "RFC 2833 DTMF relay enabled");
            }

            // Start RTP forwarding (required for media to flow)
            forge_session
                .start_forwarding()
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start forwarding: {}", e)))?;

            // Get RTP/RTCP ports from forge-media session
            let ports = forge_session.ports();

            // Create FCP MediaSession wrapper
            let fcp_session = Arc::new(MediaSession::new(
                call_id,
                dialog_id,
                forge_call_id.0.clone(), // Use forge CallId string as session ID
                config,
                ports.rtp_port,
                ports.rtcp_port,
            ));

            // Update state to active
            fcp_session
                .set_state(crate::session::MediaSessionState::Active)
                .await;

            // Store session info with cached FCP session
            let session_info = SessionInfo {
                forge_session: forge_session.clone(),
                forge_call_id: forge_call_id.clone(),
                _participant_a: participant_a.clone(),
                _participant_b: participant_b.clone(),
                dialog_id,
                fcp_session: fcp_session.clone(),
                latched: std::sync::Mutex::new([None, None]),
            };

            self.sessions.insert(call_id, session_info);

            info!(
                ?call_id,
                forge_call_id = %forge_call_id,
                ?participant_a,
                ?participant_b,
                "Embedded media session created"
            );

            Ok(fcp_session)
        }

        async fn get_session(&self, call_id: &fcp_core::CallId) -> Result<Arc<MediaSession>> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            Ok(session_info.fcp_session.clone())
        }

        async fn update_session(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<String> {
            debug!(?call_id, "Updating session with remote SDP");

            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Generate negotiated SDP (intersects remote codecs with our supported set)
            let result = generate_negotiated_sdp(
                &session_info,
                &remote_sdp,
                &self.advertise_ip,
                false, // enable_srtp: controlled by caller / trunk config
            )?;

            // Parse remote SDP to extract client's RTP endpoint and set codec config
            if let Ok(remote_endpoint) = parse_remote_endpoint_from_sdp(&remote_sdp) {
                let mut pa = session_info.forge_session.participant_a().write().await;
                match session_info.latched(Leg::A) {
                    // The SDP's address is not where the packets will come
                    // from: wait for the first one, from the peer's own.
                    Some(source) => {
                        pa.remote_addr = None;
                        pa.latch_allowed_ips = Some(std::iter::once(source).collect());
                    }
                    None => pa.remote_addr = Some(remote_endpoint),
                }
                pa.codec_config = sdp_codec_to_forge_config(&result.primary_codec);
                pa.payload_type = result.primary_codec.payload_type;
                info!(
                    ?call_id,
                    endpoint = %remote_endpoint,
                    codec = %result.primary_codec.encoding_name,
                    "Set A-leg endpoint and codec from SDP"
                );
            } else {
                warn!(?call_id, "Failed to parse remote endpoint from SDP - will rely on symmetric RTP learning");
            }

            // Keep the payload-type map in step with the A-leg's negotiated
            // codec (also covers a re-INVITE changing the codec mid-call).
            sync_transcoding_state(call_id, &session_info.forge_session).await;

            // Extract telephone-event PT from A-leg's SDP and store on forge session
            let te_pt = crate::sdp::extract_telephone_event_pt(&remote_sdp).unwrap_or_else(|| {
                // Default to PT 101 — virtually all phones use this
                debug!(
                    ?call_id,
                    "No telephone-event PT in A-leg SDP, defaulting to 101"
                );
                101
            });
            session_info.forge_session.set_telephone_event_pt_a(te_pt);
            debug!(?call_id, te_pt, "A-leg telephone-event PT set");

            // Wire SRTP keys for A-leg if SRTP was negotiated
            if let (Some(ref local_material), Some(ref remote_crypto)) =
                (&result.local_crypto, &result.remote_crypto)
            {
                match crate::sdp::decode_sdes_key_params(
                    &remote_crypto.key_params,
                    &remote_crypto.suite,
                ) {
                    Ok((remote_key, remote_salt, profile)) => {
                        match forge_rtp::srtp::SrtpKeyMaterial::new(
                            remote_key,
                            remote_salt,
                            profile,
                        ) {
                            Ok(remote_material) => {
                                let mut srtp_a = session_info.forge_session.srtp_a().lock().await;
                                srtp_a.set_local_key(local_material.clone());
                                srtp_a.set_remote_key(remote_material);
                                info!(?call_id, "SRTP keys configured for A-leg");
                            }
                            Err(e) => {
                                warn!(?call_id, error = %e, "Failed to create A-leg remote SRTP key material");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(?call_id, error = %e, "Failed to decode A-leg SDES key params");
                    }
                }
            } else if result.remote_crypto.is_none() {
                // The A side is now plain — a transfer put a plain phone
                // where an encrypted one was: the old keys must not go on
                // encrypting toward it.
                let mut srtp_a = session_info.forge_session.srtp_a().lock().await;
                if srtp_a.is_enabled() {
                    *srtp_a = forge_rtp::srtp::SrtpContext::new();
                    info!(?call_id, "A-leg is plain now; its SRTP keys are cleared");
                }
            }

            // Update cached session state
            session_info.fcp_session.set_remote_sdp(remote_sdp).await;
            session_info
                .fcp_session
                .set_local_sdp(result.sdp.clone())
                .await;

            Ok(result.sdp)
        }

        async fn update_session_b_leg(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<()> {
            self.update_session_b_leg_offered(call_id, remote_sdp, None)
                .await
        }

        async fn update_session_b_leg_offered(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
            offer: Option<crate::sdp::SrtpOffer>,
        ) -> Result<()> {
            debug!(?call_id, "Setting B-leg remote endpoint from SDP");

            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let b_leg_codecs = crate::sdp::parse_sdp_codecs(&remote_sdp);

            if let Ok(remote_endpoint) = parse_remote_endpoint_from_sdp(&remote_sdp) {
                let mut pb = session_info.forge_session.participant_b().write().await;
                match session_info.latched(Leg::B) {
                    Some(source) => {
                        pb.remote_addr = None;
                        pb.latch_allowed_ips = Some(std::iter::once(source).collect());
                    }
                    None => pb.remote_addr = Some(remote_endpoint),
                }
                if let Some(first) = b_leg_codecs.first() {
                    pb.codec_config = sdp_codec_to_forge_config(first);
                    pb.payload_type = first.payload_type;
                    info!(
                        ?call_id,
                        endpoint = %remote_endpoint,
                        codec = %first.encoding_name,
                        "Set B-leg endpoint and codec from SDP"
                    );

                    // Both legs' codecs are now known: rebuild the payload-type
                    // map and arm transcoders if the legs disagree.
                    drop(pb); // Release write lock before the helper reads both participants
                    sync_transcoding_state(call_id, &session_info.forge_session).await;
                } else {
                    info!(
                        ?call_id,
                        endpoint = %remote_endpoint,
                        "Set B-leg endpoint from SDP (no codecs parsed)"
                    );
                }
            } else {
                warn!(?call_id, "Failed to parse B-leg remote endpoint from SDP");
            }

            // Extract telephone-event PT from B-leg's SDP and store on forge session
            if let Some(te_pt) = crate::sdp::extract_telephone_event_pt(&remote_sdp) {
                session_info.forge_session.set_telephone_event_pt_b(te_pt);
                debug!(?call_id, te_pt, "B-leg telephone-event PT set from SDP");
            }

            // Wire SRTP keys for the B-leg from the answer's `a=crypto`.
            //
            // The key this node encrypts with is the one it advertised in
            // its own offer, not a fresh one: in SDES each side states the
            // key it sends with (RFC 4568 §5.1.2), so a new key here would
            // be one the far end never saw. Without an offer of our own —
            // a peer that sent crypto unasked — a fresh key is the best
            // that can be done, and is what this did before.
            let b_leg_crypto = crate::sdp::parse_sdes_crypto(&remote_sdp);
            if let Some(first_crypto) = b_leg_crypto.into_iter().next() {
                match offer {
                    Some(offer) => match offer.accept(&first_crypto) {
                        Ok(context) => {
                            let mut srtp_b = session_info.forge_session.srtp_b().lock().await;
                            *srtp_b = context;
                            info!(
                                ?call_id,
                                "SRTP keys configured for B-leg from our own offer"
                            );
                        }
                        Err(e) => {
                            warn!(?call_id, error = %e, "The B-leg answered SRTP on terms we cannot use")
                        }
                    },
                    None => match crate::sdp::decode_sdes_key_params(
                        &first_crypto.key_params,
                        &first_crypto.suite,
                    ) {
                        Ok((remote_key, remote_salt, profile)) => {
                            match forge_rtp::srtp::SrtpKeyMaterial::new(
                                remote_key,
                                remote_salt,
                                profile,
                            ) {
                                Ok(remote_material) => {
                                    // Generate local key for B-leg
                                    match crate::sdp::generate_srtp_key_material() {
                                        Ok((local_material, _)) => {
                                            let mut srtp_b =
                                                session_info.forge_session.srtp_b().lock().await;
                                            srtp_b.set_local_key(local_material);
                                            srtp_b.set_remote_key(remote_material);
                                            info!(?call_id, "SRTP keys configured for B-leg");
                                        }
                                        Err(e) => {
                                            warn!(?call_id, error = %e, "Failed to generate B-leg local SRTP key material");
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(?call_id, error = %e, "Failed to create B-leg remote SRTP key material");
                                }
                            }
                        }
                        Err(e) => {
                            warn!(?call_id, error = %e, "Failed to decode B-leg SDES key params");
                        }
                    },
                }
            } else {
                // The B side is now plain (see the A side's note).
                let mut srtp_b = session_info.forge_session.srtp_b().lock().await;
                if srtp_b.is_enabled() {
                    *srtp_b = forge_rtp::srtp::SrtpContext::new();
                    info!(?call_id, "B-leg is plain now; its SRTP keys are cleared");
                }
            }

            Ok(())
        }

        async fn hold_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Putting session on hold");

            let _session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // forge-media doesn't have explicit hold/resume
            // Hold is typically done via SDP re-negotiation (sendonly/inactive)
            // For now, this is a no-op

            info!(?call_id, "Session on hold");
            Ok(())
        }

        async fn resume_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Resuming session from hold");

            let _session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Resume via SDP re-negotiation
            // For now, this is a no-op

            info!(?call_id, "Session resumed");
            Ok(())
        }

        async fn terminate_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Terminating media session");

            if let Some((_, session_info)) = self.sessions.remove(call_id) {
                // Stop all active playbacks for this session (closes file handles)
                let forge_call_id = &session_info.forge_call_id;
                if let Err(e) = self
                    .playback_manager
                    .stop_all_playbacks(forge_call_id)
                    .await
                {
                    warn!(?call_id, error = %e, "Failed to stop playbacks during session termination");
                }
                // Remove any playback handles referencing this session
                self.playback_handles.retain(|_id, _handle| true); // handles are dropped when session stops

                // Stop recording if active (must do this before terminating session)
                if session_info.forge_session.is_recording().await {
                    if let Err(e) = session_info.forge_session.disable_recording().await {
                        warn!(?call_id, error = %e, "Failed to stop recording during session termination");
                    }
                }

                // Listeners hear the session end.
                self.bridge.detach_call(forge_call_id);

                // Stop forwarding (terminate session)
                session_info
                    .forge_session
                    .stop_forwarding()
                    .await
                    .map_err(|e| Error::ForgeMedia(e.to_string()))?;

                info!(?call_id, "Media session terminated");
            } else {
                warn!(?call_id, "Attempted to terminate non-existent session");
            }

            Ok(())
        }

        async fn get_stats(&self, call_id: &fcp_core::CallId) -> Result<MediaSessionStats> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Get stats from forge-media session
            let stats_a = session_info.forge_session.participant_a_stats().await;
            let stats_b = session_info.forge_session.participant_b_stats().await;

            // Aggregate stats
            Ok(MediaSessionStats {
                packets_sent: stats_a.packets_sent + stats_b.packets_sent,
                packets_received: stats_a.packets_received + stats_b.packets_received,
                bytes_sent: stats_a.bytes_sent + stats_b.bytes_sent,
                bytes_received: stats_a.bytes_received + stats_b.bytes_received,
                packets_lost: stats_a.packets_lost + stats_b.packets_lost,
                jitter_ms: 0.0, // TODO: Calculate from forge stats
                rtt_ms: None,
                current_codec: Some("PCMU".to_string()), // TODO: Get from session
                start_time: None,
                duration_secs: 0,
            })
        }

        async fn send_dtmf(
            &self,
            call_id: &fcp_core::CallId,
            leg: MediaLeg,
            digit: char,
            duration_ms: u32,
        ) -> Result<()> {
            let forge_session = self
                .sessions
                .get(call_id)
                .map(|s| Arc::clone(&s.forge_session))
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            let digit = forge_engine::DtmfDigit::from_char(digit)
                .map_err(|e| Error::InvalidState(e.to_string()))?;
            debug!(?call_id, ?leg, %digit, duration_ms, "Sending DTMF");
            forge_session
                .send_dtmf(forge_leg(leg), digit, duration_ms)
                .await
                .map_err(|e| Error::ForgeMedia(e.to_string()))
        }

        fn set_rfc2833_relay_to(
            &self,
            call_id: &fcp_core::CallId,
            leg: MediaLeg,
            relay: bool,
        ) -> Result<()> {
            let session = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            session
                .forge_session
                .set_rfc2833_relay_to(forge_leg(leg), relay);
            Ok(())
        }

        async fn listen(
            &self,
            call_id: &fcp_core::CallId,
            leg: MediaLeg,
        ) -> Result<tokio::sync::mpsc::Receiver<Vec<i16>>> {
            let (forge_session, forge_call_id) = {
                let info = self
                    .sessions
                    .get(call_id)
                    .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
                (Arc::clone(&info.forge_session), info.forge_call_id.clone())
            };
            forge_session
                .set_media_bridge_manager(Arc::clone(&self.bridge))
                .await;
            let (tx, rx) = tokio::sync::mpsc::channel(LISTEN_QUEUE);
            // A pump that has just stopped may not yet have let the bridge
            // go: try again for a moment.
            for _ in 0..50 {
                match self.listeners.entry(forge_call_id.clone()) {
                    dashmap::mapref::entry::Entry::Occupied(entry) => {
                        if entry.get().add(leg, tx.clone()) {
                            return Ok(rx);
                        }
                        entry.remove();
                    }
                    dashmap::mapref::entry::Entry::Vacant(slot) => {
                        match self.bridge.attach_call(forge_call_id.clone()) {
                            Ok(handle) => {
                                let fanout = Arc::new(Listeners {
                                    subscribers: std::sync::Mutex::new(Some(vec![(leg, tx)])),
                                });
                                slot.insert(Arc::clone(&fanout));
                                tokio::spawn(pump_listeners(
                                    handle,
                                    fanout,
                                    Arc::clone(&self.listeners),
                                ));
                                debug!(?call_id, ?leg, "listening to the call");
                                return Ok(rx);
                            }
                            Err(_) => drop(slot),
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(Error::Backend(format!(
                "the media bridge of {call_id} did not come free"
            )))
        }

        fn subscribe_digits(
            &self,
            call_id: &fcp_core::CallId,
        ) -> Option<tokio::sync::mpsc::UnboundedReceiver<LegDigit>> {
            let mut digits = self.sessions.get(call_id)?.forge_session.subscribe_digits();
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            // Ends when the session does (the sender goes with it) or when
            // the subscriber stops listening.
            tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match digits.recv().await {
                        Ok(d) => {
                            let Some(digit) = d.digit.to_string().chars().next() else {
                                continue;
                            };
                            let digit = LegDigit {
                                leg: match d.leg {
                                    forge_engine::ParticipantLabel::A => MediaLeg::A,
                                    forge_engine::ParticipantLabel::B => MediaLeg::B,
                                },
                                digit,
                                duration_ms: d.duration_ms,
                                in_band: d.method == forge_engine::DtmfMethod::Inband,
                            };
                            if tx.send(digit).is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            warn!(lagged = n, "Digit subscriber lagged; digits were lost");
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            Some(rx)
        }

        fn create_dtmf_collector(
            &self,
            call_id: &fcp_core::CallId,
            max_digits: usize,
            finish_on_key: Option<char>,
        ) -> crate::dtmf_collector::DtmfCollector {
            let collector = crate::dtmf_collector::DtmfCollector::new(max_digits, finish_on_key);
            let sender = collector.get_sender();

            // Look up the forge CallId for this FCP call
            let forge_call_id = self.sessions.get(call_id).map(|s| s.forge_call_id.clone());

            if let Some(forge_call_id) = forge_call_id {
                info!(
                    fcp_call_id = %call_id,
                    forge_call_id = %forge_call_id,
                    max_digits = max_digits,
                    finish_on_key = ?finish_on_key,
                    active_sessions = self.sessions.len(),
                    "DTMF collector wired to forge EventBus"
                );
                let mut rx = self.event_bus.subscribe();
                // Spawn a task that filters forge DTMF events and forwards to the collector
                tokio::spawn(async move {
                    use forge_core::{DtmfEventKind, ForgeEvent};
                    loop {
                        match rx.recv().await {
                            Ok(ForgeEvent::DtmfDigitDetected {
                                call_id: ref event_cid,
                                digit,
                                event_type,
                                ..
                            }) if *event_cid == forge_call_id
                                && event_type == DtmfEventKind::End =>
                            {
                                debug!(
                                    digit = %digit,
                                    call_id = %forge_call_id,
                                    "Forwarding DTMF to collector"
                                );
                                if sender.send(digit).is_err() {
                                    // Collector dropped — stop listening
                                    break;
                                }
                            }
                            Ok(_) => {} // ignore other events
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                warn!(lagged = n, "DTMF event receiver lagged");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            } else {
                let session_keys: Vec<String> =
                    self.sessions.iter().map(|e| e.key().to_string()).collect();
                warn!(
                    call_id = %call_id,
                    active_sessions = self.sessions.len(),
                    session_keys = ?session_keys,
                    "No forge session found for DTMF collector — DTMF will not work"
                );
            }

            collector
        }

        async fn start_recording(&self, call_id: &fcp_core::CallId, path: String) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Determine format from file extension and codec
            // For G.711 codecs (PCMU/PCMA), use 8kHz sample rate
            // For other codecs, use 48kHz
            let format = if path.ends_with(".opus") {
                forge_core::AudioFormat::opus_mono()
            } else {
                // Default to WAV (PCM) at 8kHz for G.711 compatibility
                // Most VoIP codecs operate at 8kHz (G.711, G.729)
                forge_core::AudioFormat::new(8000, 1, forge_core::AudioCodec::PCM)
            };

            debug!(?call_id, path = %path, format = ?format, "Starting recording");

            // Enable recording on forge-media session
            session_info
                .forge_session
                .enable_recording(&path, format)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start recording: {}", e)))?;

            info!(?call_id, path = %path, "Recording started");

            Ok(())
        }

        async fn stop_recording(&self, call_id: &fcp_core::CallId) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            debug!(?call_id, "Stopping recording");

            // Disable recording on forge-media session
            session_info
                .forge_session
                .disable_recording()
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to stop recording: {}", e)))?;

            info!(?call_id, "Recording stopped");

            Ok(())
        }

        async fn play_file(
            &self,
            call_id: &fcp_core::CallId,
            file_path: String,
            target: AudioTarget,
        ) -> Result<String> {
            self.start_file_playback(call_id, file_path, target, false)
                .await
        }

        async fn play_file_looped(
            &self,
            call_id: &fcp_core::CallId,
            file_path: String,
            target: AudioTarget,
        ) -> Result<String> {
            self.start_file_playback(call_id, file_path, target, true)
                .await
        }

        async fn add_party(&self, call_id: &fcp_core::CallId, mode: Supervision) -> Result<String> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            let forge_session = Arc::clone(&session_info.forge_session);
            drop(session_info);
            let ports = forge_session
                .add_party(forge_supervision(mode))
                .await
                .map_err(|e| Error::Backend(format!("add a supervisor: {e}")))?;
            info!(
                ?call_id,
                port = ports.rtp_port,
                mode = mode.as_str(),
                "Supervisor media added"
            );
            Ok(party_sdp(
                &self.advertise_ip,
                ports.rtp_port,
                &[(0, "PCMU"), (8, "PCMA")],
            ))
        }

        async fn connect_party(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<String> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            let forge_session = Arc::clone(&session_info.forge_session);
            drop(session_info);
            let remote = parse_remote_endpoint_from_sdp(&remote_sdp).map_err(Error::Backend)?;
            let (codec, payload_type, name) = crate::sdp::parse_sdp_codecs(&remote_sdp)
                .into_iter()
                .find_map(|c| match c.encoding_name.to_ascii_uppercase().as_str() {
                    "PCMU" => Some((forge_core::AudioCodec::PCMU, c.payload_type, "PCMU")),
                    "PCMA" => Some((forge_core::AudioCodec::PCMA, c.payload_type, "PCMA")),
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Backend("the supervisor's phone offers neither PCMU nor PCMA".into())
                })?;
            forge_session
                .connect_party(remote, codec, payload_type)
                .map_err(|e| Error::Backend(format!("connect the supervisor: {e}")))?;
            let port = forge_session
                .party_ports()
                .map(|p| p.rtp_port)
                .ok_or_else(|| Error::Backend("the supervisor was removed".into()))?;
            Ok(party_sdp(&self.advertise_ip, port, &[(payload_type, name)]))
        }

        async fn set_party_mode(
            &self,
            call_id: &fcp_core::CallId,
            mode: Supervision,
        ) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;
            session_info
                .forge_session
                .set_party_mode(forge_supervision(mode))
                .map_err(|e| Error::Backend(format!("supervisor mode: {e}")))
        }

        async fn remove_party(&self, call_id: &fcp_core::CallId) -> Result<()> {
            let Some(session_info) = self.sessions.get(call_id) else {
                return Ok(());
            };
            let forge_session = Arc::clone(&session_info.forge_session);
            drop(session_info);
            forge_session
                .remove_party()
                .await
                .map_err(|e| Error::Backend(format!("remove the supervisor: {e}")))
        }

        async fn play_tone(
            &self,
            call_id: &fcp_core::CallId,
            frequency: f32,
            duration_ms: u32,
            amplitude: f32,
            target: AudioTarget,
        ) -> Result<String> {
            debug!(
                ?call_id,
                frequency,
                duration_ms,
                amplitude,
                ?target,
                "Starting tone playback"
            );

            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let forge_call_id = session_info.forge_call_id.clone();
            drop(session_info);

            let source = Box::new(
                forge_injection::ToneGenerator::single_tone(frequency, 8000)
                    .with_duration(duration_ms)
                    .with_amplitude(amplitude),
            );

            let forge_target = match target {
                AudioTarget::ParticipantA => ForgeAudioTarget::ParticipantA,
                AudioTarget::ParticipantB => ForgeAudioTarget::ParticipantB,
                AudioTarget::Both => ForgeAudioTarget::Both,
            };

            let handle = self
                .playback_manager
                .start_playback(forge_call_id, source, forge_target, MixMode::Replace)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start tone playback: {}", e)))?;

            let playback_id = handle.id().to_string();
            self.playback_handles.insert(playback_id.clone(), handle);

            info!(?call_id, playback_id = %playback_id, frequency, duration_ms, "Tone playback started");

            Ok(playback_id)
        }

        async fn play_tts(
            &self,
            call_id: &fcp_core::CallId,
            text: String,
            language: String,
            voice: Option<String>,
            target: AudioTarget,
        ) -> Result<String> {
            debug!(
                ?call_id,
                text = %text,
                language = %language,
                voice = ?voice,
                target = ?target,
                "Starting TTS playback"
            );

            let tts = self.tts_service.as_ref().ok_or_else(|| {
                Error::InvalidState(
                    "TTS not configured. Set [media.tts] in config or use play_file() with pre-recorded audio."
                        .to_string(),
                )
            })?;

            // Synthesize text to WAV file (cached by TtsService)
            let audio_path = tts
                .synthesize(&text, Some(&language), voice.as_deref())
                .await
                .map_err(|e| Error::InvalidState(format!("TTS synthesis failed: {}", e)))?;

            // Play the generated file using existing playback infrastructure
            self.play_file(call_id, audio_path.to_string_lossy().to_string(), target)
                .await
        }

        async fn stop_playback(
            &self,
            call_id: &fcp_core::CallId,
            playback_id: String,
        ) -> Result<()> {
            debug!(?call_id, playback_id = %playback_id, "Stopping playback");

            // Parse playback_id (format: "playback-{u64}")
            // Validate format but we'll use stop_all_playbacks for now
            let _id_num = playback_id
                .strip_prefix("playback-")
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| {
                    Error::InvalidState(format!("Invalid playback ID format: {}", playback_id))
                })?;

            // Create PlaybackId from number
            // Note: This is a workaround since PlaybackId::new() generates a new ID
            // We need to reconstruct the PlaybackId from the string representation
            // TODO: Add PlaybackId::from_u64() method to forge-engine
            // For now, we'll use stop_all_playbacks as a workaround

            // The playback's own handle, when this backend started it:
            // only that playback stops (hold music, not a prompt beside it).
            if let Some((_, handle)) = self.playback_handles.remove(&playback_id) {
                handle
                    .stop()
                    .await
                    .map_err(|e| Error::ForgeMedia(format!("Failed to stop playback: {}", e)))?;
                info!(?call_id, playback_id = %playback_id, "Playback stopped");
                return Ok(());
            }

            // Get session info to retrieve forge call_id
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let forge_call_id = session_info.forge_call_id.clone();
            drop(session_info);

            // Stop all playbacks for this call
            self.playback_manager
                .stop_all_playbacks(&forge_call_id)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to stop playback: {}", e)))?;

            info!(?call_id, "Playback stopped");

            Ok(())
        }

        async fn wait_playback_completion(&self, playback_id: String) -> Result<()> {
            debug!(playback_id = %playback_id, "Waiting for playback completion");

            // Get the handle from storage
            let handle = self
                .playback_handles
                .remove(&playback_id)
                .ok_or_else(|| {
                    Error::InvalidState(format!(
                        "Playback ID not found or already completed: {}",
                        playback_id
                    ))
                })?
                .1; // DashMap::remove returns (key, value) tuple

            // Wait for completion
            match handle.wait_completion().await {
                Ok(status) => {
                    use forge_engine::PlaybackStatus;
                    match status {
                        PlaybackStatus::Completed => {
                            info!(playback_id = %playback_id, "Playback completed successfully");
                            Ok(())
                        }
                        PlaybackStatus::Stopped => {
                            info!(playback_id = %playback_id, "Playback stopped by request");
                            Ok(())
                        }
                        PlaybackStatus::Failed(reason) => {
                            warn!(playback_id = %playback_id, reason = %reason, "Playback failed");
                            Err(Error::ForgeMedia(format!("Playback failed: {}", reason)))
                        }
                    }
                }
                Err(e) => {
                    warn!(playback_id = %playback_id, error = %e, "Failed to wait for playback completion");
                    Err(Error::ForgeMedia(format!(
                        "Failed to wait for playback: {}",
                        e
                    )))
                }
            }
        }

        async fn bridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()> {
            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Bridging calls for B2BUA operation"
            );

            let a_session = self
                .sessions
                .get(a_leg_call_id)
                .ok_or_else(|| Error::SessionNotFound(a_leg_call_id.to_string()))?;

            // Single-session mode: both call IDs refer to the same media session
            // (used by handler.rs B2BUA where one session handles both legs)
            if a_leg_call_id == b_leg_call_id {
                let a_addr = a_session
                    .forge_session
                    .participant_a()
                    .read()
                    .await
                    .remote_addr;
                let b_addr = a_session
                    .forge_session
                    .participant_b()
                    .read()
                    .await
                    .remote_addr;
                info!(
                    a_leg = %a_leg_call_id,
                    a_endpoint = ?a_addr,
                    b_endpoint = ?b_addr,
                    "Single-session bridge verified"
                );
                return Ok(());
            }

            // Two-session mode: cross-wire endpoints between separate sessions
            // A-leg session's participant_b → B-leg phone address
            // B-leg session's participant_b → A-leg phone address
            let b_session = self
                .sessions
                .get(b_leg_call_id)
                .ok_or_else(|| Error::SessionNotFound(b_leg_call_id.to_string()))?;

            // Get B-leg phone's address from B-leg session's participant_a
            let b_phone_addr = b_session
                .forge_session
                .participant_a()
                .read()
                .await
                .remote_addr;

            // Get A-leg phone's address from A-leg session's participant_a
            let a_phone_addr = a_session
                .forge_session
                .participant_a()
                .read()
                .await
                .remote_addr;

            // Cross-wire: A-leg session forwards to B-leg phone
            if let Some(addr) = b_phone_addr {
                a_session
                    .forge_session
                    .participant_b()
                    .write()
                    .await
                    .remote_addr = Some(addr);
            }

            // Cross-wire: B-leg session forwards to A-leg phone
            if let Some(addr) = a_phone_addr {
                b_session
                    .forge_session
                    .participant_b()
                    .write()
                    .await
                    .remote_addr = Some(addr);
            }

            // Cross-copy codecs so each session can transcode toward the far
            // leg: forwarding compares its own participants' payload types, so
            // each session's participant_b must describe the *other* phone's
            // negotiated codec, not this session's placeholder.
            let (a_phone_codec, a_phone_pt) = {
                let p = a_session.forge_session.participant_a().read().await;
                (p.codec_config.clone(), p.payload_type)
            };
            let (b_phone_codec, b_phone_pt) = {
                let p = b_session.forge_session.participant_a().read().await;
                (p.codec_config.clone(), p.payload_type)
            };
            {
                let mut pb = a_session.forge_session.participant_b().write().await;
                pb.codec_config = b_phone_codec;
                pb.payload_type = b_phone_pt;
            }
            {
                let mut pb = b_session.forge_session.participant_b().write().await;
                pb.codec_config = a_phone_codec;
                pb.payload_type = a_phone_pt;
            }
            sync_transcoding_state(a_leg_call_id, &a_session.forge_session).await;
            sync_transcoding_state(b_leg_call_id, &b_session.forge_session).await;

            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                a_endpoint = ?a_phone_addr,
                b_endpoint = ?b_phone_addr,
                "Two-session bridge established"
            );

            Ok(())
        }

        async fn unbridge_calls(
            &self,
            a_leg_call_id: &CallId,
            b_leg_call_id: &CallId,
        ) -> Result<()> {
            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Unbridging calls"
            );

            // Single-session mode: nothing to cross-unwire
            if a_leg_call_id == b_leg_call_id {
                return Ok(());
            }

            // Clear cross-wired participant_b on both sessions
            if let Some(a_session) = self.sessions.get(a_leg_call_id) {
                a_session
                    .forge_session
                    .participant_b()
                    .write()
                    .await
                    .remote_addr = None;
            }
            if let Some(b_session) = self.sessions.get(b_leg_call_id) {
                b_session
                    .forge_session
                    .participant_b()
                    .write()
                    .await
                    .remote_addr = None;
            }

            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Call bridge removed"
            );

            Ok(())
        }

        fn port_pool(&self) -> Option<Arc<forge_rtp::PortPool>> {
            Some(self.session_manager.port_pool())
        }

        fn backend_name(&self) -> &'static str {
            "embedded"
        }

        async fn health_check(&self) -> Result<bool> {
            // Check if SessionManager is healthy
            // For now, just check if we can access it
            Ok(true)
        }

        async fn list_sessions(&self) -> Vec<RtpSessionInfo> {
            let mut sessions = Vec::new();

            for entry in self.sessions.iter() {
                let call_id = entry.key();
                let info = entry.value();

                // Get participant info from forge session
                let pa = info.forge_session.participant_a().read().await;
                let pb = info.forge_session.participant_b().read().await;

                // Determine if transcoding is active (different codecs)
                let a_codec = format!("{:?}", pa.codec_config.codec);
                let b_codec = format!("{:?}", pb.codec_config.codec);
                let transcoding_active = pa.codec_config.codec != pb.codec_config.codec
                    || pa.codec_config.clock_rate != pb.codec_config.clock_rate;

                // Get session state
                let state = info.forge_session.state().await;

                // Get port info
                let ports = info.forge_session.ports();

                // Calculate duration from uptime
                let duration_secs = info.forge_session.uptime().as_secs();

                sessions.push(RtpSessionInfo {
                    call_id: call_id.to_string(),
                    dialog_id: info.dialog_id.to_string(),
                    state: format!("{:?}", state),
                    a_leg: RtpLegInfo {
                        remote_addr: pa.remote_addr.map(|a| a.to_string()),
                        codec: a_codec,
                        clock_rate: pa.codec_config.clock_rate,
                        payload_type: pa.codec_config.payload_type,
                        packets_received: pa.stats.packets_received,
                        packets_sent: pa.stats.packets_sent,
                        bytes_received: pa.stats.bytes_received,
                        bytes_sent: pa.stats.bytes_sent,
                    },
                    b_leg: RtpLegInfo {
                        remote_addr: pb.remote_addr.map(|a| a.to_string()),
                        codec: b_codec,
                        clock_rate: pb.codec_config.clock_rate,
                        payload_type: pb.codec_config.payload_type,
                        packets_received: pb.stats.packets_received,
                        packets_sent: pb.stats.packets_sent,
                        bytes_received: pb.stats.bytes_received,
                        bytes_sent: pb.stats.bytes_sent,
                    },
                    transcoding_active,
                    local_rtp_port: ports.rtp_port,
                    duration_secs,
                });
            }

            sessions
        }
    }

    /// Result of SDP negotiation: the generated SDP string and the primary codec selected.
    struct SdpNegotiationResult {
        sdp: String,
        primary_codec: crate::sdp::SdpCodecInfo,
        /// Our local SRTP key material (if SRTP is active)
        local_crypto: Option<forge_rtp::srtp::SrtpKeyMaterial>,
        /// Remote SDES crypto line (if remote offered crypto)
        remote_crypto: Option<crate::sdp::SdesCryptoLine>,
    }

    /// Generate a negotiated SDP answer based on the remote offer.
    ///
    /// Intersects remote codecs with locally supported codecs (preserving remote
    /// preference order), always appends telephone-event PT 101 for DTMF, and
    /// falls back to PCMU+PCMA if no common codecs are found.
    ///
    /// When `enable_srtp` is true (or the remote SDP contains `a=crypto:` lines),
    /// the generated SDP will use `RTP/SAVP` protocol and include an `a=crypto:`
    /// attribute with locally generated SRTP key material.
    fn generate_negotiated_sdp(
        session_info: &SessionInfo,
        remote_sdp: &str,
        advertise_ip: &str,
        enable_srtp: bool,
    ) -> Result<SdpNegotiationResult> {
        let ports = session_info.forge_session.ports();
        let remote_codecs = crate::sdp::parse_sdp_codecs(remote_sdp);
        let supported = crate::codec::supported_codec_registry();

        // Intersect: keep remote preference order, filter to supported
        let mut negotiated: Vec<crate::sdp::SdpCodecInfo> = Vec::new();
        for rc in &remote_codecs {
            let canonical = crate::codec::canonical_codec_name(&rc.encoding_name);
            if supported.contains_key(&canonical) {
                negotiated.push(rc.clone());
            }
        }

        // Fallback if no common codecs
        if negotiated.is_empty() {
            negotiated.push(crate::sdp::SdpCodecInfo {
                payload_type: 0,
                encoding_name: "PCMU".into(),
                clock_rate: 8000,
            });
            negotiated.push(crate::sdp::SdpCodecInfo {
                payload_type: 8,
                encoding_name: "PCMA".into(),
                clock_rate: 8000,
            });
        }

        let primary = negotiated[0].clone();

        // SRTP negotiation: parse remote crypto lines
        let remote_crypto_lines = crate::sdp::parse_sdes_crypto(remote_sdp);
        let use_srtp = enable_srtp || !remote_crypto_lines.is_empty();
        let (local_crypto, remote_crypto) = if use_srtp {
            let (material, b64) = crate::sdp::generate_srtp_key_material()?;
            let remote = remote_crypto_lines.into_iter().next();
            (Some((material, b64)), remote)
        } else {
            (None, None)
        };

        // Build SDP with sip-sdp builder
        let mut audio = sip_sdp::MediaDescription::audio(ports.rtp_port);

        // Set protocol to RTP/SAVP when SRTP is active
        if local_crypto.is_some() {
            audio.protocol = sip_sdp::Protocol::RtpSavp;
        }

        for codec in &negotiated {
            audio = audio
                .add_format(codec.payload_type)
                .map_err(|e| Error::Sdp(e.to_string()))?;
            audio = audio
                .add_rtpmap(
                    codec.payload_type,
                    &codec.encoding_name,
                    codec.clock_rate,
                    None,
                )
                .map_err(|e| Error::Sdp(e.to_string()))?;
        }
        // Always add telephone-event at PT 101 (forge-engine hardcodes this PT)
        audio = audio
            .add_format(101)
            .map_err(|e| Error::Sdp(e.to_string()))?;
        audio = audio
            .add_rtpmap(101, "telephone-event", 8000, None)
            .map_err(|e| Error::Sdp(e.to_string()))?;
        audio = audio
            .add_attribute("fmtp", "101 0-15")
            .map_err(|e| Error::Sdp(e.to_string()))?;
        audio = audio
            .add_property("sendrecv")
            .map_err(|e| Error::Sdp(e.to_string()))?;

        // Add a=crypto: line if SRTP is active
        if let Some((_, ref b64)) = local_crypto {
            audio = audio
                .add_attribute(
                    "crypto",
                    &format!("1 AES_CM_128_HMAC_SHA1_80 inline:{}", b64),
                )
                .map_err(|e| Error::Sdp(e.to_string()))?;
        }

        // One answer section per offered section (RFC 3264 §6): our audio
        // answer for the first active audio section, every video or other
        // section rejected with port 0 — the backend relays no video yet.
        let sdp = match sip_sdp::SessionDescription::parse(remote_sdp) {
            Ok(offer) if !offer.media.is_empty() => {
                let (sections, _video) =
                    crate::sdp::answer_sections(&offer, |_| Ok(audio.clone()), None, None, None)?;
                crate::sdp::serialize_answer(
                    advertise_ip,
                    &session_info.forge_call_id.0,
                    "FCP Media Session",
                    sections,
                )?
            }
            // Unparseable offer: answer audio alone, as before.
            _ => sip_sdp::SessionDescription::builder()
                .origin("-", &session_info.forge_call_id.0, advertise_ip)
                .map_err(|e| Error::Sdp(e.to_string()))?
                .session_name("FCP Media Session")
                .map_err(|e| Error::Sdp(e.to_string()))?
                .connection(advertise_ip)
                .map_err(|e| Error::Sdp(e.to_string()))?
                .time(0, 0)
                .media(audio)
                .map_err(|e| Error::Sdp(e.to_string()))?
                .build()
                .serialize(),
        };

        Ok(SdpNegotiationResult {
            sdp,
            primary_codec: primary,
            local_crypto: local_crypto.map(|(material, _)| material),
            remote_crypto,
        })
    }

    /// Convert an `SdpCodecInfo` to a forge-engine `ParticipantCodecConfig`.
    /// Sync the forge session's transcoding state with both participants'
    /// currently negotiated codecs: rebuild the payload-type map (payload
    /// types arrive with each leg's SDP, after session creation) and arm
    /// transcoders when the legs' codecs differ. Safe to call whenever either
    /// leg's codec changes — arming is idempotent, and a payload-type change
    /// clears stale transcoders so the forwarding path re-arms them.
    async fn sync_transcoding_state(call_id: &fcp_core::CallId, forge_session: &ForgeMediaSession) {
        let (a_codec, a_pt) = {
            let pa = forge_session.participant_a().read().await;
            (pa.codec_config.codec, pa.payload_type)
        };
        let (b_codec, b_pt) = {
            let pb = forge_session.participant_b().read().await;
            (pb.codec_config.codec, pb.payload_type)
        };

        let mut pt_map = forge_transcoder::rtp::PayloadTypeMap::default();
        for (codec, pt) in [(a_codec, a_pt), (b_codec, b_pt)] {
            match codec {
                forge_core::AudioCodec::PCMU => pt_map.pcmu = pt,
                forge_core::AudioCodec::PCMA => pt_map.pcma = pt,
                forge_core::AudioCodec::Opus => pt_map.opus = pt,
                _ => {}
            }
        }
        forge_session.set_payload_type_map(pt_map).await;

        if a_codec == b_codec {
            debug!(
                ?call_id,
                codec = ?a_codec,
                "Codecs match - direct RTP forwarding"
            );
            return;
        }

        if !forge_session.transcoding_config().enable_transcoding {
            warn!(
                ?call_id,
                a_leg_codec = ?a_codec,
                b_leg_codec = ?b_codec,
                "Codec mismatch but transcoding is disabled (media.enable_transcoding) - legs cannot decode each other's audio"
            );
            return;
        }

        match (
            ForgeMediaSession::to_transcoder_codec(a_codec),
            ForgeMediaSession::to_transcoder_codec(b_codec),
        ) {
            (Some(a), Some(b)) => {
                let armed_a = forge_session.ensure_transcoder_a_to_b(a, b).await;
                let armed_b = forge_session.ensure_transcoder_b_to_a(b, a).await;
                match (armed_a, armed_b) {
                    (Ok(()), Ok(())) => info!(
                        ?call_id,
                        a_leg_codec = ?a_codec,
                        b_leg_codec = ?b_codec,
                        "Codec mismatch detected - transcoding armed"
                    ),
                    (ra, rb) => error!(
                        ?call_id,
                        a_to_b = ?ra,
                        b_to_a = ?rb,
                        "Failed to arm transcoders for codec mismatch"
                    ),
                }
            }
            _ => warn!(
                ?call_id,
                a_leg_codec = ?a_codec,
                b_leg_codec = ?b_codec,
                "Codec pair is not transcodable (supported: PCMU, PCMA, Opus, PCM) - RTP will be forwarded untranscoded"
            ),
        }
    }

    fn sdp_codec_to_forge_config(info: &crate::sdp::SdpCodecInfo) -> ParticipantCodecConfig {
        let codec = match info.encoding_name.to_uppercase().as_str() {
            "PCMU" => forge_core::AudioCodec::PCMU,
            "PCMA" => forge_core::AudioCodec::PCMA,
            "G722" => forge_core::AudioCodec::G722,
            "OPUS" => forge_core::AudioCodec::Opus,
            _ => forge_core::AudioCodec::PCMU,
        };
        ParticipantCodecConfig {
            payload_type: info.payload_type,
            codec,
            clock_rate: info.clock_rate,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn test_sync_transcoding_state_arms_transcoders_and_updates_pt_map() {
            let pool_config = PortPoolConfig::new(42100, 42300).unwrap();
            let port_pool = std::sync::Arc::new(forge_rtp::PortPool::new(pool_config));
            let session = ForgeMediaSession::new(
                forge_core::CallId::generate(),
                ParticipantId::generate(),
                ParticipantId::generate(),
                &port_pool,
                forge_engine::session::MediaSessionConfig {
                    transcoding_config: TranscodingConfig {
                        enable_transcoding: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
            let call_id = fcp_core::CallId::new();

            // Legs disagree on always-available codecs: A is PCMU/0, B is PCMA/8.
            {
                let mut pa = session.participant_a().write().await;
                pa.codec_config.codec = forge_core::AudioCodec::PCMU;
                pa.codec_config.payload_type = 0;
                pa.payload_type = 0;
            }
            {
                let mut pb = session.participant_b().write().await;
                pb.codec_config.codec = forge_core::AudioCodec::PCMA;
                pb.codec_config.payload_type = 8;
                pb.payload_type = 8;
            }

            sync_transcoding_state(&call_id, &session).await;

            assert!(
                session.transcoder_a_to_b().lock().await.is_some(),
                "A→B transcoder must be armed on codec mismatch"
            );
            assert!(
                session.transcoder_b_to_a().lock().await.is_some(),
                "B→A transcoder must be armed on codec mismatch"
            );

            // A dynamically negotiated Opus PT must land in the map (map
            // maintenance is independent of whether the opus codec feature is
            // compiled in), and the map change must clear armed transcoders.
            {
                let mut pb = session.participant_b().write().await;
                pb.codec_config.codec = forge_core::AudioCodec::Opus;
                pb.codec_config.payload_type = 96;
                pb.payload_type = 96;
            }
            sync_transcoding_state(&call_id, &session).await;
            let map = session.payload_type_map();
            assert_eq!(map.pcmu, 0);
            assert_eq!(map.opus, 96, "dynamic Opus PT must land in the map");

            // Matching codecs: after the map change cleared the stale
            // transcoders, matching legs must not re-arm anything.
            {
                let mut pb = session.participant_b().write().await;
                pb.codec_config.codec = forge_core::AudioCodec::PCMU;
                pb.codec_config.payload_type = 0;
                pb.payload_type = 0;
            }
            sync_transcoding_state(&call_id, &session).await;
            assert!(
                session.transcoder_a_to_b().lock().await.is_none(),
                "map change must clear transcoders, and matching codecs must not re-arm"
            );
        }

        /// An Opus leg (a softphone or browser offers it first) bridged to a
        /// G.711 leg is transcoded both ways in the default build.
        #[cfg(feature = "opus")]
        #[tokio::test]
        async fn an_opus_leg_is_transcoded_to_a_pcmu_leg() {
            let pool_config = PortPoolConfig::new(42310, 42500).unwrap();
            let port_pool = std::sync::Arc::new(forge_rtp::PortPool::new(pool_config));
            let session = ForgeMediaSession::new(
                forge_core::CallId::generate(),
                ParticipantId::generate(),
                ParticipantId::generate(),
                &port_pool,
                forge_engine::session::MediaSessionConfig {
                    transcoding_config: TranscodingConfig {
                        enable_transcoding: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
            {
                let mut pa = session.participant_a().write().await;
                pa.codec_config = sdp_codec_to_forge_config(&crate::sdp::SdpCodecInfo {
                    payload_type: 111,
                    encoding_name: "opus".into(),
                    clock_rate: 48000,
                });
                pa.payload_type = 111;
            }
            {
                let mut pb = session.participant_b().write().await;
                pb.codec_config.codec = forge_core::AudioCodec::PCMU;
                pb.codec_config.payload_type = 0;
                pb.payload_type = 0;
            }
            sync_transcoding_state(&fcp_core::CallId::new(), &session).await;
            assert!(
                session.transcoder_a_to_b().lock().await.is_some(),
                "Opus to PCMU must be armed"
            );
            assert!(
                session.transcoder_b_to_a().lock().await.is_some(),
                "PCMU to Opus must be armed"
            );
        }

        #[test]
        fn test_parse_remote_endpoint_ipv4() {
            let sdp = "v=0\r\n\
                        o=- 123 1 IN IP4 192.168.1.100\r\n\
                        s=Test\r\n\
                        c=IN IP4 192.168.1.100\r\n\
                        t=0 0\r\n\
                        m=audio 51163 RTP/AVP 0 8\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_ok());
            let addr = result.unwrap();
            assert_eq!(addr.ip().to_string(), "192.168.1.100");
            assert_eq!(addr.port(), 51163);
        }

        #[test]
        fn test_parse_remote_endpoint_missing_ip() {
            let sdp = "v=0\r\n\
                        m=audio 51163 RTP/AVP 0 8\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_remote_endpoint_missing_port() {
            let sdp = "v=0\r\n\
                        c=IN IP4 192.168.1.100\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_err());
        }

        /// A listener hears one leg's audio at 16 kHz, the other leg's
        /// not at all, and its stream ends with the session.
        #[tokio::test]
        async fn a_listener_hears_one_leg_resampled_until_the_session_ends() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 43100,
                rtp_port_end: 43300,
                public_ip: Some("127.0.0.1".into()),
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();
            let call_id = fcp_core::CallId::new();
            backend
                .create_session(
                    call_id,
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();
            let phone_a = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let phone_b = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let sdp = |port: u16| {
                format!(
                    "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
                     m=audio {port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n"
                )
            };
            let local = backend
                .update_session(&call_id, sdp(phone_a.local_addr().unwrap().port()))
                .await
                .unwrap();
            backend
                .update_session_b_leg(&call_id, sdp(phone_b.local_addr().unwrap().port()))
                .await
                .unwrap();
            let node = crate::sdp::remote_audio_endpoint(&local).unwrap();

            let mut a = backend.listen(&call_id, MediaLeg::A).await.unwrap();
            let mut b = backend.listen(&call_id, MediaLeg::B).await.unwrap();

            // One second of a 1 kHz tone from phone A, as PCMU at 20 ms.
            for n in 0..50u16 {
                let mut packet = vec![0x80, 0x00];
                packet.extend(n.to_be_bytes());
                packet.extend((n as u32 * 160).to_be_bytes());
                packet.extend(0x1234_5678u32.to_be_bytes());
                packet.extend((0..160).map(|i| {
                    let t = (n as usize * 160 + i) as f64 / 8000.0;
                    forge_codecs::g711::encode_ulaw(
                        (8000.0 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as i16,
                    )
                }));
                phone_a.send_to(&packet, node).await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            let mut heard = Vec::new();
            while heard.len() < 16_000 * 3 / 4 {
                let frame = tokio::time::timeout(Duration::from_secs(5), a.recv())
                    .await
                    .expect("leg A's audio")
                    .expect("the listener is open");
                heard.extend(frame);
            }
            // 20 ms at 8 kHz is 320 samples at 16 kHz, and the tone is there.
            let power = crate::stt::rms(&heard[3200..]);
            assert!(power > 3000.0, "the tone, resampled: rms {power}");
            assert!(b.try_recv().is_err(), "leg B sent nothing");

            backend.terminate_session(&call_id).await.unwrap();
            let ended = tokio::time::timeout(Duration::from_secs(5), async {
                while a.recv().await.is_some() {}
            })
            .await;
            assert!(ended.is_ok(), "the listener ends with the session");
        }

        #[tokio::test]
        async fn test_update_session_b_leg_sets_participant_b() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 40000,
                rtp_port_end: 41000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id = fcp_core::CallId::new();
            let dialog_id = fcp_core::DialogId::new();
            let session_config = crate::MediaSessionConfig::default();

            backend
                .create_session(call_id, dialog_id, session_config)
                .await
                .unwrap();

            let b_leg_sdp = "v=0\r\n\
                              o=- 456 1 IN IP4 10.0.0.50\r\n\
                              s=Test\r\n\
                              c=IN IP4 10.0.0.50\r\n\
                              t=0 0\r\n\
                              m=audio 6000 RTP/AVP 0\r\n"
                .to_string();

            backend
                .update_session_b_leg(&call_id, b_leg_sdp)
                .await
                .unwrap();

            // Verify participant_b remote_addr is set
            let b_addr = {
                let session_info = backend.sessions.get(&call_id).unwrap();
                let participant_b = session_info.forge_session.participant_b();
                let addr = participant_b.read().await.remote_addr;
                addr
            };
            assert!(b_addr.is_some());
            let addr = b_addr.unwrap();
            assert_eq!(addr.ip().to_string(), "10.0.0.50");
            assert_eq!(addr.port(), 6000);

            backend.terminate_session(&call_id).await.unwrap();
        }

        #[tokio::test]
        async fn test_bridge_calls_single_session() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 41000,
                rtp_port_end: 42000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id = fcp_core::CallId::new();
            let dialog_id = fcp_core::DialogId::new();

            backend
                .create_session(call_id, dialog_id, crate::MediaSessionConfig::default())
                .await
                .unwrap();

            // Single-session bridge: same call_id for both legs
            let result = backend.bridge_calls(&call_id, &call_id).await;
            assert!(result.is_ok());

            backend.terminate_session(&call_id).await.unwrap();
        }

        #[tokio::test]
        async fn test_bridge_calls_two_sessions() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 42000,
                rtp_port_end: 43000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id_a = fcp_core::CallId::new();
            let call_id_b = fcp_core::CallId::new();

            // Create A-leg session with known remote endpoint
            backend
                .create_session(
                    call_id_a,
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set A-leg participant_a remote addr
            let a_sdp = "v=0\r\nc=IN IP4 10.0.0.1\r\nm=audio 5000 RTP/AVP 0\r\n";
            backend
                .update_session(&call_id_a, a_sdp.to_string())
                .await
                .unwrap();

            // Create B-leg session
            backend
                .create_session(
                    call_id_b,
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set B-leg participant_a remote addr
            let b_sdp = "v=0\r\nc=IN IP4 10.0.0.2\r\nm=audio 6000 RTP/AVP 0\r\n";
            backend
                .update_session(&call_id_b, b_sdp.to_string())
                .await
                .unwrap();

            // Bridge calls
            backend.bridge_calls(&call_id_a, &call_id_b).await.unwrap();

            // Verify cross-wiring: A-leg's participant_b should point to B-leg phone (10.0.0.2:6000)
            {
                let a_session = backend.sessions.get(&call_id_a).unwrap();
                let b_remote = a_session
                    .forge_session
                    .participant_b()
                    .read()
                    .await
                    .remote_addr;
                assert!(b_remote.is_some());
                let addr = b_remote.unwrap();
                assert_eq!(addr.ip().to_string(), "10.0.0.2");
                assert_eq!(addr.port(), 6000);
            }

            // Verify cross-wiring: B-leg's participant_b should point to A-leg phone (10.0.0.1:5000)
            {
                let b_session = backend.sessions.get(&call_id_b).unwrap();
                let a_remote = b_session
                    .forge_session
                    .participant_b()
                    .read()
                    .await
                    .remote_addr;
                assert!(a_remote.is_some());
                let addr = a_remote.unwrap();
                assert_eq!(addr.ip().to_string(), "10.0.0.1");
                assert_eq!(addr.port(), 5000);
            }

            backend.terminate_session(&call_id_a).await.unwrap();
            backend.terminate_session(&call_id_b).await.unwrap();
        }

        #[tokio::test]
        async fn test_unbridge_calls_clears_participant_b() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 43000,
                rtp_port_end: 44000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id_a = fcp_core::CallId::new();
            let call_id_b = fcp_core::CallId::new();

            backend
                .create_session(
                    call_id_a,
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            backend
                .create_session(
                    call_id_b,
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set up endpoints and bridge
            let a_sdp = "v=0\r\nc=IN IP4 10.0.0.1\r\nm=audio 5000 RTP/AVP 0\r\n";
            let b_sdp = "v=0\r\nc=IN IP4 10.0.0.2\r\nm=audio 6000 RTP/AVP 0\r\n";
            backend
                .update_session(&call_id_a, a_sdp.to_string())
                .await
                .unwrap();
            backend
                .update_session(&call_id_b, b_sdp.to_string())
                .await
                .unwrap();
            backend.bridge_calls(&call_id_a, &call_id_b).await.unwrap();

            // Unbridge
            backend
                .unbridge_calls(&call_id_a, &call_id_b)
                .await
                .unwrap();

            // Verify participant_b remote_addr cleared on both
            {
                let a_session = backend.sessions.get(&call_id_a).unwrap();
                let b_remote = a_session
                    .forge_session
                    .participant_b()
                    .read()
                    .await
                    .remote_addr;
                assert!(b_remote.is_none());
            }
            {
                let b_session = backend.sessions.get(&call_id_b).unwrap();
                let a_remote = b_session
                    .forge_session
                    .participant_b()
                    .read()
                    .await
                    .remote_addr;
                assert!(a_remote.is_none());
            }

            backend.terminate_session(&call_id_a).await.unwrap();
            backend.terminate_session(&call_id_b).await.unwrap();
        }
    }
}

#[cfg(feature = "embedded")]
pub use embedded::EmbeddedBackend;

/// Remote backend - forge-media runs as HTTP service
#[cfg(feature = "remote")]
pub mod remote {
    use super::*;
    use crate::{config::BackendType, MediaSessionStats};
    use dashmap::DashMap;
    use reqwest::Client;
    use serde::{Deserialize, Serialize};
    use tracing::{debug, info};

    pub struct RemoteBackend {
        /// HTTP client
        client: Client,

        /// Base URL of forge-media server
        base_url: String,

        /// Optional authentication token
        auth_token: Option<String>,

        /// Session cache (CallId -> MediaSession)
        sessions: DashMap<fcp_core::CallId, Arc<MediaSession>>,
    }

    impl RemoteBackend {
        pub async fn new(config: &crate::config::MediaConfig) -> Result<Self> {
            let (base_url, auth_token, timeout_ms) = match &config.backend {
                BackendType::Remote {
                    endpoint,
                    auth_token,
                    timeout_ms,
                    ..
                } => (endpoint.clone(), auth_token.clone(), *timeout_ms),
                _ => {
                    return Err(Error::Config(
                        "RemoteBackend requires Remote backend type".into(),
                    ))
                }
            };

            info!(base_url, "Initializing remote media backend");

            let client = Client::builder()
                .timeout(std::time::Duration::from_millis(timeout_ms))
                .build()
                .map_err(|e| Error::Config(format!("Failed to create HTTP client: {}", e)))?;

            // Health check
            let health_url = format!("{}/health", base_url);
            client
                .get(&health_url)
                .send()
                .await
                .map_err(|e| Error::Backend(format!("forge-media not reachable: {}", e)))?;

            info!("Remote media backend connected");

            Ok(Self {
                client,
                base_url,
                auth_token,
                sessions: DashMap::new(),
            })
        }

        fn auth_header(&self) -> Option<String> {
            self.auth_token
                .as_ref()
                .map(|token| format!("Bearer {}", token))
        }
    }

    #[derive(Serialize)]
    struct CreateSessionReq {
        call_id: String,
    }

    #[derive(Deserialize)]
    struct CreateSessionResp {
        session_id: String,
    }

    #[async_trait]
    impl MediaBackend for RemoteBackend {
        async fn create_session(
            &self,
            call_id: fcp_core::CallId,
            dialog_id: fcp_core::DialogId,
            config: MediaSessionConfig,
        ) -> Result<Arc<MediaSession>> {
            debug!(?call_id, ?dialog_id, "Creating remote media session");

            let req = CreateSessionReq {
                call_id: call_id.to_string(),
            };

            let url = format!("{}/v1/sessions", self.base_url);
            let mut request = self.client.post(&url).json(&req);

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let resp: CreateSessionResp = request.send().await?.json().await?;

            // TODO: Get actual RTP/RTCP ports from remote forge-media API
            // For now, use placeholder values since remote mode is future work
            let rtp_port = 0;
            let rtcp_port = 0;

            let session = Arc::new(MediaSession::new(
                call_id,
                dialog_id,
                resp.session_id,
                config,
                rtp_port,
                rtcp_port,
            ));

            self.sessions.insert(call_id, session.clone());

            info!(
                ?call_id,
                forge_session_id = session.forge_session_id(),
                "Remote media session created"
            );

            Ok(session)
        }

        async fn get_session(&self, call_id: &fcp_core::CallId) -> Result<Arc<MediaSession>> {
            self.sessions
                .get(call_id)
                .map(|entry| entry.value().clone())
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))
        }

        async fn update_session(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<String> {
            debug!(?call_id, "Updating remote session with SDP");

            let session = self.get_session(call_id).await?;

            #[derive(Serialize)]
            struct UpdateReq {
                sdp: String,
            }

            #[derive(Deserialize)]
            struct UpdateResp {
                sdp: String,
            }

            let url = format!(
                "{}/v1/sessions/{}/sdp",
                self.base_url,
                session.forge_session_id()
            );
            let mut request = self.client.post(&url).json(&UpdateReq {
                sdp: remote_sdp.clone(),
            });

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let resp: UpdateResp = request.send().await?.json().await?;

            session.set_remote_sdp(remote_sdp).await;
            session.set_local_sdp(resp.sdp.clone()).await;
            session
                .set_state(crate::session::MediaSessionState::Active)
                .await;

            Ok(resp.sdp)
        }

        async fn update_session_b_leg(
            &self,
            _call_id: &fcp_core::CallId,
            _remote_sdp: String,
        ) -> Result<()> {
            // POST /v1/sessions/{id}/sdp-b-leg
            Err(Error::Backend(
                "remote media backend does not implement B-leg SDP update".to_string(),
            ))
        }

        async fn hold_session(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // POST /v1/sessions/{id}/hold
            Err(Error::Backend(
                "remote media backend does not implement hold".to_string(),
            ))
        }

        async fn resume_session(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // POST /v1/sessions/{id}/resume
            Err(Error::Backend(
                "remote media backend does not implement resume".to_string(),
            ))
        }

        async fn terminate_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Terminating remote media session");

            if let Some((_, session)) = self.sessions.remove(call_id) {
                session
                    .set_state(crate::session::MediaSessionState::Terminated)
                    .await;

                let url = format!(
                    "{}/v1/sessions/{}",
                    self.base_url,
                    session.forge_session_id()
                );
                let mut request = self.client.delete(&url);

                if let Some(auth) = self.auth_header() {
                    request = request.header("Authorization", auth);
                }

                request.send().await?;

                info!(?call_id, "Remote media session terminated");
            }

            Ok(())
        }

        async fn get_stats(&self, call_id: &fcp_core::CallId) -> Result<MediaSessionStats> {
            let session = self.get_session(call_id).await?;

            let url = format!(
                "{}/v1/sessions/{}/stats",
                self.base_url,
                session.forge_session_id()
            );
            let mut request = self.client.get(&url);

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let stats: MediaSessionStats = request.send().await?.json().await?;
            Ok(stats)
        }

        async fn send_dtmf(
            &self,
            _call_id: &fcp_core::CallId,
            _leg: MediaLeg,
            _digit: char,
            _duration_ms: u32,
        ) -> Result<()> {
            // POST /v1/sessions/{id}/dtmf
            Err(Error::Backend(
                "remote media backend does not implement DTMF".to_string(),
            ))
        }

        async fn start_recording(&self, _call_id: &fcp_core::CallId, _path: String) -> Result<()> {
            // POST /v1/sessions/{id}/recording
            Err(Error::Backend(
                "remote media backend does not implement recording".to_string(),
            ))
        }

        async fn stop_recording(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // DELETE /v1/sessions/{id}/recording
            Err(Error::Backend(
                "remote media backend does not implement recording stop".to_string(),
            ))
        }

        async fn play_file(
            &self,
            _call_id: &fcp_core::CallId,
            _file_path: String,
            _target: AudioTarget,
        ) -> Result<String> {
            // POST /v1/sessions/{id}/playback/file
            Err(Error::Backend(
                "remote media backend does not implement audio playback".to_string(),
            ))
        }

        async fn play_tone(
            &self,
            _call_id: &fcp_core::CallId,
            _frequency: f32,
            _duration_ms: u32,
            _amplitude: f32,
            _target: AudioTarget,
        ) -> Result<String> {
            // POST /v1/sessions/{id}/playback/tone
            Err(Error::Backend(
                "remote media backend does not implement tone playback".to_string(),
            ))
        }

        async fn play_tts(
            &self,
            _call_id: &fcp_core::CallId,
            _text: String,
            _language: String,
            _voice: Option<String>,
            _target: AudioTarget,
        ) -> Result<String> {
            // POST /v1/sessions/{id}/playback/tts
            Err(Error::Backend(
                "remote media backend does not implement TTS playback".to_string(),
            ))
        }

        async fn stop_playback(
            &self,
            _call_id: &fcp_core::CallId,
            _playback_id: String,
        ) -> Result<()> {
            // DELETE /v1/sessions/{id}/playback/{playback_id}
            Err(Error::Backend(
                "remote media backend does not implement playback stop".to_string(),
            ))
        }

        async fn wait_playback_completion(&self, _playback_id: String) -> Result<()> {
            // GET /v1/playback/{playback_id}/wait or use websocket for completion events
            Err(Error::Backend(
                "remote media backend does not implement playback completion waiting".to_string(),
            ))
        }

        async fn bridge_calls(
            &self,
            _a_leg_call_id: &CallId,
            _b_leg_call_id: &CallId,
        ) -> Result<()> {
            // POST /v1/bridge
            // Body: { "a_leg": "call-id-1", "b_leg": "call-id-2" }
            Err(Error::Backend(
                "remote media backend does not implement call bridging".to_string(),
            ))
        }

        async fn unbridge_calls(
            &self,
            _a_leg_call_id: &CallId,
            _b_leg_call_id: &CallId,
        ) -> Result<()> {
            // DELETE /v1/bridge
            // Body: { "a_leg": "call-id-1", "b_leg": "call-id-2" }
            Err(Error::Backend(
                "remote media backend does not implement call unbridging".to_string(),
            ))
        }

        fn backend_name(&self) -> &'static str {
            "remote"
        }

        async fn health_check(&self) -> Result<bool> {
            let url = format!("{}/health", self.base_url);
            let resp = self.client.get(&url).send().await?;
            Ok(resp.status().is_success())
        }

        async fn list_sessions(&self) -> Vec<RtpSessionInfo> {
            // TODO: Implement remote session listing via HTTP API
            Vec::new()
        }
    }
}

#[cfg(feature = "remote")]
pub use remote::RemoteBackend;
