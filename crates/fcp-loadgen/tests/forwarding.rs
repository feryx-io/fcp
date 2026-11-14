//! What a forwarding room costs its node (`docs/FORWARDING_ROOMS.md` §3,
//! R3b): a forwarded stream costs no codec time, only packet work — one
//! packet in, a copy out to each viewer, each copy rewritten — so the
//! number worth knowing is CPU per forwarded packet, and from it how many
//! streams one core carries.
//!
//! **Not** part of any suite: `#[ignore]`d, and only meaningful in
//! release on a box doing nothing else, as `bench.rs`'s runs are:
//!
//! ```bash
//! cargo build --release -p fcp-conference-server
//! FCP_TEST_DATABASE_URL=… cargo test --release -p fcp-loadgen --test forwarding \
//!     -- --ignored --nocapture --test-threads 1
//! ```
//!
//! `FCP_LOADGEN_PARTICIPANTS` (9), `FCP_LOADGEN_PACKET_BYTES` (1100) and
//! `FCP_LOADGEN_SECONDS` (20) shape the run, and `FCP_LOADGEN_SRTP=1`
//! keys every section with SDES, as a phone on the internet would. Every participant sends one
//! packet a frame at 30 fps and takes the default view, so each is sent
//! everyone else's stream and the speaker's. The report goes to
//! `target/loadgen/forwarding.json`. The phones run on the same box as the
//! node, which makes the figure an upper bound on the node's cost.

use fcp_loadgen::sampler::Sampler;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{bins, free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const REALM: &str = "bench.local";
const ROOM: &str = "8100";
const FRAME: Duration = Duration::from_millis(33);

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measured run: a minute, and only meaningful in release on a quiet box"]
async fn what_a_forwarded_stream_costs() {
    let participants: usize = env_or("FCP_LOADGEN_PARTICIPANTS", 9);
    let packet_bytes: usize = env_or("FCP_LOADGEN_PACKET_BYTES", 1100);
    let seconds: u64 = env_or("FCP_LOADGEN_SECONDS", 20);
    let srtp = std::env::var("FCP_LOADGEN_SRTP").is_ok_and(|v| v == "1");
    let mode = if srtp {
        fcp_testkit::video::VideoMode::Srtp
    } else {
        fcp_testkit::video::VideoMode::Plain
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(
        ROOM,
        serde_json::json!({
            "video_enabled": true,
            "video_codecs": ["VP8"],
            "video_mode": "auto",
            "max_participants": participants + 1,
        }),
    )
    .await;
    let sip_port = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().expect("tempdir");
    let config = format!(
        r#"
[service]
node_id = "bench-conference"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip_port}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[conference]
recordings_dir = "{dir}/recordings"
announcements_dir = "{dir}/announcements"
video_native_codecs_only = false
# Every phone here is on one loopback address.
max_invite_rate_per_ip = 0
enable_srtp = {srtp}

[logging]
level = "warn"
format = "compact"
"#,
        db = db.url(),
        rtp_end = rtp_start + (8 * participants as u16).max(120),
        dir = dir.path().display(),
    );
    let bin = bins::path("fcp-conference", "fcp-conference-server");
    let service = Service::start("conference", bin, &config, &loopback(http, "/health")).await;
    let sip = SocketAddr::from(([127, 0, 0, 1], sip_port));

    let mut phones = Vec::new();
    let mut calls = Vec::new();
    for i in 0..participants {
        let phone = Phone::new(&format!("p{i}"), "x", REALM, sip)
            .await
            .expect("phone");
        phone.enable_video(mode);
        phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
        let call = phone
            .call_with_headers(ROOM, &[("Recv-Info", "fcp-content, fcp-view")])
            .await
            .expect("INVITE sent");
        call.answered(Duration::from_secs(10))
            .await
            .unwrap_or_else(|e| panic!("participant {i} not admitted: {e}"));
        call.video()
            .expect("video answered")
            .set_packet_size(packet_bytes);
        phones.push(phone);
        calls.push(call);
    }

    let send_for = |span: Duration| {
        let calls = &calls;
        async move {
            let until = Instant::now() + span;
            let mut frames = 0u64;
            let mut tick = tokio::time::interval(FRAME);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while Instant::now() < until {
                tick.tick().await;
                for call in calls {
                    call.video().unwrap().send_frames(1, 30).await.unwrap();
                }
                frames += 1;
            }
            frames
        }
    };
    let received = |calls: &[fcp_testkit::Call]| -> u64 {
        calls.iter().map(|c| c.video().unwrap().received()).sum()
    };

    // Let every viewer's streams start before measuring.
    send_for(Duration::from_secs(3)).await;
    let before = received(&calls);
    let sampler = Sampler::start(None, service.pid(), Duration::from_secs(1));
    let began = Instant::now();
    let frames = send_for(Duration::from_secs(seconds)).await;
    let elapsed = began.elapsed().as_secs_f64();
    let samples = sampler.finish().await;
    // Let what is in flight land.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let forwarded = received(&calls) - before;

    let cpu: Vec<f64> = samples.iter().filter_map(|s| s.cpu_percent).collect();
    let cpu_percent = cpu.iter().sum::<f64>() / cpu.len().max(1) as f64;
    let ingress = frames * participants as u64;
    let forwarded_per_sec = forwarded as f64 / elapsed;
    let us_per_packet = (cpu_percent / 100.0) * 1e6 / forwarded_per_sec.max(1.0);
    // One stream at 500 kb/s in packets of this size.
    let packets_per_stream = 500_000.0 / (packet_bytes as f64 * 8.0);
    let streams_per_core = 1e6 / (us_per_packet * packets_per_stream);
    let report = serde_json::json!({
        "participants": participants,
        "packet_bytes": packet_bytes,
        "srtp": srtp,
        "seconds": elapsed,
        "ingress_packets": ingress,
        "forwarded_packets": forwarded,
        "forwarded_per_sec": forwarded_per_sec,
        "forwarded_mbps": forwarded_per_sec * packet_bytes as f64 * 8.0 / 1e6,
        "cpu_percent_of_one_core": cpu_percent,
        "us_per_forwarded_packet": us_per_packet,
        "streams_per_core_at_500_kbps": streams_per_core,
        "release": !cfg!(debug_assertions),
        "samples": samples,
    });
    let out = std::path::Path::new("target/loadgen");
    std::fs::create_dir_all(out).ok();
    let path = out.join("forwarding.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).ok();
    println!(
        "\n=== what_a_forwarded_stream_costs ===\n\
         {participants} participants, {packet_bytes}-byte packets, srtp {srtp}, {elapsed:.1} s\n\
         forwarded  {forwarded_per_sec:.0} packets/s ({:.1} Mb/s) from {ingress} in\n\
         cpu        {cpu_percent:.1}% of one core\n\
         per packet {us_per_packet:.2} µs\n\
         one core   ≈ {streams_per_core:.0} streams at 500 kb/s\n\
         report     {}",
        report["forwarded_mbps"].as_f64().unwrap_or(0.0),
        path.display()
    );
    assert!(
        forwarded > ingress,
        "a forwarding room sends each packet to more than one viewer"
    );

    for call in &calls {
        let _ = call.hangup().await;
    }
    drop(phones);
    service.stop().await;
    db.drop_db().await;
}
