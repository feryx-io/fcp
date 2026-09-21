//! A queue call's recording: its consent step, its two-way mix and its
//! notice tone (`docs/COMPLIANCE_FOLLOWUPS.md` §8.2).
//!
//! The queue bridges raw RTP and has no media session, so it takes the
//! B2BUA's steps in its own terms. The tenant's announcement is decoded,
//! encoded to each leg's codec and played to the caller's and the agent's
//! sockets together, and the bridge starts only when both have heard it.
//! A failed announcement under `skip` records nothing. The recorder then
//! decodes each direction with its own leg's codec and mixes them into one
//! track as they arrive, rather than writing one direction after the other.

use crate::queue_audio;
use bytes::Bytes;
use fcp_core::compliance::{
    ConsentPlan, RecordingCounters, NOTICE_TONE_MS, RECORDING_CONSENT_TEXT,
};
use fcp_core::recording::{CallRecording, CallRecordingSource, CallRecordingStore};
use fcp_core::{RecordingConsent, TenantId};
use forge_codecs::AudioCodecType;
use forge_rtp::RtpSocketPair;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// The payload type the queue offers telephone events on.
const TELEPHONE_EVENT_PT: u8 = 101;

/// Samples in one 20 ms frame at 8 kHz, the rate queue recordings use.
const FRAME: usize = 160;

/// How long a leg must have been quiet before the other is written
/// against silence: half a second, so a phone that sends nothing while
/// its user is quiet does not hold the recording back.
///
/// It is this long because a loaded node does not hand the recorder the
/// two directions evenly — it hands it a burst from one leg and then a
/// burst from the other, a few hundred milliseconds apart. A recording
/// is not live, so waiting out that gap costs nothing and keeps two
/// people who talked at once in the file at once.
const QUIET: Duration = Duration::from_millis(500);

/// The most one direction may hold while it waits for the other: two
/// seconds, so a leg whose packets stop being delivered altogether cannot
/// grow the mix without bound.
const MAX_HELD: usize = 100 * FRAME;

/// One side of a queue call, as the bridge sees it.
#[derive(Clone)]
pub(crate) struct Leg {
    pub socket: Arc<RtpSocketPair>,
    /// `PCMU`, `PCMA` or `G722`.
    pub codec: String,
    pub payload_type: u8,
}

/// Where a queue's recordings go, how they are registered, and what their
/// consent step needs (`docs/COMPLIANCE_FOLLOWUPS.md` §4, §8.2).
#[derive(Clone)]
pub(crate) struct QueueRecordings {
    pub dir: String,
    pub store: Option<Arc<dyn CallRecordingStore>>,
    pub storage: Arc<fcp_core::DynStorage>,
    pub tts: Option<Arc<fcp_media::TtsService>>,
}

impl QueueRecordings {
    /// The tenant's consent plan.
    pub async fn plan(&self, tenant_id: &TenantId) -> ConsentPlan {
        ConsentPlan::resolve(
            self.storage.compliance_store().map(|s| s.as_ref()),
            self.storage.prompt_store().map(|s| s.as_ref()),
            tenant_id,
        )
        .await
    }

    /// Play the plan's announcement to both legs together and wait for both
    /// to finish. True when it was played.
    pub async fn announce(&self, plan: &ConsentPlan, caller: &Leg, agent: &Leg) -> bool {
        let audio = match &plan.prompt_audio {
            Some(path) => Some(path.clone()),
            None => match &self.tts {
                Some(tts) => match tokio::time::timeout(
                    Duration::from_secs(10),
                    tts.synthesize(RECORDING_CONSENT_TEXT, None, None),
                )
                .await
                {
                    Ok(Ok(path)) => Some(path),
                    Ok(Err(e)) => {
                        warn!(error = %e, "Queue recording-consent speech not synthesised");
                        None
                    }
                    Err(_) => {
                        warn!("Queue recording-consent speech timed out");
                        None
                    }
                },
                None => None,
            },
        };
        let Some(audio) = audio else {
            warn!("The tenant asks for a recording announcement but there is no prompt audio and no TTS");
            return false;
        };
        let pcm = match load_pcm_8k(&audio) {
            Ok(pcm) if !pcm.is_empty() => pcm,
            Ok(_) => {
                warn!(audio = %audio.display(), "Recording-consent prompt is empty");
                return false;
            }
            Err(e) => {
                warn!(audio = %audio.display(), error = %e, "Recording-consent prompt not readable");
                return false;
            }
        };
        let (Some(caller_frames), Some(agent_frames)) = (
            queue_audio::encode_frames(&caller.codec, &pcm),
            queue_audio::encode_frames(&agent.codec, &pcm),
        ) else {
            warn!(caller = %caller.codec, agent = %agent.codec, "No encoder for a leg's codec; announcement not played");
            return false;
        };
        tokio::join!(
            crate::queue_handler::play_prompt_frames(
                &caller.socket,
                &caller_frames,
                caller.payload_type
            ),
            crate::queue_handler::play_prompt_frames(
                &agent.socket,
                &agent_frames,
                agent.payload_type
            ),
        );
        info!(audio = %audio.display(), "Queue recording announcement played to caller and agent");
        true
    }

    /// The file for a new recording of `call_id`, its row written with the
    /// consent outcome. Under the tenant and the day, named for the
    /// recording — never for the queue or a dialled number. `None` when the
    /// row cannot be written: a recording nothing can find is not made.
    pub async fn begin(
        &self,
        tenant_id: TenantId,
        call_id: String,
        format: &str,
        consent: RecordingConsent,
    ) -> Option<String> {
        let id = fcp_core::RecordingId::new();
        let dir = Path::new(&self.dir).join(fcp_core::recording::dated_recording_dir(
            &tenant_id,
            std::time::SystemTime::now(),
        ));
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!(dir = %dir.display(), error = %e, "Queue recording directory not created; not recording");
            return None;
        }
        let path = dir
            .join(format!("{id}.{format}"))
            .to_string_lossy()
            .into_owned();
        if let Some(store) = &self.store {
            let mut recording = CallRecording::starting(
                tenant_id,
                call_id,
                CallRecordingSource::Queue,
                path.clone(),
            );
            recording.id = id;
            recording.consent = consent;
            if let Err(e) = store.create_call_recording(&recording).await {
                warn!(error = %e, "Queue recording not registered; not recording");
                return None;
            }
        }
        Some(path)
    }
}

/// What a queue call's consent step decided.
pub(crate) struct ConsentStep {
    /// On the segment's record.
    pub consent: RecordingConsent,
    /// The notice tone the bridge writes into both directions.
    pub notice: Option<Arc<NoticeTone>>,
}

impl ConsentStep {
    /// Whether the recording goes ahead.
    pub fn records(&self) -> bool {
        self.consent != RecordingConsent::Refused
    }
}

/// Run the consent step for a queue call about to be recorded.
pub(crate) async fn consent_step(
    recordings: &QueueRecordings,
    tenant_id: &TenantId,
    caller: &Leg,
    agent: &Leg,
) -> ConsentStep {
    let plan = recordings.plan(tenant_id).await;
    let announced = plan.announce && recordings.announce(&plan, caller, agent).await;
    let consent = plan.outcome(announced);
    RecordingCounters::global().note(CallRecordingSource::Queue.as_str(), consent);
    if consent == RecordingConsent::Refused {
        warn!(tenant_id = %tenant_id, "Queue call not recorded: the consent announcement failed and the policy says skip");
    }
    ConsentStep {
        consent,
        notice: plan
            .notice_interval
            .map(|every| Arc::new(NoticeTone::new(every))),
    }
}

/// A 16-bit WAV file as mono samples at 8 kHz.
fn load_pcm_8k(path: &Path) -> anyhow::Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader.samples::<i16>().collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, 8) => reader
            .samples::<i8>()
            .map(|s| s.map(|s| i16::from(s) << 8))
            .collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, bits @ (24 | 32)) => reader
            .samples::<i32>()
            .map(|s| s.map(|s| (s >> (bits - 16)) as i16))
            .collect::<Result<_, _>>()?,
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|s| s.map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16))
            .collect::<Result<_, _>>()?,
        (format, bits) => anyhow::bail!("unsupported WAV sample format {format:?}/{bits}"),
    };
    let channels = usize::from(spec.channels.max(1));
    let mono: Vec<i16> = samples
        .chunks(channels)
        .map(|frame| (frame.iter().map(|&s| i32::from(s)).sum::<i32>() / frame.len() as i32) as i16)
        .collect();
    if spec.sample_rate == 8000 {
        return Ok(mono);
    }
    let mut resampler = forge_resampler::Resampler::new(spec.sample_rate, 8000, 1)
        .map_err(|e| anyhow::anyhow!("resampling {}: {e}", spec.sample_rate))?;
    resampler
        .resample(&mono)
        .map_err(|e| anyhow::anyhow!("resampling {}: {e}", spec.sample_rate))
}

// ── The two-way recording ──────────────────────────────────────────────

/// The audio of one RTP packet from one leg, at 8 kHz.
pub(crate) struct LegDecoder {
    codec: AudioCodecType,
    payload_type: u8,
    g722: Option<forge_codecs::g722::G722Decoder>,
}

impl LegDecoder {
    pub fn new(leg: &Leg) -> Self {
        let codec = crate::queue_handler::codec_name_to_type(&leg.codec);
        Self {
            codec,
            payload_type: leg.payload_type,
            g722: (codec == AudioCodecType::G722).then(|| {
                forge_codecs::g722::G722Decoder::new(forge_codecs::g722::G722BitRate::Rate64k)
            }),
        }
    }

    /// The packet's samples; `None` for a packet that is not this leg's
    /// audio (telephone events, comfort noise, a malformed header).
    pub fn decode(&mut self, packet: &[u8]) -> Option<Vec<i16>> {
        let (payload_type, payload) = rtp_payload(packet)?;
        if payload_type != self.payload_type || payload.is_empty() {
            return None;
        }
        Some(match self.codec {
            AudioCodecType::PCMU => payload
                .iter()
                .map(|&b| forge_codecs::g711::decode_ulaw(b))
                .collect(),
            AudioCodecType::PCMA => payload
                .iter()
                .map(|&b| forge_codecs::g711::decode_alaw(b))
                .collect(),
            AudioCodecType::G722 => {
                let wide = self.g722.as_mut()?.decode(payload);
                wide.chunks(2)
                    .map(|pair| {
                        (pair.iter().map(|&s| i32::from(s)).sum::<i32>() / pair.len() as i32) as i16
                    })
                    .collect()
            }
            _ => return None,
        })
    }
}

/// An RTP packet's payload type and payload, past CSRCs, the header
/// extension and padding.
fn rtp_payload(packet: &[u8]) -> Option<(u8, &[u8])> {
    if packet.len() < 12 || packet[0] >> 6 != 2 {
        return None;
    }
    let csrcs = usize::from(packet[0] & 0x0f);
    let mut start = 12 + 4 * csrcs;
    if packet[0] & 0x10 != 0 {
        let words = usize::from(u16::from_be_bytes([
            *packet.get(start + 2)?,
            *packet.get(start + 3)?,
        ]));
        start += 4 + 4 * words;
    }
    let mut end = packet.len();
    if packet[0] & 0x20 != 0 {
        end = end.checked_sub(usize::from(*packet.last()?))?;
    }
    (start <= end).then(|| (packet[1] & 0x7f, &packet[start..end]))
}

/// Two directions of a call mixed into one track as they arrive: a frame
/// is written once both have one, or once the other has been quiet for
/// [`QUIET`] and is taken as silent.
///
/// The wait is on the clock rather than on how much one side has piled
/// up, because the two directions do not arrive evenly: a loaded node
/// hands the recorder a burst from one leg and then a burst from the
/// other, and a mix that wrote whichever side was longest would put two
/// people who talked over each other into the recording one after the
/// other.
pub(crate) struct TwoWayMix {
    a: VecDeque<i16>,
    b: VecDeque<i16>,
    /// When each direction last delivered audio.
    last_a: Instant,
    last_b: Instant,
}

impl Default for TwoWayMix {
    fn default() -> Self {
        Self::starting_at(Instant::now())
    }
}

impl TwoWayMix {
    fn starting_at(now: Instant) -> Self {
        Self {
            a: VecDeque::new(),
            b: VecDeque::new(),
            last_a: now,
            last_b: now,
        }
    }

    pub fn push_a(&mut self, samples: &[i16]) -> Vec<i16> {
        self.push_a_at(samples, Instant::now())
    }

    pub fn push_b(&mut self, samples: &[i16]) -> Vec<i16> {
        self.push_b_at(samples, Instant::now())
    }

    fn push_a_at(&mut self, samples: &[i16], now: Instant) -> Vec<i16> {
        self.a.extend(samples);
        self.last_a = now;
        self.ready(now)
    }

    fn push_b_at(&mut self, samples: &[i16], now: Instant) -> Vec<i16> {
        self.b.extend(samples);
        self.last_b = now;
        self.ready(now)
    }

    /// What is left when the call ends.
    pub fn finish(&mut self) -> Vec<i16> {
        let n = self.a.len().max(self.b.len());
        self.take(n)
    }

    fn ready(&mut self, now: Instant) -> Vec<i16> {
        let mut out = Vec::new();
        loop {
            let (a, b) = (self.a.len(), self.b.len());
            let quiet = |since: Instant, held: usize| {
                now.saturating_duration_since(since) >= QUIET || held >= MAX_HELD
            };
            let both = a >= FRAME && b >= FRAME;
            let a_alone = a >= FRAME && quiet(self.last_b, a);
            let b_alone = b >= FRAME && quiet(self.last_a, b);
            if both || a_alone || b_alone {
                out.extend(self.take(FRAME));
            } else {
                return out;
            }
        }
    }

    fn take(&mut self, n: usize) -> Vec<i16> {
        (0..n)
            .map(|_| {
                let a = i32::from(self.a.pop_front().unwrap_or(0));
                let b = i32::from(self.b.pop_front().unwrap_or(0));
                (a + b).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            })
            .collect()
    }
}

/// Record both directions of a queue call, mixed, to a WAV file at 8 kHz.
/// `a_rx` carries the caller's packets, `b_rx` the agent's, each in its own
/// leg's codec.
pub(crate) fn spawn_mixed_recording(
    mut a_rx: tokio::sync::broadcast::Receiver<Bytes>,
    mut b_rx: tokio::sync::broadcast::Receiver<Bytes>,
    caller: &Leg,
    agent: &Leg,
    path: String,
) -> JoinHandle<()> {
    let mut a_decoder = LegDecoder::new(caller);
    let mut b_decoder = LegDecoder::new(agent);
    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;

        let format = forge_core::AudioFormat {
            codec: forge_core::AudioCodec::PCM,
            sample_rate: 8000,
            channels: 1,
        };
        let recorder = match forge_recorder::AudioRecorder::new(&path, format).await {
            Ok(r) => r,
            Err(e) => {
                warn!(path = %path, error = %e, "Failed to create audio recorder");
                return;
            }
        };
        if let Err(e) = recorder.start() {
            warn!(path = %path, error = %e, "Failed to start recording");
            return;
        }
        info!(path = %path, "Recording started");

        let mut mix = TwoWayMix::default();
        let (mut a_open, mut b_open) = (true, true);
        while a_open || b_open {
            let mixed = tokio::select! {
                got = a_rx.recv(), if a_open => match got {
                    Ok(packet) => a_decoder.decode(&packet).map(|s| mix.push_a(&s)),
                    Err(RecvError::Lagged(n)) => {
                        debug!(path = %path, skipped = n, "Recording fell behind the caller's audio");
                        None
                    }
                    Err(RecvError::Closed) => {
                        a_open = false;
                        None
                    }
                },
                got = b_rx.recv(), if b_open => match got {
                    Ok(packet) => b_decoder.decode(&packet).map(|s| mix.push_b(&s)),
                    Err(RecvError::Lagged(n)) => {
                        debug!(path = %path, skipped = n, "Recording fell behind the agent's audio");
                        None
                    }
                    Err(RecvError::Closed) => {
                        b_open = false;
                        None
                    }
                },
            };
            if let Some(samples) = mixed.filter(|s| !s.is_empty()) {
                let _ = recorder.write_samples(&samples);
            }
        }
        let rest = mix.finish();
        if !rest.is_empty() {
            let _ = recorder.write_samples(&rest);
        }
        if let Err(e) = recorder.stop() {
            warn!(path = %path, error = %e, "Failed to stop recording");
        } else {
            info!(path = %path, duration = recorder.duration_secs(), "Recording completed");
        }
    })
}

// ── The notice tone ────────────────────────────────────────────────────

/// The continuing notice on a recorded queue call: every interval the
/// bridge writes the beep over the audio it forwards, in both directions.
pub(crate) struct NoticeTone {
    started: Instant,
    every: Duration,
    samples: Vec<i16>,
}

impl NoticeTone {
    pub fn new(every: Duration) -> Self {
        Self::starting_at(Instant::now(), every)
    }

    fn starting_at(started: Instant, every: Duration) -> Self {
        Self {
            started,
            every,
            samples: fcp_core::compliance::notice_tone_samples(8000),
        }
    }

    /// The beep's samples for the 20 ms frame forwarded at `now`, if the
    /// beep is sounding then. The first sounds one interval after the
    /// recording starts.
    fn frame_at(&self, now: Instant) -> Option<&[i16]> {
        let elapsed = now.checked_duration_since(self.started)?;
        if elapsed < self.every {
            return None;
        }
        let into = elapsed.as_micros() % self.every.as_micros().max(1);
        let offset = (into * 8000 / 1_000_000) as usize;
        if into >= u128::from(NOTICE_TONE_MS) * 1000 || offset >= self.samples.len() {
            return None;
        }
        let end = (offset + FRAME).min(self.samples.len());
        Some(&self.samples[offset..end])
    }
}

/// Writes the notice over one direction's packets, in that direction's
/// codec.
pub(crate) struct NoticeWriter {
    tone: Arc<NoticeTone>,
    codec: AudioCodecType,
    payload_type: Option<u8>,
    g722: Option<forge_codecs::g722::G722Encoder>,
}

impl NoticeWriter {
    /// For packets leaving in `codec`; `payload_type` is the audio type
    /// to overwrite, `None` for any but telephone events.
    pub fn new(tone: Arc<NoticeTone>, codec: AudioCodecType, payload_type: Option<u8>) -> Self {
        Self {
            tone,
            codec,
            payload_type,
            g722: (codec == AudioCodecType::G722).then(|| {
                forge_codecs::g722::G722Encoder::new(forge_codecs::g722::G722BitRate::Rate64k)
            }),
        }
    }

    /// The packet to forward: `packet` itself, or a copy carrying the beep.
    pub fn apply(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let beep = self.tone.frame_at(Instant::now())?;
        let (payload_type, payload) = rtp_payload(packet)?;
        if payload_type == TELEPHONE_EVENT_PT
            || self.payload_type.is_some_and(|pt| pt != payload_type)
        {
            return None;
        }
        let header = payload.as_ptr() as usize - packet.as_ptr() as usize;
        let encoded: Vec<u8> = match self.codec {
            AudioCodecType::PCMU => fit(beep, payload.len())
                .iter()
                .map(|&s| forge_codecs::g711::encode_ulaw(s))
                .collect(),
            AudioCodecType::PCMA => fit(beep, payload.len())
                .iter()
                .map(|&s| forge_codecs::g711::encode_alaw(s))
                .collect(),
            AudioCodecType::G722 => {
                let wide: Vec<i16> = fit(beep, payload.len())
                    .iter()
                    .flat_map(|&s| [s, s])
                    .collect();
                self.g722.as_mut()?.encode(&wide).ok()?
            }
            _ => return None,
        };
        let mut out = packet[..header].to_vec();
        out[0] &= !0x20; // the new payload carries no padding
        out.extend_from_slice(&encoded);
        Some(out)
    }
}

/// `samples` cut or padded with silence to `n`.
fn fit(samples: &[i16], n: usize) -> Vec<i16> {
    let mut out = samples[..samples.len().min(n)].to_vec();
    out.resize(n, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(pt: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, pt, 0, 1, 0, 0, 0, 160, 1, 2, 3, 4];
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn the_payload_is_found_past_csrcs_extensions_and_padding() {
        assert_eq!(
            rtp_payload(&packet(0, &[1, 2, 3])),
            Some((0, &[1u8, 2, 3][..]))
        );

        let mut with_csrc = vec![0x81, 8, 0, 1, 0, 0, 0, 160, 1, 2, 3, 4, 9, 9, 9, 9];
        with_csrc.extend_from_slice(&[7, 7]);
        assert_eq!(rtp_payload(&with_csrc), Some((8, &[7u8, 7][..])));

        let mut with_ext = vec![0x90, 0, 0, 1, 0, 0, 0, 160, 1, 2, 3, 4];
        with_ext.extend_from_slice(&[0xbe, 0xef, 0, 1, 5, 5, 5, 5]);
        with_ext.extend_from_slice(&[6]);
        assert_eq!(rtp_payload(&with_ext), Some((0, &[6u8][..])));

        let mut padded = vec![0xa0, 0, 0, 1, 0, 0, 0, 160, 1, 2, 3, 4];
        padded.extend_from_slice(&[1, 2, 0, 0, 3]);
        assert_eq!(rtp_payload(&padded), Some((0, &[1u8, 2][..])));

        assert_eq!(rtp_payload(&[0x80, 0, 0]), None);
        assert_eq!(rtp_payload(&[0x40; 20]), None, "not version 2");
    }

    #[test]
    fn a_leg_decodes_its_own_codec_and_passes_telephone_events_by() {
        let mut ulaw = LegDecoder {
            codec: AudioCodecType::PCMU,
            payload_type: 0,
            g722: None,
        };
        let silence = ulaw.decode(&packet(0, &[0xff; 160])).unwrap();
        assert_eq!(silence.len(), 160);
        assert!(silence.iter().all(|&s| s.abs() < 8));
        assert_eq!(ulaw.decode(&packet(101, &[1, 0x80, 0, 160])), None);

        let mut alaw = LegDecoder {
            codec: AudioCodecType::PCMA,
            payload_type: 8,
            g722: None,
        };
        assert_eq!(alaw.decode(&packet(8, &[0xd5; 160])).unwrap().len(), 160);
        assert_eq!(
            alaw.decode(&packet(0, &[0xd5; 160])),
            None,
            "another leg's type"
        );
    }

    #[test]
    fn both_directions_are_mixed_frame_by_frame() {
        let t0 = Instant::now();
        let mut mix = TwoWayMix::starting_at(t0);
        assert!(
            mix.push_a_at(&[100; FRAME], t0).is_empty(),
            "waits for the other side"
        );
        let out = mix.push_b_at(&[23; FRAME], t0 + ms(20));
        assert_eq!(out, vec![123; FRAME]);

        // One side silent: it is held while the other might still arrive,
        // then written against silence once that side has been quiet.
        let mut out = Vec::new();
        for i in 1..=5 {
            out.extend(mix.push_a_at(&[7; FRAME], t0 + ms(20 + 20 * i)));
        }
        assert!(out.is_empty(), "held while the other side might arrive");
        let out = mix.push_a_at(&[7; FRAME], t0 + ms(620));
        assert_eq!(out, vec![7; 6 * FRAME], "written once the other is quiet");
        assert!(mix.finish().is_empty());

        // Loud audio clamps rather than wrapping.
        let mut mix = TwoWayMix::starting_at(t0);
        mix.push_a_at(&[i16::MAX; FRAME], t0);
        assert_eq!(mix.push_b_at(&[i16::MAX; FRAME], t0), vec![i16::MAX; FRAME]);
    }

    #[test]
    fn a_burst_from_one_leg_is_mixed_with_the_burst_that_follows_it() {
        // A loaded node hands the recorder a quarter of a second of the
        // caller at once, then a quarter of a second of the agent. They
        // talked over each other, so the recording must have them over
        // each other rather than one after the other.
        let t0 = Instant::now();
        let mut mix = TwoWayMix::starting_at(t0);
        let mut out = Vec::new();
        for burst in 0..4 {
            let at = t0 + ms(500 * burst);
            for _ in 0..12 {
                out.extend(mix.push_a_at(&[100; FRAME], at));
            }
            let at = at + ms(250);
            for _ in 0..12 {
                out.extend(mix.push_b_at(&[23; FRAME], at));
            }
        }
        out.extend(mix.finish());
        let frames = out.len() / FRAME;
        let together = out.chunks(FRAME).filter(|f| f[0] == 123).count();
        assert_eq!(frames, 48, "every frame of both bursts is written");
        assert_eq!(together, frames, "and every frame has both of them");
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_notice_sounds_once_per_interval_for_its_length() {
        let start = Instant::now();
        let tone = NoticeTone::starting_at(start, Duration::from_secs(15));
        assert!(
            tone.frame_at(start + Duration::from_secs(1)).is_none(),
            "not before the first interval"
        );
        let first = tone.frame_at(start + Duration::from_secs(15)).unwrap();
        assert_eq!(first.len(), FRAME);
        assert!(tone
            .frame_at(start + Duration::from_millis(15_100))
            .is_some());
        assert!(tone
            .frame_at(start + Duration::from_millis(15_000 + u64::from(NOTICE_TONE_MS) + 5))
            .is_none());
        assert!(
            tone.frame_at(start + Duration::from_secs(30)).is_some(),
            "and again"
        );
        assert!(tone.frame_at(start + Duration::from_secs(37)).is_none());
    }

    #[test]
    fn the_notice_overwrites_audio_but_not_telephone_events() {
        let tone = Arc::new(NoticeTone::starting_at(
            Instant::now() - Duration::from_secs(5),
            Duration::from_secs(5),
        ));
        let mut writer = NoticeWriter::new(Arc::clone(&tone), AudioCodecType::PCMU, Some(0));
        let original = packet(0, &[0xff; 160]);
        let beeped = writer.apply(&original).expect("the beep is sounding");
        assert_eq!(beeped.len(), original.len());
        assert_eq!(&beeped[..12], &original[..12], "the header is kept");
        assert_ne!(&beeped[12..], &original[12..]);
        assert!(writer.apply(&packet(101, &[1, 0x80, 0, 160])).is_none());
    }

    #[test]
    fn a_prompt_is_read_at_eight_kilohertz_whatever_its_rate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..16000 {
            w.write_sample(1000i16).unwrap();
            w.write_sample(3000i16).unwrap();
        }
        w.finalize().unwrap();
        let pcm = load_pcm_8k(&path).unwrap();
        assert!((7900..=8100).contains(&pcm.len()), "{}", pcm.len());
        assert!(
            pcm[4000].abs_diff(2000) < 100,
            "channels averaged: {}",
            pcm[4000]
        );
    }
}
