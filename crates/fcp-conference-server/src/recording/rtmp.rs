//! A webinar pushed to a live service over RTMP (`docs/WEBINARS.md` §2.8,
//! W7): the recorder's composite (H.264) and mix (AAC) as FLV tags,
//! published to the meeting's RTMP destination.
//!
//! The recorder hands each frame to an [`RtmpWriter`], which never waits:
//! it queues the frame for a task of its own that owns the connection.
//! - Behind, the writer drops video until the next keyframe, which it asks
//!   for. Audio is dropped only when the queue is full.
//! - Lost, the task connects again with backoff (1 s doubling to 30 s). It
//!   sends the sequence headers again and waits for a keyframe, with the
//!   timestamps going on from where they were.
//! - A keyframe is asked for every `keyframe_secs`, as services want.
//!
//! What the push is doing is in its [`RtmpStatus`], for the webinar's JSON
//! and the events.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// What a push is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushState {
    /// Connecting for the first time.
    Connecting,
    /// Publishing.
    Live,
    /// Lost or refused, and trying again.
    Retrying,
    /// Over: the webinar ended.
    Stopped,
}

/// A push's state, shared by its task and whoever shows it.
#[derive(Debug, Clone)]
pub struct PushReport {
    pub state: PushState,
    /// When it entered the state.
    pub since: SystemTime,
    /// When it last stopped publishing, while it is not.
    pub failing_since: Option<SystemTime>,
    /// The last error, if any.
    pub error: Option<String>,
    /// Bytes sent, across connections.
    pub bytes_sent: u64,
    /// Connections made.
    pub connects: u32,
    /// Video frames dropped while behind.
    pub dropped_frames: u64,
}

impl PushReport {
    /// As the webinar's JSON shows it.
    pub fn to_json(&self) -> serde_json::Value {
        let at = |t: SystemTime| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        let mut j = serde_json::json!({
            "state": self.state,
            "since": at(self.since),
            "bytes_sent": self.bytes_sent,
            "connects": self.connects,
            "dropped_frames": self.dropped_frames,
        });
        if let Some(e) = &self.error {
            j["error"] = e.clone().into();
        }
        j
    }
}

/// The state of one push, written by its task.
#[derive(Debug)]
pub struct RtmpStatus {
    report: Mutex<PushReport>,
    changed: tokio::sync::Notify,
}

impl Default for RtmpStatus {
    fn default() -> Self {
        Self {
            report: Mutex::new(PushReport {
                state: PushState::Connecting,
                since: SystemTime::now(),
                failing_since: None,
                error: None,
                bytes_sent: 0,
                connects: 0,
                dropped_frames: 0,
            }),
            changed: tokio::sync::Notify::new(),
        }
    }
}

impl RtmpStatus {
    pub fn report(&self) -> PushReport {
        self.report.lock().unwrap().clone()
    }

    /// Wait for the next change.
    pub async fn changed(&self) {
        self.changed.notified().await
    }

    // Written by the push's task, which only the `rtmp` build has.
    #[cfg_attr(not(feature = "rtmp"), allow(dead_code))]
    fn update(&self, f: impl FnOnce(&mut PushReport)) {
        let mut r = self.report.lock().unwrap();
        let before = r.state;
        f(&mut r);
        if r.state != before {
            r.since = SystemTime::now();
        }
        drop(r);
        self.changed.notify_waiters();
    }

    // Written by the push's task, which only the `rtmp` build has.
    #[cfg_attr(not(feature = "rtmp"), allow(dead_code))]
    pub(crate) fn set(&self, state: PushState, error: Option<String>) {
        self.update(|r| {
            match state {
                PushState::Live => r.failing_since = None,
                PushState::Retrying | PushState::Connecting => {
                    r.failing_since.get_or_insert_with(SystemTime::now);
                }
                PushState::Stopped => {}
            }
            r.state = state;
            if error.is_some() {
                r.error = error;
            }
            if state == PushState::Live {
                r.connects += 1;
            }
        })
    }

    // Written by the push's task, which only the `rtmp` build has.
    #[cfg_attr(not(feature = "rtmp"), allow(dead_code))]
    pub(crate) fn sent(&self, total: u64) {
        // Not a change worth waking anyone for.
        self.report.lock().unwrap().bytes_sent = total;
    }

    // Written by the push's task, which only the `rtmp` build has.
    #[cfg_attr(not(feature = "rtmp"), allow(dead_code))]
    pub(crate) fn dropped(&self) {
        self.report.lock().unwrap().dropped_frames += 1;
    }
}

/// Where to push and how.
#[derive(Clone)]
pub struct RtmpTarget {
    pub url: String,
    pub key: String,
    pub audio_bitrate: u32,
    pub keyframe_ms: u64,
    pub status: Arc<RtmpStatus>,
}

impl std::fmt::Debug for RtmpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpTarget")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// The ingest's host, for logs and events: never the key.
pub fn ingest_host(url: &str) -> String {
    let rest = url
        .strip_prefix("rtmps://")
        .or_else(|| url.strip_prefix("rtmp://"))
        .unwrap_or(url);
    rest.split('/').next().unwrap_or_default().to_string()
}

#[cfg(feature = "rtmp")]
pub use writer::RtmpWriter;

#[cfg(feature = "rtmp")]
mod writer {
    use super::*;
    use forge_rtmp::flv::{aac_frame, aac_sequence_header, Metadata};
    use forge_rtmp::{AvcTagger, Publisher, RtmpUrl};
    use forge_video::flavor::Flavor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tracing::{debug, info, warn};

    /// Frames waiting for the network: about four seconds of video and
    /// audio.
    const QUEUE: usize = 512;
    const BACKOFF_FIRST: Duration = Duration::from_secs(1);
    const BACKOFF_MAX: Duration = Duration::from_secs(30);

    enum Item {
        Video {
            ms: u64,
            keyframe: bool,
            data: Vec<u8>,
        },
        Audio {
            ms: u64,
            data: Vec<u8>,
        },
    }

    /// The recorder's end of a push.
    pub struct RtmpWriter {
        tx: Option<mpsc::Sender<Item>>,
        status: Arc<RtmpStatus>,
        need_key: Arc<AtomicBool>,
        /// Behind: video is dropped until a keyframe.
        dropping: bool,
        keyframe_ms: u64,
        frame_ms: u64,
        last_keyframe_ms: Option<u64>,
        asked: bool,
        has_video: bool,
        first_ms: Option<u64>,
        last_ms: u64,
    }

    impl RtmpWriter {
        /// Start pushing to `target`: the composite at `video` (H.264) when
        /// there is one, and AAC with `asc` as its configuration.
        pub fn open(
            target: RtmpTarget,
            video: Option<&Flavor>,
            asc: Vec<u8>,
        ) -> anyhow::Result<Self> {
            let url = RtmpUrl::parse(&target.url)?;
            if let Some(f) = video {
                if f.codec != forge_core::VideoCodec::H264 {
                    anyhow::bail!("RTMP carries H.264, not {}", f.codec.sdp_name());
                }
            }
            let meta = Metadata {
                width: video.map(|f| f.resolution.width),
                height: video.map(|f| f.resolution.height),
                framerate: video.map(|f| f.fps as f64),
                video_kbps: None,
                audio: Some((48_000, 1)),
                audio_kbps: Some(target.audio_bitrate / 1000),
                encoder: format!("FCP {}", env!("CARGO_PKG_VERSION")),
            };
            let (tx, rx) = mpsc::channel(QUEUE);
            let need_key = Arc::new(AtomicBool::new(false));
            tokio::spawn(push(
                url,
                target.key.clone(),
                rx,
                Arc::clone(&target.status),
                Arc::clone(&need_key),
                meta,
                asc,
                video.is_some(),
            ));
            Ok(Self {
                tx: Some(tx),
                status: target.status,
                need_key,
                dropping: false,
                keyframe_ms: target.keyframe_ms.max(500),
                frame_ms: video.map_or(66, |f| 1000 / u64::from(f.fps.max(1))),
                last_keyframe_ms: None,
                asked: false,
                has_video: video.is_some(),
                first_ms: None,
                last_ms: 0,
            })
        }

        fn stamp(&mut self, ms: u64) {
            self.first_ms.get_or_insert(ms);
            self.last_ms = self.last_ms.max(ms);
        }

        pub fn write_video(&mut self, ms: u64, keyframe: bool, data: &[u8]) -> anyhow::Result<()> {
            self.stamp(ms);
            if keyframe {
                self.last_keyframe_ms = Some(ms);
                self.asked = false;
                self.dropping = false;
            } else if self.dropping {
                self.status.dropped();
                return Ok(());
            }
            let Some(tx) = &self.tx else { return Ok(()) };
            let item = Item::Video {
                ms,
                keyframe,
                data: data.to_vec(),
            };
            if tx.try_send(item).is_err() {
                // Behind: a picture lost is no use without what follows,
                // so wait for a keyframe, and ask for one.
                self.dropping = true;
                self.need_key.store(true, Ordering::Relaxed);
                self.status.dropped();
            }
            Ok(())
        }

        pub fn write_audio(&mut self, ms: u64, data: &[u8]) -> anyhow::Result<()> {
            self.stamp(ms);
            if let Some(tx) = &self.tx {
                let _ = tx.try_send(Item::Audio {
                    ms,
                    data: data.to_vec(),
                });
            }
            Ok(())
        }

        /// Whether to ask the encoder for a keyframe now: the push is
        /// behind or reconnected, or one is due.
        pub fn wants_keyframe(&mut self, ms: u64) -> bool {
            if !self.has_video {
                return false;
            }
            if self.need_key.swap(false, Ordering::Relaxed) {
                self.asked = true;
                return true;
            }
            if self.asked {
                return false;
            }
            let Some(last) = self.last_keyframe_ms else {
                return false;
            };
            if ms + self.frame_ms >= last + self.keyframe_ms {
                self.asked = true;
                return true;
            }
            false
        }

        /// The end: the task sends what it has and unpublishes. What was
        /// pushed: its length and bytes so far.
        pub fn finish(mut self) -> (u64, u64) {
            self.tx = None;
            let duration = self.last_ms.saturating_sub(self.first_ms.unwrap_or(0));
            (duration, self.status.report().bytes_sent)
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn push(
        url: RtmpUrl,
        key: String,
        mut rx: mpsc::Receiver<Item>,
        status: Arc<RtmpStatus>,
        need_key: Arc<AtomicBool>,
        meta: Metadata,
        asc: Vec<u8>,
        has_video: bool,
    ) {
        let host = super::ingest_host(&url.tc_url);
        let mut backoff = BACKOFF_FIRST;
        let mut tagger = AvcTagger::new();
        let mut sent_before = 0u64;
        let mut first = true;
        loop {
            status.set(
                if first {
                    PushState::Connecting
                } else {
                    PushState::Retrying
                },
                None,
            );
            first = false;
            let publisher = match Publisher::connect(&url, &key).await {
                Ok(p) => p,
                Err(e) => {
                    warn!(ingest = %host, error = %e, retry_in = ?backoff, "The RTMP push could not connect");
                    status.set(PushState::Retrying, Some(e.to_string()));
                    // Frames meanwhile are of no use: drop them, and stop
                    // when the webinar does.
                    let wait = tokio::time::sleep(backoff);
                    tokio::pin!(wait);
                    loop {
                        tokio::select! {
                            _ = &mut wait => break,
                            item = rx.recv() => if item.is_none() {
                                status.set(PushState::Stopped, None);
                                return;
                            },
                        }
                    }
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    continue;
                }
            };
            let opened = async {
                publisher.send_metadata(&meta).await?;
                publisher.send_audio(0, aac_sequence_header(&asc)).await
            }
            .await;
            if let Err(e) = opened {
                status.set(PushState::Retrying, Some(e.to_string()));
                continue;
            }
            info!(ingest = %host, "The RTMP push is live");
            status.set(PushState::Live, None);
            backoff = BACKOFF_FIRST;
            tagger.reset();
            if has_video {
                need_key.store(true, Ordering::Relaxed);
            }
            let lost = loop {
                let Some(item) = rx.recv().await else {
                    let total = sent_before + publisher.bytes_sent();
                    publisher.close().await;
                    status.sent(total);
                    status.set(PushState::Stopped, None);
                    debug!(ingest = %host, "The RTMP push ended");
                    return;
                };
                let result = match item {
                    Item::Video { ms, keyframe, data } => match tagger.bodies(keyframe, &data) {
                        Ok(bodies) => {
                            let mut r = Ok(());
                            for body in bodies {
                                r = publisher.send_video(ms as u32, body).await;
                                if r.is_err() {
                                    break;
                                }
                            }
                            r
                        }
                        Err(e) => {
                            debug!(error = %e, "A picture could not be tagged");
                            Ok(())
                        }
                    },
                    Item::Audio { ms, data } => {
                        publisher.send_audio(ms as u32, aac_frame(&data)).await
                    }
                };
                status.sent(sent_before + publisher.bytes_sent());
                if let Err(e) = result {
                    break e;
                }
            };
            sent_before += publisher.bytes_sent();
            warn!(ingest = %host, error = %lost, "The RTMP push was lost; connecting again");
            status.set(PushState::Retrying, Some(lost.to_string()));
            drop(publisher);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_named_and_never_the_key() {
        assert_eq!(
            ingest_host("rtmp://a.rtmp.youtube.com/live2"),
            "a.rtmp.youtube.com"
        );
        assert_eq!(
            ingest_host("rtmps://x.example:443/app/sub"),
            "x.example:443"
        );
    }

    #[test]
    fn the_status_says_when_it_started_failing_and_counts_connections() {
        let s = RtmpStatus::default();
        assert_eq!(s.report().state, PushState::Connecting);
        s.set(PushState::Live, None);
        let r = s.report();
        assert_eq!((r.state, r.connects), (PushState::Live, 1));
        assert!(r.failing_since.is_none());
        s.set(PushState::Retrying, Some("closed".into()));
        let r = s.report();
        assert!(r.failing_since.is_some());
        assert_eq!(r.error.as_deref(), Some("closed"));
        let since = r.failing_since;
        s.set(PushState::Retrying, Some("refused".into()));
        assert_eq!(s.report().failing_since, since, "an outage is one outage");
        s.set(PushState::Live, None);
        assert_eq!(s.report().connects, 2);
        assert!(s.report().failing_since.is_none());
    }
}
