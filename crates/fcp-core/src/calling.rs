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

/// Who a find-me step rings (`docs/ENTERPRISE_CALLING.md` §3.6): the
/// user's own phones, or a target — a user, an extension, a number, a SIP
/// URI. Serialised as `devices` or the target's string form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindMeTarget {
    Devices,
    Target(CallTarget),
}

/// A string schema for a type serialised as its string form.
#[cfg(feature = "openapi")]
macro_rules! string_schema {
    ($ty:ty, $doc:expr) => {
        impl utoipa::PartialSchema for $ty {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                utoipa::openapi::ObjectBuilder::new()
                    .schema_type(utoipa::openapi::schema::Type::String)
                    .description(Some($doc))
                    .into()
            }
        }
        impl utoipa::ToSchema for $ty {}
    };
}

#[cfg(feature = "openapi")]
string_schema!(
    CallTarget,
    "Where a call goes: `voicemail`, `number:+…`, `ext:…`, `user:…`, `flow:…`, `queue:…`, or a SIP URI"
);
#[cfg(feature = "openapi")]
string_schema!(
    FindMeTarget,
    "Who a find-me step rings: `devices`, or a target (`number:+…`, `ext:…`, `user:…`, a SIP URI)"
);

impl FindMeTarget {
    pub fn parse(s: &str) -> Result<Self, CallTargetError> {
        if s.trim().eq_ignore_ascii_case("devices") {
            return Ok(Self::Devices);
        }
        let target = CallTarget::parse(s)?;
        match target {
            CallTarget::Voicemail | CallTarget::Flow(_) | CallTarget::Queue(_) => Err(
                CallTargetError::Invalid(format!("{target} cannot be rung by a find-me step")),
            ),
            _ => Ok(Self::Target(target)),
        }
    }
}

impl fmt::Display for FindMeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Devices => f.write_str("devices"),
            Self::Target(t) => t.fmt(f),
        }
    }
}

impl Serialize for FindMeTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FindMeTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// One step of find-me: these targets rung together for `ring_secs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FindMeStep {
    pub targets: Vec<FindMeTarget>,
    #[serde(default = "default_step_secs")]
    pub ring_secs: u32,
    /// A leg of this step that answers must press 1 before it is
    /// connected: a mobile's voicemail does not take the call.
    #[serde(default)]
    pub confirm: bool,
}

fn default_step_secs() -> u32 {
    20
}

/// A user's find-me: steps rung one after another once the user's own
/// phones have rung, before the no-answer treatment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FindMe {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub steps: Vec<FindMeStep>,
}

/// The most steps a find-me may have.
pub const MAX_FIND_ME_STEPS: usize = 5;

/// Which of an executive's calls their assistants take first
/// (`docs/ENTERPRISE_CALLING.md` §3.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExecutiveFilter {
    /// Every call.
    #[default]
    All,
    /// Calls from outside the tenant: a carrier's, a caller who is no user.
    External,
    /// None: the executive's phones ring as if there were no assistants.
    None,
}

/// Who rings for a call the filter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExecutiveAlert {
    /// The assistants alone.
    #[default]
    Assistants,
    /// The assistants and the executive together.
    AssistantsAndExecutive,
}

/// An assistant of an executive, in the order they are listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assistant {
    pub user_id: UserId,
    /// The assistant takes the executive's calls (`*67`/`*68`, their own
    /// page). Theirs to set, not the executive's.
    #[serde(default = "default_true")]
    pub opted_in: bool,
}

/// A user as an executive: their assistants, and which calls those take.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Executive {
    /// The executive's own switch (`*65`/`*66`).
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub filter: ExecutiveFilter,
    #[serde(default)]
    pub alert: ExecutiveAlert,
    #[serde(default)]
    pub assistants: Vec<Assistant>,
}

/// The most assistants an executive may have.
pub const MAX_ASSISTANTS: usize = 10;

impl Executive {
    /// Whether a call is the assistants' to take first: the executive's
    /// handling on, the filter selecting it, and someone opted in.
    /// `external` is a call from outside the tenant.
    pub fn screens(&self, external: bool) -> bool {
        self.enabled
            && match self.filter {
                ExecutiveFilter::All => true,
                ExecutiveFilter::External => external,
                ExecutiveFilter::None => false,
            }
            && self.assistants.iter().any(|a| a.opted_in)
    }

    /// The assistants who take calls now, in order.
    pub fn opted_in(&self) -> Vec<UserId> {
        self.assistants
            .iter()
            .filter(|a| a.opted_in)
            .map(|a| a.user_id)
            .collect()
    }
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
    /// Find-me / follow-me (§3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find_me: Option<FindMe>,
    /// What a party this user holds hears: a prompt's id, or
    /// [`HOLD_MUSIC_NONE`] for silence; `None` is the tenant's hold music
    /// (`docs/ENTERPRISE_CALLING.md` §3.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_music: Option<String>,
    /// How long the user's phones ring; `None` is the node's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_answer_secs: Option<u32>,
    /// The user's class of service (§3.7); `None` is the tenant's default
    /// class, or no restriction when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_of_service_id: Option<crate::ClassOfServiceId>,
    /// The user as an executive, with assistants (§3.10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executive: Option<Executive>,
    #[serde(default = "SystemTime::now")]
    pub updated_at: SystemTime,
}

fn default_true() -> bool {
    true
}

impl CallingSettings {
    /// Forget another user who is gone (`docs/USERS.md` §4): a forward to
    /// them is cleared, a find-me step stops ringing them (and a step left
    /// ringing nobody goes), and they are no longer an assistant. Whether
    /// anything changed.
    pub fn forget_user(&mut self, gone: &UserId) -> bool {
        let target = CallTarget::User(*gone);
        let mut changed = false;
        for forward in [
            &mut self.forward_all,
            &mut self.forward_busy,
            &mut self.forward_no_answer,
            &mut self.forward_unreachable,
            &mut self.after_hours_forward,
        ] {
            if forward.as_ref() == Some(&target) {
                *forward = None;
                changed = true;
            }
        }
        if let Some(find_me) = &mut self.find_me {
            let step_target = FindMeTarget::Target(target.clone());
            for step in &mut find_me.steps {
                let before = step.targets.len();
                step.targets.retain(|t| *t != step_target);
                changed |= step.targets.len() != before;
            }
            find_me.steps.retain(|step| !step.targets.is_empty());
            if find_me.steps.is_empty() && find_me.enabled {
                find_me.enabled = false;
            }
        }
        if let Some(executive) = &mut self.executive {
            let before = executive.assistants.len();
            executive.assistants.retain(|a| a.user_id != *gone);
            changed |= executive.assistants.len() != before;
        }
        changed
    }

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
            find_me: None,
            hold_music: None,
            no_answer_secs: None,
            class_of_service_id: None,
            executive: None,
            updated_at: SystemTime::now(),
        }
    }

    /// Whether `user` is one of this executive's assistants, and opted in.
    pub fn assistant(&self, user: &UserId) -> Option<&Assistant> {
        self.executive
            .as_ref()?
            .assistants
            .iter()
            .find(|a| a.user_id == *user)
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

    /// The find-me steps to ring after the user's phones, when find-me is
    /// on.
    pub fn find_me_steps(&self) -> Vec<FindMeStep> {
        self.find_me
            .as_ref()
            .filter(|f| f.enabled)
            .map(|f| f.steps.clone())
            .unwrap_or_default()
    }

    /// Refuse what cannot be stored: a ring time out of range, a forward
    /// that names the user themself, a find-me that cannot be rung.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(find_me) = &self.find_me {
            if find_me.steps.len() > MAX_FIND_ME_STEPS {
                return Err(format!("find_me: at most {MAX_FIND_ME_STEPS} steps"));
            }
            for (i, step) in find_me.steps.iter().enumerate() {
                if step.targets.is_empty() {
                    return Err(format!("find_me: step {} rings nobody", i + 1));
                }
                if !(MIN_NO_ANSWER_SECS..=MAX_NO_ANSWER_SECS).contains(&step.ring_secs) {
                    return Err(format!(
                        "find_me: step {} rings for {}s, not between {MIN_NO_ANSWER_SECS} and {MAX_NO_ANSWER_SECS}",
                        i + 1,
                        step.ring_secs
                    ));
                }
                if step
                    .targets
                    .contains(&FindMeTarget::Target(CallTarget::User(self.user_id)))
                {
                    return Err(format!(
                        "find_me: step {} names the user; use `devices`",
                        i + 1
                    ));
                }
            }
        }
        if let Some(executive) = &self.executive {
            if executive.assistants.len() > MAX_ASSISTANTS {
                return Err(format!("executive: at most {MAX_ASSISTANTS} assistants"));
            }
            let mut seen = std::collections::HashSet::new();
            for assistant in &executive.assistants {
                if assistant.user_id == self.user_id {
                    return Err("executive: a user cannot be their own assistant".into());
                }
                if !seen.insert(assistant.user_id) {
                    return Err("executive: an assistant is listed twice".into());
                }
            }
        }
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
    fn a_gone_user_is_forgotten_by_everyone_who_named_them() {
        let me = UserId::new();
        let gone = UserId::new();
        let other = UserId::new();
        let mut s = CallingSettings::defaults(TenantId::nil(), me);
        s.forward_busy = Some(CallTarget::User(gone));
        s.forward_all = Some(CallTarget::User(other));
        s.find_me = Some(FindMe {
            enabled: true,
            steps: vec![
                FindMeStep {
                    targets: vec![FindMeTarget::Target(CallTarget::User(gone))],
                    ring_secs: 20,
                    confirm: false,
                },
                FindMeStep {
                    targets: vec![
                        FindMeTarget::Devices,
                        FindMeTarget::Target(CallTarget::User(gone)),
                    ],
                    ring_secs: 20,
                    confirm: false,
                },
            ],
        });
        s.executive = Some(Executive {
            enabled: true,
            assistants: vec![
                Assistant {
                    user_id: gone,
                    opted_in: true,
                },
                Assistant {
                    user_id: other,
                    opted_in: true,
                },
            ],
            ..Default::default()
        });

        assert!(s.forget_user(&gone));
        assert_eq!(s.forward_busy, None);
        assert_eq!(s.forward_all, Some(CallTarget::User(other)));
        let find_me = s.find_me.as_ref().unwrap();
        assert!(find_me.enabled);
        assert_eq!(find_me.steps.len(), 1);
        assert_eq!(find_me.steps[0].targets, vec![FindMeTarget::Devices]);
        let assistants = &s.executive.as_ref().unwrap().assistants;
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].user_id, other);
        assert!(s.validate().is_ok());

        // Nothing left to forget.
        assert!(!s.forget_user(&gone));
    }

    #[test]
    fn an_executive_screens_the_calls_its_filter_takes() {
        let assistant = UserId::new();
        let mut executive = Executive {
            enabled: true,
            filter: ExecutiveFilter::External,
            alert: ExecutiveAlert::Assistants,
            assistants: vec![Assistant {
                user_id: assistant,
                opted_in: true,
            }],
        };
        assert!(executive.screens(true));
        assert!(
            !executive.screens(false),
            "an internal call rings the executive"
        );
        executive.filter = ExecutiveFilter::All;
        assert!(executive.screens(false));
        executive.assistants[0].opted_in = false;
        assert!(!executive.screens(true), "nobody opted in takes nothing");
        executive.assistants[0].opted_in = true;
        executive.enabled = false;
        assert!(!executive.screens(true), "the executive's switch is off");
    }

    #[test]
    fn an_executive_lists_each_assistant_once_and_not_themself() {
        let user = UserId::new();
        let other = UserId::new();
        let mut settings = CallingSettings::defaults(TenantId::new(), user);
        settings.executive = Some(Executive {
            assistants: vec![
                Assistant {
                    user_id: other,
                    opted_in: true,
                },
                Assistant {
                    user_id: other,
                    opted_in: false,
                },
            ],
            ..Default::default()
        });
        assert!(settings.validate().is_err());
        settings.executive = Some(Executive {
            assistants: vec![Assistant {
                user_id: user,
                opted_in: true,
            }],
            ..Default::default()
        });
        assert!(settings.validate().is_err());
        settings.executive = Some(Executive {
            assistants: vec![Assistant {
                user_id: other,
                opted_in: true,
            }],
            ..Default::default()
        });
        assert!(settings.validate().is_ok());
        assert!(settings.assistant(&other).is_some());
    }

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
    fn find_me_targets_and_steps_validate() {
        assert_eq!(
            FindMeTarget::parse("devices").unwrap(),
            FindMeTarget::Devices
        );
        assert_eq!(
            FindMeTarget::parse("+15551234567").unwrap(),
            FindMeTarget::Target(CallTarget::Number("+15551234567".into()))
        );
        assert!(FindMeTarget::parse("voicemail").is_err());
        assert!(FindMeTarget::parse("flow:x").is_err());
        let json = r#"{"enabled":true,"steps":[{"targets":["devices","ext:2002"],"ring_secs":15,"confirm":true}]}"#;
        let fm: FindMe = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&fm).unwrap(), json);
        let mut s = CallingSettings::defaults(TenantId::new(), UserId::new());
        s.find_me = Some(fm);
        assert!(s.validate().is_ok());
        assert_eq!(s.find_me_steps().len(), 1);
        s.find_me.as_mut().unwrap().steps[0].ring_secs = 1;
        assert!(s.validate().is_err());
        s.find_me.as_mut().unwrap().enabled = false;
        assert!(s.find_me_steps().is_empty());
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
