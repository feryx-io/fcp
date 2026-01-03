//! Re-exports of key siphon-rs types for convenience.
//!
//! This module makes siphon-rs types available through fcp-sip without
//! requiring direct dependencies on siphon-rs crates.

// Core SIP types
pub use sip_core::{
    Header, Headers, Method, Request, RequestLine, Response, SipMessage, SipUri, StatusLine,
    TelUri, Uri,
};

// Parser
pub use sip_parse::{parse_request, parse_response};

// Transaction layer
pub use sip_transaction::{
    ClientTransaction, ClientTransactionUser, ServerTransaction, TransactionKey,
    TransactionManager, TransportContext, TransportDispatcher,
    TransportKind as SipTransactionTransportKind,
};

// Dialog layer
pub use sip_dialog::{
    Dialog, DialogId, DialogManager, Subscription, SubscriptionId, SubscriptionManager,
    SubscriptionState,
};

// Authentication
pub use sip_auth::{
    CredentialStore, DigestAlgorithm, DigestAuthenticator, DigestClient, MemoryCredentialStore,
    NonceManager,
};

// Registrar
pub use sip_registrar::{BasicRegistrar, Binding, LocationStore, MemoryLocationStore};

// UAC/UAS helpers
pub use sip_uac::integrated::IntegratedUAC;
pub use sip_uac::UserAgentClient;
pub use sip_uas::UserAgentServer;

// SDP support
pub use sip_sdp::profiles::MediaProfileBuilder;
pub use sip_sdp::{serialize, SessionDescription};

// Transport layer types
pub use sip_transport::TransportKind as SipTransportKind;

// DNS resolver
pub use sip_dns::SipResolver;
