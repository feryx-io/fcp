//! A room's roster as FCP's conference server writes it (RFC 4575
//! `application/conference-info+xml`, with FCP's extensions): the parser,
//! and a roster kept from a full document and the partial ones after it.
//! What a forwarding room sends a subscriber on is each member's
//! `<src-id>`: their own stream, and the speaker stream while it shows them
//! (`docs/FORWARDING_ROOMS.md` §2.5). The WebRTC gateway reads it for the
//! browsers it carries.

use std::collections::BTreeMap;

use quick_xml::events::Event as Xml;
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;

/// The roster's MIME type.
pub const CONTENT_TYPE: &str = "application/conference-info+xml";

/// How a member's audio reaches the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Audio {
    /// Heard.
    Sending,
    /// Muted, by themselves or a host.
    Muted,
    /// On hold.
    Held,
}

/// One participant as the room lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    /// The room's id for them (`p-…`): what a view names them by.
    pub id: String,
    pub name: String,
    pub host: bool,
    pub audio: Audio,
    /// Their camera as the room has it (`on`, `off`, `lost`, …), when the
    /// room has video.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    pub speaking: bool,
    /// They share their screen.
    pub presenting: bool,
    /// In a room that forwards: their stream to this subscriber is held
    /// back (the network is short, or the view left no room for it).
    pub paused: bool,
    /// A host has put the spotlight on them.
    pub spotlight: bool,
}

/// The SSRCs a forwarding room sends a member to this subscriber on:
/// their own stream, and the speaker stream while it shows them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sources {
    pub camera: Option<u32>,
    pub speaker: Option<u32>,
}

/// What a roster document says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterDoc {
    pub full: bool,
    pub version: u32,
    /// Each user listed: in full, or `None` when they left.
    pub users: Vec<(String, Option<(Member, Sources)>)>,
    /// A webinar's audience, counted rather than listed
    /// (`docs/WEBINARS.md` §2.4): `fcp:audience` in the conference state,
    /// when the document carries it.
    pub audience: Option<usize>,
}

/// What applying a document did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The roster changed (or was given whole).
    Changed,
    /// An old or repeated document: nothing to tell.
    Stale,
    /// Documents were missed: ask for the whole roster again.
    Gap,
}

/// A room's roster as a subscriber has been told it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roster {
    members: BTreeMap<String, Member>,
    sources: BTreeMap<String, Sources>,
    version: Option<u32>,
    speaker: Option<String>,
    audience: usize,
}

impl Roster {
    /// Take a document: a full one replaces what was known, a partial one
    /// numbered next changes it; anything else is stale or a gap.
    pub fn apply(&mut self, doc: RosterDoc) -> Applied {
        match (doc.full, self.version) {
            (_, Some(v)) if doc.version <= v => return Applied::Stale,
            (false, None) => return Applied::Gap,
            (false, Some(v)) if doc.version != v + 1 => return Applied::Gap,
            (true, _) => {
                self.members.clear();
                self.sources.clear();
                self.audience = 0;
            }
            (false, _) => {}
        }
        self.version = Some(doc.version);
        if let Some(n) = doc.audience {
            self.audience = n;
        }
        for (id, member) in doc.users {
            match member {
                Some((m, src)) => {
                    self.members.insert(id.clone(), m);
                    self.sources.insert(id, src);
                }
                None => {
                    self.members.remove(&id);
                    self.sources.remove(&id);
                }
            }
        }
        self.speaker = self.choose_speaker();
        Applied::Changed
    }

    /// Who speaks: the one already chosen while they still speak, else the
    /// first speaking; with nobody speaking, the last speaker while they
    /// are still here.
    fn choose_speaker(&self) -> Option<String> {
        let speaking = |id: &String| self.members.get(id).is_some_and(|m| m.speaking);
        if let Some(current) = self.speaker.as_ref().filter(|id| speaking(id)) {
            return Some(current.clone());
        }
        self.members
            .values()
            .find(|m| m.speaking)
            .map(|m| m.id.clone())
            .or_else(|| {
                self.speaker
                    .clone()
                    .filter(|id| self.members.contains_key(id))
            })
    }

    /// Everyone, by id.
    pub fn members(&self) -> Vec<Member> {
        self.members.values().cloned().collect()
    }

    /// How many of a webinar's audience are in the room, never listed.
    pub fn audience(&self) -> usize {
        self.audience
    }

    /// Who speaks, or last spoke, by id.
    pub fn speaker(&self) -> Option<String> {
        self.speaker.clone()
    }

    /// Every SSRC the roster names: whose, and whether it is the speaker
    /// stream.
    pub fn sources(&self) -> BTreeMap<u32, (String, bool)> {
        let mut out = BTreeMap::new();
        for (id, s) in &self.sources {
            if let Some(ssrc) = s.camera {
                out.insert(ssrc, (id.clone(), false));
            }
        }
        // The speaker stream after: an SSRC is one or the other.
        for (id, s) in &self.sources {
            if let Some(ssrc) = s.speaker {
                out.insert(ssrc, (id.clone(), true));
            }
        }
        out
    }

    /// The version last taken.
    pub fn version(&self) -> Option<u32> {
        self.version
    }

    /// Forget what was told: the next document must be whole.
    pub fn reset(&mut self) {
        self.version = None;
    }
}

/// The participant id in a roster entity, `sip:<id>@<domain>`.
fn entity_id(entity: &str) -> String {
    let rest = entity
        .strip_prefix("sips:")
        .or_else(|| entity.strip_prefix("sip:"))
        .unwrap_or(entity);
    rest.split('@').next().unwrap_or(rest).to_string()
}

/// A user being read.
#[derive(Default)]
struct UserBuild {
    id: String,
    deleted: bool,
    name: String,
    host: bool,
    audio: Option<Audio>,
    camera: Option<String>,
    speaking: bool,
    presenting: bool,
    spotlight: bool,
    paused: bool,
    sources: Sources,
    /// The media element being read.
    media: Option<MediaBuild>,
}

/// A `<media>` being read.
#[derive(Default)]
struct MediaBuild {
    id: String,
    kind: String,
    status: String,
    label: String,
    src: Option<u32>,
    paused: bool,
}

impl UserBuild {
    fn finish(self) -> (String, Option<(Member, Sources)>) {
        if self.deleted {
            return (self.id, None);
        }
        let member = Member {
            id: self.id.clone(),
            name: self.name,
            host: self.host,
            audio: self.audio.unwrap_or(Audio::Sending),
            camera: self.camera,
            speaking: self.speaking,
            presenting: self.presenting,
            paused: self.paused,
            spotlight: self.spotlight,
        };
        (self.id, Some((member, self.sources)))
    }
}

/// An RFC 4575 document, as FCP's rooms write it: `None` when it is not
/// one.
pub fn parse(xml: &str) -> Option<RosterDoc> {
    let mut reader = Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut doc: Option<RosterDoc> = None;
    let mut user: Option<UserBuild> = None;
    let mut text = String::new();
    loop {
        let event = reader.read_event().ok()?;
        match event {
            Xml::Start(ref e) | Xml::Empty(ref e) => {
                let empty = matches!(event, Xml::Empty(_));
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let attr = |key: &str| -> Option<String> {
                    e.try_get_attribute(key)
                        .ok()
                        .flatten()
                        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                        .map(|v| v.into_owned())
                };
                match name.as_str() {
                    "conference-info" if path.is_empty() => {
                        doc = Some(RosterDoc {
                            full: attr("state").as_deref() != Some("partial"),
                            version: attr("version")?.trim().parse().ok()?,
                            users: Vec::new(),
                            audience: None,
                        });
                    }
                    "user" if path.last().map(String::as_str) == Some("users") => {
                        user = Some(UserBuild {
                            id: entity_id(&attr("entity")?),
                            deleted: attr("state").as_deref() == Some("deleted"),
                            ..Default::default()
                        });
                    }
                    "media" => {
                        if let Some(u) = user.as_mut() {
                            u.media = Some(MediaBuild {
                                id: attr("id").unwrap_or_default(),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {}
                }
                text.clear();
                if empty {
                    end(&name, &mut user, &mut doc, &mut path, &text);
                } else {
                    path.push(name);
                }
            }
            Xml::Text(t) => text.push_str(&t.xml10_content().ok()?),
            Xml::GeneralRef(r) => {
                let name = r.xml10_content().ok()?;
                match r.resolve_char_ref().ok().flatten() {
                    Some(c) => text.push(c),
                    None => text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    }),
                }
            }
            Xml::CData(c) => text.push_str(&String::from_utf8_lossy(c.as_ref())),
            Xml::End(_) => {
                let name = path.pop()?;
                end(&name, &mut user, &mut doc, &mut path, &text);
                text.clear();
            }
            Xml::Eof => break,
            _ => {}
        }
    }
    doc
}

/// An element closed, `path` holding its parents.
fn end(
    name: &str,
    user: &mut Option<UserBuild>,
    doc: &mut Option<RosterDoc>,
    path: &mut [String],
    text: &str,
) {
    let parent = path.last().map(String::as_str);
    let value = text.trim();
    if (name, parent) == ("audience", Some("conference-state")) {
        if let Some(d) = doc.as_mut() {
            d.audience = value.parse().ok();
        }
        return;
    }
    let Some(u) = user.as_mut() else {
        return;
    };
    match (name, parent) {
        ("user", _) => {
            if let (Some(u), Some(d)) = (user.take(), doc.as_mut()) {
                d.users.push(u.finish());
            }
        }
        ("display-text", Some("user")) => u.name = value.to_string(),
        ("entry", Some("roles")) => u.host |= value.eq_ignore_ascii_case("host"),
        ("type", Some("media")) => {
            if let Some(m) = u.media.as_mut() {
                m.kind = value.to_ascii_lowercase();
            }
        }
        ("status", Some("media")) => {
            if let Some(m) = u.media.as_mut() {
                m.status = value.to_ascii_lowercase();
            }
        }
        ("label", Some("media")) => {
            if let Some(m) = u.media.as_mut() {
                m.label = value.to_string();
            }
        }
        ("src-id", Some("media")) => {
            if let Some(m) = u.media.as_mut() {
                m.src = value.parse().ok();
            }
        }
        ("paused", Some("media")) => {
            if let Some(m) = u.media.as_mut() {
                m.paused = value.eq_ignore_ascii_case("true");
            }
        }
        ("camera", Some("media")) => u.camera = Some(value.to_string()),
        ("media", _) => {
            if let Some(MediaBuild {
                id,
                kind,
                status,
                label,
                src,
                paused,
            }) = u.media.take()
            {
                let speaker = id == "speaker" || label == "speaker";
                match kind.as_str() {
                    "audio" => {
                        u.audio = Some(match status.as_str() {
                            "recvonly" => Audio::Muted,
                            "inactive" => Audio::Held,
                            _ => Audio::Sending,
                        })
                    }
                    // The stream that follows the speaker, while it
                    // shows them.
                    "video" if speaker => u.sources.speaker = src,
                    "video" if label != "content" => {
                        u.sources.camera = src;
                        u.paused = paused;
                        // The camera's state is its `fcp:camera`; a room
                        // that does not say it has the status.
                        if u.camera.is_none() {
                            let on = matches!(status.as_str(), "sendrecv" | "sendonly");
                            u.camera = Some(if on { "on" } else { "off" }.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        ("speaking", _) => u.speaking = value.eq_ignore_ascii_case("true"),
        ("presenting", _) => u.presenting = value.eq_ignore_ascii_case("true"),
        ("spotlight", _) => u.spotlight = value.eq_ignore_ascii_case("true"),
        _ => {}
    }
}
