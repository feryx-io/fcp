//! Integration layer between fcp-core and siphon-rs SIP stack.
//!
//! This crate bridges the FCP platform abstractions (fcp-core) with the
//! siphon-rs SIP stack implementation. It provides:
//!
//! - Re-exports of key siphon-rs types for convenience
//! - Adapters that convert between fcp-core and siphon-rs types
//! - Transport implementations using fcp-core::Transport trait
//! - Service implementations using fcp-core::Service trait

pub mod adapters;
pub mod admission;
pub mod auth;
pub mod claims;
pub use fcp_b2bua_core::body;
pub mod conference;
pub mod conference_migration;
pub mod config;
pub mod drain;
pub mod emergency;
pub mod exports;
pub mod flow;
pub mod flow_handler;
pub mod handler;
pub mod listener;
pub mod metrics;
pub mod mwi;
pub mod presence;
pub mod presence_checker;
pub mod queue;
pub mod recording;
pub mod registrar;
pub mod service;
pub mod settings;
pub mod transport;
pub mod trunk_egress;
pub mod trunk_ingress;
pub mod uac;

// Re-export shared config validation helper
pub use config::validate_sip_binds;
#[cfg(feature = "tls")]
pub use sip_transport::ClientAuthMode;
pub use sip_transport::PeerIdentity;

// Re-export B2BUA types from fcp-b2bua-core
pub use fcp_b2bua_core::{
    B2buaConfig, Call, CallDirection, CallId, CallLeg, CallManager, CallState, CallStats,
};

// Re-export the CDR pipeline's types the binaries wire up
// (`docs/CDR_PIPELINE.md`).
pub use fcp_cdr::{CdrWriter, RtpStats, SystemCause, TerminationReason};

// Re-export commonly used types for convenience
pub use exports::*;

// Re-export outbound route provider trait and types
pub use handler::{
    ConferenceNodeSelector, ConferenceVideoNeed, OutboundRoute, OutboundRouteProvider,
    RouteResolution,
};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::adapters::*;
    pub use crate::exports::*;
    pub use crate::flow::{FlowInfo, FlowRegistry, FlowToken};
    pub use crate::handler::SipMessageHandler;
    pub use crate::listener::{InboundMessageHandler, ListenerConfig, TransportListener};
    pub use crate::registrar::{CredentialStoreAdapter, FcpRegistrar, LocationStoreAdapter};
    pub use crate::service::*;
    pub use crate::transport::{
        to_fcp_transport_type, to_sip_transport_kind, ConnectionPool, RealTransportDispatcher,
    };
    pub use crate::uac::{OutboundCall, SipUserAgentClient};

    // B2BUA types from fcp-b2bua-core
    pub use fcp_b2bua_core::{
        Call, CallDirection, CallId, CallLeg, CallManager, CallState, CallStats,
    };
}
