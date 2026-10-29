//! Presence a person sets (`docs/PRESENCE.md`): the status a user chose,
//! kept on their account, and what others see once calls, do not
//! disturb, idleness and being signed out are laid over it
//! ([`effective`]).
//!
//! A status is one of four built in (`available`, `away`, `busy`, `dnd`)
//! or one of the user's own: a label ("WFH") on one of those four states.
//! Only `dnd` stops calls, and it does so through the user's calling
//! settings (`CallingSettings::dnd`), which picking a `dnd` status turns
//! on and any other turns off.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{TenantId, UserId};

/// The most statuses of their own a user keeps.
pub const MAX_STATUSES: usize = 20;

/// The longest label of a user's own status.
pub const MAX_LABEL: usize = 40;

/// The longest note.
pub const MAX_NOTE: usize = 100;

/// The furthest ahead a status may be set to clear.
pub const MAX_UNTIL: Duration = Duration::from_secs(7 * 24 * 3600);

/// How long an app's word that its person is idle lasts unless said again.
pub const IDLE_LASTS: Duration = Duration::from_secs(15 * 60);

/// A new status's id.
pub fn new_status_id() -> String {
    format!("status-{}", uuid::Uuid::new_v4())
}

/// The state a status carries: what colour it is and what others' phones
/// are told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Available,
    Away,
    Busy,
    Dnd,
}

impl StatusState {
    pub const ALL: [StatusState; 4] = [Self::Available, Self::Away, Self::Busy, Self::Dnd];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Dnd => "dnd",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// What others see of a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Shown {
    Available,
    Away,
    Busy,
    Dnd,
    /// A call is ringing one of their lines.
    Ringing,
    /// They are on a call.
    OnCall,
    /// None of their phones or apps is registered.
    Offline,
}

impl Shown {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Dnd => "dnd",
            Self::Ringing => "ringing",
            Self::OnCall => "on_call",
            Self::Offline => "offline",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "available" => Self::Available,
            "away" => Self::Away,
            "busy" => Self::Busy,
            "dnd" => Self::Dnd,
            "ringing" => Self::Ringing,
            "on_call" => Self::OnCall,
            "offline" => Self::Offline,
            _ => return None,
        })
    }
}

impl From<StatusState> for Shown {
    fn from(s: StatusState) -> Self {
        match s {
            StatusState::Available => Self::Available,
            StatusState::Away => Self::Away,
            StatusState::Busy => Self::Busy,
            StatusState::Dnd => Self::Dnd,
        }
    }
}

/// One of a user's own statuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnStatus {
    /// `status-` and a UUID ([`new_status_id`]).
    pub id: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub label: String,
    pub state: StatusState,
    /// Where it is in the user's list, from 0.
    pub position: i32,
    pub updated_at: SystemTime,
    /// Deleted: kept as a tombstone for [`TOMBSTONE_DAYS`] so the delete
    /// reaches the other side of a site's link (`docs/PRESENCE.md` §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<SystemTime>,
}

/// How long a deleted status's tombstone is kept.
pub const TOMBSTONE_DAYS: u64 = 30;

/// The status a user chose. No row is [`StatusState::Available`], and so
/// is a row saying `available` with no note or time: clearing a status
/// writes that, so the clearing crosses a site's link
/// (`docs/PRESENCE.md` §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChosenStatus {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// A built-in state's name or one of the user's [`OwnStatus`] ids.
    pub status: String,
    pub note: Option<String>,
    /// When it clears back to Available.
    pub until: Option<SystemTime>,
    pub updated_at: SystemTime,
}

impl ChosenStatus {
    /// Available with nothing to say: the same as no status.
    pub fn is_plain_available(&self) -> bool {
        self.status == StatusState::Available.as_str()
            && self.note.is_none()
            && self.until.is_none()
    }

    /// Whether it has cleared by `now`.
    pub fn cleared(&self, now: SystemTime) -> bool {
        self.until.is_some_and(|u| u <= now)
    }
}

/// A chosen status made sense of: its state, and its label when it is one
/// of the user's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub state: StatusState,
    pub label: Option<String>,
    pub note: Option<String>,
    pub until: Option<SystemTime>,
}

impl Resolved {
    pub fn available() -> Self {
        Self {
            state: StatusState::Available,
            label: None,
            note: None,
            until: None,
        }
    }

    /// `chosen` among the user's `own` statuses at `now`: Available when
    /// none was chosen, it has cleared, or it names a status since
    /// deleted.
    pub fn of(chosen: Option<&ChosenStatus>, own: &[OwnStatus], now: SystemTime) -> Self {
        let Some(c) = chosen.filter(|c| !c.cleared(now)) else {
            return Self::available();
        };
        let (state, label) = match StatusState::parse(&c.status) {
            Some(state) => (state, None),
            None => match own.iter().find(|s| s.id == c.status) {
                Some(s) => (s.state, Some(s.label.clone())),
                None => return Self::available(),
            },
        };
        Self {
            state,
            label,
            note: c.note.clone(),
            until: c.until,
        }
    }
}

/// What is happening to a person besides what they chose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Happening {
    /// At least one of their phones or apps is registered.
    pub registered: bool,
    /// A call is ringing them.
    pub ringing: bool,
    /// They are on a call.
    pub on_call: bool,
    /// Their calling settings' do not disturb.
    pub dnd: bool,
    /// Every app of theirs that says so says they are idle.
    pub idle: bool,
}

/// What others see of a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Presence {
    pub state: Shown,
    /// Their own status's label, kept beside a call ("On a call · WFH").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Shown away because they are idle, not because they said so.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub idle: bool,
}

/// A person's presence: signed out shows offline; a call shows over what
/// they chose, keeping its label and note; do not disturb shows `dnd`;
/// idle shows away while they chose to be available; else what they
/// chose. A `dnd` status no longer holds once DND was turned off
/// elsewhere (`*79`): they are available.
pub fn effective(chosen: &Resolved, now: Happening) -> Presence {
    if !now.registered {
        return Presence {
            state: Shown::Offline,
            label: None,
            note: None,
            idle: false,
        };
    }
    let available = Resolved::available();
    let chosen = if chosen.state == StatusState::Dnd && !now.dnd {
        &available
    } else {
        chosen
    };
    let (state, idle) = if now.on_call {
        (Shown::OnCall, false)
    } else if now.ringing {
        (Shown::Ringing, false)
    } else if now.dnd {
        (Shown::Dnd, false)
    } else if now.idle && chosen.state == StatusState::Available {
        (Shown::Away, true)
    } else {
        (chosen.state.into(), false)
    };
    // DND turned on elsewhere is not their status: its label would say
    // otherwise.
    let label = if state == Shown::Dnd && chosen.state != StatusState::Dnd {
        None
    } else {
        chosen.label.clone()
    };
    Presence {
        state,
        label,
        note: chosen.note.clone(),
        idle,
    }
}

/// Where chosen statuses and users' own statuses are kept.
#[async_trait]
pub trait PresenceStatusStore: Send + Sync {
    /// The status a user chose; `None` is Available (a plain `available`
    /// row reads as `None`).
    async fn get_chosen_status(&self, user: &UserId) -> anyhow::Result<Option<ChosenStatus>>;

    /// Every chosen status kept other than plain Available (the admin
    /// reads them at start).
    async fn list_chosen_statuses(&self) -> anyhow::Result<Vec<ChosenStatus>>;

    /// Write a user's chosen status.
    async fn put_chosen_status(&self, chosen: &ChosenStatus) -> anyhow::Result<()>;

    /// Back to Available: the row says `available`, with no note or time,
    /// as of now, so the clearing reaches a site's other side.
    async fn delete_chosen_status(&self, user: &UserId) -> anyhow::Result<()>;

    /// A user's own statuses, in their order; tombstones left out.
    async fn list_own_statuses(&self, user: &UserId) -> anyhow::Result<Vec<OwnStatus>>;

    /// Write one of a user's own statuses.
    async fn put_own_status(&self, status: &OwnStatus) -> anyhow::Result<()>;

    /// Delete one of a user's own statuses to a tombstone; whether it was
    /// theirs.
    async fn delete_own_status(&self, user: &UserId, id: &str) -> anyhow::Result<bool>;

    // Syncing with sites (`docs/PRESENCE.md` §3): rows as they are, plain
    // Available and tombstones included.

    /// A user's chosen-status row as it is.
    async fn get_chosen_status_row(&self, user: &UserId) -> anyhow::Result<Option<ChosenStatus>>;

    /// A tenant's chosen-status rows, for a site's snapshot.
    async fn tenant_chosen_statuses(&self, tenant: &TenantId) -> anyhow::Result<Vec<ChosenStatus>>;

    /// Write `chosen` when it is newer than the row kept (or none is) and
    /// names the same tenant; whether it was written.
    async fn put_chosen_status_if_newer(&self, chosen: &ChosenStatus) -> anyhow::Result<bool>;

    /// Remove a user's chosen-status row outright (the user went).
    async fn delete_chosen_status_row(&self, user: &UserId) -> anyhow::Result<bool>;

    /// Chosen-status rows written after shipping sequence `after`, in
    /// that order.
    async fn chosen_statuses_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, ChosenStatus)>>;

    /// One own status as it is, tombstone or not.
    async fn get_own_status(&self, id: &str) -> anyhow::Result<Option<OwnStatus>>;

    /// A tenant's own statuses, tombstones included, for a site's
    /// snapshot.
    async fn tenant_own_statuses(&self, tenant: &TenantId) -> anyhow::Result<Vec<OwnStatus>>;

    /// Write `status` when it is newer than the row kept (or none is) and
    /// names the same tenant and user; a tombstone only marks a status
    /// kept here, so a late echo brings no swept row back. Whether it was
    /// written.
    async fn put_own_status_if_newer(&self, status: &OwnStatus) -> anyhow::Result<bool>;

    /// Remove an own status's row outright.
    async fn delete_own_status_row(&self, id: &str) -> anyhow::Result<bool>;

    /// Own statuses written after shipping sequence `after`, in that
    /// order.
    async fn own_statuses_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, OwnStatus)>>;

    /// Remove the tombstones of statuses deleted before `before`.
    async fn sweep_status_tombstones(&self, before: SystemTime) -> anyhow::Result<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(label: &str, state: StatusState) -> OwnStatus {
        OwnStatus {
            id: "status-wfh".into(),
            tenant_id: TenantId::new(),
            user_id: UserId::new(),
            label: label.into(),
            state,
            position: 0,
            updated_at: SystemTime::UNIX_EPOCH,
            deleted_at: None,
        }
    }

    fn chosen(status: &str, until: Option<SystemTime>) -> ChosenStatus {
        ChosenStatus {
            tenant_id: TenantId::new(),
            user_id: UserId::new(),
            status: status.into(),
            note: Some("back at 2".into()),
            until,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    const UP: Happening = Happening {
        registered: true,
        ringing: false,
        on_call: false,
        dnd: false,
        idle: false,
    };

    #[test]
    fn a_status_is_built_in_or_one_of_theirs_until_it_clears() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let wfh = own("WFH", StatusState::Available);
        let r = Resolved::of(
            Some(&chosen("status-wfh", None)),
            std::slice::from_ref(&wfh),
            now,
        );
        assert_eq!(
            (r.state, r.label.as_deref()),
            (StatusState::Available, Some("WFH"))
        );

        let r = Resolved::of(Some(&chosen("busy", None)), &[], now);
        assert_eq!((r.state, r.label), (StatusState::Busy, None));

        let gone = Resolved::of(Some(&chosen("status-wfh", None)), &[], now);
        assert_eq!(gone, Resolved::available(), "a deleted status");

        let past = now - Duration::from_secs(1);
        let r = Resolved::of(Some(&chosen("away", Some(past))), &[], now);
        assert_eq!(r, Resolved::available(), "cleared");
        assert_eq!(Resolved::of(None, &[wfh], now), Resolved::available());
    }

    #[test]
    fn signed_out_calls_dnd_and_idle_lie_over_what_was_chosen() {
        let wfh = Resolved {
            state: StatusState::Available,
            label: Some("WFH".into()),
            note: Some("back at 2".into()),
            until: None,
        };
        assert_eq!(effective(&wfh, UP).state, Shown::Available);
        assert_eq!(effective(&wfh, UP).label.as_deref(), Some("WFH"));

        let off = effective(&wfh, Happening::default());
        assert_eq!((off.state, off.label), (Shown::Offline, None));

        let call = effective(
            &wfh,
            Happening {
                on_call: true,
                ringing: true,
                ..UP
            },
        );
        assert_eq!(call.state, Shown::OnCall);
        assert_eq!(call.label.as_deref(), Some("WFH"), "kept beside the call");
        let ring = effective(
            &wfh,
            Happening {
                ringing: true,
                ..UP
            },
        );
        assert_eq!(ring.state, Shown::Ringing);

        let dnd = effective(&wfh, Happening { dnd: true, ..UP });
        assert_eq!((dnd.state, dnd.label), (Shown::Dnd, None));

        let idle = effective(&wfh, Happening { idle: true, ..UP });
        assert_eq!((idle.state, idle.idle), (Shown::Away, true));
        let busy = Resolved {
            state: StatusState::Busy,
            ..wfh.clone()
        };
        assert_eq!(
            effective(&busy, Happening { idle: true, ..UP }).state,
            Shown::Busy,
            "idle only stands in for available"
        );
    }

    #[test]
    fn a_dnd_status_holds_only_while_dnd_is_on() {
        let focus = Resolved {
            state: StatusState::Dnd,
            label: Some("Focus".into()),
            note: None,
            until: None,
        };
        let on = effective(&focus, Happening { dnd: true, ..UP });
        assert_eq!((on.state, on.label.as_deref()), (Shown::Dnd, Some("Focus")));
        let off = effective(&focus, UP);
        assert_eq!((off.state, off.label), (Shown::Available, None));
    }

    #[test]
    fn states_are_named_as_the_api_says() {
        for s in StatusState::ALL {
            assert_eq!(StatusState::parse(s.as_str()), Some(s));
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                serde_json::Value::String(s.as_str().into())
            );
            assert_eq!(Shown::from(s).as_str(), s.as_str());
        }
        assert_eq!(
            serde_json::to_value(Shown::OnCall).unwrap(),
            serde_json::json!("on_call")
        );
        assert_eq!(StatusState::parse("on_call"), None);
    }
}
