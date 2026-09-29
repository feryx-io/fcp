//! Schedules (`docs/ENTERPRISE_CALLING.md` §3.5): a tenant's business hours
//! and holidays, in a timezone, and the state they give at an instant —
//! open, closed, or a holiday. A number, a user's after-hours forward, a
//! queue and a flow each name a schedule and say where a call goes when it
//! is not open.

use crate::{ScheduleId, TenantId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A span of hours on some days: `start` to `end` local time; an `end`
/// before its `start` runs past midnight into the next day, and an `end`
/// equal to its `start` is the whole day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WeeklyWindow {
    /// `mon` … `sun`.
    /// `Mon` … `Sun`.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<String>))]
    pub days: Vec<Weekday>,
    /// `HH:MM`.
    #[serde(with = "hhmm")]
    pub start: NaiveTime,
    #[serde(with = "hhmm")]
    pub end: NaiveTime,
}

/// A day (or days, `from` to `to` inclusive) the schedule is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Holiday {
    pub name: String,
    pub from: NaiveDate,
    /// The last day, when more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<NaiveDate>,
}

/// What a schedule says at an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleState {
    Open,
    Closed,
    Holiday,
}

impl ScheduleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Holiday => "holiday",
        }
    }
}

/// A tenant's schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: ScheduleId,
    pub tenant_id: TenantId,
    pub name: String,
    /// IANA, e.g. `America/New_York`.
    pub timezone: String,
    #[serde(default)]
    pub weekly: Vec<WeeklyWindow>,
    #[serde(default)]
    pub holidays: Vec<Holiday>,
    #[serde(default = "SystemTime::now")]
    pub created_at: SystemTime,
    #[serde(default = "SystemTime::now")]
    pub updated_at: SystemTime,
}

impl Schedule {
    pub fn new(tenant_id: TenantId, name: impl Into<String>, timezone: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            id: ScheduleId::new(),
            tenant_id,
            name: name.into(),
            timezone: timezone.into(),
            weekly: Vec::new(),
            holidays: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Refuse what cannot be evaluated: an unknown timezone, a window with
    /// no days or no length, a holiday ending before it starts.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a schedule needs a name".into());
        }
        self.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| format!("unknown timezone: {}", self.timezone))?;
        for w in &self.weekly {
            if w.days.is_empty() {
                return Err("a weekly window needs at least one day".into());
            }
        }
        for h in &self.holidays {
            if h.to.is_some_and(|to| to < h.from) {
                return Err(format!("holiday {} ends before it starts", h.name));
            }
        }
        Ok(())
    }

    /// The state at `at`: a holiday first, then open inside any weekly
    /// window, else closed. An unknown timezone is read as UTC.
    pub fn state_at(&self, at: DateTime<Utc>) -> ScheduleState {
        let tz: chrono_tz::Tz = self.timezone.parse().unwrap_or(chrono_tz::UTC);
        let local = tz.from_utc_datetime(&at.naive_utc());
        let today = local.date_naive();
        if self
            .holidays
            .iter()
            .any(|h| today >= h.from && today <= h.to.unwrap_or(h.from))
        {
            return ScheduleState::Holiday;
        }
        let (day, time) = (local.weekday(), local.time());
        let open = self.weekly.iter().any(|w| {
            if w.start == w.end {
                // The whole day.
                w.days.contains(&day)
            } else if w.start < w.end {
                w.days.contains(&day) && time >= w.start && time < w.end
            } else {
                // Past midnight: the evening of a listed day, or the
                // morning after one.
                (w.days.contains(&day) && time >= w.start)
                    || (w.days.contains(&day.pred()) && time < w.end)
            }
        });
        if open {
            ScheduleState::Open
        } else {
            ScheduleState::Closed
        }
    }

    /// The state now.
    pub fn state_now(&self) -> ScheduleState {
        self.state_at(Utc::now())
    }
}

mod hhmm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.format("%H:%M").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let raw = String::deserialize(d)?;
        NaiveTime::parse_from_str(&raw, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(&raw, "%H:%M:%S"))
            .map_err(serde::de::Error::custom)
    }
}

/// The store of schedules.
#[async_trait]
pub trait ScheduleStore: Send + Sync {
    async fn get_schedule(&self, id: &ScheduleId) -> AnyhowResult<Option<Schedule>>;
    async fn list_schedules(&self, tenant_id: &TenantId) -> AnyhowResult<Vec<Schedule>>;
    async fn put_schedule(&self, schedule: Schedule) -> AnyhowResult<()>;
    async fn delete_schedule(&self, id: &ScheduleId) -> AnyhowResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(tz: &str, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        let tz: chrono_tz::Tz = tz.parse().unwrap();
        tz.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn office() -> Schedule {
        let mut s = Schedule::new(TenantId::new(), "office", "America/New_York");
        s.weekly.push(WeeklyWindow {
            days: vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ],
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        });
        s.holidays.push(Holiday {
            name: "Thanksgiving".into(),
            from: NaiveDate::from_ymd_opt(2026, 11, 26).unwrap(),
            to: Some(NaiveDate::from_ymd_opt(2026, 11, 27).unwrap()),
        });
        s
    }

    #[test]
    fn weekdays_open_in_hours_in_their_own_timezone() {
        let s = office();
        // Monday 2026-09-28, 10:00 in New York.
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 9, 28, 10, 0)),
            ScheduleState::Open
        );
        // The same instant is 14:00 UTC: the schedule's zone is what counts.
        assert_eq!(
            s.state_at(at("UTC", 2026, 9, 28, 14, 0)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 9, 28, 17, 0)),
            ScheduleState::Closed
        );
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 9, 28, 8, 59)),
            ScheduleState::Closed
        );
        // Saturday.
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 10, 3, 10, 0)),
            ScheduleState::Closed
        );
    }

    #[test]
    fn a_holiday_wins_over_the_hours_and_spans_its_days() {
        let s = office();
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 11, 26, 10, 0)),
            ScheduleState::Holiday
        );
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 11, 27, 10, 0)),
            ScheduleState::Holiday
        );
        assert_eq!(
            s.state_at(at("America/New_York", 2026, 11, 30, 10, 0)),
            ScheduleState::Open
        );
    }

    #[test]
    fn a_window_past_midnight_opens_the_next_morning() {
        let mut s = Schedule::new(TenantId::new(), "night", "Europe/London");
        s.weekly.push(WeeklyWindow {
            days: vec![Weekday::Fri],
            start: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
        });
        // Friday 23:00, Saturday 01:30 open; Saturday 02:00 and Friday 21:00 not.
        assert_eq!(
            s.state_at(at("Europe/London", 2026, 10, 2, 23, 0)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("Europe/London", 2026, 10, 3, 1, 30)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("Europe/London", 2026, 10, 3, 2, 0)),
            ScheduleState::Closed
        );
        assert_eq!(
            s.state_at(at("Europe/London", 2026, 10, 2, 21, 0)),
            ScheduleState::Closed
        );
    }

    #[test]
    fn a_window_starting_where_it_ends_is_the_whole_day() {
        let mut s = Schedule::new(TenantId::new(), "always", "UTC");
        s.weekly.push(WeeklyWindow {
            days: vec![Weekday::Sat],
            start: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        });
        assert!(s.validate().is_ok());
        assert_eq!(
            s.state_at(at("UTC", 2026, 10, 3, 0, 0)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("UTC", 2026, 10, 3, 23, 59)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("UTC", 2026, 10, 4, 0, 0)),
            ScheduleState::Closed
        );
    }

    #[test]
    fn daylight_saving_moves_with_the_zone() {
        let s = office();
        // 2026-11-02 (Monday, after the change): 09:30 New York is 14:30 UTC.
        assert_eq!(
            s.state_at(at("UTC", 2026, 11, 2, 14, 30)),
            ScheduleState::Open
        );
        assert_eq!(
            s.state_at(at("UTC", 2026, 11, 2, 13, 30)),
            ScheduleState::Closed
        );
    }

    #[test]
    fn a_schedule_round_trips_and_validates() {
        let s = office();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"09:00\""), "{json}");
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert!(s.validate().is_ok());
        let mut bad = s.clone();
        bad.timezone = "Mars/Olympus".into();
        assert!(bad.validate().is_err());
        let mut bad = s;
        bad.weekly[0].days.clear();
        assert!(bad.validate().is_err());
    }
}
