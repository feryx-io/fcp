//! Video capacity accounting (design §9.3).
//!
//! A room's video costs codec time: a decode per source, an encode per
//! flavor, a composite per output. Each is a constant in nanoseconds per
//! pixel — measured on this node at start by the self-benchmark, or taken
//! from the phase 0 table (§9.4) — times the stream's pixel rate. This
//! module prices what runs here, keeps the node's budget, refuses a join
//! that would not fit, and advertises what is left in cluster membership
//! so placement sends video rooms where the capacity is.
//!
//! **Work unit.** One unit is a thousandth of one execution unit's time
//! per second — on the host, a millicore of one codec-pool thread. A node
//! with a ten-thread pool advertises `10 × 1000 × headroom` units; a 720p15
//! H.264 encode of hard content (4.4 ns/px) costs `4.4 × 921 600 × 15 / 1e6
//! ≈ 61`. The same definition fits a GPU media engine later.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_conference::video::forward::{
    DEFAULT_MAX_STREAMS, DEFAULT_OTHER_HEIGHT, DEFAULT_SPEAKER_HEIGHT,
};
use forge_conference::video::{
    default_kbps, ContentGate, OutputScope, VideoBackend, VideoRoom, VideoRoomSettings,
    VideoViewerInfo, View,
};
use forge_conference::ConferenceRoom;
use forge_core::VideoCodec;
use forge_video::bench::{self, BenchSettings};
use forge_video::frame::{MediaDevice, Resolution};
use tracing::{info, warn};

/// Nanoseconds of device time per second of video in one work unit.
pub const NS_PER_UNIT: f64 = 1e6;

/// `[conference] video_egress_mbps` when unset (FORWARDING_ROOMS §8 Q11):
/// a gigabit link.
pub const DEFAULT_EGRESS_MBPS: u32 = 1000;

/// Work units in one execution unit (a codec-pool thread, a media engine).
pub const UNITS_PER_EXECUTION_UNIT: f64 = 1000.0;

/// Where a cost table's constants came from.
#[derive(Debug, Clone, PartialEq)]
pub enum CostSource {
    /// The §9.4 table, hard-content column.
    Phase0,
    /// This node's self-benchmark at start.
    Benchmark {
        /// Wall-clock the benchmark took.
        seconds: f64,
        /// Codecs it measured; the rest fell back to the table.
        measured: Vec<VideoCodec>,
    },
}

/// Per-pixel cost constants for one device, in nanoseconds per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct CostTable {
    pub encode: HashMap<VideoCodec, f64>,
    pub decode: HashMap<VideoCodec, f64>,
    /// Composition, per canvas pixel per tick, before the
    /// `1 + 0.1 × tiles` term.
    pub compose: f64,
    pub source: CostSource,
}

impl CostTable {
    /// The phase 0 measurements (§9.4), hard-content column: OpenH264,
    /// libvpx VP8/VP9, SVT-AV1 preset 12 with dav1d. H.265 has no binding
    /// and takes AV1's numbers as a conservative stand-in.
    pub fn phase0() -> Self {
        let mut encode = HashMap::new();
        let mut decode = HashMap::new();
        for (codec, enc, dec) in [
            (VideoCodec::H264, 4.4, 1.4),
            (VideoCodec::H265, 17.2, 1.9),
            (VideoCodec::VP8, 11.7, 1.8),
            (VideoCodec::VP9, 9.1, 1.7),
            (VideoCodec::AV1, 17.2, 1.9),
        ] {
            encode.insert(codec, enc);
            decode.insert(codec, dec);
        }
        Self {
            encode,
            decode,
            compose: 2.0,
            source: CostSource::Phase0,
        }
    }

    /// The dearest constant in a column: what an unknown codec is charged.
    fn worst(column: &HashMap<VideoCodec, f64>) -> f64 {
        column.values().copied().fold(0.0, f64::max)
    }

    pub fn encode_ns_per_px(&self, codec: VideoCodec) -> f64 {
        self.encode
            .get(&codec)
            .copied()
            .unwrap_or_else(|| Self::worst(&self.encode))
    }

    pub fn decode_ns_per_px(&self, codec: VideoCodec) -> f64 {
        self.decode
            .get(&codec)
            .copied()
            .unwrap_or_else(|| Self::worst(&self.decode))
    }
}

/// Work units a stream costs at `resolution` and `fps` for a constant of
/// `ns_per_px`, rounded up; never zero for a real stream.
pub fn units(ns_per_px: f64, resolution: Resolution, fps: u32) -> u32 {
    let ns_per_second = ns_per_px * resolution.pixels() as f64 * fps as f64;
    let units = (ns_per_second / NS_PER_UNIT).ceil();
    if ns_per_second > 0.0 {
        units.max(1.0) as u32
    } else {
        0
    }
}

/// One device's budget as advertised in cluster membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBudget {
    pub device: String,
    /// The codecs the device *encodes* (design §15.7, decision 5): what
    /// placement matches a meeting's first codec against.
    pub codecs: Vec<String>,
    /// The tightest engine's, on a device with engines.
    pub left: u32,
    pub total: u32,
    /// Per engine `(engine, left, total)`, on a GPU (§15.8).
    pub engines: Option<[(Engine, u32, u32); 3]>,
    /// The node's link `(left, total)` in kilobits a second
    /// (FORWARDING_ROOMS §3): on the host's line, since the link is the
    /// node's, whichever device draws the frames.
    pub egress: Option<(u32, u32)>,
}

/// The stages of the pipeline as engines that saturate separately on a
/// GPU (design §15.8, decision 1): NVENC, NVDEC and the compute cores.
/// The host has one pool for all three, its codec threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    Encode,
    Decode,
    Compose,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Encode, Engine::Decode, Engine::Compose];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Encode => "encode",
            Engine::Decode => "decode",
            Engine::Compose => "compose",
        }
    }
}

/// A cost in work units, per device and engine, and in egress: what a
/// join, a share or a room takes of each it touches. A room on a GPU
/// spends the device's decode engine on its sources, its compose engine
/// on its outputs and its encode engine on the flavors the device has,
/// and the host on the encodes it lacks. Every stream it sends leaves
/// through the node's link (FORWARDING_ROOMS §3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cost {
    units: Vec<(MediaDevice, Engine, u32)>,
    /// Kilobits a second sent: the composite to each subscriber, each
    /// forwarded stream to each viewer.
    egress_kbps: u32,
}

impl Cost {
    /// Whether it holds any engine's units, beside egress.
    pub fn has_units(&self) -> bool {
        self.units.iter().any(|(_, _, u)| *u > 0)
    }

    pub fn zero() -> Self {
        Self::default()
    }

    fn add(&mut self, device: &MediaDevice, engine: Engine, units: u32) {
        if units == 0 {
            return;
        }
        match self
            .units
            .iter_mut()
            .find(|(d, e, _)| d == device && *e == engine)
        {
            Some((_, _, u)) => *u = u.saturating_add(units),
            None => self.units.push((device.clone(), engine, units)),
        }
    }

    fn add_egress(&mut self, kbps: u32) {
        self.egress_kbps = self.egress_kbps.saturating_add(kbps);
    }

    fn extend(&mut self, other: &Cost) {
        for (d, e, u) in &other.units {
            self.add(d, *e, *u);
        }
        self.add_egress(other.egress_kbps);
    }

    /// Kilobits a second this sends out of the node.
    pub fn egress_kbps(&self) -> u32 {
        self.egress_kbps
    }

    /// Units on `device`, every engine together.
    pub fn on(&self, device: &MediaDevice) -> u32 {
        self.units
            .iter()
            .filter(|(d, _, _)| d == device)
            .fold(0u32, |a, (_, _, u)| a.saturating_add(*u))
    }

    /// Units on one engine of `device`.
    pub fn on_engine(&self, device: &MediaDevice, engine: Engine) -> u32 {
        self.units
            .iter()
            .find(|(d, e, _)| d == device && *e == engine)
            .map(|(_, _, u)| *u)
            .unwrap_or(0)
    }

    /// Units over every device and engine (egress apart).
    pub fn total(&self) -> u32 {
        self.units
            .iter()
            .fold(0u32, |a, (_, _, u)| a.saturating_add(*u))
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn is_zero(&self) -> bool {
        self.total() == 0 && self.egress_kbps == 0
    }

    /// The devices this cost touches, in order of first mention.
    fn devices(&self) -> Vec<MediaDevice> {
        let mut out: Vec<MediaDevice> = Vec::new();
        for (d, _, _) in &self.units {
            if !out.contains(d) {
                out.push(d.clone());
            }
        }
        out
    }
}

impl std::fmt::Display for Cost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.units.is_empty() && self.egress_kbps == 0 {
            return write!(f, "0");
        }
        if self.units.is_empty() {
            write!(f, "0")?;
        }
        for (i, d) in self.devices().iter().enumerate() {
            if i > 0 {
                write!(f, " + ")?;
            }
            write!(f, "{} on {d} (", self.on(d))?;
            let mut first = true;
            for e in Engine::ALL {
                let u = self.on_engine(d, e);
                if u > 0 {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{u} {}", e.name())?;
                    first = false;
                }
            }
            write!(f, ")")?;
        }
        if self.egress_kbps > 0 {
            write!(f, " + {} kb/s out", self.egress_kbps)?;
        }
        Ok(())
    }
}

/// One engine's (or the host's one) budget counters.
struct Pool {
    total: u32,
    /// Priced from the rooms at the last recompute.
    committed: AtomicU32,
    /// Admitted since the last recompute, not yet visible in any room.
    reserved: AtomicU32,
    /// Joins refused for want of budget, since start.
    refused: AtomicU64,
}

impl Pool {
    fn new(total: u32) -> Self {
        Self {
            total,
            committed: AtomicU32::new(0),
            reserved: AtomicU32::new(0),
            refused: AtomicU64::new(0),
        }
    }

    fn committed(&self) -> u32 {
        self.committed.load(Ordering::Relaxed)
    }

    fn reserved(&self) -> u32 {
        self.reserved.load(Ordering::Relaxed)
    }

    fn left(&self) -> u32 {
        self.total
            .saturating_sub(self.committed())
            .saturating_sub(self.reserved())
    }

    /// Take `units` if they fit; `false` refuses.
    fn reserve(&self, units: u32) -> bool {
        let committed = self.committed();
        let mut reserved = self.reserved();
        loop {
            if committed.saturating_add(reserved).saturating_add(units) > self.total {
                self.refused.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            match self.reserved.compare_exchange_weak(
                reserved,
                reserved + units,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(now) => reserved = now,
            }
        }
    }

    fn release(&self, units: u32) {
        let mut reserved = self.reserved();
        loop {
            let next = reserved.saturating_sub(units);
            match self.reserved.compare_exchange_weak(
                reserved,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(now) => reserved = now,
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total,
            "committed": self.committed(),
            "reserved": self.reserved(),
            "left": self.left(),
            "refused": self.refused.load(Ordering::Relaxed),
        })
    }
}

/// One device's budget: what it runs, what that costs, what is committed
/// and what a join may still take. A GPU has a pool per [`Engine`]; the
/// host one pool for all three (design §15.8, decision 1).
pub struct DeviceCapacity {
    device: MediaDevice,
    /// Codecs it encodes: what its budget advertises.
    encodes: Vec<VideoCodec>,
    decodes: Vec<VideoCodec>,
    table: CostTable,
    /// One pool (the host), or one per engine in [`Engine::ALL`]'s order.
    pools: Vec<Pool>,
}

impl DeviceCapacity {
    /// A device with `total` work units — per engine on a GPU, in all on
    /// the host.
    pub fn new(
        device: MediaDevice,
        encodes: Vec<VideoCodec>,
        decodes: Vec<VideoCodec>,
        table: CostTable,
        total: u32,
    ) -> Self {
        let pools = if device.is_host() {
            vec![Pool::new(total)]
        } else {
            Engine::ALL.iter().map(|_| Pool::new(total)).collect()
        };
        Self {
            device,
            encodes,
            decodes,
            table,
            pools,
        }
    }

    fn pool(&self, engine: Engine) -> &Pool {
        if self.pools.len() == 1 {
            &self.pools[0]
        } else {
            &self.pools[engine as usize]
        }
    }

    /// Whether the device has a pool per engine.
    pub fn has_engines(&self) -> bool {
        self.pools.len() > 1
    }

    /// The pool with the least left: what `left` and `total` mean for a
    /// device with engines.
    fn tightest(&self) -> &Pool {
        self.pools
            .iter()
            .min_by_key(|p| p.left())
            .expect("a device has a pool")
    }

    /// The host's total: `execution_units` codec-pool threads at
    /// `headroom` of their time (0.8 keeps a fifth for the async side and
    /// for content harder than the benchmark's).
    pub fn host_total(execution_units: usize, headroom: f64) -> u32 {
        (execution_units.max(1) as f64 * UNITS_PER_EXECUTION_UNIT * clamp_headroom(headroom))
            .round() as u32
    }

    /// A GPU's total: one execution unit — the device, whose engines the
    /// saturation benchmark priced as a whole (design §15.7, decision 7)
    /// — at `headroom`.
    pub fn device_total(headroom: f64) -> u32 {
        (UNITS_PER_EXECUTION_UNIT * clamp_headroom(headroom)).round() as u32
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn device(&self) -> &MediaDevice {
        &self.device
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn encodes(&self) -> &[VideoCodec] {
        &self.encodes
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn decodes(&self) -> &[VideoCodec] {
        &self.decodes
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn table(&self) -> &CostTable {
        &self.table
    }

    /// The tightest pool's total (the device's, on the host).
    pub fn total(&self) -> u32 {
        self.tightest().total
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn committed(&self) -> u32 {
        self.tightest().committed()
    }

    pub fn reserved(&self) -> u32 {
        self.tightest().reserved()
    }

    /// Units a new room or participant may still cost here: the tightest
    /// engine's.
    pub fn left(&self) -> u32 {
        self.tightest().left()
    }

    /// Refusals over every pool, since start.
    pub fn refused(&self) -> u64 {
        self.pools
            .iter()
            .map(|p| p.refused.load(Ordering::Relaxed))
            .sum()
    }

    /// One engine's units left (the device's, on the host).
    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn engine_left(&self, engine: Engine) -> u32 {
        self.pool(engine).left()
    }

    fn decode_units(&self, codec: VideoCodec, resolution: Resolution, fps: u32) -> u32 {
        units(self.table.decode_ns_per_px(codec), resolution, fps)
    }

    fn encode_units(&self, codec: VideoCodec, resolution: Resolution, fps: u32) -> u32 {
        units(self.table.encode_ns_per_px(codec), resolution, fps)
    }

    fn compose_units(&self, tiles: usize, resolution: Resolution, fps: u32) -> u32 {
        units(
            self.table.compose * (1.0 + 0.1 * tiles as f64),
            resolution,
            fps,
        )
    }

    /// The budget line for cluster membership: the tightest engine's
    /// `left` and `total`, and the engines when the device has them.
    pub fn budget(&self) -> DeviceBudget {
        DeviceBudget {
            device: self.device.to_string(),
            codecs: self
                .encodes
                .iter()
                .map(|c| c.sdp_name().to_string())
                .collect(),
            left: self.left(),
            total: self.total(),
            engines: self.has_engines().then(|| {
                Engine::ALL.map(|e| {
                    let p = self.pool(e);
                    (e, p.left(), p.total)
                })
            }),
            egress: None,
        }
    }

    /// For `/status`.
    pub fn json(&self) -> serde_json::Value {
        let mut codecs: Vec<VideoCodec> = self.encodes.clone();
        for c in &self.decodes {
            if !codecs.contains(c) {
                codecs.push(*c);
            }
        }
        codecs.sort_by_key(|c| c.sdp_name());
        let table: serde_json::Map<String, serde_json::Value> = codecs
            .iter()
            .map(|&c| {
                let mut row = serde_json::Map::new();
                if self.encodes.contains(&c) {
                    row.insert(
                        "encode_ns_per_px".into(),
                        round2(self.table.encode_ns_per_px(c)).into(),
                    );
                    // What one 720p15 stream of it costs, for a feel of the unit.
                    row.insert(
                        "encode_units_720p15".into(),
                        self.encode_units(c, Resolution::new(1280, 720), 15).into(),
                    );
                }
                if self.decodes.contains(&c) {
                    row.insert(
                        "decode_ns_per_px".into(),
                        round2(self.table.decode_ns_per_px(c)).into(),
                    );
                    row.insert(
                        "decode_units_720p15".into(),
                        self.decode_units(c, Resolution::new(1280, 720), 15).into(),
                    );
                }
                (c.sdp_name().to_string(), serde_json::Value::Object(row))
            })
            .collect();
        let engines: Option<serde_json::Value> = self.has_engines().then(|| {
            let mut m = serde_json::Map::new();
            for e in Engine::ALL {
                m.insert(e.name().to_string(), self.pool(e).json());
            }
            serde_json::Value::Object(m)
        });
        serde_json::json!({
            "device": self.device.to_string(),
            "unit": if self.device.is_host() {
                "one thousandth of one codec-pool thread's time per second"
            } else {
                "one thousandth of an engine's saturated throughput"
            },
            // The tightest engine's, on a device with engines.
            "total": self.total(),
            "committed": self.committed(),
            "reserved": self.reserved(),
            "left": self.left(),
            "refused": self.refused(),
            "engines": engines,
            "codecs": self.encodes.iter().map(|c| c.sdp_name()).collect::<Vec<_>>(),
            "decodes": self.decodes.iter().map(|c| c.sdp_name()).collect::<Vec<_>>(),
            "compose_ns_per_px": round2(self.table.compose),
            "table": table,
            "source": match &self.table.source {
                CostSource::Phase0 => serde_json::json!({ "kind": "phase0_table" }),
                CostSource::Benchmark { seconds, measured } => serde_json::json!({
                    "kind": "self_benchmark",
                    "seconds": round2(*seconds),
                    "measured": measured.iter().map(|c| c.sdp_name()).collect::<Vec<_>>(),
                }),
            },
        })
    }
}

fn clamp_headroom(headroom: f64) -> f64 {
    if headroom.is_finite() {
        headroom.clamp(0.05, 1.0)
    } else {
        0.8
    }
}

/// A node's video budget: one [`DeviceCapacity`] per device — the
/// device its rooms run on, and the host beside it when that is a GPU —
/// with every stage priced on the device it runs on (design §15.7,
/// block 8c). A join is admitted only when it fits on every device it
/// touches.
pub struct VideoCapacity {
    /// Where the rooms run.
    room_device: MediaDevice,
    devices: Vec<DeviceCapacity>,
    /// The node's link in kilobits a second (FORWARDING_ROOMS §3): what
    /// every composite and forwarded stream leaving the node shares.
    egress: Pool,
    last_recompute: parking_lot::Mutex<Option<Instant>>,
}

impl VideoCapacity {
    /// A budget on the devices given; `room_device` must be one of them
    /// and the host must be among them.
    pub fn new(room_device: MediaDevice, devices: Vec<DeviceCapacity>) -> Self {
        debug_assert!(devices.iter().any(|d| d.device == room_device));
        Self {
            room_device,
            devices,
            egress: Pool::new(DEFAULT_EGRESS_MBPS.saturating_mul(1000)),
            last_recompute: parking_lot::Mutex::new(None),
        }
    }

    /// The node's link, `[conference] video_egress_mbps`.
    pub fn with_egress_mbps(mut self, mbps: u32) -> Self {
        self.egress = Pool::new(mbps.max(1).saturating_mul(1000));
        self
    }

    /// Kilobits a second the node may still send.
    pub fn egress_left_kbps(&self) -> u32 {
        self.egress.left()
    }

    /// The node's link in kilobits a second.
    #[allow(dead_code)] // the budget's API: tests and the status
    pub fn egress_total_kbps(&self) -> u32 {
        self.egress.total
    }

    /// The host alone: `execution_units` codec-pool threads at
    /// `headroom`, running `codecs` both ways.
    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn host(
        codecs: Vec<VideoCodec>,
        table: CostTable,
        execution_units: usize,
        headroom: f64,
    ) -> Self {
        Self::new(
            MediaDevice::Host,
            vec![DeviceCapacity::new(
                MediaDevice::Host,
                codecs.clone(),
                codecs,
                table,
                DeviceCapacity::host_total(execution_units, headroom),
            )],
        )
    }

    /// The device the rooms run on.
    pub fn device(&self) -> &MediaDevice {
        &self.room_device
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn devices(&self) -> &[DeviceCapacity] {
        &self.devices
    }

    /// The room device's budget.
    fn room(&self) -> &DeviceCapacity {
        self.on(&self.room_device)
    }

    /// The budget of `device`, or the room device's when the node has no
    /// such device (a stage reported on a device this node never priced).
    fn on(&self, device: &MediaDevice) -> &DeviceCapacity {
        self.devices
            .iter()
            .find(|d| &d.device == device)
            .or_else(|| self.devices.iter().find(|d| d.device == self.room_device))
            .or_else(|| self.devices.first())
            .expect("a capacity has at least one device")
    }

    /// Where a `codec` would be decoded: the room's device when it has
    /// the decoder, the host otherwise (the room uploads what the host
    /// decodes).
    fn decoder_of(&self, codec: VideoCodec) -> &DeviceCapacity {
        let room = self.room();
        if room.decodes.contains(&codec) {
            return room;
        }
        self.devices
            .iter()
            .find(|d| d.decodes.contains(&codec))
            .unwrap_or(room)
    }

    /// Where a `codec` would be encoded: the room's device when it has
    /// the encoder, the host otherwise (the canvas is downloaded for it).
    fn encoder_of(&self, codec: VideoCodec) -> &DeviceCapacity {
        let room = self.room();
        if room.encodes.contains(&codec) {
            return room;
        }
        self.devices
            .iter()
            .find(|d| d.encodes.contains(&codec))
            .unwrap_or(room)
    }

    /// Total units on the room's device.
    pub fn total(&self) -> u32 {
        self.room().total()
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn committed(&self) -> u32 {
        self.room().committed()
    }

    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn reserved(&self) -> u32 {
        self.room().reserved()
    }

    /// Units a new room or participant may still cost on the room's
    /// device.
    pub fn left(&self) -> u32 {
        self.room().left()
    }

    /// Joins refused for want of budget on any device, since start.
    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn refused(&self) -> u64 {
        self.devices.iter().map(|d| d.refused()).sum()
    }

    /// What a shared screen costs to admit (§15.6): one more decode, at
    /// the content cap and the room's default rate, priced at the dearest
    /// codec this node decodes since the codec is the sender's choice.
    /// Asked before the floor is granted, so a share the node cannot
    /// afford is refused rather than the room slowed.
    pub fn content_cost(&self, settings: &VideoRoomSettings) -> Cost {
        let res = settings.content_max_resolution;
        let fps = settings.fps.max(1);
        let mut best: Option<(&DeviceCapacity, u32)> = None;
        for d in &self.devices {
            for &c in &d.decodes {
                let u = self.decoder_of(c).decode_units(c, res, fps);
                if best.map(|(_, b)| u > b).unwrap_or(true) {
                    best = Some((self.decoder_of(c), u));
                }
            }
        }
        let mut cost = Cost::zero();
        match best {
            Some((d, u)) => cost.add(&d.device, Engine::Decode, u),
            None => {
                let room = self.room();
                cost.add(
                    &room.device,
                    Engine::Decode,
                    units(CostTable::worst(&room.table.decode), res, fps),
                );
            }
        }
        cost
    }

    /// What a room's video costs now: a decode per source being decoded —
    /// a camera or a shared screen — at what it sends (the room's canvas
    /// until frames arrive), an encode per flavor, a composite per output
    /// at the room's target rate, each on the device the room says it
    /// runs on, and what it sends out of the node. A presentation
    /// composite has one more tile than the room has cameras, which
    /// `sources` already counts. A room that composes nothing — every
    /// subscriber forwarded, no recording — decodes nothing, and is
    /// charged only its egress (FORWARDING_ROOMS §2.1, §3).
    pub fn room_cost(&self, video: &VideoRoom) -> Cost {
        let st = video.status();
        let fps = st.target_fps.max(1);
        let mut cost = Cost::zero();
        let participants = video.participants();
        for p in &participants {
            for s in p
                .source
                .iter()
                .chain(p.content.iter())
                .filter(|s| s.decoding)
            {
                let resolution = if s.resolution.pixels() == 0 {
                    st.resolution
                } else {
                    s.resolution
                };
                let d = self.on(&s.device);
                cost.add(
                    &d.device,
                    Engine::Decode,
                    d.decode_units(s.codec, resolution, if s.fps > 0 { s.fps } else { fps }),
                );
            }
        }
        // A flavor every one of whose subscribers is on the passthrough
        // fast path is not being encoded, and an output with no flavor
        // left to encode is not being composed (§5.6). Charging for them
        // would keep reserving a budget nothing spends.
        let idle = idle_flavors(video, &st);
        for o in &st.outputs {
            let mut live = false;
            for f in o.flavors.iter().filter(|f| !idle.contains(&f.flavor)) {
                live = true;
                let d = self.on(&f.device);
                cost.add(
                    &d.device,
                    Engine::Encode,
                    d.encode_units(f.flavor.codec, f.flavor.resolution, f.flavor.fps.max(1)),
                );
            }
            if live {
                let d = self.on(&o.device);
                cost.add(
                    &d.device,
                    Engine::Compose,
                    d.compose_units(st.sources, o.resolution, fps),
                );
            }
        }
        // A trunk forwarding this node's people to a peer (§2.8): each
        // stream it is sent leaves the node like a viewer's.
        for (_, v) in video.trunk_viewers() {
            cost.add_egress(viewer_kbps(&v));
        }
        for p in &participants {
            if let Some(v) = &p.viewer {
                cost.add_egress(viewer_kbps(v));
            }
            if let Some(sub) = &p.subscription {
                cost.add_egress(default_kbps(sub.flavor.resolution));
            }
            if let (Some(sub), Some(_)) = (&p.content_subscription, &st.content) {
                cost.add_egress(default_kbps(sub.flavor.resolution));
            }
        }
        cost
    }

    /// What one more video participant would add: its decode, an encode
    /// (it may need a flavor of its own), and a composite when it starts
    /// the room's video or wants a canvas without itself — each where it
    /// would run. Priced at the dearest of `codecs`, since which one the
    /// caller ends up with is decided in the answer.
    ///
    /// A `forwarding` caller (FORWARDING_ROOMS §3) costs no encode or
    /// composite of its own: its decode, which the room spends only while
    /// something composes, and its egress — the default view's streams of
    /// those already in the room, and its own camera sent to each viewer
    /// there. A composite caller's egress is its flavor's rate.
    pub fn join_cost(
        &self,
        settings: &VideoRoomSettings,
        codecs: &[VideoCodec],
        room_video: Option<&VideoRoom>,
        forwarding: bool,
    ) -> Cost {
        if forwarding {
            return self.forwarding_join_cost(settings, codecs, room_video);
        }
        let fps = settings.fps.max(1);
        let mut dearest: Option<Cost> = None;
        for &c in codecs {
            let mut cost = Cost::zero();
            let dec = self.decoder_of(c);
            cost.add(
                &dec.device,
                Engine::Decode,
                dec.decode_units(c, settings.resolution, fps),
            );
            let enc = self.encoder_of(c);
            cost.add(
                &enc.device,
                Engine::Encode,
                enc.encode_units(c, settings.resolution, fps),
            );
            if dearest
                .as_ref()
                .map(|d| cost.total() > d.total())
                .unwrap_or(true)
            {
                dearest = Some(cost);
            }
        }
        let mut cost = dearest.unwrap_or_default();
        let room = self.room();
        let compose = match room_video {
            None => room.compose_units(1, settings.resolution, fps),
            Some(video) if settings.exclude_self => {
                room.compose_units(video.status().sources + 1, settings.resolution, fps)
            }
            Some(_) => 0,
        };
        cost.add(&room.device, Engine::Compose, compose);
        cost.add_egress(default_kbps(settings.resolution));
        if let Some(video) = room_video {
            cost.add_egress(viewers_of(video).saturating_mul(other_stream_kbps()));
        }
        cost
    }

    fn forwarding_join_cost(
        &self,
        settings: &VideoRoomSettings,
        codecs: &[VideoCodec],
        room_video: Option<&VideoRoom>,
    ) -> Cost {
        let fps = settings.fps.max(1);
        let mut cost = Cost::zero();
        let dearest = codecs
            .iter()
            .map(|&c| {
                let dec = self.decoder_of(c);
                (dec, dec.decode_units(c, settings.resolution, fps))
            })
            .max_by_key(|(_, u)| *u);
        if let Some((dec, u)) = dearest {
            cost.add(&dec.device, Engine::Decode, u);
        }
        if let Some(video) = room_video {
            let cameras = video
                .participants()
                .iter()
                .filter(|p| p.source.is_some())
                .count() as u32;
            cost.add_egress(default_view_kbps(cameras));
            cost.add_egress(viewers_of(video).saturating_mul(other_stream_kbps()));
        }
        cost
    }

    /// What one more of a webinar's audience would add
    /// (`docs/WEBINARS.md` §2.3): nothing decoded, as they send nothing,
    /// and only egress once the room composes for the audience — they
    /// share one composite of the whole room. The first of them pays for
    /// that composite and its encode. A forwarding attendee is sent the
    /// default view of the stage's cameras. Nobody is sent their camera.
    pub fn audience_join_cost(
        &self,
        settings: &VideoRoomSettings,
        codecs: &[VideoCodec],
        room_video: Option<&VideoRoom>,
        forwarding: bool,
        composite_paid: bool,
    ) -> Cost {
        let mut cost = Cost::zero();
        if forwarding {
            let cameras = room_video.map_or(0, |video| {
                video
                    .participants()
                    .iter()
                    .filter(|p| p.source.is_some())
                    .count() as u32
            });
            cost.add_egress(default_view_kbps(cameras));
            return cost;
        }
        cost.add_egress(default_kbps(settings.resolution));
        // `composite_paid`: a webinar's edge, which forwards its audience
        // the stage's composite (`docs/WEBINARS.md` §2.7), or another
        // attendee already holding the composite's encode until the next
        // recompute sees it.
        let shared = composite_paid
            || room_video.is_some_and(|video| {
                video
                    .status()
                    .outputs
                    .iter()
                    .any(|o| o.scope == OutputScope::All && o.view == View::Composite)
            });
        if !shared {
            let fps = settings.fps.max(1);
            let dearest = codecs
                .iter()
                .map(|&c| {
                    let enc = self.encoder_of(c);
                    (enc, enc.encode_units(c, settings.resolution, fps))
                })
                .max_by_key(|(_, u)| *u);
            if let Some((enc, u)) = dearest {
                cost.add(&enc.device, Engine::Encode, u);
            }
            let room = self.room();
            let tiles = room_video.map_or(1, |v| v.status().sources.max(1) as u32);
            cost.add(
                &room.device,
                Engine::Compose,
                room.compose_units(tiles as usize, settings.resolution, fps),
            );
        }
        cost
    }

    /// Admit a join costing `cost` if it fits on every device it
    /// touches, holding the units until the next recompute sees the
    /// participant in a room. `false` refuses, and holds nothing.
    pub fn try_reserve(&self, cost: &Cost) -> bool {
        let mut taken: Vec<(&Pool, u32)> = Vec::new();
        let wanted = cost
            .units
            .iter()
            .map(|(device, engine, units)| (self.on(device).pool(*engine), *units))
            .chain((cost.egress_kbps > 0).then_some((&self.egress, cost.egress_kbps)));
        for (pool, units) in wanted {
            if pool.reserve(units) {
                taken.push((pool, units));
            } else {
                for (p, u) in taken {
                    p.release(u);
                }
                return false;
            }
        }
        true
    }

    /// Re-price every room and drop the reservations; returns what is
    /// committed.
    pub fn recompute(&self, rooms: impl IntoIterator<Item = Arc<ConferenceRoom>>) -> Cost {
        let mut committed = Cost::zero();
        for room in rooms {
            if let Some(video) = room.video() {
                committed.extend(&self.room_cost(&video));
            }
        }
        for d in &self.devices {
            if d.has_engines() {
                for e in Engine::ALL {
                    let p = d.pool(e);
                    p.committed
                        .store(committed.on_engine(&d.device, e), Ordering::Relaxed);
                    p.reserved.store(0, Ordering::Relaxed);
                }
            } else {
                let p = d.pool(Engine::Encode);
                p.committed
                    .store(committed.on(&d.device), Ordering::Relaxed);
                p.reserved.store(0, Ordering::Relaxed);
            }
        }
        self.egress
            .committed
            .store(committed.egress_kbps, Ordering::Relaxed);
        self.egress.reserved.store(0, Ordering::Relaxed);
        *self.last_recompute.lock() = Some(Instant::now());
        committed
    }

    /// The budget lines for cluster membership, the room device's first.
    pub fn budgets(&self) -> Vec<DeviceBudget> {
        self.devices
            .iter()
            .map(|d| {
                let mut b = d.budget();
                if d.device.is_host() {
                    b.egress = Some((self.egress.left(), self.egress.total));
                }
                b
            })
            .collect()
    }

    /// The room device's budget line.
    #[allow(dead_code)] // the budget's API: tests, the box test, and callers to come
    pub fn budget(&self) -> DeviceBudget {
        self.room().budget()
    }

    /// For `/status`: the room device's figures at the top, every
    /// device under `devices`.
    pub fn json(&self) -> serde_json::Value {
        let mut json = self.room().json();
        json["devices"] = serde_json::Value::Array(self.devices.iter().map(|d| d.json()).collect());
        // Kilobits a second out of the node (FORWARDING_ROOMS §3).
        json["egress"] = self.egress.json();
        json["last_recompute_secs_ago"] =
            serde_json::json!(self.last_recompute.lock().map(|t| t.elapsed().as_secs()));
        json
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Price the host's codecs by running each for a share of `seconds` on
/// the host (design §9.3: "re-measured per node and per device at
/// start"). `seconds == 0` skips the run and uses `fallback` as it is.
/// A codec that fails to measure keeps its fallback constant. Composition
/// is timed once on a 720p nine-tile grid. Blocking: run it off the async
/// runtime.
pub fn self_benchmark(
    backend: &VideoBackend,
    codecs: &[VideoCodec],
    seconds: u64,
    fallback: CostTable,
) -> CostTable {
    if seconds == 0 || codecs.is_empty() {
        info!("Video self-benchmark skipped; using the phase 0 cost table");
        return fallback;
    }
    let started = Instant::now();
    let mut table = fallback;
    let per_codec = Duration::from_secs_f64(seconds as f64 / (codecs.len() as f64 + 1.0));
    let settings = BenchSettings {
        frames: 60,
        budget: per_codec,
        ..BenchSettings::default()
    };
    let mut measured = Vec::new();
    for &codec in codecs {
        match bench::measure_codec(&backend.registry, &MediaDevice::Host, codec, &settings) {
            Ok(cost) => {
                info!(
                    device = "host",
                    codec = %codec.sdp_name(),
                    encode_ns_per_px = format!("{:.2}", cost.encode_ns_per_px),
                    decode_ns_per_px = format!("{:.2}", cost.decode_ns_per_px),
                    encode_streams_per_thread = format!("{:.1}", cost.encode_streams_per_unit()),
                    frames = cost.frames,
                    complete = cost.complete,
                    "Video self-benchmark"
                );
                table.encode.insert(codec, cost.encode_ns_per_px);
                table.decode.insert(codec, cost.decode_ns_per_px);
                measured.push(codec);
            }
            Err(e) => {
                warn!(codec = %codec.sdp_name(), error = %e, "Video self-benchmark failed for codec; keeping the table's constant");
            }
        }
    }
    table.compose = bench::measure_compose(Resolution::new(1280, 720), 9, 5);
    info!(
        device = "host",
        compose_ns_per_px = format!("{:.2}", table.compose),
        "Video self-benchmark: composition"
    );
    table.source = CostSource::Benchmark {
        seconds: started.elapsed().as_secs_f64(),
        measured,
    };
    table
}

/// The stream the device benchmark saturates with: 720p30, the design's
/// unit of GPU capacity (§9.1).
#[allow(dead_code)] // the device benchmark's, behind `video-hwaccel`
pub const DEVICE_BENCH_RESOLUTION: Resolution = Resolution {
    width: 1280,
    height: 720,
};
#[allow(dead_code)] // the device benchmark's, behind `video-hwaccel`
pub const DEVICE_BENCH_FPS: u32 = 30;

/// The per-pixel constant that makes `streams` real-time streams of
/// [`DEVICE_BENCH_RESOLUTION`] at [`DEVICE_BENCH_FPS`] fill one execution
/// unit (design §15.7, decision 7): a GPU is priced by what it sustains,
/// not by what one stream takes.
#[allow(dead_code)] // the device benchmark's, behind `video-hwaccel`
pub fn ns_per_px_for_streams(streams: f64) -> f64 {
    let streams = streams.max(0.1);
    let units_per_stream = UNITS_PER_EXECUTION_UNIT / streams;
    units_per_stream * NS_PER_UNIT
        / (DEVICE_BENCH_RESOLUTION.pixels() as f64 * DEVICE_BENCH_FPS as f64)
}

/// Price a GPU's codecs by saturation (design §15.7, decision 7): each
/// stage of each codec is run with 1, 2, 4, … streams for a share of
/// `seconds` until its throughput stops growing, and the constant is
/// what makes that many real-time 720p30 streams fill the device.
/// Composition is timed on the device's compositor. `seconds == 0`
/// skips the run and uses `fallback`.
#[cfg(feature = "video-hwaccel")]
pub fn self_benchmark_device(
    backend: &VideoBackend,
    encodes: &[VideoCodec],
    decodes: &[VideoCodec],
    seconds: u64,
    fallback: CostTable,
) -> CostTable {
    use forge_video_hw::bench::{saturate, Stage};
    if seconds == 0 || (encodes.is_empty() && decodes.is_empty()) {
        info!(device = %backend.device, "Video device benchmark skipped; using the phase 0 cost table");
        return fallback;
    }
    let started = Instant::now();
    let mut table = fallback;
    // Each stage steps 1, 2, 4, 8 streams, the composite too; a step is
    // at least a second whatever the budget says (§15.8, decision 2:
    // short steps overshoot), plus the bench's own warm-up.
    let runs = (encodes.len() + decodes.len() + 1) as f64 * 4.0;
    let per_step = Duration::from_secs_f64((seconds as f64 / runs).max(1.0));
    let mut measured = Vec::new();
    let run = |codec: VideoCodec, stage: Stage| -> Option<f64> {
        match saturate(
            &backend.registry,
            &backend.device,
            codec,
            stage,
            DEVICE_BENCH_RESOLUTION,
            DEVICE_BENCH_FPS,
            per_step,
            8,
        ) {
            Ok(sat) => {
                let ns = ns_per_px_for_streams(sat.realtime_streams);
                info!(
                    device = %backend.device,
                    codec = %codec.sdp_name(),
                    stage = ?stage,
                    streams = sat.streams,
                    realtime_streams = format!("{:.1}", sat.realtime_streams),
                    saturated = sat.saturated,
                    ns_per_px = format!("{:.3}", ns),
                    "Video device benchmark"
                );
                Some(ns)
            }
            Err(e) => {
                warn!(device = %backend.device, codec = %codec.sdp_name(), stage = ?stage, error = %e,
                      "Video device benchmark failed; keeping the table's constant");
                None
            }
        }
    };
    for &codec in encodes {
        if let Some(ns) = run(codec, Stage::Encode) {
            table.encode.insert(codec, ns);
            if !measured.contains(&codec) {
                measured.push(codec);
            }
        }
    }
    for &codec in decodes {
        if let Some(ns) = run(codec, Stage::Decode) {
            table.decode.insert(codec, ns);
            if !measured.contains(&codec) {
                measured.push(codec);
            }
        }
    }
    match bench::measure_compose_saturated(&*backend.hw, DEVICE_BENCH_RESOLUTION, 9, per_step, 8) {
        Ok(sat) => {
            table.compose = sat.ns_per_px();
            info!(device = %backend.device, graphs = sat.graphs,
                  ticks_per_second = format!("{:.0}", sat.ticks_per_second),
                  saturated = sat.saturated,
                  compose_ns_per_px = format!("{:.3}", table.compose),
                  "Video device benchmark: composition");
        }
        Err(e) => {
            warn!(device = %backend.device, error = %e, "Video device benchmark: composition failed; keeping the table's constant");
        }
    }
    info!(device = %backend.device, seconds = format!("{:.1}", started.elapsed().as_secs_f64()),
          per_step_secs = format!("{:.1}", per_step.as_secs_f64()),
          "Video device benchmark done");
    table.source = CostSource::Benchmark {
        seconds: started.elapsed().as_secs_f64(),
        measured,
    };
    table
}

/// The flavors nothing is encoding right now: every subscriber of them
/// is being forwarded a sender's own stream instead, and no recording
/// wants the composite either (§5.6).
fn idle_flavors(
    video: &VideoRoom,
    st: &forge_conference::video::VideoRoomStatus,
) -> std::collections::HashSet<forge_video::flavor::Flavor> {
    use std::collections::{HashMap, HashSet};
    let mut total: HashMap<forge_video::flavor::Flavor, (usize, usize)> = HashMap::new();
    for p in video.participants() {
        if let Some(sub) = p.subscription {
            let e = total.entry(sub.flavor).or_insert((0, 0));
            e.0 += 1;
            if sub.forwarding.is_some() {
                e.1 += 1;
            }
        }
    }
    let recorded: HashSet<forge_video::flavor::Flavor> =
        st.recordings.iter().map(|r| r.flavor.clone()).collect();
    total
        .into_iter()
        .filter(|(flavor, (subs, forwarded))| {
            *subs > 0 && subs == forwarded && !recorded.contains(flavor)
        })
        .map(|(flavor, _)| flavor)
        .collect()
}

/// What a viewer is being sent (FORWARDING_ROOMS §3): each stream that
/// is not paused at its layer's nominal rate, no more than its REMB.
fn viewer_kbps(v: &VideoViewerInfo) -> u32 {
    let nominal = v
        .streams
        .iter()
        .filter(|s| !s.paused)
        .fold(0u32, |a, s| a.saturating_add(default_kbps(s.resolution)));
    if v.remb_kbps > 0 {
        nominal.min(v.remb_kbps)
    } else {
        nominal
    }
}

/// One of the default view's other streams, at its height.
fn other_stream_kbps() -> u32 {
    default_kbps(Resolution::new(
        DEFAULT_OTHER_HEIGHT * 16 / 9,
        DEFAULT_OTHER_HEIGHT,
    ))
}

/// The default view in a room of `cameras` others: the speaker at 720p
/// and up to eight of them at 180p (§8 Q12).
fn default_view_kbps(cameras: u32) -> u32 {
    if cameras == 0 {
        return 0;
    }
    let speaker = default_kbps(Resolution::new(
        DEFAULT_SPEAKER_HEIGHT * 16 / 9,
        DEFAULT_SPEAKER_HEIGHT,
    ));
    let others = cameras.min(DEFAULT_MAX_STREAMS as u32 - 1);
    speaker.saturating_add(others.saturating_mul(other_stream_kbps()))
}

/// How many in the room are being forwarded streams: each would be sent
/// a newcomer's camera too.
fn viewers_of(video: &VideoRoom) -> u32 {
    video
        .participants()
        .iter()
        .filter(|p| p.viewer.is_some())
        .count() as u32
}

/// The room asks before it grants the floor (§15.6): a share is one more
/// decode, reserved like a join and priced into `committed` once the next
/// recompute sees the content source in the room.
impl ContentGate for VideoCapacity {
    fn admit(&self, room: &str, participant: &str, settings: &VideoRoomSettings) -> bool {
        let cost = self.content_cost(settings);
        let admitted = self.try_reserve(&cost);
        if !admitted {
            warn!(
                room,
                participant,
                %cost,
                left = self.left(),
                "screen share refused: over the video budget"
            );
        }
        admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_conference::video::SubscribeRequest;
    use forge_conference::AudioFormat;

    fn capacity(units: usize) -> VideoCapacity {
        VideoCapacity::host(
            vec![VideoCodec::H264, VideoCodec::VP8],
            CostTable::phase0(),
            units,
            0.8,
        )
    }

    /// A GPU node: the device encodes H.264 only and decodes H.264 and
    /// VP8, at a total of `device_units`; the host beside it encodes
    /// both with one thread.
    fn gpu_capacity(device_units: u32) -> (MediaDevice, VideoCapacity) {
        let gpu = MediaDevice::parse("cuda:0").unwrap();
        let mut table = CostTable::phase0();
        // A saturated engine: 60 real-time 720p30 H.264 encodes.
        table
            .encode
            .insert(VideoCodec::H264, ns_per_px_for_streams(60.0));
        table
            .decode
            .insert(VideoCodec::H264, ns_per_px_for_streams(240.0));
        table
            .decode
            .insert(VideoCodec::VP8, ns_per_px_for_streams(240.0));
        table.compose = 3.5;
        let cap = VideoCapacity::new(
            gpu.clone(),
            vec![
                DeviceCapacity::new(
                    gpu.clone(),
                    vec![VideoCodec::H264],
                    vec![VideoCodec::H264, VideoCodec::VP8],
                    table,
                    device_units,
                ),
                DeviceCapacity::new(
                    MediaDevice::Host,
                    vec![VideoCodec::H264, VideoCodec::VP8],
                    vec![VideoCodec::H264, VideoCodec::VP8],
                    CostTable::phase0(),
                    DeviceCapacity::host_total(1, 0.8),
                ),
            ],
        );
        (gpu, cap)
    }

    fn settings(exclude_self: bool) -> VideoRoomSettings {
        VideoRoomSettings {
            resolution: Resolution::new(1280, 720),
            fps: 15,
            exclude_self,
            ..VideoRoomSettings::default()
        }
    }

    #[test]
    fn a_unit_is_a_millicore_and_the_table_prices_as_the_design_says() {
        // §9.3's worked example: 720p15 H.264 hard content ≈ 61 units.
        assert_eq!(units(4.4, Resolution::new(1280, 720), 15), 61);
        assert_eq!(units(0.0, Resolution::new(1280, 720), 15), 0);
        assert_eq!(units(0.0001, Resolution::new(16, 16), 1), 1, "never free");
        let cap = capacity(10);
        assert_eq!(cap.total(), 8000, "ten threads at 0.8 headroom");
        assert_eq!(cap.left(), 8000);
        let t = CostTable::phase0();
        assert_eq!(t.encode_ns_per_px(VideoCodec::VP8), 11.7);
        assert_eq!(t.decode_ns_per_px(VideoCodec::H265), 1.9);
    }

    #[test]
    fn a_join_is_priced_conservatively_and_reserved_until_recompute() {
        let cap = capacity(1);
        assert_eq!(cap.total(), 800);
        // First participant of a room: decode + encode + compose (1 tile).
        let first = cap.join_cost(&settings(false), &[VideoCodec::H264], None, false);
        let h264 =
            units(4.4, Resolution::new(1280, 720), 15) + units(1.4, Resolution::new(1280, 720), 15);
        let compose1 = units(2.0 * 1.1, Resolution::new(1280, 720), 15);
        assert_eq!(first.total(), h264 + compose1);
        assert_eq!(
            first.on(&MediaDevice::Host),
            first.total(),
            "all on the host"
        );
        assert!(
            first
                .to_string()
                .starts_with(&format!("{} on host (", first.total())),
            "{first}"
        );
        assert_eq!(
            first.on_engine(&MediaDevice::Host, Engine::Encode),
            h264 - units(1.4, Resolution::new(1280, 720), 15)
        );
        assert_eq!(
            first.on_engine(&MediaDevice::Host, Engine::Compose),
            compose1
        );
        // The dearest allowed codec sets the price.
        let either = cap.join_cost(
            &settings(false),
            &[VideoCodec::H264, VideoCodec::VP8],
            None,
            false,
        );
        assert!(either.total() > first.total());

        assert!(cap.try_reserve(&first));
        assert_eq!(cap.left(), 800 - first.total());
        // Ten more do not fit in one thread.
        let mut admitted = 1;
        while cap.try_reserve(&first) {
            admitted += 1;
        }
        assert!((4..10).contains(&admitted), "{admitted}");
        assert_eq!(cap.refused(), 1);
        // A recompute over no rooms frees everything.
        assert!(cap.recompute(Vec::<Arc<ConferenceRoom>>::new()).is_zero());
        assert_eq!(cap.left(), 800);
    }

    /// The room's clock task needs a runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_room_is_priced_from_its_sources_flavors_and_outputs() {
        let room = Arc::new(
            ConferenceRoom::new(
                "3000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        let backend = VideoBackend::raw();
        let video = room.enable_video(
            VideoRoomSettings {
                resolution: Resolution::new(640, 360),
                fps: 15,
                ..VideoRoomSettings::default()
            },
            &backend,
        );
        let cap = VideoCapacity::host(vec![VideoCodec::VP8], CostTable::phase0(), 4, 1.0);
        assert!(
            cap.room_cost(&video).is_zero(),
            "no sources, no outputs yet"
        );

        video.add_source("alice", VideoCodec::VP8, "").unwrap();
        let sub = video
            .subscribe(
                "bob",
                SubscribeRequest {
                    codec: VideoCodec::VP8,
                    profile: String::new(),
                    payload_type: 97,
                    resolution: None,
                    fps: None,
                    max_kbps: None,
                    scope: None,
                    view: None,
                },
            )
            .unwrap();
        let cost = cap.room_cost(&video);
        let res = Resolution::new(640, 360);
        let expected = units(1.8, res, 15)            // alice's decode at the canvas size
            + units(11.7, res, 15)                   // bob's VP8 flavor
            + units(2.0 * 1.1, res, 15); // one output, one source tile
        assert_eq!(cost.total(), expected);

        // Joining an existing room with a canvas to share adds no composite;
        // asking for one without yourself does.
        let more = cap.join_cost(&settings(false), &[VideoCodec::VP8], Some(&video), false);
        assert_eq!(
            more.total(),
            units(1.8, Resolution::new(1280, 720), 15)
                + units(11.7, Resolution::new(1280, 720), 15)
        );
        let own = cap.join_cost(&settings(true), &[VideoCodec::VP8], Some(&video), false);
        assert!(own.total() > more.total());

        // The recompute over this room commits exactly its cost.
        let mut five = Cost::zero();
        five.add(&MediaDevice::Host, Engine::Encode, 5);
        cap.try_reserve(&five);
        assert_eq!(cap.recompute(vec![Arc::clone(&room)]).total(), expected);
        assert_eq!(cap.reserved(), 0);
        assert_eq!(cap.left(), cap.total() - expected);
        let budget = cap.budget();
        assert_eq!(budget.device, "host");
        assert_eq!(budget.codecs, vec!["VP8"]);
        assert_eq!(budget.total, 4000);
        assert_eq!(cap.budgets().len(), 1);
        assert_eq!(cap.json()["left"], cap.left());
        assert_eq!(cap.json()["devices"].as_array().unwrap().len(), 1);
        drop(sub);
        room.disable_video();
    }

    /// FORWARDING_ROOMS §3: a room whose subscribers are all forwarded
    /// decodes and encodes nothing and is charged only what it sends; a
    /// forwarding join is priced at its egress and the link refuses what
    /// it cannot carry.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forwarding_room_is_charged_its_egress_and_no_codec_time() {
        use forge_conference::video::ForwardRequest;
        let room = Arc::new(
            ConferenceRoom::new(
                "3001",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        for id in ["alice", "bob"] {
            room.add_participant(id, true).unwrap();
        }
        let res = Resolution::new(640, 360);
        let video = room.enable_video(
            VideoRoomSettings {
                resolution: res,
                fps: 15,
                ..VideoRoomSettings::default()
            },
            &VideoBackend::raw(),
        );
        let cap = VideoCapacity::host(vec![VideoCodec::VP8], CostTable::phase0(), 4, 1.0)
            .with_egress_mbps(3);
        assert_eq!(cap.egress_total_kbps(), 3000);
        video.add_source("alice", VideoCodec::VP8, "").unwrap();
        video.add_source("bob", VideoCodec::VP8, "").unwrap();

        // Bob takes the composite: two decodes, his flavor, one output,
        // and the composite's rate out.
        let sub = video
            .subscribe(
                "bob",
                SubscribeRequest {
                    codec: VideoCodec::VP8,
                    profile: String::new(),
                    payload_type: 97,
                    resolution: None,
                    fps: None,
                    max_kbps: None,
                    scope: None,
                    view: None,
                },
            )
            .unwrap();
        let mixed = cap.room_cost(&video);
        assert_eq!(
            mixed.on_engine(&MediaDevice::Host, Engine::Decode),
            2 * units(1.8, res, 15)
        );
        assert_eq!(mixed.egress_kbps(), default_kbps(res), "{mixed}");
        assert!(mixed.to_string().ends_with("kb/s out"), "{mixed}");
        drop(sub);

        // Both forwarded: once the room's clock has seen nothing to
        // compose, nothing is decoded and nothing is charged for it.
        let forward = ForwardRequest {
            codec: VideoCodec::VP8,
            profile: String::new(),
            payload_type: 97,
            transport_cc: None,
        };
        let _a = video.forward("alice", forward.clone()).unwrap();
        let _b = video.forward("bob", forward.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while video
            .participants()
            .iter()
            .any(|p| p.source.as_ref().is_some_and(|s| s.decoding))
        {
            assert!(
                Instant::now() < deadline,
                "the sources never stopped decoding"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let forwarded = cap.room_cost(&video);
        assert_eq!(forwarded.total(), 0, "no codec time: {forwarded}");

        // A forwarding join: its decode, the default view of the two
        // cameras there (the speaker at 720p, both at 180p), and its
        // camera to each of the two viewers.
        let join = cap.join_cost(&settings(false), &[VideoCodec::VP8], Some(&video), true);
        assert_eq!(join.on_engine(&MediaDevice::Host, Engine::Encode), 0);
        assert_eq!(join.on_engine(&MediaDevice::Host, Engine::Compose), 0);
        assert_eq!(
            join.on_engine(&MediaDevice::Host, Engine::Decode),
            units(1.8, Resolution::new(1280, 720), 15)
        );
        assert_eq!(join.egress_kbps(), 1200 + 2 * 200 + 2 * 200);
        // A composite join sends its flavor and is sent to the viewers.
        let mix = cap.join_cost(&settings(false), &[VideoCodec::VP8], Some(&video), false);
        assert_eq!(mix.egress_kbps(), 1200 + 2 * 200);

        // The link: one fits in 3 Mb/s, a second does not, and the
        // refusal holds nothing — not even the codec units it fit.
        let units_left = cap.left();
        assert!(cap.try_reserve(&join));
        assert_eq!(cap.egress_left_kbps(), 3000 - 2000);
        let units_after_one = cap.left();
        assert!(units_after_one < units_left);
        assert!(!cap.try_reserve(&join), "over the link");
        assert_eq!(
            cap.left(),
            units_after_one,
            "the refused join's units let go"
        );
        assert_eq!(cap.egress_left_kbps(), 1000);
        let budget = cap.budgets().remove(0);
        assert_eq!(budget.egress, Some((1000, 3000)));
        assert_eq!(cap.json()["egress"]["total"], 3000);

        // The recompute commits what the room sends (its viewers' streams,
        // none of which has a frame to carry yet) and drops the hold.
        let committed = cap.recompute(vec![Arc::clone(&room)]);
        assert_eq!(cap.egress_left_kbps(), 3000 - committed.egress_kbps());
        room.disable_video();
    }

    #[tokio::test]
    async fn an_attendee_is_priced_at_what_they_watch() {
        let room = Arc::new(
            ConferenceRoom::new(
                "3002",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        room.add_listener("x").unwrap();
        let res = Resolution::new(640, 360);
        let settings = VideoRoomSettings {
            resolution: res,
            fps: 15,
            exclude_self: true,
            ..VideoRoomSettings::default()
        };
        let video = room.enable_video(settings.clone(), &VideoBackend::raw());
        let cap = VideoCapacity::host(vec![VideoCodec::VP8], CostTable::phase0(), 4, 1.0);
        video.add_source("alice", VideoCodec::VP8, "").unwrap();
        video.add_source("bob", VideoCodec::VP8, "").unwrap();
        let vp8 = [VideoCodec::VP8];

        // The first attendee watching starts the audience's composite.
        let first = cap.audience_join_cost(&settings, &vp8, Some(&video), false, false);
        assert_eq!(first.on_engine(&MediaDevice::Host, Engine::Decode), 0);
        assert!(first.on_engine(&MediaDevice::Host, Engine::Encode) > 0);
        assert!(first.on_engine(&MediaDevice::Host, Engine::Compose) > 0);
        assert_eq!(first.egress_kbps(), default_kbps(res));
        assert!(first.has_units());
        // On a webinar's edge, or with another join already paying for it:
        // the picture's rate out alone.
        let paid = cap.audience_join_cost(&settings, &vp8, Some(&video), false, true);
        assert_eq!(paid.total(), 0, "{paid}");
        assert!(!paid.has_units());
        assert_eq!(paid.egress_kbps(), default_kbps(res));
        let _x = video
            .subscribe(
                "x",
                SubscribeRequest {
                    codec: VideoCodec::VP8,
                    profile: String::new(),
                    payload_type: 97,
                    resolution: None,
                    fps: None,
                    max_kbps: None,
                    scope: None,
                    view: None,
                },
            )
            .unwrap();
        // Every one after shares it: their picture's rate out, nothing more.
        let next = cap.audience_join_cost(&settings, &vp8, Some(&video), false, false);
        assert_eq!(next.total(), 0, "{next}");
        assert_eq!(next.egress_kbps(), default_kbps(res));
        // A stage member costs a decode, and sends to the room.
        let stage = cap.join_cost(&settings, &vp8, Some(&video), false);
        assert!(stage.on_engine(&MediaDevice::Host, Engine::Decode) > 0);
        // A forwarding attendee: the default view of the two cameras.
        let fwd = cap.audience_join_cost(&settings, &vp8, Some(&video), true, false);
        assert_eq!(fwd.total(), 0);
        assert_eq!(fwd.egress_kbps(), 1200 + 2 * 200);
        room.disable_video();
    }

    #[test]
    fn a_join_on_a_gpu_node_is_priced_where_each_stage_runs() {
        let (gpu, cap) = gpu_capacity(DeviceCapacity::device_total(1.0));
        assert_eq!(cap.device(), &gpu);
        assert_eq!(cap.total(), 1000, "one execution unit: the device");
        let res = Resolution::new(1280, 720);
        // H.264: decode, encode and composite all on the device. Sixty
        // real-time 720p30 encodes fill it, so one 720p15 encode is a
        // 120th of it, and the decode a 480th.
        let h264 = cap.join_cost(&settings(false), &[VideoCodec::H264], None, false);
        assert_eq!(h264.on(&MediaDevice::Host), 0);
        assert_eq!(
            h264.on(&gpu),
            units(ns_per_px_for_streams(60.0), res, 15)
                + units(ns_per_px_for_streams(240.0), res, 15)
                + units(3.5 * 1.1, res, 15)
        );
        assert_eq!(
            units(ns_per_px_for_streams(60.0), res, 15),
            9,
            "1000 / 60 / 2, rounded up"
        );
        // VP8: decoded on the device, encoded on the host, composed on
        // the device.
        let vp8 = cap.join_cost(&settings(false), &[VideoCodec::VP8], None, false);
        assert_eq!(vp8.on(&MediaDevice::Host), units(11.7, res, 15));
        assert_eq!(
            vp8.on(&gpu),
            units(ns_per_px_for_streams(240.0), res, 15) + units(3.5 * 1.1, res, 15)
        );
        assert!(vp8.to_string().contains("on host"));
        assert!(vp8.to_string().contains("on cuda:0"));
        // The budgets advertise what each device encodes.
        let budgets = cap.budgets();
        assert_eq!(budgets.len(), 2);
        assert_eq!(budgets[0].device, "cuda:0");
        assert_eq!(budgets[0].codecs, vec!["H264"]);
        assert_eq!(budgets[1].device, "host");
        assert_eq!(budgets[1].codecs, vec!["H264", "VP8"]);
        assert_eq!(cap.json()["devices"].as_array().unwrap().len(), 2);
        assert_eq!(cap.json()["device"], "cuda:0");
        // The GPU has an engine per stage on the wire; the host has none.
        let engines = budgets[0].engines.expect("a GPU has engines");
        assert_eq!(engines[0], (Engine::Encode, 1000, 1000));
        assert!(budgets[1].engines.is_none());
        assert!(cap.json()["devices"][0]["engines"]["compose"]["total"] == 1000);
        assert!(cap.json()["devices"][1]["engines"].is_null());
        // A VP8 join is charged to the engines it uses: the host's encode,
        // the device's decode and compose.
        assert_eq!(vp8.on_engine(&gpu, Engine::Encode), 0);
        assert!(vp8.on_engine(&gpu, Engine::Decode) > 0);
        assert!(vp8.on_engine(&gpu, Engine::Compose) > 0);
        assert!(vp8.on_engine(&MediaDevice::Host, Engine::Encode) > 0);
    }

    #[test]
    fn an_engine_out_of_budget_refuses_what_needs_it_and_admits_what_does_not() {
        // A device whose engines each hold 120 units: about two H.264
        // joins' composites, many more encodes and decodes.
        let (gpu, cap) = gpu_capacity(120);
        let h264 = cap.join_cost(&settings(false), &[VideoCodec::H264], None, false);
        let compose = h264.on_engine(&gpu, Engine::Compose);
        assert!(compose > h264.on_engine(&gpu, Engine::Encode));
        let mut joins = 0;
        while cap.try_reserve(&h264) {
            joins += 1;
        }
        assert_eq!(joins, 120 / compose, "the compose engine is what ran out");
        let dev = &cap.devices()[0];
        assert!(dev.engine_left(Engine::Compose) < compose);
        assert!(
            dev.engine_left(Engine::Encode) > compose,
            "encode has plenty"
        );
        assert_eq!(
            dev.left(),
            dev.engine_left(Engine::Compose),
            "left is the tightest engine's"
        );
        // A share is a decode alone: admitted while the compose engine is
        // out, because it does not touch it.
        let share = cap.content_cost(&settings(false));
        assert_eq!(share.on_engine(&gpu, Engine::Compose), 0);
        assert!(share.on_engine(&gpu, Engine::Decode) > 0);
        assert!(cap.try_reserve(&share));
        // The gate prices the share at the room's own cap, not defaults:
        // a room capping content lower pays less.
        let mut small = settings(false);
        small.content_max_resolution = Resolution::new(640, 360);
        assert!(cap.content_cost(&small).total() < share.total());
        // The wire carries the engines: placement reads the encoder's.
        let b = cap.budgets().into_iter().next().unwrap();
        let engines = b.engines.unwrap();
        assert_eq!(engines[0].0, Engine::Encode);
        assert!(
            engines[0].1 > b.left,
            "the encoder has more left than the tightest engine"
        );
    }

    #[test]
    fn a_saturated_device_refuses_a_join_and_holds_nothing_for_it() {
        // A device with room for a couple of joins; the host beside it
        // has a whole thread.
        let (gpu, cap) = gpu_capacity(120);
        let h264 = cap.join_cost(&settings(false), &[VideoCodec::H264], None, false);
        let vp8 = cap.join_cost(&settings(false), &[VideoCodec::VP8], None, false);
        let mut admitted = 0;
        while cap.try_reserve(&h264) {
            admitted += 1;
        }
        assert!(admitted >= 1, "{admitted}");
        assert_eq!(cap.refused(), 1);
        // The device is full: a VP8 join, whose encode would go to the
        // host, still needs the device for its decode and composite —
        // refused, and the host keeps every unit it had.
        let host_left = cap.devices()[1].left();
        assert!(!cap.try_reserve(&vp8));
        assert_eq!(
            cap.devices()[1].left(),
            host_left,
            "a refused join holds nothing"
        );
        assert_eq!(cap.devices()[1].reserved(), 0);
        assert_eq!(cap.refused(), 2);
        assert!(cap.devices()[0].left() < h264.on(&gpu));
        // A recompute over no rooms frees everything on both devices.
        cap.recompute(Vec::<Arc<ConferenceRoom>>::new());
        assert_eq!(cap.devices()[0].left(), 120);
        assert!(cap.try_reserve(&vp8));
    }

    #[test]
    fn the_saturation_constant_fills_the_device_with_that_many_streams() {
        // Sixty real-time 720p30 streams: each costs 1000/60 units.
        let ns = ns_per_px_for_streams(60.0);
        assert_eq!(units(ns, DEVICE_BENCH_RESOLUTION, DEVICE_BENCH_FPS), 17);
        assert!((ns - 0.6029).abs() < 0.001, "{ns}");
        // Nothing sustained is priced as a tenth of a stream, never free.
        assert!(ns_per_px_for_streams(0.0) > ns);
    }

    #[test]
    fn the_self_benchmark_measures_what_it_can_and_falls_back_for_the_rest() {
        let backend = VideoBackend::raw();
        let table = self_benchmark(&backend, &[VideoCodec::VP8], 1, CostTable::phase0());
        match &table.source {
            CostSource::Benchmark { measured, .. } => assert_eq!(measured, &vec![VideoCodec::VP8]),
            other => panic!("{other:?}"),
        }
        // The raw codec is far cheaper than libvpx; H.264 kept the table's.
        assert!(table.encode_ns_per_px(VideoCodec::VP8) < 11.7);
        assert_eq!(table.encode_ns_per_px(VideoCodec::H264), 4.4);
        assert!(table.compose > 0.0);
        // Zero seconds: the table, untouched.
        let table = self_benchmark(&backend, &[VideoCodec::VP8], 0, CostTable::phase0());
        assert_eq!(table.source, CostSource::Phase0);
    }

    /// The node on a GPU (design §15.7, block 8c), on the box: the
    /// device named by `FCP_TEST_VIDEO_DEVICE` opens as the rooms'
    /// backend, its codecs are what the node offers, the saturation
    /// benchmark prices it, and the budgets advertise the device beside
    /// the host. Skips without the variable or when the device does not
    /// open. Run through `deploy/gcp-gpu/gpu-box test`.
    #[cfg(feature = "video-hwaccel")]
    #[test]
    fn the_node_opens_the_device_and_prices_it_by_saturation() {
        use crate::video_bridge::node_codecs;
        use forge_conference::video::CodecPool;
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
        assert_eq!(backend.device, device);

        // What the node offers: the device's codecs, and — with the raw
        // codec allowed, as in CI — the host filling in the rest.
        let codecs = node_codecs(&backend, false);
        let dev = codecs.on(&device).expect("the device is listed").clone();
        let host = codecs
            .on(&MediaDevice::Host)
            .expect("the host is listed")
            .clone();
        eprintln!(
            "{device}: encodes {:?}, decodes {:?}; host encodes {:?}",
            dev.encodes, dev.decodes, host.encodes
        );
        assert!(dev.encodes.contains(&VideoCodec::H264), "NVENC has H.264");
        assert!(dev.decodes.contains(&VideoCodec::H264), "NVDEC has H.264");
        assert!(!dev.encodes.contains(&VideoCodec::VP8), "NVENC has no VP8");
        assert!(
            codecs.usable.contains(&VideoCodec::VP8),
            "VP8 is encoded on the host"
        );
        assert!(node_codecs(&backend, true)
            .usable
            .contains(&VideoCodec::H264));

        // The saturation benchmark, briefly.
        let table =
            self_benchmark_device(&backend, &dev.encodes, &dev.decodes, 6, CostTable::phase0());
        let h264 = table.encode_ns_per_px(VideoCodec::H264);
        let streams = 1000.0
            / (h264 * DEVICE_BENCH_RESOLUTION.pixels() as f64 * DEVICE_BENCH_FPS as f64 / 1e6);
        eprintln!(
            "H.264 encode {h264:.3} ns/px = {streams:.1} real-time 720p30 streams; compose {:.2} ns/px",
            table.compose
        );
        assert!(h264 > 0.0 && h264 < 4.4, "measured, not the table's 4.4");
        assert!(streams > 10.0, "{streams}");
        assert!(table.compose > 0.0);

        // The budgets: the device first with what it encodes, the host beside it.
        let capacity = VideoCapacity::new(
            device.clone(),
            vec![
                DeviceCapacity::new(
                    device.clone(),
                    dev.encodes.clone(),
                    dev.decodes.clone(),
                    table,
                    DeviceCapacity::device_total(0.8),
                ),
                DeviceCapacity::new(
                    MediaDevice::Host,
                    host.encodes.clone(),
                    host.decodes.clone(),
                    CostTable::phase0(),
                    DeviceCapacity::host_total(2, 0.8),
                ),
            ],
        );
        let budgets = capacity.budgets();
        assert_eq!(budgets[0].device, device.to_string());
        assert!(budgets[0].codecs.contains(&"H264".to_string()));
        assert!(!budgets[0].codecs.contains(&"VP8".to_string()));
        assert_eq!(budgets[1].device, "host");
        // A VP8 join spends the host on its encode and the device on the rest.
        let settings = VideoRoomSettings::default();
        let vp8 = capacity.join_cost(&settings, &[VideoCodec::VP8], None, false);
        assert!(vp8.on(&MediaDevice::Host) > 0);
        assert!(vp8.on(&device) > 0);
        let h264 = capacity.join_cost(&settings, &[VideoCodec::H264], None, false);
        assert_eq!(h264.on(&MediaDevice::Host), 0);
        eprintln!("join costs: VP8 {vp8}; H.264 {h264}");
        assert!(capacity.try_reserve(&h264));
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&capacity.json()).unwrap()
        );
    }
}
