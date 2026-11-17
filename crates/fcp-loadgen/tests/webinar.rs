//! What a webinar's audience costs its node (`docs/WEBINARS.md` §2.2, W1).
//! An attendee hears the room's audience feed — mixed once a frame and
//! encoded once per codec for all of them — so the numbers worth knowing
//! are CPU per attendee and the feed's frame time, against a meeting of
//! the same size where everyone has an N−1 mix and an encode of their own.
//!
//! **Not** part of any suite: `#[ignore]`d, and only meaningful in
//! release on a box doing nothing else, as `forwarding.rs`'s run is:
//!
//! ```bash
//! cargo build --release -p fcp-conference-server
//! FCP_TEST_DATABASE_URL=… cargo test --release -p fcp-loadgen --test webinar \
//!     -- --ignored --nocapture --test-threads 1
//! ```
//!
//! `FCP_LOADGEN_ATTENDEES` (1000), `FCP_LOADGEN_KIND` (`webinar`, or
//! `meeting` for the N−1 comparison) and `FCP_LOADGEN_SECONDS` (20) shape
//! the run; the legs are plain RTP. One host speaks a tone throughout;
//! every attendee sends silence, as a phone does. The report goes to
//! `target/loadgen/webinar-<kind>[-<video>].json`. The phones run on the
//! same box as the node, which makes the figures an upper bound on the
//! node's cost.
//!
//! **Watching** (W2, §2.3): `FCP_LOADGEN_VIDEO=composite` or `forward` puts
//! a stage of the host and `FCP_LOADGEN_PANELISTS` (2) on camera at 15
//! frames a second, and every attendee on video — the room's composite at
//! `FCP_LOADGEN_RESOLUTION` (`1280x720`), or the stage's own streams
//! (`video_mode = auto`, `Recv-Info: fcp-view`). The run then says what
//! the audience is sent: packets and megabits a second, per attendee, and
//! how many a node's link would hold. The composite needs a real encoder:
//! build the node with `--features video-vpx`. The stage's frames are the
//! test kit's, which no decoder takes, so its tiles are drawn empty — the
//! encode and the sending are the real thing.

use fcp_loadgen::sampler::Sampler;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{bins, free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const REALM: &str = "bench.local";
const ROOM: &str = "8200";

/// The stage's cameras sending for `d`: 15 frames a second each, a
/// keyframe a second.
async fn on_camera_for(cameras: &[&fcp_testkit::Call], d: Duration) {
    let until = Instant::now() + d;
    let mut tick = tokio::time::interval(Duration::from_millis(66));
    while Instant::now() < until {
        tick.tick().await;
        for c in cameras {
            let _ = c.video().unwrap().send_frames(1, 15).await;
        }
    }
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn what_an_attendee_costs() {
    let attendees: usize = env_or("FCP_LOADGEN_ATTENDEES", 1000);
    let seconds: u64 = env_or("FCP_LOADGEN_SECONDS", 20);
    let kind: String = env_or("FCP_LOADGEN_KIND", "webinar".to_string());
    let webinar = kind == "webinar";
    let video: String = env_or("FCP_LOADGEN_VIDEO", "none".to_string());
    let watching = video != "none";
    let forward = video == "forward";
    let panelists: usize = if watching {
        env_or("FCP_LOADGEN_PANELISTS", 2)
    } else {
        0
    };
    let resolution: String = env_or("FCP_LOADGEN_RESOLUTION", "1280x720".to_string());
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    // The host is let in by the host PIN; everyone else by a guest join
    // token — an attendee in a webinar, a guest in a meeting — so a join
    // storm does not hang on DTMF.
    let mut fields = serde_json::json!({
        "kind": kind,
        "max_participants": attendees + 10,
        "play_entry_tone": false,
        "play_exit_tone": false,
    });
    if webinar {
        fields["host_pin"] = "9999".into();
    }
    if watching {
        fields["video_enabled"] = true.into();
        fields["video_codecs"] = serde_json::json!(["VP8"]);
        fields["video_mode"] = if forward { "auto" } else { "mix" }.into();
        fields["video_max_resolution"] = resolution.clone().into();
        fields["video_fps"] = 15.into();
    }
    seed.meeting_with(ROOM, fields).await;
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
# A thousand phones on one address are not one flooding source.
max_messages_per_source = 0

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
# Every phone here is on one loopback address.
max_invite_rate_per_ip = 0
max_participants_per_node = {cap}
join_secret = "bench-join"
# What the audience is sent is measured here, not refused.
video_egress_mbps = 100000

[logging]
level = "warn"
format = "compact"
"#,
        db = db.url(),
        // Two pairs a watching attendee, with room for those the phones on
        // this box hold; the node takes at most 10,000 ports.
        rtp_end = (u32::from(rtp_start) + (8 * (attendees as u32 + 10)).min(9_998)).min(65_534),
        cap = attendees + 10,
        dir = dir.path().display(),
    );
    let bin = bins::path("fcp-conference", "fcp-conference-server");
    let service = Service::start("conference", bin, &config, &loopback(http, "/health")).await;
    let sip = SocketAddr::from(([127, 0, 0, 1], sip_port));

    async fn dial(call: &fcp_testkit::Call, digits: &str) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for d in digits.chars().chain(std::iter::once('#')) {
            call.media().send_dtmf(d);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    let on_camera = |phone: &Phone| {
        phone.enable_video(fcp_testkit::video::VideoMode::Plain);
        phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    };
    let host = Phone::new("host", "x", REALM, sip).await.expect("phone");
    if watching {
        on_camera(&host);
    }
    let host_call = host.call(ROOM).await.expect("INVITE sent");
    host_call
        .answered(Duration::from_secs(10))
        .await
        .expect("the host is admitted");
    if webinar {
        dial(&host_call, "9999").await;
    }
    // The rest of the stage, on camera, by panelist token (a guest's in a
    // meeting).
    let mut stage = Vec::new();
    for i in 0..panelists {
        let phone = Phone::new(&format!("p{i}"), "x", REALM, sip)
            .await
            .expect("phone");
        on_camera(&phone);
        let token = fcp_core::JoinToken::new(
            seed.tenant_id,
            ROOM,
            format!("Panelist {i}"),
            if webinar {
                fcp_core::JoinRole::Panelist
            } else {
                fcp_core::JoinRole::Guest
            },
            Duration::from_secs(3600),
        )
        .mint(b"bench-join");
        let call = phone
            .call_with_headers(ROOM, &[("X-FCP-Join", &token)])
            .await
            .expect("INVITE sent");
        call.answered(Duration::from_secs(10))
            .await
            .expect("a panelist is admitted");
        stage.push((phone, call));
    }

    // The audience joins in batches, each pressing # at once.
    let mut phones = Vec::with_capacity(attendees);
    let mut calls = Vec::with_capacity(attendees);
    let joined_at = Instant::now();
    for batch in (0..attendees).collect::<Vec<_>>().chunks(50) {
        let mut joining = Vec::new();
        for &i in batch {
            let phone = Phone::new(&format!("a{i}"), "x", REALM, sip)
                .await
                .expect("phone");
            if watching {
                on_camera(&phone);
            }
            let token = fcp_core::JoinToken::new(
                seed.tenant_id,
                ROOM,
                format!("Attendee {i}"),
                fcp_core::JoinRole::Guest,
                Duration::from_secs(3600),
            )
            .mint(b"bench-join");
            let mut headers = vec![("X-FCP-Join", token.as_str())];
            if forward {
                headers.push(("Recv-Info", "fcp-view"));
            }
            let call = phone
                .call_with_headers(ROOM, &headers)
                .await
                .expect("INVITE sent");
            joining.push((phone, call));
        }
        for (phone, call) in joining {
            call.answered(Duration::from_secs(30))
                .await
                .unwrap_or_else(|e| panic!("attendee {} not admitted: {e}", phone.username));
            phones.push(phone);
            calls.push(call);
        }
    }
    let join_secs = joined_at.elapsed().as_secs_f64();
    let room = |path: String| async move {
        let (_, body) = http_get(&loopback(http, &path)).await.expect("room");
        serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default()
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let r = room(format!("/conferences/{ROOM}")).await;
        if r["participant_count"].as_u64() == Some((attendees + 1 + panelists) as u64) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {} are in; audience {}",
            r["participant_count"],
            attendees + 1 + panelists,
            r["audience"]
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // The host speaks throughout; let everyone's audio start, then measure.
    host_call.media().play_tone(440.0, (seconds + 10) * 1000);
    // The stage on camera, while the run waits: 15 frames a second each,
    // a keyframe a second, packets the size a camera's are.
    let cameras: Vec<&fcp_testkit::Call> = std::iter::once(&host_call)
        .chain(stage.iter().map(|(_, c)| c))
        .filter(|_| watching)
        .collect();
    for c in &cameras {
        c.video()
            .expect("the stage has video")
            .set_packet_size(1200);
    }
    on_camera_for(&cameras, Duration::from_secs(3)).await;
    let received = |calls: &[fcp_testkit::Call]| -> u64 {
        calls.iter().map(|c| c.media().packets_received()).sum()
    };
    let video_received = |calls: &[fcp_testkit::Call]| -> (u64, u64) {
        calls
            .iter()
            .filter_map(|c| c.video())
            .map(|v| v.received_any())
            .fold((0, 0), |(p, b), (p2, b2)| (p + p2, b + b2))
    };
    let before = received(&calls);
    let video_before = video_received(&calls);
    let before_feed = room(format!("/conferences/{ROOM}")).await["audience"].clone();
    let sampler = Sampler::start(None, service.pid(), Duration::from_secs(1));
    let began = Instant::now();
    on_camera_for(&cameras, Duration::from_secs(seconds)).await;
    let elapsed = began.elapsed().as_secs_f64();
    let samples = sampler.finish().await;
    let delivered = received(&calls) - before;
    let video_after = video_received(&calls);
    let after = room(format!("/conferences/{ROOM}")).await;
    let video_packets_per_sec = (video_after.0 - video_before.0) as f64 / elapsed;
    let video_mbps = (video_after.1 - video_before.1) as f64 * 8.0 / elapsed / 1e6;
    let kbps_per_attendee = video_mbps * 1000.0 / attendees as f64;
    // What a node's link of 1 Gb/s would hold of this audience watching.
    let per_gbps = 1e6 / kbps_per_attendee.max(1e-9);
    let feed = after["audience"].clone();

    let cpu: Vec<f64> = samples.iter().filter_map(|s| s.cpu_percent).collect();
    let cpu_percent = cpu.iter().sum::<f64>() / cpu.len().max(1) as f64;
    let rss: Vec<u64> = samples.iter().filter_map(|s| s.rss_bytes).collect();
    let rss_mb = rss.iter().copied().max().unwrap_or(0) as f64 / 1e6;
    let delivered_per_sec = delivered as f64 / elapsed;
    // Each attendee should be sent 50 packets a second.
    let delivery = delivered_per_sec / (attendees as f64 * 50.0);
    let core_ms_per_attendee_sec = cpu_percent / 100.0 * 1000.0 / attendees as f64;
    let attendees_per_core = 1000.0 / core_ms_per_attendee_sec.max(1e-9);
    let report = serde_json::json!({
        "kind": kind,
        "attendees": attendees,
        "seconds": elapsed,
        "join_seconds": join_secs,
        "delivered_packets_per_sec": delivered_per_sec,
        "delivery_ratio": delivery,
        "cpu_percent_of_one_core": cpu_percent,
        "core_ms_per_attendee_second": core_ms_per_attendee_sec,
        "attendees_per_core": attendees_per_core,
        "peak_rss_mb": rss_mb,
        "audience_before": before_feed,
        "audience": feed,
        "video": video,
        "panelists": panelists,
        "resolution": resolution,
        "video_packets_per_sec": video_packets_per_sec,
        "video_mbps": video_mbps,
        "video_kbps_per_attendee": kbps_per_attendee,
        "watching_per_gbps": per_gbps,
        // Attendees whose call carries video, and of those, how many were
        // sent any.
        "attendees_with_video": calls.iter().filter(|c| c.video().is_some()).count(),
        "attendees_sent_video": calls
            .iter()
            .filter(|c| c.video().is_some_and(|v| v.received_any().0 > 0))
            .count(),
        "room_video": after["video"],
        "release": !cfg!(debug_assertions),
        "samples": samples,
    });
    let out = std::path::Path::new("target/loadgen");
    std::fs::create_dir_all(out).ok();
    let path = if watching {
        out.join(format!("webinar-{kind}-{video}.json"))
    } else {
        out.join(format!("webinar-{kind}.json"))
    };
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).ok();
    println!(
        "\n=== what_an_attendee_costs ({kind}) ===\n\
         {attendees} attendees, {elapsed:.1} s (joined in {join_secs:.1} s)\n\
         delivered  {delivered_per_sec:.0} packets/s ({:.1}% of 50 a second each)\n\
         cpu        {cpu_percent:.1}% of one core, peak RSS {rss_mb:.0} MB\n\
         per head   {core_ms_per_attendee_sec:.3} ms of core per second\n\
         one core   ≈ {attendees_per_core:.0} attendees\n\
         feed       {} frames, mean {:.1} µs, max {:.1} µs\n\
         video      {video}: {video_packets_per_sec:.0} packets/s, {video_mbps:.1} Mb/s, \
         {kbps_per_attendee:.0} kb/s each, ≈ {per_gbps:.0} watching per Gb/s\n\
         report     {}",
        delivery * 100.0,
        feed["frames_fed"],
        feed["frame_mean_us"].as_f64().unwrap_or(0.0),
        feed["frame_max_us"].as_f64().unwrap_or(0.0),
        path.display()
    );
    if webinar {
        assert_eq!(feed["listeners"], attendees, "{after}");
        assert_eq!(
            feed["flavors"].as_array().map(|f| f.len()),
            Some(1),
            "one codec, one feed: {after}"
        );
    }
    assert!(delivery > 0.9, "the audience is sent what the host says");
    if watching {
        assert!(
            video_packets_per_sec > 0.0,
            "the audience is sent the stage"
        );
        if webinar {
            assert_eq!(after["video"]["audience"], attendees, "{after}");
        }
    }

    for (_, call) in &stage {
        let _ = call.hangup().await;
    }
    let _ = host_call.hangup().await;
    for call in &calls {
        let _ = call.hangup().await;
    }
    drop(phones);
    // The node's warnings, beside the report.
    let logs = service.stop().await;
    std::fs::write(out.join(format!("webinar-{kind}-{video}.log")), logs).ok();
    db.drop_db().await;
}
