//! Recording a conference room (design §11, §15.9).
//!
//! Three containers, one decision. A room whose meeting has no video
//! records its mixed audio to a **WAV** file, as conference recordings
//! always have, written by forge-conference's own recorder off the
//! mixer's clock. A room with video records the **composite and the
//! mix** to one file: **MP4** (H.264, HEVC or AV1 with Opus, fragmented —
//! the container every player opens, and what a GPU node records on its
//! encoder) or **WebM** (VP8 or VP9 with Opus). This module's task takes
//! coded frames from the video room, resamples the mix to 48 kHz and
//! Opus-encodes it, and muxes both on one timebase; the two writers
//! differ only in the codecs they carry.
//!
//! The two sides start a moment apart and run on different clocks, so
//! each carries the instant its frame was made: the earlier of the two
//! first frames is time zero, audio counts samples from there (drift-free
//! even if the tap lags, which it reports), and video follows the room's
//! own 90 kHz timestamps. That keeps lip sync without a shared clock.
//!
//! Both video containers need Opus, which builds libopus from source, so
//! they are behind the `recording-webm` and `recording-mp4` features.
//! Without them every recording is a WAV, and `recording_format` says so
//! when a recording starts rather than failing later.

use fcp_core::recording::RecordingFormat;
use forge_core::VideoCodec;
use forge_video::frame::Resolution;

/// What the operator asked for in `conference.recording_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatChoice {
    /// MP4 when the meeting has video and this node can encode H.264
    /// (natively or on its device), WebM when it can encode VP8 or VP9
    /// instead, WAV otherwise.
    #[default]
    Auto,
    /// Always audio-only WAV.
    Wav,
    /// Always WebM, with video when the room has any.
    Webm,
    /// Always MP4, with video when the room has any.
    Mp4,
}

impl FormatChoice {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(FormatChoice::Auto),
            "wav" => Some(FormatChoice::Wav),
            "webm" => Some(FormatChoice::Webm),
            "mp4" => Some(FormatChoice::Mp4),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FormatChoice::Auto => "auto",
            FormatChoice::Wav => "wav",
            FormatChoice::Webm => "webm",
            FormatChoice::Mp4 => "mp4",
        }
    }

    /// The video container this choice names outright, `None` for `auto`
    /// and `wav`.
    pub fn container(&self) -> Option<RecordingFormat> {
        match self {
            FormatChoice::Webm => Some(RecordingFormat::Webm),
            FormatChoice::Mp4 => Some(RecordingFormat::Mp4),
            FormatChoice::Auto | FormatChoice::Wav => None,
        }
    }
}

/// Whether this build can write WebM at all.
pub const fn webm_supported() -> bool {
    cfg!(feature = "recording-webm")
}

/// Whether this build can write MP4 at all.
pub const fn mp4_supported() -> bool {
    cfg!(feature = "recording-mp4")
}

/// Whether this build can write `container`.
pub const fn container_supported(container: RecordingFormat) -> bool {
    match container {
        RecordingFormat::Wav => true,
        RecordingFormat::Webm => webm_supported(),
        RecordingFormat::Mp4 => mp4_supported(),
    }
}

/// The video codecs WebM carries, in the order we prefer them.
pub const WEBM_VIDEO_CODECS: [VideoCodec; 2] = [VideoCodec::VP8, VideoCodec::VP9];

/// The video codecs MP4 carries, in the order we prefer them: H.264
/// first, the one every player decodes and every GPU encodes.
pub const MP4_VIDEO_CODECS: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::H265, VideoCodec::AV1];

/// The containers a room with video can be recorded to, in the order
/// `auto` tries them.
const VIDEO_CONTAINERS: [RecordingFormat; 2] = [RecordingFormat::Mp4, RecordingFormat::Webm];

/// The video codecs `container` carries, in preference order; none for
/// WAV.
pub fn codecs_of(container: RecordingFormat) -> &'static [VideoCodec] {
    match container {
        RecordingFormat::Wav => &[],
        RecordingFormat::Webm => &WEBM_VIDEO_CODECS,
        RecordingFormat::Mp4 => &MP4_VIDEO_CODECS,
    }
}

/// Whether `container` carries `codec`.
pub fn container_carries(container: RecordingFormat, codec: VideoCodec) -> bool {
    codecs_of(container).contains(&codec)
}

/// The container a codec is recorded in.
pub fn container_of(codec: VideoCodec) -> RecordingFormat {
    if WEBM_VIDEO_CODECS.contains(&codec) {
        RecordingFormat::Webm
    } else {
        RecordingFormat::Mp4
    }
}

/// The codec's name as `conference.recording_video_codec` spells it.
pub fn codec_config_name(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "hevc",
        VideoCodec::AV1 => "av1",
        VideoCodec::VP8 => "vp8",
        VideoCodec::VP9 => "vp9",
    }
}

/// Parse `conference.recording_video_codec`: `Ok(None)` for `auto`, the
/// codec for a name, `Err` with the offending text otherwise. `h265` is
/// taken as `hevc`, and the SDP spellings are accepted too.
pub fn parse_video_codec(s: &str) -> Result<Option<VideoCodec>, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Ok(None),
        "h264" | "avc" => Ok(Some(VideoCodec::H264)),
        "hevc" | "h265" => Ok(Some(VideoCodec::H265)),
        "av1" => Ok(Some(VideoCodec::AV1)),
        "vp8" => Ok(Some(VideoCodec::VP8)),
        "vp9" => Ok(Some(VideoCodec::VP9)),
        other => Err(other.to_string()),
    }
}

/// The codec to record `container`'s video with: the configured one when
/// the container carries it and this node can encode it, else the first
/// of the container's codecs it can.
pub fn pick_video_codec(
    container: RecordingFormat,
    configured: Option<VideoCodec>,
    usable: &[VideoCodec],
) -> Option<VideoCodec> {
    if let Some(want) = configured {
        if container_carries(container, want) && usable.contains(&want) {
            return Some(want);
        }
    }
    codecs_of(container)
        .iter()
        .find(|c| usable.contains(c))
        .copied()
}

/// The container and video codec for a recording.
///
/// `configured` is `recording_video_codec`, `usable` the codecs this node
/// both decodes and encodes (natively or on its device), `room_video`
/// whether the room has a composite to record. The rules, per §15.9:
///
/// - `wav` is WAV.
/// - `webm` and `mp4` are that container — with the composite when the
///   room has one and the node encodes a codec the container carries,
///   audio-only otherwise — or WAV when this build cannot write it.
/// - `auto` is MP4 when the room has video and the node encodes H.264
///   (or another MP4 codec), WebM when it encodes VP8 or VP9 instead,
///   WAV otherwise. A named codec puts its own container first.
pub fn plan_format(
    choice: FormatChoice,
    configured: Option<VideoCodec>,
    usable: &[VideoCodec],
    room_video: bool,
) -> (RecordingFormat, Option<VideoCodec>) {
    let pick = |container: RecordingFormat| {
        room_video
            .then(|| pick_video_codec(container, configured, usable))
            .flatten()
    };
    match choice {
        FormatChoice::Wav => (RecordingFormat::Wav, None),
        FormatChoice::Webm | FormatChoice::Mp4 => {
            let container = choice.container().expect("a video container");
            if !container_supported(container) {
                return (RecordingFormat::Wav, None);
            }
            (container, pick(container))
        }
        FormatChoice::Auto => {
            if !room_video {
                return (RecordingFormat::Wav, None);
            }
            // The configured codec's container first, then the rest in
            // the standing order.
            let first = configured.map(container_of);
            first
                .into_iter()
                .chain(VIDEO_CONTAINERS.iter().copied())
                .filter(|c| container_supported(*c))
                .find_map(|c| pick(c).map(|codec| (c, Some(codec))))
                .unwrap_or((RecordingFormat::Wav, None))
        }
    }
}

/// What a finished recording came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecordingOutcome {
    pub duration_secs: u64,
    pub size_bytes: u64,
    pub video_frames: u64,
    pub audio_frames: u64,
    /// Frames the recorder was handed too late to keep, and mix frames it
    /// fell behind by. Nothing is wrong with the file; it has gaps.
    pub dropped: u64,
}

/// How a video-container recording (WebM or MP4) is set up.
#[derive(Debug, Clone)]
pub struct RecorderSettings {
    /// Where to write.
    pub path: std::path::PathBuf,
    /// Which container: WebM or MP4 (WAV is forge-conference's own
    /// recorder, not this one).
    pub container: RecordingFormat,
    /// The composite: its codec, and the size and rate to record it at
    /// (the room's own when absent).
    pub video: Option<VideoSettings>,
    /// Opus bitrate for the mix.
    pub audio_bitrate: u32,
}

/// The video half of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub resolution: Option<Resolution>,
    pub max_kbps: Option<u32>,
}

#[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
pub use mux::Recorder;

#[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
mod mux {
    use super::{RecorderSettings, RecordingOutcome};

    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};
    use fcp_core::recording::RecordingFormat;
    use forge_codecs::opus::{OpusApplication, OpusCodec, OpusConfig};
    use forge_codecs::AudioCodec as _;
    use forge_conference::video::{RecordRequest, RecordedFrame, RecordingSink, VideoRoom};
    use forge_conference::{ConferenceRoom, MixedFrame};
    use forge_core::VideoCodec;
    use forge_resampler::Resampler;
    use forge_video::flavor::Flavor;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::{mpsc, Notify};
    use tokio::task::JoinHandle;
    use tracing::{debug, info, warn};

    /// Opus is written at 48 kHz whatever the room mixes at.
    const OPUS_RATE: u32 = 48_000;
    /// One Opus frame: 20 ms.
    const OPUS_FRAME: usize = (OPUS_RATE as usize) / 50;

    /// What a closed file came to, whichever writer wrote it.
    struct Finished {
        duration_ms: u64,
        bytes: u64,
    }

    /// The container writer: WebM or MP4, both taking coded frames and
    /// Opus packets stamped in milliseconds.
    enum Writer {
        #[cfg(feature = "recording-webm")]
        Webm(forge_webm::WebmWriter<std::fs::File>),
        #[cfg(feature = "recording-mp4")]
        Mp4(forge_mp4::Mp4Writer<std::fs::File>),
    }

    impl Writer {
        fn open(
            container: RecordingFormat,
            file: std::fs::File,
            video: Option<&Flavor>,
            audio_bitrate: u32,
        ) -> Result<Self> {
            let _ = audio_bitrate;
            match container {
                #[cfg(feature = "recording-webm")]
                RecordingFormat::Webm => {
                    use forge_webm::{
                        AudioTrack, VideoTrack, WebmConfig, WebmVideoCodec, WebmWriter,
                    };
                    let mut config = WebmConfig::new()
                        .audio(AudioTrack::opus(OPUS_RATE, 1))
                        .writing_app(format!("fcp-conference {}", env!("CARGO_PKG_VERSION")));
                    if let Some(flavor) = video {
                        let codec = match flavor.codec {
                            VideoCodec::VP8 => WebmVideoCodec::Vp8,
                            VideoCodec::VP9 => WebmVideoCodec::Vp9,
                            other => anyhow::bail!("WebM does not carry {}", other.sdp_name()),
                        };
                        config = config.video(
                            VideoTrack::new(
                                codec,
                                flavor.resolution.width,
                                flavor.resolution.height,
                            )
                            .fps(flavor.fps),
                        );
                    }
                    Ok(Writer::Webm(
                        WebmWriter::new(file, config).context("opening the WebM file")?,
                    ))
                }
                #[cfg(feature = "recording-mp4")]
                RecordingFormat::Mp4 => {
                    use forge_mp4::{AudioTrack, Mp4Config, Mp4VideoCodec, Mp4Writer, VideoTrack};
                    let mut config = Mp4Config::new().audio(AudioTrack::opus(OPUS_RATE, 1));
                    if let Some(flavor) = video {
                        let codec = match flavor.codec {
                            VideoCodec::H264 => Mp4VideoCodec::H264,
                            VideoCodec::H265 => Mp4VideoCodec::Hevc,
                            VideoCodec::AV1 => Mp4VideoCodec::Av1,
                            other => anyhow::bail!("MP4 does not carry {} here", other.sdp_name()),
                        };
                        config = config.video(
                            VideoTrack::new(
                                codec,
                                flavor.resolution.width,
                                flavor.resolution.height,
                            )
                            .fps(flavor.fps),
                        );
                    }
                    Ok(Writer::Mp4(
                        Mp4Writer::new(file, config).context("opening the MP4 file")?,
                    ))
                }
                other => anyhow::bail!(
                    "this build cannot write {} (feature `recording-{}`)",
                    other.as_str(),
                    other.as_str()
                ),
            }
        }

        fn write_video(&mut self, ms: u64, keyframe: bool, data: &[u8]) -> Result<()> {
            match self {
                #[cfg(feature = "recording-webm")]
                Writer::Webm(w) => Ok(w.write_video(ms, keyframe, data)?),
                #[cfg(feature = "recording-mp4")]
                Writer::Mp4(w) => Ok(w.write_video(ms, keyframe, data)?),
            }
        }

        fn write_audio(&mut self, ms: u64, data: &[u8]) -> Result<()> {
            match self {
                #[cfg(feature = "recording-webm")]
                Writer::Webm(w) => Ok(w.write_audio(ms, data)?),
                #[cfg(feature = "recording-mp4")]
                Writer::Mp4(w) => Ok(w.write_audio(ms, data)?),
            }
        }

        fn finish(self) -> Result<Finished> {
            match self {
                #[cfg(feature = "recording-webm")]
                Writer::Webm(w) => {
                    let s = w.finish()?;
                    Ok(Finished {
                        duration_ms: s.duration_ms,
                        bytes: s.bytes,
                    })
                }
                #[cfg(feature = "recording-mp4")]
                Writer::Mp4(w) => {
                    let s = w.finish()?;
                    Ok(Finished {
                        duration_ms: s.duration_ms,
                        bytes: s.bytes,
                    })
                }
            }
        }
    }

    /// A running WebM or MP4 recording of one room.
    pub struct Recorder {
        container: RecordingFormat,
        stop: Arc<Notify>,
        task: JoinHandle<RecordingOutcome>,
        /// The video room and the name it knows this recording by, so
        /// stopping releases the composite there and then rather than
        /// waiting for the room to notice a closed channel.
        video: Option<(Arc<VideoRoom>, String)>,
    }

    impl Recorder {
        /// Start recording `room` to the settings' container. The video
        /// room, when the settings ask for video, hands out the composite
        /// as coded frames; the audio comes from the room's mix tap.
        pub fn start(
            id: &str,
            room: &Arc<ConferenceRoom>,
            settings: RecorderSettings,
        ) -> Result<Self> {
            let container = settings.container;
            let name = container.as_str();
            if !super::container_supported(container) || container == RecordingFormat::Wav {
                anyhow::bail!("this build cannot write {name} (feature `recording-{name}`)");
            }
            let sample_rate = room.format().sample_rate;
            let channels = room.format().channels;
            if channels != 1 {
                // The mixer is mono everywhere in FCP; a stereo mix would
                // need the resampler and Opus set up for it.
                anyhow::bail!("{name} recording expects a mono mix, not {channels} channels");
            }

            // The composite, when the room has video.
            let sink = match (&settings.video, room.video()) {
                (Some(v), Some(video_room)) => {
                    if !super::container_carries(container, v.codec) {
                        anyhow::bail!("{name} does not carry {}", v.codec.sdp_name());
                    }
                    let request = RecordRequest {
                        codec: v.codec,
                        resolution: v.resolution,
                        fps: None,
                        max_kbps: v.max_kbps,
                        ..RecordRequest::new(v.codec)
                    };
                    Some(
                        video_room
                            .record(id, request)
                            .context("the video room refused the recording")?,
                    )
                }
                (Some(_), None) => {
                    warn!(recording = %id, "video was asked for but the room has none; recording audio only");
                    None
                }
                (None, _) => None,
            };

            if let Some(parent) = settings.path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            let file = std::fs::File::create(&settings.path)
                .with_context(|| format!("creating {}", settings.path.display()))?;
            let writer = Writer::open(
                container,
                file,
                sink.as_ref().map(|s| &s.flavor),
                settings.audio_bitrate,
            )?;

            let opus = OpusCodec::with_config(OpusConfig {
                sample_rate: OPUS_RATE,
                channels: 1,
                application: OpusApplication::Voip,
                bitrate: settings.audio_bitrate,
                frame_duration_ms: 20,
            })
            .context("no Opus encoder")?;
            let resampler = (sample_rate != OPUS_RATE)
                .then(|| Resampler::new(sample_rate, OPUS_RATE, 1))
                .transpose()
                .context("no resampler for the mix")?;

            let stop = Arc::new(Notify::new());
            let video = sink
                .as_ref()
                .and_then(|_| room.video())
                .map(|room| (room, id.to_string()));
            let task = tokio::spawn(run(
                id.to_string(),
                name,
                room.tap_mix(),
                sink,
                writer,
                opus,
                resampler,
                sample_rate,
                Arc::clone(&stop),
            ));
            info!(
                recording = %id,
                path = %settings.path.display(),
                container = name,
                video = ?settings.video.map(|v| v.codec.sdp_name()),
                "recording started"
            );
            Ok(Self {
                container,
                stop,
                task,
                video,
            })
        }

        /// The container being written.
        pub fn container(&self) -> RecordingFormat {
            self.container
        }

        /// Stop, close the file, and report what it came to.
        pub async fn stop(self) -> RecordingOutcome {
            // `notify_one` and not `notify_waiters`: the task is only
            // parked in its `select!` between frames, and a wakeup sent
            // while it is busy would be dropped on the floor, leaving it
            // waiting for a mix that has stopped coming.
            self.stop.notify_one();
            if let Some((room, id)) = &self.video {
                room.stop_record(id);
            }
            // Bounded: a wedged muxer must not hold up the room's stop,
            // and the file is whatever it flushed.
            let name = self.container.as_str();
            match tokio::time::timeout(STOP_TIMEOUT, self.task).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(e)) => {
                    warn!(error = %e, container = name, "the recorder task ended badly");
                    RecordingOutcome::default()
                }
                Err(_) => {
                    warn!(
                        seconds = STOP_TIMEOUT.as_secs(),
                        container = name,
                        "the recorder did not stop in time; the file may be unfinished"
                    );
                    RecordingOutcome::default()
                }
            }
        }
    }

    /// The muxer loop: audio from the mix tap, video from the room, both
    /// stamped against whichever arrived first.
    #[allow(clippy::too_many_arguments)]
    async fn run(
        id: String,
        container: &'static str,
        mut mix: tokio::sync::broadcast::Receiver<MixedFrame>,
        sink: Option<RecordingSink>,
        mut writer: Writer,
        mut opus: OpusCodec,
        mut resampler: Option<Resampler>,
        sample_rate: u32,
        stop: Arc<Notify>,
    ) -> RecordingOutcome {
        let mut video: Option<mpsc::Receiver<RecordedFrame>> = sink.map(|s| s.frames);
        let mut out = RecordingOutcome::default();

        // Time zero: whichever side produced a frame first.
        let mut zero: Option<Instant> = None;
        // Audio: samples written at 48 kHz since the audio anchor.
        let mut audio_anchor_ms = 0u64;
        let mut audio_samples = 0u64;
        let mut pending: Vec<i16> = Vec::with_capacity(OPUS_FRAME * 2);
        // Video: the room's 90 kHz clock since the video anchor.
        let mut video_anchor_ms = 0u64;
        let mut video_pts0: Option<u32> = None;

        loop {
            tokio::select! {
                _ = stop.notified() => break,

                frame = mix.recv() => match frame {
                    Ok(frame) => {
                        let at = frame.at;
                        let base = *zero.get_or_insert(at);
                        if audio_samples == 0 && pending.is_empty() {
                            audio_anchor_ms = at.saturating_duration_since(base).as_millis() as u64;
                        }
                        let samples = match resampler.as_mut() {
                            Some(r) => match r.resample(&frame.samples) {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(recording = %id, error = %e, "resampling the mix failed");
                                    continue;
                                }
                            },
                            None => frame.samples.as_ref().clone(),
                        };
                        pending.extend_from_slice(&samples);
                        while pending.len() >= OPUS_FRAME {
                            let ms = audio_anchor_ms + audio_samples * 1_000 / OPUS_RATE as u64;
                            let chunk: Vec<i16> = pending.drain(..OPUS_FRAME).collect();
                            match opus.encode(&chunk) {
                                Ok(packet) => {
                                    if let Err(e) = writer.write_audio(ms, &packet) {
                                        warn!(recording = %id, error = %e, "writing audio failed");
                                        out.dropped += 1;
                                    } else {
                                        out.audio_frames += 1;
                                    }
                                }
                                Err(e) => {
                                    warn!(recording = %id, error = %e, "Opus encoding failed");
                                    out.dropped += 1;
                                }
                            }
                            audio_samples += OPUS_FRAME as u64;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        // The tap ran ahead of us: keep the timeline
                        // honest by skipping the audio we missed rather
                        // than letting it slide against the video.
                        let frame_samples = (sample_rate as u64 / 50).max(1);
                        let missed = n * frame_samples * OPUS_RATE as u64 / sample_rate as u64;
                        audio_samples += missed;
                        out.dropped += n;
                        warn!(recording = %id, frames = n, "the recorder fell behind the mix");
                    }
                    Err(RecvError::Closed) => {
                        debug!(recording = %id, "the room's mix ended");
                        break;
                    }
                },

                frame = async {
                    match video.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => match frame {
                    Some(frame) => {
                        let base = *zero.get_or_insert(frame.at);
                        let pts0 = *video_pts0.get_or_insert_with(|| {
                            video_anchor_ms =
                                frame.at.saturating_duration_since(base).as_millis() as u64;
                            frame.frame.timestamp
                        });
                        // The room's clock is 90 kHz and wraps; the delta
                        // is what matters.
                        let delta = frame.frame.timestamp.wrapping_sub(pts0) as u64;
                        let ms = video_anchor_ms + delta / 90;
                        match writer.write_video(ms, frame.frame.keyframe, &frame.frame.data) {
                            Ok(()) => out.video_frames += 1,
                            Err(e) if out.video_frames == 0 => {
                                // The first frame opens the video track
                                // (MP4 lifts its parameter sets from it);
                                // if that cannot be done nothing later
                                // will do it either, so the recording is
                                // its audio. Dropping the receiver tells
                                // the room to stop encoding for us.
                                warn!(
                                    recording = %id,
                                    error = %e,
                                    "the first video frame could not open the track; recording audio only"
                                );
                                out.dropped += 1;
                                video = None;
                            }
                            Err(e) => {
                                warn!(recording = %id, error = %e, "writing video failed");
                                out.dropped += 1;
                            }
                        }
                    }
                    None => {
                        debug!(recording = %id, "the room's composite ended");
                        video = None;
                    }
                },
            }
        }

        match writer.finish() {
            Ok(finished) => {
                out.duration_secs = finished.duration_ms.div_ceil(1_000);
                out.size_bytes = finished.bytes;
                info!(
                    recording = %id,
                    container,
                    seconds = out.duration_secs,
                    bytes = out.size_bytes,
                    video_frames = out.video_frames,
                    audio_frames = out.audio_frames,
                    dropped = out.dropped,
                    "recording finished"
                );
            }
            Err(e) => warn!(recording = %id, container, error = %e, "closing the file failed"),
        }
        out
    }

    /// How long the recorder waits for its task after being told to stop.
    pub const STOP_TIMEOUT: Duration = Duration::from_secs(5);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CHOICES: [FormatChoice; 4] = [
        FormatChoice::Auto,
        FormatChoice::Wav,
        FormatChoice::Webm,
        FormatChoice::Mp4,
    ];

    #[test]
    fn the_format_choice_parses_what_the_config_allows() {
        for c in ALL_CHOICES {
            assert_eq!(FormatChoice::parse(c.as_str()), Some(c));
        }
        assert_eq!(FormatChoice::parse("WebM"), Some(FormatChoice::Webm));
        assert_eq!(FormatChoice::parse("MP4"), Some(FormatChoice::Mp4));
        assert_eq!(FormatChoice::parse("mkv"), None);
        assert_eq!(FormatChoice::Mp4.container(), Some(RecordingFormat::Mp4));
        assert_eq!(FormatChoice::Webm.container(), Some(RecordingFormat::Webm));
        assert_eq!(FormatChoice::Auto.container(), None);
        assert_eq!(FormatChoice::Wav.container(), None);
    }

    #[test]
    fn the_video_codec_setting_names_every_codec_and_auto() {
        assert_eq!(parse_video_codec("auto"), Ok(None));
        assert_eq!(parse_video_codec(""), Ok(None));
        for codec in VideoCodec::ALL {
            assert_eq!(parse_video_codec(codec_config_name(codec)), Ok(Some(codec)));
            assert_eq!(parse_video_codec(codec.sdp_name()), Ok(Some(codec)));
        }
        assert_eq!(parse_video_codec("HEVC"), Ok(Some(VideoCodec::H265)));
        assert_eq!(parse_video_codec("mpeg2"), Err("mpeg2".to_string()));
    }

    #[test]
    fn each_container_carries_its_own_codecs() {
        for codec in WEBM_VIDEO_CODECS {
            assert_eq!(container_of(codec), RecordingFormat::Webm);
            assert!(container_carries(RecordingFormat::Webm, codec));
            assert!(!container_carries(RecordingFormat::Mp4, codec));
        }
        for codec in MP4_VIDEO_CODECS {
            assert_eq!(container_of(codec), RecordingFormat::Mp4);
            assert!(container_carries(RecordingFormat::Mp4, codec));
            assert!(!container_carries(RecordingFormat::Webm, codec));
        }
        assert!(codecs_of(RecordingFormat::Wav).is_empty());
        // Every codec has exactly one home.
        for codec in VideoCodec::ALL {
            assert!(container_carries(container_of(codec), codec));
        }
    }

    #[test]
    fn the_video_codec_is_one_the_container_carries_and_this_node_encodes() {
        let vpx = [VideoCodec::VP8, VideoCodec::VP9];
        assert_eq!(
            pick_video_codec(RecordingFormat::Webm, Some(VideoCodec::VP9), &vpx),
            Some(VideoCodec::VP9)
        );
        // Configured but not compiled in: fall back to what is.
        assert_eq!(
            pick_video_codec(
                RecordingFormat::Webm,
                Some(VideoCodec::VP9),
                &[VideoCodec::VP8]
            ),
            Some(VideoCodec::VP8)
        );
        // H.264 is not a WebM codec, whatever the node can do.
        assert_eq!(
            pick_video_codec(
                RecordingFormat::Webm,
                Some(VideoCodec::H264),
                &[VideoCodec::H264]
            ),
            None
        );
        assert_eq!(
            pick_video_codec(RecordingFormat::Webm, None, &vpx),
            Some(VideoCodec::VP8)
        );
        assert_eq!(pick_video_codec(RecordingFormat::Webm, None, &[]), None);

        // MP4 prefers H.264, takes a named HEVC or AV1, and never VP8.
        assert_eq!(
            pick_video_codec(RecordingFormat::Mp4, None, &VideoCodec::ALL),
            Some(VideoCodec::H264)
        );
        assert_eq!(
            pick_video_codec(
                RecordingFormat::Mp4,
                Some(VideoCodec::AV1),
                &VideoCodec::ALL
            ),
            Some(VideoCodec::AV1)
        );
        assert_eq!(
            pick_video_codec(
                RecordingFormat::Mp4,
                Some(VideoCodec::VP8),
                &VideoCodec::ALL
            ),
            Some(VideoCodec::H264)
        );
        assert_eq!(pick_video_codec(RecordingFormat::Mp4, None, &vpx), None);
    }

    #[test]
    fn wav_is_wav_whatever_the_node_can_do() {
        assert_eq!(
            plan_format(
                FormatChoice::Wav,
                Some(VideoCodec::H264),
                &VideoCodec::ALL,
                true
            ),
            (RecordingFormat::Wav, None)
        );
    }

    #[test]
    fn a_meeting_without_video_records_wav_under_auto() {
        assert_eq!(
            plan_format(FormatChoice::Auto, None, &VideoCodec::ALL, false),
            (RecordingFormat::Wav, None)
        );
    }

    #[test]
    fn a_named_container_is_written_even_without_video() {
        let want = |c: RecordingFormat| {
            if container_supported(c) {
                (c, None)
            } else {
                (RecordingFormat::Wav, None)
            }
        };
        assert_eq!(
            plan_format(FormatChoice::Webm, None, &VideoCodec::ALL, false),
            want(RecordingFormat::Webm)
        );
        assert_eq!(
            plan_format(FormatChoice::Mp4, None, &VideoCodec::ALL, false),
            want(RecordingFormat::Mp4)
        );
        // With video but no codec the container carries: audio only in
        // that container, not a fall to the other one.
        assert_eq!(
            plan_format(FormatChoice::Mp4, None, &[VideoCodec::VP8], true),
            want(RecordingFormat::Mp4)
        );
        assert_eq!(
            plan_format(FormatChoice::Webm, None, &[VideoCodec::H264], true),
            want(RecordingFormat::Webm)
        );
    }

    #[test]
    fn auto_is_mp4_where_h264_is_encodable_and_webm_otherwise() {
        if !(webm_supported() && mp4_supported()) {
            // The rules below need both writers; the feature-poor
            // builds are covered by `a_build_without_a_writer_records_what_it_can`.
            return;
        }
        // A node with everything: MP4 on H.264.
        assert_eq!(
            plan_format(FormatChoice::Auto, None, &VideoCodec::ALL, true),
            (RecordingFormat::Mp4, Some(VideoCodec::H264))
        );
        // A VP8-only build: WebM, as before phase 9.
        assert_eq!(
            plan_format(FormatChoice::Auto, None, &[VideoCodec::VP8], true),
            (RecordingFormat::Webm, Some(VideoCodec::VP8))
        );
        // A named codec picks its container.
        assert_eq!(
            plan_format(
                FormatChoice::Auto,
                Some(VideoCodec::VP9),
                &VideoCodec::ALL,
                true
            ),
            (RecordingFormat::Webm, Some(VideoCodec::VP9))
        );
        assert_eq!(
            plan_format(
                FormatChoice::Auto,
                Some(VideoCodec::AV1),
                &VideoCodec::ALL,
                true
            ),
            (RecordingFormat::Mp4, Some(VideoCodec::AV1))
        );
        // A named codec the node lacks: its container's next, then the
        // standing order.
        assert_eq!(
            plan_format(
                FormatChoice::Auto,
                Some(VideoCodec::VP9),
                &[VideoCodec::VP8, VideoCodec::H264],
                true
            ),
            (RecordingFormat::Webm, Some(VideoCodec::VP8))
        );
        assert_eq!(
            plan_format(
                FormatChoice::Auto,
                Some(VideoCodec::VP9),
                &[VideoCodec::H264],
                true
            ),
            (RecordingFormat::Mp4, Some(VideoCodec::H264))
        );
        // Nothing encodable: WAV.
        assert_eq!(
            plan_format(FormatChoice::Auto, None, &[], true),
            (RecordingFormat::Wav, None)
        );
    }

    #[test]
    fn a_build_without_a_writer_records_what_it_can() {
        let (format, codec) = plan_format(FormatChoice::Auto, None, &VideoCodec::ALL, true);
        match (mp4_supported(), webm_supported()) {
            (true, _) => assert_eq!(
                (format, codec),
                (RecordingFormat::Mp4, Some(VideoCodec::H264))
            ),
            (false, true) => {
                assert_eq!(
                    (format, codec),
                    (RecordingFormat::Webm, Some(VideoCodec::VP8))
                )
            }
            (false, false) => assert_eq!((format, codec), (RecordingFormat::Wav, None)),
        }
        if !mp4_supported() {
            assert_eq!(
                plan_format(FormatChoice::Mp4, None, &VideoCodec::ALL, true),
                (RecordingFormat::Wav, None)
            );
        }
        if !webm_supported() {
            assert_eq!(
                plan_format(FormatChoice::Webm, None, &VideoCodec::ALL, true),
                (RecordingFormat::Wav, None)
            );
        }
    }
}
