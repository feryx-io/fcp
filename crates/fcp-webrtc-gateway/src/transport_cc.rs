//! Transport-cc across the gateway (`docs/TRANSPORT_CC.md` §3.4).
//!
//! **Downlink.** The room numbers what it sends on each SIP section, and
//! the browser's connection is one BUNDLE with a sequence of its own. So
//! [`Downlink`] numbers each room packet again as it goes to the browser,
//! remembering which room number it carried. It then rewrites the
//! browser's feedback into each section's room numbers, the arrival
//! times as the browser reported them. The room estimates, and probes,
//! the whole path to the browser, as an SFU that ends the browser's
//! connection itself would.
//!
//! **Uplink.** The browser's connection ends here, so the gateway is the
//! receiver the browser's own estimate needs: it writes the feedback
//! itself, from the browser's numbers ([`forge_bwe::FeedbackWriter`]).

use bytes::Bytes;
use forge_rtp::transport_cc::{
    stamp, transport_sequence, PacketStatus, TransportCcFeedback, DELTA_TICK_US,
    MAX_PACKETS_PER_FEEDBACK, REFERENCE_TICK_US,
};
use forge_rtp::{RtpExtension, RtpPacket};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// The extension id the gateway offers transport-cc under on its SIP
/// sections. Not the browser's (3): a browser packet carries its own
/// number there, which means nothing to the room.
pub const SIP_EXTENSION_ID: u8 = 5;

/// How many packets sent to the browser are remembered for its feedback.
const REMEMBERED: usize = 8192;

/// A SIP section of the gateway's leg to the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Camera,
    Content,
}

/// Sequence numbers as they run on past 65535.
#[derive(Debug, Default, Clone, Copy)]
struct Unwrap {
    last: Option<i64>,
}

impl Unwrap {
    fn next(&mut self, seq: u16) -> i64 {
        let s = match self.last {
            None => i64::from(seq),
            Some(l) => l + i64::from(seq.wrapping_sub(l as u16) as i16),
        };
        self.last = Some(self.last.map_or(s, |l| l.max(s)));
        s
    }

    fn peek(&self, seq: u16) -> i64 {
        match self.last {
            None => i64::from(seq),
            Some(l) => l + i64::from(seq.wrapping_sub(l as u16) as i16),
        }
    }
}

#[derive(Debug, Default)]
struct Room {
    unwrap: Unwrap,
    /// Room numbers that reached the gateway, recent ones.
    seen: BTreeMap<i64, ()>,
    count: u8,
}

/// The downlink's renumbering.
#[derive(Debug)]
pub struct Downlink {
    browser_ext: u8,
    next: u16,
    unwrap: Unwrap,
    /// (browser number, section, room number), in the browser's order.
    sent: VecDeque<(i64, Section, i64)>,
    rooms: HashMap<Section, Room>,
}

impl Downlink {
    /// `browser_ext`: the id the browser's connection agreed.
    pub fn new(browser_ext: u8) -> Self {
        Self {
            browser_ext,
            next: rand::random(),
            unwrap: Unwrap::default(),
            sent: VecDeque::new(),
            rooms: HashMap::new(),
        }
    }

    /// The room's numbering starts over (a new SIP leg).
    pub fn reset_room(&mut self) {
        self.rooms.clear();
        self.sent.clear();
    }

    /// A room packet from `section` (plain RTP), on its way to the browser:
    /// numbered for the browser's connection in place of the room's
    /// number (under `sip_ext`). A packet the room did not number goes as
    /// it is.
    pub fn forward(&mut self, plain: Bytes, section: Section, sip_ext: u8) -> Bytes {
        let Ok(mut packet) = RtpPacket::parse(plain.clone()) else {
            return plain;
        };
        let Some(room_seq) = transport_sequence(&packet, sip_ext) else {
            return plain;
        };
        let room = self.rooms.entry(section).or_default();
        let room_seq = room.unwrap.next(room_seq);
        room.seen.insert(room_seq, ());
        while room.seen.len() > REMEMBERED {
            room.seen.pop_first();
        }
        // The room's number off, the browser's on.
        if let Some(ext) = packet.extension.take() {
            let kept: Vec<(u8, Vec<u8>)> = ext
                .elements()
                .into_iter()
                .filter(|(id, _)| *id != sip_ext)
                .map(|(id, d)| (id, d.to_vec()))
                .collect();
            if !kept.is_empty() {
                let refs: Vec<(u8, &[u8])> = kept.iter().map(|(i, d)| (*i, d.as_slice())).collect();
                packet.extension = Some(RtpExtension::one_byte(&refs));
            }
        }
        let seq = self.next;
        self.next = seq.wrapping_add(1);
        stamp(&mut packet, self.browser_ext, seq);
        let ours = self.unwrap.next(seq);
        self.sent.push_back((ours, section, room_seq));
        while self.sent.len() > REMEMBERED {
            self.sent.pop_front();
        }
        packet.to_bytes().freeze()
    }

    /// The browser's feedback, in the room's numbers: a message per
    /// section it covers.
    ///
    /// A room number that never reached the gateway is reported lost. So
    /// is one that reached it but is not in this message, which happens
    /// only when the gateway sent packets in another order than the room
    /// did; the room takes the loss.
    pub fn translate(&mut self, fb: &TransportCcFeedback) -> Vec<(Section, TransportCcFeedback)> {
        let mut by_section: HashMap<Section, BTreeMap<i64, Option<i64>>> = HashMap::new();
        for (seq, arrival) in fb.arrivals() {
            let ours = self.unwrap.peek(seq);
            let Ok(i) = self.sent.binary_search_by_key(&ours, |(s, _, _)| *s) else {
                continue;
            };
            let (_, section, room_seq) = self.sent[i];
            by_section
                .entry(section)
                .or_default()
                .insert(room_seq, arrival);
        }
        let mut out = Vec::new();
        for (section, reported) in by_section {
            let Some(room) = self.rooms.get_mut(&section) else {
                continue;
            };
            let (Some(&first), Some(&last)) = (reported.keys().next(), reported.keys().next_back())
            else {
                continue;
            };
            let statuses: Vec<(i64, Option<i64>)> = (first..=last)
                .map(|s| (s, reported.get(&s).copied().flatten()))
                .collect();
            for chunk in statuses.chunks(MAX_PACKETS_PER_FEEDBACK) {
                if let Some(msg) = build(chunk, room.count) {
                    room.count = room.count.wrapping_add(1);
                    out.push((section, msg));
                }
            }
        }
        out
    }
}

/// A feedback message for `statuses` (room number, arrival in µs), which
/// run on one after another.
fn build(statuses: &[(i64, Option<i64>)], count: u8) -> Option<TransportCcFeedback> {
    let base = statuses.first()?.0;
    let first_arrival = statuses.iter().find_map(|(_, a)| *a)?;
    let reference = first_arrival.div_euclid(REFERENCE_TICK_US);
    let mut last_tick = reference * (REFERENCE_TICK_US / DELTA_TICK_US);
    let packets = statuses
        .iter()
        .map(|(_, a)| match a {
            None => PacketStatus::NotReceived,
            Some(us) => {
                let tick = (us + DELTA_TICK_US / 2).div_euclid(DELTA_TICK_US);
                let delta = tick - last_tick;
                if delta < i64::from(i16::MIN) || delta > i64::from(i16::MAX) {
                    // Too far from the one before to say: as good as lost.
                    return PacketStatus::NotReceived;
                }
                last_tick = tick;
                PacketStatus::Received(delta as i16)
            }
        })
        .collect();
    Some(TransportCcFeedback {
        base_sequence: base as u16,
        reference_time: ((((reference & 0xFF_FFFF) as u32) << 8) as i32) >> 8,
        feedback_count: count,
        packets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROWSER: u8 = 3;

    fn room_packet(room_seq: u16, ssrc: u32) -> Bytes {
        let mut p = RtpPacket::build(96, 1, 0, ssrc, Bytes::from_static(&[0x10, 0, 1]), false);
        stamp(&mut p, SIP_EXTENSION_ID, room_seq);
        p.to_bytes().freeze()
    }

    fn browser_seq(bytes: &Bytes) -> (u16, bool) {
        let p = RtpPacket::parse(bytes.clone()).unwrap();
        (
            transport_sequence(&p, BROWSER).unwrap(),
            transport_sequence(&p, SIP_EXTENSION_ID).is_some(),
        )
    }

    #[test]
    fn packets_are_renumbered_and_feedback_comes_back_per_section_in_the_room_s_numbers() {
        let mut d = Downlink::new(BROWSER);
        // Camera 100..=103 with 102 lost before the gateway; content 7, 8
        // interleaved.
        let mut ours = Vec::new();
        for (section, seq) in [
            (Section::Camera, 100),
            (Section::Content, 7),
            (Section::Camera, 101),
            (Section::Camera, 103),
            (Section::Content, 8),
        ] {
            let out = d.forward(room_packet(seq, 1), section, SIP_EXTENSION_ID);
            let (b, room_left) = browser_seq(&out);
            assert!(!room_left, "the room's number is taken off");
            ours.push(b);
        }
        for w in ours.windows(2) {
            assert_eq!(w[1], w[0].wrapping_add(1), "one gapless sequence: {ours:?}");
        }
        // The browser got all but the content's 8 (our fifth).
        let fb = TransportCcFeedback {
            base_sequence: ours[0],
            reference_time: 10,
            feedback_count: 0,
            packets: vec![
                PacketStatus::Received(4),
                PacketStatus::Received(4),
                PacketStatus::Received(4),
                PacketStatus::Received(4),
                PacketStatus::NotReceived,
            ],
        };
        let mut out = d.translate(&fb);
        out.sort_by_key(|(s, _)| *s == Section::Content);
        assert_eq!(out.len(), 2);
        let (s, camera) = &out[0];
        assert_eq!(*s, Section::Camera);
        assert_eq!(camera.base_sequence, 100);
        let cam = camera.arrivals();
        assert_eq!(cam.len(), 4);
        assert!(cam[0].1.is_some() && cam[1].1.is_some());
        assert_eq!(cam[2], (102, None), "lost on the way to the gateway");
        // Arrival times are the browser's: 100 at +1 ms, 103 at +4 ms.
        assert_eq!(cam[3].1.unwrap() - cam[0].1.unwrap(), 3_000);
        let (_, content) = &out[1];
        assert_eq!(content.base_sequence, 7);
        assert_eq!(content.arrivals(), {
            let a = content.arrivals();
            vec![(7, a[0].1), (8, None)]
        });
        assert!(content.arrivals()[0].1.is_some());
    }

    #[test]
    fn a_packet_the_room_did_not_number_goes_as_it_is() {
        let mut d = Downlink::new(BROWSER);
        let plain = RtpPacket::build(96, 1, 0, 9, Bytes::from_static(&[1]), false)
            .to_bytes()
            .freeze();
        assert_eq!(
            d.forward(plain.clone(), Section::Camera, SIP_EXTENSION_ID),
            plain
        );
        // Feedback about numbers we never gave says nothing to the room.
        let fb = TransportCcFeedback {
            base_sequence: 1,
            reference_time: 0,
            feedback_count: 0,
            packets: vec![PacketStatus::Received(1)],
        };
        assert!(d.translate(&fb).is_empty());
    }
}
