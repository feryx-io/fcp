//! Core primitives shared across the FCP project.
//!
//! This crate holds common error types, identifiers, and configuration
//! structures that other crates depend on.

pub mod call_forking;
pub mod call_state;
pub mod codec;
pub mod config;
pub mod device;
pub mod error;
pub mod extension;
pub mod forwarding;
pub mod geographic;
pub mod identifiers;
pub mod meeting;
pub mod line_binding;
pub mod permissions;
pub mod phone_number;
pub mod porting;
pub mod routing;
pub mod routing_engine;
pub mod service;
pub mod sip_handler;
pub mod storage;
pub mod tenant;
pub mod time;
pub mod transport;
pub mod voicemail;

// Re-export commonly used types
pub use crate::call_forking::{CallForker, ForkResult, ForkStrategy};
pub use crate::call_state::{
    ActiveCall, CallDirection, CallFork, CallState, CallStateManager, CallStats, DeviceState,
    DeviceStatus,
};
pub use crate::codec::{AudioCodec, Codec, CodecPreferences, VideoCodec};
pub use crate::config::{
    ApiConfig, FcpConfig, LogFormat, NodeConfig, ObservabilityConfig, RegistrarConfig,
    StorageBackend, StorageConfig, TransportConfig,
};
pub use crate::device::{Device, DeviceType, RegistrationState};
pub use crate::error::{
    DialogError, Error, MediaError, ParseError, Result, StorageError, TransactionError,
    TransportError,
};
pub use crate::extension::{Extension, ExtensionTarget};
pub use crate::forwarding::{
    ForwardDestination, ForwardType, ForwardingRule, ForwardingRulesManager, TimeRange,
};
pub use crate::geographic::{
    Address, Coordinates, GeoRoutingManager, GeoRoutingRule, GeoRoutingStrategy, Location,
    PhoneNumberLocation, Region, RegionDestination,
};
pub use crate::identifiers::*;
pub use crate::line_binding::{BindingType, DeviceBinding, LineBinding, RingStrategy};
pub use crate::meeting::Meeting;
pub use crate::permissions::{DeploymentMode, Permission, UserRole};
pub use crate::phone_number::{E911Address, NumberState, PhoneNumber};
pub use crate::porting::{
    PortRejectionReason, PortRequest, PortRequestId, PortStatus, PortType, PortingManager,
    PortingStore,
};
pub use crate::routing::{
    RingGroup, RingPlan, RoutingDestination, RoutingEngine, RoutingError, RoutingResult,
    RoutingTarget,
};
pub use crate::routing_engine::DefaultRoutingEngine;
pub use crate::service::{Service, ServiceManager};
pub use crate::sip_handler::{
    InviteContext, InviteHandler, InviteResponse, SipError, SipResponseCode, SipResult,
};
pub use crate::storage::{
    AccountStatus, CallDetailRecord, CallDisposition, CdrQuery, CdrStore, ConferenceStore,
    CredentialStore, DynStorage, EmergencyContact, FileBackedStorage, HangupCause, HangupParty,
    MediaPreferences, Registration, RegistrationStore, TenantStore, User, UserFileEntry,
};
pub use crate::tenant::{Tenant, TenantStatus};
pub use crate::transport::{
    ConnectionId, ConnectionMetadata, ConnectionState, Transport, TransportMessage,
    TransportReceiver, TransportType,
};
pub use crate::voicemail::{
    GreetingType, MessagePriority, MessageStatus, MwiNotifier, MwiStatus, VoicemailGreeting,
    VoicemailMailbox, VoicemailMessage, VoicemailStats,
};
