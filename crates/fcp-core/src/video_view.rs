//! The body of an `fcp-view` INFO (`docs/FORWARDING_ROOMS.md` §2.3): what a
//! participant of a forwarding room asks to be sent.
//!
//! ```json
//! { "streams": [
//!     { "participant": "p-1a2b", "max_height": 720, "priority": 1 },
//!     { "participant": "p-3c4d", "max_height": 180 },
//!     { "speaker": true, "max_height": 720, "priority": 1 }
//!   ],
//!   "max_streams": 9 }
//! ```
//!
//! A participant entry asks for that person at no more than that height; a
//! `speaker` entry is a stream that follows the active speaker. Lower
//! `priority` is kept first when bandwidth is short. Whoever is left out is
//! not sent. A view without `streams` is the default: the speaker and the
//! others who spoke last.

use serde::{Deserialize, Serialize};

/// The most streams a view may name.
pub const MAX_VIEW_STREAMS: usize = 25;

/// What a participant asks to be sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// `None`: the room's default view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<Vec<ViewStream>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_streams: Option<usize>,
}

/// One stream a view names: a participant, or the speaker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewStream {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub speaker: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

impl View {
    /// Read an INFO body, refusing one that names neither a participant
    /// nor the speaker in an entry, names both, or asks for too much.
    pub fn parse(body: &[u8]) -> Result<Self, String> {
        let view: View = serde_json::from_slice(body).map_err(|e| e.to_string())?;
        if let Some(streams) = &view.streams {
            if streams.len() > MAX_VIEW_STREAMS {
                return Err(format!("at most {MAX_VIEW_STREAMS} streams"));
            }
            for s in streams {
                match (&s.participant, s.speaker) {
                    (Some(_), false) | (None, true) => {}
                    _ => {
                        return Err(
                            "each stream names a participant or the speaker, not both".into()
                        )
                    }
                }
            }
        }
        if view.max_streams == Some(0) || view.max_streams.is_some_and(|m| m > MAX_VIEW_STREAMS) {
            return Err(format!("max_streams is 1 to {MAX_VIEW_STREAMS}"));
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_designs_example_reads_back() {
        let view = View::parse(
            br#"{ "streams": [
                { "participant": "p-1a2b", "max_height": 720, "priority": 1 },
                { "participant": "p-3c4d", "max_height": 180 },
                { "speaker": true, "max_height": 720, "priority": 1 }
              ], "max_streams": 9 }"#,
        )
        .unwrap();
        let streams = view.streams.as_ref().unwrap();
        assert_eq!(streams.len(), 3);
        assert_eq!(streams[0].participant.as_deref(), Some("p-1a2b"));
        assert_eq!(streams[1].priority, None);
        assert!(streams[2].speaker);
        assert_eq!(view.max_streams, Some(9));
        let back = View::parse(serde_json::to_string(&view).unwrap().as_bytes()).unwrap();
        assert_eq!(back, view);
        assert_eq!(View::parse(b"{}").unwrap(), View::default());
    }

    #[test]
    fn a_view_that_cannot_mean_anything_is_refused() {
        for bad in [
            &br#"{"streams":[{}]}"#[..],
            br#"{"streams":[{"participant":"a","speaker":true}]}"#,
            br#"{"max_streams":0}"#,
            br#"{"streams":[],"colour":"red"}"#,
            b"not json",
        ] {
            assert!(
                View::parse(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
