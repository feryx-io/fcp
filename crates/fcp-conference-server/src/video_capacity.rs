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

use forge_conference::video::{VideoBackend, VideoRoom, VideoRoomSettings};
use forge_conference::ConferenceRoom;
use forge_core::VideoCodec;
use forge_video::bench::{self, BenchSettings};
use forge_video::frame::{MediaDevice, Resolution};
use tracing::{info, warn};

/// Nanoseconds of device time per second of video in one work unit.
pub const NS_PER_UNIT: f64 = 1e6;

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
    pub codecs: Vec<String>,
    pub left: u32,
    pub total: u32,
}

/// A node's video budget on one device: what it can run, what it costs,
/// what is committed, and what a join may still take.
pub struct VideoCapacity {
    device: MediaDevice,
    codecs: Vec<VideoCodec>,
    table: CostTable,
    total: u32,
    /// Priced from the rooms at the last recompute.
    committed: AtomicU32,
    /// Admitted since the last recompute, not yet visible in any room.
    reserved: AtomicU32,
    /// Joins refused for want of budget, since start.
    refused: AtomicU64,
    last_recompute: parking_lot::Mutex<Option<Instant>>,
}

impl VideoCapacity {
    /// `execution_units` codec-pool threads (or media engines) at
    /// `headroom` of their time (0.8 keeps a fifth for the async side and
    /// for content harder than the benchmark's).
    pub fn new(
        device: MediaDevice,
        codecs: Vec<VideoCodec>,
        table: CostTable,
        execution_units: usize,
        headroom: f64,
    ) -> Self {
        let headroom = if headroom.is_finite() {
            headroom.clamp(0.05, 1.0)
        } else {
            0.8
        };
        let total = (execution_units.max(1) as f64 * UNITS_PER_EXECUTION_UNIT * headroom).round();
        Self {
            device,
            codecs,
            table,
            total: total as u32,
            committed: AtomicU32::new(0),
            reserved: AtomicU32::new(0),
            refused: AtomicU64::new(0),
            last_recompute: parking_lot::Mutex::new(None),
        }
    }

    pub fn device(&self) -> &MediaDevice {
        &self.device
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn committed(&self) -> u32 {
        self.committed.load(Ordering::Relaxed)
    }

    pub fn reserved(&self) -> u32 {
        self.reserved.load(Ordering::Relaxed)
    }

    /// Units a new room or participant may still cost here.
    pub fn left(&self) -> u32 {
        self.total
            .saturating_sub(self.committed())
            .saturating_sub(self.reserved())
    }

    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
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

    /// What a room's video costs now: a decode per source at what it sends
    /// (the room's canvas until frames arrive), an encode per flavor, a
    /// composite per output at the room's target rate.
    pub fn room_cost(&self, video: &VideoRoom) -> u32 {
        let st = video.status();
        let fps = st.target_fps.max(1);
        let decode: u32 = video
            .participants()
            .iter()
            .filter_map(|p| p.source.as_ref())
            .map(|s| {
                let resolution = if s.resolution.pixels() == 0 {
                    st.resolution
                } else {
                    s.resolution
                };
                self.decode_units(s.codec, resolution, if s.fps > 0 { s.fps } else { fps })
            })
            .sum();
        // A flavor every one of whose subscribers is on the passthrough
        // fast path is not being encoded, and an output with no flavor
        // left to encode is not being composed (§5.6). Charging for them
        // would keep reserving a budget nothing spends.
        let idle = idle_flavors(video, &st);
        let encode: u32 = st
            .outputs
            .iter()
            .flat_map(|o| o.flavors.iter())
            .filter(|f| !idle.contains(&f.flavor))
            .map(|f| self.encode_units(f.flavor.codec, f.flavor.resolution, f.flavor.fps.max(1)))
            .sum();
        let compose: u32 = st
            .outputs
            .iter()
            .filter(|o| o.flavors.iter().any(|f| !idle.contains(&f.flavor)))
            .map(|o| self.compose_units(st.sources, o.resolution, fps))
            .sum();
        decode.saturating_add(encode).saturating_add(compose)
    }

    /// What one more video participant would add: its decode, an encode
    /// (it may need a flavor of its own), and a composite when it starts
    /// the room's video or wants a canvas without itself. Priced at the
    /// dearest of `codecs`, since which one the caller ends up with is
    /// decided in the answer.
    pub fn join_cost(
        &self,
        settings: &VideoRoomSettings,
        codecs: &[VideoCodec],
        room_video: Option<&VideoRoom>,
    ) -> u32 {
        let fps = settings.fps.max(1);
        let codec_cost = codecs
            .iter()
            .map(|&c| {
                self.decode_units(c, settings.resolution, fps)
                    .saturating_add(self.encode_units(c, settings.resolution, fps))
            })
            .max()
            .unwrap_or(0);
        let compose = match room_video {
            None => self.compose_units(1, settings.resolution, fps),
            Some(video) if settings.exclude_self => {
                self.compose_units(video.status().sources + 1, settings.resolution, fps)
            }
            Some(_) => 0,
        };
        codec_cost.saturating_add(compose)
    }

    /// Admit a join costing `cost` if it fits, holding the units until the
    /// next recompute sees the participant in a room. `false` refuses.
    pub fn try_reserve(&self, cost: u32) -> bool {
        let committed = self.committed();
        let mut reserved = self.reserved();
        loop {
            if committed.saturating_add(reserved).saturating_add(cost) > self.total {
                self.refused.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            match self.reserved.compare_exchange_weak(
                reserved,
                reserved + cost,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(now) => reserved = now,
            }
        }
    }

    /// Re-price every room and drop the reservations; returns what is
    /// committed.
    pub fn recompute(&self, rooms: impl IntoIterator<Item = Arc<ConferenceRoom>>) -> u32 {
        let committed: u32 = rooms
            .into_iter()
            .filter_map(|room| room.video())
            .map(|video| self.room_cost(&video))
            .fold(0u32, |a, b| a.saturating_add(b));
        self.committed.store(committed, Ordering::Relaxed);
        self.reserved.store(0, Ordering::Relaxed);
        *self.last_recompute.lock() = Some(Instant::now());
        committed
    }

    /// The budget line for cluster membership.
    pub fn budget(&self) -> DeviceBudget {
        DeviceBudget {
            device: self.device.to_string(),
            codecs: self
                .codecs
                .iter()
                .map(|c| c.sdp_name().to_string())
                .collect(),
            left: self.left(),
            total: self.total,
        }
    }

    /// For `/status`.
    pub fn json(&self) -> serde_json::Value {
        let mut codecs: Vec<&VideoCodec> = self.codecs.iter().collect();
        codecs.sort_by_key(|c| c.sdp_name());
        let table: serde_json::Map<String, serde_json::Value> = codecs
            .iter()
            .map(|&&c| {
                (
                    c.sdp_name().to_string(),
                    serde_json::json!({
                        "encode_ns_per_px": round2(self.table.encode_ns_per_px(c)),
                        "decode_ns_per_px": round2(self.table.decode_ns_per_px(c)),
                        // What one 720p15 stream of it costs, for a feel of the unit.
                        "encode_units_720p15": self.encode_units(c, Resolution::new(1280, 720), 15),
                        "decode_units_720p15": self.decode_units(c, Resolution::new(1280, 720), 15),
                    }),
                )
            })
            .collect();
        serde_json::json!({
            "device": self.device.to_string(),
            "unit": "one thousandth of one codec-pool thread's time per second",
            "total": self.total,
            "committed": self.committed(),
            "reserved": self.reserved(),
            "left": self.left(),
            "refused": self.refused(),
            "codecs": codecs.iter().map(|c| c.sdp_name()).collect::<Vec<_>>(),
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
            "last_recompute_secs_ago": self.last_recompute.lock().map(|t| t.elapsed().as_secs()),
        })
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Price this node's codecs by running each for a share of `seconds` on
/// the backend's device (design §9.3: "re-measured per node and per device
/// at start"). `seconds == 0` skips the run and uses `fallback` as it is.
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
        match bench::measure_codec(&backend.registry, &backend.device, codec, &settings) {
            Ok(cost) => {
                info!(
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
    if backend.device.is_host() {
        table.compose = bench::measure_compose(Resolution::new(1280, 720), 9, 5);
        info!(
            compose_ns_per_px = format!("{:.2}", table.compose),
            "Video self-benchmark: composition"
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use forge_conference::video::SubscribeRequest;
    use forge_conference::AudioFormat;

    fn capacity(units: usize) -> VideoCapacity {
        VideoCapacity::new(
            MediaDevice::Host,
            vec![VideoCodec::H264, VideoCodec::VP8],
            CostTable::phase0(),
            units,
            0.8,
        )
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
        let first = cap.join_cost(&settings(false), &[VideoCodec::H264], None);
        let h264 =
            units(4.4, Resolution::new(1280, 720), 15) + units(1.4, Resolution::new(1280, 720), 15);
        let compose1 = units(2.0 * 1.1, Resolution::new(1280, 720), 15);
        assert_eq!(first, h264 + compose1);
        // The dearest allowed codec sets the price.
        let either = cap.join_cost(&settings(false), &[VideoCodec::H264, VideoCodec::VP8], None);
        assert!(either > first);

        assert!(cap.try_reserve(first));
        assert_eq!(cap.left(), 800 - first);
        // Ten more do not fit in one thread.
        let mut admitted = 1;
        while cap.try_reserve(first) {
            admitted += 1;
        }
        assert!((4..10).contains(&admitted), "{admitted}");
        assert_eq!(cap.refused(), 1);
        // A recompute over no rooms frees everything.
        assert_eq!(cap.recompute(Vec::<Arc<ConferenceRoom>>::new()), 0);
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
        let cap = VideoCapacity::new(
            MediaDevice::Host,
            vec![VideoCodec::VP8],
            CostTable::phase0(),
            4,
            1.0,
        );
        assert_eq!(cap.room_cost(&video), 0, "no sources, no outputs yet");

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
                },
            )
            .unwrap();
        let cost = cap.room_cost(&video);
        let res = Resolution::new(640, 360);
        let expected = units(1.8, res, 15)            // alice's decode at the canvas size
            + units(11.7, res, 15)                   // bob's VP8 flavor
            + units(2.0 * 1.1, res, 15); // one output, one source tile
        assert_eq!(cost, expected);

        // Joining an existing room with a canvas to share adds no composite;
        // asking for one without yourself does.
        let more = cap.join_cost(&settings(false), &[VideoCodec::VP8], Some(&video));
        assert_eq!(
            more,
            units(1.8, Resolution::new(1280, 720), 15)
                + units(11.7, Resolution::new(1280, 720), 15)
        );
        let own = cap.join_cost(&settings(true), &[VideoCodec::VP8], Some(&video));
        assert!(own > more);

        // The recompute over this room commits exactly its cost.
        cap.try_reserve(5);
        assert_eq!(cap.recompute(vec![Arc::clone(&room)]), expected);
        assert_eq!(cap.reserved(), 0);
        assert_eq!(cap.left(), cap.total() - expected);
        let budget = cap.budget();
        assert_eq!(budget.device, "host");
        assert_eq!(budget.codecs, vec!["VP8"]);
        assert_eq!(budget.total, 4000);
        assert_eq!(cap.json()["left"], cap.left());
        drop(sub);
        room.disable_video();
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
}
