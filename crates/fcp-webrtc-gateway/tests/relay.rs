//! The relay in process: a real browser-side peer connection (a second
//! forge-webrtc peer standing in for the browser), the relay's SIP-side
//! sockets, and a far end's sockets standing in for the conference server.
//! Audio, video, a shared screen and feedback cross both ways, with the
//! payload types and SSRCs each side expects.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use fcp_media::SrtpOffer;
use fcp_webrtc_gateway::relay::{
    run_relay, BrowserLeg, RelayEnd, RelayStats, SipAudioLeg, SipLeg, SipVideoLeg,
};
use fcp_webrtc_gateway::session::browser_audio_codecs;
use forge_rtp::rtcp::{PayloadFeedback, PsFeedback, RtpFeedback, TransportFeedback};
use forge_rtp::{
    LatchPolicy, PortPair, RemoteEndpoint, RtcpPacket, RtpPacket, RtpSocketConfig, RtpSocketPair,
    SrtpContext,
};
use forge_webrtc::{
    ConnectionState, Direction, PeerConfig, PeerConnection, PeerEvent, VideoCodec, VideoConfig,
};
use tokio::sync::{mpsc, Mutex, Notify};

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fcp_webrtc_gateway=debug,forge_webrtc=info".into()),
        )
        .with_test_writer()
        .try_init();
}

fn peer_cfg() -> PeerConfig {
    PeerConfig {
        codecs: browser_audio_codecs(),
        video: Some(VideoConfig {
            codecs: vec![(VideoCodec::VP8, 96)],
            direction: Direction::SendRecv,
            content: true,
            ..VideoConfig::default()
        }),
        ..PeerConfig::default()
    }
}

/// Split a peer's events into trickled candidates and everything else.
fn pump(
    mut events: mpsc::Receiver<PeerEvent>,
) -> (
    mpsc::UnboundedReceiver<forge_webrtc::IceCandidate>,
    mpsc::Receiver<PeerEvent>,
) {
    let (cand_tx, cand_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel(4096);
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                PeerEvent::LocalCandidate(c) => {
                    let _ = cand_tx.send(c);
                }
                other => {
                    let _ = ev_tx.send(other).await;
                }
            }
        }
    });
    (cand_rx, ev_rx)
}

/// The gateway's peer (offerer) connected to a "browser" peer (answerer).
async fn connected_pair() -> (
    PeerConnection,
    PeerConnection,
    mpsc::Receiver<PeerEvent>,
    mpsc::Receiver<PeerEvent>,
) {
    let mut gateway = PeerConnection::with_config(peer_cfg()).await.unwrap();
    let mut browser = PeerConnection::with_config(peer_cfg()).await.unwrap();
    let offer = gateway.create_offer().await.unwrap();
    let (mut gw_cands, gw_events) = pump(gateway.take_events().unwrap());
    browser.set_remote_offer(&offer).await.unwrap();
    let answer = browser.create_answer().await.unwrap();
    let (mut br_cands, br_events) = pump(browser.take_events().unwrap());
    gateway.set_remote_answer(&answer).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        tokio::select! {
            Some(c) = gw_cands.recv() => browser.add_ice_candidate(c).await.unwrap(),
            Some(c) = br_cands.recv() => gateway.add_ice_candidate(c).await.unwrap(),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        if gateway.get_state() == ConnectionState::Connected
            && browser.get_state() == ConnectionState::Connected
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "peers did not connect"
        );
    }
    (gateway, browser, gw_events, br_events)
}

/// A socket pair bound on a free loopback port. Ports are probed and then
/// bound, so another socket can take one in between; that is retried
/// rather than failed (a fixed range here collided with other tests'
/// sockets on CI).
async fn bound_pair(cfg: &RtpSocketConfig) -> (RtpSocketPair, u16) {
    for _ in 0..20 {
        let ports = fcp_testkit::media::allocate_port_pair();
        let port = ports.rtp_port;
        if let Ok(pair) = RtpSocketPair::new(ports, cfg.clone()).await {
            return (pair, port);
        }
    }
    panic!("no loopback socket pair after 20 attempts");
}

/// Two socket pairs on loopback pointed at each other.
async fn socket_pair(tos: u8) -> (Arc<RtpSocketPair>, Arc<RtpSocketPair>) {
    let cfg = RtpSocketConfig {
        bind_addr: "127.0.0.1".parse().unwrap(),
        tos,
        ..RtpSocketConfig::default()
    };
    let (a, a_port) = bound_pair(&cfg).await;
    let (b, b_port) = bound_pair(&cfg).await;
    for (s, peer) in [(&a, b_port), (&b, a_port)] {
        s.set_remote_endpoint(RemoteEndpoint {
            rtp_addr: format!("127.0.0.1:{peer}").parse().unwrap(),
            rtcp_addr: Some(format!("127.0.0.1:{}", peer + 1).parse().unwrap()),
        })
        .await;
        let mut allowed = HashSet::new();
        allowed.insert("127.0.0.1".parse().unwrap());
        s.set_latch_policy(LatchPolicy::AllowedIps(allowed)).await;
    }
    (Arc::new(a), Arc::new(b))
}

/// Mirrored SRTP contexts for one section, as an offer/answer would leave them.
fn srtp_pair() -> (Arc<Mutex<SrtpContext>>, Arc<Mutex<SrtpContext>>) {
    let ours = SrtpOffer::generate().unwrap();
    let theirs = SrtpOffer::generate().unwrap();
    let their_line = fcp_media::SdesCryptoLine {
        tag: 1,
        suite: SrtpOffer::SUITE.to_string(),
        key_params: theirs.sdp.local_key_b64.clone(),
    };
    let our_line = fcp_media::SdesCryptoLine {
        tag: 1,
        suite: SrtpOffer::SUITE.to_string(),
        key_params: ours.sdp.local_key_b64.clone(),
    };
    let relay_ctx = ours.accept(&their_line).unwrap();
    let far_ctx = theirs.accept(&our_line).unwrap();
    (
        Arc::new(Mutex::new(relay_ctx)),
        Arc::new(Mutex::new(far_ctx)),
    )
}

async fn recv_rtp(socket: &RtpSocketPair, srtp: &Option<Arc<Mutex<SrtpContext>>>) -> RtpPacket {
    let (raw, _) = tokio::time::timeout(Duration::from_secs(5), socket.recv_rtp_raw())
        .await
        .expect("timed out waiting for RTP on the far end")
        .unwrap();
    let plain = match srtp {
        Some(ctx) => Bytes::from(ctx.lock().await.unprotect_rtp(&raw).unwrap()),
        None => raw,
    };
    RtpPacket::parse(plain).unwrap()
}

async fn send_rtp(socket: &RtpSocketPair, srtp: &Option<Arc<Mutex<SrtpContext>>>, pkt: &RtpPacket) {
    let plain = pkt.to_bytes();
    let out = match srtp {
        Some(ctx) => ctx.lock().await.protect_rtp(&plain).unwrap(),
        None => plain.to_vec(),
    };
    socket.send_rtp_raw(&out).await.unwrap();
}

async fn next_event(
    events: &mut mpsc::Receiver<PeerEvent>,
    want: impl Fn(&PeerEvent) -> bool,
) -> PeerEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("timed out waiting for a browser event")
            .expect("browser events closed");
        if want(&ev) {
            return ev;
        }
    }
}

async fn relay_case(srtp: bool) {
    init_tracing();
    let (gateway, browser, gw_events, mut br_events) = connected_pair().await;
    let (codec, browser_audio_pt) = gateway.negotiated_codec();
    assert_eq!(codec, forge_webrtc::AudioCodec::G722);
    assert_eq!(browser_audio_pt, 9);
    let browser_te = fcp_media::extract_telephone_event_pt(gateway.remote_sdp().unwrap());
    assert_eq!(browser_te, Some(101));
    let video = gateway
        .negotiated_video()
        .cloned()
        .expect("video negotiated");
    assert_eq!(video.payload_type, 96);
    let content = gateway
        .negotiated_content()
        .cloned()
        .expect("a shared screen's section negotiated");
    assert_eq!(content.payload_type, 96);

    // The SIP side: the relay's sockets and the far end's, plain or SRTP,
    // one pair per section.
    let (relay_audio, far_audio) = socket_pair(0xB8).await;
    let (relay_video, far_video) = socket_pair(0xA0).await;
    let (relay_content, far_content) = socket_pair(0xA0).await;
    let pair = || {
        if srtp {
            let (a, b) = srtp_pair();
            (Some(a), Some(b))
        } else {
            (None, None)
        }
    };
    let (audio_srtp, far_audio_srtp) = pair();
    let (video_srtp, far_video_srtp) = pair();
    let (content_srtp, far_content_srtp) = pair();
    // The far end answered with different payload types for each.
    let sip = SipLeg {
        audio: SipAudioLeg {
            socket: relay_audio,
            srtp: audio_srtp,
            payload_type: 9,
            telephone_event_pt: Some(120),
        },
        video: Some(SipVideoLeg {
            socket: relay_video,
            srtp: video_srtp,
            payload_type: 121,
        }),
        content: Some(SipVideoLeg {
            socket: relay_content,
            srtp: content_srtp,
            payload_type: 122,
        }),
    };
    let video_sender = gateway.video_sender().unwrap();
    let our_video_ssrc = video_sender.ssrc();
    let content_sender = gateway.content_sender().unwrap();
    let our_content_ssrc = content_sender.ssrc();
    assert_ne!(our_video_ssrc, our_content_ssrc);
    let browser_leg = BrowserLeg {
        events: gw_events,
        audio: gateway.sender().unwrap(),
        audio_payload_type: browser_audio_pt,
        telephone_event_pt: browser_te,
        video: Some((video_sender, 96)),
        content: Some((content_sender, 96)),
        inactivity_timeout: Duration::from_secs(30),
    };
    let stats = Arc::new(RelayStats::default());
    let stop = Arc::new(Notify::new());
    let (_swap_tx, swap_rx) = mpsc::channel(1);
    let relay = tokio::spawn(run_relay(
        browser_leg,
        sip,
        stats.clone(),
        stop.clone(),
        swap_rx,
    ));

    // Browser → far end: audio, a DTMF event, video.
    let br_audio = browser.sender().unwrap();
    br_audio
        .send_audio(Bytes::from_static(&[0x55; 160]), 160)
        .await
        .unwrap();
    let got = recv_rtp(&far_audio, &far_audio_srtp).await;
    assert_eq!(got.header.payload_type(), 9);
    assert_eq!(got.payload.len(), 160);
    assert_eq!({ got.header.ssrc }, browser.ssrc());
    br_audio
        .send_rtp(101, true, 1_000, Bytes::from_static(&[5, 0x8a, 0, 160]))
        .await
        .unwrap();
    let got = recv_rtp(&far_audio, &far_audio_srtp).await;
    assert_eq!(got.header.payload_type(), 120, "telephone-event re-tagged");
    assert!(got.header.marker());

    let br_video = browser.video_sender().unwrap();
    for seq in 0..3u16 {
        let pkt = RtpPacket::build(
            96,
            500 + seq,
            9000,
            0xABCD,
            Bytes::from(vec![seq as u8; 200]),
            seq == 2,
        );
        br_video.send_packet(pkt.to_bytes().freeze()).await.unwrap();
    }
    for seq in 0..3u16 {
        let got = recv_rtp(&far_video, &far_video_srtp).await;
        assert_eq!(
            got.header.payload_type(),
            121,
            "video re-tagged for the SIP side"
        );
        assert_eq!({ got.header.sequence_number }, 500 + seq);
        assert_eq!(
            { got.header.ssrc },
            browser.video_ssrc(),
            "browser SSRC preserved towards SIP"
        );
        assert_eq!(got.header.marker(), seq == 2);
    }

    // The browser's screen, on the second section: its own SSRC, the
    // content section's payload type on the SIP side.
    let br_screen = browser.content_sender().unwrap();
    for seq in 0..2u16 {
        let pkt = RtpPacket::build(
            96,
            700 + seq,
            12_000,
            0x5C5C,
            Bytes::from(vec![0x50 + seq as u8; 240]),
            seq == 1,
        );
        br_screen
            .send_packet(pkt.to_bytes().freeze())
            .await
            .unwrap();
    }
    for seq in 0..2u16 {
        let got = recv_rtp(&far_content, &far_content_srtp).await;
        assert_eq!(
            got.header.payload_type(),
            122,
            "screen re-tagged for the SIP side"
        );
        assert_eq!({ got.header.sequence_number }, 700 + seq);
        assert_eq!({ got.header.ssrc }, browser.content_ssrc());
        assert_eq!(got.payload.len(), 240);
    }

    // Far end → browser: audio (re-tagged for the browser), video (SSRC
    // becomes the gateway's, sequence kept).
    let pkt = RtpPacket::build(
        9,
        77,
        4_000,
        0xF00D,
        Bytes::from_static(&[0x33; 160]),
        false,
    );
    send_rtp(&far_audio, &far_audio_srtp, &pkt).await;
    let ev = next_event(
        &mut br_events,
        |e| matches!(e, PeerEvent::Rtp(p) if p.payload.as_ref() == [0x33; 160]),
    )
    .await;
    let PeerEvent::Rtp(p) = ev else {
        unreachable!()
    };
    assert_eq!(p.header.payload_type(), 9);
    assert_eq!({ p.header.timestamp }, 4_000);
    assert_eq!({ p.header.ssrc }, gateway.ssrc());
    let te = RtpPacket::build(
        120,
        78,
        4_160,
        0xF00D,
        Bytes::from_static(&[1, 0x8a, 0, 160]),
        true,
    );
    send_rtp(&far_audio, &far_audio_srtp, &te).await;
    let ev = next_event(
        &mut br_events,
        |e| matches!(e, PeerEvent::Rtp(p) if p.header.payload_type() == 101),
    )
    .await;
    let PeerEvent::Rtp(p) = ev else {
        unreachable!()
    };
    assert!(p.header.marker());

    let far_video_ssrc = 0xCAFE_F00Du32;
    let vpkt = RtpPacket::build(
        121,
        9_000,
        1_234_567,
        far_video_ssrc,
        Bytes::from(vec![7u8; 300]),
        true,
    );
    send_rtp(&far_video, &far_video_srtp, &vpkt).await;
    let ev = next_event(&mut br_events, |e| matches!(e, PeerEvent::VideoRtp(_))).await;
    let PeerEvent::VideoRtp(p) = ev else {
        unreachable!()
    };
    assert_eq!(p.header.payload_type(), 96);
    assert_eq!({ p.header.sequence_number }, 9_000);
    assert_eq!({ p.header.timestamp }, 1_234_567);
    assert_eq!({ p.header.ssrc }, our_video_ssrc);
    assert_eq!(p.payload.len(), 300);

    // The room's shared screen, on the second section, under the
    // gateway's content SSRC, sorted onto the content event by that SSRC.
    let far_content_ssrc = 0xC0FF_EE00u32;
    let cpkt = RtpPacket::build(
        122,
        4_242,
        99_000,
        far_content_ssrc,
        Bytes::from(vec![9u8; 500]),
        true,
    );
    send_rtp(&far_content, &far_content_srtp, &cpkt).await;
    let ev = next_event(&mut br_events, |e| matches!(e, PeerEvent::ContentRtp(_))).await;
    let PeerEvent::ContentRtp(p) = ev else {
        unreachable!()
    };
    assert_eq!(p.header.payload_type(), 96);
    assert_eq!({ p.header.sequence_number }, 4_242);
    assert_eq!({ p.header.ssrc }, our_content_ssrc);
    assert_eq!(p.payload.len(), 500);

    // Feedback both ways, re-addressed. The browser asks for a keyframe
    // of what it receives (our SSRC): the far end hears it about its own.
    browser
        .send_rtcp(&[RtcpPacket::PayloadFeedback(PsFeedback::pli(
            browser.video_ssrc(),
            our_video_ssrc,
        ))])
        .await
        .unwrap();
    let (raw, _) = tokio::time::timeout(Duration::from_secs(5), far_video.recv_rtcp())
        .await
        .expect("timed out waiting for feedback on the far end")
        .unwrap();
    let plain = match &far_video_srtp {
        Some(ctx) => ctx.lock().await.unprotect_rtcp(&raw).unwrap(),
        None => raw.to_vec(),
    };
    let packets = RtcpPacket::parse_compound(&plain);
    let pli = packets
        .iter()
        .find_map(|p| match p {
            RtcpPacket::PayloadFeedback(fb) if matches!(fb.kind, PayloadFeedback::Pli) => Some(fb),
            _ => None,
        })
        .expect("PLI reached the far end");
    assert_eq!(pli.media_ssrc, far_video_ssrc);
    assert_eq!(pli.sender_ssrc, browser.video_ssrc());

    // The far end NACKs the browser's stream: the browser hears it about
    // its own SSRC from the gateway.
    let nack = RtcpPacket::TransportFeedback(RtpFeedback::nack(
        far_video_ssrc,
        browser.video_ssrc(),
        &[501],
    ));
    let bytes = nack.to_bytes();
    let out = match &far_video_srtp {
        Some(ctx) => ctx.lock().await.protect_rtcp(&bytes).unwrap(),
        None => bytes,
    };
    far_video.send_rtcp(&out).await.unwrap();
    let ev = next_event(&mut br_events, |e| {
        matches!(e, PeerEvent::Rtcp(ps) if ps.iter().any(|p| matches!(p, RtcpPacket::TransportFeedback(_))))
    })
    .await;
    let PeerEvent::Rtcp(ps) = ev else {
        unreachable!()
    };
    let fb = ps
        .iter()
        .find_map(|p| match p {
            RtcpPacket::TransportFeedback(fb) => Some(fb),
            _ => None,
        })
        .unwrap();
    assert_eq!(fb.media_ssrc, browser.video_ssrc());
    assert_eq!(fb.sender_ssrc, our_video_ssrc);
    assert!(matches!(&fb.kind, TransportFeedback::Nack(e) if e[0].lost() == vec![501]));

    // Feedback about the screen goes to the screen's far end, not the
    // camera's, and back again about the browser's screen SSRC.
    browser
        .send_rtcp(&[RtcpPacket::PayloadFeedback(PsFeedback::pli(
            browser.content_ssrc(),
            our_content_ssrc,
        ))])
        .await
        .unwrap();
    let (raw, _) = tokio::time::timeout(Duration::from_secs(5), far_content.recv_rtcp())
        .await
        .expect("timed out waiting for screen feedback on the far end")
        .unwrap();
    let plain = match &far_content_srtp {
        Some(ctx) => ctx.lock().await.unprotect_rtcp(&raw).unwrap(),
        None => raw.to_vec(),
    };
    let pli = RtcpPacket::parse_compound(&plain)
        .into_iter()
        .find_map(|p| match p {
            RtcpPacket::PayloadFeedback(fb) if matches!(fb.kind, PayloadFeedback::Pli) => Some(fb),
            _ => None,
        })
        .expect("PLI reached the screen's far end");
    assert_eq!(pli.media_ssrc, far_content_ssrc);
    assert_eq!(pli.sender_ssrc, browser.content_ssrc());
    let nack = RtcpPacket::TransportFeedback(RtpFeedback::nack(
        far_content_ssrc,
        browser.content_ssrc(),
        &[701],
    ));
    let bytes = nack.to_bytes();
    let out = match &far_content_srtp {
        Some(ctx) => ctx.lock().await.protect_rtcp(&bytes).unwrap(),
        None => bytes,
    };
    far_content.send_rtcp(&out).await.unwrap();
    let ev = next_event(&mut br_events, |e| {
        matches!(e, PeerEvent::Rtcp(ps) if ps.iter().any(|p| matches!(p, RtcpPacket::TransportFeedback(fb) if fb.media_ssrc == browser.content_ssrc())))
    })
    .await;
    let PeerEvent::Rtcp(ps) = ev else {
        unreachable!()
    };
    let fb = ps
        .iter()
        .find_map(|p| match p {
            RtcpPacket::TransportFeedback(fb) if fb.media_ssrc == browser.content_ssrc() => {
                Some(fb)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(fb.sender_ssrc, our_content_ssrc);
    assert!(matches!(&fb.kind, TransportFeedback::Nack(e) if e[0].lost() == vec![701]));

    let snap = stats.snapshot();
    assert_eq!(snap.audio_to_sip, 2);
    assert_eq!(snap.video_to_sip, 3);
    assert_eq!(snap.content_to_sip, 2);
    assert_eq!(snap.audio_to_browser, 2);
    assert_eq!(snap.video_to_browser, 1);
    assert_eq!(snap.content_to_browser, 1);
    assert_eq!(snap.feedback_to_sip, 2);
    assert_eq!(snap.feedback_to_browser, 2);
    assert_eq!(snap.srtp_errors, 0);
    assert!(stats.content_sent_within(Duration::from_secs(5)));

    stop.notify_one();
    assert_eq!(relay.await.unwrap(), RelayEnd::Stopped);
}

#[tokio::test]
async fn relays_audio_video_and_feedback_both_ways_plain() {
    relay_case(false).await;
}

#[tokio::test]
async fn relays_audio_video_and_feedback_both_ways_srtp() {
    relay_case(true).await;
}

#[tokio::test]
async fn relay_ends_when_the_browser_closes() {
    init_tracing();
    let (gateway, mut browser, gw_events, _br_events) = connected_pair().await;
    let (relay_audio, _far_audio) = socket_pair(0xB8).await;
    let sip = SipLeg {
        audio: SipAudioLeg {
            socket: relay_audio,
            srtp: None,
            payload_type: 9,
            telephone_event_pt: None,
        },
        video: None,
        content: None,
    };
    let browser_leg = BrowserLeg {
        events: gw_events,
        audio: gateway.sender().unwrap(),
        audio_payload_type: 9,
        telephone_event_pt: None,
        video: None,
        content: None,
        inactivity_timeout: Duration::from_secs(2),
    };
    let stop = Arc::new(Notify::new());
    let (_swap_tx, swap_rx) = mpsc::channel(1);
    let relay = tokio::spawn(run_relay(
        browser_leg,
        sip,
        Arc::new(RelayStats::default()),
        stop,
        swap_rx,
    ));
    // A video packet from the browser with no SIP video leg is dropped, not fatal.
    let vs = browser.video_sender().unwrap();
    vs.send_packet(
        RtpPacket::build(96, 1, 0, 1, Bytes::from_static(&[1; 10]), true)
            .to_bytes()
            .freeze(),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    browser.close();
    let end = tokio::time::timeout(Duration::from_secs(10), relay)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&end, RelayEnd::BrowserFailed(why) if why.contains("no media or RTCP")),
        "{end:?}"
    );
}

/// The SIP leg swapped under a running relay (phase 5c): the browser's
/// audio goes to the new far end, the new far end's audio reaches the
/// browser, and the old far end hears nothing more.
#[tokio::test]
async fn the_sip_leg_can_be_swapped_under_the_browser() {
    init_tracing();
    let (gateway, browser, gw_events, mut br_events) = connected_pair().await;
    let (_, browser_audio_pt) = gateway.negotiated_codec();
    let (relay_audio, far_audio) = socket_pair(0xB8).await;
    let sip = SipLeg {
        audio: SipAudioLeg {
            socket: relay_audio,
            srtp: None,
            payload_type: 9,
            telephone_event_pt: None,
        },
        video: None,
        content: None,
    };
    let browser_leg = BrowserLeg {
        events: gw_events,
        audio: gateway.sender().unwrap(),
        audio_payload_type: browser_audio_pt,
        telephone_event_pt: None,
        video: None,
        content: None,
        inactivity_timeout: Duration::from_secs(30),
    };
    let stats = Arc::new(RelayStats::default());
    let stop = Arc::new(Notify::new());
    let (swap_tx, swap_rx) = mpsc::channel(1);
    let relay = tokio::spawn(run_relay(
        browser_leg,
        sip,
        stats.clone(),
        stop.clone(),
        swap_rx,
    ));

    // Before the swap: the first far end hears the browser.
    let br_audio = browser.sender().unwrap();
    br_audio
        .send_audio(Bytes::from_static(&[0x11; 160]), 160)
        .await
        .unwrap();
    let got = recv_rtp(&far_audio, &None).await;
    assert_eq!(got.payload[0], 0x11);

    // A new far end, SRTP this time, with a different payload type: the
    // relay takes the new leg between packets.
    let (relay_audio2, far_audio2) = socket_pair(0xB8).await;
    let (audio_srtp, far_audio_srtp) = srtp_pair();
    let far_audio_srtp = Some(far_audio_srtp);
    swap_tx
        .send(SipLeg {
            audio: SipAudioLeg {
                socket: relay_audio2,
                srtp: Some(audio_srtp),
                payload_type: 96,
                telephone_event_pt: None,
            },
            video: None,
            content: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // After: the browser's audio reaches the new far end, re-tagged and
    // protected under the new key.
    br_audio
        .send_audio(Bytes::from_static(&[0x22; 160]), 320)
        .await
        .unwrap();
    let got = recv_rtp(&far_audio2, &far_audio_srtp).await;
    assert_eq!(got.header.payload_type(), 96);
    assert_eq!(got.payload[0], 0x22);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), far_audio.recv_rtp_raw())
            .await
            .is_err(),
        "the old far end hears nothing after the swap"
    );

    // And the new far end's audio reaches the browser, re-tagged.
    let pkt = RtpPacket::build(96, 7, 4800, 0xF00D, Bytes::from_static(&[0x33; 160]), false);
    send_rtp(&far_audio2, &far_audio_srtp, &pkt).await;
    let ev = next_event(&mut br_events, |e| matches!(e, PeerEvent::Rtp(_))).await;
    let PeerEvent::Rtp(pkt) = ev else {
        unreachable!()
    };
    assert_eq!(pkt.header.payload_type(), browser_audio_pt);
    assert_eq!(pkt.payload[0], 0x33);

    stop.notify_one();
    let end = tokio::time::timeout(Duration::from_secs(5), relay)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, RelayEnd::Stopped);
}
