//! The CDR archive's bookkeeping (`docs/CDR_PIPELINE.md` §13, phase 5): each
//! tenant's records for a UTC day go to one gzipped JSON Lines object, and
//! the system of record remembers which days are archived, which are not
//! yet, and which gained records after they were.
//!
//! The archiver walks days in order. `next_day` is the first day it has not
//! enumerated; every earlier day is archived for every tenant that had
//! records on it, except the days listed as pending: begun and never
//! confirmed, or given a record after they were archived (a late record).
//! A tenant's *frontier* is the start of its first day that is not archived
//! and confirmed; retention may delete a tenant's records before the
//! frontier and no later when its policy requires the archive.

use crate::TenantId;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// What the archive holds for one tenant and one day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdrArchiveEntry {
    pub tenant_id: TenantId,
    pub day: NaiveDate,
    /// How many times the day has been written; each rewrite bumps it.
    pub generation: u64,
    /// The object, `None` until a write with records confirms one.
    pub object_key: Option<String>,
    /// Records in the object.
    pub records: u64,
    /// The object's size, compressed.
    pub bytes: u64,
    /// Hex SHA-256 of the object as stored.
    pub sha256: Option<String>,
    /// When the target confirmed the object; `None` while a write is under
    /// way or after one failed.
    pub archived_at: Option<SystemTime>,
    /// Records were stored for the day after this write began; the day is
    /// written again on the next run.
    pub late: bool,
}

/// A day written and confirmed, as the archiver reports it to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdrArchiveWrite {
    pub tenant_id: TenantId,
    pub day: NaiveDate,
    /// The generation [`begin`](crate::storage::CdrStore::begin_cdr_archive)
    /// handed out; a write for an older generation is ignored.
    pub generation: u64,
    /// `None` when the day turned out to have no records: an earlier
    /// object, if there is one, is kept as it is.
    pub object_key: Option<String>,
    pub records: u64,
    pub bytes: u64,
    pub sha256: Option<String>,
}

/// The archive as a whole, for status pages and metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdrArchiveStatus {
    /// The first day not yet enumerated; `None` before the first run, or
    /// while there are no records.
    pub next_day: Option<NaiveDate>,
    /// Tenant-days archived and confirmed.
    pub archived_days: u64,
    /// Tenant-days begun and not confirmed, or given late records.
    pub pending_days: u64,
    /// Records and bytes across the confirmed objects.
    pub records: u64,
    pub bytes: u64,
    /// The newest confirmation.
    pub last_archived_at: Option<SystemTime>,
}

/// Midnight UTC at the start of `day`.
pub fn day_start(day: NaiveDate) -> SystemTime {
    Utc.from_utc_datetime(&day.and_time(NaiveTime::MIN)).into()
}

/// The UTC day `time` falls on.
pub fn day_of(time: SystemTime) -> NaiveDate {
    chrono::DateTime::<Utc>::from(time).date_naive()
}

/// A day is closed once it ended at least `grace` ago: records that reach
/// the store later than that are late records, and the day is written again.
pub fn day_is_closed(day: NaiveDate, grace: Duration, now: SystemTime) -> bool {
    match day.succ_opt() {
        Some(next) => day_start(next) + grace <= now,
        None => false,
    }
}

/// The object key for a tenant's day: `<prefix><tenant>/<yyyy>/<mm>/<dd>.jsonl.gz`.
pub fn object_key(prefix: &str, tenant_id: &TenantId, day: NaiveDate) -> String {
    format!("{prefix}{tenant_id}/{}.jsonl.gz", day.format("%Y/%m/%d"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_utc() {
        let day = NaiveDate::from_ymd_opt(2026, 9, 13).unwrap();
        let start = day_start(day);
        assert_eq!(
            start
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                % 86_400,
            0
        );
        assert_eq!(day_of(start), day);
        assert_eq!(
            day_of(start - Duration::from_secs(1)),
            day.pred_opt().unwrap()
        );
        assert_eq!(day_of(start + Duration::from_secs(86_399)), day);
    }

    #[test]
    fn a_day_closes_after_its_grace() {
        let day = NaiveDate::from_ymd_opt(2026, 9, 13).unwrap();
        let end = day_start(day.succ_opt().unwrap());
        let grace = Duration::from_secs(7200);
        assert!(!day_is_closed(day, grace, end));
        assert!(!day_is_closed(day, grace, end + Duration::from_secs(7199)));
        assert!(day_is_closed(day, grace, end + grace));
    }

    #[test]
    fn keys_sort_by_tenant_then_day() {
        let tenant: TenantId = "00000000-0000-0000-0000-000000000007".parse().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        assert_eq!(
            object_key("cdrs/", &tenant, day),
            "cdrs/tenant-00000000-0000-0000-0000-000000000007/2026/09/03.jsonl.gz"
        );
    }
}
