//! Parking lots (`docs/PARKING.md`, D8 finding 35).
//!
//! A lot belongs to a tenant and holds a range of slots, numbered by the
//! tenant: `701`–`720`, `500`–`599`, `4320`–`4329`, `050`–`059`. A slot is a
//! digit string, as an extension is, so a leading zero is kept. A call parked
//! in a slot is held by its tenant and slot, cluster-wide.

use crate::{CallTarget, TenantId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The shortest and longest slot number.
pub const MIN_DIGITS: usize = 2;
pub const MAX_DIGITS: usize = 8;
/// The most slots one lot holds.
pub const MAX_SLOTS: u64 = 1000;
/// How long a call may wait, at most.
pub const MAX_TIMEOUT_SECS: u32 = 3600;

/// The id of the lot a tenant with none parks in, built from the node's
/// `[pbx.parking]`.
pub const DEFAULT_LOT_ID: &str = "default";

/// A tenant's parking lot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParkingLot {
    /// `lot-<uuid>`.
    pub id: String,
    pub tenant_id: TenantId,
    pub name: String,
    /// The range, inclusive: digit strings of the same length.
    pub first: String,
    pub last: String,
    /// A number that parks in the lot's next free slot, as `*70` does.
    pub park_number: Option<String>,
    /// How long a call waits before it rings its parker again.
    pub timeout_secs: u32,
    /// Where a call goes when its parker does not take it back.
    pub fallback: Option<CallTarget>,
    /// The lot's hold music prompt, else the tenant's.
    pub music: Option<String>,
    /// A site's lot: `*70` from a phone at that location parks here.
    pub location_id: Option<String>,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl ParkingLot {
    /// A new, enabled lot.
    pub fn new(tenant_id: TenantId, name: &str, first: &str, last: &str) -> Self {
        let now = SystemTime::now();
        Self {
            id: format!("lot-{}", uuid::Uuid::new_v4()),
            tenant_id,
            name: name.to_string(),
            first: first.to_string(),
            last: last.to_string(),
            park_number: None,
            timeout_secs: 60,
            fallback: None,
            music: None,
            location_id: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// The lot a tenant with none parks in: the node's range, numbers as
    /// they were (`701`–`720`).
    pub fn node_default(tenant_id: TenantId, first: u16, last: u16, timeout_secs: u64) -> Self {
        let mut lot = Self::new(tenant_id, "Default", &first.to_string(), &last.to_string());
        lot.id = DEFAULT_LOT_ID.to_string();
        lot.timeout_secs = timeout_secs.min(u64::from(MAX_TIMEOUT_SECS)) as u32;
        lot
    }

    /// Refuse a lot no phone could dial or the cluster could hold.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 64 {
            return Err("name must be 1 to 64 characters".into());
        }
        for (what, n) in [("first", &self.first), ("last", &self.last)] {
            if !is_number(n) {
                return Err(format!(
                    "{what} must be {MIN_DIGITS} to {MAX_DIGITS} digits, got {n:?}"
                ));
            }
        }
        if self.first.len() != self.last.len() {
            return Err("first and last must have the same number of digits".into());
        }
        if self.first > self.last {
            return Err("first must not be after last".into());
        }
        if self.slot_count() > MAX_SLOTS {
            return Err(format!("a lot holds at most {MAX_SLOTS} slots"));
        }
        if let Some(n) = &self.park_number {
            if !is_number(n) {
                return Err(format!(
                    "park_number must be {MIN_DIGITS} to {MAX_DIGITS} digits, got {n:?}"
                ));
            }
            if self.contains(n) {
                return Err("park_number must not be one of the lot's slots".into());
            }
        }
        if self.timeout_secs == 0 || self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!("timeout_secs must be 1 to {MAX_TIMEOUT_SECS}"));
        }
        Ok(())
    }

    /// How many slots the range holds.
    pub fn slot_count(&self) -> u64 {
        match (self.first.parse::<u64>(), self.last.parse::<u64>()) {
            (Ok(a), Ok(b)) if b >= a => b - a + 1,
            _ => 0,
        }
    }

    /// Whether `number` is one of the lot's slots: the same width, inside
    /// the range.
    pub fn contains(&self, number: &str) -> bool {
        number.len() == self.first.len()
            && number.bytes().all(|b| b.is_ascii_digit())
            && number >= self.first.as_str()
            && number <= self.last.as_str()
    }

    /// The slots in order, zero-padded to the range's width.
    pub fn slots(&self) -> impl Iterator<Item = String> + '_ {
        let width = self.first.len();
        let (a, b) = (
            self.first.parse::<u64>().unwrap_or(1),
            self.last.parse::<u64>().unwrap_or(0),
        );
        (a..=b).map(move |n| format!("{n:0width$}"))
    }

    /// The numbers the lot takes in the dial plan: its slots and its park
    /// number.
    pub fn numbers(&self) -> impl Iterator<Item = String> + '_ {
        self.slots().chain(self.park_number.clone())
    }

    /// The first number two lots share, if any.
    pub fn overlap(&self, other: &ParkingLot) -> Option<String> {
        if let Some(n) = &other.park_number {
            if self.contains(n) || self.park_number.as_deref() == Some(n) {
                return Some(n.clone());
            }
        }
        if let Some(n) = &self.park_number {
            if other.contains(n) {
                return Some(n.clone());
            }
        }
        if self.first.len() == other.first.len()
            && self.first <= other.last
            && other.first <= self.last
        {
            return Some(std::cmp::max(&self.first, &other.first).clone());
        }
        None
    }
}

/// A slot or park number: two to eight digits.
pub fn is_number(s: &str) -> bool {
    (MIN_DIGITS..=MAX_DIGITS).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// The lot a call from `location` parks in: the location's own, else the
/// first enabled lot with no location, else any enabled lot.
pub fn lot_for<'a>(lots: &'a [ParkingLot], location: Option<&str>) -> Option<&'a ParkingLot> {
    let enabled = || lots.iter().filter(|l| l.enabled);
    location
        .and_then(|loc| {
            enabled().find(|l| {
                l.location_id
                    .as_deref()
                    .is_some_and(|l| same_location(l, loc))
            })
        })
        .or_else(|| enabled().find(|l| l.location_id.is_none()))
        .or_else(|| enabled().next())
}

/// Whether two location ids name one location: `loc-<uuid>` and the bare
/// uuid are the same, as `LocationId` reads them.
fn same_location(a: &str, b: &str) -> bool {
    match (
        a.parse::<crate::LocationId>(),
        b.parse::<crate::LocationId>(),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Where a tenant's parking lots are kept.
#[async_trait]
pub trait ParkingLotStore: Send + Sync {
    /// The tenant's lots, by first slot.
    async fn list_parking_lots(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<ParkingLot>>;
    async fn get_parking_lot(
        &self,
        tenant_id: &TenantId,
        id: &str,
    ) -> anyhow::Result<Option<ParkingLot>>;
    async fn put_parking_lot(&self, lot: &ParkingLot) -> anyhow::Result<()>;
    /// Whether it existed.
    async fn delete_parking_lot(&self, tenant_id: &TenantId, id: &str) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(first: &str, last: &str) -> ParkingLot {
        ParkingLot::new(TenantId::nil(), "Lot", first, last)
    }

    #[test]
    fn a_range_is_digits_of_one_width_in_order() {
        assert!(lot("701", "720").validate().is_ok());
        assert!(lot("4320", "4329").validate().is_ok());
        assert!(lot("050", "059").validate().is_ok());
        assert!(lot("7", "9").validate().is_err(), "one digit");
        assert!(lot("701", "7200").validate().is_err(), "two widths");
        assert!(lot("720", "701").validate().is_err(), "backwards");
        assert!(lot("7a1", "720").validate().is_err(), "not digits");
        assert!(lot("1000", "2000").validate().is_err(), "over 1000 slots");
        assert!(lot("1000", "1999").validate().is_ok(), "1000 slots");
    }

    #[test]
    fn slots_keep_their_width() {
        let l = lot("050", "053");
        assert_eq!(l.slots().collect::<Vec<_>>(), ["050", "051", "052", "053"]);
        assert!(l.contains("051"));
        assert!(!l.contains("51"), "another width is another number");
        assert!(!l.contains("054"));
        assert_eq!(lot("4320", "4329").slot_count(), 10);
    }

    #[test]
    fn a_park_number_is_outside_the_range() {
        let mut l = lot("701", "720");
        l.park_number = Some("700".into());
        assert!(l.validate().is_ok());
        l.park_number = Some("705".into());
        assert!(l.validate().is_err());
    }

    #[test]
    fn lots_overlap_by_slot_or_park_number() {
        let a = lot("701", "720");
        assert_eq!(a.overlap(&lot("715", "730")), Some("715".into()));
        assert_eq!(a.overlap(&lot("721", "730")), None);
        assert_eq!(a.overlap(&lot("7010", "7020")), None, "another width");
        let mut b = lot("500", "510");
        b.park_number = Some("705".into());
        assert_eq!(a.overlap(&b), Some("705".into()));
    }

    #[test]
    fn a_location_parks_in_its_own_lot() {
        let mut site = lot("500", "510");
        site.location_id = Some("loc-1".into());
        let main = lot("701", "720");
        let lots = vec![site, main];
        assert_eq!(lot_for(&lots, Some("loc-1")).unwrap().first, "500");
        assert_eq!(lot_for(&lots, Some("loc-2")).unwrap().first, "701");
        assert_eq!(lot_for(&lots, None).unwrap().first, "701");
    }
}
