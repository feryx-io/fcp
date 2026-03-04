//! Shared API state.

use fcp_core::config_store::ConfigStore;
use fcp_core::DynStorage;
use fcp_events::EventBus;
use fcp_flow::FlowStore;
use fcp_media::MediaManager;
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::prelude::{DialogManager, SubscriptionManager};
use fcp_sip::CallManager;
use std::path::PathBuf;
use std::sync::Arc;

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Storage for users and registrations (optional)
    pub storage: Option<Arc<DynStorage>>,

    /// Flow store for managing call flows (optional)
    pub flow_store: Option<Arc<dyn FlowStore>>,

    /// Dialog manager for inspecting active dialogs
    pub dialog_manager: Arc<DialogManager>,

    /// Subscription manager for inspecting active subscriptions
    pub subscription_manager: Arc<SubscriptionManager>,

    /// Call manager for inspecting active B2BUA calls
    pub call_manager: Arc<CallManager>,

    /// SIP message handler (for accessing rate limits, subscription index)
    pub handler: Arc<SipMessageHandler>,

    /// Media manager for recording control (optional)
    pub media_manager: Option<Arc<MediaManager>>,

    /// Event bus for real-time event streaming
    pub event_bus: Arc<dyn EventBus>,

    /// Queue manager for ACD queue operations (optional)
    pub queue_manager: Option<Arc<fcp_queue::QueueManager>>,

    /// Presence manager for presence/BLF operations (optional)
    pub presence_manager: Option<fcp_presence::PresenceManager>,

    /// SIP realm for authentication
    pub realm: String,

    /// Default tenant ID (used when no tenant context is provided)
    pub default_tenant_id: fcp_core::TenantId,

    /// Server start time for uptime tracking
    pub start_time: std::time::Instant,

    /// URL of fcp-events-server (e.g. "http://localhost:8081") for proxying
    /// cluster-wide queries (active calls, etc.)
    pub events_server_url: Option<String>,

    /// Config store for database-backed configuration management (optional)
    pub config_store: Option<Arc<dyn ConfigStore>>,

    /// Directory for voicemail greetings (used by greeting upload/download endpoints)
    pub voicemail_greetings_dir: Option<PathBuf>,

    /// Directory for voicemail recordings (used by message audio playback)
    pub voicemail_recordings_dir: Option<PathBuf>,
}

impl ApiState {
    pub fn new(
        storage: Option<Arc<DynStorage>>,
        flow_store: Option<Arc<dyn FlowStore>>,
        dialog_manager: Arc<DialogManager>,
        subscription_manager: Arc<SubscriptionManager>,
        call_manager: Arc<CallManager>,
        handler: Arc<SipMessageHandler>,
        media_manager: Option<Arc<MediaManager>>,
        event_bus: Arc<dyn EventBus>,
        queue_manager: Option<Arc<fcp_queue::QueueManager>>,
        presence_manager: Option<fcp_presence::PresenceManager>,
        realm: String,
        default_tenant_id: fcp_core::TenantId,
    ) -> Self {
        Self {
            storage,
            flow_store,
            dialog_manager,
            subscription_manager,
            call_manager,
            handler,
            media_manager,
            event_bus,
            queue_manager,
            presence_manager,
            realm,
            default_tenant_id,
            start_time: std::time::Instant::now(),
            events_server_url: None,
            config_store: None,
            voicemail_greetings_dir: None,
            voicemail_recordings_dir: None,
        }
    }

    /// Set the events server URL for proxying cluster-wide queries.
    pub fn with_events_server_url(mut self, url: Option<String>) -> Self {
        self.events_server_url = url;
        self
    }

    /// Set the config store for database-backed configuration management.
    pub fn with_config_store(mut self, store: Arc<dyn ConfigStore>) -> Self {
        self.config_store = Some(store);
        self
    }

    /// Set the voicemail greetings directory.
    pub fn with_voicemail_greetings_dir(mut self, dir: PathBuf) -> Self {
        self.voicemail_greetings_dir = Some(dir);
        self
    }

    /// Set the voicemail recordings directory.
    pub fn with_voicemail_recordings_dir(mut self, dir: PathBuf) -> Self {
        self.voicemail_recordings_dir = Some(dir);
        self
    }
}
