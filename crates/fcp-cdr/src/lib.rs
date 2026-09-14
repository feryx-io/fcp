//! The CDR pipeline (`docs/CDR_PIPELINE.md`).
//!
//! A node builds a complete [`Record`] at the end of a call with
//! [`RecordBuilder`] and hands it to a [`CdrWriter`], which appends it to a
//! local [`Spool`] before the hangup path moves on. A [`Shipper`] drains the
//! spool into a [`Sink`]; the sink that exists in phase 1 is [`LocalSink`],
//! which runs the [`Processor`] in-process against the node's own store.
//! Every stage tolerates a replay: the record's `cdr_id` is minted once, in
//! the builder, and the store skips a duplicate.

pub mod consumer;
pub mod enrich;
pub mod maintenance;
pub mod node;
pub mod processor;
pub mod record;
pub mod shipper;
pub mod sink;
pub mod spool;
pub mod stats;
pub mod writer;

pub use consumer::{
    Consumer, ConsumerConfig, RecordSource, SourceStatus, SourcedRecord, Undecodable,
};
pub use enrich::{DirectionEnricher, Enricher};
pub use fcp_core::storage::{CallDetailRecord as Record, CdrKind, CDR_SCHEMA_VERSION};
pub use maintenance::{spawn_maintenance, MaintenanceConfig};
pub use node::{start_node, NodeConfig, SinkKind};
pub use processor::{Outcome, Processor, Rater, Rating, Rejection};
pub use record::{RecordBuilder, RtpStats, SystemCause, TerminationReason};
pub use shipper::{Shipper, ShipperConfig};
pub use sink::{LocalSink, Sink};
pub use spool::{Cursor, Spool, SpoolConfig, SpoolStatus};
pub use stats::{ProcessorStats, StatsSnapshot};
pub use writer::CdrWriter;

/// What can go wrong in the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("spool I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("record encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("store: {0}")]
    Store(#[from] anyhow::Error),
    #[error("sink: {0}")]
    Sink(String),
    #[error("record rejected: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
