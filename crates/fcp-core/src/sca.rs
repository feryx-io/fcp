//! Shared call appearances (`docs/ENTERPRISE_CALLING.md` §3.9).
//!
//! A shared line is a user's number whose line binding says `shared`:
//! every device bound to it registers the number as an account of its own
//! and the devices share the line's appearances — at most
//! `max_appearances` calls, each at an index the phones show on a line
//! key. The flavour is BroadSoft's, which Yealink, Polycom, Grandstream
//! and Cisco desk phones implement: phones SUBSCRIBE to `call-info` on the
//! line and are sent `Call-Info` headers naming each appearance's state;
//! a phone about to call SUBSCRIBEs to `line-seize` for an index first.
//!
//! What lives here has no SIP in it: the states, one appearance as the
//! table keeps it, and reading and writing the `Call-Info` header.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The `Event` package phones watch a shared line's appearances with.
pub const CALL_INFO_EVENT: &str = "call-info";

/// The `Event` package a phone claims an appearance with before it calls.
pub const LINE_SEIZE_EVENT: &str = "line-seize";

/// How long a seized appearance stands without an INVITE, whatever the
/// phone asked for: long enough to dial, short enough that a phone that
/// went away frees the key.
pub const MAX_SEIZE_SECS: u64 = 30;

/// An appearance's state, as a phone's line key shows it. An idle
/// appearance is one the table does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceState {
    /// A phone has claimed the index to place a call from it.
    Seized,
    /// A call from the line is being placed.
    Progressing,
    /// A call to the line is ringing its phones.
    Alerting,
    /// A call on the line is up.
    Active,
    /// The line's phone put the far party on hold: any phone on the line
    /// may pick it up.
    Held,
}

impl AppearanceState {
    /// The `appearance-state` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Seized => "seized",
            Self::Progressing => "progressing",
            Self::Alerting => "alerting",
            Self::Active => "active",
            Self::Held => "held",
        }
    }
}

impl fmt::Display for AppearanceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One appearance of a shared line, as the table keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appearance {
    /// The index, from 1.
    pub index: u8,
    pub state: AppearanceState,
    /// Who holds it: `call:<call-id>` for a call, `seize:<SIP Call-ID>`
    /// for a phone's `line-seize` subscription. Only its holder moves it
    /// on or frees it.
    pub owner: String,
    /// The node the call is on.
    pub node: String,
    /// Where that node takes SIP: a retrieve that reaches another node is
    /// sent here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_uri: Option<String>,
    /// What a retrieve forwarded from another node carries to prove it
    /// read this table (`X-FCP-Node-Handoff`); never sent to a phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
    /// The phone on it: the seizing phone's Contact, then the line's leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// The far party, for the lamp's `appearance-uri`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// When it lapses unless refreshed (unix seconds): a seized index
    /// soon, a call's long after its node stopped refreshing it.
    pub expires_at: u64,
}

impl Appearance {
    /// Whether the entry has lapsed at `now` (unix seconds).
    pub fn lapsed(&self, now: u64) -> bool {
        self.expires_at <= now
    }

    /// The owner string of a call's appearance.
    pub fn call_owner(call_id: &str) -> String {
        format!("call:{call_id}")
    }

    /// The owner string of a phone's seize.
    pub fn seize_owner(sip_call_id: &str) -> String {
        format!("seize:{sip_call_id}")
    }

    /// The call this appearance is, if it is one.
    pub fn call_id(&self) -> Option<&str> {
        self.owner.strip_prefix("call:")
    }
}

/// The index a request's `Call-Info` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceIndex {
    /// `appearance-index=*`: any.
    Any,
    Index(u8),
}

/// The `appearance-index` a `Call-Info` header value names, if any (the
/// first entry that names one).
pub fn requested_index(call_info: &str) -> Option<AppearanceIndex> {
    for entry in split_entries(call_info) {
        for param in entry.split(';').skip(1) {
            let Some((name, value)) = param.split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("appearance-index") {
                let value = value.trim().trim_matches('"');
                if value == "*" {
                    return Some(AppearanceIndex::Any);
                }
                return value.parse().ok().map(AppearanceIndex::Index);
            }
        }
    }
    None
}

/// The `appearance-index` and `appearance-state` of each entry of a
/// `Call-Info` header value (`*` as `None`), for a phone reading its
/// lamps.
pub fn appearance_states(call_info: &str) -> Vec<(Option<u8>, String)> {
    let mut out = Vec::new();
    for entry in split_entries(call_info) {
        let mut index = None;
        let mut any = false;
        let mut state = None;
        for param in entry.split(';').skip(1) {
            let Some((name, value)) = param.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match name.trim().to_ascii_lowercase().as_str() {
                "appearance-index" if value == "*" => any = true,
                "appearance-index" => index = value.parse().ok(),
                "appearance-state" => state = Some(value.to_ascii_lowercase()),
                _ => {}
            }
        }
        if let Some(state) = state {
            if index.is_some() || any {
                out.push((index, state));
            }
        }
    }
    out
}

/// The `Call-Info` header value telling a phone every appearance of a
/// line: each held one with its state, then every other index idle, as
/// BroadSoft phones expect. `host` is the server's address, the URI the
/// entries are about.
pub fn call_info_value(host: &str, appearances: &[Appearance]) -> String {
    let mut sorted: Vec<&Appearance> = appearances.iter().collect();
    sorted.sort_by_key(|a| a.index);
    let mut entries: Vec<String> = sorted
        .into_iter()
        .map(|a| {
            let mut entry = format!(
                "<sip:{host}>;appearance-index={};appearance-state={}",
                a.index, a.state
            );
            if let Some(remote) = a
                .remote
                .as_deref()
                .filter(|_| a.state != AppearanceState::Seized)
            {
                entry.push_str(&format!(
                    ";appearance-uri=\"{}\"",
                    remote.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
            entry
        })
        .collect();
    entries.push(format!(
        "<sip:{host}>;appearance-index=*;appearance-state=idle"
    ));
    entries.join(",")
}

/// The `Call-Info` header value of a `line-seize` NOTIFY: the one index,
/// seized.
pub fn seized_value(host: &str, index: u8) -> String {
    format!("<sip:{host}>;appearance-index={index};appearance-state=seized")
}

/// The entries of a `Call-Info` value: commas outside `<…>` and quotes.
fn split_entries(value: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    for (i, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => depth += 1,
            '>' if !quoted => depth -= 1,
            ',' if !quoted && depth <= 0 => {
                entries.push(value[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = value[start..].trim();
    if !last.is_empty() {
        entries.push(last);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance(index: u8, state: AppearanceState, remote: Option<&str>) -> Appearance {
        Appearance {
            index,
            state,
            owner: Appearance::call_owner("call-1"),
            node: "node-a".into(),
            node_uri: None,
            handoff: None,
            contact: None,
            remote: remote.map(str::to_string),
            expires_at: 100,
        }
    }

    #[test]
    fn a_request_names_an_index_or_any() {
        assert_eq!(
            requested_index("<sip:10.0.0.1>;appearance-index=2"),
            Some(AppearanceIndex::Index(2))
        );
        assert_eq!(
            requested_index("<sip:10.0.0.1>;appearance-index=*"),
            Some(AppearanceIndex::Any)
        );
        assert_eq!(
            requested_index("<sip:10.0.0.1>;purpose=icon"),
            None,
            "a Call-Info that is not about appearances names none"
        );
    }

    #[test]
    fn the_lamps_list_every_held_index_then_the_rest_idle() {
        let value = call_info_value(
            "10.0.0.1",
            &[
                appearance(2, AppearanceState::Held, Some("\"Bob\" <sip:bob@x>")),
                appearance(1, AppearanceState::Active, Some("<sip:+15550001@x>")),
            ],
        );
        assert_eq!(
            value,
            "<sip:10.0.0.1>;appearance-index=1;appearance-state=active;appearance-uri=\"<sip:+15550001@x>\",\
             <sip:10.0.0.1>;appearance-index=2;appearance-state=held;appearance-uri=\"\\\"Bob\\\" <sip:bob@x>\",\
             <sip:10.0.0.1>;appearance-index=*;appearance-state=idle"
        );
        assert_eq!(
            appearance_states(&value),
            vec![
                (Some(1), "active".to_string()),
                (Some(2), "held".to_string()),
                (None, "idle".to_string()),
            ],
            "a phone reads back what the server wrote, quoted commas and all"
        );
    }

    #[test]
    fn an_idle_line_is_one_entry() {
        assert_eq!(
            call_info_value("h", &[]),
            "<sip:h>;appearance-index=*;appearance-state=idle"
        );
    }

    #[test]
    fn a_seize_names_no_far_party() {
        let value = call_info_value("h", &[appearance(3, AppearanceState::Seized, Some("x"))]);
        assert!(!value.contains("appearance-uri"), "{value}");
        assert_eq!(
            seized_value("h", 3),
            "<sip:h>;appearance-index=3;appearance-state=seized"
        );
    }

    #[test]
    fn owners_say_what_holds_the_index() {
        let a = appearance(1, AppearanceState::Active, None);
        assert_eq!(a.call_id(), Some("call-1"));
        assert!(a.lapsed(100) && !a.lapsed(99));
        assert_eq!(Appearance::seize_owner("abc"), "seize:abc");
    }
}
