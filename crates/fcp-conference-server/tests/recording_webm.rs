//! Recording a room to WebM, end to end: the room's mix and its
//! composite go through the resampler, the Opus encoder and the muxer,
//! and the file that comes out says what it should.
//!
//! Needs the `recording-webm` feature, which brings the Opus encoder in.

#![cfg(feature = "recording-webm")]

use std::sync::Arc;
use std::time::Duration;

use fcp_conference_server::recording::{Recorder, RecorderSettings, VideoSettings};
use fcp_core::recording::RecordingFormat;
use forge_conference::video::{VideoBackend, VideoRoomSettings};
use forge_conference::{AudioFormat, ConferenceRoom};
use forge_core::{AudioCodec, VideoCodec};
use forge_video::frame::Resolution;
use forge_video::layout::Layout;
use forge_webm::{read_summary_file, TrackKind, AUDIO_TRACK, VIDEO_TRACK};

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
async fn an_audio_only_recording_is_opus_in_webm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audio.webm");
    let room = room("rec-audio");
    room.add_participant("alice", true).unwrap();

    let recorder = Recorder::start(
        "rec-1",
        &room,
        RecorderSettings {
            path: path.clone(),
            container: RecordingFormat::Webm,
            video: None,
            audio_bitrate: 32_000,
            hls_segment_ms: None,
        },
    )
    .expect("the recorder starts");

    speak(&room, "alice", 50).await; // one second
    let outcome = recorder.stop().await;

    assert!(outcome.audio_frames >= 40, "{outcome:?}");
    assert_eq!(outcome.video_frames, 0);
    assert!(outcome.size_bytes > 0);
    assert_eq!(outcome.dropped, 0, "{outcome:?}");

    let summary = read_summary_file(&path).unwrap();
    assert_eq!(summary.doc_type, "webm");
    assert_eq!(summary.tracks.len(), 1);
    let audio = summary.track(TrackKind::Audio).expect("an audio track");
    assert_eq!(audio.codec_id, "A_OPUS");
    assert_eq!(audio.channels, 1);
    // Opus always decodes at 48 kHz, whatever the room mixed at.
    assert_eq!(audio.sample_rate, 48_000.0);
    assert_eq!(&audio.codec_private[..8], b"OpusHead");

    let blocks = summary.blocks_of(AUDIO_TRACK);
    assert_eq!(blocks.len(), outcome.audio_frames as usize);
    assert!(blocks.iter().all(|b| b.keyframe));
    assert!(blocks.iter().all(|b| b.bytes > 0), "every frame encoded");
    // 20 ms apart, from zero.
    assert_eq!(blocks[0].ms, 0);
    assert_eq!(blocks[1].ms, 20);
    assert_eq!(blocks[10].ms, 200);
    assert!(summary.duration_ms >= 780.0, "{}", summary.duration_ms);
    assert!(summary.segment_size.is_some(), "the file was finished");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_room_with_video_records_the_composite_beside_the_mix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("room.webm");
    let room = room("rec-video");
    room.add_participant("alice", true).unwrap();
    room.add_participant("bob", false).unwrap();
    // The raw test codec stands in for VP8: nothing here decodes the
    // video, and what matters is that the composite reaches the file.
    let video = room.enable_video(
        VideoRoomSettings {
            layout: Layout::Grid,
            resolution: Resolution::new(128, 72),
            fps: 15,
            codecs: vec![VideoCodec::VP8],
            ..VideoRoomSettings::default()
        },
        &VideoBackend::raw(),
    );

    let recorder = Recorder::start(
        "rec-1",
        &room,
        RecorderSettings {
            path: path.clone(),
            container: RecordingFormat::Webm,
            video: Some(VideoSettings {
                codec: VideoCodec::VP8,
                resolution: None,
                max_kbps: None,
            }),
            audio_bitrate: 32_000,
            hls_segment_ms: None,
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

    let summary = read_summary_file(&path).unwrap();
    assert_eq!(summary.tracks.len(), 2);
    let track = summary.track(TrackKind::Video).expect("a video track");
    assert_eq!(track.codec_id, "V_VP8");
    assert_eq!((track.width, track.height), (128, 72));
    assert!(summary.track(TrackKind::Audio).is_some());

    let video_blocks = summary.blocks_of(VIDEO_TRACK);
    assert_eq!(video_blocks.len(), outcome.video_frames as usize);
    assert!(video_blocks[0].keyframe, "a recording starts on a keyframe");
    // Time zero is whichever side produced a frame first — the mix here,
    // since the composite only lands on the video clock's next tick — so
    // the first video block sits a tick in, not at the room's own pts.
    assert!(
        video_blocks[0].ms < 200,
        "video starts near zero, not at {} ms",
        video_blocks[0].ms
    );
    let mut last = i64::MIN;
    for b in &video_blocks {
        assert!(b.ms >= last, "video timestamps advance: {b:?}");
        last = b.ms;
    }
    // Audio and video are on one timeline: neither runs away from the
    // other by more than a moment.
    let audio_end = summary.blocks_of(AUDIO_TRACK).last().unwrap().ms;
    let video_end = video_blocks.last().unwrap().ms;
    assert!(
        (audio_end - video_end).abs() < 400,
        "audio ended at {audio_end} ms, video at {video_end} ms"
    );
    assert!(!summary.cues.is_empty(), "a keyframe cue");
    assert!(summary.cues.iter().all(|c| c.points_at_cluster));
}
