//! B2BUA (Back-to-Back User Agent) core functionality.
//!
//! This crate provides the foundational B2BUA implementation used by multiple
//! FCP services (SBC, PBX, Session Manager). It handles:
//!
//! - Call state management (A-leg and B-leg)
//! - Call lifecycle (creation, establishment, termination)
//! - CDR generation
//! - Event publishing
//!
//! ## Architecture
//!
//! The B2BUA core is designed as a shared library that can be used by different
//! service types:
//!
//! - **SBC**: Uses B2BUA for topology hiding and media anchoring
//! - **PBX**: Uses B2BUA for call forking, transfers, and feature implementation
//! - **Session Manager**: Uses B2BUA for call routing and failover
//!
//! ## Key Types
//!
//! - `Call`: Represents a B2BUA call with A-leg and B-leg
//! - `CallManager`: Manages all active calls
//! - `CallState`: State machine for call progression
//! - `CallLeg`: Represents one side of a B2BUA call
//!
//! ## Example
//!
//! ```rust,no_run
//! use fcp_b2bua_core::{CallManager, Call, CallState};
//! # use std::net::SocketAddr;
//! # use sip_core::Request;
//! # use fcp_core::TenantId;
//!
//! // Create call manager
//! let manager = CallManager::new();
//!
//! // Create call from inbound INVITE
//! # let request: Request = todo!();
//! # let peer: SocketAddr = "127.0.0.1:5060".parse().unwrap();
//! let call = Call::from_invite(request, peer, TenantId::nil());
//! let call_id = manager.create_call(call);
//!
//! // Update call state
//! manager.update_call(&call_id, |call| {
//!     call.establish();
//! }).unwrap();
//!
//! // Terminate call
//! manager.terminate_call(&call_id, None).unwrap();
//! ```

pub mod body;
pub mod call;
pub mod call_manager;
pub mod forking;
pub mod media_anchoring;
pub mod session;
pub mod topology;
pub mod transfer;

// Re-export key types for convenience
pub use call::{
    Call, CallDirection, CallId, CallLeg, CallState, DtmfCarriage, DtmfWay, EmergencyCallInfo,
    EmergencyLeg, LegSide, RouteTrunk,
};
pub use call_manager::{CallManager, CallStats};
pub use forking::{EarlyMediaPolicy, ForkingConfig, ForkingMode};
pub use media_anchoring::{MediaAnchoringConfig, MediaAnchoringMode};
pub use session::{B2buaConfig, B2buaSession, SessionTimerConfig};
pub use topology::{TopologyHidingConfig, TopologyHidingMode};
pub use transfer::{TransferConfig, TransferPolicy, TransferResult, TransferType};
