//! PBX feature library for the Ferrous Communications Platform.
//!
//! This crate is a **decision library** — it returns routing instructions and
//! metadata but never touches SIP messages directly. The SipMessageHandler
//! queries it ("is this a feature code?", "is this a hunt group?") and then
//! executes the actual SIP operations itself.
//!
//! # Features
//!
//! - **Feature codes**: Star-code parsing (`*8` pickup, `*70` park, etc.)
//! - **Hunt groups**: Ring strategy resolution (simultaneous, sequential, round-robin, random)
//! - **Call parking**: Orbit management with automatic timeout
//! - **Pickup groups**: Extension-to-group mapping for call pickup
//! - **Intercom**: Auto-answer header generation
//! - **CMC/FAC**: Client Matter Code and Forced Authorization Code validation

pub mod codes;
pub mod config;
pub mod error;
pub mod feature_codes;
pub mod hunt_group;
pub mod intercom;
pub mod parking;
pub mod pickup;

pub use codes::AuthorizationRequirement;
pub use config::PbxConfig;
pub use error::PbxError;
pub use feature_codes::FeatureCodeAction;
pub use hunt_group::{HuntGroup, HuntGroupPlan};
pub use intercom::{IntercomMode, IntercomParams};
pub use parking::{ParkedCall, ParkingManager};
pub use pickup::PickupGroups;

use fcp_b2bua_core::call::{CallId, CallState};
use fcp_b2bua_core::CallManager;
use std::collections::HashMap;
use tracing::debug;

/// Central PBX handler that coordinates all PBX features.
///
/// Created once from configuration and shared (via `Arc`) across all
/// call-handling tasks. Thread-safe and lock-free for hot-path operations.
pub struct PbxHandler {
    config: PbxConfig,
    hunt_groups: HashMap<String, HuntGroup>,
    pickup_groups: PickupGroups,
    parking_manager: ParkingManager,
}

impl PbxHandler {
    /// Create a new PBX handler from configuration.
    pub fn new(config: PbxConfig) -> Self {
        // Build hunt groups from config
        let hunt_groups: HashMap<String, HuntGroup> = config
            .hunt_groups
            .iter()
            .map(|(ext, hg_config)| (ext.clone(), HuntGroup::new(ext.clone(), hg_config.clone())))
            .collect();

        // Build pickup group lookups
        let pickup_groups = PickupGroups::from_config(&config.pickup_groups);

        // Build parking manager
        let parking_manager = ParkingManager::new(config.parking.clone());

        Self {
            config,
            hunt_groups,
            pickup_groups,
            parking_manager,
        }
    }

    /// Parse a dialed string to see if it matches a configured feature code.
    pub fn parse_feature_code(&self, dialed: &str) -> Option<FeatureCodeAction> {
        feature_codes::parse(dialed, &self.config.feature_codes)
    }

    /// Resolve a hunt group for the given extension.
    ///
    /// Returns `None` if the extension is not a hunt group.
    pub fn resolve_hunt_group(&self, extension: &str) -> Option<HuntGroupPlan> {
        self.hunt_groups.get(extension).map(|hg| hg.resolve())
    }

    /// Park a call and return the assigned orbit.
    pub fn park_call(
        &self,
        call_id: CallId,
        parker_extension: String,
        tenant_id: fcp_core::TenantId,
    ) -> Result<u16, PbxError> {
        self.parking_manager
            .park(call_id, parker_extension, tenant_id)
    }

    /// Retrieve a parked call from the given orbit.
    pub fn retrieve_parked_call(&self, orbit: u16) -> Result<ParkedCall, PbxError> {
        self.parking_manager.retrieve(orbit)
    }

    /// Check whether a destination requires CMC, FAC, or both.
    pub fn check_authorization_required(&self, destination: &str) -> AuthorizationRequirement {
        codes::check_required(destination, &self.config.cmc, &self.config.fac)
    }

    /// Validate a Forced Authorization Code.
    pub fn validate_fac(&self, code: &str) -> bool {
        codes::validate_fac(code, &self.config.fac)
    }

    /// Validate a Client Matter Code.
    pub fn validate_cmc(&self, code: &str) -> bool {
        codes::validate_cmc(code, &self.config.cmc)
    }

    /// Find a ringing call targeting any member of the caller's pickup group.
    ///
    /// Returns the `CallId` of the first ringing call found, or `None`.
    pub fn find_ringing_in_group(
        &self,
        caller_extension: &str,
        call_manager: &CallManager,
    ) -> Option<CallId> {
        let group_members = self.pickup_groups.group_members_for(caller_extension)?;

        debug!(
            caller = caller_extension,
            group_size = group_members.len(),
            "Searching for ringing call in pickup group"
        );

        let active = call_manager.active_calls();
        for call in &active {
            if call.state != CallState::Proceeding {
                continue;
            }
            // Check if the B-leg targets a group member
            if let Some(callee) = call.callee() {
                let callee_user = extract_user_part(callee);
                if group_members.iter().any(|m| m == callee_user) {
                    debug!(
                        call_id = %call.id,
                        callee = callee_user,
                        "Found ringing call for pickup"
                    );
                    return Some(call.id);
                }
            }
        }

        None
    }

    /// Find a ringing call targeting a specific extension (directed pickup).
    pub fn find_ringing_for_extension(
        &self,
        target_extension: &str,
        call_manager: &CallManager,
    ) -> Option<CallId> {
        let active = call_manager.active_calls();
        for call in &active {
            if call.state != CallState::Proceeding {
                continue;
            }
            if let Some(callee) = call.callee() {
                let callee_user = extract_user_part(callee);
                if callee_user == target_extension {
                    return Some(call.id);
                }
            }
        }
        None
    }

    /// Build intercom parameters for a target extension.
    pub fn build_intercom_params(
        &self,
        target_extension: &str,
        mode: IntercomMode,
    ) -> IntercomParams {
        intercom::build_intercom_params(target_extension, mode)
    }

    /// Access the parking manager directly (e.g., for the timeout sweep task).
    pub fn parking_manager(&self) -> &ParkingManager {
        &self.parking_manager
    }

    /// Whether PBX features are enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

/// Extract the user-part from a SIP URI, name-addr, or raw extension string.
///
/// Handles:
/// - `"Alice" <sip:1001@host>` → `1001`
/// - `<sip:1001@host>` → `1001`
/// - `sip:1001@host` → `1001`
/// - `1001` → `1001`
fn extract_user_part(input: &str) -> &str {
    let input = input.trim();

    // Handle name-addr: find the URI inside angle brackets
    let uri = if let Some(start) = input.find('<') {
        if let Some(end) = input.find('>') {
            &input[start + 1..end]
        } else {
            input
        }
    } else {
        input
    };

    // Strip scheme
    let without_scheme = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);

    // Take user part before '@'
    without_scheme.split('@').next().unwrap_or(without_scheme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::*;
    use std::collections::HashMap;

    fn test_config() -> PbxConfig {
        let mut hunt_groups = HashMap::new();
        hunt_groups.insert(
            "5000".to_string(),
            HuntGroupConfig {
                name: "Sales".to_string(),
                strategy: RingStrategy::Simultaneous,
                members: vec!["1001".to_string(), "1002".to_string()],
                ring_timeout: 20,
                total_timeout: 30,
                no_answer_destination: None,
                skip_busy: false,
            },
        );

        let mut pickup_groups = HashMap::new();
        pickup_groups.insert(
            "floor1".to_string(),
            PickupGroupConfig {
                members: vec!["1001".to_string(), "1002".to_string()],
            },
        );

        PbxConfig {
            enabled: true,
            feature_codes: FeatureCodeConfig::default(),
            hunt_groups,
            parking: ParkingConfig {
                enabled: true,
                orbit_start: 701,
                orbit_end: 703,
                timeout_seconds: 60,
            },
            pickup_groups,
            cmc: CmcConfig::default(),
            fac: FacConfig::default(),
        }
    }

    #[test]
    fn test_pbx_handler_feature_codes() {
        let handler = PbxHandler::new(test_config());
        assert_eq!(
            handler.parse_feature_code("*8"),
            Some(FeatureCodeAction::GroupPickup)
        );
        assert_eq!(
            handler.parse_feature_code("*70"),
            Some(FeatureCodeAction::ParkCall)
        );
        assert_eq!(handler.parse_feature_code("1001"), None);
    }

    #[test]
    fn test_pbx_handler_hunt_group() {
        let handler = PbxHandler::new(test_config());
        let plan = handler.resolve_hunt_group("5000");
        assert!(plan.is_some());
        let plan = plan.unwrap();
        assert_eq!(plan.ring_phases.len(), 1);
        assert!(plan.ring_phases[0].simultaneous);

        assert!(handler.resolve_hunt_group("9999").is_none());
    }

    #[test]
    fn test_pbx_handler_parking() {
        let handler = PbxHandler::new(test_config());
        let call_id = CallId::new();
        let tenant = fcp_core::TenantId::nil();

        let orbit = handler
            .park_call(call_id, "1001".to_string(), tenant)
            .unwrap();
        assert_eq!(orbit, 701);

        let parked = handler.retrieve_parked_call(701).unwrap();
        assert_eq!(parked.call_id, call_id);
    }

    #[test]
    fn test_extract_user_part() {
        assert_eq!(extract_user_part("sip:1001@10.0.0.1"), "1001");
        assert_eq!(extract_user_part("<sip:1001@10.0.0.1>"), "1001");
        assert_eq!(extract_user_part("1001"), "1001");
        assert_eq!(extract_user_part("\"Alice\" <sip:1001@10.0.0.1>"), "1001");
    }
}
