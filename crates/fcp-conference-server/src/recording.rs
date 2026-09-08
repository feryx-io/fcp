//! Recording a conference room (design §11).
//!
//! Two shapes, one decision. A room whose meeting has no video records
//! its mixed audio to a **WAV** file, as conference recordings always
//! have, written by forge-conference's own recorder off the mixer's
//! clock. A room with video records the **composite and the mix** to one
//! **WebM** file: this module's task takes coded VP8 (or VP9) frames from
//! the video room, resamples the mix to 48 kHz and Opus-encodes it, and
//! muxes both on one timebase.
//!
//! The two sides start a moment apart and run on different clocks, so
//! each carries the instant its frame was made: the earlier of the two
//! first frames is time zero, audio counts samples from there (drift-free
//! even if the tap lags, which it reports), and video follows the room's
//! own 90 kHz timestamps. That keeps lip sync without a shared clock.
//!
//! WebM needs Opus, which builds libopus from source, so it is behind the
//! `recording-webm` feature. Without it every recording is a WAV, and
//! `recording_format = "auto"` says so at startup rather than failing
//! later.

use fcp_core::recording::RecordingFormat;
use forge_core::VideoCodec;
use forge_video::frame::Resolution;

/// What the operator asked for in `conference.recording_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatChoice {
    /// WebM when the meeting has video and this node can encode for it,
    /// WAV otherwise.
    #[default]
    Auto,
    /// Always audio-only WAV.
    Wav,
    /// Always WebM, with video when the room has any.
    Webm,
}

impl FormatChoice {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(FormatChoice::Auto),
            "wav" => Some(FormatChoice::Wav),
            "webm" => Some(FormatChoice::Webm),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FormatChoice::Auto => "auto",
            FormatChoice::Wav => "wav",
            FormatChoice::Webm => "webm",
        }
    }
}

/// Whether this build can write WebM at all.
pub const fn webm_supported() -> bool {
    cfg!(feature = "recording-webm")
}

/// The video codecs WebM carries, in the order we prefer them.
pub const WEBM_VIDEO_CODECS: [VideoCodec; 2] = [VideoCodec::VP8, VideoCodec::VP9];

/// The codec to record video with: the configured one when this node can
/// encode it, else the first WebM codec it can.
pub fn pick_video_codec(
    configured: Option<VideoCodec>,
    usable: &[VideoCodec],
) -> Option<VideoCodec> {
    if let Some(want) = configured {
        if WEBM_VIDEO_CODECS.contains(&want) && usable.contains(&want) {
            return Some(want);
        }
    }
    WEBM_VIDEO_CODECS
        .iter()
        .find(|c| usable.contains(c))
        .copied()
}

/// The container and video codec for a recording.
///
/// `room_video` is the codec the node would record the composite with,
/// `None` when the room has no video or nothing here can encode for WebM.
pub fn plan_format(
    choice: FormatChoice,
    room_video: Option<VideoCodec>,
) -> (RecordingFormat, Option<VideoCodec>) {
    if !webm_supported() || choice == FormatChoice::Wav {
        return (RecordingFormat::Wav, None);
    }
    match choice {
        FormatChoice::Webm => (RecordingFormat::Webm, room_video),
        // A meeting with no video, or a node that cannot encode for WebM,
        // records what it always did.
        FormatChoice::Auto => match room_video {
            Some(codec) => (RecordingFormat::Webm, Some(codec)),
            None => (RecordingFormat::Wav, None),
        },
        FormatChoice::Wav => unreachable!("handled above"),
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

/// How the WebM recorder is set up.
#[derive(Debug, Clone)]
pub struct WebmSettings {
    /// Where to write.
    pub path: std::path::PathBuf,
    /// The composite: its codec, and the size and rate to record it at
    /// (the room's own when absent).
    pub video: Option<WebmVideoSettings>,
    /// Opus bitrate for the mix.
    pub audio_bitrate: u32,
}

/// The video half of a WebM recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebmVideoSettings {
    pub codec: VideoCodec,
    pub resolution: Option<Resolution>,
    pub max_kbps: Option<u32>,
}

#[cfg(feature = "recording-webm")]
pub use webm::WebmRecorder;

#[cfg(feature = "recording-webm")]
mod webm {
    use super::{RecordingOutcome, WebmSettings};

    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};
    use forge_codecs::opus::{OpusApplication, OpusCodec, OpusConfig};
    use forge_codecs::AudioCodec as _;
    use forge_conference::video::{RecordRequest, RecordedFrame, RecordingSink, VideoRoom};
    use forge_conference::{ConferenceRoom, MixedFrame};
    use forge_core::VideoCodec;
    use forge_resampler::Resampler;
    use forge_webm::{AudioTrack, VideoTrack, WebmConfig, WebmVideoCodec, WebmWriter};
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::{mpsc, Notify};
    use tokio::task::JoinHandle;
    use tracing::{debug, info, warn};

    /// Opus is written at 48 kHz whatever the room mixes at.
    const OPUS_RATE: u32 = 48_000;
    /// One Opus frame: 20 ms.
    const OPUS_FRAME: usize = (OPUS_RATE as usize) / 50;

    /// A running WebM recording of one room.
    pub struct WebmRecorder {
        stop: Arc<Notify>,
        task: JoinHandle<RecordingOutcome>,
        /// The video room and the name it knows this recording by, so
        /// stopping releases the composite there and then rather than
        /// waiting for the room to notice a closed channel.
        video: Option<(Arc<VideoRoom>, String)>,
    }

    impl WebmRecorder {
        /// Start recording `room` to a WebM file. The video room, when
        /// the settings ask for video, hands out the composite as coded
        /// frames; the audio comes from the room's mix tap.
        pub fn start(id: &str, room: &Arc<ConferenceRoom>, settings: WebmSettings) -> Result<Self> {
            let sample_rate = room.format().sample_rate;
            let channels = room.format().channels;
            if channels != 1 {
                // The mixer is mono everywhere in FCP; a stereo mix would
                // need the resampler and Opus set up for it.
                anyhow::bail!("WebM recording expects a mono mix, not {channels} channels");
            }

            // The composite, when the room has video.
            let sink = match (&settings.video, room.video()) {
                (Some(v), Some(video_room)) => {
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

            let mut config = WebmConfig::new()
                .audio(AudioTrack::opus(OPUS_RATE, 1))
                .writing_app(format!("fcp-conference {}", env!("CARGO_PKG_VERSION")));
            if let Some(sink) = &sink {
                let codec = webm_codec(sink.flavor.codec).context("codec WebM cannot carry")?;
                config = config.video(
                    VideoTrack::new(
                        codec,
                        sink.flavor.resolution.width,
                        sink.flavor.resolution.height,
                    )
                    .fps(sink.flavor.fps),
                );
            }
            let writer = WebmWriter::new(file, config).context("opening the WebM file")?;

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
                video = settings.video.is_some(),
                "WebM recording started"
            );
            Ok(Self { stop, task, video })
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
            match tokio::time::timeout(STOP_TIMEOUT, self.task).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(e)) => {
                    warn!(error = %e, "the WebM recorder task ended badly");
                    RecordingOutcome::default()
                }
                Err(_) => {
                    warn!(
                        seconds = STOP_TIMEOUT.as_secs(),
                        "the WebM recorder did not stop in time; the file may be unfinished"
                    );
                    RecordingOutcome::default()
                }
            }
        }
    }

    fn webm_codec(codec: VideoCodec) -> Option<WebmVideoCodec> {
        match codec {
            VideoCodec::VP8 => Some(WebmVideoCodec::Vp8),
            VideoCodec::VP9 => Some(WebmVideoCodec::Vp9),
            _ => None,
        }
    }

    /// The muxer loop: audio from the mix tap, video from the room, both
    /// stamped against whichever arrived first.
    #[allow(clippy::too_many_arguments)]
    async fn run(
        id: String,
        mut mix: tokio::sync::broadcast::Receiver<MixedFrame>,
        sink: Option<RecordingSink>,
        mut writer: WebmWriter<std::fs::File>,
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
            Ok(stats) => {
                out.duration_secs = stats.duration_ms.div_ceil(1_000);
                out.size_bytes = stats.bytes;
                info!(
                    recording = %id,
                    seconds = out.duration_secs,
                    bytes = out.size_bytes,
                    video_frames = out.video_frames,
                    audio_frames = out.audio_frames,
                    dropped = out.dropped,
                    "WebM recording finished"
                );
            }
            Err(e) => warn!(recording = %id, error = %e, "closing the WebM file failed"),
        }
        out
    }

    /// How long the recorder waits for its task after being told to stop.
    pub const STOP_TIMEOUT: Duration = Duration::from_secs(5);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_choice_parses_what_the_config_allows() {
        for c in [FormatChoice::Auto, FormatChoice::Wav, FormatChoice::Webm] {
            assert_eq!(FormatChoice::parse(c.as_str()), Some(c));
        }
        assert_eq!(FormatChoice::parse("WebM"), Some(FormatChoice::Webm));
        assert_eq!(FormatChoice::parse("mp4"), None);
    }

    #[test]
    fn the_video_codec_is_one_webm_carries_and_this_node_encodes() {
        let both = [VideoCodec::VP8, VideoCodec::VP9];
        assert_eq!(
            pick_video_codec(Some(VideoCodec::VP9), &both),
            Some(VideoCodec::VP9)
        );
        // Configured but not compiled in: fall back to what is.
        assert_eq!(
            pick_video_codec(Some(VideoCodec::VP9), &[VideoCodec::VP8]),
            Some(VideoCodec::VP8)
        );
        // H.264 is not a WebM codec, whatever the node can do.
        assert_eq!(
            pick_video_codec(Some(VideoCodec::H264), &[VideoCodec::H264]),
            None
        );
        assert_eq!(pick_video_codec(None, &both), Some(VideoCodec::VP8));
        assert_eq!(pick_video_codec(None, &[]), None);
    }

    #[test]
    fn the_container_follows_the_choice_and_what_the_room_has() {
        // Without the feature everything is a WAV, whatever is asked.
        if !webm_supported() {
            for choice in [FormatChoice::Auto, FormatChoice::Wav, FormatChoice::Webm] {
                assert_eq!(
                    plan_format(choice, Some(VideoCodec::VP8)),
                    (RecordingFormat::Wav, None)
                );
            }
            return;
        }
        assert_eq!(
            plan_format(FormatChoice::Auto, Some(VideoCodec::VP8)),
            (RecordingFormat::Webm, Some(VideoCodec::VP8))
        );
        // A meeting with no video, or a node that cannot encode: WAV.
        assert_eq!(
            plan_format(FormatChoice::Auto, None),
            (RecordingFormat::Wav, None)
        );
        // Asked for outright, WebM even without video.
        assert_eq!(
            plan_format(FormatChoice::Webm, None),
            (RecordingFormat::Webm, None)
        );
        assert_eq!(
            plan_format(FormatChoice::Wav, Some(VideoCodec::VP8)),
            (RecordingFormat::Wav, None)
        );
    }
}
