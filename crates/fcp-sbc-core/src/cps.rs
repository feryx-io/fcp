//! CPS (calls-per-second) checking trait for trunk routing.
//!
//! Decouples trunk routing from SBC-specific admission control so that
//! `TrunkRouter` can work with different CPS implementations:
//! - SBC uses `AdmissionController` (full CPS + concurrent call tracking)
//! - Call-manager/session-manager uses `NoCpsLimit` (no CPS checking)

/// Trait for per-trunk calls-per-second checking.
pub trait CpsChecker: Send + Sync {
    /// Returns true if the trunk is within its CPS limit.
    fn is_trunk_within_cps(&self, trunk_name: &str) -> bool;

    /// Record a call attempt for CPS tracking.
    fn record_trunk_call(&self, trunk_name: &str);
}

/// No-op CPS checker that always allows calls.
pub struct NoCpsLimit;

impl CpsChecker for NoCpsLimit {
    fn is_trunk_within_cps(&self, _trunk_name: &str) -> bool {
        true
    }

    fn record_trunk_call(&self, _trunk_name: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cps_limit_always_allows() {
        let checker = NoCpsLimit;
        assert!(checker.is_trunk_within_cps("any-trunk"));
        // record_trunk_call is a no-op — just verify it doesn't panic
        checker.record_trunk_call("any-trunk");
    }
}
