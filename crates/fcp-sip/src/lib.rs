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
pub mod exports;
pub mod handler;
pub mod listener;
pub mod registrar;
pub mod service;
pub mod transport;
pub mod uac;

// Re-export commonly used types for convenience
pub use exports::*;

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::adapters::*;
    pub use crate::exports::*;
    pub use crate::handler::SipMessageHandler;
    pub use crate::listener::{InboundMessageHandler, ListenerConfig, TransportListener};
    pub use crate::service::*;
    pub use crate::transport::{
        to_fcp_transport_type, to_sip_transport_kind, ConnectionPool, RealTransportDispatcher,
    };
    pub use crate::registrar::{CredentialStoreAdapter, FcpRegistrar, LocationStoreAdapter};
    pub use crate::uac::{OutboundCall, SipUserAgentClient};
}
