//! The node side, wired up in one call: the spool, its fsync task, the
//! shipper to the in-process sink, and the writer the call path uses.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::processor::Processor;
use crate::shipper::{Shipper, ShipperConfig};
use crate::sink::LocalSink;
use crate::sink::Sink;
use crate::spool::{Spool, SpoolConfig};
use crate::writer::CdrWriter;
use crate::{Error, Result};

/// Where a node's spool ships to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkKind {
    /// The processor runs on the node, against its own database.
    #[default]
    Local,
    /// Records go to the Redis stream for the processor in `fcp-jobs`
    /// (`[cdr] enabled` there). Needs the node's `storage.redis_url`.
    Stream,
}

/// The `[cdr]` section of a node's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NodeConfig {
    /// Where records are spooled before they ship. Unset, records go
    /// straight to the processor on a task each, which is the MVP's
    /// durability; set it on any node whose records matter.
    #[serde(default)]
    pub spool_dir: Option<PathBuf>,

    /// `local` (default): the node stores its records. `stream`: it ships
    /// them to the Redis stream, and an `fcp-jobs` processor stores them.
    /// Nothing reads the stream unless one runs; the choice is explicit
    /// for that reason.
    #[serde(default)]
    pub sink: SinkKind,

    /// How often the open spool segment is synced to disk.
    #[serde(default = "default_fsync_interval_ms")]
    pub fsync_interval_ms: u64,

    /// A spool segment is rolled once it holds this many bytes.
    #[serde(default = "default_segment_bytes")]
    pub segment_bytes: u64,

    /// Records per shipped batch.
    #[serde(default = "default_ship_batch")]
    pub ship_batch: usize,
}

fn default_fsync_interval_ms() -> u64 {
    100
}

fn default_segment_bytes() -> u64 {
    16 * 1024 * 1024
}

fn default_ship_batch() -> usize {
    200
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            spool_dir: None,
            sink: SinkKind::Local,
            fsync_interval_ms: default_fsync_interval_ms(),
            segment_bytes: default_segment_bytes(),
            ship_batch: default_ship_batch(),
        }
    }
}

/// Start the node side of the pipeline over `processor` and return the
/// writer the call path hands records to. With a `spool_dir`, opens the
/// spool (draining whatever an earlier run left there) and starts its
/// fsync task and the shipper, to the in-process processor or, for `sink =
/// "stream"`, to `stream`; without one, records go to the processor
/// directly. Needs a tokio runtime.
pub fn start_node(
    config: &NodeConfig,
    processor: Arc<Processor>,
    stream: Option<Arc<dyn Sink>>,
) -> Result<CdrWriter> {
    if config.sink == SinkKind::Stream && config.spool_dir.is_none() {
        return Err(Error::Sink(
            "[cdr] sink = \"stream\" needs a spool_dir: the spool is what stands between a call and the stream".into(),
        ));
    }
    let Some(dir) = &config.spool_dir else {
        warn!("No [cdr] spool_dir configured: records go straight to the database and are lost if it refuses them");
        return Ok(CdrWriter::direct(processor));
    };
    let spool = Spool::open(
        dir,
        SpoolConfig {
            fsync_interval: Duration::from_millis(config.fsync_interval_ms.max(1)),
            segment_bytes: config.segment_bytes.max(4096),
        },
    )?;
    let status = spool.status()?;
    info!(
        dir = %dir.display(),
        pending_bytes = status.pending_bytes,
        segments = status.segments,
        "CDR spool open"
    );
    spool.start_fsync();
    let sink: Arc<dyn Sink> = match config.sink {
        SinkKind::Local => {
            Arc::new(LocalSink::new(Arc::clone(&processor)).with_dead_letters(spool.clone()))
        }
        SinkKind::Stream => stream.ok_or_else(|| {
            Error::Sink("[cdr] sink = \"stream\" needs storage.redis_url on this node".into())
        })?,
    };
    info!(sink = sink.name(), "CDR shipper sink");
    Shipper::spawn(
        spool.clone(),
        sink,
        ShipperConfig {
            batch: config.ship_batch.max(1),
            ..ShipperConfig::default()
        },
    );
    Ok(CdrWriter::spooled(spool, Some(processor)))
}
