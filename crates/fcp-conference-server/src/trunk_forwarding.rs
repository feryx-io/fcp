//! Forwarding across nodes (`docs/FORWARDING_ROOMS.md` §2.8, R7).
//!
//! A room hosted on several nodes whose meeting forwards (`video_mode =
//! auto`) trunks each person's own stream rather than each node's
//! composite. Each node is a forwarding viewer of the other over the
//! trunk's one video section: it is sent the peer's own people — never a
//! third node's — each under an SSRC of their own, and it decides which of
//! them, and at what height, by what its own viewers and composites want.
//!
//! The section carries packets; who is behind each SSRC, who is in the
//! room on the peer, and what the peer wants of this node travel beside it
//! as an [`Announcement`], posted by each node to each peer it forwards
//! with on the cluster's control channel (`/internal/conferences/{room}/
//! forwarding`) at the speaker announcer's interval. Each announcement is
//! the whole picture, so one that is lost is made good by the next.

use forge_conference::video::forward::MAX_STREAMS;
use forge_conference::video::{StreamTarget, StreamWant, ViewRequest};
use serde::{Deserialize, Serialize};

/// What one node tells a peer it forwards with, about one room.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// The node speaking.
    pub node: String,
    /// Its own people in the room, all of them: one missing has left.
    #[serde(default)]
    pub members: Vec<PeerMember>,
    /// Whose each SSRC it sends the peer over the trunk is.
    #[serde(default)]
    pub streams: Vec<StreamName>,
    /// Which of the peer's people it wants, and how tall.
    #[serde(default)]
    pub wants: Vec<Want>,
}

/// One of a node's people, as its peers list them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerMember {
    pub participant_id: String,
    pub display_name: String,
    #[serde(default)]
    pub host: bool,
    /// `sending`, `muted` or `held`.
    #[serde(default = "sending")]
    pub audio: String,
    /// Their camera is sending in the trunk's codec, so it can cross.
    #[serde(default)]
    pub camera: bool,
    /// When they joined, in milliseconds since the Unix epoch.
    #[serde(default)]
    pub joined_at_ms: u64,
}

fn sending() -> String {
    "sending".to_string()
}

/// An SSRC on the trunk and whose it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamName {
    pub ssrc: u32,
    pub participant_id: String,
}

/// One of the peer's people wanted here, no taller than this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Want {
    pub participant_id: String,
    pub max_height: u32,
}

/// What the peer's wants make the trunk's view: each person asked for by
/// name, so each keeps one SSRC while they are wanted. At most
/// [`MAX_STREAMS`] — more than a room with video holds.
pub fn view_of(wants: &[Want]) -> ViewRequest {
    ViewRequest {
        streams: Some(
            wants
                .iter()
                .take(MAX_STREAMS)
                .map(|w| StreamWant {
                    target: StreamTarget::Participant(w.participant_id.clone()),
                    max_height: Some(w.max_height),
                    priority: None,
                })
                .collect(),
        ),
        max_streams: Some(MAX_STREAMS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announcement_reads_back_and_fills_in_what_it_leaves_out() {
        let a = Announcement {
            node: "conf-a".into(),
            members: vec![PeerMember {
                participant_id: "p1".into(),
                display_name: "Alice".into(),
                host: true,
                audio: "muted".into(),
                camera: true,
                joined_at_ms: 7,
            }],
            streams: vec![StreamName {
                ssrc: 42,
                participant_id: "p1".into(),
            }],
            wants: vec![Want {
                participant_id: "p9".into(),
                max_height: 360,
            }],
        };
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(serde_json::from_value::<Announcement>(json).unwrap(), a);
        let bare: Announcement = serde_json::from_value(serde_json::json!({
            "node": "conf-b",
            "members": [{"participant_id": "p2", "display_name": "Bob"}],
        }))
        .unwrap();
        assert_eq!(bare.members[0].audio, "sending");
        assert!(!bare.members[0].camera);
        assert!(bare.streams.is_empty() && bare.wants.is_empty());
    }

    #[test]
    fn wants_become_a_view_that_names_each_person() {
        let v = view_of(&[
            Want {
                participant_id: "a".into(),
                max_height: 720,
            },
            Want {
                participant_id: "b".into(),
                max_height: 180,
            },
        ]);
        let streams = v.streams.unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].target, StreamTarget::Participant("a".into()));
        assert_eq!(streams[0].max_height, Some(720));
        assert_eq!(streams[1].max_height, Some(180));
        // Nothing wanted is nothing sent, not the default view.
        assert_eq!(view_of(&[]).streams, Some(Vec::new()));
    }
}
