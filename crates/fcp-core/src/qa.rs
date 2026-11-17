//! The body of an `fcp-qa` INFO (`docs/WEBINARS.md` §2.5): the audience's
//! voice in a room — a raised hand, and being brought to the stage.
//!
//! A participant sends the room its hand:
//!
//! ```json
//! { "op": "hand", "raised": true }
//! ```
//!
//! and a participant whose INVITE said `Recv-Info: fcp-qa` is told what
//! the room did to them:
//!
//! ```json
//! { "op": "hand", "raised": false }
//! { "op": "stage", "on": true }
//! ```
//!
//! A hand comes down when its owner lowers it, a host lowers it (or all
//! of them), or a host brings its owner to the stage. Being brought to the
//! stage means being heard by the room; being sent back means listening
//! again.

use serde::{Deserialize, Serialize};

/// One `fcp-qa` message, either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum QaMessage {
    /// A hand raised or lowered: by its owner toward the room, or by the
    /// room toward its owner.
    Hand { raised: bool },
    /// The room brought the participant to the stage (`on`) or sent them
    /// back to the audience. Only the room sends it.
    Stage { on: bool },
}

impl QaMessage {
    /// Read an INFO's body. What it means and who may send it is the
    /// room's to judge.
    pub fn parse(body: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(body).map_err(|e| format!("not an fcp-qa message: {e}"))
    }

    /// The body to send.
    pub fn to_body(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_read_as_they_are_written() {
        for (body, msg) in [
            (
                r#"{"op":"hand","raised":true}"#,
                QaMessage::Hand { raised: true },
            ),
            (
                r#"{"op":"stage","on":false}"#,
                QaMessage::Stage { on: false },
            ),
        ] {
            assert_eq!(QaMessage::parse(body.as_bytes()).unwrap(), msg);
            assert_eq!(msg.to_body(), body);
        }
        assert!(QaMessage::parse(br#"{"op":"hand"}"#).is_err());
        assert!(QaMessage::parse(br#"{"op":"wave","raised":true}"#).is_err());
        assert!(QaMessage::parse(br#"{"op":"hand","raised":true,"x":1}"#).is_err());
    }
}
