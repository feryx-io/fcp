//! A webinar pushed to a live service over RTMP (`docs/WEBINARS.md` §2.8,
//! W7).
//!
//! A webinar whose meeting has an RTMP destination is pushed by its stage
//! from the moment it goes live until it ends, under `[conference.rtmp]
//! enabled`: never in practice, never from an edge. The push is the
//! recorder's composite (H.264) and mix (AAC). A node with no H.264
//! encoder does not push, and says so.
//!
//! A push broadcasts the room, so the room hears the recording
//! announcement first, as for a stream: the recording's or the stream's
//! serves when there is one. FCP keeps nothing of the push: no row.
//!
//! Its state is in the webinar's JSON (`rtmp`), and it publishes
//! `conference.rtmp_live` each time it starts publishing and
//! `conference.rtmp_failed` once an outage has lasted a minute.

use super::{ConferenceCallHandler, RoomKey};
use crate::recording::rtmp::{PushState, RtmpStatus};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;
use tracing::warn;

/// How long an outage lasts before it is told as `rtmp_failed`.
const FAILED_AFTER: Duration = Duration::from_secs(60);

/// A push under way on this node.
pub(super) struct ActivePush {
    host: String,
    status: Arc<RtmpStatus>,
    /// The announcement anyone joining must hear, when the push made it.
    pub(super) consent_audio: Option<Arc<Vec<i16>>>,
    notice: Option<JoinHandle<()>>,
    watcher: JoinHandle<()>,
    #[cfg(feature = "rtmp")]
    recorder: crate::recording::Recorder,
}

impl ConferenceCallHandler {
    /// `key`'s webinar went live here: push it, if its meeting has a
    /// destination and this node pushes. Nothing on an edge.
    pub(super) async fn start_push(&self, key: &RoomKey) {
        if self.is_edge(key) || self.pushes.contains_key(key) {
            return;
        }
        if !self.conference_config.read().rtmp.enabled {
            return;
        }
        let meeting = match self.lookup_meeting(&key.tenant, &key.room).await {
            Ok(Some(m)) if m.is_webinar() => m,
            _ => return,
        };
        let Some(store) = self.storage.rtmp_destination_store() else {
            return;
        };
        let destination = match store
            .get_rtmp_destination(&key.tenant, &meeting.id.to_string())
            .await
        {
            Ok(Some(d)) => d,
            Ok(None) => return,
            Err(e) => {
                warn!(room = %key.room, error = %e, "The webinar's RTMP destination could not be read");
                return;
            }
        };
        if let Err(e) = self.open_push(key, &meeting, destination).await {
            warn!(room = %key.room, error = %e, "The webinar could not be pushed over RTMP");
        }
    }

    #[cfg(feature = "rtmp")]
    async fn open_push(
        &self,
        key: &RoomKey,
        meeting: &fcp_core::Meeting,
        destination: fcp_core::webinar::RtmpDestination,
    ) -> anyhow::Result<()> {
        use crate::recording::rtmp::RtmpTarget;
        use crate::recording::{RecorderSettings, VideoSettings};
        let room = self
            .bridge
            .get_room(key)
            .map_err(|_| anyhow::anyhow!("no room"))?;
        let conf = self.conference_config.read().clone();
        if !(meeting.video_enabled && room.video().is_some()) {
            anyhow::bail!("the webinar has no video, and an RTMP service wants a picture");
        }
        let usable = self
            .video_backend
            .as_ref()
            .map(|b| crate::video_bridge::usable_codecs(b, conf.video_native_codecs_only))
            .unwrap_or_default();
        if !usable.contains(&forge_core::VideoCodec::H264) {
            anyhow::bail!("no H.264 encoder on this node: RTMP carries H.264");
        }

        // Announced once: by the recording or the stream that started as
        // the webinar went live, else by the push.
        let shared = self
            .recordings
            .get(key)
            .map(|r| r.consent_audio.clone())
            .or_else(|| self.streams.get(key).map(|s| s.consent_audio.clone()));
        let (consent_audio, notice) = match shared {
            Some(audio) => (audio, None),
            None => {
                let (consent, audio, plan) =
                    self.announce_recording_consent(key, conf.sample_rate).await;
                if consent == fcp_core::RecordingConsent::Refused {
                    anyhow::bail!(
                        "the recording announcement could not be played and the tenant's policy \
                         does not broadcast without one"
                    );
                }
                (audio, Some(plan.notice_interval))
            }
        };

        let status = Arc::new(RtmpStatus::default());
        let host = crate::recording::rtmp::ingest_host(&destination.url);
        let settings = RecorderSettings {
            path: std::path::PathBuf::new(),
            container: fcp_core::RecordingFormat::Mp4,
            video: Some(VideoSettings {
                codec: forge_core::VideoCodec::H264,
                resolution: conf
                    .recording_resolution
                    .as_deref()
                    .and_then(forge_video::frame::Resolution::parse),
                max_kbps: None,
            }),
            audio_bitrate: conf.recording_audio_bitrate,
            hls_segment_ms: None,
            hls_mirror: None,
            rtmp: Some(RtmpTarget {
                url: destination.url.clone(),
                key: destination.key.clone(),
                audio_bitrate: conf.rtmp.audio_bitrate,
                keyframe_ms: conf.rtmp.keyframe_secs.max(1) * 1000,
                status: Arc::clone(&status),
            }),
        };
        let sitting = self.sitting_id(key);
        let recorder =
            crate::recording::Recorder::start(&format!("rtmp-{sitting}"), &room, settings)?;
        let notice = notice
            .and_then(|interval| self.spawn_recording_notice(key, interval, conf.sample_rate));
        let watcher = self.watch_push(key, &sitting, &host, Arc::clone(&status));
        tracing::info!(room = %key.room, sitting = %sitting, ingest = %host, "The webinar is pushed over RTMP");
        self.pushes.insert(
            key.clone(),
            ActivePush {
                host,
                status,
                consent_audio,
                notice,
                watcher,
                recorder,
            },
        );
        Ok(())
    }

    #[cfg(not(feature = "rtmp"))]
    async fn open_push(
        &self,
        _key: &RoomKey,
        _meeting: &fcp_core::Meeting,
        _destination: fcp_core::webinar::RtmpDestination,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this build cannot push RTMP (feature `rtmp`)")
    }

    /// Tell the push's comings and goings as events.
    #[cfg_attr(not(feature = "rtmp"), allow(dead_code))]
    fn watch_push(
        &self,
        key: &RoomKey,
        sitting: &str,
        host: &str,
        status: Arc<RtmpStatus>,
    ) -> JoinHandle<()> {
        let me = self.me();
        let key = key.clone();
        let sitting = sitting.to_string();
        let host = host.to_string();
        tokio::spawn(async move {
            let Some(me) = me else { return };
            let mut told_connects = 0u32;
            let mut told_failure = false;
            loop {
                tokio::select! {
                    _ = status.changed() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                let r = status.report();
                if r.state == PushState::Stopped {
                    return;
                }
                if r.state == PushState::Live && r.connects > told_connects {
                    told_connects = r.connects;
                    told_failure = false;
                    me.publish_event(fcp_events::ConferenceEvent::RtmpLive {
                        room_number: key.room.clone(),
                        sitting_id: sitting.clone(),
                        host: host.clone(),
                        tenant_id: key.tenant,
                        timestamp: SystemTime::now(),
                    });
                }
                let failing_for = r
                    .failing_since
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .unwrap_or_default();
                if r.state != PushState::Live && failing_for >= FAILED_AFTER && !told_failure {
                    told_failure = true;
                    me.publish_event(fcp_events::ConferenceEvent::RtmpFailed {
                        room_number: key.room.clone(),
                        sitting_id: sitting.clone(),
                        host: host.clone(),
                        error: r.error.clone().unwrap_or_default(),
                        tenant_id: key.tenant,
                        timestamp: SystemTime::now(),
                    });
                }
            }
        })
    }

    /// `key`'s webinar ended here: its push ends, unpublished.
    pub(super) async fn stop_push(&self, key: &RoomKey) {
        let Some((_, push)) = self.pushes.remove(key) else {
            return;
        };
        if let Some(notice) = &push.notice {
            notice.abort();
        }
        #[cfg(feature = "rtmp")]
        {
            let outcome = push.recorder.stop().await;
            tracing::info!(room = %key.room, ingest = %push.host, seconds = outcome.duration_secs, bytes = push.status.report().bytes_sent, "The webinar's RTMP push ended");
        }
        // The task unpublishes when the recorder lets go; the watcher ends
        // when it says so, or here.
        let watcher = push.watcher;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            watcher.abort();
        });
    }

    /// The push under way in `key`'s room, for the webinar's JSON.
    pub(super) fn push_json(&self, key: &RoomKey) -> Option<serde_json::Value> {
        let p = self.pushes.get(key)?;
        let mut j = p.status.report().to_json();
        j["ingest"] = p.host.clone().into();
        Some(j)
    }
}
