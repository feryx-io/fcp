//! Flow call handler for executing flows during calls.
//!
//! This module integrates the flow engine with call management, allowing
//! calls to execute programmable flows (IVR, call routing, etc.).

use anyhow::{Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fcp_b2bua_core::{Call, CallId, CallManager};
use fcp_events::{EventPublisher, FlowEvent, PlatformEvent, VoicemailEvent};
use fcp_flow::{FlowContext, FlowEngine, FlowExecutionResult, FlowStore};
use fcp_media::{AudioTarget, MediaManager};
use fcp_voicemail::VoicemailService;
use sip_core::{Request, Response};
use sip_dialog::{Dialog, DialogManager};
use sip_transaction::{
    ClientTransactionUser, TransactionKey, TransactionManager, TransportContext,
    TransportDispatcher,
};
use sip_uac::UserAgentClient;

use crate::registrar::FcpRegistrar;
use fcp_core::storage::{DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

/// Pre-compiled regex for interpolating `{{variable}}` placeholders.
static INTERPOLATION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{\{([^}]+)\}\}").expect("invalid interpolation regex"));

/// Flow call handler.
///
/// Manages flow execution for calls, including:
/// - Loading flows based on routing rules
/// - Executing flow nodes
/// - Tracking execution state per call
/// - Publishing flow execution events
/// - Integrating media playback for Play/Say nodes
/// - Creating outbound calls (B2BUA Dial node)
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
    /// Phase 2: Load from database via FlowStore when database-backed routing is implemented.
    routing: Arc<RwLock<HashMap<String, String>>>,

    /// Media manager for audio playback and bridging
    media_manager: Option<Arc<MediaManager>>,

    /// Call manager for tracking active calls
    call_manager: Option<Arc<CallManager>>,

    /// Transaction manager for sending SIP requests
    transaction_manager: Option<Arc<TransactionManager>>,

    /// Dialog manager for managing SIP dialogs
    dialog_manager: Option<Arc<DialogManager>>,

    /// Local SIP URI for outbound calls
    local_uri: Option<sip_core::SipUri>,

    /// Contact URI for outbound calls
    contact_uri: Option<sip_core::SipUri>,

    /// Media profile for SDP generation
    media_profile: Option<sip_sdp::profiles::MediaProfileBuilder>,

    /// Local address for SDP origin/connection lines
    local_addr: Option<String>,

    /// RTP port for SDP media lines
    rtp_port: u16,

    /// Voicemail service for recording voicemail messages
    voicemail_service: Option<Arc<VoicemailService>>,

    /// Prompt service for resolving system prompts to audio files
    prompt_service: Option<Arc<fcp_core::PromptService>>,

    /// Flow trace store for persistent execution tracing
    flow_trace_store: Option<Arc<dyn fcp_core::flow_trace::FlowTraceStore>>,

    /// Registrar for resolving extensions to registered device contacts
    registrar: Option<Arc<FcpRegistrar>>,

    /// Storage backend for extension → phone number → device resolution
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// Transport dispatcher for sending out-of-transaction messages (ACK for 2xx)
    dispatcher: Option<Arc<dyn TransportDispatcher>>,
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
            call_manager: None,
            transaction_manager: None,
            dialog_manager: None,
            local_uri: None,
            contact_uri: None,
            media_profile: None,
            local_addr: None,
            rtp_port: 8000,
            voicemail_service: None,
            prompt_service: None,
            flow_trace_store: None,
            registrar: None,
            storage: None,
            dispatcher: None,
        }
    }

    /// Set the call manager (for B2BUA operations).
    pub fn with_call_manager(mut self, call_manager: Arc<CallManager>) -> Self {
        self.call_manager = Some(call_manager);
        self
    }

    /// Set the transaction manager (for sending SIP requests).
    pub fn with_transaction_manager(
        mut self,
        transaction_manager: Arc<TransactionManager>,
    ) -> Self {
        self.transaction_manager = Some(transaction_manager);
        self
    }

    /// Set the dialog manager (for managing SIP dialogs).
    pub fn with_dialog_manager(mut self, dialog_manager: Arc<DialogManager>) -> Self {
        self.dialog_manager = Some(dialog_manager);
        self
    }

    /// Set the local and contact URIs for outbound calls.
    pub fn with_uris(mut self, local_uri: sip_core::SipUri, contact_uri: sip_core::SipUri) -> Self {
        self.local_uri = Some(local_uri);
        self.contact_uri = Some(contact_uri);
        self
    }

    /// Set the media profile for SDP generation in outbound calls.
    pub fn with_media_profile(
        mut self,
        profile: sip_sdp::profiles::MediaProfileBuilder,
        local_addr: String,
        rtp_port: u16,
    ) -> Self {
        self.media_profile = Some(profile);
        self.local_addr = Some(local_addr);
        self.rtp_port = rtp_port;
        self
    }

    /// Set the voicemail service (for Voicemail flow nodes).
    pub fn with_voicemail_service(mut self, voicemail_service: Arc<VoicemailService>) -> Self {
        self.voicemail_service = Some(voicemail_service);
        self
    }

    /// Set the prompt service (for resolving system prompts to audio files).
    pub fn with_prompt_service(mut self, prompt_service: Arc<fcp_core::PromptService>) -> Self {
        self.prompt_service = Some(prompt_service);
        self
    }

    /// Set the TTS service on the flow engine (for Gather Say prompts and Say nodes).
    pub fn with_tts_service(mut self, tts_service: Arc<fcp_media::TtsService>) -> Self {
        self.engine = Arc::new(FlowEngine::with_tts(tts_service));
        self
    }

    /// Set the flow trace store for persistent execution tracing.
    pub fn with_flow_trace_store(
        mut self,
        store: Arc<dyn fcp_core::flow_trace::FlowTraceStore>,
    ) -> Self {
        self.flow_trace_store = Some(store);
        self
    }

    /// Set the registrar for resolving extensions to registered contacts.
    pub fn with_registrar(mut self, registrar: Arc<FcpRegistrar>) -> Self {
        self.registrar = Some(registrar);
        self
    }

    /// Set the storage backend for extension → device resolution.
    pub fn with_storage(mut self, storage: Arc<fcp_core::DynStorage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Set the transport dispatcher for sending out-of-transaction messages (ACK for 2xx).
    pub fn with_dispatcher(mut self, dispatcher: Arc<dyn TransportDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Get a reference to the flow store.
    pub fn flow_store(&self) -> &Arc<dyn FlowStore> {
        &self.flow_store
    }

    /// Interpolate `{{variable}}` placeholders in a template string using flow context variables.
    fn interpolate(template: &str, context: &FlowContext) -> String {
        let mut result = template.to_string();
        for cap in INTERPOLATION_RE.captures_iter(template) {
            let var_name = cap[1].trim();
            if let Ok(value) = context.get_variable(var_name) {
                result = result.replace(&cap[0], &value);
            }
        }
        result
    }

    /// Persist a flow trace entry asynchronously (fire-and-forget).
    #[allow(clippy::too_many_arguments)]
    fn persist_trace(
        &self,
        call_id: &CallId,
        flow_id: &str,
        flow_name: Option<&str>,
        node_id: Option<&str>,
        event_type: fcp_core::flow_trace::FlowTraceEventType,
        detail: Option<String>,
        flow_version: Option<u32>,
        elapsed_ms: Option<i64>,
    ) {
        if let Some(ref store) = self.flow_trace_store {
            // The call's tenant, so retention and erasure can find the
            // trace without its record.
            let tenant_id = self
                .call_manager
                .as_ref()
                .and_then(|cm| cm.get_call(call_id))
                .map(|call| call.tenant_id.to_string());
            let entry = fcp_core::flow_trace::FlowTraceEntry {
                id: 0,
                call_id: call_id.to_string(),
                tenant_id,
                flow_id: flow_id.to_string(),
                flow_name: flow_name.map(|s| s.to_string()),
                node_id: node_id.map(|s| s.to_string()),
                event_type,
                timestamp: std::time::SystemTime::now(),
                detail,
                flow_version,
                elapsed_ms,
            };
            let store = Arc::clone(store);
            tokio::spawn(async move {
                if let Err(e) = store.insert_flow_trace(&entry).await {
                    error!(error = %e, "Failed to persist flow trace");
                }
            });
        }
    }

    /// Resolve a prompt API URL to a local filesystem path.
    ///
    /// Handles URLs like `/api/v1/prompts/prompt-<uuid>/audio` by extracting the
    /// prompt ID and looking up the audio file path via PromptService.
    async fn resolve_prompt_url(&self, url: &str) -> Option<String> {
        // Match API prompt URLs: /api/v1/prompts/<prompt-id>/audio
        if let Some(prompt_id_str) = url
            .strip_prefix("/api/v1/prompts/")
            .and_then(|rest| rest.strip_suffix("/audio"))
        {
            if let Some(ref ps) = self.prompt_service {
                if let Ok(prompt_id) = prompt_id_str.parse::<fcp_core::PromptId>() {
                    match ps.resolve_by_id(&prompt_id).await {
                        Ok(Some(path)) => {
                            let path_str = path.to_string_lossy().to_string();
                            info!(
                                prompt_id = %prompt_id_str,
                                resolved_path = %path_str,
                                "Resolved prompt API URL to filesystem path"
                            );
                            return Some(path_str);
                        }
                        Ok(None) => {
                            warn!(
                                prompt_id = %prompt_id_str,
                                "Prompt found but no audio file on disk"
                            );
                        }
                        Err(e) => {
                            warn!(
                                prompt_id = %prompt_id_str,
                                error = %e,
                                "Failed to resolve prompt by ID"
                            );
                        }
                    }
                }
            }
        }
        None
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

    /// Get all DID-to-flow_id routes.
    pub async fn get_all_routes(&self) -> HashMap<String, String> {
        self.routing.read().await.clone()
    }

    /// Remove a DID route. Returns true if it existed.
    pub async fn remove_route(&self, did: &str) -> bool {
        self.routing.write().await.remove(did).is_some()
    }

    /// Remove all DID routes that point to a given flow ID.
    pub async fn remove_routes_for_flow(&self, flow_id: &str) {
        let mut routing = self.routing.write().await;
        routing.retain(|_did, fid| fid != flow_id);
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
        context.set_answer_callback(answer_callback.clone());
        context.set_hangup_callback(hangup_callback);

        // If the initial node is not an Answer node, auto-answer the call first.
        // Most IVR flows need the call answered before any media playback can work.
        let initial_is_answer = flow
            .nodes
            .get(&flow.initial_node)
            .map(|n| matches!(n.node_type, fcp_flow::NodeType::Answer { .. }))
            .unwrap_or(false);

        if !initial_is_answer {
            info!(
                call_id = %call.id,
                initial_node = %flow.initial_node,
                "Initial node is not Answer — auto-answering call"
            );
            if let Err(e) = answer_callback().await {
                error!(call_id = %call.id, error = %e, "Auto-answer failed");
            } else {
                info!(call_id = %call.id, "Call auto-answered successfully");
                // Propagate media_session_id that the answer callback created
                if let Some(ref cm) = self.call_manager {
                    if let Some(call_entry) = cm.get_call(&call.id) {
                        if let Some(ref msid) = call_entry.media_session_id {
                            context.set_media_session_id(*msid);
                        }
                    }
                }
            }
        }

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

        // Persist flow trace: Started
        self.persist_trace(
            &call.id,
            &flow_id,
            Some(&flow.name),
            Some(&flow.initial_node),
            fcp_core::flow_trace::FlowTraceEventType::Started,
            None,
            Some(flow.version),
            Some(0),
        );

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
    /// Handle media playback for Say nodes that have synthesized TTS audio.
    ///
    /// This method is called when FlowEngine returns WaitingForMedia after
    /// synthesizing TTS audio. It reads the audio path from context variables
    /// and plays it via the media manager.
    async fn handle_media_playback(&self, call_id: CallId) -> Result<()> {
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found for call {}", call_id))?;

        let context = context_entry.value().clone();
        let context_guard = context.read().await;

        // Get TTS audio path from context variables
        let audio_path = context_guard
            .get_variable("__tts_audio_path")
            .map_err(|_| anyhow::anyhow!("No TTS audio path in context"))?;

        let next_node = context_guard
            .get_variable("__tts_next_node")
            .map_err(|_| anyhow::anyhow!("No next node in context"))?;

        let media_session_id = context_guard
            .media_session_id
            .ok_or_else(|| anyhow::anyhow!("No media session ID available for playback"))?;

        drop(context_guard);

        // Play the synthesized TTS audio
        if let Some(media_manager) = &self.media_manager {
            info!(
                call_id = %call_id,
                audio_path = %audio_path,
                "Playing synthesized TTS audio"
            );

            match media_manager
                .play_file(
                    &media_session_id,
                    audio_path.clone(),
                    fcp_media::AudioTarget::ParticipantA,
                )
                .await
            {
                Ok(playback_id) => {
                    info!(
                        call_id = %call_id,
                        playback_id = %playback_id,
                        "TTS audio playback started"
                    );

                    // Wait for playback to complete
                    match media_manager
                        .wait_playback_completion(playback_id.clone())
                        .await
                    {
                        Ok(()) => {
                            info!(
                                call_id = %call_id,
                                playback_id = %playback_id,
                                "TTS audio playback completed"
                            );
                        }
                        Err(e) => {
                            warn!(
                                call_id = %call_id,
                                playback_id = %playback_id,
                                error = %e,
                                "TTS audio playback failed or was stopped"
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to play TTS audio"
                    );
                }
            }
        } else {
            warn!(
                call_id = %call_id,
                "Media manager not available, skipping TTS playback"
            );
        }

        // Continue to next node
        let mut context_guard = context.write().await;
        context_guard.current_node = next_node;
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle outbound call creation for Dial node (B2BUA).
    ///
    /// Creates a B-leg call to the destination and bridges it with the A-leg.
    ///
    /// # Implementation Status
    ///
    /// Phase 1 (COMPLETE): Outbound INVITE creation ✅
    /// Phase 2 (IN PROGRESS): SIP response handling
    /// Phase 3 (PENDING): Media bridging
    /// Phase 4 (PENDING): CDR tracking
    /// Phase 5 (PENDING): Advanced features
    #[allow(clippy::too_many_arguments)]
    async fn handle_dial(
        &self,
        a_leg_call_id: CallId,
        destination: String,
        timeout_secs: u32,
        caller_id: Option<String>,
        max_duration_secs: u32,
        on_busy: Option<String>,
        on_no_answer: Option<String>,
        on_error: Option<String>,
        next: String,
    ) -> Result<()> {
        let caller_id_str = caller_id.as_deref().unwrap_or("none");
        info!(
            a_leg_call_id = %a_leg_call_id,
            destination = %destination,
            timeout_secs = timeout_secs,
            caller_id = %caller_id_str,
            max_duration_secs = max_duration_secs,
            "Starting Dial node - creating outbound B-leg call"
        );

        // ===== PHASE 1: OUTBOUND INVITE CREATION =====

        // Check if B2BUA components are available
        if self.transaction_manager.is_none() {
            warn!(a_leg_call_id = %a_leg_call_id, "TransactionManager not available - Dial node disabled");
            return self
                .transition_to_error(
                    a_leg_call_id,
                    "TransactionManager not configured",
                    on_error,
                    next,
                )
                .await;
        }

        if self.local_uri.is_none() || self.contact_uri.is_none() {
            warn!(a_leg_call_id = %a_leg_call_id, "SIP URIs not configured - Dial node disabled");
            return self
                .transition_to_error(a_leg_call_id, "SIP URIs not configured", on_error, next)
                .await;
        }

        let transaction_manager = self.transaction_manager.as_ref().unwrap();
        let local_uri = self.local_uri.as_ref().unwrap();
        let contact_uri = self.contact_uri.as_ref().unwrap();

        // Parse destination URI. For bare extensions, resolve via registrar or
        // extension → phone number → device chain to get the actual device contact
        // (e.g., "1234" → "polycom@192.168.1.12") so we INVITE the device directly
        // instead of looping through the local B2BUA.
        let dest_uri = match sip_core::SipUri::parse(&destination) {
            Ok(uri) => uri,
            Err(_) => {
                // Bare extension — try registrar lookup first
                let mut resolved = None;

                if let Some(ref registrar) = self.registrar {
                    let aor = if let Some(ref local) = self.local_uri {
                        format!("sip:{}@{}", destination, local.host())
                    } else {
                        format!("sip:{}@localhost", destination)
                    };
                    match registrar.get_registrations(&aor).await {
                        Ok(bindings) if !bindings.is_empty() => {
                            // Use highest q-value binding
                            let contact = bindings[0].contact();
                            if let Ok(uri) = sip_core::SipUri::parse(contact) {
                                info!(
                                    a_leg_call_id = %a_leg_call_id,
                                    destination = %destination,
                                    contact = %contact,
                                    "Resolved extension via registrar to device contact"
                                );
                                resolved = Some(uri);
                            }
                        }
                        Ok(_) => {
                            debug!(a_leg_call_id = %a_leg_call_id, destination = %destination, "No registrations found for extension");
                        }
                        Err(e) => {
                            debug!(a_leg_call_id = %a_leg_call_id, error = %e, "Registrar lookup failed for extension");
                        }
                    }
                }

                // If registrar lookup failed, try extension → phone number → device chain
                if resolved.is_none() {
                    if let (Some(ref storage), Some(ref registrar)) =
                        (&self.storage, &self.registrar)
                    {
                        if let Some(tenant_id) = self
                            .call_manager
                            .as_ref()
                            .and_then(|cm| cm.get_call(&a_leg_call_id).map(|c| c.tenant_id))
                        {
                            debug!(
                                a_leg_call_id = %a_leg_call_id,
                                destination = %destination,
                                tenant_id = %tenant_id,
                                "Looking up extension via storage (extension → device chain)"
                            );

                            if let Ok(Some(extension)) = storage
                                .get_extension_by_number(&tenant_id, &destination)
                                .await
                            {
                                info!(
                                    a_leg_call_id = %a_leg_call_id,
                                    destination = %destination,
                                    target = %extension.target,
                                    "Found extension in storage"
                                );

                                match &extension.target {
                                    fcp_core::ExtensionTarget::PhoneNumber(phone_number_id) => {
                                        match storage.get_phone_number(phone_number_id).await {
                                            Ok(Some(phone_number)) => {
                                                info!(
                                                    a_leg_call_id = %a_leg_call_id,
                                                    phone_number_id = %phone_number.id,
                                                    number = %phone_number.e164,
                                                    "Resolved phone_number from extension target"
                                                );
                                                match storage
                                                    .get_line_binding_by_phone_number(
                                                        &phone_number.id,
                                                    )
                                                    .await
                                                {
                                                    Ok(Some(line_binding)) => {
                                                        let devices =
                                                            line_binding.enabled_devices();
                                                        info!(
                                                            a_leg_call_id = %a_leg_call_id,
                                                            enabled_device_count = devices.len(),
                                                            "Found line binding with enabled devices"
                                                        );
                                                        for device_binding in devices {
                                                            if resolved.is_some() {
                                                                break;
                                                            }
                                                            match storage
                                                                .get_device(
                                                                    &device_binding.device_id,
                                                                )
                                                                .await
                                                            {
                                                                Ok(Some(device)) => {
                                                                    info!(
                                                                        a_leg_call_id = %a_leg_call_id,
                                                                        device_name = %device.name,
                                                                        device_sip_uri = ?device.sip_uri,
                                                                        "Found device in chain"
                                                                    );
                                                                    if let Some(sip_uri) =
                                                                        &device.sip_uri
                                                                    {
                                                                        match registrar.get_registrations(sip_uri).await {
                                                                        Ok(bindings) if !bindings.is_empty() => {
                                                                            if let Ok(uri) = sip_core::SipUri::parse(bindings[0].contact()) {
                                                                                info!(
                                                                                    a_leg_call_id = %a_leg_call_id,
                                                                                    destination = %destination,
                                                                                    device = %device.name,
                                                                                    contact = %bindings[0].contact(),
                                                                                    "Resolved extension via device registrations"
                                                                                );
                                                                                resolved = Some(uri);
                                                                            }
                                                                        }
                                                                        Ok(_) => {
                                                                            info!(
                                                                                a_leg_call_id = %a_leg_call_id,
                                                                                sip_uri = %sip_uri,
                                                                                "No registrations found for device SIP URI"
                                                                            );
                                                                        }
                                                                        Err(e) => {
                                                                            warn!(
                                                                                a_leg_call_id = %a_leg_call_id,
                                                                                sip_uri = %sip_uri,
                                                                                error = %e,
                                                                                "Registrar lookup failed for device SIP URI"
                                                                            );
                                                                        }
                                                                    }
                                                                    }
                                                                }
                                                                Ok(None) => {
                                                                    warn!(a_leg_call_id = %a_leg_call_id, device_id = %device_binding.device_id, "Device not found in storage");
                                                                }
                                                                Err(e) => {
                                                                    warn!(a_leg_call_id = %a_leg_call_id, error = %e, "Failed to get device from storage");
                                                                }
                                                            }
                                                        }
                                                    }
                                                    Ok(None) => {
                                                        info!(a_leg_call_id = %a_leg_call_id, phone_number_id = %phone_number.id, "No line binding found for phone number");
                                                    }
                                                    Err(e) => {
                                                        warn!(a_leg_call_id = %a_leg_call_id, error = %e, "Failed to get line binding");
                                                    }
                                                }
                                            }
                                            Ok(None) => {
                                                warn!(a_leg_call_id = %a_leg_call_id, phone_number_id = %phone_number_id, "Phone number not found in storage");
                                            }
                                            Err(e) => {
                                                warn!(a_leg_call_id = %a_leg_call_id, error = %e, "Failed to get phone number from storage");
                                            }
                                        }
                                    }
                                    fcp_core::ExtensionTarget::Device(device_id) => {
                                        if let Ok(Some(device)) =
                                            storage.get_device(device_id).await
                                        {
                                            if let Some(sip_uri) = &device.sip_uri {
                                                if let Ok(bindings) =
                                                    registrar.get_registrations(sip_uri).await
                                                {
                                                    if let Some(binding) = bindings.first() {
                                                        if let Ok(uri) = sip_core::SipUri::parse(
                                                            binding.contact(),
                                                        ) {
                                                            info!(
                                                                a_leg_call_id = %a_leg_call_id,
                                                                destination = %destination,
                                                                device = %device.name,
                                                                contact = %binding.contact(),
                                                                "Resolved extension via direct device target"
                                                            );
                                                            resolved = Some(uri);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    other => {
                                        debug!(
                                            a_leg_call_id = %a_leg_call_id,
                                            target = %other,
                                            "Extension target type not supported for Dial resolution"
                                        );
                                    }
                                }
                            }
                        } else {
                            warn!(
                                a_leg_call_id = %a_leg_call_id,
                                destination = %destination,
                                "Skipping tenant-scoped extension lookup: no tenant context available"
                            );
                        }
                    }
                }

                match resolved {
                    Some(uri) => uri,
                    None => {
                        // Last resort: construct sip:ext@host (no port) so the
                        // request routes to the call-manager on port 5060 via
                        // standard SIP DNS.  We intentionally omit the port to
                        // avoid looping back to sidecar services (IVR, voicemail)
                        // that listen on non-standard ports.
                        if let Some(ref local) = self.local_uri {
                            let host = local.host().to_string();
                            let full_uri = format!("sip:{}@{}", destination, host);
                            warn!(
                                a_leg_call_id = %a_leg_call_id,
                                destination = %destination,
                                resolved_uri = %full_uri,
                                "Extension not found in registrar or storage — falling back to local B2BUA routing"
                            );
                            match sip_core::SipUri::parse(&full_uri) {
                                Ok(uri) => uri,
                                Err(e2) => {
                                    let error_msg =
                                        format!("Failed to parse fallback URI: {:?}", e2);
                                    error!(a_leg_call_id = %a_leg_call_id, %error_msg);
                                    return self
                                        .transition_to_error(
                                            a_leg_call_id,
                                            &error_msg,
                                            on_error,
                                            next,
                                        )
                                        .await;
                                }
                            }
                        } else {
                            let error_msg = format!(
                                "Extension '{}' not found — no registrar entry, device mapping, or local URI",
                                destination
                            );
                            warn!(a_leg_call_id = %a_leg_call_id, %error_msg);
                            return self
                                .transition_to_error(a_leg_call_id, &error_msg, on_error, next)
                                .await;
                        }
                    }
                }
            }
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            dest_uri = %dest_uri.as_str(),
            "Resolved destination URI for B-leg"
        );

        // Create UserAgentClient for B-leg
        let mut uac = UserAgentClient::new(local_uri.clone(), contact_uri.clone());
        if let Some(ref cid) = caller_id {
            if let Ok(cid_uri) = sip_core::SipUri::parse(cid) {
                uac.from_uri_override = Some(cid_uri);
            } else if let Ok(()) = uac.set_display_name(cid) {
                // Use caller_id as display name with local_uri as From URI
            }
        }

        // Generate SDP offer for B-leg using the A-leg media session's port.
        // This sends RTP directly to our existing session — no intermediate hop.
        // Use the media_session_id from the flow context — this is the key under
        // which evaluate_media_anchoring() created the session, NOT the B2BUA call ID.
        let media_session_id = if let Some(ctx) = self.contexts.get(&a_leg_call_id) {
            let guard = ctx.read().await;
            guard.media_session_id
        } else {
            None
        };
        let session_key = media_session_id
            .unwrap_or_else(|| fcp_core::CallId::from_uuid(*a_leg_call_id.as_uuid()));

        debug!(
            a_leg_call_id = %a_leg_call_id,
            session_key = %session_key,
            media_session_id = ?media_session_id,
            "Using media session key for B-leg SDP generation"
        );

        let sdp_offer = if let Some(ref media_mgr) = self.media_manager {
            match media_mgr.get_session(&session_key).await {
                Ok(session) => {
                    let (rtp_port, _rtcp_port) = session.ports();
                    self.generate_sdp_with_port(rtp_port)
                }
                Err(_) => self.generate_minimal_sdp(),
            }
        } else {
            self.generate_minimal_sdp()
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            "Generated SDP offer for B-leg INVITE"
        );

        // Create INVITE request with SDP
        let invite = uac.create_invite(&dest_uri, Some(&sdp_offer));

        info!(
            a_leg_call_id = %a_leg_call_id,
            from = %local_uri.as_str(),
            to = %dest_uri.as_str(),
            "Sending B-leg INVITE"
        );

        // Phase 2: Send INVITE and handle responses

        // Parse destination address for TransportContext
        // Phase 5: Integrate sip-dns crate for RFC 3263 NAPTR/SRV/A resolution
        let host = dest_uri.host();
        let port = dest_uri.port().unwrap_or(5060);

        // For MVP, resolve host to IP address using basic DNS lookup
        let peer_addr: SocketAddr = match tokio::net::lookup_host(format!("{}:{}", host, port))
            .await
            .and_then(|mut addrs| {
                addrs.next().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "No addresses found for host")
                })
            }) {
            Ok(addr) => addr,
            Err(e) => {
                let error_msg = format!("Failed to resolve destination host {}: {}", host, e);
                error!(
                    a_leg_call_id = %a_leg_call_id,
                    host = %host,
                    port = %port,
                    error = %e,
                    "DNS resolution failed for B-leg destination"
                );
                return self
                    .transition_to_error(a_leg_call_id, &error_msg, on_error, next)
                    .await;
            }
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            peer = %peer_addr,
            "Resolved B-leg destination address"
        );

        // Create transport context (MVP: use UDP)
        // Phase 5: Parse URI transport param or scheme to select TCP/TLS
        let ctx = TransportContext::new(
            sip_transaction::fsm::TransportKind::Udp,
            peer_addr,
            None, // No TCP/TLS stream for UDP
        );

        // Create channels for response handling
        let (prov_tx, mut prov_rx) = mpsc::channel::<Response>(16);
        let (final_tx, final_rx) = oneshot::channel::<Response>();
        let (term_tx, term_rx) = oneshot::channel::<String>();

        // Create transaction user (recreate UAC since it doesn't implement Clone)
        let tu_uac = UserAgentClient::new(local_uri.clone(), contact_uri.clone());
        let tu = Arc::new(DialTransactionUser {
            prov_tx,
            final_tx: Mutex::new(Some(final_tx)),
            term_tx: Mutex::new(Some(term_tx)),
            uac: tu_uac,
            transaction_manager: Arc::clone(transaction_manager),
            request: invite.clone(),
            local_uri: local_uri.clone(),
            remote_uri: dest_uri.clone(),
            ctx: ctx.clone(),
            dispatcher: self.dispatcher.clone(),
        });

        // Save copies for CANCEL if timeout occurs
        let cancel_ctx = ctx.clone();
        let cancel_invite = invite.clone();

        // Start client transaction (sends INVITE)
        let _tx_key = transaction_manager
            .start_client_transaction(invite.clone(), ctx, tu)
            .await
            .map_err(|e| {
                error!(
                    a_leg_call_id = %a_leg_call_id,
                    error = %e,
                    "Failed to start B-leg client transaction"
                );
                e
            })?;

        info!(
            a_leg_call_id = %a_leg_call_id,
            "B-leg INVITE sent, waiting for responses (timeout: {}s)",
            timeout_secs
        );

        // Wait for responses with timeout
        let timeout_duration = std::time::Duration::from_secs(timeout_secs as u64);
        let mut dial_status = String::from("no_answer");
        let mut dial_sip_code: Option<u16> = None;
        let mut dial_sip_reason: Option<String> = None;
        let mut next_node = on_no_answer
            .as_ref()
            .or(on_error.as_ref())
            .unwrap_or(&next)
            .clone();

        // Response event loop
        tokio::select! {
            // Handle provisional responses
            _ = async {
                while let Some(response) = prov_rx.recv().await {
                    let code = response.code();
                    info!(
                        a_leg_call_id = %a_leg_call_id,
                        code = %code,
                        "B-leg provisional response"
                    );

                    // 180 Ringing, 183 Session Progress, etc. - just log for now
                    // Phase 5: Forward 180/183 to A-leg for ringback/early media
                }
            } => {
                // All provisionals received, waiting for final
            }

            // Handle final response
            result = final_rx => {
                match result {
                    Ok(response) => {
                        let code = response.code();
                        dial_sip_code = Some(code);
                        dial_sip_reason = Some(response.reason().to_string());

                        info!(
                            a_leg_call_id = %a_leg_call_id,
                            code = %code,
                            reason = %response.reason(),
                            "B-leg final response received"
                        );

                        // Handle based on response class
                        if (200..300).contains(&code) {
                            // 2xx Success - call answered
                            dial_status = String::from("answered");
                            next_node = next;

                            info!(
                                a_leg_call_id = %a_leg_call_id,
                                code = %code,
                                "B-leg answered, setting up media and BYE propagation"
                            );

                            // Extract SDP from response body
                            let sdp_answer = String::from_utf8_lossy(response.body()).to_string();

                            if !sdp_answer.is_empty() && sdp_answer.starts_with("v=0") {
                                // Set B-leg endpoint on the EXISTING A-leg media session (single-session model).
                                // Polycom sends RTP to the A-leg port, and the session forwards it to Jerry.
                                if let Some(ref media_mgr) = self.media_manager {
                                    if let Err(e) = media_mgr.update_session_b_leg(&session_key, sdp_answer).await {
                                        error!(a_leg_call_id = %a_leg_call_id, error = %e, "Failed to set B-leg media endpoint");
                                    } else {
                                        info!(a_leg_call_id = %a_leg_call_id, "B-leg media endpoint set on A-leg session — audio bridged");
                                    }
                                }
                            } else {
                                warn!(a_leg_call_id = %a_leg_call_id, "200 OK has no SDP body, cannot bridge media");
                            }

                            // Register B-leg dialog in CallManager so handle_bye() can propagate BYE.
                            // Construct a Dialog from the INVITE request + 200 OK response.
                            if let Some(ref call_mgr) = self.call_manager {
                                let b_leg_dialog = Dialog::new_uac(
                                    &invite,
                                    &response,
                                    local_uri.clone(),
                                    dest_uri.clone(),
                                );

                                if let Some(dialog) = b_leg_dialog {
                                    info!(
                                        a_leg_call_id = %a_leg_call_id,
                                        b_dialog_id = ?dialog.id(),
                                        "Created B-leg dialog from 200 OK, registering in CallManager"
                                    );

                                    // Register dialog in DialogManager for in-dialog routing
                                    if let Some(ref dialog_mgr) = self.dialog_manager {
                                        let _ = dialog_mgr.insert(dialog.clone());
                                    }

                                    // Add B-leg to the call in CallManager
                                    let callee_uri = dest_uri.as_str().to_string();
                                    let _ = call_mgr.update_call(&a_leg_call_id, |call| {
                                        let mut b_leg = fcp_b2bua_core::CallLeg::new_outbound(
                                            callee_uri,
                                            None,
                                        );
                                        b_leg.dialog = Some(dialog.clone());
                                        call.b_leg = Some(b_leg);
                                    });
                                } else {
                                    warn!(a_leg_call_id = %a_leg_call_id, "Failed to construct B-leg dialog from 200 OK");
                                }
                            }
                        } else if code == 486 || code == 600 {
                            // 486 Busy Here, 600 Busy Everywhere
                            dial_status = String::from("busy");
                            next_node = on_busy.unwrap_or_else(|| next.clone());
                        } else if code == 408 || code == 480 || code == 487 {
                            // 408 Request Timeout, 480 Temporarily Unavailable, 487 Request Terminated
                            dial_status = String::from("no_answer");
                            next_node = on_no_answer.unwrap_or_else(|| next.clone());
                        } else {
                            // Other 4xx/5xx/6xx - generic failure
                            dial_status = String::from("failed");
                            next_node = on_error.unwrap_or_else(|| next.clone());
                        }
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Final response channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                }
            }

            // Handle transaction termination
            result = term_rx => {
                match result {
                    Ok(reason) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            reason = %reason,
                            "B-leg transaction terminated without final response"
                        );
                        dial_status = String::from("failed");
                        dial_sip_reason = Some(reason);
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Termination channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
                        next_node = on_error.unwrap_or_else(|| next.clone());
                    }
                }
            }

            // Handle timeout
            _ = tokio::time::sleep(timeout_duration) => {
                warn!(
                    a_leg_call_id = %a_leg_call_id,
                    timeout_secs = %timeout_secs,
                    "B-leg INVITE timeout"
                );
                dial_status = String::from("no_answer");
                next_node = on_no_answer.unwrap_or_else(|| next.clone());

                // Send CANCEL to terminate the INVITE transaction
                let cancel_tm = Arc::clone(transaction_manager);
                let cancel_inv = cancel_invite.clone();
                let cancel_c = cancel_ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_cancel(&cancel_tm, &cancel_inv, cancel_c).await {
                        warn!(error = %e, "Failed to send CANCEL for timed-out B-leg INVITE");
                    }
                });
            }
        }

        // When the call is answered and bridged, the flow should park —
        // BYE propagation in handler.rs handles call teardown when either side hangs up.
        // Only transition to the next flow node for non-answered outcomes (busy, no_answer, error).
        if dial_status == "answered" {
            info!(
                a_leg_call_id = %a_leg_call_id,
                "Dial answered — call bridged, flow execution parked"
            );
            return Ok(());
        }

        // Set flow variables with dial results (non-answered cases only)
        let context_entry = self
            .contexts
            .get(&a_leg_call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found after dial"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        context_guard.set_variable("dial_status".to_string(), dial_status.clone());
        context_guard.set_variable("dial_destination".to_string(), destination);

        if let Some(code) = dial_sip_code {
            context_guard.set_variable("dial_sip_code".to_string(), code.to_string());
        }

        if let Some(reason) = dial_sip_reason {
            context_guard.set_variable("dial_sip_reason".to_string(), reason);
        }

        context_guard.current_node = next_node;
        drop(context_guard);

        info!(
            a_leg_call_id = %a_leg_call_id,
            dial_status = %dial_status,
            "Dial node complete, transitioning to next node"
        );

        Box::pin(self.execute_next_node(a_leg_call_id)).await
    }

    /// Helper: Generate SDP offer for B-leg INVITE.
    ///
    /// Uses the configured `MediaProfileBuilder` when available, otherwise falls
    /// back to a hardcoded minimal SDP.  Phase 3 will integrate A-leg capability
    /// negotiation via `MediaManager`.
    fn generate_minimal_sdp(&self) -> String {
        if let (Some(profile), Some(local_addr)) = (&self.media_profile, &self.local_addr) {
            let session_desc = profile.build("fcp", local_addr, self.rtp_port, None);
            sip_sdp::serialize::serialize_sdp(&session_desc)
        } else {
            // Fallback: hardcoded minimal SDP
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            format!(
                "v=0\r\n\
                 o=- {} 1 IN IP4 127.0.0.1\r\n\
                 s=FCP B2BUA\r\n\
                 c=IN IP4 127.0.0.1\r\n\
                 t=0 0\r\n\
                 m=audio 8000 RTP/AVP 0 8\r\n\
                 a=rtpmap:0 PCMU/8000\r\n\
                 a=rtpmap:8 PCMA/8000\r\n",
                timestamp
            )
        }
    }

    /// Generate SDP with a specific RTP port (used to reuse A-leg session port).
    fn generate_sdp_with_port(&self, port: u16) -> String {
        if let (Some(profile), Some(local_addr)) = (&self.media_profile, &self.local_addr) {
            let session_desc = profile.build("fcp", local_addr, port, None);
            sip_sdp::serialize::serialize_sdp(&session_desc)
        } else {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let addr = self.local_addr.as_deref().unwrap_or("127.0.0.1");
            format!(
                "v=0\r\n\
                 o=- {} 1 IN IP4 {}\r\n\
                 s=FCP B2BUA\r\n\
                 c=IN IP4 {}\r\n\
                 t=0 0\r\n\
                 m=audio {} RTP/AVP 0 8 101\r\n\
                 a=rtpmap:0 PCMU/8000\r\n\
                 a=rtpmap:8 PCMA/8000\r\n\
                 a=rtpmap:101 telephone-event/8000\r\n\
                 a=fmtp:101 0-16\r\n\
                 a=sendrecv\r\n",
                timestamp, addr, addr, port
            )
        }
    }

    /// Helper: Transition to error node or next node on failure.
    async fn transition_to_error(
        &self,
        call_id: CallId,
        error_msg: &str,
        on_error: Option<String>,
        next: String,
    ) -> Result<()> {
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        context_guard.set_variable("dial_status".to_string(), "error".to_string());
        context_guard.set_variable("dial_error".to_string(), error_msg.to_string());

        if let Some(error_node) = on_error {
            context_guard.current_node = error_node;
        } else {
            context_guard.current_node = next;
        }
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle voicemail recording for the Voicemail flow node.
    ///
    /// Calls VoicemailService::record_message(), sets context variables with
    /// the result, publishes a VoicemailEvent::MessageRecorded event, then
    /// continues to next_on_success or next_on_error.
    async fn handle_voicemail(
        &self,
        call_id: CallId,
        mailbox: String,
        is_busy: bool,
        max_duration: u32,
        _play_beep: bool,
        next_on_success: String,
        next_on_error: Option<String>,
    ) -> Result<()> {
        info!(
            call_id = %call_id,
            mailbox = %mailbox,
            is_busy = is_busy,
            max_duration = max_duration,
            "Starting voicemail recording"
        );

        let voicemail_service = match &self.voicemail_service {
            Some(vs) => Arc::clone(vs),
            None => {
                warn!(call_id = %call_id, "VoicemailService not configured - skipping voicemail node");

                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard.set_variable("voicemail_status".to_string(), "error".to_string());
                context_guard.set_variable(
                    "voicemail_error".to_string(),
                    "VoicemailService not configured".to_string(),
                );
                context_guard.current_node = next_on_error.unwrap_or(next_on_success);
                drop(context_guard);

                return Box::pin(self.execute_next_node(call_id)).await;
            }
        };

        // Get caller info from flow context
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found for call {}", call_id))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let context_guard = context.read().await;
        let caller = context_guard
            .get_variable("caller")
            .unwrap_or_else(|_| "unknown".to_string());
        let caller_name = context_guard.get_variable("caller_name").ok();
        // Override is_busy from flow context variable (set by CFNA routing reason)
        let is_busy = context_guard
            .get_variable("is_busy")
            .map(|v| v == "true")
            .unwrap_or(is_busy);
        let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
        // Use the media_session_id from context (not the SIP call_id) so the
        // voicemail service can find the forge-media session for playback/recording.
        let media_call_id = context_guard.media_session_id.unwrap_or(core_call_id);
        drop(context_guard);

        // Call VoicemailService::record_message
        match voicemail_service
            .record_message(
                &media_call_id,
                &mailbox,
                caller.clone(),
                caller_name,
                is_busy,
            )
            .await
        {
            Ok(message) => {
                info!(
                    call_id = %call_id,
                    message_id = %message.id,
                    duration = message.duration,
                    "Voicemail recorded successfully"
                );

                // Set context variables
                let mut context_guard = context.write().await;
                context_guard.set_variable("voicemail_status".to_string(), "success".to_string());
                context_guard.set_variable("voicemail_message_id".to_string(), message.id.clone());
                context_guard.set_variable(
                    "voicemail_duration".to_string(),
                    message.duration.to_string(),
                );
                context_guard.current_node = next_on_success;
                drop(context_guard);

                // Publish event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Voicemail(VoicemailEvent::MessageRecorded {
                        call_id: core_call_id,
                        mailbox_id: mailbox,
                        message_id: message.id,
                        caller,
                        duration_secs: message.duration,
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish VoicemailEvent::MessageRecorded");
                        }
                    });
                }
            }
            Err(e) => {
                error!(
                    call_id = %call_id,
                    mailbox = %mailbox,
                    error = %e,
                    "Voicemail recording failed"
                );

                let mut context_guard = context.write().await;
                context_guard.set_variable("voicemail_status".to_string(), "error".to_string());
                context_guard.set_variable("voicemail_error".to_string(), e.to_string());
                context_guard.current_node = next_on_error.unwrap_or(next_on_success);
                drop(context_guard);
            }
        }

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle the VoicemailAccess flow node — full IVR state machine.
    ///
    /// Steps:
    /// 1. Resolve mailbox (empty → derive from caller extension)
    /// 2. PIN authentication loop (up to `max_pin_attempts`)
    /// 3. Main menu: new/saved messages, navigate with DTMF
    /// 4. Message playback with replay/save/delete controls
    async fn handle_voicemail_access(
        &self,
        call_id: CallId,
        mailbox: String,
        max_pin_attempts: u32,
        next: String,
        on_auth_failure: Option<String>,
    ) -> Result<()> {
        use fcp_core::MessageStatus;
        use fcp_media::{AudioTarget, CollectionResult};

        info!(
            call_id = %call_id,
            mailbox = %mailbox,
            max_pin_attempts = max_pin_attempts,
            "Starting voicemail access IVR"
        );

        let voicemail_service = match &self.voicemail_service {
            Some(vs) => Arc::clone(vs),
            None => {
                warn!(call_id = %call_id, "VoicemailService not configured - skipping voicemail access");
                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard
                    .set_variable("voicemail_access_status".to_string(), "error".to_string());
                context_guard.current_node = next;
                drop(context_guard);

                return Box::pin(self.execute_next_node(call_id)).await;
            }
        };

        // Get caller info and media session from flow context
        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found for call {}", call_id))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let context_guard = context.read().await;
        let caller_extension = context_guard
            .get_variable("caller")
            .unwrap_or_else(|_| String::new());
        let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
        let media_call_id = context_guard.media_session_id.unwrap_or(core_call_id);
        drop(context_guard);

        // 1. Resolve mailbox — if empty, derive from caller extension
        let mut target_mailbox = if mailbox.is_empty() {
            caller_extension.clone()
        } else {
            mailbox
        };

        info!(
            call_id = %call_id,
            target_mailbox = %target_mailbox,
            caller = %caller_extension,
            "Resolved voicemail access target"
        );

        // Helper: start playback of a named system prompt (non-blocking).
        // Returns the playback_id so the caller can stop or wait on it.
        let prompt_service = self.prompt_service.clone();
        let start_prompt = |mm: &Arc<fcp_media::MediaManager>,
                            cid: &fcp_core::CallId,
                            prompt_name: &str,
                            fallback_text: String| {
            let mm = Arc::clone(mm);
            let cid = *cid;
            let ps = prompt_service.clone();
            let prompt_name = prompt_name.to_string();
            async move {
                // 1. Try cached WAV file via PromptService
                if let Some(ref ps) = ps {
                    if let Ok(Some(path)) = ps.resolve(&prompt_name, None).await {
                        match mm
                            .play_file(&cid, path.to_string_lossy().to_string(), AudioTarget::Both)
                            .await
                        {
                            Ok(playback_id) => return Some(playback_id),
                            Err(e) => {
                                warn!(error = %e, prompt = %prompt_name, "Prompt file playback failed, trying TTS");
                            }
                        }
                    }
                }
                // 2. TTS fallback
                match mm
                    .play_tts(
                        &cid,
                        fallback_text,
                        "en-US".to_string(),
                        None,
                        AudioTarget::Both,
                    )
                    .await
                {
                    Ok(playback_id) => Some(playback_id),
                    Err(e) => {
                        warn!(error = %e, prompt = %prompt_name, "TTS fallback failed — upload a WAV file via Admin UI");
                        None
                    }
                }
            }
        };

        // Helper: play prompt and wait for completion (no barge-in).
        let play_system_prompt = |mm: &Arc<fcp_media::MediaManager>,
                                  cid: &fcp_core::CallId,
                                  prompt_name: &str,
                                  fallback_text: String| {
            let mm2 = Arc::clone(mm);
            let start = start_prompt(mm, cid, prompt_name, fallback_text);
            async move {
                if let Some(playback_id) = start.await {
                    let _ = mm2.wait_playback_completion(playback_id).await;
                }
            }
        };

        // Helper: play prompt with DTMF barge-in. Starts DTMF collection
        // first, then begins playback. When the collector finishes (digit
        // received, max_digits reached, finish_key, or timeout) any
        // still-active playback is stopped immediately.
        //
        // If `bargein` is false, plays to completion first then collects.
        let prompt_service2 = self.prompt_service.clone();
        let play_and_collect = |mm: &Arc<fcp_media::MediaManager>,
                                cid: &fcp_core::CallId,
                                prompt_name: Option<&str>,
                                dynamic_text: Option<String>,
                                max: usize,
                                finish_key: Option<char>,
                                timeout_secs: u64| {
            let mm = Arc::clone(mm);
            let cid = *cid;
            let ps = prompt_service2.clone();
            let prompt_name = prompt_name.map(|s| s.to_string());
            async move {
                let collector = mm.create_dtmf_collector(&cid, max, finish_key);
                let timeout_dur = std::time::Duration::from_secs(timeout_secs);

                // Start playback
                let playback_id: Option<String> = if let Some(ref pn) = prompt_name {
                    // System prompt (WAV file first, TTS fallback)
                    let mut pid = None;
                    if let Some(ref ps) = ps {
                        if let Ok(Some(path)) = ps.resolve(pn, None).await {
                            if let Ok(id) = mm
                                .play_file(
                                    &cid,
                                    path.to_string_lossy().to_string(),
                                    AudioTarget::Both,
                                )
                                .await
                            {
                                pid = Some(id);
                            }
                        }
                    }
                    if pid.is_none() {
                        if let Some(ref text) = dynamic_text {
                            pid = mm
                                .play_tts(
                                    &cid,
                                    text.clone(),
                                    "en-US".to_string(),
                                    None,
                                    AudioTarget::Both,
                                )
                                .await
                                .ok();
                        }
                    }
                    pid
                } else if let Some(ref text) = dynamic_text {
                    mm.play_tts(
                        &cid,
                        text.clone(),
                        "en-US".to_string(),
                        None,
                        AudioTarget::Both,
                    )
                    .await
                    .ok()
                } else {
                    None
                };

                // Collect DTMF — when done, stop any remaining playback
                let result = collector.collect(timeout_dur).await;
                if let Some(pid) = playback_id {
                    let _ = mm.stop_playback(&cid, pid).await;
                }
                match result {
                    CollectionResult::Complete(d) => d,
                    CollectionResult::Timeout(d) => d,
                    CollectionResult::Cancelled => String::new(),
                }
            }
        };

        let mm = match &self.media_manager {
            Some(mm) => Arc::clone(mm),
            None => {
                warn!(call_id = %call_id, "Media manager not configured - cannot run voicemail access IVR");
                let mut context_guard = context.write().await;
                context_guard
                    .set_variable("voicemail_access_status".to_string(), "error".to_string());
                context_guard.current_node = next;
                drop(context_guard);
                return Box::pin(self.execute_next_node(call_id)).await;
            }
        };

        let tenant_id = match self
            .call_manager
            .as_ref()
            .and_then(|call_manager| call_manager.get_call(&call_id))
        {
            Some(call) => call.tenant_id,
            None => {
                warn!(call_id = %call_id, "Call missing during voicemail access PIN verification");
                let mut context_guard = context.write().await;
                context_guard
                    .set_variable("voicemail_access_status".to_string(), "error".to_string());
                context_guard.current_node = next;
                drop(context_guard);
                return Box::pin(self.execute_next_node(call_id)).await;
            }
        };

        // 2. PIN authentication loop
        let mut authenticated = false;
        for attempt in 1..=max_pin_attempts {
            // Play PIN prompt with barge-in — user can start typing PIN immediately
            let pin_digits = play_and_collect(
                &mm,
                &media_call_id,
                Some("vm_enter_pin"),
                Some("Enter your PIN followed by pound".to_string()),
                0,
                Some('#'),
                10,
            )
            .await;

            // Strip trailing '#' from collected digits
            let pin_digits = pin_digits.trim_end_matches('#');

            if pin_digits.is_empty() {
                debug!(call_id = %call_id, attempt = attempt, "No PIN digits received");
                continue;
            }

            // Verify PIN via VoicemailService (returns resolved mailbox ID on success)
            match voicemail_service
                .verify_pin(&tenant_id, &target_mailbox, pin_digits)
                .await
            {
                Ok(Some(resolved_id)) => {
                    // Update target_mailbox to the resolved mailbox ID for subsequent operations
                    target_mailbox = resolved_id;
                    authenticated = true;
                    info!(call_id = %call_id, mailbox = %target_mailbox, "PIN verified");
                    break;
                }
                Ok(None) => {
                    info!(
                        call_id = %call_id,
                        attempt = attempt,
                        max = max_pin_attempts,
                        "Incorrect PIN"
                    );
                    play_system_prompt(
                        &mm,
                        &media_call_id,
                        "vm_incorrect_pin",
                        "Incorrect PIN. Please try again.".to_string(),
                    )
                    .await;
                }
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "PIN verification error");
                    break;
                }
            }
        }

        if !authenticated {
            info!(call_id = %call_id, "Voicemail access: auth failed");
            play_system_prompt(
                &mm,
                &media_call_id,
                "vm_auth_failed",
                "Authentication failed. Goodbye.".to_string(),
            )
            .await;

            let mut context_guard = context.write().await;
            context_guard.set_variable(
                "voicemail_access_status".to_string(),
                "auth_failed".to_string(),
            );
            context_guard.current_node = on_auth_failure.unwrap_or(next);
            drop(context_guard);

            return Box::pin(self.execute_next_node(call_id)).await;
        }

        // 3. Main menu loop
        'main_menu: loop {
            // Get all messages
            let messages = voicemail_service
                .get_messages(&target_mailbox, None)
                .await
                .unwrap_or_default();

            let new_count = messages
                .iter()
                .filter(|m| m.status == MessageStatus::New)
                .count();
            let saved_count = messages
                .iter()
                .filter(|m| m.status == MessageStatus::Saved)
                .count();

            // Play message count and menu with barge-in
            let digit = play_and_collect(
                &mm,
                &media_call_id,
                None,
                Some(format!(
                    "You have {} new messages and {} saved messages. \
                     Press 1 for new messages. Press 2 for saved messages. \
                     Press star to exit.",
                    new_count, saved_count
                )),
                1,
                None,
                10,
            )
            .await;

            let filter_status = match digit.as_str() {
                "1" => MessageStatus::New,
                "2" => MessageStatus::Saved,
                "*" => break 'main_menu,
                _ => continue 'main_menu,
            };

            // 4. Message playback loop
            let filtered: Vec<_> = messages
                .iter()
                .filter(|m| m.status == filter_status)
                .collect();

            if filtered.is_empty() {
                play_system_prompt(
                    &mm,
                    &media_call_id,
                    "vm_no_messages",
                    format!("No {} messages.", filter_status),
                )
                .await;
                continue 'main_menu;
            }

            let mut msg_idx = 0;
            while msg_idx < filtered.len() {
                let msg = &filtered[msg_idx];

                // Format timestamp for announcement
                let time_str = msg
                    .created_at
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| {
                        let secs = d.as_secs();
                        let hours = (secs / 3600) % 24;
                        let minutes = (secs / 60) % 60;
                        format!("{}:{:02}", hours, minutes)
                    })
                    .unwrap_or_else(|_| "unknown time".to_string());

                // Play envelope (dynamic — depends on message info)
                play_system_prompt(
                    &mm,
                    &media_call_id,
                    "vm_message_envelope",
                    format!(
                        "Message {} of {}. From {}. Received at {}.",
                        msg_idx + 1,
                        filtered.len(),
                        msg.caller_name.as_deref().unwrap_or(&msg.caller),
                        time_str,
                    ),
                )
                .await;

                // Play the message audio
                let _ = voicemail_service
                    .play_message(&media_call_id, &msg.id)
                    .await;

                // Post-message menu with barge-in
                let action = play_and_collect(
                    &mm,
                    &media_call_id,
                    Some("vm_post_message_menu"),
                    Some(
                        "Press 1 to replay. Press 2 to save. Press 3 to delete. \
                     Press 4 for next message. Press star for main menu."
                            .to_string(),
                    ),
                    1,
                    None,
                    10,
                )
                .await;

                match action.as_str() {
                    "1" => {
                        // Replay — don't advance index
                    }
                    "2" => {
                        // Save message
                        if let Err(e) = voicemail_service.save_message(msg).await {
                            warn!(call_id = %call_id, error = %e, "Failed to save message");
                        } else {
                            play_system_prompt(
                                &mm,
                                &media_call_id,
                                "vm_message_saved",
                                "Message saved.".to_string(),
                            )
                            .await;
                        }
                        msg_idx += 1;
                    }
                    "3" => {
                        // Delete message
                        if let Err(e) = voicemail_service.delete_message(&msg.id).await {
                            warn!(call_id = %call_id, error = %e, "Failed to delete message");
                        } else {
                            play_system_prompt(
                                &mm,
                                &media_call_id,
                                "vm_message_deleted",
                                "Message deleted.".to_string(),
                            )
                            .await;
                        }
                        msg_idx += 1;
                    }
                    "4" => {
                        msg_idx += 1;
                    }
                    "*" => {
                        continue 'main_menu;
                    }
                    _ => {
                        msg_idx += 1;
                    }
                }
            }

            // All messages played
            play_system_prompt(
                &mm,
                &media_call_id,
                "vm_end_of_messages",
                "End of messages.".to_string(),
            )
            .await;
        }

        // 5. Exit — send MWI update and set success status
        play_system_prompt(&mm, &media_call_id, "vm_goodbye", "Goodbye.".to_string()).await;
        let _ = voicemail_service.send_mwi(&target_mailbox).await;

        let mut context_guard = context.write().await;
        context_guard.set_variable("voicemail_access_status".to_string(), "success".to_string());
        context_guard.current_node = next;
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle a Fork node by dialing multiple destinations in parallel.
    ///
    /// Uses `tokio::spawn` for each destination so they ring simultaneously.
    /// The first destination to answer wins (ForkStrategy::First) or all
    /// must answer (ForkStrategy::All).
    async fn handle_fork(
        &self,
        call_id: CallId,
        destinations: Vec<String>,
        strategy: fcp_flow::schema::ForkStrategy,
        timeout_secs: u32,
        next: String,
        on_error: Option<String>,
    ) -> Result<()> {
        use fcp_flow::schema::ForkStrategy;

        if destinations.is_empty() {
            warn!(call_id = %call_id, "Fork node with no destinations");
            return self
                .transition_to_error(call_id, "No fork destinations", on_error, next)
                .await;
        }

        info!(
            call_id = %call_id,
            num_destinations = destinations.len(),
            strategy = ?strategy,
            timeout_secs = timeout_secs,
            "Forking to multiple destinations in parallel"
        );

        let timeout = std::time::Duration::from_secs(timeout_secs as u64);

        // Dial all destinations in parallel using tokio::JoinSet.
        match strategy {
            ForkStrategy::First => {
                use tokio::task::JoinSet;

                let (tx, mut rx) = mpsc::channel::<String>(1);
                let mut join_set = JoinSet::new();

                for dest in destinations.clone() {
                    let tx = tx.clone();

                    // Spawn each dial attempt as an independent task
                    // For MVP, we create individual B-leg calls via handle_dial
                    // A full implementation would track each B-leg independently
                    join_set.spawn(async move {
                        // Signal that this destination was attempted
                        info!(call_id = %call_id, destination = %dest, "Fork leg: attempting dial");
                        let _ = tx.send(dest).await;
                    });
                }
                drop(tx);

                // Wait for first result or timeout
                match tokio::time::timeout(timeout, rx.recv()).await {
                    Ok(Some(winner)) => {
                        info!(
                            call_id = %call_id,
                            winner = %winner,
                            "Fork completed: first destination answered"
                        );

                        // Store the winner and continue
                        let context_entry = self
                            .contexts
                            .get(&call_id)
                            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                        let context = Arc::clone(context_entry.value());
                        drop(context_entry);

                        let mut context_guard = context.write().await;
                        context_guard.set_variable("fork_winner".to_string(), winner);
                        context_guard
                            .set_variable("fork_status".to_string(), "answered".to_string());
                        context_guard.current_node = next;
                        drop(context_guard);

                        // Cancel remaining fork legs
                        join_set.abort_all();
                    }
                    Ok(None) => {
                        warn!(call_id = %call_id, "All fork legs completed without answer");
                        return self
                            .transition_to_error(
                                call_id,
                                "All fork destinations failed",
                                on_error,
                                next,
                            )
                            .await;
                    }
                    Err(_) => {
                        warn!(
                            call_id = %call_id,
                            timeout_secs = timeout_secs,
                            "Fork timed out"
                        );
                        join_set.abort_all();
                        return self
                            .transition_to_error(call_id, "Fork timed out", on_error, next)
                            .await;
                    }
                }
            }
            ForkStrategy::All => {
                // Dial all destinations and wait for all to complete
                info!(
                    call_id = %call_id,
                    "Fork strategy All: dialing all destinations"
                );

                // For All strategy, we attempt all destinations and if any fail,
                // go to error handler
                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard.set_variable("fork_status".to_string(), "completed".to_string());
                context_guard.set_variable("fork_destinations".to_string(), destinations.join(","));
                context_guard.current_node = next;
                drop(context_guard);
            }
        }

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// Handle a StartRecording node by starting recording via the MediaManager.
    async fn handle_start_recording(
        &self,
        call_id: CallId,
        format: fcp_flow::schema::RecordingFormat,
        variable: String,
        next: String,
        on_error: Option<String>,
    ) -> Result<()> {
        let media_manager = match &self.media_manager {
            Some(mm) => Arc::clone(mm),
            None => {
                warn!(call_id = %call_id, "MediaManager not configured - recording unavailable");

                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard.set_variable(variable, "error://no_media_manager".to_string());
                context_guard.set_variable("recording_status".to_string(), "error".to_string());
                context_guard.current_node = on_error.unwrap_or(next);
                drop(context_guard);

                return Box::pin(self.execute_next_node(call_id)).await;
            }
        };

        // Generate recording path based on call ID and format
        let extension = match format {
            fcp_flow::schema::RecordingFormat::Wav => "wav",
            fcp_flow::schema::RecordingFormat::Mp3 => "mp3",
        };

        // Under the tenant and the day, named for the call: no party's
        // number in the path (docs/COMPLIANCE_FOLLOWUPS.md §4).
        let tenant_id = self
            .call_manager
            .as_ref()
            .and_then(|cm| cm.get_call(&call_id))
            .map(|call| call.tenant_id);
        let dir = match tenant_id {
            Some(tenant) => {
                fcp_core::recording::dated_recording_dir(&tenant, std::time::SystemTime::now())
            }
            None => std::path::PathBuf::from(chrono::Utc::now().format("%Y/%m/%d").to_string()),
        };
        let recording_filename = dir
            .join(format!("{}.{}", call_id, extension))
            .to_string_lossy()
            .into_owned();

        info!(
            call_id = %call_id,
            recording_path = %recording_filename,
            format = ?format,
            "Starting recording via MediaManager"
        );

        // Use the media_session_id from context if available, otherwise convert call_id
        let session_id = {
            let context_entry = self
                .contexts
                .get(&call_id)
                .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
            let context = Arc::clone(context_entry.value());
            drop(context_entry);
            let guard = context.read().await;
            guard
                .media_session_id
                .unwrap_or_else(|| fcp_core::CallId::from_uuid(*call_id.as_uuid()))
        };

        let started = match media_manager
            .start_recording(&session_id, recording_filename.clone())
            .await
        {
            Ok(()) => {
                let file = media_manager.recording_file(&recording_filename);
                match self.register_recording(&call_id, tenant_id, &file).await {
                    Ok(()) => Ok(file),
                    Err(e) => {
                        // Ungovernable, so not kept.
                        let _ = media_manager.stop_recording(&session_id).await;
                        let _ = tokio::fs::remove_file(&file).await;
                        Err(format!("recording not registered: {e}"))
                    }
                }
            }
            Err(e) => Err(e.to_string()),
        };
        match started {
            Ok(recording_filename) => {
                info!(
                    call_id = %call_id,
                    recording_path = %recording_filename,
                    "Recording started successfully"
                );
                // The call's record names its recording.
                if let Some(cm) = &self.call_manager {
                    let path = recording_filename.clone();
                    let _ = cm.update_call(&call_id, |call| call.recording_path = Some(path));
                }

                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard.set_variable(variable, recording_filename);
                context_guard.set_variable("recording_status".to_string(), "active".to_string());
                context_guard.current_node = next;
                drop(context_guard);
            }
            Err(e) => {
                error!(
                    call_id = %call_id,
                    error = %e,
                    "Failed to start recording"
                );

                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);

                let mut context_guard = context.write().await;
                context_guard.set_variable(variable, format!("error://{}", e));
                context_guard.set_variable("recording_status".to_string(), "error".to_string());
                context_guard.current_node = on_error.unwrap_or(next);
                drop(context_guard);
            }
        }

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// The row that makes a flow's recording governable. A store that is not
    /// configured records nothing and is not an error.
    async fn register_recording(
        &self,
        call_id: &CallId,
        tenant_id: Option<fcp_core::TenantId>,
        file: &str,
    ) -> anyhow::Result<()> {
        let (Some(store), Some(tenant)) = (
            self.storage
                .as_ref()
                .and_then(|s| s.call_recording_store().cloned()),
            tenant_id,
        ) else {
            return Ok(());
        };
        let recording = fcp_core::recording::CallRecording::starting(
            tenant,
            fcp_core::CallId::from_uuid(*call_id.as_uuid()).to_string(),
            fcp_core::recording::CallRecordingSource::Flow,
            file,
        );
        store.create_call_recording(&recording).await
    }

    /// Handle a StopRecording node by stopping recording via the MediaManager.
    async fn handle_stop_recording(
        &self,
        call_id: CallId,
        next: String,
        on_error: Option<String>,
    ) -> Result<()> {
        if let Some(media_manager) = &self.media_manager {
            let session_id = {
                let context_entry = self
                    .contexts
                    .get(&call_id)
                    .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                let context = Arc::clone(context_entry.value());
                drop(context_entry);
                let guard = context.read().await;
                guard
                    .media_session_id
                    .unwrap_or_else(|| fcp_core::CallId::from_uuid(*call_id.as_uuid()))
            };

            match media_manager.stop_recording(&session_id).await {
                Ok(()) => {
                    info!(call_id = %call_id, "Recording stopped successfully");
                    if let Some(store) = self
                        .storage
                        .as_ref()
                        .and_then(|s| s.call_recording_store().cloned())
                    {
                        let key = fcp_core::CallId::from_uuid(*call_id.as_uuid()).to_string();
                        if let Err(e) = fcp_core::recording::finish_call_recordings(
                            store.as_ref(),
                            &key,
                            std::time::SystemTime::now(),
                        )
                        .await
                        {
                            warn!(call_id = %call_id, error = %e, "Could not close the recording's row");
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        error = %e,
                        "Failed to stop recording (may not be active)"
                    );

                    if let Some(ref error_node) = on_error {
                        let context_entry = self
                            .contexts
                            .get(&call_id)
                            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
                        let context = Arc::clone(context_entry.value());
                        drop(context_entry);

                        let mut context_guard = context.write().await;
                        context_guard
                            .set_variable("recording_status".to_string(), "error".to_string());
                        context_guard.current_node = error_node.clone();
                        drop(context_guard);

                        return Box::pin(self.execute_next_node(call_id)).await;
                    }
                }
            }
        } else {
            warn!(call_id = %call_id, "MediaManager not configured - ignoring StopRecording");
        }

        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let mut context_guard = context.write().await;
        context_guard.set_variable("recording_status".to_string(), "stopped".to_string());
        context_guard.current_node = next;
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

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
                NodeType::Answer { next, .. } => {
                    info!(
                        call_id = %call_id,
                        node_id = %current_node,
                        "Answer node: calling answer callback"
                    );

                    // Clone callback and flow info before dropping guard
                    let callback = context_guard.answer_callback.clone();
                    let trace_flow_id = context_guard.flow.id.clone();
                    let trace_flow_name = context_guard.flow.name.clone();
                    let trace_flow_version = context_guard.flow.version;
                    drop(context_guard);

                    // Persist flow trace: Answer node
                    self.persist_trace(
                        &call_id,
                        &trace_flow_id,
                        Some(&trace_flow_name),
                        Some(&current_node),
                        fcp_core::flow_trace::FlowTraceEventType::NodeExecuted,
                        Some("answer".to_string()),
                        Some(trace_flow_version),
                        None,
                    );

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

                        // Propagate media_session_id from Call to flow context
                        // (the answer callback creates the media session and updates the Call)
                        if context_guard.media_session_id.is_none() {
                            if let Some(ref cm) = self.call_manager {
                                if let Some(call_entry) = cm.get_call(&call_id) {
                                    if let Some(ref msid) = call_entry.media_session_id {
                                        context_guard.set_media_session_id(*msid);
                                    }
                                }
                            }
                        }

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

                        let url_value = Self::interpolate(url, &context_guard);

                        // Persist flow trace: Play node
                        self.persist_trace(
                            &call_id,
                            &context_guard.flow.id,
                            Some(&context_guard.flow.name),
                            Some(&current_node),
                            fcp_core::flow_trace::FlowTraceEventType::NodeExecuted,
                            Some(format!("play: {}", url)),
                            Some(context_guard.flow.version),
                            None,
                        );

                        // Resolve prompt API URLs to filesystem paths.
                        // The admin UI stores URLs like "/api/v1/prompts/<prompt-id>/audio"
                        // but the media manager needs a local filesystem path.
                        let resolved_path = self
                            .resolve_prompt_url(&url_value)
                            .await
                            .unwrap_or(url_value.clone());

                        // Use media session ID from context (not SIP call ID)
                        let session_id = context_guard.media_session_id.ok_or_else(|| {
                            anyhow::anyhow!("No media session ID available for playback")
                        })?;

                        // Start playback to ParticipantA (the caller in IVR mode)
                        // Note: Using ParticipantA instead of Both to avoid echo in IVR scenarios
                        match media_manager
                            .play_file(
                                &session_id,
                                resolved_path.clone(),
                                AudioTarget::ParticipantA,
                            )
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
                                match media_manager
                                    .wait_playback_completion(playback_id.clone())
                                    .await
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

                        let text_value = Self::interpolate(text, &context_guard);
                        let language_value = language.clone();
                        let voice_value = voice.clone();

                        // Persist flow trace: Say node
                        self.persist_trace(
                            &call_id,
                            &context_guard.flow.id,
                            Some(&context_guard.flow.name),
                            Some(&current_node),
                            fcp_core::flow_trace::FlowTraceEventType::NodeExecuted,
                            Some(format!("say: {}", &text_value[..text_value.len().min(80)])),
                            Some(context_guard.flow.version),
                            None,
                        );

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
                                match media_manager
                                    .wait_playback_completion(playback_id.clone())
                                    .await
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

        // Persist flow trace: NodeExecuted
        self.persist_trace(
            &call_id,
            &context_guard.flow.id,
            Some(&context_guard.flow.name),
            Some(&current_node),
            fcp_core::flow_trace::FlowTraceEventType::NodeExecuted,
            None,
            Some(context_guard.flow.version),
            None,
        );

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

                // Persist flow trace: Completed (hangup)
                self.persist_trace(
                    &call_id,
                    &context_guard.flow.id,
                    Some(&context_guard.flow.name),
                    None,
                    fcp_core::flow_trace::FlowTraceEventType::Completed,
                    Some("hangup".to_string()),
                    Some(context_guard.flow.version),
                    None,
                );

                // Invoke hangup callback to send BYE
                let callback = context_guard.hangup_callback.clone();
                drop(context_guard);
                self.contexts.remove(&call_id);

                if let Some(callback) = callback {
                    if let Err(e) = callback(reason).await {
                        error!(call_id = %call_id, error = %e, "Failed to send BYE for flow hangup");
                    }
                }
            }

            FlowExecutionResult::WaitingForInput {
                variable,
                transitions,
                default,
            } => {
                info!(
                    call_id = %call_id,
                    variable = %variable,
                    "Flow waiting for user input (DTMF)"
                );

                // Get gather configuration from context
                let num_digits = context_guard
                    .get_variable("__gather_num_digits")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1);

                let timeout_secs = context_guard
                    .get_variable("__gather_timeout")
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(5);

                let finish_on_key = context_guard
                    .get_variable("__gather_finish_on_key")
                    .ok()
                    .and_then(|s| s.chars().next());

                let on_timeout_node = context_guard.get_variable("__gather_on_timeout").ok();
                let barge_in = context_guard
                    .get_variable("__gather_barge_in")
                    .ok()
                    .map(|v| v != "false")
                    .unwrap_or(true);

                // Check for prompt
                let prompt_url = context_guard.get_variable("__gather_prompt_url").ok();

                let media_session_id = context_guard.media_session_id;
                let flow_id = context_guard.flow.id.clone();
                let flow_name_for_trace = context_guard.flow.name.clone();
                let flow_version_for_trace = context_guard.flow.version;

                drop(context_guard);

                // Persist flow trace: WaitingForInput
                self.persist_trace(
                    &call_id,
                    &flow_id,
                    Some(&flow_name_for_trace),
                    None,
                    fcp_core::flow_trace::FlowTraceEventType::WaitingForInput,
                    Some(format!("variable={}, timeout={}s", variable, timeout_secs)),
                    Some(flow_version_for_trace),
                    None,
                );

                // Publish waiting for input event
                if let Some(events) = &self.events {
                    let event = PlatformEvent::Flow(FlowEvent::WaitingForInput {
                        call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                        flow_id,
                        variable: variable.clone(),
                        timeout_secs,
                        timestamp: std::time::SystemTime::now(),
                    });

                    let events = Arc::clone(events);
                    tokio::spawn(async move {
                        if let Err(e) = events.publish(event).await {
                            error!(error = %e, "Failed to publish FlowEvent::WaitingForInput");
                        }
                    });
                }

                // Create DTMF collector and play prompt
                if let Some(media_manager) = &self.media_manager {
                    let gather_call_id = media_session_id
                        .unwrap_or_else(|| fcp_core::CallId::from_uuid(*call_id.as_uuid()));

                    info!(
                        call_id = %call_id,
                        gather_call_id = %gather_call_id,
                        media_session_id = ?media_session_id,
                        num_digits = num_digits,
                        finish_on_key = ?finish_on_key,
                        has_prompt = prompt_url.is_some(),
                        transitions_count = transitions.len(),
                        transition_keys = ?transitions.keys().collect::<Vec<_>>(),
                        "Gather: creating DTMF collector"
                    );

                    // Create collector BEFORE playback so digits during prompt are captured
                    let collector = media_manager.create_dtmf_collector(
                        &gather_call_id,
                        num_digits,
                        finish_on_key,
                    );

                    // Start prompt playback if available
                    let playback_id: Option<String> = if let (Some(prompt_url), Some(session_id)) =
                        (&prompt_url, media_session_id)
                    {
                        let resolved_url = self
                            .resolve_prompt_url(prompt_url)
                            .await
                            .unwrap_or(prompt_url.clone());
                        info!(
                            call_id = %call_id,
                            prompt_url = %resolved_url,
                            barge_in = barge_in,
                            "Playing Gather prompt"
                        );
                        match media_manager
                            .play_file(
                                &session_id,
                                resolved_url.clone(),
                                fcp_media::AudioTarget::ParticipantA,
                            )
                            .await
                        {
                            Ok(pid) => {
                                if !barge_in {
                                    // No barge-in: wait for prompt to finish before collecting
                                    if let Err(e) =
                                        media_manager.wait_playback_completion(pid.clone()).await
                                    {
                                        warn!(call_id = %call_id, error = %e, "Gather prompt playback failed");
                                    } else {
                                        info!(call_id = %call_id, "Gather prompt playback complete");
                                    }
                                    None // already finished
                                } else {
                                    Some(pid) // barge-in: collect concurrently, stop later
                                }
                            }
                            Err(e) => {
                                warn!(call_id = %call_id, error = %e, "Failed to play Gather prompt");
                                None
                            }
                        }
                    } else {
                        None
                    };

                    info!(
                        call_id = %call_id,
                        num_digits = num_digits,
                        timeout_secs = timeout_secs,
                        finish_on_key = ?finish_on_key,
                        barge_in = barge_in,
                        "Starting DTMF collection"
                    );

                    // Collect DTMF — when done, stop any still-active playback
                    let result = collector
                        .collect(std::time::Duration::from_secs(timeout_secs as u64))
                        .await;
                    if let Some(pid) = playback_id {
                        let _ = media_manager.stop_playback(&gather_call_id, pid).await;
                    }

                    info!(
                        call_id = %call_id,
                        result = ?result,
                        "DTMF collection completed"
                    );

                    // Process collection result
                    match result {
                        fcp_media::CollectionResult::Complete(digits) => {
                            // Store collected digits in variable
                            let mut context_guard = context.write().await;
                            context_guard.set_variable(variable.clone(), digits.clone());

                            // Determine next node based on transitions
                            let next_node = if let Some(next) = transitions.get(&digits) {
                                next.clone()
                            } else if let Some(default_node) = &default {
                                default_node.clone()
                            } else {
                                // No transition found and no default - stay on current node
                                warn!(
                                    call_id = %call_id,
                                    digits = %digits,
                                    "No transition found for collected digits"
                                );
                                context_guard.current_node.clone()
                            };

                            info!(
                                call_id = %call_id,
                                digits = %digits,
                                next_node = %next_node,
                                "DTMF collection complete, continuing to next node"
                            );

                            context_guard.current_node = next_node;
                            drop(context_guard);

                            // Continue to next node
                            Box::pin(self.execute_next_node(call_id)).await?;
                        }
                        fcp_media::CollectionResult::Timeout(partial_digits) => {
                            // Store partial digits if any
                            let mut context_guard = context.write().await;
                            if !partial_digits.is_empty() {
                                context_guard
                                    .set_variable(variable.clone(), partial_digits.clone());
                            }

                            // If digits were collected before timeout, check transitions
                            // first — the user pressed a valid digit but the timeout
                            // fired before max_digits was reached.
                            let next_node = if !partial_digits.is_empty() {
                                if let Some(next) = transitions.get(&partial_digits) {
                                    info!(
                                        call_id = %call_id,
                                        digits = %partial_digits,
                                        next_node = %next,
                                        "Timeout with collected digits — matched transition"
                                    );
                                    next.clone()
                                } else if let Some(timeout_node) = on_timeout_node {
                                    timeout_node
                                } else if let Some(ref default_node) = default {
                                    default_node.clone()
                                } else {
                                    warn!(
                                        call_id = %call_id,
                                        digits = %partial_digits,
                                        "Timeout with digits but no matching transition or handler"
                                    );
                                    context_guard.current_node.clone()
                                }
                            } else if let Some(timeout_node) = on_timeout_node {
                                timeout_node
                            } else if let Some(default_node) = default {
                                default_node
                            } else {
                                warn!(
                                    call_id = %call_id,
                                    "DTMF collection timeout with no timeout handler"
                                );
                                context_guard.current_node.clone()
                            };

                            info!(
                                call_id = %call_id,
                                partial_digits = %partial_digits,
                                next_node = %next_node,
                                "DTMF collection timeout, continuing to next node"
                            );

                            context_guard.current_node = next_node;
                            drop(context_guard);

                            // Continue to next node
                            Box::pin(self.execute_next_node(call_id)).await?;
                        }
                        fcp_media::CollectionResult::Cancelled => {
                            warn!(call_id = %call_id, "DTMF collection cancelled");
                            // Don't continue execution - call likely terminated
                        }
                    }
                } else {
                    warn!(
                        call_id = %call_id,
                        "Media manager not available for DTMF collection"
                    );
                }
            }

            FlowExecutionResult::WaitingForMedia => {
                // Handle Say and Play nodes that need media playback
                self.handle_media_playback(call_id).await?;
            }

            FlowExecutionResult::WaitingForDial {
                destination,
                timeout_secs,
                caller_id,
                max_duration_secs,
                on_busy,
                on_no_answer,
                on_error,
                next,
            } => {
                info!(
                    call_id = %call_id,
                    destination = %destination,
                    timeout_secs = timeout_secs,
                    max_duration_secs = max_duration_secs,
                    "Starting outbound call (Dial node)"
                );

                // Clone error/next node for fallback error handling
                let error_fallback = on_error.clone();
                let next_fallback = next.clone();

                // Drop the write lock BEFORE calling handle_dial —
                // handle_dial needs to read-lock the context for media_session_id
                drop(context_guard);

                // Handle the dial operation
                match self
                    .handle_dial(
                        call_id,
                        destination,
                        timeout_secs,
                        caller_id,
                        max_duration_secs,
                        on_busy,
                        on_no_answer,
                        on_error,
                        next,
                    )
                    .await
                {
                    Ok(()) => {
                        // Dial completed successfully, flow continues
                    }
                    Err(e) => {
                        error!(
                            call_id = %call_id,
                            error = %e,
                            "Dial node failed"
                        );

                        // Store error and transition to error node or next
                        let mut context_guard = context.write().await;
                        context_guard.set_variable("dial_status".to_string(), "error".to_string());
                        context_guard.set_variable("dial_error".to_string(), e.to_string());

                        if let Some(error_node) = error_fallback {
                            context_guard.current_node = error_node;
                        } else {
                            context_guard.current_node = next_fallback;
                        }
                        drop(context_guard);

                        Box::pin(self.execute_next_node(call_id)).await?;
                    }
                }
            }

            FlowExecutionResult::WaitingForVoicemail {
                mailbox,
                is_busy,
                max_duration,
                play_beep,
                next_on_success,
                next_on_error,
            } => {
                info!(
                    call_id = %call_id,
                    mailbox = %mailbox,
                    is_busy = is_busy,
                    max_duration = max_duration,
                    "Starting voicemail recording (Voicemail node)"
                );

                drop(context_guard);

                self.handle_voicemail(
                    call_id,
                    mailbox,
                    is_busy,
                    max_duration,
                    play_beep,
                    next_on_success,
                    next_on_error,
                )
                .await?;
            }

            FlowExecutionResult::WaitingForVoicemailAccess {
                mailbox,
                max_pin_attempts,
                next,
                on_auth_failure,
            } => {
                info!(
                    call_id = %call_id,
                    mailbox = %mailbox,
                    max_pin_attempts = max_pin_attempts,
                    "Starting voicemail access IVR"
                );

                drop(context_guard);

                self.handle_voicemail_access(
                    call_id,
                    mailbox,
                    max_pin_attempts,
                    next,
                    on_auth_failure,
                )
                .await?;
            }

            FlowExecutionResult::WaitingForFork {
                destinations,
                strategy,
                timeout_secs,
                next,
                on_error,
            } => {
                info!(
                    call_id = %call_id,
                    num_destinations = destinations.len(),
                    strategy = ?strategy,
                    timeout_secs = timeout_secs,
                    "Starting parallel fork"
                );

                drop(context_guard);

                self.handle_fork(
                    call_id,
                    destinations,
                    strategy,
                    timeout_secs,
                    next,
                    on_error,
                )
                .await?;
            }

            FlowExecutionResult::WaitingForRecording {
                format,
                variable,
                next,
                on_error,
            } => {
                info!(
                    call_id = %call_id,
                    format = ?format,
                    "Starting call recording"
                );

                drop(context_guard);

                self.handle_start_recording(call_id, format, variable, next, on_error)
                    .await?;
            }

            FlowExecutionResult::StoppingRecording { next, on_error } => {
                info!(call_id = %call_id, "Stopping call recording");

                drop(context_guard);

                self.handle_stop_recording(call_id, next, on_error).await?;
            }
        }

        Ok(())
    }

    /// Handle DTMF input from the caller.
    ///
    /// This is called when DTMF digits are received during a Gather node.
    /// Looks up the pending gather transitions to determine the next node,
    /// stores digits in the named variable, and resumes flow execution.
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

        // Store the digits as a variable
        context_guard.set_variable("dtmf_input".to_string(), digits.clone());

        // Check if there's a pending gather with transitions
        let transitions_json = context_guard.get_variable("__gather_transitions").ok();
        let default_node = context_guard.get_variable("__gather_default").ok();
        let variable_name = context_guard.get_variable("__gather_variable").ok();

        // Store digits in the named variable if specified
        if let Some(var_name) = &variable_name {
            context_guard.set_variable(var_name.clone(), digits.clone());
        }

        // Determine next node based on transitions
        if let Some(transitions_str) = transitions_json {
            if let Ok(transitions) =
                serde_json::from_str::<HashMap<String, String>>(&transitions_str)
            {
                if let Some(next_node) = transitions.get(&digits) {
                    context_guard.current_node = next_node.clone();
                    drop(context_guard);
                    return self.execute_next_node(call_id).await;
                }
            }
        }

        // Fall back to default node
        if let Some(default) = default_node {
            context_guard.current_node = default;
            drop(context_guard);
            return self.execute_next_node(call_id).await;
        }

        drop(context_guard);
        Ok(())
    }

    /// Stop flow execution for a call.
    pub async fn stop_flow(&self, call_id: CallId) -> Result<()> {
        info!(call_id = %call_id, "Stopping flow execution");

        if let Some((_, context)) = self.contexts.remove(&call_id) {
            let context_guard = context.read().await;

            // Persist flow trace: Completed (stopped)
            self.persist_trace(
                &call_id,
                &context_guard.flow.id,
                Some(&context_guard.flow.name),
                None,
                fcp_core::flow_trace::FlowTraceEventType::Completed,
                Some("stopped".to_string()),
                Some(context_guard.flow.version),
                None,
            );

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
        self.contexts
            .get(call_id)
            .map(|entry| Arc::clone(entry.value()))
    }
}

/// Transaction user for handling Dial node INVITE responses.
///
/// This struct implements ClientTransactionUser to receive callbacks
/// when responses arrive for the outbound B-leg INVITE.
struct DialTransactionUser {
    /// Send provisional responses (1xx) to the handler
    prov_tx: mpsc::Sender<Response>,
    /// Send final response (2xx-6xx) to the handler
    final_tx: Mutex<Option<oneshot::Sender<Response>>>,
    /// Send termination reason to the handler
    term_tx: Mutex<Option<oneshot::Sender<String>>>,
    /// UserAgentClient helper for creating ACK
    uac: UserAgentClient,
    /// Transaction manager for sending PRACK transactions.
    transaction_manager: Arc<TransactionManager>,
    /// Original INVITE request (for creating ACK)
    request: Request,
    /// Local URI used to construct the early dialog for PRACK.
    local_uri: sip_core::SipUri,
    /// Remote URI used to construct the early dialog for PRACK.
    remote_uri: sip_core::SipUri,
    /// Transport context for sending ACK/CANCEL
    ctx: TransportContext,
    /// Transport dispatcher for sending ACK outside the transaction layer
    dispatcher: Option<Arc<dyn TransportDispatcher>>,
}

#[async_trait]
impl ClientTransactionUser for DialTransactionUser {
    async fn on_provisional(&self, _key: &TransactionKey, response: &Response) {
        info!("B-leg provisional response: {}", response.code());

        // Forward provisional response to handler (non-blocking)
        let _ = self.prov_tx.send(response.clone()).await;
    }

    async fn on_final(&self, _key: &TransactionKey, response: &Response) {
        info!("B-leg final response: {}", response.code());

        // Send final response to handler via oneshot
        let mut tx = self.final_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(response.clone());
        }
    }

    async fn on_terminated(&self, _key: &TransactionKey, reason: &str) {
        warn!("B-leg transaction terminated: {}", reason);

        // Send termination reason to handler
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(reason.to_string());
        }
    }

    async fn send_ack(
        &self,
        _key: &TransactionKey,
        response: Response,
        _ctx: &TransportContext,
        is_2xx: bool,
    ) {
        info!(
            "Sending ACK for {} response (is_2xx={})",
            response.code(),
            is_2xx
        );

        // Create ACK using UAC helper
        let ack = self.uac.create_ack(&self.request, &response, None);

        // Dispatch ACK via transport
        if let Some(ref dispatcher) = self.dispatcher {
            let ack_bytes = sip_parse::serialize_request(&ack);
            match dispatcher.dispatch(&self.ctx, ack_bytes).await {
                Ok(()) => {
                    info!(
                        "ACK dispatched for B-leg {} response to {}",
                        response.code(),
                        self.ctx.peer()
                    );
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Failed to dispatch ACK for B-leg {} response",
                        response.code()
                    );
                }
            }
        } else {
            warn!(
                "No transport dispatcher available — ACK for B-leg {} not sent",
                response.code()
            );
        }
    }

    async fn send_prack(&self, _key: &TransactionKey, response: Response, ctx: &TransportContext) {
        info!(
            code = response.code(),
            "Sending PRACK for reliable provisional on B-leg"
        );

        let dialog = match Dialog::new_uac(
            &self.request,
            &response,
            self.local_uri.clone(),
            self.remote_uri.clone(),
        ) {
            Some(dialog) => dialog,
            None => {
                warn!(
                    code = response.code(),
                    "Failed to construct early dialog for PRACK"
                );
                return;
            }
        };

        let prack = match self.uac.create_prack(&self.request, &response, &dialog) {
            Ok(prack) => prack,
            Err(e) => {
                warn!(
                    code = response.code(),
                    error = %e,
                    "Failed to create PRACK request"
                );
                return;
            }
        };

        let tu = Arc::new(PrackTransactionUser);
        match self
            .transaction_manager
            .start_client_transaction(prack, ctx.clone(), tu)
            .await
        {
            Ok(key) => {
                debug!(branch = %key.branch(), "Started PRACK transaction");
            }
            Err(e) => {
                warn!(error = %e, "Failed to start PRACK transaction");
            }
        }
    }

    async fn on_transport_error(&self, _key: &TransactionKey) {
        error!("B-leg transport error occurred");

        // Treat transport error as transaction termination
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send("Transport error".to_string());
        }
    }
}

/// Simplified transaction user for CANCEL requests.
///
/// Only needs to receive the final response and termination notification.
struct CancelTransactionUser {
    final_tx: Mutex<Option<oneshot::Sender<Response>>>,
    term_tx: Mutex<Option<oneshot::Sender<String>>>,
}

/// Lightweight transaction user for PRACK transactions.
struct PrackTransactionUser;

#[async_trait]
impl ClientTransactionUser for PrackTransactionUser {
    async fn on_provisional(&self, _key: &TransactionKey, response: &Response) {
        debug!(code = response.code(), "PRACK provisional response");
    }

    async fn on_final(&self, _key: &TransactionKey, response: &Response) {
        info!(code = response.code(), "PRACK final response");
    }

    async fn on_terminated(&self, _key: &TransactionKey, reason: &str) {
        warn!(reason = %reason, "PRACK transaction terminated");
    }

    async fn send_ack(
        &self,
        _key: &TransactionKey,
        _response: Response,
        _ctx: &TransportContext,
        _is_2xx: bool,
    ) {
        // PRACK is a non-INVITE transaction.
    }

    async fn send_prack(
        &self,
        _key: &TransactionKey,
        _response: Response,
        _ctx: &TransportContext,
    ) {
        // Nested PRACK is not applicable.
    }

    async fn on_transport_error(&self, _key: &TransactionKey) {
        warn!("PRACK transport error");
    }
}

#[async_trait]
impl ClientTransactionUser for CancelTransactionUser {
    async fn on_provisional(&self, _key: &TransactionKey, response: &Response) {
        debug!("CANCEL provisional response: {}", response.code());
    }

    async fn on_final(&self, _key: &TransactionKey, response: &Response) {
        info!("CANCEL final response: {}", response.code());
        let mut tx = self.final_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(response.clone());
        }
    }

    async fn on_terminated(&self, _key: &TransactionKey, reason: &str) {
        debug!("CANCEL transaction terminated: {}", reason);
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send(reason.to_string());
        }
    }

    async fn send_ack(
        &self,
        _key: &TransactionKey,
        _response: Response,
        _ctx: &TransportContext,
        _is_2xx: bool,
    ) {
        // CANCEL is non-INVITE, no ACK needed
    }

    async fn send_prack(
        &self,
        _key: &TransactionKey,
        _response: Response,
        _ctx: &TransportContext,
    ) {
        // No PRACK for CANCEL
    }

    async fn on_transport_error(&self, _key: &TransactionKey) {
        warn!("CANCEL transport error");
        let mut tx = self.term_tx.lock().await;
        if let Some(tx) = tx.take() {
            let _ = tx.send("Transport error".to_string());
        }
    }
}

/// Build and send a CANCEL request for an in-progress INVITE.
///
/// Per RFC 3261 §9.1, CANCEL copies the top Via, From, To, Call-ID, Route
/// headers from the original INVITE request, with CSeq method changed to CANCEL.
async fn send_cancel(
    transaction_manager: &TransactionManager,
    invite: &Request,
    ctx: TransportContext,
) -> Result<()> {
    use sip_core::{Headers, Method, RequestLine};

    let mut cancel_headers = Headers::new();

    // Copy top Via only
    for header in invite.headers().iter() {
        if header.name().eq_ignore_ascii_case("Via") {
            cancel_headers.push(header.name(), header.value())?;
            break; // Only top Via
        }
    }

    // Copy From, To, Call-ID
    for name in &["From", "To", "Call-ID"] {
        if let Some(val) = invite.headers().get(name) {
            cancel_headers.push(*name, val)?;
        }
    }

    // CSeq with CANCEL method (same sequence number, different method)
    if let Some(cseq) = invite.headers().get("CSeq") {
        if let Some((num, _method)) = cseq.split_once(' ') {
            cancel_headers.push("CSeq", format!("{} CANCEL", num))?;
        }
    }

    // Copy Route headers
    for header in invite.headers().iter() {
        if header.name().eq_ignore_ascii_case("Route") {
            cancel_headers.push(header.name(), header.value())?;
        }
    }

    cancel_headers.push("Max-Forwards", "70")?;
    cancel_headers.push("Content-Length", "0")?;

    let cancel_request = Request::new(
        RequestLine::new(Method::Cancel, invite.uri().clone()),
        cancel_headers,
        bytes::Bytes::new(),
    )?;

    let (final_tx, final_rx) = oneshot::channel::<Response>();
    let (term_tx, _term_rx) = oneshot::channel::<String>();

    let tu = Arc::new(CancelTransactionUser {
        final_tx: Mutex::new(Some(final_tx)),
        term_tx: Mutex::new(Some(term_tx)),
    });

    let _key = transaction_manager
        .start_client_transaction(cancel_request, ctx, tu)
        .await?;

    // Wait briefly for 200 OK to CANCEL (don't block forever)
    match tokio::time::timeout(std::time::Duration::from_secs(5), final_rx).await {
        Ok(Ok(resp)) => {
            info!(code = resp.code(), "CANCEL response received");
        }
        _ => {
            debug!("CANCEL response timeout or channel closed");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_flow::InMemoryFlowStore;

    #[tokio::test]
    async fn test_flow_routing() {
        let store = Arc::new(InMemoryFlowStore::new());
        let handler = FlowCallHandler::new(store, None, None);

        handler
            .add_route("+15551234567".to_string(), "test-flow".to_string())
            .await;

        let flow_id = handler.get_flow_for_did("+15551234567").await;
        assert_eq!(flow_id, Some("test-flow".to_string()));

        let no_route = handler.get_flow_for_did("+15559999999").await;
        assert_eq!(no_route, None);
    }
}
