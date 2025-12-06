//! Shared API state.

use fcp_core::FileBackedStorage;
use fcp_sip::prelude::{DialogManager, SubscriptionManager};
use fcp_sip::call_manager::CallManager;
use fcp_sip::handler::SipMessageHandler;
use std::sync::Arc;

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Storage for users and registrations (optional)
    pub storage: Option<Arc<FileBackedStorage>>,

    /// Dialog manager for inspecting active dialogs
    pub dialog_manager: Arc<DialogManager>,

    /// Subscription manager for inspecting active subscriptions
    pub subscription_manager: Arc<SubscriptionManager>,

    /// Call manager for inspecting active B2BUA calls
    pub call_manager: Arc<CallManager>,

    /// SIP message handler (for accessing rate limits, subscription index)
    pub handler: Arc<SipMessageHandler>,
}

impl ApiState {
    pub fn new(
        storage: Option<Arc<FileBackedStorage>>,
        dialog_manager: Arc<DialogManager>,
        subscription_manager: Arc<SubscriptionManager>,
        call_manager: Arc<CallManager>,
        handler: Arc<SipMessageHandler>,
    ) -> Self {
        Self {
            storage,
            dialog_manager,
            subscription_manager,
            call_manager,
            handler,
        }
    }
}
