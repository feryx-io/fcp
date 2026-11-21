//! Recording a room to MP4, end to end (design §15.9): the room's mix
//! and its composite go through the resampler, the Opus encoder and the
//! fragmented-MP4 writer, and the file that comes out says what it
//! should.
//!
//! Needs the `recording-mp4` feature, which brings the Opus encoder in.
//! The video half needs real H.264 frames — the writer lifts its
//! parameter sets from the first keyframe, and the raw test codec has
//! none — so it runs only with `video-openh264` as well.

#![cfg(feature = "recording-mp4")]

use std::sync::Arc;
use std::time::Duration;

use fcp_conference_server::recording::{Recorder, RecorderSettings};
use fcp_core::recording::RecordingFormat;
use forge_conference::{AudioFormat, ConferenceRoom};
use forge_core::AudioCodec;
use forge_mp4::{read_summary_file, TrackKind, AUDIO_TRACK};

/// A room mixing at 8 kHz, as a conference node does, on a frame clock so
/// the mix tap is fed a frame at a time.
fn room(id: &str) -> Arc<ConferenceRoom> {
    Arc::new(
        ConferenceRoom::new(
            id,
            AudioFormat::new(8_000, 1, AudioCodec::PCM),
            160,
            forge_mixer::MixerOptions {
                frame_clock: true,
                ..Default::default()
            },
        )
        .unwrap(),
    )
}

/// Speak into the room for `frames` frames of 20 ms, driving its clock.
async fn speak(room: &Arc<ConferenceRoom>, participant: &str, frames: usize) {
    for i in 0..frames {
        // A quiet sawtooth, so the encoder has something to chew on.
        let samples: Vec<i16> = (0..160)
            .map(|n| (((n + i * 160) % 160) as i16 - 80) * 100)
            .collect();
        room.write_audio(participant, &samples).unwrap();
        room.advance_frame();
        // Let the mix tap and the video clock run.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_audio_only_recording_is_opus_in_mp4() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audio.mp4");
    let room = room("rec-audio");
    room.add_participant("alice", true).unwrap();

    let recorder = Recorder::start(
        "rec-1",
        &room,
        RecorderSettings {
            path: path.clone(),
            container: RecordingFormat::Mp4,
            video: None,
            audio_bitrate: 32_000,
            hls_segment_ms: None,
            hls_mirror: None,
        },
    )
    .expect("the recorder starts");
    assert_eq!(recorder.container(), RecordingFormat::Mp4);

    speak(&room, "alice", 50).await; // one second
    let outcome = recorder.stop().await;

    assert!(outcome.audio_frames >= 40, "{outcome:?}");
    assert_eq!(outcome.video_frames, 0);
    assert!(outcome.size_bytes > 0);
    assert_eq!(outcome.dropped, 0, "{outcome:?}");

    let summary = read_summary_file(&path).unwrap();
    assert!(
        summary.brands.iter().any(|b| b == "isom"),
        "{:?}",
        summary.brands
    );
    assert_eq!(summary.tracks.len(), 1);
    let audio = summary.track(TrackKind::Audio).expect("an audio track");
    assert_eq!(audio.codec, "Opus");
    assert_eq!(audio.channels, 1);
    assert!(!audio.config.is_empty(), "a dOps record");
    assert!(!summary.fragments.is_empty(), "at least one fragment");
    let samples: u32 = summary
        .fragments
        .iter()
        .filter_map(|f| f.traf(AUDIO_TRACK))
        .map(|t| t.samples)
        .sum();
    assert_eq!(samples as u64, outcome.audio_frames);
    // 20 ms apart, from zero.
    let first = summary.fragments[0].traf(AUDIO_TRACK).unwrap();
    assert_eq!(first.base_time, 0);
    assert!(first.sizes.iter().all(|s| *s > 0), "every frame encoded");
    assert!(summary.duration_ms >= 780, "{}", summary.duration_ms);
    // A clean stop patches the movie's duration into the header (it is
    // zero while the file is being written); the seek index (`mfra`)
    // covers video keyframes, so an audio-only file has none.
    assert!(summary.duration_ms <= 1_100, "{}", summary.duration_ms);
    assert!(!summary.mfra);
}

/// The files of an HLS stream's directory, as a player would fetch them:
/// the playlist, then the init segment and each segment it names, joined.
fn fetch_stream(dir: &std::path::Path) -> (String, Vec<u8>, Vec<u64>) {
    let playlist = std::fs::read_to_string(dir.join("index.m3u8")).expect("a playlist");
    let mut whole = std::fs::read(dir.join("init.mp4")).expect("an init segment");
    let mut durations = Vec::new();
    let mut lines = playlist.lines();
    while let Some(line) = lines.next() {
        if let Some(d) = line.strip_prefix("#EXTINF:") {
            let secs: f64 = d.trim_end_matches(',').parse().unwrap();
            durations.push((secs * 1000.0).round() as u64);
            let name = lines.next().expect("a segment after its EXTINF");
            whole.extend(std::fs::read(dir.join(name)).expect("the segment"));
        }
    }
    (playlist, whole, durations)
}

/// A webinar streamed as HLS (`docs/WEBINARS.md` W6.2), audio alone: the
/// recorder cuts the mix into segments of the target, and the playlist ends
/// when the stream stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_audio_only_stream_is_opus_in_cmaf_segments() {
    let dir = tempfile::tempdir().unwrap();
    let stream = dir.path().join("sitting-1");
    let room = room("hls-audio");
    room.add_participant("alice", true).unwrap();
    let recorder = Recorder::start(
        "hls-1",
        &room,
        RecorderSettings {
            path: stream.clone(),
            container: RecordingFormat::Mp4,
            video: None,
            audio_bitrate: 32_000,
            hls_segment_ms: Some(1_000),
            hls_mirror: None,
        },
    )
    .expect("the stream starts");
    speak(&room, "alice", 175).await; // three and a half seconds
    let live = std::fs::read_to_string(stream.join("index.m3u8")).expect("a live playlist");
    assert!(!live.contains("#EXT-X-ENDLIST"), "{live}");
    let outcome = recorder.stop().await;
    assert!(outcome.audio_frames >= 150, "{outcome:?}");

    let (playlist, whole, durations) = fetch_stream(&stream);
    assert!(playlist.ends_with("#EXT-X-ENDLIST\n"), "{playlist}");
    assert!(durations.len() >= 3, "{playlist}");
    for d in &durations[..durations.len() - 1] {
        assert_eq!(*d, 1_000, "{playlist}");
    }
    let summary = forge_mp4::read_summary(&whole).unwrap();
    assert_eq!(summary.fragments.len(), durations.len());
    let audio = summary.track(TrackKind::Audio).expect("an audio track");
    assert_eq!(audio.codec, "Opus");
    let samples: u32 = summary
        .fragments
        .iter()
        .filter_map(|f| f.traf(AUDIO_TRACK))
        .map(|t| t.samples)
        .sum();
    assert_eq!(samples as u64, outcome.audio_frames);
}

#[cfg(feature = "video-openh264")]
mod video {
    use super::*;

    use fcp_conference_server::recording::VideoSettings;
    use forge_conference::video::{CodecPool, VideoBackend, VideoRoomSettings};
    use forge_core::VideoCodec;
    use forge_mp4::VIDEO_TRACK;
    use forge_video::frame::Resolution;
    use forge_video::layout::Layout;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_room_with_video_records_h264_beside_the_mix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("room.mp4");
        let room = room("rec-video");
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        // OpenH264 encodes the composite for real: the writer needs the
        // SPS and PPS off the first keyframe for its `avcC`.
        let backend = VideoBackend::new(forge_video_codecs::default_registry(), CodecPool::new(2));
        let video = room.enable_video(
            VideoRoomSettings {
                layout: Layout::Grid,
                resolution: Resolution::new(320, 180),
                fps: 15,
                codecs: vec![VideoCodec::H264],
                ..VideoRoomSettings::default()
            },
            &backend,
        );

        let recorder = Recorder::start(
            "rec-1",
            &room,
            RecorderSettings {
                path: path.clone(),
                container: RecordingFormat::Mp4,
                video: Some(VideoSettings {
                    codec: VideoCodec::H264,
                    resolution: None,
                    max_kbps: None,
                }),
                audio_bitrate: 32_000,
                hls_segment_ms: None,
                hls_mirror: None,
            },
        )
        .expect("the recorder starts");

        // The recording is a consumer of the composite, not a participant.
        assert!(video.is_recording("rec-1"));
        assert_eq!(video.participants().len(), 2);

        speak(&room, "alice", 50).await;
        let outcome = recorder.stop().await;
        assert!(
            !video.is_recording("rec-1"),
            "the sink went with the recorder"
        );

        assert!(outcome.video_frames >= 5, "{outcome:?}");
        assert!(outcome.audio_frames >= 40, "{outcome:?}");
        assert_eq!(outcome.dropped, 0, "{outcome:?}");

        let summary = read_summary_file(&path).unwrap();
        assert_eq!(summary.tracks.len(), 2);
        let track = summary.track(TrackKind::Video).expect("a video track");
        assert_eq!(track.codec, "avc1");
        assert_eq!((track.width, track.height), (320, 180));
        // An `avcC`: version 1, then the profile the encoder chose.
        assert!(track.config.len() > 7, "{:?}", track.config);
        assert_eq!(track.config[0], 1);
        assert!(summary.track(TrackKind::Audio).is_some());

        assert_eq!(summary.video_samples(), outcome.video_frames);
        let first = summary.fragments[0]
            .traf(VIDEO_TRACK)
            .expect("video in the first fragment");
        assert!(first.keyframes >= 1, "a recording starts on a keyframe");
        // Time zero is whichever side produced a frame first — the mix
        // here, since the composite only lands on the video clock's next
        // tick — so the first video sample sits a tick in, not at the
        // room's own pts.
        assert!(
            first.base_time < 200,
            "video starts near zero, not at {} ms",
            first.base_time
        );
        // Fragments run forward on both tracks.
        let mut last = 0u64;
        for f in &summary.fragments {
            for t in &f.trafs {
                assert!(
                    t.base_time >= last.saturating_sub(100),
                    "fragments advance: {t:?}"
                );
            }
            if let Some(t) = f.traf(VIDEO_TRACK) {
                last = t.base_time;
            }
        }
        // Audio and video are on one timeline: neither runs away from the
        // other by more than a moment.
        let end = |track| {
            summary
                .fragments
                .iter()
                .filter_map(|f| f.traf(track))
                .map(|t| t.base_time + t.duration)
                .max()
                .unwrap()
        };
        let (audio_end, video_end) = (end(AUDIO_TRACK) as i64, end(VIDEO_TRACK) as i64);
        assert!(
            (audio_end - video_end).abs() < 400,
            "audio ended at {audio_end} ms, video at {video_end} ms"
        );
        assert!(summary.mfra, "a clean stop writes the random-access index");
        assert!(summary.access_points >= 1, "a keyframe to seek to");
    }

    /// A video webinar streamed as HLS: the room keys every ten seconds,
    /// and the stream asks for a keyframe each segment, so it is cut about
    /// every two, every segment starting on one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stream_asks_for_a_keyframe_each_segment() {
        let dir = tempfile::tempdir().unwrap();
        let stream = dir.path().join("sitting-v");
        let room = room("hls-video");
        room.add_participant("alice", true).unwrap();
        let backend = VideoBackend::new(forge_video_codecs::default_registry(), CodecPool::new(2));
        room.enable_video(
            VideoRoomSettings {
                layout: Layout::Grid,
                resolution: Resolution::new(320, 180),
                fps: 15,
                codecs: vec![VideoCodec::H264],
                keyframe_interval: Duration::from_secs(10),
                ..VideoRoomSettings::default()
            },
            &backend,
        );
        let recorder = Recorder::start(
            "hls-v",
            &room,
            RecorderSettings {
                path: stream.clone(),
                container: RecordingFormat::Mp4,
                video: Some(VideoSettings {
                    codec: VideoCodec::H264,
                    resolution: None,
                    max_kbps: None,
                }),
                audio_bitrate: 32_000,
                hls_segment_ms: Some(2_000),
                hls_mirror: None,
            },
        )
        .expect("the stream starts");
        speak(&room, "alice", 350).await; // seven seconds
        let outcome = recorder.stop().await;
        assert!(outcome.video_frames >= 60, "{outcome:?}");

        let (playlist, whole, durations) = fetch_stream(&stream);
        assert!(playlist.ends_with("#EXT-X-ENDLIST\n"), "{playlist}");
        assert!(
            durations.len() >= 3,
            "cut about every two seconds, not at the room's ten: {playlist}"
        );
        for d in &durations[..durations.len() - 1] {
            assert!((1_800..=2_600).contains(d), "{playlist}");
        }
        let summary = forge_mp4::read_summary(&whole).unwrap();
        let track = summary.track(TrackKind::Video).expect("a video track");
        assert_eq!(track.codec, "avc1");
        for f in &summary.fragments {
            let v = f.traf(VIDEO_TRACK).expect("video in every segment");
            assert!(
                v.keyframes >= 1,
                "every segment starts on a keyframe: {v:?}"
            );
        }
        assert_eq!(summary.video_samples(), outcome.video_frames);
    }
}

/// On the box (design §15.9, block 9b): a room placed on the GPU named
/// by `FCP_TEST_VIDEO_DEVICE` records to an MP4 whose H.264 came off
/// NVENC, and — when `ffprobe` is on the machine — the file probes as
/// `h264` and `opus`. Skips without the variable or when the device does
/// not open. Run through `deploy/gcp-gpu/gpu-box test`.
#[cfg(feature = "video-hwaccel")]
mod device {
    use super::*;

    use fcp_conference_server::recording::VideoSettings;
    use forge_conference::video::{CodecPool, VideoBackend, VideoRoomSettings};
    use forge_core::VideoCodec;
    use forge_mp4::VIDEO_TRACK;
    use forge_video::frame::{MediaDevice, Resolution};
    use forge_video::layout::Layout;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gpu_room_records_h264_off_its_encoder() {
        let Some(device) = std::env::var("FCP_TEST_VIDEO_DEVICE")
            .ok()
            .and_then(|d| MediaDevice::parse(&d))
            .filter(|d| !d.is_host())
        else {
            eprintln!("FCP_TEST_VIDEO_DEVICE unset: skipping");
            return;
        };
        let mut registry = forge_video_codecs::default_registry();
        let Some(hw) = forge_video_codecs::open_device(&mut registry, &device) else {
            eprintln!("{device} did not open: skipping");
            return;
        };
        let backend = VideoBackend::on_device(registry, CodecPool::new(2), hw);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gpu.mp4");
        let room = room("rec-gpu");
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        let video = room.enable_video(
            VideoRoomSettings {
                layout: Layout::Grid,
                resolution: Resolution::new(1280, 720),
                fps: 15,
                codecs: vec![VideoCodec::H264],
                ..VideoRoomSettings::default()
            },
            &backend,
        );

        let recorder = Recorder::start(
            "rec-1",
            &room,
            RecorderSettings {
                path: path.clone(),
                container: RecordingFormat::Mp4,
                video: Some(VideoSettings {
                    codec: VideoCodec::H264,
                    resolution: None,
                    max_kbps: None,
                }),
                audio_bitrate: 64_000,
                hls_segment_ms: None,
                hls_mirror: None,
            },
        )
        .expect("the recorder starts");
        speak(&room, "alice", 100).await; // two seconds
        let outcome = recorder.stop().await;
        eprintln!("{device}: {outcome:?}");
        assert!(outcome.video_frames >= 15, "{outcome:?}");
        assert!(outcome.audio_frames >= 80, "{outcome:?}");
        assert_eq!(outcome.dropped, 0, "{outcome:?}");
        drop(video);

        let summary = read_summary_file(&path).unwrap();
        let track = summary.track(TrackKind::Video).expect("a video track");
        assert_eq!(track.codec, "avc1");
        assert_eq!((track.width, track.height), (1280, 720));
        assert_eq!(summary.video_samples(), outcome.video_frames);
        assert!(summary.fragments[0].traf(VIDEO_TRACK).is_some());
        assert!(summary.mfra);

        // What a player sees, when there is one to ask.
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .output();
        match probe {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let codecs: Vec<&str> = text.lines().map(str::trim).collect();
                eprintln!("ffprobe: {codecs:?}");
                assert!(codecs.contains(&"h264"), "{codecs:?}");
                assert!(codecs.contains(&"opus"), "{codecs:?}");
            }
            Ok(out) => panic!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr)),
            Err(e) => {
                eprintln!("no ffprobe here ({e}): the container is checked, the decode is not")
            }
        }
    }
}
