//! PBX error types.

use thiserror::Error;

/// Errors returned by PBX operations.
#[derive(Debug, Error)]
pub enum PbxError {
    /// No parking orbits available in the configured range.
    #[error("no parking orbits available")]
    NoParkingOrbitsAvailable,

    /// The requested parking orbit is not occupied.
    #[error("orbit {0} is not occupied")]
    OrbitNotOccupied(u16),

    /// The parking orbit is out of the configured range.
    #[error("orbit {0} is out of range ({1}-{2})")]
    OrbitOutOfRange(u16, u16, u16),

    /// The requested hunt group was not found.
    #[error("hunt group '{0}' not found")]
    HuntGroupNotFound(String),

    /// FAC code validation failed.
    #[error("invalid authorization code")]
    InvalidAuthorizationCode,
}
