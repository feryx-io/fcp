//! What a webinar across nodes costs (`docs/WEBINARS.md` §2.7, W5.4): a
//! stage node and its edges, each edge trunked to the stage alone and
//! sending its own audience the stage's feed. The figures worth knowing are
//! each node's CPU, what the stage pays for its trunks, and whether every
//! attendee on every edge is sent what the host says.
//!
//! **Not** part of any suite: `#[ignore]`d, and only meaningful in release
//! on a box doing nothing else:
//!
//! ```bash
//! cargo build --release -p fcp-conference-server
//! FCP_TEST_DATABASE_URL=… FCP_TEST_REDIS_URL=… \
//!     cargo test --release -p fcp-loadgen --test webinar_tree \
//!     -- --ignored --nocapture --test-threads 1
//! ```
//!
//! `FCP_LOADGEN_NODES` (3: the stage and two edges), `FCP_LOADGEN_ATTENDEES`
//! (2000, split evenly over the edges) and `FCP_LOADGEN_SECONDS` (20) shape
//! the run. `FCP_LOADGEN_VIDEO=composite` puts the host on camera and every
//! attendee on the stage's composite, which each edge forwards (W5.3); build
//! the node with `--features video-vpx` for a real encode on the stage. The
//! legs are plain RTP; the host speaks a tone throughout and every attendee
//! sends silence. Attendees dial their edge straight, by a guest token:
//! placement is not what is measured. The phones run on the same box as the
//! nodes, so the figures are an upper bound. The report goes to
//! `target/loadgen/webinar-tree-<nodes>-<attendees>[-video].json`.

use fcp_loadgen::sampler::Sampler;
use fcp_testkit::cluster::{ClusterOptions, ConferenceCluster};
use fcp_testkit::smoke::{http_get, http_request, loopback};
use fcp_testkit::{Phone, Seed, TestDatabase};
use std::time::{Duration, Instant};

const REALM: &str = "bench.local";
const ROOM: &str = "8300";

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn json(http: u16, path: &str) -> serde_json::Value {
    let (_, body) = http_get(&loopback(http, path)).await.expect("node answers");
    serde_json::from_str(&body).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn what_a_tree_costs() {
    let nodes_n: usize = env_or("FCP_LOADGEN_NODES", 3).max(2);
    let attendees: usize = env_or("FCP_LOADGEN_ATTENDEES", 2000);
    let seconds: u64 = env_or("FCP_LOADGEN_SECONDS", 20);
    let video: String = env_or("FCP_LOADGEN_VIDEO", "none".to_string());
    let watching = video != "none";
    let edges = nodes_n - 1;
    let per_edge = attendees.div_ceil(edges);

    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let mut fields = serde_json::json!({
        "kind": "webinar",
        "max_participants": attendees + 10,
        "play_entry_tone": false,
        "play_exit_tone": false,
    });
    if watching {
        fields["video_enabled"] = true.into();
        fields["video_codecs"] = serde_json::json!(["VP8"]);
        fields["video_mode"] = "mix".into();
        fields["video_max_resolution"] = "1280x720".into();
        fields["video_fps"] = 15.into();
    }
    seed.meeting_with(ROOM, fields).await;

    // An edge's ports: a pair an attendee and some to spare (two pairs a
    // watching one), and a node takes at most 10,000. The stage needs only
    // its trunks'. The phones are on an address of their own.
    let ports = ((if watching { 5 } else { 3 }) * per_edge + 200).min(9_998) as u16;
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(mut cluster) = ConferenceCluster::start(
        db.url(),
        dir.path(),
        ClusterOptions {
            nodes: nodes_n,
            join_secret: Some("bench-join".into()),
            conference_extra: format!(
                "max_invite_rate_per_ip = 0\n\
                 max_participants_per_node = {cap}\n\
                 video_egress_mbps = 100000\n",
                cap = per_edge + 20
            ),
            // Thousands of phones on one address are not one flooding source.
            sip_extra: "max_messages_per_source = 0\n".into(),
            rtp_ports: ports,
            first_node_rtp_ports: Some(400),
            log_level: env_or("FCP_LOADGEN_LOG", "warn".to_string()),
        },
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        db.drop_db().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let token = |name: &str, role: fcp_core::JoinRole| {
        fcp_core::JoinToken::new(seed.tenant_id, ROOM, name, role, Duration::from_secs(3600))
            .mint(b"bench-join")
    };
    let on_camera = |phone: &Phone| {
        phone.enable_video(fcp_testkit::video::VideoMode::Plain);
        phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    };

    // The host claims node 0 as the stage and takes the webinar live.
    let stage = &cluster.nodes[0];
    let host = Phone::new("host", "x", REALM, stage.sip)
        .await
        .expect("phone");
    if watching {
        on_camera(&host);
    }
    let host_call = host
        .call_with_headers(
            ROOM,
            &[("X-FCP-Join", &token("Host", fcp_core::JoinRole::Host))],
        )
        .await
        .expect("INVITE sent");
    host_call
        .answered(Duration::from_secs(10))
        .await
        .expect("the host is admitted");
    let live = format!("/conferences/{ROOM}/webinar/live");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (code, _) = http_request("POST", &loopback(stage.http, &live), &[], Some("{}"))
            .await
            .expect("go live");
        if code == 200 {
            break;
        }
        assert!(Instant::now() < deadline, "the webinar never went live");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The audience, evenly over the edges, joining in batches.
    let mut phones = Vec::with_capacity(attendees);
    let mut calls: Vec<(usize, fcp_testkit::Call)> = Vec::with_capacity(attendees);
    let joined_at = Instant::now();
    for batch in (0..attendees).collect::<Vec<_>>().chunks(50) {
        let mut joining = Vec::new();
        for &i in batch {
            let edge = 1 + i % edges;
            // On an address of their own, so thousands of phones' sockets
            // do not take the nodes' ports.
            let phone = Phone::bound_to(
                "127.0.0.2:0",
                &format!("a{i}"),
                "x",
                REALM,
                cluster.nodes[edge].sip,
            )
            .await
            .expect("phone");
            if watching {
                on_camera(&phone);
            }
            let t = token(&format!("Attendee {i}"), fcp_core::JoinRole::Guest);
            let call = phone
                .call_with_headers(ROOM, &[("X-FCP-Join", &t)])
                .await
                .expect("INVITE sent");
            joining.push((edge, phone, call));
        }
        for (edge, phone, call) in joining {
            if let Err(e) = call.answered(Duration::from_secs(30)).await {
                let logs = cluster.kill_all().await;
                panic!("attendee {} not admitted: {e}\n{logs}", phone.username);
            }
            phones.push(phone);
            calls.push((edge, call));
        }
    }
    let join_secs = joined_at.elapsed().as_secs_f64();

    // Every edge is the stage's, trunked to it, its lobby let in; the stage
    // hears every edge's audience.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let w = json(stage.http, &format!("/conferences/{ROOM}/webinar")).await;
        let heard: u64 = w["tree"]["edges"]
            .as_array()
            .map(|e| e.iter().filter_map(|e| e["audience"].as_u64()).sum())
            .unwrap_or(0);
        if heard == attendees as u64 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the stage hears {heard} of {attendees}: {w:#}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    host_call.media().play_tone(440.0, (seconds + 10) * 1000);
    let pump = |d: Duration| {
        let host_call = &host_call;
        async move {
            let until = Instant::now() + d;
            while Instant::now() < until {
                if watching {
                    let _ = host_call.video().expect("video").send_frames(1, 10).await;
                }
                tokio::time::sleep(Duration::from_millis(66)).await;
            }
        }
    };
    if watching {
        host_call.video().expect("video").set_packet_size(1200);
    }
    pump(Duration::from_secs(3)).await;
    let audio = |calls: &[(usize, fcp_testkit::Call)], edge: usize| -> u64 {
        calls
            .iter()
            .filter(|(e, _)| *e == edge)
            .map(|(_, c)| c.media().packets_received())
            .sum()
    };
    let video_bytes = |calls: &[(usize, fcp_testkit::Call)], edge: usize| -> (u64, u64) {
        calls
            .iter()
            .filter(|(e, _)| *e == edge)
            .filter_map(|(_, c)| c.video())
            .map(|v| v.received_any())
            .fold((0, 0), |(p, b), (p2, b2)| (p + p2, b + b2))
    };
    let before: Vec<(u64, (u64, u64))> = (0..nodes_n)
        .map(|n| (audio(&calls, n), video_bytes(&calls, n)))
        .collect();
    let samplers: Vec<Sampler> = cluster
        .nodes
        .iter()
        .map(|n| {
            Sampler::start(
                None,
                n.service.as_ref().and_then(|s| s.pid()),
                Duration::from_secs(1),
            )
        })
        .collect();
    let began = Instant::now();
    pump(Duration::from_secs(seconds)).await;
    let elapsed = began.elapsed().as_secs_f64();
    let mut per_node = Vec::new();
    let mut worst_delivery: f64 = 1.0;
    for (n, sampler) in samplers.into_iter().enumerate() {
        let samples = sampler.finish().await;
        let cpu: Vec<f64> = samples.iter().filter_map(|s| s.cpu_percent).collect();
        let cpu_percent = cpu.iter().sum::<f64>() / cpu.len().max(1) as f64;
        let rss_mb = samples
            .iter()
            .filter_map(|s| s.rss_bytes)
            .max()
            .unwrap_or(0) as f64
            / 1e6;
        let here = calls.iter().filter(|(e, _)| *e == n).count();
        let delivered = (audio(&calls, n) - before[n].0) as f64 / elapsed;
        let delivery = if here > 0 {
            delivered / (here as f64 * 50.0)
        } else {
            1.0
        };
        if here > 0 {
            worst_delivery = worst_delivery.min(delivery);
        }
        let v = video_bytes(&calls, n);
        let video_mbps = (v.1 - before[n].1 .1) as f64 * 8.0 / elapsed / 1e6;
        let room = json(cluster.nodes[n].http, &format!("/conferences/{ROOM}/video")).await;
        per_node.push(serde_json::json!({
            "node": cluster.nodes[n].id,
            "role": if n == 0 { "stage" } else { "edge" },
            "attendees": here,
            "cpu_percent_of_one_core": cpu_percent,
            "peak_rss_mb": rss_mb,
            "delivered_packets_per_sec": delivered,
            "delivery_ratio": delivery,
            "video_mbps_to_attendees": video_mbps,
            "passthrough": room["video"]["passthrough"],
            "encoders": room["video"]["encoders"],
        }));
    }
    let report = serde_json::json!({
        "nodes": nodes_n,
        "attendees": attendees,
        "video": video,
        "seconds": elapsed,
        "join_seconds": join_secs,
        "per_node": per_node,
        "release": !cfg!(debug_assertions),
    });
    let out = std::path::Path::new("target/loadgen");
    std::fs::create_dir_all(out).ok();
    let name = if watching {
        format!("webinar-tree-{nodes_n}-{attendees}-{video}.json")
    } else {
        format!("webinar-tree-{nodes_n}-{attendees}.json")
    };
    std::fs::write(
        out.join(&name),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .ok();
    println!(
        "\n=== what_a_tree_costs: {nodes_n} nodes, {attendees} attendees, {video} ===\n\
         joined in {join_secs:.1} s, measured {elapsed:.1} s"
    );
    for n in &per_node {
        println!(
            "{:>5} {:<20} {:>5} attendees  cpu {:>6.1}%  rss {:>5.0} MB  delivery {:>5.1}%  video {:>7.1} Mb/s  passthrough {}  encoders {}",
            n["role"].as_str().unwrap_or(""),
            n["node"].as_str().unwrap_or(""),
            n["attendees"],
            n["cpu_percent_of_one_core"].as_f64().unwrap_or(0.0),
            n["peak_rss_mb"].as_f64().unwrap_or(0.0),
            n["delivery_ratio"].as_f64().unwrap_or(0.0) * 100.0,
            n["video_mbps_to_attendees"].as_f64().unwrap_or(0.0),
            n["passthrough"],
            n["encoders"],
        );
    }
    println!("report {}", out.join(&name).display());
    assert!(
        worst_delivery > 0.9,
        "every edge's audience is sent what the host says"
    );

    let _ = host_call.hangup().await;
    for (_, call) in &calls {
        let _ = call.hangup().await;
    }
    drop(phones);
    cluster.stop_all().await;
    db.drop_db().await;
}
