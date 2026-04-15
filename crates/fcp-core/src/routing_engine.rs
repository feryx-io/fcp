//! Default implementation of the routing engine.

use crate::{
    call_state::CallStateManager,
    forwarding::{ForwardType, ForwardingRulesManager},
    routing::{RoutingDestination, RoutingEngine, RoutingError, RoutingResult, RoutingTarget},
    storage::{
        ConferenceStore, DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore,
        VoicemailStore,
    },
    ExtensionTarget, LineBinding, PhoneNumberId, RingStrategy, TenantId, UserId,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// Default routing engine implementation.
///
/// This implementation uses the storage backends to look up phone numbers,
/// line bindings, extensions, and devices to determine where calls should be routed.
/// It also integrates with the call state manager for round-robin and longest-idle strategies.
pub struct DefaultRoutingEngine {
    phone_number_store: Arc<dyn PhoneNumberStore>,
    line_binding_store: Arc<dyn LineBindingStore>,
    extension_store: Arc<dyn ExtensionStore>,
    device_store: Arc<dyn DeviceStore>,
    voicemail_store: Option<Arc<dyn VoicemailStore>>,
    conference_store: Option<Arc<dyn ConferenceStore>>,
    voicemail_service_uri: Option<String>,
    conference_service_uri: Option<String>,
    #[allow(dead_code)] // TODO: Implement round-robin and longest-idle ring strategies
    call_state_manager: Arc<CallStateManager>,
    forwarding_rules_manager: Arc<tokio::sync::RwLock<ForwardingRulesManager>>,
}

impl DefaultRoutingEngine {
    /// Creates a new routing engine with the given storage backends.
    pub fn new(
        phone_number_store: Arc<dyn PhoneNumberStore>,
        line_binding_store: Arc<dyn LineBindingStore>,
        extension_store: Arc<dyn ExtensionStore>,
        device_store: Arc<dyn DeviceStore>,
        call_state_manager: Arc<CallStateManager>,
        forwarding_rules_manager: Arc<tokio::sync::RwLock<ForwardingRulesManager>>,
    ) -> Self {
        Self {
            phone_number_store,
            line_binding_store,
            extension_store,
            device_store,
            voicemail_store: None,
            conference_store: None,
            voicemail_service_uri: None,
            conference_service_uri: None,
            call_state_manager,
            forwarding_rules_manager,
        }
    }

    pub fn with_voicemail_service(
        mut self,
        store: Arc<dyn VoicemailStore>,
        service_uri: impl Into<String>,
    ) -> Self {
        self.voicemail_store = Some(store);
        self.voicemail_service_uri = Some(service_uri.into());
        self
    }

    pub fn with_conference_service(
        mut self,
        store: Arc<dyn ConferenceStore>,
        service_uri: impl Into<String>,
    ) -> Self {
        self.conference_store = Some(store);
        self.conference_service_uri = Some(service_uri.into());
        self
    }

    /// Converts a line binding and its devices into routing targets.
    async fn line_binding_to_targets(
        &self,
        line_binding: &LineBinding,
    ) -> Result<Vec<RoutingTarget>, RoutingError> {
        let mut targets = Vec::new();

        for device_binding in &line_binding.devices {
            if !device_binding.enabled {
                continue;
            }

            // Get the device details
            let device = self
                .device_store
                .get_device(&device_binding.device_id)
                .await
                .map_err(|e| RoutingError::Storage(e.to_string()))?;

            let device = match device {
                Some(d) => d,
                None => {
                    warn!(
                        "Device {} not found for line binding {}",
                        device_binding.device_id, line_binding.id
                    );
                    continue;
                }
            };

            // Only route to enabled devices with a SIP URI
            if !device.enabled {
                debug!("Device {} is disabled, skipping", device.id);
                continue;
            }

            let sip_uri = match device.sip_uri {
                Some(uri) => uri,
                None => {
                    warn!("Device {} has no SIP URI, skipping", device.id);
                    continue;
                }
            };

            targets.push(RoutingTarget {
                device_id: device.id,
                sip_uri,
                priority: device_binding.priority,
                line_appearance: device_binding.line_appearance,
                shared_line: device_binding.shared_line,
                enabled: true,
                timeout: Duration::from_secs(30), // Default timeout
            });
        }

        if targets.is_empty() {
            return Err(RoutingError::NoDevicesAvailable);
        }

        match line_binding.ring_strategy {
            RingStrategy::RoundRobin => {
                let device_ids: Vec<_> = targets.iter().map(|target| target.device_id).collect();
                let selected = self
                    .call_state_manager
                    .get_next_round_robin_device(&line_binding.id, &device_ids)
                    .await
                    .ok_or(RoutingError::NoDevicesAvailable)?;

                Ok(targets
                    .into_iter()
                    .filter(|target| target.device_id == selected)
                    .collect())
            }
            RingStrategy::LongestIdle => {
                let device_ids: Vec<_> = targets.iter().map(|target| target.device_id).collect();
                let selected = self
                    .call_state_manager
                    .get_longest_idle_device(&device_ids)
                    .await
                    .unwrap_or(device_ids[0]);

                Ok(targets
                    .into_iter()
                    .filter(|target| target.device_id == selected)
                    .collect())
            }
            _ => Ok(targets),
        }
    }

    fn resolve_user_ring_strategy(&self, line_bindings: &[LineBinding]) -> RingStrategy {
        let Some(first) = line_bindings.first() else {
            return RingStrategy::Simultaneous;
        };

        if line_bindings
            .iter()
            .all(|binding| binding.ring_strategy == first.ring_strategy)
        {
            first.ring_strategy
        } else {
            warn!(
                user_bindings = line_bindings.len(),
                "Mixed ring strategies across user line bindings; falling back to simultaneous"
            );
            RingStrategy::Simultaneous
        }
    }

    fn merge_routing_targets(&self, targets: Vec<RoutingTarget>) -> Vec<RoutingTarget> {
        let mut merged: HashMap<crate::DeviceId, RoutingTarget> = HashMap::new();

        for target in targets {
            merged
                .entry(target.device_id)
                .and_modify(|existing| {
                    if target.priority < existing.priority {
                        existing.priority = target.priority;
                    }
                    if target.timeout > existing.timeout {
                        existing.timeout = target.timeout;
                    }
                    if target.line_appearance < existing.line_appearance {
                        existing.line_appearance = target.line_appearance;
                    }
                    existing.shared_line |= target.shared_line;
                    existing.enabled |= target.enabled;
                })
                .or_insert(target);
        }

        let mut targets: Vec<_> = merged.into_values().collect();
        targets.sort_by_key(|target| {
            (
                target.priority,
                target.line_appearance,
                target.device_id.to_string(),
            )
        });
        targets
    }

    fn build_service_forward_uri(service_uri: &str, extension: &str) -> String {
        let host = service_uri
            .strip_prefix("sip:")
            .or_else(|| service_uri.strip_prefix("sips:"))
            .unwrap_or(service_uri)
            .split('@')
            .last()
            .unwrap_or(service_uri)
            .trim();
        format!("sip:{}@{}", extension, host)
    }

    fn service_routing_result(forward_to: String) -> RoutingResult {
        RoutingResult {
            phone_number: None,
            line_binding: None,
            targets: Vec::new(),
            ring_strategy: RingStrategy::Simultaneous,
            forward_to: Some(forward_to),
            no_answer_timeout: Duration::from_secs(60),
        }
    }
}

#[async_trait]
impl RoutingEngine for DefaultRoutingEngine {
    async fn route_inbound_call(
        &self,
        destination: &RoutingDestination,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult, RoutingError> {
        info!(
            "Routing inbound call from {} to {} for tenant {}",
            caller, destination, tenant_id
        );

        match destination {
            RoutingDestination::E164(number) => {
                // Look up the phone number
                // Note: get_phone_number_by_e164 doesn't filter by tenant, so we need to verify tenant after
                let phone_number = self
                    .phone_number_store
                    .get_phone_number_by_e164(number)
                    .await
                    .map_err(|e| RoutingError::Storage(e.to_string()))?
                    .ok_or_else(|| RoutingError::PhoneNumberNotFound(number.clone()))?;

                // Verify the phone number belongs to this tenant
                if &phone_number.tenant_id != tenant_id {
                    return Err(RoutingError::PhoneNumberNotFound(number.clone()));
                }

                // Route to this phone number
                self.route_to_phone_number(&phone_number.id, tenant_id, caller)
                    .await
            }

            RoutingDestination::Extension(ext) => {
                // Resolve the extension
                let target = self.resolve_extension(ext, tenant_id).await?;

                // Route based on the extension target
                match target {
                    ExtensionTarget::User(user_id) => {
                        self.route_to_user(&user_id, tenant_id, caller).await
                    }
                    ExtensionTarget::PhoneNumber(phone_number_id) => {
                        self.route_to_phone_number(&phone_number_id, tenant_id, caller)
                            .await
                    }
                    ExtensionTarget::Device(device_id) => {
                        // Route directly to the device
                        let device = self
                            .device_store
                            .get_device(&device_id)
                            .await
                            .map_err(|e| RoutingError::Storage(e.to_string()))?
                            .ok_or_else(|| {
                                RoutingError::NoRoute(format!("Device {} not found", device_id))
                            })?;

                        if !device.enabled {
                            return Err(RoutingError::NoDevicesAvailable);
                        }

                        let sip_uri = device
                            .sip_uri
                            .ok_or_else(|| RoutingError::NoDevicesAvailable)?;

                        Ok(RoutingResult {
                            phone_number: None,
                            line_binding: None,
                            targets: vec![RoutingTarget {
                                device_id: device.id,
                                sip_uri,
                                priority: 1,
                                line_appearance: 1,
                                shared_line: false,
                                enabled: true,
                                timeout: Duration::from_secs(30),
                            }],
                            ring_strategy: RingStrategy::Simultaneous,
                            forward_to: None,
                            no_answer_timeout: Duration::from_secs(60),
                        })
                    }
                    ExtensionTarget::Service(_service_id) => {
                        if let Some(voicemail_store) = &self.voicemail_store {
                            let mailbox = voicemail_store
                                .get_mailbox_by_extension(tenant_id, ext)
                                .await
                                .map_err(|e| RoutingError::Storage(e.to_string()))?;
                            if let Some(mailbox) = mailbox {
                                if mailbox.enabled {
                                    if let Some(service_uri) = &self.voicemail_service_uri {
                                        return Ok(Self::service_routing_result(
                                            Self::build_service_forward_uri(service_uri, ext),
                                        ));
                                    }
                                    return Err(RoutingError::NoRoute(
                                        "Voicemail service URI not configured".to_string(),
                                    ));
                                }
                            }
                        }

                        if let Some(conference_store) = &self.conference_store {
                            let meeting = conference_store
                                .get_meeting_by_room_number(tenant_id, ext)
                                .await
                                .map_err(|e| RoutingError::Storage(e.to_string()))?;
                            if let Some(meeting) = meeting {
                                if meeting.enabled {
                                    if let Some(service_uri) = &self.conference_service_uri {
                                        return Ok(Self::service_routing_result(
                                            Self::build_service_forward_uri(service_uri, ext),
                                        ));
                                    }
                                    return Err(RoutingError::NoRoute(
                                        "Conference service URI not configured".to_string(),
                                    ));
                                }
                            }
                        }

                        Err(RoutingError::NoRoute(format!(
                            "No routable service target found for extension {}",
                            ext
                        )))
                    }
                }
            }

            RoutingDestination::SipUri(_uri) => {
                // For now, SIP URI routing is not supported (would be peer-to-peer)
                Err(RoutingError::NoRoute(
                    "Direct SIP URI routing not supported".to_string(),
                ))
            }

            RoutingDestination::UserId(user_id) => {
                self.route_to_user(user_id, tenant_id, caller).await
            }
        }
    }

    async fn resolve_extension(
        &self,
        extension: &str,
        tenant_id: &TenantId,
    ) -> Result<ExtensionTarget, RoutingError> {
        debug!("Resolving extension {} for tenant {}", extension, tenant_id);

        let ext = self
            .extension_store
            .get_extension_by_number(tenant_id, extension)
            .await
            .map_err(|e| RoutingError::Storage(e.to_string()))?
            .ok_or_else(|| RoutingError::ExtensionNotFound(extension.to_string()))?;

        if !ext.enabled {
            return Err(RoutingError::ExtensionNotFound(extension.to_string()));
        }

        Ok(ext.target)
    }

    async fn route_to_user(
        &self,
        user_id: &UserId,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult, RoutingError> {
        debug!(
            "Routing to user {} for tenant {} from caller {}",
            user_id, tenant_id, caller
        );

        // Find line bindings for this user
        // Note: list_line_bindings_by_user doesn't filter by tenant, so we need to verify tenant after
        let all_bindings = self
            .line_binding_store
            .list_line_bindings_by_user(user_id)
            .await
            .map_err(|e| RoutingError::Storage(e.to_string()))?;

        // Filter by tenant
        let line_bindings: Vec<_> = all_bindings
            .into_iter()
            .filter(|b| &b.tenant_id == tenant_id)
            .collect();

        if line_bindings.is_empty() {
            return Err(RoutingError::NoRoute(format!(
                "No line bindings found for user {}",
                user_id
            )));
        }

        let primary_line_binding = line_bindings[0].clone();

        // Honor the first applicable forwarding rule across all of the user's bindings.
        for line_binding in &line_bindings {
            if let Some(forward_to) = self
                .check_call_forward(line_binding, tenant_id, caller)
                .await
            {
                return Ok(RoutingResult {
                    phone_number: None,
                    line_binding: Some(primary_line_binding.clone()),
                    targets: Vec::new(),
                    ring_strategy: self.resolve_user_ring_strategy(&line_bindings),
                    forward_to: Some(forward_to),
                    no_answer_timeout: line_bindings
                        .iter()
                        .map(|binding| binding.no_answer_timeout)
                        .max()
                        .unwrap_or(primary_line_binding.no_answer_timeout),
                });
            }
        }

        let mut all_targets = Vec::new();
        for line_binding in &line_bindings {
            match self.line_binding_to_targets(line_binding).await {
                Ok(mut targets) => all_targets.append(&mut targets),
                Err(RoutingError::NoDevicesAvailable) => {
                    debug!(
                        line_binding_id = %line_binding.id,
                        "Skipping user line binding with no available devices"
                    );
                }
                Err(err) => return Err(err),
            }
        }

        let targets = self.merge_routing_targets(all_targets);
        if targets.is_empty() {
            return Err(RoutingError::NoDevicesAvailable);
        }

        Ok(RoutingResult {
            phone_number: None,
            line_binding: Some(primary_line_binding),
            targets,
            ring_strategy: self.resolve_user_ring_strategy(&line_bindings),
            forward_to: None,
            no_answer_timeout: line_bindings
                .iter()
                .map(|binding| binding.no_answer_timeout)
                .max()
                .unwrap_or(Duration::from_secs(30)),
        })
    }

    async fn route_to_phone_number(
        &self,
        phone_number_id: &PhoneNumberId,
        tenant_id: &TenantId,
        caller: &str,
    ) -> Result<RoutingResult, RoutingError> {
        debug!(
            "Routing to phone number {} for tenant {} from caller {}",
            phone_number_id, tenant_id, caller
        );

        // Get the phone number
        let phone_number = self
            .phone_number_store
            .get_phone_number(phone_number_id)
            .await
            .map_err(|e| RoutingError::Storage(e.to_string()))?
            .ok_or_else(|| RoutingError::PhoneNumberNotFound(phone_number_id.to_string()))?;

        // Check phone number state - only route to Active numbers
        if phone_number.state != crate::NumberState::Active {
            return Err(RoutingError::PhoneNumberNotRoutable(format!(
                "Phone number {} is in state {:?}, only Active numbers can receive calls",
                phone_number.e164, phone_number.state
            )));
        }

        // Find line binding for this phone number
        let line_binding = self
            .line_binding_store
            .get_line_binding_by_phone_number(phone_number_id)
            .await
            .map_err(|e| RoutingError::Storage(e.to_string()))?
            .ok_or_else(|| RoutingError::LineBindingNotFound(phone_number.e164.clone()))?;

        // Verify the line binding belongs to this tenant
        if &line_binding.tenant_id != tenant_id {
            return Err(RoutingError::LineBindingNotFound(phone_number.e164.clone()));
        }

        // Check for call forwarding
        if let Some(forward_to) = self
            .check_call_forward(&line_binding, tenant_id, caller)
            .await
        {
            return Ok(RoutingResult {
                phone_number: Some(phone_number),
                line_binding: Some(line_binding.clone()),
                targets: Vec::new(),
                ring_strategy: line_binding.ring_strategy,
                forward_to: Some(forward_to),
                no_answer_timeout: line_binding.no_answer_timeout,
            });
        }

        // Convert line binding to targets
        let targets = self.line_binding_to_targets(&line_binding).await?;

        Ok(RoutingResult {
            phone_number: Some(phone_number),
            line_binding: Some(line_binding.clone()),
            targets,
            ring_strategy: line_binding.ring_strategy,
            forward_to: None,
            no_answer_timeout: line_binding.no_answer_timeout,
        })
    }

    async fn check_call_forward(
        &self,
        line_binding: &LineBinding,
        _tenant_id: &TenantId,
        caller: &str,
    ) -> Option<String> {
        // First check the legacy forwarding field on line_binding
        if let Some(ref forward_to) = line_binding.forward_unconditional {
            info!(
                "Line binding {} has legacy unconditional forwarding to {}",
                line_binding.id, forward_to
            );
            return Some(forward_to.clone());
        }

        // Then check advanced forwarding rules
        let rules_manager = self.forwarding_rules_manager.read().await;
        let current_time = SystemTime::now();

        // Check for unconditional forwarding rule
        if let Some(rule) = rules_manager.find_applicable_rule(
            &line_binding.id,
            ForwardType::Unconditional,
            caller,
            current_time,
        ) {
            info!(
                "Line binding {} has unconditional forwarding rule to {}",
                line_binding.id, rule.destination
            );
            return Some(rule.destination.to_string());
        }

        // Other forwarding types (busy, no-answer, unavailable) would be checked during call progress
        None
    }
}

// Additional helper methods for DefaultRoutingEngine (not part of trait)
impl DefaultRoutingEngine {
    /// Checks for busy forwarding.
    pub async fn check_busy_forward(
        &self,
        line_binding: &LineBinding,
        caller: &str,
    ) -> Option<String> {
        let rules_manager = self.forwarding_rules_manager.read().await;
        let current_time = SystemTime::now();

        rules_manager
            .find_applicable_rule(&line_binding.id, ForwardType::Busy, caller, current_time)
            .map(|rule| {
                info!(
                    "Line binding {} has busy forwarding rule to {}",
                    line_binding.id, rule.destination
                );
                rule.destination.to_string()
            })
    }

    /// Checks for no-answer forwarding.
    pub async fn check_no_answer_forward(
        &self,
        line_binding: &LineBinding,
        caller: &str,
    ) -> Option<String> {
        let rules_manager = self.forwarding_rules_manager.read().await;
        let current_time = SystemTime::now();

        rules_manager
            .find_applicable_rule(
                &line_binding.id,
                ForwardType::NoAnswer,
                caller,
                current_time,
            )
            .map(|rule| {
                info!(
                    "Line binding {} has no-answer forwarding rule to {}",
                    line_binding.id, rule.destination
                );
                rule.destination.to_string()
            })
    }

    /// Checks for unavailable forwarding (device offline/unreachable).
    pub async fn check_unavailable_forward(
        &self,
        line_binding: &LineBinding,
        caller: &str,
    ) -> Option<String> {
        let rules_manager = self.forwarding_rules_manager.read().await;
        let current_time = SystemTime::now();

        rules_manager
            .find_applicable_rule(
                &line_binding.id,
                ForwardType::Unavailable,
                caller,
                current_time,
            )
            .map(|rule| {
                info!(
                    "Line binding {} has unavailable forwarding rule to {}",
                    line_binding.id, rule.destination
                );
                rule.destination.to_string()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DynStorage;
    use std::sync::Arc;

    // Helper to create a mock storage with test data
    async fn create_test_storage() -> Arc<DynStorage> {
        // In a real test, we would create an in-memory SQLite database
        // For now, this is a placeholder
        todo!("Create in-memory test storage")
    }

    #[tokio::test]
    #[ignore] // Ignore until we have in-memory storage
    async fn test_route_e164_to_device() {
        let storage = create_test_storage().await;
        let call_state = Arc::new(CallStateManager::new());
        let forwarding_rules = Arc::new(tokio::sync::RwLock::new(ForwardingRulesManager::new()));
        let engine = DefaultRoutingEngine::new(
            storage.clone(),
            storage.clone(),
            storage.clone(),
            storage.clone(),
            call_state,
            forwarding_rules,
        );

        let tenant_id = TenantId::nil();
        let destination = RoutingDestination::E164("+15551234567".to_string());
        let caller = "+15559999999";

        let result = engine
            .route_inbound_call(&destination, &tenant_id, caller)
            .await;
        assert!(result.is_ok());
    }
}
