//! A webinar's stream copied to a bucket (`docs/WEBINARS.md` §2.8, W6.3c).
//!
//! With `[conference.hls.s3]`, every file the stream writes to its
//! directory is also put in the bucket, at `<prefix><sitting>/<file>`, in
//! the order it was written: the init segment, then each segment before the
//! playlist that names it. The playlist is `no-cache` and the segments
//! immutable, as the node serves them.
//!
//! The directory stays the stream's recording: it is what downloads,
//! retention and erasure act on, and the bucket's copy goes with it. The
//! bucket is for delivery: a request for the init segment or a segment
//! that is in the bucket is redirected there, through
//! `[conference.hls] public_url` (a CDN in front of the bucket) when that
//! is set. The playlist is always served by the node, so a player keeps
//! coming back through the stream link for it, and a segment the bucket
//! does not have yet, or never got, is served from the directory.
//!
//! Uploads run on a task of their own and never hold up the writer. A copy
//! that falls too far behind, or whose upload fails for good, stops: what
//! it has stays redirected and the rest is served by the node.

use std::collections::HashSet;
use std::sync::Arc;

use dashmap::DashMap;
use fcp_cdr::archive::{S3Config, S3Target};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::hls;

/// Files waiting to be put before a copy is given up on: about two minutes
/// of 2 s segments with their playlists.
const QUEUE: usize = 128;

/// What the bucket holds of one sitting's stream.
#[derive(Default)]
struct Copy {
    uploaded: HashSet<String>,
    /// Given up on: nothing more is put.
    stopped: bool,
}

/// The bucket a node copies its streams to.
pub struct HlsBucket {
    target: S3Target,
    bucket: String,
    prefix: String,
    public_url: Option<String>,
    copies: DashMap<String, Arc<Mutex<Copy>>>,
}

/// The writer's end of one stream's copy.
#[derive(Clone)]
pub struct HlsMirror {
    sitting: String,
    tx: mpsc::Sender<(String, Vec<u8>)>,
    copy: Arc<Mutex<Copy>>,
}

impl std::fmt::Debug for HlsMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HlsMirror")
            .field("sitting", &self.sitting)
            .finish_non_exhaustive()
    }
}

impl HlsMirror {
    /// `name` was written with `bytes`: put it in the bucket too.
    pub fn send(&self, name: &str, bytes: &[u8]) {
        if self.copy.lock().stopped {
            return;
        }
        if let Err(e) = self.tx.try_send((name.to_string(), bytes.to_vec())) {
            let mut copy = self.copy.lock();
            if !copy.stopped {
                copy.stopped = true;
                warn!(sitting = %self.sitting, file = name, error = %e, "The stream's bucket copy fell behind; the node serves the rest");
            }
        }
    }
}

impl HlsBucket {
    /// The bucket `config` names; keys under its prefix, `hls/` unless set.
    pub fn new(
        config: &fcp_core::FirmwareS3Config,
        public_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let target = S3Target::new(S3Config {
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
            path_style: config.path_style,
            access_key_id: config.access_key_id.clone(),
            secret_access_key: config.secret_access_key.clone(),
            session_token: config.session_token.clone(),
            ..S3Config::new(config.bucket.clone())
        })
        .map_err(|e| anyhow::anyhow!("[conference.hls.s3]: {e}"))?;
        Ok(Self {
            target,
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone().unwrap_or_else(|| "hls/".to_string()),
            public_url: public_url
                .map(|u| u.trim_end_matches('/').to_string())
                .filter(|u| !u.is_empty()),
            copies: DashMap::new(),
        })
    }

    /// Where, for the log: `s3://bucket/prefix`.
    pub fn location(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.prefix)
    }

    fn key(&self, sitting: &str, file: &str) -> String {
        format!("{}{sitting}/{file}", self.prefix)
    }

    /// Start copying `sitting`'s stream: the writer sends each file it
    /// writes, and a task puts them in order.
    pub fn mirror(self: &Arc<Self>, sitting: &str) -> HlsMirror {
        let copy = Arc::new(Mutex::new(Copy::default()));
        self.copies.insert(sitting.to_string(), Arc::clone(&copy));
        let (tx, mut rx) = mpsc::channel::<(String, Vec<u8>)>(QUEUE);
        let bucket = Arc::clone(self);
        let task_copy = Arc::clone(&copy);
        let task_sitting = sitting.to_string();
        tokio::spawn(async move {
            while let Some((name, bytes)) = rx.recv().await {
                if task_copy.lock().stopped {
                    continue;
                }
                let key = bucket.key(&task_sitting, &name);
                let cache = if name == hls::PLAYLIST {
                    "no-cache"
                } else {
                    "public, max-age=31536000, immutable"
                };
                match bucket
                    .target
                    .put_bytes(&key, bytes, hls::content_type(&name), Some(cache))
                    .await
                {
                    Ok(()) => {
                        debug!(sitting = %task_sitting, %key, "Stream file put in the bucket");
                        task_copy.lock().uploaded.insert(name);
                    }
                    Err(e) => {
                        warn!(sitting = %task_sitting, %key, error = %e, "A stream file could not be put in the bucket; the node serves the rest");
                        task_copy.lock().stopped = true;
                    }
                }
            }
        });
        HlsMirror {
            sitting: sitting.to_string(),
            tx,
            copy,
        }
    }

    /// Where a player is sent for `file` of `sitting`: the bucket's copy,
    /// when it has one. Never the playlist.
    pub fn redirect(&self, sitting: &str, file: &str) -> Option<String> {
        if file == hls::PLAYLIST {
            return None;
        }
        let copy = self.copies.get(sitting)?;
        if !copy.lock().uploaded.contains(file) {
            return None;
        }
        let key = self.key(sitting, file);
        Some(match &self.public_url {
            Some(base) => format!("{base}/{key}"),
            None => self.target.object_url(&key),
        })
    }

    /// `sitting`'s stream is deleted: take its copy out of the bucket.
    /// `files` are the names its directory held. The copy is forgotten
    /// first, so nothing is redirected to an object on its way out.
    pub async fn remove(&self, sitting: &str, files: &[String]) -> anyhow::Result<()> {
        self.copies.remove(sitting);
        for file in files {
            self.target
                .delete(&self.key(sitting, file))
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        info!(
            sitting,
            files = files.len(),
            "The stream's bucket copy was removed"
        );
        Ok(())
    }
}

/// The names of the files in a stream's directory that the bucket may
/// hold: the playlist, the init segment and each segment.
pub fn copied_files(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| hls::is_stream_file(n))
        .collect()
}
