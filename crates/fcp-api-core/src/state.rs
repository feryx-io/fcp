//! Shared API state.

use fcp_core::DynStorage;
use fcp_events::EventBus;
use fcp_flow::FlowStore;
use fcp_media::MediaManager;
use fcp_sip::CallManager;
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::prelude::{DialogManager, SubscriptionManager};
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

    /// SIP realm for authentication
    pub realm: String,

    /// Default tenant ID (used when no tenant context is provided)
    pub default_tenant_id: fcp_core::TenantId,

    /// Server start time for uptime tracking
    pub start_time: std::time::Instant,
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
            realm,
            default_tenant_id,
            start_time: std::time::Instant::now(),
        }
    }
}
