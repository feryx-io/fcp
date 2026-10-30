//! PBX error types.

use thiserror::Error;

/// Errors returned by PBX operations.
#[derive(Debug, Error)]
pub enum PbxError {
    /// Every slot of the lot holds a call.
    #[error("no parking slot free")]
    NoParkingOrbitsAvailable,

    /// No call is parked in the slot.
    #[error("slot {0} is not occupied")]
    OrbitNotOccupied(String),

    /// The slot already holds a call.
    #[error("slot {0} is taken")]
    SlotTaken(String),

    /// The requested hunt group was not found.
    #[error("hunt group '{0}' not found")]
    HuntGroupNotFound(String),

    /// FAC code validation failed.
    #[error("invalid authorization code")]
    InvalidAuthorizationCode,
}
