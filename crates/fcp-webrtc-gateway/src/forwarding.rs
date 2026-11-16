//! A forwarding room's streams to one browser (`docs/FORWARDING_ROOMS.md`
//! §2.7). The room sends each person the browser sees under a stable SSRC
//! of its own; the browser took K send-only sections at the start, each a
//! track. Each SSRC is put on a free section when its first packet comes,
//! and the section is free again once the stream stops, so people come and
//! go without renegotiating. The room's roster, taken by subscription in
//! the call, says whose each SSRC is; the page reads both from the session.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fcp_core::conference_info::{Member, Roster};
use serde::Serialize;

/// A section whose stream sent nothing for this long is free again.
pub const IDLE: Duration = Duration::from_secs(3);

/// With every section taken, one whose stream the roster no longer names
/// and that has been quiet this long gives way to a new stream.
const GONE: Duration = Duration::from_millis(500);

/// The stream on one section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    ssrc: u32,
    last: Instant,
}

/// One browser's sections, the streams on them, and the room's roster.
#[derive(Debug, Default)]
pub struct Forwarding {
    /// Each section's `a=mid`, by index.
    mids: Vec<String>,
    /// Each section the browser kept (a rejected one carries nothing).
    usable: Vec<bool>,
    slots: Vec<Option<Slot>>,
    roster: Roster,
}

/// Whose stream is on a section, as the page reads it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SectionStream {
    /// The section's `a=mid`: the browser's transceiver for it.
    pub mid: String,
    /// The member it shows, by the room's id, once the roster says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<String>,
    /// It follows whoever speaks; `participant` is whom it shows now.
    pub speaker: bool,
}

/// The room as a forwarding browser sees it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ForwardingSummary {
    /// Everyone in the room, as its roster lists them.
    pub members: Vec<Member>,
    /// Who speaks, or last spoke, by id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// The streams on the browser's sections now.
    pub streams: Vec<SectionStream>,
}

impl Forwarding {
    /// Sections by `a=mid`, each usable or not (the answer rejected it).
    pub fn new(mids: Vec<String>, usable: Vec<bool>) -> Self {
        let slots = vec![None; mids.len()];
        Self {
            mids,
            usable,
            slots,
            roster: Roster::default(),
        }
    }

    /// The section a packet on `ssrc` goes out on: the one it has, else a
    /// free one; `None` when every section is taken.
    pub fn slot_for(&mut self, ssrc: u32, now: Instant) -> Option<usize> {
        if let Some(i) = self
            .slots
            .iter()
            .position(|s| s.is_some_and(|s| s.ssrc == ssrc))
        {
            if let Some(s) = self.slots[i].as_mut() {
                s.last = now;
            }
            return Some(i);
        }
        let named = self.roster.sources();
        let free = (0..self.slots.len())
            .filter(|&i| self.usable.get(i).copied().unwrap_or(false))
            .find(|&i| match self.slots[i] {
                None => true,
                Some(s) => {
                    let quiet = now.saturating_duration_since(s.last);
                    quiet >= IDLE || (quiet >= GONE && !named.contains_key(&s.ssrc))
                }
            })?;
        self.slots[free] = Some(Slot { ssrc, last: now });
        Some(free)
    }

    /// Free the sections whose streams have stopped.
    pub fn expire(&mut self, now: Instant) {
        for slot in &mut self.slots {
            if slot.is_some_and(|s| now.saturating_duration_since(s.last) >= IDLE) {
                *slot = None;
            }
        }
    }

    /// The room's SSRC on section `index`, while one is.
    pub fn ssrc_on(&self, index: usize) -> Option<u32> {
        self.slots.get(index).copied().flatten().map(|s| s.ssrc)
    }

    /// Every section free: the room's SSRCs start over (a move to another
    /// node).
    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = None);
        self.roster = Roster::default();
    }

    /// The roster, to take a document into.
    pub fn roster_mut(&mut self) -> &mut Roster {
        &mut self.roster
    }

    /// The room and the sections as the page reads them.
    pub fn summary(&self) -> ForwardingSummary {
        let named: BTreeMap<u32, (String, bool)> = self.roster.sources();
        ForwardingSummary {
            members: self.roster.members(),
            speaker: self.roster.speaker(),
            streams: self
                .slots
                .iter()
                .zip(&self.mids)
                .filter_map(|(slot, mid)| {
                    let slot = (*slot)?;
                    let (participant, speaker) = match named.get(&slot.ssrc) {
                        Some((who, speaker)) => (Some(who.clone()), *speaker),
                        None => (None, false),
                    };
                    Some(SectionStream {
                        mid: mid.clone(),
                        participant,
                        speaker,
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::conference_info::parse;

    fn roster(version: u32, users: &[(&str, u32)]) -> String {
        let users: String = users
            .iter()
            .map(|(id, ssrc)| {
                format!(
                    "<user entity=\"sip:{id}@x\" state=\"full\"><display-text>{id}</display-text>\
                     <endpoint><media id=\"video\"><type>video</type><src-id>{ssrc}</src-id>\
                     <status>sendrecv</status></media></endpoint></user>"
                )
            })
            .collect();
        format!(
            "<conference-info xmlns=\"urn:ietf:params:xml:ns:conference-info\" state=\"full\" \
             version=\"{version}\"><users>{users}</users></conference-info>"
        )
    }

    #[test]
    fn each_stream_takes_a_free_section_and_gives_it_back_when_it_stops() {
        let t0 = Instant::now();
        let mids = vec!["3".into(), "4".into(), "5".into()];
        // The browser rejected the second section.
        let mut f = Forwarding::new(mids, vec![true, false, true]);
        assert_eq!(f.slot_for(0xA, t0), Some(0));
        assert_eq!(f.slot_for(0xB, t0), Some(2), "the rejected one is skipped");
        assert_eq!(f.slot_for(0xA, t0), Some(0), "a stream keeps its section");
        assert_eq!(f.slot_for(0xC, t0), None, "every section is taken");

        // Named by the roster, A on 3 is Ann's and B on 5 Bob's.
        f.roster_mut()
            .apply(parse(&roster(1, &[("ann", 0xA), ("bob", 0xB)])).unwrap());
        let s = f.summary();
        assert_eq!(s.members.len(), 2);
        assert_eq!(
            s.streams,
            [
                SectionStream {
                    mid: "3".into(),
                    participant: Some("ann".into()),
                    speaker: false
                },
                SectionStream {
                    mid: "5".into(),
                    participant: Some("bob".into()),
                    speaker: false
                },
            ]
        );

        // B goes quiet: still its section until it has been gone a while.
        let later = t0 + Duration::from_secs(1);
        assert_eq!(f.slot_for(0xA, later), Some(0));
        assert_eq!(f.slot_for(0xC, later), None, "B is still in the roster");
        // Bob leaves the roster: his quiet section is C's.
        f.roster_mut()
            .apply(parse(&roster(2, &[("ann", 0xA)])).unwrap());
        assert_eq!(f.slot_for(0xC, later), Some(2));
        assert_eq!(f.ssrc_on(2), Some(0xC));

        // Everything stops: every section free.
        f.expire(later + IDLE);
        assert_eq!((f.ssrc_on(0), f.ssrc_on(2)), (None, None));
        assert!(f.summary().streams.is_empty());
    }
}
