//! Flow call handler for executing flows during calls.
//!
//! This module integrates the flow engine with call management, allowing
//! calls to execute programmable flows (IVR, call routing, etc.).

use crate::call::{Call, CallId};
use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_events::{EventPublisher, FlowEvent, PlatformEvent};
use fcp_flow::{FlowContext, FlowEngine, FlowExecutionResult, FlowNode, FlowStore};
use fcp_media::{AudioTarget, MediaManager};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Flow call handler.
///
/// Manages flow execution for calls, including:
/// - Loading flows based on routing rules
/// - Executing flow nodes
/// - Tracking execution state per call
/// - Publishing flow execution events
/// - Integrating media playback for Play/Say nodes
pub struct FlowCallHandler {
    /// Flow engine for executing flows
    engine: Arc<FlowEngine>,

    /// Flow store for loading flow definitions
    flow_store: Arc<dyn FlowStore>,

    /// Active flow contexts, indexed by call ID
    contexts: Arc<DashMap<CallId, Arc<RwLock<FlowContext>>>>,

    /// Event publisher for flow events
    events: Option<Arc<dyn EventPublisher>>,

    /// Flow routing rules (DID -> flow_id mapping)
    /// TODO: Phase 2 - Load from database
    routing: Arc<RwLock<HashMap<String, String>>>,

    /// Media manager for audio playback
    media_manager: Option<Arc<MediaManager>>,
}

impl FlowCallHandler {
    /// Create a new flow call handler.
    pub fn new(
        flow_store: Arc<dyn FlowStore>,
        events: Option<Arc<dyn EventPublisher>>,
        media_manager: Option<Arc<MediaManager>>,
    ) -> Self {
        Self {
            engine: Arc::new(FlowEngine::new()),
            flow_store,
            contexts: Arc::new(DashMap::new()),
            events,
            routing: Arc::new(RwLock::new(HashMap::new())),
            media_manager,
        }
    }

    /// Add a flow routing rule.
    ///
    /// Maps a phone number (DID) to a flow ID.
    pub async fn add_route(&self, did: String, flow_id: String) {
        info!(did = %did, flow_id = %flow_id, "Added flow routing rule");
        let mut routing = self.routing.write().await;
        routing.insert(did, flow_id);
    }

    /// Get flow ID for a phone number (DID).
    pub async fn get_flow_for_did(&self, did: &str) -> Option<String> {
        let routing = self.routing.read().await;
        routing.get(did).cloned()
    }

    /// Start flow execution for a call.
    ///
    /// Loads the flow definition and begins execution from the initial node.
    pub async fn start_flow(
        &self,
        call: &mut Call,
        flow_id: String,
        initial_variables: HashMap<String, String>,
        answer_callback: fcp_flow::AnswerCallback,
        hangup_callback: fcp_flow::HangupCallback,
    ) -> Result<()> {
        info!(
            call_id = %call.id,
            flow_id = %flow_id,
            "Starting flow execution for call"
        );

        // Load flow definition
        let flow = self
            .flow_store
            .get_flow(&flow_id)
            .await
            .context("Failed to load flow")?
            .ok_or_else(|| anyhow::anyhow!("Flow not found: {}", flow_id))?;

        if !flow.enabled {
            return Err(anyhow::anyhow!("Flow is disabled: {}", flow_id));
        }

        let flow = Arc::new(flow);

        // Create flow context
        let call_id_core = fcp_core::CallId::from_uuid(*call.id.as_uuid());
        let mut context = self
            .engine
            .execute(flow.clone(), call_id_core, initial_variables)
            .await?;

        // Set media session ID if available (needed for Play/Say nodes)
        if let Some(media_session_id) = call.media_session_id {
            context.set_media_session_id(media_session_id);
        }

        // Set callbacks for Answer and Hangup nodes
        context.set_answer_callback(answer_callback);
        context.set_hangup_callback(hangup_callback);

        // Store context
        self.contexts
            .insert(call.id, Arc::new(RwLock::new(context)));

        // Update call
        call.flow_id = Some(flow_id.clone());
        call.flow_current_node = Some(flow.initial_node.clone());

        // Publish flow started event
        if let Some(events) = &self.events {
            let event = PlatformEvent::Flow(FlowEvent::Started {
                call_id: call_id_core,
                flow_id: flow_id.clone(),
                flow_version: flow.version,
                initial_node: flow.initial_node.clone(),
                timestamp: std::time::SystemTime::now(),
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    error!(error = %e, "Failed to publish FlowEvent::Started");
                }
            });
        }

        // Execute the initial node
        self.execute_next_node(call.id).await?;

        Ok(())
    }

    /// Execute the next node in the flow.
    ///
    /// # Media Integration (Play/Say Nodes)
    ///
    /// Play and Say nodes are handled by the FlowEngine, which returns `Continue(next_node)`.
    /// The actual media playback/TTS is currently a placeholder. For full implementation:
    ///
    /// ## Architecture Options:
    ///
    /// ### Option 1: External Media Server (Recommended)
    /// - Deploy a media server (e.g., Asterisk, FreeSWITCH, or custom RTP engine)
    /// - On Play/Say node: Send SIP re-INVITE to insert media server in call path
    /// - Media server handles RTP, plays files, generates TTS
    /// - When complete, send another re-INVITE to remove media server
    ///
    /// ### Option 2: Direct RTP Control
    /// - Implement RTP engine in fcp-media crate
    /// - Anchor media on FCP server
    /// - Send RTP packets directly for audio playback
    /// - Integrate TTS engine (e.g., Google TTS, AWS Polly) for Say nodes
    ///
    /// ### Option 3: Hybrid
    /// - Use external media server for complex operations (conferencing, recording)
    /// - Use direct RTP for simple operations (Play, Say)
    ///
    /// ## Implementation Steps (Future):
    /// 1. Add media_server field to FlowCallHandler with media server SIP URI
    /// 2. On Play/Say nodes, check node type from context before executing
    /// 3. Send re-INVITE to A-leg with media server's SDP
    /// 4. Command media server via SIP INFO or proprietary protocol
    /// 5. Wait for playback completion event
    /// 6. Send re-INVITE to restore direct media path or bridge to B-leg
    /// 7. Continue to next node
    ///
    /// ## Current State:
    /// Play/Say nodes are executed by FlowEngine with placeholder logging.
    /// No actual media playback occurs. Calls continue through flow normally.
    pub async fn execute_next_node(&self, call_id: CallId) -> Result<()> {
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No flow context for call: {}", call_id))?;

        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        let current_node = context_guard.current_node.clone();

        debug!(
            call_id = %call_id,
            node_id = %current_node,
            "Executing flow node"
        );

        // Check if this is an Answer or Hangup node - execute callbacks
        let node_type = context_guard
            .flow
            .nodes
            .get(&current_node)
            .map(|n| n.node_type.clone());

        if let Some(node_type) = node_type {
            use fcp_flow::NodeType;

            match node_type {
                NodeType::Answer { next } => {
                    info!(
                        call_id = %call_id,
                        node_id = %current_node,
                        "Answer node: calling answer callback"
                    );

                    // Clone callback before dropping guard
                    let callback = context_guard.answer_callback.clone();
                    drop(context_guard);

                    // Execute answer callback if available
                    if let Some(callback) = callback {
                        match callback().await {
                            Ok(()) => {
                                info!(call_id = %call_id, "Call answered successfully");
                            }
                            Err(e) => {
                                error!(call_id = %call_id, error = %e, "Failed to answer call");
                            }
                        }

                        // Re-acquire lock and continue to next node
                        let mut context_guard = context.write().await;
                        context_guard.current_node = next.clone();
                        drop(context_guard);

                        return Box::pin(self.execute_next_node(call_id)).await;
                    } else {
                        warn!(call_id = %call_id, "Answer callback not set");
                        let mut context_guard = context.write().await;
                        context_guard.current_node = next.clone();
                        drop(context_guard);
                        return Box::pin(self.execute_next_node(call_id)).await;
                    }
                }

                NodeType::Hangup { reason } => {
                    info!(
                        call_id = %call_id,
                        node_id = %current_node,
                        reason = ?reason,
                        "Hangup node: calling hangup callback"
                    );

                    // Clone callback before dropping guard
                    let callback = context_guard.hangup_callback.clone();
                    drop(context_guard);

                    // Execute hangup callback if available
                    if let Some(callback) = callback {
                        match callback(reason).await {
                            Ok(()) => {
                                info!(call_id = %call_id, "Call hung up successfully");
                            }
                            Err(e) => {
                                error!(call_id = %call_id, error = %e, "Failed to hang up call");
                            }
                        }

                        // Clean up context
                        self.contexts.remove(&call_id);
                        return Ok(());
                    } else {
                        warn!(call_id = %call_id, "Hangup callback not set");
                        self.contexts.remove(&call_id);
                        return Ok(());
                    }
                }

                _ => {
                    // Not Answer or Hangup, continue with normal processing
                }
            }
        }

        // Check if this is a Play or Say node - handle media playback before execution
        if let Some(media_manager) = &self.media_manager {
            if let Some(node) = context_guard.flow.nodes.get(&current_node) {
                use fcp_flow::NodeType;

                match &node.node_type {
                    NodeType::Play { url, next, .. } => {
                        info!(
                            call_id = %call_id,
                            node_id = %current_node,
                            url = %url,
                            "Playing audio file"
                        );

                        // TODO: Interpolate variables in URL (requires implementing interpolation)
                        // For now, use raw URL value
                        let url_value = url.clone();

                        // Use media session ID from context (not SIP call ID)
                        let session_id = context_guard.media_session_id.ok_or_else(|| {
                            anyhow::anyhow!("No media session ID available for playback")
                        })?;

                        // Start playback to ParticipantA (the caller in IVR mode)
                        // Note: Using ParticipantA instead of Both to avoid echo in IVR scenarios
                        match media_manager
                            .play_file(&session_id, url_value.clone(), AudioTarget::ParticipantA)
                            .await
                        {
                            Ok(playback_id) => {
                                info!(
                                    call_id = %call_id,
                                    playback_id = %playback_id,
                                    "Audio playback started"
                                );

                                // Clone next node ID before dropping guard
                                let next_node = next.clone();

                                // Drop context guard before waiting for completion
                                drop(context_guard);

                                // Wait for playback to complete before continuing
                                match media_manager.wait_playback_completion(playback_id.clone()).await
                                {
                                    Ok(()) => {
                                        info!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            "Audio playback completed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            error = %e,
                                            "Audio playback failed or was stopped"
                                        );
                                    }
                                }

                                // Re-acquire lock and continue to next node
                                let mut context_guard = context.write().await;
                                context_guard.current_node = next_node;
                                drop(context_guard);

                                // Continue to next node
                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                            Err(e) => {
                                error!(
                                    call_id = %call_id,
                                    error = %e,
                                    "Failed to play audio file"
                                );

                                // Continue to next node anyway
                                context_guard.current_node = next.clone();
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                        }
                    }

                    NodeType::Say {
                        text,
                        language,
                        voice,
                        next,
                        ..
                    } => {
                        info!(
                            call_id = %call_id,
                            node_id = %current_node,
                            text = %text,
                            "Playing text-to-speech"
                        );

                        // TODO: Interpolate variables in text (requires implementing interpolation)
                        // For now, use raw values
                        let text_value = text.clone();
                        let language_value = language.clone();
                        let voice_value = voice.clone();

                        // Use media session ID from context (not SIP call ID)
                        let session_id = context_guard.media_session_id.ok_or_else(|| {
                            anyhow::anyhow!("No media session ID available for TTS playback")
                        })?;

                        // Start TTS playback
                        match media_manager
                            .play_tts(
                                &session_id,
                                text_value,
                                language_value,
                                voice_value,
                                AudioTarget::Both,
                            )
                            .await
                        {
                            Ok(playback_id) => {
                                info!(
                                    call_id = %call_id,
                                    playback_id = %playback_id,
                                    "TTS playback started"
                                );

                                // Clone next node ID before dropping guard
                                let next_node = next.clone();

                                // Drop context guard before waiting for completion
                                drop(context_guard);

                                // Wait for playback to complete before continuing
                                match media_manager.wait_playback_completion(playback_id.clone()).await
                                {
                                    Ok(()) => {
                                        info!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            "TTS playback completed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            call_id = %call_id,
                                            playback_id = %playback_id,
                                            error = %e,
                                            "TTS playback failed or was stopped"
                                        );
                                    }
                                }

                                // Re-acquire lock and continue to next node
                                let mut context_guard = context.write().await;
                                context_guard.current_node = next_node;
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                            Err(e) => {
                                // TTS not configured yet - log and continue
                                warn!(
                                    call_id = %call_id,
                                    error = %e,
                                    "TTS not available, continuing flow"
                                );

                                // Continue to next node
                                context_guard.current_node = next.clone();
                                drop(context_guard);

                                return Box::pin(self.execute_next_node(call_id)).await;
                            }
                        }
                    }

                    _ => {
                        // Not a media node, continue with normal execution
                    }
                }
            }
        }

        // Normal node execution (non-media nodes or when media_manager not available)
        let result = self
            .engine
            .execute_node(&mut context_guard, &current_node)
            .await?;

        // Publish node executed event
        if let Some(events) = &self.events {
            let event = PlatformEvent::Flow(FlowEvent::NodeExecuted {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                flow_id: context_guard.flow.id.clone(),
                node_id: current_node.clone(),
                timestamp: std::time::SystemTime::now(),
            });

            let events = Arc::clone(events);
            tokio::spawn(async move {
                if let Err(e) = events.publish(event).await {
                    error!(error = %e, "Failed to publish FlowEvent::NodeExecuted");
                }
            });
        }

        match result {
            FlowExecutionResult::Continue(next_node) => {
                info!(
                    call_id = %call_id,
                    current = %current_node,
                    next = %next_node,
                    "Flow continuing to next node"
                );

                context_guard.current_node = next_node;
                drop(context_guard);

                // Recursively execute next node
                Box::pin(self.execute_next_node(call_id)).await?;
            }

            FlowExecutionResult::Hangup(reason) => {
                info!(
                    call_id = %call_id,
                    reason = ?reason,
                    "Flow requested hangup"
                );

                // Publish flow completed event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Flow(FlowEvent::Completed {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        flow_id: context_guard.flow.id.clone(),
                        outcome: "hangup".to_string(),
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish FlowEvent::Completed");
                        }
                    });
                }

                // Clean up context
                drop(context_guard);
                self.contexts.remove(&call_id);

                // TODO: Send BYE to hangup the call
                warn!(call_id = %call_id, "Flow requested hangup, but SIP hangup not yet implemented");
            }

            FlowExecutionResult::WaitingForInput {
                variable,
                transitions: _,
                default: _,
            } => {
                info!(
                    call_id = %call_id,
                    variable = %variable,
                    "Flow waiting for user input (DTMF)"
                );

                // Publish waiting for input event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Flow(FlowEvent::WaitingForInput {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        flow_id: context_guard.flow.id.clone(),
                        variable: variable.clone(),
                        timeout_secs: 5, // TODO: Get from Gather node
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish FlowEvent::WaitingForInput");
                        }
                    });
                }

                // TODO: Set up DTMF listener and wait for input
                warn!(call_id = %call_id, "DTMF input handling not yet implemented");
            }
        }

        Ok(())
    }

    /// Handle DTMF input from the caller.
    ///
    /// This is called when DTMF digits are received during a Gather node.
    pub async fn handle_dtmf_input(&self, call_id: CallId, digits: String) -> Result<()> {
        info!(
            call_id = %call_id,
            digits = %digits,
            "Received DTMF input"
        );

        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No flow context for call: {}", call_id))?;

        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;

        // TODO: Process DTMF input based on current Gather node
        // For now, just store the digits as a variable
        context_guard.set_variable("dtmf_input".to_string(), digits.clone());

        drop(context_guard);

        // Continue execution
        self.execute_next_node(call_id).await?;

        Ok(())
    }

    /// Stop flow execution for a call.
    pub async fn stop_flow(&self, call_id: CallId) -> Result<()> {
        info!(call_id = %call_id, "Stopping flow execution");

        if let Some((_, context)) = self.contexts.remove(&call_id) {
            let context_guard = context.read().await;

            // Publish flow stopped event
            if let Some(events) = &self.events {
                let event = PlatformEvent::Flow(FlowEvent::Completed {
                    call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                    flow_id: context_guard.flow.id.clone(),
                    outcome: "stopped".to_string(),
                    timestamp: std::time::SystemTime::now(),
                });

                let events = Arc::clone(events);
                tokio::spawn(async move {
                    if let Err(e) = events.publish(event).await {
                        error!(error = %e, "Failed to publish FlowEvent::Completed");
                    }
                });
            }
        }

        Ok(())
    }

    /// Get the current flow context for a call.
    pub async fn get_context(&self, call_id: &CallId) -> Option<Arc<RwLock<FlowContext>>> {
        self.contexts.get(call_id).map(|entry| Arc::clone(entry.value()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_flow::InMemoryFlowStore;

    #[tokio::test]
    async fn test_flow_routing() {
        let store = Arc::new(InMemoryFlowStore::new());
        let handler = FlowCallHandler::new(store, None, None);

        handler.add_route("+15551234567".to_string(), "test-flow".to_string()).await;

        let flow_id = handler.get_flow_for_did("+15551234567").await;
        assert_eq!(flow_id, Some("test-flow".to_string()));

        let no_route = handler.get_flow_for_did("+15559999999").await;
        assert_eq!(no_route, None);
    }
}
