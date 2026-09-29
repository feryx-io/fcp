//! Core primitives shared across the FCP project.
//!
//! This crate holds common error types, identifiers, and configuration
//! structures that other crates depend on.

pub mod billing;
pub mod cached_storage;
pub mod calling;
pub mod cdr_archive;
pub mod class_of_service;
pub mod codec;
pub mod compliance;
pub mod config;
pub mod config_store;
pub mod config_template;
pub mod device;
pub mod e911;
pub mod encryption;
pub mod error;
pub mod extension;
pub mod flow_trace;
pub mod geographic;
pub mod hoteling;
pub mod identifiers;
pub mod join_token;
pub mod lcr_route;
pub mod legal_hold;
pub mod line_binding;
pub mod meeting;
pub mod metrics;
pub mod migration_ticket;
pub mod permissions;
pub mod phone_number;
pub mod pidf_lo;
pub mod porting;
pub mod prompt;
pub mod queue_config;
pub mod recording;
pub mod region;
pub mod routing;
pub mod routing_rule;
pub mod sca;
pub mod schedule;
pub mod secret;
pub mod service;
pub mod sip_trace;
pub mod site;
pub mod site_location;
pub mod storage;
pub mod subject;
pub mod subject_erasure;
pub mod subject_request;
pub mod sync;
pub mod tenant;
pub mod time;
pub mod transport;
pub mod trunk;
pub mod trunk_report;
pub mod voicemail;

// Re-export commonly used types
pub use crate::cached_storage::{
    CacheStats, CachedCredentialStore, CachedDeviceStore, CachedExtensionStore,
};
pub use crate::calling::{
    Assistant, CallTarget, CallTargetError, CallingSettings, CallingStore, Executive,
    ExecutiveAlert, ExecutiveFilter, FindMe, FindMeStep, FindMeTarget, Treatment, Treatments,
};
pub use crate::codec::{AudioCodec, Codec, CodecPreferences, VideoCodec};
pub use crate::config::{
    ApiConfig, ApiRateLimitConfig, FcpConfig, LogFormat, NodeConfig, ObservabilityConfig,
    RegistrarConfig, StorageBackend, StorageConfig, TransportConfig,
};
pub use crate::config_store::{
    get_config_with_tenant_fallback, load_or_seed_config, spawn_config_poll_loop, AuditEntry,
    ConfigEntry, ConfigStore, ConfigWatcher,
};
pub use crate::device::{Device, DeviceType, RegistrationState};
pub use crate::encryption::{EncryptionStatus, EncryptionStore, ReencryptReport};
pub use crate::error::{
    DialogError, Error, MediaError, ParseError, Result, StorageError, TransactionError,
    TransportError,
};
pub use crate::extension::{Extension, ExtensionTarget};
pub use crate::geographic::{
    Address, Coordinates, GeoRoutingManager, GeoRoutingRule, GeoRoutingStrategy, Location,
    PhoneNumberLocation, Region, RegionDestination,
};
pub use crate::identifiers::*;
pub use crate::join_token::{JoinRole, JoinToken, JoinTokenError, JOIN_TOKEN_HEADER};
pub use crate::storage::CallDirection;

/// The header a call manager's INVITE to a conference node, and a WebRTC
/// gateway's, carries naming its own call or session, so the conference
/// leg's record shares that call's id (`docs/CDR_PIPELINE.md` §5). It
/// correlates records and decides nothing else.
pub const CALL_ID_HEADER: &str = "X-FCP-Call-Id";

/// The tenant a trusted edge (the WebRTC gateway) places a call for; with
/// [`CALLER_USER_HEADER`], believed by a call manager only from the
/// addresses in its `[e911] trusted_edges` (`docs/E911_BLOCK_2.md` §4).
pub const CALLER_TENANT_HEADER: &str = "X-FCP-Caller-Tenant";
/// The user, by username, a trusted edge places a call for.
pub const CALLER_USER_HEADER: &str = "X-FCP-Caller-User";
/// The site a trusted edge (the cloud SBC) places a call for: a call
/// that came up a site's link is the site's tenant's own, placed by a
/// phone the site already authenticated (`docs/HYBRID_CONTROL_PLANE.md`
/// §3.6). Believed only from `[e911] trusted_edges`.
pub const CALLER_SITE_HEADER: &str = "X-FCP-Caller-Site";
/// The address a trusted edge (an SBC, the WebRTC gateway) saw its client
/// at, so the call manager can place the caller by network even though the
/// call reached it from the edge (`docs/E911_BLOCK_2.md` §6.2). Believed
/// only from `[e911] trusted_edges`, and never forwarded further.
pub const CLIENT_ADDRESS_HEADER: &str = "X-FCP-Client-Address";
pub use crate::class_of_service::{
    CallerIdPolicy, ClassOfService, ClassOfServiceStore, DestinationClass, DestinationOverride,
};
pub use crate::hoteling::{HotelingSession, HotelingStore, SessionEnd};
pub use crate::lcr_route::{LcrRoute, LcrRouteId};
pub use crate::line_binding::{BindingType, DeviceBinding, LineBinding, RingStrategy};
pub use crate::meeting::Meeting;
pub use crate::migration_ticket::{
    MigrationTicket, MigrationTicketError, MIGRATION_TICKET_HEADER, MIGRATION_TICKET_TTL,
};
pub use crate::permissions::{DeploymentMode, Permission, UserRole};
pub use crate::phone_number::{E911Address, NumberState, PhoneNumber, Screening};
pub use crate::porting::{
    PortRejectionReason, PortRequest, PortRequestId, PortStatus, PortType, PortingManager,
    PortingStore,
};
pub use crate::prompt::{Prompt, PromptCategory, PromptService, PromptSourceType, PromptStore};
pub use crate::queue_config::{QueueConfigId, QueueConfigRecord};
pub use crate::recording::{
    CallRecording, CallRecordingSource, CallRecordingStore, Recording, RecordingConsent,
    RecordingFilter, RecordingFormat, RecordingKind, RecordingParticipant, RecordingStore,
    RecordingTrigger,
};
pub use crate::routing::{
    RingGroup, RingPlan, RoutingDestination, RoutingEngine, RoutingError, RoutingResult,
    RoutingTarget,
};
pub use crate::routing_rule::{RoutingAction, RoutingRule, RoutingRuleId, TrunkTarget};
pub use crate::schedule::{Holiday, Schedule, ScheduleState, ScheduleStore, WeeklyWindow};
pub use crate::service::{Service, ServiceManager};
pub use crate::sip_trace::{SipTraceMessage, SipTraceStore};
pub use crate::site::{
    EnrollmentToken, ShippedAnswer, ShippedDeletion, ShippedEvents, ShippedRecord, ShippedRecords,
    Site, SiteHeartbeat, SiteNode, SiteRegistration, SiteRegistrations, SiteRequest,
    SiteRequestKind, SiteRequestOutcome, SiteStatus, SiteStore, SiteSync, SITE_REGISTRATIONS_CAP,
};
pub use crate::site_location::SiteLocation;
pub use crate::storage::{
    authenticate_api_token, room_number_exists_globally, AccountStatus, ApiToken, ApiTokenScope,
    ApiTokenStore, AuthenticatedApiToken, AvailableNumberQuery, CallDetailRecord, CallDisposition,
    CdrQuery, CdrStore, ClusterParkedCall, ClusterParkingStore, ConferenceStore,
    ConfigTemplateStore, CredentialStore, DynStorage, EmergencyContact, FileBackedStorage,
    HangupCause, HangupParty, MediaPreferences, NumberUsage, PresenceStore, QueueConfigStore,
    QueueStore, Registration, RegistrationStore, RoutingRuleStore, SiteLocationStore, StoredAgent,
    StoredAuthPolicy, StoredPresentity, StoredSubscription, TenantStore, TrunkStore,
    UsageAggregation, User, UserFileEntry, WebhookDeliveryLog, WebhookEndpoint, WebhookStore,
};
pub use crate::sync::{JournalEntry, SyncOp, SyncStore, SYNC_TABLES};
pub use crate::tenant::{is_host_literal, Tenant, TenantCalling, TenantStatus};
pub use crate::transport::{
    ConnectionId, ConnectionMetadata, ConnectionState, Transport, TransportMessage,
    TransportReceiver, TransportType,
};
pub use crate::trunk::{LinkEnd, Trunk, TrunkDirection, TrunkId, TrunkStats};
pub use crate::voicemail::{
    GreetingType, MessagePriority, MessageStatus, MwiNotifier, MwiStatus, VoicemailGreeting,
    VoicemailMailbox, VoicemailMessage, VoicemailStats,
};
