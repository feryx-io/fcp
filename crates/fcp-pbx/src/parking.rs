//! Parked calls on this node (`docs/PARKING.md`).
//!
//! A call is held by its tenant and slot: two tenants may both number a
//! slot `701`. Which slot a call takes is decided by the handler, which
//! claims it cluster-wide first; this map is the node's own calls.

use crate::config::ParkingConfig;
use crate::error::PbxError;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use fcp_b2bua_core::call::CallId;
use fcp_core::parking::ParkingLot;
use fcp_core::TenantId;
use std::time::{Duration, Instant};
use tracing::debug;

/// The calls parked on this node, by tenant and slot.
pub struct ParkingManager {
    config: ParkingConfig,
    parked: DashMap<(TenantId, String), ParkedCall>,
}

/// A single parked call.
#[derive(Debug, Clone)]
pub struct ParkedCall {
    /// Internal call ID of the parked call.
    pub call_id: CallId,

    /// The slot, as the tenant numbers it.
    pub slot: String,

    /// The lot it is in ([`fcp_core::parking::DEFAULT_LOT_ID`] for the
    /// node's own range).
    pub lot_id: String,

    /// Extension of the user who parked the call.
    pub parker_extension: String,

    /// Tenant context.
    pub tenant_id: TenantId,

    /// When the call was parked.
    pub parked_at: Instant,

    /// How long until the parked call times out.
    pub timeout: Duration,
}

impl ParkedCall {
    /// Check whether this parked call has timed out.
    pub fn is_expired(&self) -> bool {
        self.parked_at.elapsed() >= self.timeout
    }
}

impl ParkingManager {
    /// Create a new parking manager from configuration.
    pub fn new(config: ParkingConfig) -> Self {
        Self {
            config,
            parked: DashMap::new(),
        }
    }

    /// The lot a tenant with none of its own parks in: this node's range.
    pub fn node_lot(&self, tenant_id: TenantId) -> ParkingLot {
        ParkingLot::node_default(
            tenant_id,
            self.config.orbit_start,
            self.config.orbit_end,
            self.config.timeout_seconds,
        )
    }

    /// Whether this node holds a call in the tenant's slot.
    pub fn is_held(&self, tenant_id: &TenantId, slot: &str) -> bool {
        self.parked.contains_key(&(*tenant_id, slot.to_string()))
    }

    /// Hold `call` in its slot; refused when the slot holds a call here.
    pub fn hold(&self, call: ParkedCall) -> Result<(), PbxError> {
        match self.parked.entry((call.tenant_id, call.slot.clone())) {
            Entry::Occupied(_) => Err(PbxError::SlotTaken(call.slot)),
            Entry::Vacant(v) => {
                debug!(slot = %call.slot, call_id = %call.call_id, parker = %call.parker_extension, "Parked call");
                v.insert(call);
                Ok(())
            }
        }
    }

    /// Park a call in `lot`'s first slot free on this node, with no one
    /// else to ask: a node without a cluster store.
    pub fn park_in(
        &self,
        lot: &ParkingLot,
        call_id: CallId,
        parker_extension: String,
    ) -> Result<String, PbxError> {
        for slot in lot.slots() {
            let call = ParkedCall {
                call_id,
                slot: slot.clone(),
                lot_id: lot.id.clone(),
                parker_extension: parker_extension.clone(),
                tenant_id: lot.tenant_id,
                parked_at: Instant::now(),
                timeout: Duration::from_secs(u64::from(lot.timeout_secs)),
            };
            if self.hold(call).is_ok() {
                return Ok(slot);
            }
        }
        Err(PbxError::NoParkingOrbitsAvailable)
    }

    /// Take the call out of the tenant's slot.
    pub fn retrieve(&self, tenant_id: &TenantId, slot: &str) -> Result<ParkedCall, PbxError> {
        self.parked
            .remove(&(*tenant_id, slot.to_string()))
            .map(|(_, call)| {
                debug!(slot = %slot, call_id = %call.call_id, "Retrieved parked call");
                call
            })
            .ok_or_else(|| PbxError::OrbitNotOccupied(slot.to_string()))
    }

    /// Collect and remove all expired parked calls.
    ///
    /// Returns a list of expired calls so the handler can route them
    /// (e.g., back to the parker or to voicemail).
    pub fn expired_calls(&self) -> Vec<ParkedCall> {
        let mut expired = Vec::new();
        self.parked.retain(|_, call| {
            if call.is_expired() {
                debug!(
                    slot = %call.slot,
                    call_id = %call.call_id,
                    parker = %call.parker_extension,
                    "Parked call expired"
                );
                expired.push(call.clone());
                false
            } else {
                true
            }
        });
        expired
    }

    /// Number of currently parked calls.
    pub fn parked_count(&self) -> usize {
        self.parked.len()
    }

    /// Whether parking is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the parking configuration.
    pub fn config(&self) -> &ParkingConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ParkingConfig {
        ParkingConfig {
            enabled: true,
            orbit_start: 701,
            orbit_end: 703,
            timeout_seconds: 60,
        }
    }

    #[test]
    fn test_park_and_retrieve() {
        let mgr = ParkingManager::new(test_config());
        let call_id = CallId::new();
        let tenant = TenantId::nil();
        let lot = mgr.node_lot(tenant);

        let slot = mgr.park_in(&lot, call_id, "1001".to_string()).unwrap();
        assert_eq!(slot, "701");
        assert_eq!(mgr.parked_count(), 1);

        let parked = mgr.retrieve(&tenant, "701").unwrap();
        assert_eq!(parked.call_id, call_id);
        assert_eq!(parked.parker_extension, "1001");
        assert_eq!(mgr.parked_count(), 0);
    }

    #[test]
    fn slots_fill_in_order_until_the_lot_is_full() {
        let mgr = ParkingManager::new(test_config());
        let lot = mgr.node_lot(TenantId::nil());
        let slots: Vec<String> = (0..3)
            .map(|_| mgr.park_in(&lot, CallId::new(), "1001".into()).unwrap())
            .collect();
        assert_eq!(slots, ["701", "702", "703"]);
        assert!(mgr.park_in(&lot, CallId::new(), "1004".into()).is_err());
    }

    #[test]
    fn two_tenants_number_their_slots_alike() {
        // D8 finding 35: tenant B's `*71701` took tenant A's call.
        let mgr = ParkingManager::new(test_config());
        let (a, b) = (TenantId::new(), TenantId::new());
        mgr.park_in(&mgr.node_lot(a), CallId::new(), "1001".into())
            .unwrap();
        assert!(mgr.retrieve(&b, "701").is_err(), "not B's");
        let at_b = mgr
            .park_in(&mgr.node_lot(b), CallId::new(), "2001".into())
            .unwrap();
        assert_eq!(at_b, "701", "B's own 701");
        assert!(mgr.retrieve(&a, "701").is_ok());
    }

    #[test]
    fn a_lot_numbers_its_own_slots() {
        let mgr = ParkingManager::new(test_config());
        let lot = ParkingLot::new(TenantId::nil(), "Lobby", "050", "051");
        assert_eq!(mgr.park_in(&lot, CallId::new(), "1".into()).unwrap(), "050");
        assert_eq!(mgr.park_in(&lot, CallId::new(), "1".into()).unwrap(), "051");
    }

    #[test]
    fn test_retrieve_not_occupied() {
        let mgr = ParkingManager::new(test_config());
        let result = mgr.retrieve(&TenantId::nil(), "701");
        assert!(matches!(result, Err(PbxError::OrbitNotOccupied(s)) if s == "701"));
    }

    #[test]
    fn test_expired_calls() {
        let mgr = ParkingManager::new(test_config());
        let mut lot = mgr.node_lot(TenantId::nil());
        lot.timeout_secs = 0; // immediate expiry
        mgr.park_in(&lot, CallId::new(), "1001".to_string())
            .unwrap();

        let expired = mgr.expired_calls();
        assert_eq!(expired.len(), 1);
        assert_eq!(mgr.parked_count(), 0);
    }

    #[test]
    fn test_orbit_reuse_after_retrieve() {
        let mgr = ParkingManager::new(test_config());
        let tenant = TenantId::nil();
        let lot = mgr.node_lot(tenant);

        mgr.park_in(&lot, CallId::new(), "1001".to_string())
            .unwrap();
        mgr.retrieve(&tenant, "701").unwrap();

        let slot = mgr
            .park_in(&lot, CallId::new(), "1002".to_string())
            .unwrap();
        assert_eq!(slot, "701");
    }
}
