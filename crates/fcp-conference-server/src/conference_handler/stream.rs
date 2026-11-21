//! A webinar streamed out as HLS (`docs/WEBINARS.md` §2.8, W6.2).
//!
//! A webinar whose meeting asks for it (`streaming = "hls"`) is streamed by
//! its stage from the moment it goes live until it ends: never in practice,
//! never from an edge. The stream is the recorder's composite and mix, cut
//! into CMAF segments in a directory named for the sitting under
//! `[conference.hls] dir`. The node serves the directory at
//! `/hls/{sitting}/{file}`. When the webinar ends, its playlist ends too and
//! stays, the stream's own replay.
//!
//! A stream is a recording (W6.3a): a `stream` row naming its directory is
//! written before its first segment, and none is streamed without one. The
//! room hears the tenant's recording announcement first, unless the
//! recording that started as it went live already announced it. Retention,
//! holds, deletion and subject requests then treat the directory as one
//! item; it downloads from `/recordings/{sitting}` as one MP4.

use super::{ConferenceCallHandler, RoomKey};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::warn;

/// A stream under way on this node.
pub(super) struct ActiveStream {
    pub(super) sitting: String,
    /// Its row, completed when it ends.
    record: fcp_core::Recording,
    /// The announcement anyone joining must hear, when the tenant asks.
    pub(super) consent_audio: Option<Arc<Vec<i16>>>,
    /// The continuing notice tone, when the stream announced itself.
    notice: Option<JoinHandle<()>>,
    #[cfg(feature = "recording-mp4")]
    recorder: crate::recording::Recorder,
}

impl ConferenceCallHandler {
    /// Where this node writes its streams.
    pub fn hls_dir(&self) -> PathBuf {
        let conf = self.conference_config.read();
        match conf.hls.dir.as_deref() {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(&conf.recordings_dir).join("hls"),
        }
    }

    /// Whether `sitting`'s stream is still being written here.
    pub fn is_streaming(&self, sitting: &str) -> bool {
        self.streams.iter().any(|s| s.sitting == sitting)
    }

    /// `key`'s webinar went live here: stream it, if its meeting asks and
    /// this node streams. Nothing when this node is an edge.
    pub(super) async fn start_stream(&self, key: &RoomKey) {
        if self.is_edge(key) || self.streams.contains_key(key) {
            return;
        }
        if !self.conference_config.read().hls.enabled {
            return;
        }
        let meeting = match self.lookup_meeting(&key.tenant, &key.room).await {
            Ok(Some(m)) if m.streams_hls() => m,
            _ => return,
        };
        if let Err(e) = self.open_stream(key, &meeting).await {
            warn!(room = %key.room, error = %e, "The webinar could not be streamed");
        }
    }

    #[cfg(feature = "recording-mp4")]
    async fn open_stream(&self, key: &RoomKey, meeting: &fcp_core::Meeting) -> anyhow::Result<()> {
        use crate::recording::{plan_format, FormatChoice, RecorderSettings, VideoSettings};
        let room = self
            .bridge
            .get_room(key)
            .map_err(|_| anyhow::anyhow!("no room"))?;
        let conf = self.conference_config.read().clone();
        let room_video = meeting.video_enabled && room.video().is_some();
        let usable = if room_video {
            self.video_backend
                .as_ref()
                .map(|b| crate::video_bridge::usable_codecs(b, conf.video_native_codecs_only))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let configured = crate::recording::parse_video_codec(&conf.recording_video_codec)
            .ok()
            .flatten();
        let (format, codec) = plan_format(FormatChoice::Mp4, configured, &usable, room_video);
        if format != fcp_core::RecordingFormat::Mp4 {
            anyhow::bail!("this build cannot write MP4");
        }
        if room_video && codec.is_none() {
            warn!(room = %key.room, "No H.264, HEVC or AV1 encoder here: the stream is audio alone");
        }
        let sitting = self.sitting_id(key);
        let dir = self.hls_dir().join(&sitting);

        let mut record = fcp_core::Recording::starting(
            key.tenant,
            &key.room,
            &self.node_id,
            dir.to_string_lossy(),
            fcp_core::RecordingFormat::Mp4,
            fcp_core::RecordingTrigger::Auto,
        );
        record.kind = fcp_core::RecordingKind::Stream;
        record.meeting_id = Some(meeting.id);
        record.participants = room.participant_count() as u32;
        record.participant_list = self.recording_participants(key, None);

        // Announced once: by the recording that started as the webinar
        // went live, else by the stream.
        let (consent, consent_audio, notice) = match self.recordings.get(key) {
            Some(r) => (r.record.consent, r.consent_audio.clone(), None),
            None => {
                let (consent, audio, plan) =
                    self.announce_recording_consent(key, conf.sample_rate).await;
                (consent, audio, Some(plan.notice_interval))
            }
        };
        if consent == fcp_core::RecordingConsent::Refused {
            anyhow::bail!(
                "the recording announcement could not be played and the tenant's policy does not \
                 record without one"
            );
        }
        record.consent = consent;

        // The row before the first segment: a stream no retention, hold or
        // erasure can find is not streamed.
        let store = self.storage.recording_store().ok_or_else(|| {
            anyhow::anyhow!("no recording store: a stream is kept as a recording")
        })?;
        store
            .create_recording(&record)
            .await
            .map_err(|e| anyhow::anyhow!("the stream's recording was not registered: {e}"))?;

        let settings = RecorderSettings {
            path: dir.clone(),
            container: fcp_core::RecordingFormat::Mp4,
            video: codec.map(|codec| VideoSettings {
                codec,
                resolution: conf
                    .recording_resolution
                    .as_deref()
                    .and_then(forge_video::frame::Resolution::parse),
                max_kbps: None,
            }),
            audio_bitrate: conf.recording_audio_bitrate,
            hls_segment_ms: Some(conf.hls.segment_secs.max(1) * 1000),
            hls_mirror: self.hls_bucket.as_ref().map(|b| b.mirror(&sitting)),
            rtmp: None,
        };
        let recorder = match crate::recording::Recorder::start(
            &format!("hls-{sitting}"),
            &room,
            settings,
        ) {
            Ok(r) => r,
            Err(e) => {
                if let Err(e) = store.delete_recording(&record.id).await {
                    warn!(recording = %record.id, error = %e, "Could not forget a stream that never started");
                }
                return Err(e);
            }
        };
        let notice = notice
            .and_then(|interval| self.spawn_recording_notice(key, interval, conf.sample_rate));
        tracing::info!(room = %key.room, sitting = %sitting, recording = %record.id, dir = %dir.display(), video = ?codec.map(|c| c.sdp_name()), "The webinar is streamed");
        self.streams.insert(
            key.clone(),
            ActiveStream {
                sitting,
                record,
                consent_audio,
                notice,
                recorder,
            },
        );
        Ok(())
    }

    #[cfg(not(feature = "recording-mp4"))]
    async fn open_stream(
        &self,
        _key: &RoomKey,
        _meeting: &fcp_core::Meeting,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this build cannot stream HLS (feature `recording-mp4`)")
    }

    /// `key`'s webinar ended here: its stream ends, the playlist with it.
    pub(super) async fn stop_stream(&self, key: &RoomKey) {
        let Some((_, stream)) = self.streams.remove(key) else {
            return;
        };
        if let Some(notice) = &stream.notice {
            notice.abort();
        }
        #[cfg(feature = "recording-mp4")]
        {
            let outcome = stream.recorder.stop().await;
            tracing::info!(room = %key.room, sitting = %stream.sitting, seconds = outcome.duration_secs, bytes = outcome.size_bytes, "The webinar's stream ended");
            if let Some(store) = self.storage.recording_store() {
                if let Err(e) = store
                    .finish_recording(
                        &stream.record.id,
                        std::time::SystemTime::now(),
                        outcome.duration_secs,
                        outcome.size_bytes,
                    )
                    .await
                {
                    warn!(recording = %stream.record.id, error = %e, "Could not complete the stream's row");
                }
            }
        }
        #[cfg(not(feature = "recording-mp4"))]
        let _ = stream;
    }

    /// The stream under way in `key`'s room, for the webinar's JSON.
    pub(super) fn stream_json(&self, key: &RoomKey) -> Option<serde_json::Value> {
        let s = self.streams.get(key)?;
        Some(serde_json::json!({
            "format": "hls",
            "sitting": s.sitting,
            "recording_id": s.record.id.to_string(),
            "playlist": format!("/hls/{}/{}", s.sitting, "index.m3u8"),
        }))
    }
}
