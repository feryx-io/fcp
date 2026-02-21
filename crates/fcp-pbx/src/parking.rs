//! Call parking orbit management.
//!
//! Manages parked calls in numbered orbits. The handler parks a call by
//! placing its A-leg on hold and recording it here; retrieval removes the
//! entry and lets the handler reconnect media.

use crate::config::ParkingConfig;
use crate::error::PbxError;
use dashmap::DashMap;
use fcp_b2bua_core::call::CallId;
use std::time::{Duration, Instant};
use tracing::debug;

/// Manages parked calls across numbered orbits.
pub struct ParkingManager {
    config: ParkingConfig,
    parked: DashMap<u16, ParkedCall>,
}

/// A single parked call.
#[derive(Debug, Clone)]
pub struct ParkedCall {
    /// Internal call ID of the parked call.
    pub call_id: CallId,

    /// Orbit number where the call is parked.
    pub orbit: u16,

    /// Extension of the user who parked the call.
    pub parker_extension: String,

    /// Tenant context.
    pub tenant_id: fcp_core::TenantId,

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

    /// Park a call in the next available orbit.
    ///
    /// Returns the assigned orbit number.
    pub fn park(
        &self,
        call_id: CallId,
        parker_extension: String,
        tenant_id: fcp_core::TenantId,
    ) -> Result<u16, PbxError> {
        let orbit = self.next_available_orbit()?;
        let timeout = Duration::from_secs(self.config.timeout_seconds);

        let parked_call = ParkedCall {
            call_id,
            orbit,
            parker_extension: parker_extension.clone(),
            tenant_id,
            parked_at: Instant::now(),
            timeout,
        };

        debug!(
            orbit = orbit,
            call_id = %call_id,
            parker = %parker_extension,
            "Parked call"
        );

        self.parked.insert(orbit, parked_call);
        Ok(orbit)
    }

    /// Retrieve (un-park) a call from a specific orbit.
    ///
    /// Removes the call from the parking map and returns it.
    pub fn retrieve(&self, orbit: u16) -> Result<ParkedCall, PbxError> {
        if orbit < self.config.orbit_start || orbit > self.config.orbit_end {
            return Err(PbxError::OrbitOutOfRange(
                orbit,
                self.config.orbit_start,
                self.config.orbit_end,
            ));
        }

        self.parked
            .remove(&orbit)
            .map(|(_, call)| {
                debug!(orbit = orbit, call_id = %call.call_id, "Retrieved parked call");
                call
            })
            .ok_or(PbxError::OrbitNotOccupied(orbit))
    }

    /// Collect and remove all expired parked calls.
    ///
    /// Returns a list of expired calls so the handler can route them
    /// (e.g., back to the parker or to voicemail).
    pub fn expired_calls(&self) -> Vec<ParkedCall> {
        let mut expired = Vec::new();
        self.parked.retain(|_orbit, call| {
            if call.is_expired() {
                debug!(
                    orbit = call.orbit,
                    call_id = %call.call_id,
                    parker = %call.parker_extension,
                    "Parked call expired"
                );
                expired.push(call.clone());
                false // remove
            } else {
                true // keep
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

    /// Find the next available orbit number.
    fn next_available_orbit(&self) -> Result<u16, PbxError> {
        for orbit in self.config.orbit_start..=self.config.orbit_end {
            if !self.parked.contains_key(&orbit) {
                return Ok(orbit);
            }
        }
        Err(PbxError::NoParkingOrbitsAvailable)
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
        let tenant = fcp_core::TenantId::nil();

        let orbit = mgr.park(call_id, "1001".to_string(), tenant).unwrap();
        assert_eq!(orbit, 701);
        assert_eq!(mgr.parked_count(), 1);

        let parked = mgr.retrieve(701).unwrap();
        assert_eq!(parked.call_id, call_id);
        assert_eq!(parked.parker_extension, "1001");
        assert_eq!(mgr.parked_count(), 0);
    }

    #[test]
    fn test_auto_increment_orbits() {
        let mgr = ParkingManager::new(test_config());
        let tenant = fcp_core::TenantId::nil();

        let o1 = mgr.park(CallId::new(), "1001".to_string(), tenant).unwrap();
        let o2 = mgr.park(CallId::new(), "1002".to_string(), tenant).unwrap();
        let o3 = mgr.park(CallId::new(), "1003".to_string(), tenant).unwrap();
        assert_eq!(o1, 701);
        assert_eq!(o2, 702);
        assert_eq!(o3, 703);

        // No more orbits available
        let result = mgr.park(CallId::new(), "1004".to_string(), tenant);
        assert!(result.is_err());
    }

    #[test]
    fn test_retrieve_not_occupied() {
        let mgr = ParkingManager::new(test_config());
        let result = mgr.retrieve(701);
        assert!(matches!(result, Err(PbxError::OrbitNotOccupied(701))));
    }

    #[test]
    fn test_retrieve_out_of_range() {
        let mgr = ParkingManager::new(test_config());
        let result = mgr.retrieve(999);
        assert!(matches!(result, Err(PbxError::OrbitOutOfRange(999, 701, 703))));
    }

    #[test]
    fn test_expired_calls() {
        let config = ParkingConfig {
            enabled: true,
            orbit_start: 701,
            orbit_end: 703,
            timeout_seconds: 0, // immediate expiry
        };
        let mgr = ParkingManager::new(config);
        let tenant = fcp_core::TenantId::nil();

        mgr.park(CallId::new(), "1001".to_string(), tenant).unwrap();

        // Should be expired immediately (timeout_seconds = 0)
        let expired = mgr.expired_calls();
        assert_eq!(expired.len(), 1);
        assert_eq!(mgr.parked_count(), 0);
    }

    #[test]
    fn test_orbit_reuse_after_retrieve() {
        let mgr = ParkingManager::new(test_config());
        let tenant = fcp_core::TenantId::nil();

        mgr.park(CallId::new(), "1001".to_string(), tenant).unwrap();
        mgr.retrieve(701).unwrap();

        // Orbit 701 should be available again
        let orbit = mgr.park(CallId::new(), "1002".to_string(), tenant).unwrap();
        assert_eq!(orbit, 701);
    }
}
