//! A user's calling settings (`docs/ENTERPRISE_CALLING.md` §2, §3.1): the
//! one row per user that every path to the user reads — do not disturb,
//! call waiting, the four forwards and the no-answer time — and the typed
//! target every forward names. Later phases add their fields here.

use crate::{TenantId, UserId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

/// Where a call goes when it is forwarded, a schedule branches, or a
/// find-me step rings: one type, one executor. Serialised as a string
/// (`voicemail`, `number:+15551234567`, `ext:2001`, `user:<id>`,
/// `flow:<id>`, `queue:<name>`, `sip:…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    /// The user's own voicemail (the callee's, for a forward).
    Voicemail,
    /// An E.164 number, placed as any outbound call is.
    Number(String),
    /// An extension of the tenant.
    Extension(String),
    /// A user of the tenant, rung by their own plan.
    User(UserId),
    /// A flow run for the call.
    Flow(String),
    /// A queue the call joins.
    Queue(String),
    /// A SIP URI, as given.
    Sip(String),
}

impl CallTarget {
    /// The string form, as stored and as the API shows it.
    pub fn as_string(&self) -> String {
        self.to_string()
    }

    /// Parse a target: the typed forms, and leniently the bare forms a
    /// person types — `voicemail`, a `+` number, digits as an extension,
    /// a `sip:` URI.
    pub fn parse(s: &str) -> Result<Self, CallTargetError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(CallTargetError::Empty);
        }
        if s.eq_ignore_ascii_case("voicemail") {
            return Ok(Self::Voicemail);
        }
        if let Some(rest) = s.strip_prefix("number:") {
            return Self::number(rest);
        }
        if let Some(rest) = s.strip_prefix("ext:") {
            return Self::extension(rest);
        }
        if let Some(rest) = s.strip_prefix("user:") {
            return rest
                .parse::<UserId>()
                .map(Self::User)
                .map_err(|_| CallTargetError::Invalid(format!("not a user id: {rest}")));
        }
        if let Some(rest) = s.strip_prefix("flow:") {
            return if rest.is_empty() {
                Err(CallTargetError::Empty)
            } else {
                Ok(Self::Flow(rest.to_string()))
            };
        }
        if let Some(rest) = s.strip_prefix("queue:") {
            return if rest.is_empty() {
                Err(CallTargetError::Empty)
            } else {
                Ok(Self::Queue(rest.to_string()))
            };
        }
        if s.starts_with("sip:") || s.starts_with("sips:") {
            return Ok(Self::Sip(s.to_string()));
        }
        if let Some(digits) = s.strip_prefix('+') {
            return Self::number(digits).map(|_| Self::Number(s.to_string()));
        }
        Self::extension(s)
    }

    fn number(digits: &str) -> Result<Self, CallTargetError> {
        let digits = digits.trim_start_matches('+');
        if digits.is_empty() || digits.len() > 15 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CallTargetError::Invalid(format!(
                "not an E.164 number: {digits}"
            )));
        }
        Ok(Self::Number(format!("+{digits}")))
    }

    fn extension(s: &str) -> Result<Self, CallTargetError> {
        if s.is_empty() {
            return Err(CallTargetError::Empty);
        }
        if s.len() > 32
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'*' || b == b'#')
        {
            return Err(CallTargetError::Invalid(format!("not an extension: {s}")));
        }
        Ok(Self::Extension(s.to_string()))
    }
}

impl fmt::Display for CallTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Voicemail => f.write_str("voicemail"),
            Self::Number(n) => write!(f, "number:{n}"),
            Self::Extension(e) => write!(f, "ext:{e}"),
            Self::User(u) => write!(f, "user:{u}"),
            Self::Flow(id) => write!(f, "flow:{id}"),
            Self::Queue(q) => write!(f, "queue:{q}"),
            Self::Sip(u) => f.write_str(u),
        }
    }
}

impl FromStr for CallTarget {
    type Err = CallTargetError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for CallTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CallTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallTargetError {
    #[error("a call target cannot be empty")]
    Empty,
    #[error("{0}")]
    Invalid(String),
}

/// What happens to a call for a user when it is not answered by ringing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Treatment {
    /// Forward to a target.
    Forward { target: CallTarget },
    /// The user's voicemail.
    Voicemail,
    /// Tell the caller: this SIP code (a phone's own refusal is relayed
    /// as it came; 480 when nothing could be reached).
    Refuse { code: u16 },
}

/// The treatments a ringing call falls back to, kept on the call while
/// its legs ring: what the plan decided for busy and for no answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Treatments {
    pub busy: Treatment,
    pub no_answer: Treatment,
}

/// The hold-music choice for silence.
pub const HOLD_MUSIC_NONE: &str = "none";

/// The range a ring time may be set to; unset, the node's own forking
/// timeout applies.
pub const MIN_NO_ANSWER_SECS: u32 = 5;
pub const MAX_NO_ANSWER_SECS: u32 = 300;

/// One user's calling settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallingSettings {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// Do not disturb: calls get the busy treatment without ringing.
    #[serde(default)]
    pub dnd: bool,
    /// A second call rings while the user is on one (phase 4 enforces).
    #[serde(default = "default_true")]
    pub call_waiting: bool,
    /// Every call, without ringing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_all: Option<CallTarget>,
    /// The user's phones refused, or DND, or call waiting off on a call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_busy: Option<CallTarget>,
    /// Rung for `no_answer_secs` and not answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_no_answer: Option<CallTarget>,
    /// No phone registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_unreachable: Option<CallTarget>,
    /// The user's hours (`docs/ENTERPRISE_CALLING.md` §3.5): outside them,
    /// calls go to `after_hours_forward` without ringing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_hours_schedule_id: Option<crate::ScheduleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_hours_forward: Option<CallTarget>,
    /// What a party this user holds hears: a prompt's id, or
    /// [`HOLD_MUSIC_NONE`] for silence; `None` is the tenant's hold music
    /// (`docs/ENTERPRISE_CALLING.md` §3.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_music: Option<String>,
    /// How long the user's phones ring; `None` is the node's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_answer_secs: Option<u32>,
    #[serde(default = "SystemTime::now")]
    pub updated_at: SystemTime,
}

fn default_true() -> bool {
    true
}

impl CallingSettings {
    /// The settings of a user who has never set any.
    pub fn defaults(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            tenant_id,
            user_id,
            dnd: false,
            call_waiting: true,
            forward_all: None,
            forward_busy: None,
            forward_no_answer: None,
            forward_unreachable: None,
            after_hours_schedule_id: None,
            after_hours_forward: None,
            hold_music: None,
            no_answer_secs: None,
            updated_at: SystemTime::now(),
        }
    }

    /// The ring time the user set, within the allowed range; `None` for
    /// the node's default.
    pub fn no_answer(&self) -> Option<std::time::Duration> {
        self.no_answer_secs.map(|secs| {
            std::time::Duration::from_secs(
                secs.clamp(MIN_NO_ANSWER_SECS, MAX_NO_ANSWER_SECS).into(),
            )
        })
    }

    /// Refuse what cannot be stored: a ring time out of range, a forward
    /// that names the user themself.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(secs) = self.no_answer_secs {
            if !(MIN_NO_ANSWER_SECS..=MAX_NO_ANSWER_SECS).contains(&secs) {
                return Err(format!(
                    "no_answer_secs must be between {MIN_NO_ANSWER_SECS} and {MAX_NO_ANSWER_SECS}"
                ));
            }
        }
        for (name, target) in [
            ("forward_all", &self.forward_all),
            ("forward_busy", &self.forward_busy),
            ("forward_no_answer", &self.forward_no_answer),
            ("forward_unreachable", &self.forward_unreachable),
            ("after_hours_forward", &self.after_hours_forward),
        ] {
            if let Some(CallTarget::User(id)) = target {
                if *id == self.user_id {
                    return Err(format!("{name} cannot forward a user to themself"));
                }
            }
        }
        Ok(())
    }
}

/// The store of calling settings: one row per user, absent until set.
#[async_trait]
pub trait CallingStore: Send + Sync {
    /// A user's settings, `None` when never set (the caller applies
    /// [`CallingSettings::defaults`]).
    async fn get_calling_settings(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> AnyhowResult<Option<CallingSettings>>;

    /// Every user's settings in a tenant (the site snapshot).
    async fn list_calling_settings(
        &self,
        tenant_id: &TenantId,
    ) -> AnyhowResult<Vec<CallingSettings>>;

    /// Write a user's settings.
    async fn put_calling_settings(&self, settings: CallingSettings) -> AnyhowResult<()>;

    /// Remove a user's settings (back to the defaults).
    async fn delete_calling_settings(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> AnyhowResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_round_trip_through_their_string_form() {
        let user = UserId::new();
        for target in [
            CallTarget::Voicemail,
            CallTarget::Number("+15551234567".into()),
            CallTarget::Extension("2001".into()),
            CallTarget::User(user),
            CallTarget::Flow("night".into()),
            CallTarget::Queue("sales".into()),
            CallTarget::Sip("sip:bob@example.com".into()),
        ] {
            let s = target.to_string();
            assert_eq!(CallTarget::parse(&s).unwrap(), target, "{s}");
            let json = serde_json::to_string(&target).unwrap();
            assert_eq!(serde_json::from_str::<CallTarget>(&json).unwrap(), target);
        }
    }

    #[test]
    fn bare_forms_are_accepted() {
        assert_eq!(
            CallTarget::parse("+1 555").unwrap_err(),
            CallTargetError::Invalid("not an E.164 number: 1 555".into())
        );
        assert_eq!(
            CallTarget::parse("+15551234567").unwrap(),
            CallTarget::Number("+15551234567".into())
        );
        assert_eq!(
            CallTarget::parse("2001").unwrap(),
            CallTarget::Extension("2001".into())
        );
        assert_eq!(
            CallTarget::parse("Voicemail").unwrap(),
            CallTarget::Voicemail
        );
        assert_eq!(CallTarget::parse("  ").unwrap_err(), CallTargetError::Empty);
        assert!(CallTarget::parse("bob").is_err());
    }

    #[test]
    fn settings_validate_their_range_and_targets() {
        let user = UserId::new();
        let mut s = CallingSettings::defaults(TenantId::new(), user);
        assert!(s.validate().is_ok());
        s.no_answer_secs = Some(2);
        assert!(s.validate().is_err());
        s.no_answer_secs = Some(20);
        s.forward_all = Some(CallTarget::User(user));
        assert!(s.validate().is_err());
    }
}
