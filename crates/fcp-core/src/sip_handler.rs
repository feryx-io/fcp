//! SIP INVITE handler integration.
//!
//! This module provides the integration between the SIP stack (siphon-rs) and the
//! FCP routing engine, call forking, and state management.
//!
//! Key responsibilities:
//! - Handle incoming INVITE requests
//! - Route calls using the routing engine
//! - Fork calls to multiple targets
//! - Manage call state and dialogs
//! - Apply forwarding rules
//! - Handle voicemail fallback
//! - Emergency (E911) routing

use crate::{
    call_forking::{CallForker, ForkResult},
    call_state::{ActiveCall, CallDirection, CallStateManager},
    forwarding::{ForwardType, ForwardingRulesManager},
    geographic::GeoRoutingManager,
    routing::{RoutingDestination, RoutingEngine, RoutingResult},
    storage::{CdrStore, VoicemailStore},
    CallId, TenantId,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Result type for SIP operations.
pub type SipResult<T> = Result<T, SipError>;

/// SIP operation errors.
#[derive(Debug, thiserror::Error)]
pub enum SipError {
    #[error("Routing error: {0}")]
    Routing(String),

    #[error("No targets available")]
    NoTargets,

    #[error("All targets failed")]
    AllTargetsFailed,

    #[error("Call forwarding loop detected")]
    ForwardingLoop,

    #[error("Invalid SIP URI: {0}")]
    InvalidUri(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("SIP transaction error: {0}")]
    Transaction(String),

    #[error("Dialog error: {0}")]
    Dialog(String),

    #[error("Timeout")]
    Timeout,

    #[error("Busy")]
    Busy,

    #[error("Rejected")]
    Rejected,

    #[error("Unavailable")]
    Unavailable,
}

/// SIP response codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipResponseCode {
    // 1xx - Provisional
    Trying = 100,
    Ringing = 180,
    SessionProgress = 183,

    // 2xx - Success
    Ok = 200,

    // 3xx - Redirection
    MultipleChoices = 300,
    MovedPermanently = 301,
    MovedTemporarily = 302,

    // 4xx - Client Error
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    RequestTimeout = 408,
    Gone = 410,
    TemporarilyUnavailable = 480,
    CallTransactionDoesNotExist = 481,
    LoopDetected = 482,
    TooManyHops = 483,
    BusyHere = 486,

    // 5xx - Server Error
    InternalServerError = 500,
    NotImplemented = 501,
    ServiceUnavailable = 503,

    // 6xx - Global Failure
    BusyEverywhere = 600,
    Decline = 603,
}

impl SipResponseCode {
    pub fn as_u16(&self) -> u16 {
        *self as u16
    }

    pub fn reason_phrase(&self) -> &'static str {
        match self {
            SipResponseCode::Trying => "Trying",
            SipResponseCode::Ringing => "Ringing",
            SipResponseCode::SessionProgress => "Session Progress",
            SipResponseCode::Ok => "OK",
            SipResponseCode::MultipleChoices => "Multiple Choices",
            SipResponseCode::MovedPermanently => "Moved Permanently",
            SipResponseCode::MovedTemporarily => "Moved Temporarily",
            SipResponseCode::BadRequest => "Bad Request",
            SipResponseCode::Unauthorized => "Unauthorized",
            SipResponseCode::Forbidden => "Forbidden",
            SipResponseCode::NotFound => "Not Found",
            SipResponseCode::RequestTimeout => "Request Timeout",
            SipResponseCode::Gone => "Gone",
            SipResponseCode::TemporarilyUnavailable => "Temporarily Unavailable",
            SipResponseCode::CallTransactionDoesNotExist => "Call/Transaction Does Not Exist",
            SipResponseCode::LoopDetected => "Loop Detected",
            SipResponseCode::TooManyHops => "Too Many Hops",
            SipResponseCode::BusyHere => "Busy Here",
            SipResponseCode::InternalServerError => "Internal Server Error",
            SipResponseCode::NotImplemented => "Not Implemented",
            SipResponseCode::ServiceUnavailable => "Service Unavailable",
            SipResponseCode::BusyEverywhere => "Busy Everywhere",
            SipResponseCode::Decline => "Decline",
        }
    }
}

/// SIP INVITE request context.
#[derive(Debug, Clone)]
pub struct InviteContext {
    /// Call ID from SIP
    pub sip_call_id: String,

    /// From URI
    pub from_uri: String,

    /// To URI (destination)
    pub to_uri: String,

    /// Request URI
    pub request_uri: String,

    /// Caller phone number (extracted from From)
    pub caller: String,

    /// Called phone number (extracted from To/R-URI)
    pub called: String,

    /// SDP offer (if present)
    pub sdp_offer: Option<String>,

    /// Max-Forwards header value
    pub max_forwards: u8,

    /// Contact URI from caller
    pub contact: Option<String>,

    /// User-Agent header
    pub user_agent: Option<String>,
}

impl InviteContext {
    /// Creates a new INVITE context.
    pub fn new(sip_call_id: String, from_uri: String, to_uri: String, request_uri: String) -> Self {
        Self {
            sip_call_id,
            from_uri: from_uri.clone(),
            to_uri: to_uri.clone(),
            request_uri: request_uri.clone(),
            caller: extract_phone_number(&from_uri),
            called: extract_phone_number(&request_uri),
            sdp_offer: None,
            max_forwards: 70,
            contact: None,
            user_agent: None,
        }
    }

    /// Decrements Max-Forwards.
    pub fn decrement_max_forwards(&mut self) -> Result<(), SipError> {
        if self.max_forwards == 0 {
            return Err(SipError::Transaction("Max-Forwards reached 0".to_string()));
        }
        self.max_forwards -= 1;
        Ok(())
    }
}

/// Extracts phone number from SIP URI.
fn extract_phone_number(uri: &str) -> String {
    // Strip angle brackets first
    let uri = uri.trim_start_matches('<').trim_end_matches('>');

    // Strip sip: prefix
    let uri = uri.trim_start_matches("sip:").trim_start_matches("sips:");

    // Extract user part (before @)
    if let Some(at_pos) = uri.find('@') {
        uri[..at_pos].to_string()
    } else {
        uri.to_string()
    }
}

/// SIP INVITE handler.
pub struct InviteHandler {
    routing_engine: Arc<dyn RoutingEngine>,
    call_forker: Arc<CallForker>,
    call_state_manager: Arc<CallStateManager>,
    forwarding_rules_manager: Arc<tokio::sync::RwLock<ForwardingRulesManager>>,
    #[allow(dead_code)] // TODO: Implement geographic routing for emergency services
    geo_routing_manager: Arc<tokio::sync::RwLock<GeoRoutingManager>>,
    voicemail_store: Option<Arc<dyn VoicemailStore>>,
    cdr_store: Option<Arc<dyn CdrStore>>,
}

impl InviteHandler {
    /// Creates a new INVITE handler.
    pub fn new(
        routing_engine: Arc<dyn RoutingEngine>,
        call_forker: Arc<CallForker>,
        call_state_manager: Arc<CallStateManager>,
        forwarding_rules_manager: Arc<tokio::sync::RwLock<ForwardingRulesManager>>,
        geo_routing_manager: Arc<tokio::sync::RwLock<GeoRoutingManager>>,
    ) -> Self {
        Self {
            routing_engine,
            call_forker,
            call_state_manager,
            forwarding_rules_manager,
            geo_routing_manager,
            voicemail_store: None,
            cdr_store: None,
        }
    }

    /// Sets the voicemail store.
    pub fn with_voicemail_store(mut self, store: Arc<dyn VoicemailStore>) -> Self {
        self.voicemail_store = Some(store);
        self
    }

    /// Sets the CDR store.
    pub fn with_cdr_store(mut self, store: Arc<dyn CdrStore>) -> Self {
        self.cdr_store = Some(store);
        self
    }

    /// Handles an incoming INVITE request.
    ///
    /// This is the main entry point for call processing.
    pub async fn handle_invite(
        &self,
        ctx: InviteContext,
        tenant_id: TenantId,
    ) -> SipResult<InviteResponse> {
        info!(
            "Handling INVITE from {} to {} for tenant {}",
            ctx.caller, ctx.called, tenant_id
        );

        // Create internal call ID
        let call_id = CallId::new();

        // Determine routing destination
        let destination = self.determine_destination(&ctx.called)?;

        // Route the call
        let routing_result = self
            .routing_engine
            .route_inbound_call(&destination, &tenant_id, &ctx.caller)
            .await
            .map_err(|e| SipError::Routing(e.to_string()))?;

        // Check if call should be forwarded
        if let Some(forward_to) = routing_result.forward_to {
            info!("Call forwarded unconditionally to {}", forward_to);
            return self.handle_forward(ctx, tenant_id, &forward_to).await;
        }

        // Create active call
        let mut call = ActiveCall::new(
            call_id,
            tenant_id,
            CallDirection::Inbound,
            ctx.from_uri.clone(),
            ctx.to_uri.clone(),
        );

        call.phone_number_id = routing_result.phone_number.as_ref().map(|pn| pn.id);
        call.line_binding_id = routing_result.line_binding.as_ref().map(|lb| lb.id);

        // Register call in state manager
        self.call_state_manager.register_call(call.clone()).await;

        // Check if targets are available
        if routing_result.targets.is_empty() {
            warn!("No targets available for call {}", call_id);
            return self.handle_no_targets(&call, &ctx, &routing_result).await;
        }

        // Build ring plan
        let ring_plan = self.build_ring_plan(&routing_result);

        // Fork the call
        info!(
            "Forking call {} to {} targets with strategy {:?}",
            call_id,
            routing_result.targets.len(),
            routing_result.ring_strategy
        );

        let fork_result = self
            .call_forker
            .fork_call(&mut call, &ring_plan, |target| async move {
                // Fork to target (this would use siphon-rs in production)
                // In production, this would:
                // 1. Create a new INVITE to target.sip_uri
                // 2. Send via siphon-rs transaction layer
                // 3. Wait for response (180, 183, 200)
                // 4. Return dialog ID on success

                tokio::time::sleep(Duration::from_millis(100)).await;

                // Simulate answer based on device_id hash
                let hash = target.device_id.to_string().bytes().sum::<u8>();
                if hash % 2 == 0 {
                    Ok(format!("dialog-{}", target.device_id))
                } else {
                    Err("No answer".to_string())
                }
            })
            .await;

        // Handle fork result
        match fork_result {
            ForkResult::Answered {
                device_id,
                dialog_id,
                fork_index,
            } => {
                info!(
                    "Call {} answered by device {} (fork {})",
                    call_id, device_id, fork_index
                );

                Ok(InviteResponse::Accepted {
                    call_id,
                    dialog_id,
                    contact: format!("sip:{}@fcp.local", device_id),
                    sdp_answer: None, // Would come from the device
                })
            }

            ForkResult::Failed { device_id, reason } => {
                warn!(
                    "Call {} failed on device {}: {}",
                    call_id, device_id, reason
                );
                self.handle_no_answer(&call, &ctx, &routing_result).await
            }

            ForkResult::AllFailed => {
                warn!("All forks failed for call {}", call_id);
                self.handle_no_answer(&call, &ctx, &routing_result).await
            }
        }
    }

    /// Determines the routing destination from the called number.
    fn determine_destination(&self, called: &str) -> SipResult<RoutingDestination> {
        // Check if it's an E.164 number (starts with +)
        if called.starts_with('+') {
            return Ok(RoutingDestination::E164(called.to_string()));
        }

        // Check if it's a numeric extension
        if called.chars().all(|c| c.is_ascii_digit()) && called.len() <= 6 {
            return Ok(RoutingDestination::Extension(called.to_string()));
        }

        // Otherwise treat as SIP URI
        Ok(RoutingDestination::SipUri(called.to_string()))
    }

    /// Builds a ring plan from routing result.
    fn build_ring_plan(&self, result: &RoutingResult) -> crate::routing::RingPlan {
        use crate::routing::RingGroup;
        use crate::RingStrategy;

        let timeout = result.no_answer_timeout;

        match result.ring_strategy {
            RingStrategy::Simultaneous => crate::routing::RingPlan {
                groups: vec![RingGroup {
                    targets: result.targets.clone(),
                    timeout,
                }],
                strategy: RingStrategy::Simultaneous,
                total_timeout: timeout,
            },

            RingStrategy::Sequential => {
                let target_count = result.targets.len();
                crate::routing::RingPlan {
                    groups: result
                        .targets
                        .iter()
                        .map(|target| RingGroup {
                            targets: vec![target.clone()],
                            timeout: Duration::from_secs(30),
                        })
                        .collect(),
                    strategy: RingStrategy::Sequential,
                    total_timeout: Duration::from_secs(30) * target_count as u32,
                }
            }

            RingStrategy::RoundRobin | RingStrategy::LongestIdle | RingStrategy::Primary => {
                // For these strategies, call_forker will handle device selection
                // For now, treat as simultaneous
                crate::routing::RingPlan {
                    groups: vec![RingGroup {
                        targets: result.targets.clone(),
                        timeout,
                    }],
                    strategy: result.ring_strategy,
                    total_timeout: timeout,
                }
            }
        }
    }

    /// Handles call when no targets are available.
    async fn handle_no_targets(
        &self,
        call: &ActiveCall,
        ctx: &InviteContext,
        result: &RoutingResult,
    ) -> SipResult<InviteResponse> {
        // Check for unavailable forwarding
        if let Some(ref line_binding) = result.line_binding {
            let forwarding_rules = self.forwarding_rules_manager.read().await;
            let current_time = SystemTime::now();

            if let Some(rule) = forwarding_rules.find_applicable_rule(
                &line_binding.id,
                ForwardType::Unavailable,
                &ctx.caller,
                current_time,
            ) {
                let destination = rule.destination.to_string();
                info!(
                    "No targets available, forwarding to {} (unavailable rule)",
                    destination
                );
                drop(forwarding_rules);
                return self
                    .handle_forward(ctx.clone(), call.tenant_id, &destination)
                    .await;
            }
        }

        // Check for voicemail
        if let Some(ref line_binding) = result.line_binding {
            if let Some(ref voicemail_store) = self.voicemail_store {
                if let Ok(Some(_mailbox)) = voicemail_store
                    .get_mailbox_by_line_binding(&line_binding.id)
                    .await
                {
                    info!("No targets available, routing to voicemail");
                    return Ok(InviteResponse::Voicemail {
                        call_id: call.call_id,
                        greeting_uri: "sip:voicemail@fcp.local".to_string(),
                    });
                }
            }
        }

        // No forwarding or voicemail available
        warn!("No targets, forwarding, or voicemail available");
        Ok(InviteResponse::Rejected {
            call_id: call.call_id,
            status_code: SipResponseCode::TemporarilyUnavailable,
            reason: "No devices available".to_string(),
        })
    }

    /// Handles call when no answer after timeout.
    async fn handle_no_answer(
        &self,
        call: &ActiveCall,
        ctx: &InviteContext,
        result: &RoutingResult,
    ) -> SipResult<InviteResponse> {
        // Check for no-answer forwarding
        if let Some(ref line_binding) = result.line_binding {
            let forwarding_rules = self.forwarding_rules_manager.read().await;
            let current_time = SystemTime::now();

            if let Some(rule) = forwarding_rules.find_applicable_rule(
                &line_binding.id,
                ForwardType::NoAnswer,
                &ctx.caller,
                current_time,
            ) {
                let destination = rule.destination.to_string();
                info!("No answer, forwarding to {} (no-answer rule)", destination);
                drop(forwarding_rules);
                return self
                    .handle_forward(ctx.clone(), call.tenant_id, &destination)
                    .await;
            }
        }

        // Check for voicemail
        if let Some(ref line_binding) = result.line_binding {
            if let Some(ref voicemail_store) = self.voicemail_store {
                if let Ok(Some(_mailbox)) = voicemail_store
                    .get_mailbox_by_line_binding(&line_binding.id)
                    .await
                {
                    info!("No answer, routing to voicemail");
                    return Ok(InviteResponse::Voicemail {
                        call_id: call.call_id,
                        greeting_uri: "sip:voicemail@fcp.local".to_string(),
                    });
                }
            }
        }

        // No forwarding or voicemail available
        warn!("No answer and no voicemail configured");
        Ok(InviteResponse::Rejected {
            call_id: call.call_id,
            status_code: SipResponseCode::RequestTimeout,
            reason: "No answer".to_string(),
        })
    }

    /// Handles call forwarding.
    async fn handle_forward(
        &self,
        mut ctx: InviteContext,
        _tenant_id: TenantId,
        forward_to: &str,
    ) -> SipResult<InviteResponse> {
        // Check for forwarding loops
        ctx.decrement_max_forwards()?;

        if ctx.max_forwards < 10 {
            warn!(
                "Potential forwarding loop detected (Max-Forwards: {})",
                ctx.max_forwards
            );
        }

        // Update called number to forwarding destination
        ctx.called = forward_to.to_string();

        // Recursively handle the forwarded call
        // In production, this would create a new INVITE
        info!("Forwarding call to {}", forward_to);

        Ok(InviteResponse::Redirect {
            call_id: CallId::new(),
            redirect_uri: format!("sip:{}@fcp.local", forward_to),
        })
    }
}

/// Response to an INVITE request.
#[derive(Debug)]
pub enum InviteResponse {
    /// Call was accepted (200 OK)
    Accepted {
        call_id: CallId,
        dialog_id: String,
        contact: String,
        sdp_answer: Option<String>,
    },

    /// Call was rejected with an error code
    Rejected {
        call_id: CallId,
        status_code: SipResponseCode,
        reason: String,
    },

    /// Call is ringing (180 Ringing)
    Ringing { call_id: CallId, contact: String },

    /// Early media (183 Session Progress)
    EarlyMedia { call_id: CallId, sdp_answer: String },

    /// Call forwarded/redirected (3xx)
    Redirect {
        call_id: CallId,
        redirect_uri: String,
    },

    /// Routed to voicemail
    Voicemail {
        call_id: CallId,
        greeting_uri: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_phone_number() {
        assert_eq!(
            extract_phone_number("sip:+15551234567@example.com"),
            "+15551234567"
        );
        assert_eq!(extract_phone_number("+15551234567"), "+15551234567");
        assert_eq!(extract_phone_number("sip:alice@example.com"), "alice");
        assert_eq!(extract_phone_number("<sip:1234@example.com>"), "1234");
    }

    #[test]
    fn test_sip_response_codes() {
        assert_eq!(SipResponseCode::Ok.as_u16(), 200);
        assert_eq!(SipResponseCode::Ringing.as_u16(), 180);
        assert_eq!(SipResponseCode::BusyHere.as_u16(), 486);
        assert_eq!(SipResponseCode::Ok.reason_phrase(), "OK");
    }

    #[test]
    fn test_invite_context_max_forwards() {
        let mut ctx = InviteContext::new(
            "call-123".to_string(),
            "sip:alice@example.com".to_string(),
            "sip:bob@example.com".to_string(),
            "sip:bob@example.com".to_string(),
        );

        assert_eq!(ctx.max_forwards, 70);
        ctx.decrement_max_forwards().unwrap();
        assert_eq!(ctx.max_forwards, 69);
    }
}
