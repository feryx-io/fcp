//! A room's roster as RFC 4575 tells it (`docs/SOFTPHONE_VIDEO.md` §2.6):
//! who is in the room, by name, with their audio and video, whether they
//! are muted, speaking or presenting. A subscriber is sent the whole roster
//! first (`state="full"`), then only what changed (`state="partial"`), each
//! document numbered one above the last.
//!
//! What RFC 4575 has no element for — speaking, presenting, the camera's
//! state — is in FCP's own namespace ([`FCP_NS`]), which a reader that does
//! not know it skips (RFC 4575 §5).
//!
//! A participant sent each person's own stream (`docs/FORWARDING_ROOMS.md`
//! §2.2) is told whose each SSRC is: a member's `<media id="video">` carries
//! the `<src-id>` that subscriber receives them on, `fcp:paused` when the
//! stream is held back, and the member the speaker stream shows has a
//! `<media id="speaker">` with that stream's `<src-id>`. Every subscriber's
//! roster is its own, since NOTIFYs are sent per subscription.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::SystemTime;

/// FCP's elements in a conference-info document.
pub(crate) const FCP_NS: &str = "urn:fcp:params:xml:ns:conference-info";

/// The most users one document lists; a larger room's roster stops there.
const MAX_USERS: usize = 1000;

/// How a participant's audio reaches the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Audio {
    /// Heard.
    Sending,
    /// Muted (by themselves or a host): they hear the room, it does not
    /// hear them.
    Muted,
    /// On hold: neither way.
    Held,
}

impl Audio {
    /// RFC 4575 media `status`, from the room's side.
    fn status(self) -> &'static str {
        match self {
            Audio::Sending => "sendrecv",
            Audio::Muted => "recvonly",
            Audio::Held => "inactive",
        }
    }
}

/// One participant as the roster shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    pub name: String,
    pub host: bool,
    pub audio: Audio,
    /// Their camera as the room has it (`on`, `off`, `lost`, `disabled`,
    /// `failed`), when the room has video.
    pub video: Option<&'static str>,
    pub speaking: bool,
    pub presenting: bool,
    /// A host's spotlight is on them: a client drawing its own layout
    /// honours it as the composite does.
    pub spotlight: bool,
    /// The stream the subscriber receives them on, when the subscriber
    /// forwards.
    pub stream: Option<StreamOf>,
    /// The subscriber's speaker stream shows them, under this SSRC.
    pub speaker_stream: Option<u32>,
    pub joined_at: SystemTime,
}

/// A member's stream, as one forwarding subscriber receives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamOf {
    pub ssrc: u32,
    pub paused: bool,
}

/// A room's roster: its members by participant id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Roster {
    pub members: BTreeMap<String, Member>,
}

/// One of a forwarding subscriber's streams, for [`Roster::for_viewer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewerStream {
    pub ssrc: u32,
    /// The participant the stream was asked for; `None` for the speaker
    /// stream.
    pub asked_for: Option<String>,
    /// Whom it shows now.
    pub showing: Option<String>,
    pub paused: bool,
}

impl Roster {
    /// This roster as a forwarding subscriber is told it: each member with
    /// the stream the subscriber receives them on.
    pub(crate) fn for_viewer(&self, streams: &[ViewerStream]) -> Roster {
        let mut out = self.clone();
        for s in streams {
            match &s.asked_for {
                Some(id) => {
                    if let Some(m) = out.members.get_mut(id) {
                        m.stream = Some(StreamOf {
                            ssrc: s.ssrc,
                            paused: s.paused,
                        });
                    }
                }
                None if !s.paused => {
                    if let Some(m) = s.showing.as_ref().and_then(|id| out.members.get_mut(id)) {
                        m.speaker_stream = Some(s.ssrc);
                    }
                }
                None => {}
            }
        }
        out
    }
}

/// Where a roster's entities live: `sip:<id>@<domain>`.
#[derive(Debug, Clone)]
pub(crate) struct Names<'a> {
    pub room: &'a str,
    pub domain: &'a str,
}

impl Names<'_> {
    fn room(&self) -> String {
        format!("sip:{}@{}", escape(self.room), escape(self.domain))
    }

    fn user(&self, id: &str) -> String {
        format!("sip:{}@{}", escape(id), escape(self.domain))
    }
}

/// The whole roster, numbered `version`.
pub(crate) fn full(names: &Names, version: u32, roster: &Roster) -> String {
    let mut users = String::new();
    for (id, m) in roster.members.iter().take(MAX_USERS) {
        user(&mut users, names, id, Some(m));
    }
    document(names, version, "full", roster, &users)
}

/// What changed from `before` to `now`, numbered `version`; `None` when
/// nothing did.
pub(crate) fn partial(
    names: &Names,
    version: u32,
    before: &Roster,
    now: &Roster,
) -> Option<String> {
    let mut users = String::new();
    for (id, m) in now.members.iter().take(MAX_USERS) {
        if before.members.get(id) != Some(m) {
            user(&mut users, names, id, Some(m));
        }
    }
    for id in before.members.keys() {
        if !now.members.contains_key(id) {
            user(&mut users, names, id, None);
        }
    }
    if users.is_empty() {
        return None;
    }
    Some(document(names, version, "partial", now, &users))
}

/// The room is over: everyone gone, numbered `version`.
pub(crate) fn ended(names: &Names, version: u32) -> String {
    document(names, version, "full", &Roster::default(), "")
}

fn document(names: &Names, version: u32, state: &str, roster: &Roster, users: &str) -> String {
    let count = roster.members.len();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <conference-info xmlns=\"urn:ietf:params:xml:ns:conference-info\" \
         xmlns:fcp=\"{FCP_NS}\" entity=\"{room}\" state=\"{state}\" version=\"{version}\">\n  \
         <conference-description>\n    <display-text>Conference {name}</display-text>\n  \
         </conference-description>\n  \
         <conference-state>\n    <user-count>{count}</user-count>\n    \
         <active>{active}</active>\n  </conference-state>\n  \
         <users>\n{users}  </users>\n\
         </conference-info>\n",
        room = names.room(),
        name = escape(names.room),
        active = count > 0,
    )
}

/// A `<user>`: in full, or (`None`) deleted.
fn user(out: &mut String, names: &Names, id: &str, member: Option<&Member>) {
    let entity = names.user(id);
    let Some(m) = member else {
        let _ = writeln!(out, "    <user entity=\"{entity}\" state=\"deleted\"/>");
        return;
    };
    let _ = writeln!(out, "    <user entity=\"{entity}\" state=\"full\">");
    let _ = writeln!(
        out,
        "      <display-text>{}</display-text>",
        escape(&m.name)
    );
    let _ = writeln!(
        out,
        "      <roles><entry>{}</entry></roles>",
        if m.host { "host" } else { "participant" }
    );
    let _ = writeln!(out, "      <endpoint entity=\"{entity}\" state=\"full\">");
    let _ = writeln!(out, "        <status>connected</status>");
    let when = chrono::DateTime::<chrono::Utc>::from(m.joined_at)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let _ = writeln!(
        out,
        "        <joining-method>dialed-in</joining-method>\n        <joining-info><when>{when}</when></joining-info>"
    );
    let _ = writeln!(
        out,
        "        <media id=\"audio\"><type>audio</type><status>{}</status></media>",
        m.audio.status()
    );
    if let Some(video) = m.video {
        // Sent to the room while the camera is on; the room's picture
        // comes down either way.
        let status = if video == "on" {
            "sendrecv"
        } else {
            "recvonly"
        };
        let src = m
            .stream
            .map(|s| format!("<src-id>{}</src-id>", s.ssrc))
            .unwrap_or_default();
        let paused = if m.stream.is_some_and(|s| s.paused) {
            "<fcp:paused>true</fcp:paused>"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "        <media id=\"video\"><type>video</type>{src}<status>{status}</status><fcp:camera>{video}</fcp:camera>{paused}</media>"
        );
    }
    if let Some(ssrc) = m.speaker_stream {
        let _ = writeln!(
            out,
            "        <media id=\"speaker\"><type>video</type><label>speaker</label><src-id>{ssrc}</src-id><status>sendonly</status></media>"
        );
    }
    if m.presenting {
        let _ = writeln!(
            out,
            "        <media id=\"content\"><type>video</type><label>content</label><status>sendonly</status></media>"
        );
    }
    let _ = writeln!(out, "        <fcp:speaking>{}</fcp:speaking>", m.speaking);
    let _ = writeln!(
        out,
        "        <fcp:presenting>{}</fcp:presenting>",
        m.presenting
    );
    if m.spotlight {
        let _ = writeln!(out, "        <fcp:spotlight>true</fcp:spotlight>");
    }
    let _ = writeln!(out, "      </endpoint>\n    </user>");
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    const NAMES: Names = Names {
        room: "3000",
        domain: "conf.example.com",
    };

    fn member(name: &str) -> Member {
        Member {
            name: name.into(),
            host: false,
            audio: Audio::Sending,
            video: Some("on"),
            speaking: false,
            presenting: false,
            spotlight: false,
            stream: None,
            speaker_stream: None,
            joined_at: UNIX_EPOCH + Duration::from_secs(1_794_000_000),
        }
    }

    fn roster(members: &[(&str, Member)]) -> Roster {
        Roster {
            members: members
                .iter()
                .map(|(id, m)| (id.to_string(), m.clone()))
                .collect(),
        }
    }

    #[test]
    fn the_whole_roster_names_everyone_with_their_media() {
        let mut ann = member("Ann <Lee> & Co");
        ann.host = true;
        let mut bob = member("Bob");
        bob.audio = Audio::Muted;
        bob.video = Some("off");
        let doc = full(&NAMES, 1, &roster(&[("p1", ann), ("p2", bob)]));
        assert!(
            doc.contains("entity=\"sip:3000@conf.example.com\" state=\"full\" version=\"1\""),
            "{doc}"
        );
        assert!(doc.contains("<user-count>2</user-count>"), "{doc}");
        assert!(
            doc.contains("<display-text>Ann &lt;Lee&gt; &amp; Co</display-text>"),
            "{doc}"
        );
        assert!(doc.contains("<roles><entry>host</entry></roles>"), "{doc}");
        assert!(doc.contains("<when>2026-11-06T21:20:00Z"), "{doc}");
        // Bob: muted, his camera off, still seeing the room.
        let bob = &doc[doc.find("sip:p2@").unwrap()..];
        assert!(
            bob.contains("<type>audio</type><status>recvonly</status>"),
            "{bob}"
        );
        assert!(
            bob.contains("<status>recvonly</status><fcp:camera>off</fcp:camera>"),
            "{bob}"
        );
        assert!(doc.contains(&format!("xmlns:fcp=\"{FCP_NS}\"")));
    }

    #[test]
    fn an_update_carries_only_who_changed_or_left() {
        let before = roster(&[("p1", member("Ann")), ("p2", member("Bob"))]);
        let mut speaking = member("Ann");
        speaking.speaking = true;
        let mut presenting = member("Cy");
        presenting.presenting = true;
        let now = roster(&[("p1", speaking), ("p3", presenting)]);
        let doc = partial(&NAMES, 4, &before, &now).unwrap();
        assert!(doc.contains("state=\"partial\" version=\"4\""), "{doc}");
        assert!(
            doc.contains("<user entity=\"sip:p2@conf.example.com\" state=\"deleted\"/>"),
            "{doc}"
        );
        assert!(doc.contains("<fcp:speaking>true</fcp:speaking>"), "{doc}");
        assert!(doc.contains("<label>content</label>"), "{doc}");
        assert!(
            doc.contains("<fcp:presenting>true</fcp:presenting>"),
            "{doc}"
        );
        assert!(doc.contains("<user-count>2</user-count>"), "{doc}");
        assert_eq!(partial(&NAMES, 5, &now, &now), None, "nothing changed");
    }

    #[test]
    fn a_forwarding_subscriber_is_told_whose_each_stream_is() {
        let mut ann = member("Ann");
        ann.stream = Some(StreamOf {
            ssrc: 1111,
            paused: false,
        });
        ann.speaker_stream = Some(3333);
        ann.spotlight = true;
        let mut bob = member("Bob");
        bob.stream = Some(StreamOf {
            ssrc: 2222,
            paused: true,
        });
        let doc = full(&NAMES, 1, &roster(&[("p1", ann.clone()), ("p2", bob)]));
        let (ann_doc, bob_doc) = doc.split_at(doc.find("sip:p2@").unwrap());
        assert!(
            ann_doc.contains(
                "<media id=\"video\"><type>video</type><src-id>1111</src-id><status>sendrecv</status>"
            ),
            "{ann_doc}"
        );
        assert!(
            ann_doc.contains("<media id=\"speaker\"><type>video</type><label>speaker</label><src-id>3333</src-id>"),
            "{ann_doc}"
        );
        assert!(ann_doc.contains("<fcp:spotlight>true</fcp:spotlight>"));
        assert!(!ann_doc.contains("fcp:paused"));
        assert!(
            bob_doc.contains("<src-id>2222</src-id>")
                && bob_doc.contains("<fcp:paused>true</fcp:paused>"),
            "{bob_doc}"
        );
        assert!(!bob_doc.contains("speaker\""), "{bob_doc}");

        // The speaker stream moving on is an update to both.
        let mut moved = ann;
        moved.speaker_stream = None;
        let before = roster(&[("p1", member("x"))]);
        let now = roster(&[("p1", moved)]);
        let doc = partial(&NAMES, 2, &before, &now).unwrap();
        assert!(!doc.contains("speaker\""), "{doc}");
    }

    #[test]
    fn a_viewers_streams_are_laid_on_the_rooms_roster() {
        let room = roster(&[("p1", member("Ann")), ("p2", member("Bob"))]);
        let mine = room.for_viewer(&[
            ViewerStream {
                ssrc: 7,
                asked_for: None,
                showing: Some("p2".into()),
                paused: false,
            },
            ViewerStream {
                ssrc: 8,
                asked_for: Some("p1".into()),
                showing: Some("p1".into()),
                paused: true,
            },
            ViewerStream {
                ssrc: 9,
                asked_for: Some("gone".into()),
                showing: None,
                paused: true,
            },
        ]);
        assert_eq!(
            mine.members["p1"].stream,
            Some(StreamOf {
                ssrc: 8,
                paused: true
            })
        );
        assert_eq!(mine.members["p2"].speaker_stream, Some(7));
        assert_eq!(mine.members["p2"].stream, None, "not asked for by name");
        assert_eq!(mine.members.len(), 2);
        assert_eq!(
            room.members["p1"].stream, None,
            "the room's own is left alone"
        );
    }

    #[test]
    fn an_ended_room_lists_nobody() {
        let doc = ended(&NAMES, 7);
        assert!(doc.contains("state=\"full\" version=\"7\""), "{doc}");
        assert!(doc.contains("<active>false</active>"), "{doc}");
        assert!(!doc.contains("<user "), "{doc}");
    }
}
