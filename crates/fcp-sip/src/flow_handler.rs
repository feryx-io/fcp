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
    /// Where a Voicemail node sends the call when this node records no
    /// messages itself: the voicemail server (the call manager's
    /// `voicemail_server_uri`).
    voicemail_server: Option<String>,

    /// Hoteling sessions, for the login node (`docs/ENTERPRISE_CALLING.md` §3.8)
    hoteling: Option<Arc<crate::hoteling::Hoteling>>,

    /// Prompt service for resolving system prompts to audio files
    prompt_service: Option<Arc<fcp_core::PromptService>>,

    /// Flow trace store for persistent execution tracing
    flow_trace_store: Option<Arc<dyn fcp_core::flow_trace::FlowTraceStore>>,

    /// Registrar for resolving extensions to registered device contacts
    registrar: Option<Arc<FcpRegistrar>>,

    /// Storage backend for extension → phone number → device resolution
    storage: Option<Arc<fcp_core::DynStorage>>,

    /// The node's queues, where a Queue node dials its caller
    queue_handler: Option<Arc<dyn crate::queue::QueueHandler>>,

    /// Transport dispatcher for sending out-of-transaction messages (ACK for 2xx)
    dispatcher: Option<Arc<dyn TransportDispatcher>>,

    /// The recorder a flow's recordings start through, shared with the SIP
    /// handler so a call is recorded once whoever asks.
    call_recorder: std::sync::OnceLock<Arc<crate::recording::CallRecorder>>,

    /// The routes emergency calls take, shared with the SIP handler, so a
    /// `Dial` to an emergency number goes where the B2BUA would send it.
    outbound_route_provider: Option<Arc<dyn crate::handler::OutboundRouteProvider>>,

    /// The digit collection of each call's Gather in progress, so a digit
    /// that arrives by SIP INFO joins it.
    gather_digits: Arc<DashMap<CallId, mpsc::UnboundedSender<char>>>,

    /// Each call's live transcript (the Transcribe node), one task per leg.
    transcripts: Arc<DashMap<CallId, Vec<tokio::task::JoinHandle<()>>>>,
}

/// What a Gather heard, for its retries: a branch chosen, or nothing, or
/// a choice the menu does not have.
enum Heard {
    Chose(String),
    Nothing,
    Invalid,
}

/// A call's Gather collection, taken out of [`FlowCallHandler`]'s map when
/// the Gather ends however it ends.
struct GatherDigitSlot {
    slots: Arc<DashMap<CallId, mpsc::UnboundedSender<char>>>,
    call_id: CallId,
}

impl Drop for GatherDigitSlot {
    fn drop(&mut self) {
        self.slots.remove(&self.call_id);
    }
}

/// How a `Dial` node places an emergency call: the emergency routes in
/// order, and the identity every attempt carries.
struct EmergencyDialPlan {
    targets: Vec<(sip_core::SipUri, crate::emergency::EmergencyAttempt)>,
    from: Option<sip_core::SipUri>,
    headers: sip_core::Headers,
}

/// The outcome of one B-leg INVITE.
#[derive(Debug, Default)]
struct DialAttempt {
    /// `answered`, `busy`, `no_answer`, `failed` or `error`.
    status: String,
    sip_code: Option<u16>,
    sip_reason: Option<String>,
    answered_at: Option<std::time::SystemTime>,
    /// The trunk refused the location the attempt carried
    /// (`Geolocation-Error` or 424 on a failure).
    location_refused: bool,
}

impl DialAttempt {
    /// Whether an emergency call moves on to its next route after this
    /// attempt: after anything but an answer or the caller's cancel.
    fn emergency_fails_over(&self) -> bool {
        self.status != "answered" && self.sip_code != Some(487)
    }
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
            voicemail_server: None,
            hoteling: None,
            prompt_service: None,
            flow_trace_store: None,
            registrar: None,
            storage: None,
            queue_handler: None,
            dispatcher: None,
            call_recorder: std::sync::OnceLock::new(),
            outbound_route_provider: None,
            gather_digits: Arc::new(DashMap::new()),
            transcripts: Arc::new(DashMap::new()),
        }
    }

    /// Record through `recorder`, the SIP handler's.
    pub fn with_call_recorder(self, recorder: Arc<crate::recording::CallRecorder>) -> Self {
        let _ = self.call_recorder.set(recorder);
        self
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

    /// The voicemail server a Voicemail node hands its call to when this
    /// node has no voicemail service of its own (`docs/DEEP_TESTING.md`
    /// D8: a mailbox's own flow ends in a message after all).
    pub fn with_voicemail_server(mut self, uri: impl Into<String>) -> Self {
        self.voicemail_server = Some(uri.into());
        self
    }

    /// Start hoteling sessions from the login node.
    pub fn with_hoteling(mut self, hoteling: Arc<crate::hoteling::Hoteling>) -> Self {
        self.hoteling = Some(hoteling);
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

    /// Route a `Dial` to an emergency number the way the SIP handler routes
    /// emergency calls.
    pub fn with_outbound_route_provider(
        mut self,
        provider: Option<Arc<dyn crate::handler::OutboundRouteProvider>>,
    ) -> Self {
        self.outbound_route_provider = provider;
        self
    }

    /// Set the storage backend for extension → device resolution.
    pub fn with_storage(mut self, storage: Arc<fcp_core::DynStorage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// The node's queues, for the Queue node.
    pub fn with_queue_handler(mut self, queues: Arc<dyn crate::queue::QueueHandler>) -> Self {
        self.queue_handler = Some(queues);
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
            let (var_name, filter) = match cap[1].split_once('|') {
                Some((name, filter)) => (name.trim(), Some(filter.trim())),
                None => (cap[1].trim(), None),
            };
            if let Ok(value) = context.get_variable(var_name) {
                let value = match filter {
                    Some(filter) => fcp_flow::apply_filter(&value, filter).unwrap_or(value),
                    None => value,
                };
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
        self: &Arc<Self>,
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

        // Update call, and the call manager's copy before the flow runs:
        // an in-dialog request (a digit by SIP INFO) finds the flow there.
        call.flow_id = Some(flow_id.clone());
        call.flow_current_node = Some(flow.initial_node.clone());
        if let Some(call_manager) = &self.call_manager {
            let _ = call_manager.update_call(&call.id, |stored| {
                stored.flow_id = Some(flow_id.clone());
                stored.flow_current_node = Some(flow.initial_node.clone());
            });
        }

        // Publish flow started event
        if let Some(events) = &self.events {
            let event = PlatformEvent::Flow(FlowEvent::Started {
                call_id: call_id_core,
                tenant_id: Some(call.tenant_id),
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

        // Execute the initial node, on a task of its own, awaited so the
        // order is unchanged. The flow runs until it waits for the caller —
        // a pause, a hangup and the SIP work behind it included — and it
        // used to run on the INVITE's own stack, under `handle_invite`'s
        // frame, which in a debug build left it within 50 KB of a worker's
        // 2 MB. A spawned task is polled from the top of a stack.
        let handler = Arc::clone(self);
        let call_id = call.id;
        tokio::spawn(async move { handler.execute_next_node(call_id).await })
            .await
            .map_err(|e| anyhow::anyhow!("Flow task did not finish: {e}"))??;

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
    /// `sip:<user>@` this node's own address, port included: a leg placed
    /// there comes back into this node (`own_leg`).
    fn own_uri(&self, user: &str) -> Option<sip_core::SipUri> {
        let local = self.local_uri.as_ref()?;
        let host = match local.port() {
            Some(port) => format!("{}:{}", local.host(), port),
            None => local.host().to_string(),
        };
        sip_core::SipUri::parse(&format!("sip:{user}@{host}")).ok()
    }

    /// Where a Queue node's caller is dialed: the extension at this node
    /// where the call's tenant's queue, named by its name or extension,
    /// answers.
    fn queue_leg_uri(
        &self,
        a_leg_call_id: CallId,
        queue: &str,
    ) -> std::result::Result<sip_core::SipUri, String> {
        let queues = self
            .queue_handler
            .as_ref()
            .ok_or("Queues are off on this node")?;
        let tenant_id = self
            .call_manager
            .as_ref()
            .and_then(|cm| cm.get_call(&a_leg_call_id))
            .map(|c| c.tenant_id)
            .ok_or("No tenant for a queue")?;
        let extension = queues
            .extension_of(&tenant_id, queue)
            .ok_or_else(|| format!("No queue '{queue}'"))?;
        let uri = self.own_uri(&extension).ok_or("SIP URIs not configured")?;
        info!(a_leg_call_id = %a_leg_call_id, queue = %queue, extension = %extension, "Queue node: the caller is dialed into the queue");
        Ok(uri)
    }

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

        // A destination that is an emergency number is an emergency call:
        // classified, located and recorded as the B2BUA's are, and placed on
        // each emergency route in turn with the callback number as caller
        // (`docs/E911_BLOCK_2.md` §4).
        let emergency = match self.emergency_dial_plan(a_leg_call_id, &destination).await {
            Some(Ok(plan)) => Some(plan),
            Some(Err(error_msg)) => {
                return self
                    .transition_to_error(a_leg_call_id, &error_msg, on_error, next)
                    .await;
            }
            None => None,
        };
        let targets: Vec<(sip_core::SipUri, Option<crate::emergency::EmergencyAttempt>)> =
            match &emergency {
                Some(plan) => plan
                    .targets
                    .iter()
                    .map(|(uri, attempt)| (uri.clone(), Some(attempt.clone())))
                    .collect(),
                None => {
                    // Parse destination URI. For bare extensions, resolve via registrar or
                    // extension → phone number → device chain to get the actual device contact
                    // (e.g., "1234" → "polycom@192.168.1.12") so we INVITE the device directly
                    // instead of looping through the local B2BUA.
                    // A Queue node's caller goes to the queue's extension here.
                    let queue_uri = match destination.strip_prefix(fcp_flow::QUEUE_DIAL_PREFIX) {
                        Some(queue) => match self.queue_leg_uri(a_leg_call_id, queue) {
                            Ok(uri) => Some(uri),
                            Err(error_msg) => {
                                warn!(a_leg_call_id = %a_leg_call_id, %error_msg);
                                return self
                                    .transition_to_error(a_leg_call_id, &error_msg, on_error, next)
                                    .await;
                            }
                        },
                        None => None,
                    };
                    let parsed = match queue_uri {
                        Some(uri) => Ok(uri),
                        None => sip_core::SipUri::parse(&destination),
                    };
                    let dest_uri = match parsed {
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
                                    if let Some(tenant_id) =
                                        self.call_manager.as_ref().and_then(|cm| {
                                            cm.get_call(&a_leg_call_id).map(|c| c.tenant_id)
                                        })
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
                                                fcp_core::ExtensionTarget::PhoneNumber(
                                                    phone_number_id,
                                                ) => {
                                                    match storage
                                                        .get_phone_number(phone_number_id)
                                                        .await
                                                    {
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
                                                                    let devices = line_binding
                                                                        .enabled_devices();
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
                                                                                &device_binding
                                                                                    .device_id,
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
                                                                                if let Some(
                                                                                    sip_uri,
                                                                                ) =
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
                                                            if let Ok(bindings) = registrar
                                                                .get_registrations(sip_uri)
                                                                .await
                                                            {
                                                                if let Some(binding) =
                                                                    bindings.first()
                                                                {
                                                                    if let Ok(uri) =
                                                                        sip_core::SipUri::parse(
                                                                            binding.contact(),
                                                                        )
                                                                    {
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
                                                fcp_core::ExtensionTarget::User(user_id) => {
                                                    // The user's plan, all of it
                                                    // (`docs/ENTERPRISE_CALLING.md` §2): the
                                                    // leg goes back into this node for the
                                                    // extension, which rings the user as any
                                                    // call would — forwards, call waiting,
                                                    // find-me, an executive's assistants, apps
                                                    // woken by push. Its treatments come back
                                                    // as codes (`FLOW_DIAL_HEADER`), so a busy
                                                    // or unanswered user takes the flow's own
                                                    // branches.
                                                    resolved = self.own_uri(&destination);
                                                    info!(a_leg_call_id = %a_leg_call_id, user_id = %user_id, "Dial to a user: rung by their plan at this node");
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
                                                let error_msg = format!(
                                                    "Failed to parse fallback URI: {:?}",
                                                    e2
                                                );
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
                                            .transition_to_error(
                                                a_leg_call_id,
                                                &error_msg,
                                                on_error,
                                                next,
                                            )
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

                    vec![(dest_uri, None)]
                }
            };

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

        let mut attempt = DialAttempt::default();
        let mut answering_trunk = None;
        let mut pending: std::collections::VecDeque<_> = targets.into_iter().collect();
        while let Some((dest_uri, route)) = pending.pop_front() {
            attempt = self
                .dial_attempt(
                    a_leg_call_id,
                    &dest_uri,
                    caller_id.as_deref(),
                    emergency.as_ref().zip(route.as_ref()),
                    &sdp_offer,
                    session_key,
                    timeout_secs,
                )
                .await?;
            answering_trunk = route.as_ref().and_then(|r| r.trunk.clone());
            // A trunk that refused the location and also takes ELIN is
            // tried again without it before the next route (§12.9).
            if let Some(retry) = route
                .as_ref()
                .filter(|_| attempt.location_refused)
                .and_then(|r| r.without_pidf_lo())
            {
                warn!(
                    a_leg_call_id = %a_leg_call_id,
                    trunk = ?retry.trunk,
                    "Emergency trunk refused the location; trying it again with ELIN alone"
                );
                pending.push_front((dest_uri, Some(retry)));
                continue;
            }
            if emergency.is_none() || pending.is_empty() || !attempt.emergency_fails_over() {
                break;
            }
            warn!(
                a_leg_call_id = %a_leg_call_id,
                trunk = ?answering_trunk,
                status = %attempt.status,
                sip_code = ?attempt.sip_code,
                "Emergency route failed; trying the next emergency route"
            );
            // The route this call has just given up on (§9). The tenant
            // is the call's, and an emergency route is taken in the
            // tenant's name whoever owns the trunk.
            if let Some(trunk_id) = route.as_ref().and_then(|r| r.trunk_id.as_ref()) {
                fcp_core::trunk_report::TrunkCounters::global().note(
                    &trunk_id.to_string(),
                    None,
                    fcp_core::trunk_report::TrunkOutcome::FailedOver,
                );
            }
        }
        if let (Some(_), Some(call_mgr)) = (&emergency, &self.call_manager) {
            let leg = fcp_b2bua_core::EmergencyLeg {
                outcome: attempt.status.clone(),
                sip_status: attempt.sip_code,
                answered_at: attempt.answered_at,
            };
            let _ = call_mgr.update_call(&a_leg_call_id, |call| {
                if let Some(em) = call.emergency.as_mut() {
                    em.leg = Some(leg);
                }
                if answering_trunk.is_some() {
                    call.trunk_name = answering_trunk.clone();
                }
            });
        }
        let dial_status = attempt.status;
        let dial_sip_code = attempt.sip_code;
        let dial_sip_reason = attempt.sip_reason;
        let next_node = match dial_status.as_str() {
            "answered" => next,
            "busy" => on_busy.unwrap_or(next),
            "no_answer" => on_no_answer.unwrap_or(next),
            _ => on_error.unwrap_or(next),
        };

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

    /// The emergency plan for a `Dial` to `destination`, when it is an
    /// emergency number for the call's tenant: the call is marked, its
    /// record written and `call.emergency` published before any attempt.
    /// `Some(Err)` when there is no route to place it on.
    async fn emergency_dial_plan(
        &self,
        a_leg_call_id: CallId,
        destination: &str,
    ) -> Option<std::result::Result<EmergencyDialPlan, String>> {
        let call_mgr = self.call_manager.as_ref()?;
        let call = call_mgr.get_call(&a_leg_call_id)?;
        let caller = call
            .original_invite
            .as_ref()
            .and_then(|r| sip_parse::header(r.headers(), "From"))
            .and_then(|from| {
                let from = from.as_str();
                let uri = match (from.find('<'), from.find('>')) {
                    (Some(start), Some(end)) if start < end => &from[start + 1..end],
                    _ => from.split(';').next().unwrap_or(from),
                };
                sip_core::SipUri::parse(uri.trim()).ok()
            });
        let username = caller.as_ref().and_then(|u| u.user().map(str::to_string));
        let aor = caller
            .as_ref()
            .map(|u| format!("{}@{}", u.user().unwrap_or_default(), u.host()));
        // The flow's caller was placed when their call arrived; the A-leg's
        // source stands in for where they are.
        let client_address = call.a_leg.contact.map(|peer| peer.ip());
        let supplied = call
            .original_invite
            .as_ref()
            .and_then(crate::emergency::supplied_location);
        let info = crate::emergency::detect(
            self.storage.as_ref(),
            &call.tenant_id,
            destination,
            username.as_deref(),
            aor.as_deref(),
            client_address,
            supplied.as_ref(),
        )
        .await?;

        let local_host = self
            .local_uri
            .as_ref()
            .map(|u| u.host().to_string())
            .unwrap_or_else(|| "localhost".to_string());
        info!(
            a_leg_call_id = %a_leg_call_id,
            dialed = %info.dialed,
            kind = %info.kind,
            callback = ?info.callback_number,
            "Flow dialing an emergency number"
        );
        // The tenant whose call this is decides which trunks may carry it
        // (`docs/TENANT_TRUNKS.md` §5).
        let tenant = self
            .call_manager
            .as_ref()
            .and_then(|cm| cm.get_call(&a_leg_call_id))
            .map(|call| call.tenant_id);
        let routes = match &self.outbound_route_provider {
            Some(provider) => {
                provider
                    .resolve_emergency_ordered(
                        tenant,
                        &format!("sip:{}@{}", info.dialed, local_host),
                    )
                    .await
            }
            None => Vec::new(),
        };
        let (from, headers) =
            crate::emergency::outbound_identity(&info, caller.clone(), &local_host);
        let trunks = crate::handler::route_trunks(&routes);
        let first_trunk = routes.first().and_then(|r| r.trunk_name());
        let first_trunk_id = routes.first().and_then(|r| r.trunk_id());
        let marked = info.clone();
        let _ = call_mgr.update_call(&a_leg_call_id, |c| {
            c.emergency = Some(marked);
            c.route_trunks = trunks;
            c.trunk_name = first_trunk;
            c.trunk_id = first_trunk_id;
        });
        let record = crate::emergency::placement_record(
            a_leg_call_id,
            call.tenant_id,
            call.original_invite
                .as_ref()
                .and_then(|r| sip_parse::header(r.headers(), "Call-ID"))
                .map(|h| h.as_str().to_string()),
            aor.map(|a| format!("sip:{a}")).unwrap_or_default(),
            username,
            &info,
        );
        crate::emergency::announce(call_mgr, a_leg_call_id, record).await;

        let targets: Vec<(sip_core::SipUri, crate::emergency::EmergencyAttempt)> = routes
            .iter()
            .filter_map(|r| {
                sip_core::SipUri::parse(&r.target_uri)
                    .ok()
                    .map(|u| (u, crate::emergency::EmergencyAttempt::from_route(r)))
            })
            .collect();
        if targets.is_empty() {
            error!(
                a_leg_call_id = %a_leg_call_id,
                dialed = %info.dialed,
                "EMERGENCY CALL COULD NOT BE ROUTED: no upstream trunk available"
            );
            let _ = call_mgr.update_call(&a_leg_call_id, |c| {
                if let Some(em) = c.emergency.as_mut() {
                    em.leg = Some(fcp_b2bua_core::EmergencyLeg {
                        outcome: "failed".to_string(),
                        sip_status: Some(503),
                        answered_at: None,
                    });
                }
            });
            return Some(Err("No trunk available for emergency call".to_string()));
        }
        Some(Ok(EmergencyDialPlan {
            targets,
            from,
            headers,
        }))
    }

    /// Place one B-leg INVITE for a `Dial` node and wait for its outcome;
    /// an answered leg is bridged onto the A-leg's media session and its
    /// dialog registered. An emergency plan's identity replaces the caller.
    #[allow(clippy::too_many_arguments)]
    async fn dial_attempt(
        &self,
        a_leg_call_id: CallId,
        dest_uri: &sip_core::SipUri,
        caller_id: Option<&str>,
        emergency: Option<(&EmergencyDialPlan, &crate::emergency::EmergencyAttempt)>,
        sdp_offer: &str,
        session_key: fcp_core::CallId,
        timeout_secs: u32,
    ) -> Result<DialAttempt> {
        let transaction_manager = self
            .transaction_manager
            .as_ref()
            .context("TransactionManager not configured")?;
        let local_uri = self.local_uri.as_ref().context("SIP URIs not configured")?;
        let contact_uri = self
            .contact_uri
            .as_ref()
            .context("SIP URIs not configured")?;

        // Create UserAgentClient for B-leg
        let mut uac = UserAgentClient::new(local_uri.clone(), contact_uri.clone());
        if emergency.is_none() {
            if let Some(cid) = caller_id {
                if let Ok(cid_uri) = sip_core::SipUri::parse(cid) {
                    uac.from_uri_override = Some(cid_uri);
                } else if let Ok(()) = uac.set_display_name(cid) {
                    // Use caller_id as display name with local_uri as From URI
                }
            }
        }

        // Create INVITE request with SDP; an emergency attempt carries the
        // callback number as caller and the location its trunk takes.
        let invite = match emergency {
            None => uac.create_invite(dest_uri, Some(sdp_offer)),
            Some((plan, route)) => {
                let info = self
                    .call_manager
                    .as_ref()
                    .and_then(|m| m.get_call(&a_leg_call_id))
                    .and_then(|c| c.emergency)
                    .context("emergency call is gone")?;
                let (options, delivered) = crate::emergency::attempt_invite(
                    &info,
                    route,
                    a_leg_call_id,
                    Some(sdp_offer),
                    plan.from.clone(),
                    Some(&plan.headers),
                    local_uri.host(),
                );
                if let Some(call_mgr) = &self.call_manager {
                    let trunk = route.trunk.clone();
                    let _ = call_mgr.update_call(&a_leg_call_id, |call| {
                        call.trunk_name = trunk;
                        if let Some(em) = call.emergency.as_mut() {
                            em.delivered = delivered;
                        }
                    });
                }
                uac.create_invite_with_options(dest_uri, &options)?
            }
        };
        let mut location_refused = false;
        // A leg back into this node: its token, so it is not challenged;
        // the call it dials for, so it is the tenant's and its forwards
        // follow on; and that a flow's Dial placed it, so the plan answers
        // with a code (`docs/ENTERPRISE_CALLING.md` §3.1.1).
        let mut invite = invite;
        let own = crate::own_leg::OwnAddress::new(
            local_uri.host(),
            local_uri.port().unwrap_or(5060),
            None,
        );
        if emergency.is_none() && own.is_target(dest_uri.as_str()) {
            let headers = invite.headers_mut();
            let _ = headers.push(
                smol_str::SmolStr::new(crate::own_leg::OWN_LEG_HEADER),
                smol_str::SmolStr::new(crate::own_leg::token()),
            );
            let _ = headers.push(
                smol_str::SmolStr::new(crate::handler::FORWARDED_FOR_HEADER),
                smol_str::SmolStr::new(a_leg_call_id.to_string()),
            );
            let _ = headers.push(
                smol_str::SmolStr::new(crate::handler::FLOW_DIAL_HEADER),
                smol_str::SmolStr::new("1"),
            );
        }

        info!(
            a_leg_call_id = %a_leg_call_id,
            from = %local_uri.as_str(),
            to = %dest_uri.as_str(),
            "Sending B-leg INVITE"
        );

        // Phase 2: Send INVITE and handle responses

        // Where the INVITE goes: the route's target, which the trunk
        // router built with an explicit port — an RFC 3263 target where
        // the host was given by name (`docs/DEPLOYMENT_GAPS.md` §1.8) —
        // over the transport the URI names.
        let transport_kind = transport_of(dest_uri);
        let host = dest_uri.host();
        let port = dest_uri.port().unwrap_or(match transport_kind {
            sip_transaction::fsm::TransportKind::Tls => 5061,
            _ => 5060,
        });

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
                return Ok(DialAttempt {
                    status: "error".to_string(),
                    sip_reason: Some(error_msg),
                    ..Default::default()
                });
            }
        };

        info!(
            a_leg_call_id = %a_leg_call_id,
            peer = %peer_addr,
            "Resolved B-leg destination address"
        );

        // TCP and TLS without a stream go through the transport pool.
        let ctx = TransportContext::new(transport_kind, peer_addr, None).with_server_name(
            match transport_kind {
                sip_transaction::fsm::TransportKind::Tls => Some(host.to_string()),
                _ => None,
            },
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
        let mut answered_at = None;

        // Response event loop
        // Biased: a transaction that ends reports its final response
        // first and its termination after, and both can be ready at once.
        tokio::select! {
            biased;

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
                // The provisional channel closes when the transaction ends,
                // which can be just after its final response: the final
                // response, termination or timeout decides, never this.
                std::future::pending::<()>().await
            } => {}

            // Handle final response
            result = final_rx => {
                match result {
                    Ok(response) => {
                        let code = response.code();
                        dial_sip_code = Some(code);
                        dial_sip_reason = Some(response.reason().to_string());
                        if let (Some(error), Some(_)) =
                            (crate::emergency::geolocation_error_of(&response), emergency)
                        {
                            warn!(
                                a_leg_call_id = %a_leg_call_id,
                                code = %code,
                                geolocation_error = %error,
                                "Emergency trunk answered the location with an error"
                            );
                            location_refused = !(200..300).contains(&code);
                            if let Some(call_mgr) = &self.call_manager {
                                let _ = call_mgr.update_call(&a_leg_call_id, |call| {
                                    if let Some(em) = call.emergency.as_mut() {
                                        em.geolocation_error = Some(error);
                                    }
                                });
                            }
                        }

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
                            answered_at = Some(std::time::SystemTime::now());

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
                                    // Under the call's lock: a caller who hung up
                                    // as this leg answered found no leg to end,
                                    // and this leg is ended here instead.
                                    let callee_uri = dest_uri.as_str().to_string();
                                    let mut caller_gone = true;
                                    let _ = call_mgr.update_call(&a_leg_call_id, |call| {
                                        if call.state.is_terminal()
                                            || call.state
                                                == fcp_b2bua_core::CallState::Terminating
                                        {
                                            return;
                                        }
                                        caller_gone = false;
                                        let mut b_leg = fcp_b2bua_core::CallLeg::new_outbound(
                                            callee_uri,
                                            None,
                                        );
                                        b_leg.dialog = Some(dialog.clone());
                                        call.b_leg = Some(b_leg);
                                    });
                                    if caller_gone {
                                        info!(
                                            a_leg_call_id = %a_leg_call_id,
                                            "The caller hung up as the dialed leg answered; ending the leg"
                                        );
                                        let bye = UserAgentClient::new(
                                            local_uri.clone(),
                                            contact_uri.clone(),
                                        )
                                        .create_bye(&dialog);
                                        if let Err(e) = transaction_manager
                                            .start_client_transaction(
                                                bye,
                                                cancel_ctx.clone(),
                                                Arc::new(PrackTransactionUser),
                                            )
                                            .await
                                        {
                                            warn!(a_leg_call_id = %a_leg_call_id, error = %e, "BYE to the dialed leg failed");
                                        }
                                        if let Some(ref dialog_mgr) = self.dialog_manager {
                                            dialog_mgr.remove(dialog.id());
                                        }
                                    }
                                } else {
                                    warn!(a_leg_call_id = %a_leg_call_id, "Failed to construct B-leg dialog from 200 OK");
                                }
                            }
                        } else if code == 486 || code == 600 {
                            // 486 Busy Here, 600 Busy Everywhere
                            dial_status = String::from("busy");
                        } else if code == 408 || code == 480 || code == 487 {
                            // 408 Request Timeout, 480 Temporarily Unavailable, 487 Request Terminated
                            dial_status = String::from("no_answer");
                        } else {
                            // Other 4xx/5xx/6xx - generic failure
                            dial_status = String::from("failed");
                        }
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Final response channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
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
                    }
                    Err(_) => {
                        warn!(
                            a_leg_call_id = %a_leg_call_id,
                            "Termination channel closed unexpectedly"
                        );
                        dial_status = String::from("error");
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

        Ok(DialAttempt {
            status: dial_status,
            sip_code: dial_sip_code,
            sip_reason: dial_sip_reason,
            answered_at,
            location_refused,
        })
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
            // The voicemail server takes the message: the call is bridged
            // to it, addressed to the mailbox, as an unanswered call is.
            None if self.voicemail_server.is_some() => {
                let server = self.voicemail_server.as_deref().unwrap_or_default();
                let host = server
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .rsplit('@')
                    .next()
                    .unwrap_or(server)
                    .trim_start_matches("sip:");
                let destination = format!("sip:{mailbox}@{host}");
                info!(call_id = %call_id, destination = %destination, "Voicemail node: the voicemail server takes the message");
                let on_error = next_on_error.clone();
                return Box::pin(self.handle_dial(
                    call_id,
                    destination,
                    30,
                    None,
                    max_duration.saturating_add(60),
                    on_error.clone(),
                    on_error.clone(),
                    on_error,
                    next_on_success,
                ))
                .await;
            }
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

                // Transcribed after the call, when the mailbox asks for it;
                // the text lands on the message and in
                // `voicemail.transcribed` (`docs/DEVELOPER_SURFACE.md`
                // phase 6).
                {
                    let voicemail_service = Arc::clone(&voicemail_service);
                    let events = self.events.clone();
                    let message = message.clone();
                    tokio::spawn(async move {
                        match voicemail_service.transcribe_message(&message).await {
                            Ok(Some((mailbox, transcript))) => {
                                let Some(events) = events else { return };
                                let event =
                                    PlatformEvent::Voicemail(VoicemailEvent::MessageTranscribed {
                                        tenant_id: mailbox.tenant_id,
                                        mailbox_id: mailbox.id,
                                        message_id: message.id,
                                        transcription: transcript.text,
                                        confidence: transcript.confidence,
                                        timestamp: std::time::SystemTime::now(),
                                    });
                                if let Err(e) = events.publish(event).await {
                                    error!(error = %e, "Failed to publish voicemail.transcribed");
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                warn!(message_id = %message.id, error = %e, "Voicemail transcription failed")
                            }
                        }
                    });
                }

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
                    // The mailbox's own id, as message waiting knows it
                    // (the flow may have named it by extension).
                    let event = PlatformEvent::Voicemail(VoicemailEvent::MessageRecorded {
                        call_id: core_call_id,
                        tenant_id: self.tenant_of(&call_id),
                        mailbox_id: message.mailbox_id.clone(),
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

    /// The HotelingLogin node (`docs/ENTERPRISE_CALLING.md` §3.8): the
    /// caller's extension, then their PIN, until a login or the attempts
    /// run out; `hoteling_login_status` says which.
    async fn handle_hoteling_login(
        &self,
        call_id: CallId,
        device_id: String,
        max_attempts: u32,
        next: String,
        on_failure: Option<String>,
    ) -> Result<()> {
        let context = {
            let entry = self
                .contexts
                .get(&call_id)
                .ok_or_else(|| anyhow::anyhow!("Flow context not found for call {}", call_id))?;
            Arc::clone(entry.value())
        };
        let media_call_id = {
            let guard = context.read().await;
            let core_call_id = fcp_core::CallId::from_uuid(*call_id.as_uuid());
            guard.media_session_id.unwrap_or(core_call_id)
        };
        let logged_in = self
            .hoteling_login_ivr(call_id, &device_id, max_attempts, media_call_id)
            .await;
        let mut guard = context.write().await;
        guard.set_variable(
            "hoteling_login_status".to_string(),
            if logged_in { "ok" } else { "failed" }.to_string(),
        );
        guard.current_node = if logged_in {
            next
        } else {
            on_failure.unwrap_or(next)
        };
        drop(guard);
        Box::pin(self.execute_next_node(call_id)).await
    }

    async fn hoteling_login_ivr(
        &self,
        call_id: CallId,
        device_id: &str,
        max_attempts: u32,
        media_call_id: fcp_core::CallId,
    ) -> bool {
        let (Some(hoteling), Some(mm)) = (&self.hoteling, &self.media_manager) else {
            warn!(call_id = %call_id, "Hoteling login without hoteling or media configured");
            return false;
        };
        let storage = hoteling.storage();
        let device = match device_id.parse::<fcp_core::DeviceId>() {
            Ok(id) => match storage.get_device(&id).await {
                Ok(Some(device)) if device.hoteling_host && device.enabled => device,
                _ => {
                    warn!(call_id = %call_id, device_id, "Hoteling login from a device that is not a host");
                    return false;
                }
            },
            Err(_) => return false,
        };
        for attempt in 1..=max_attempts.max(1) {
            let extension = self
                .prompt_digits(
                    mm,
                    &media_call_id,
                    "hoteling_enter_extension",
                    "Enter your extension, then pound.",
                )
                .await;
            let pin = self
                .prompt_digits(
                    mm,
                    &media_call_id,
                    "hoteling_enter_pin",
                    "Enter your pin, then pound.",
                )
                .await;
            let user = if extension.is_empty() || pin.is_empty() {
                None
            } else {
                hoteling_guest(storage, &device.tenant_id, &extension, &pin).await
            };
            match user {
                Some(user) => match hoteling.login(&device, &user).await {
                    Ok(_) => {
                        self.say(
                            mm,
                            &media_call_id,
                            "hoteling_logged_in",
                            "You are logged in.",
                        )
                        .await;
                        return true;
                    }
                    Err(e) => {
                        warn!(call_id = %call_id, error = %e, "Hoteling login failed");
                        return false;
                    }
                },
                None => {
                    info!(call_id = %call_id, attempt, extension = %extension, "Hoteling login refused");
                    self.say(
                        mm,
                        &media_call_id,
                        "hoteling_login_incorrect",
                        "Login incorrect.",
                    )
                    .await;
                }
            }
        }
        false
    }

    /// Play a system prompt (its file, else speech) and collect digits up
    /// to `#`, the prompt cut short by the first digit.
    async fn prompt_digits(
        &self,
        mm: &Arc<fcp_media::MediaManager>,
        cid: &fcp_core::CallId,
        prompt: &str,
        text: &str,
    ) -> String {
        use fcp_media::CollectionResult;
        let collector = mm.create_dtmf_collector(cid, 0, Some('#'));
        let playback = self.start_prompt(mm, cid, prompt, text).await;
        let result = collector.collect(std::time::Duration::from_secs(10)).await;
        if let Some(playback) = playback {
            let _ = mm.stop_playback(cid, playback).await;
        }
        match result {
            CollectionResult::Complete(d) | CollectionResult::Timeout(d) => {
                d.trim_end_matches('#').to_string()
            }
            CollectionResult::Cancelled => String::new(),
        }
    }

    /// Play a system prompt to the end.
    async fn say(
        &self,
        mm: &Arc<fcp_media::MediaManager>,
        cid: &fcp_core::CallId,
        prompt: &str,
        text: &str,
    ) {
        if let Some(playback) = self.start_prompt(mm, cid, prompt, text).await {
            let _ = mm.wait_playback_completion(playback).await;
        }
    }

    async fn start_prompt(
        &self,
        mm: &Arc<fcp_media::MediaManager>,
        cid: &fcp_core::CallId,
        prompt: &str,
        text: &str,
    ) -> Option<String> {
        use fcp_media::AudioTarget;
        if let Some(ps) = &self.prompt_service {
            if let Ok(Some(path)) = ps.resolve(prompt, None).await {
                if let Ok(id) = mm
                    .play_file(cid, path.to_string_lossy().to_string(), AudioTarget::Both)
                    .await
                {
                    return Some(id);
                }
            }
        }
        mm.play_tts(
            cid,
            text.to_string(),
            "en-US".to_string(),
            None,
            AudioTarget::Both,
        )
        .await
        .ok()
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
                Some("Enter your pin, followed by pound".to_string()),
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
                    // A suspended user's mailbox is closed to its PIN too
                    // (docs/IDENTITY_AND_DEVICES.md phase 5).
                    if let (Some(storage), Ok(Some(owner))) = (
                        self.storage.as_ref(),
                        voicemail_service
                            .mailbox_owner(&tenant_id, &resolved_id)
                            .await,
                    ) {
                        let suspended =
                            fcp_core::CredentialStore::get_user_by_id(storage.as_ref(), &owner)
                                .await
                                .ok()
                                .flatten()
                                .is_some_and(|u| u.is_suspended());
                        if suspended {
                            info!(call_id = %call_id, mailbox = %resolved_id, "The mailbox's owner is suspended");
                            break;
                        }
                    }
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
                        "Incorrect pin. Please try again.".to_string(),
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
                Some(main_menu_text(new_count, saved_count)),
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
        // The mailbox's counts as the caller left them, for the phones
        // subscribed to message waiting: messages heard, saved or deleted
        // here change the lamp (`docs/DEEP_TESTING.md` D8).
        if let Some(events) = &self.events {
            let messages = voicemail_service
                .get_messages(&target_mailbox, None)
                .await
                .unwrap_or_default();
            let count = |pick: fn(&MessageStatus) -> bool| {
                messages.iter().filter(|m| pick(&m.status)).count() as u32
            };
            let event = PlatformEvent::Voicemail(VoicemailEvent::MwiUpdated {
                tenant_id: Some(tenant_id),
                mailbox_id: target_mailbox.clone(),
                new_messages: count(|s| *s == MessageStatus::New),
                old_messages: count(|s| matches!(s, MessageStatus::Heard | MessageStatus::Saved)),
                timestamp: std::time::SystemTime::now(),
            });
            if let Err(e) = events.publish(event).await {
                warn!(call_id = %call_id, error = %e, "Message waiting not announced");
            }
        }

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

    /// Handle a StartRecording node: the call's recorder starts it, through
    /// the tenant's consent step (`docs/COMPLIANCE_FOLLOWUPS.md` §8.3) — a
    /// flow cannot record around the policy.
    async fn handle_start_recording(
        &self,
        call_id: CallId,
        format: fcp_flow::schema::RecordingFormat,
        variable: String,
        next: String,
        on_error: Option<String>,
    ) -> Result<()> {
        use crate::recording::{RecordingError, RecordingPath};

        let context_entry = self
            .contexts
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("Flow context not found"))?;
        let context = Arc::clone(context_entry.value());
        drop(context_entry);

        let Some(recorder) = self.recorder() else {
            warn!(call_id = %call_id, "No media manager or call manager - recording unavailable");
            let mut context_guard = context.write().await;
            context_guard.set_variable(variable, "error://no_media_manager".to_string());
            context_guard.set_variable("recording_status".to_string(), "error".to_string());
            context_guard.current_node = on_error.unwrap_or(next);
            drop(context_guard);
            return Box::pin(self.execute_next_node(call_id)).await;
        };

        // The flow's media session is the call's: a flow that anchored media
        // itself tells the call, which the recorder records by.
        let flow_session = context.read().await.media_session_id;
        if let (Some(session), Some(cm)) = (flow_session, &self.call_manager) {
            let _ = cm.update_call(&call_id, |call| {
                if call.media_session_id.is_none() {
                    call.media_session_id = Some(session);
                    call.media_anchored = true;
                }
            });
        }

        let extension = match format {
            fcp_flow::schema::RecordingFormat::Wav => "wav",
            fcp_flow::schema::RecordingFormat::Mp3 => "mp3",
        };
        let started = recorder
            .start(
                &call_id,
                fcp_core::recording::CallRecordingSource::Flow,
                RecordingPath::Dated { extension },
            )
            .await;

        let mut context_guard = context.write().await;
        match started {
            Ok(recording) => {
                info!(
                    call_id = %call_id,
                    recording_path = %recording.file,
                    consent = recording.consent.as_str(),
                    "Recording started successfully"
                );
                context_guard.set_variable(variable, recording.file);
                context_guard.set_variable("recording_status".to_string(), "active".to_string());
                context_guard.set_variable(
                    "recording_consent".to_string(),
                    recording.consent.as_str().to_string(),
                );
                context_guard.current_node = next;
            }
            Err(e) => {
                error!(call_id = %call_id, error = %e, "Failed to start recording");
                let status = match e {
                    RecordingError::ConsentRefused => {
                        context_guard.set_variable(
                            "recording_consent".to_string(),
                            fcp_core::RecordingConsent::Refused.as_str().to_string(),
                        );
                        "refused"
                    }
                    _ => "error",
                };
                context_guard.set_variable(variable, format!("error://{}", e));
                context_guard.set_variable("recording_status".to_string(), status.to_string());
                context_guard.current_node = on_error.unwrap_or(next);
            }
        }
        drop(context_guard);

        Box::pin(self.execute_next_node(call_id)).await
    }

    /// The recorder this handler's recordings go through: the one shared with
    /// the SIP handler when it was given one, else its own.
    fn recorder(&self) -> Option<Arc<crate::recording::CallRecorder>> {
        if let Some(recorder) = self.call_recorder.get() {
            return Some(Arc::clone(recorder));
        }
        let call_manager = self.call_manager.as_ref()?;
        self.media_manager.as_ref()?;
        Some(Arc::clone(self.call_recorder.get_or_init(|| {
            Arc::new(crate::recording::CallRecorder::new(
                Arc::clone(call_manager),
                self.media_manager.clone(),
                self.storage.clone(),
            ))
        })))
    }

    /// Handle a StopRecording node: the call's recorder stops it and closes
    /// its row.
    async fn handle_stop_recording(
        &self,
        call_id: CallId,
        next: String,
        on_error: Option<String>,
    ) -> Result<()> {
        match self.recorder() {
            Some(recorder) => match recorder.stop(&call_id).await {
                Ok(()) => info!(call_id = %call_id, "Recording stopped successfully"),
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
            },
            None => {
                warn!(call_id = %call_id, "MediaManager not configured - ignoring StopRecording")
            }
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

                NodeType::Schedule {
                    schedule_id,
                    open,
                    closed,
                    holiday,
                } => {
                    // The tenant's schedule, now (`docs/ENTERPRISE_CALLING.md`
                    // §3.5). A schedule that cannot be read is open: a flow
                    // is not closed by a database hiccup.
                    use fcp_core::ScheduleStore;
                    let tenant = self
                        .call_manager
                        .as_ref()
                        .and_then(|cm| cm.get_call(&call_id))
                        .map(|c| c.tenant_id);
                    let state = match (&self.storage, schedule_id.parse::<fcp_core::ScheduleId>()) {
                        (Some(storage), Ok(id)) => match storage.get_schedule(&id).await {
                            Ok(Some(schedule)) if Some(schedule.tenant_id) == tenant => {
                                schedule.state_now()
                            }
                            other => {
                                warn!(call_id = %call_id, schedule = %schedule_id, found = ?other.as_ref().map(|o| o.is_some()), "Schedule node: schedule not readable; taking the open branch");
                                fcp_core::ScheduleState::Open
                            }
                        },
                        _ => {
                            warn!(call_id = %call_id, schedule = %schedule_id, "Schedule node: no storage or a bad id; taking the open branch");
                            fcp_core::ScheduleState::Open
                        }
                    };
                    let next = match state {
                        fcp_core::ScheduleState::Open => open.clone(),
                        fcp_core::ScheduleState::Closed => closed.clone(),
                        fcp_core::ScheduleState::Holiday => {
                            holiday.clone().unwrap_or_else(|| closed.clone())
                        }
                    };
                    info!(call_id = %call_id, node_id = %current_node, state = state.as_str(), next = %next, "Schedule node");
                    context_guard
                        .set_variable("schedule_state".to_string(), state.as_str().to_string());
                    context_guard.current_node = next;
                    drop(context_guard);
                    return Box::pin(self.execute_next_node(call_id)).await;
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
                    self.stop_transcript(&call_id);

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
                    NodeType::Play {
                        url,
                        next,
                        loop_count,
                        ..
                    } => {
                        // Played `loop_count` times; 0 and 1 are once, as
                        // flows written before the count worked expect.
                        let times = (*loop_count).max(1);
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

                                // The repeats, each after the last has ended.
                                for _ in 1..times {
                                    match media_manager
                                        .play_file(
                                            &session_id,
                                            resolved_path.clone(),
                                            AudioTarget::ParticipantA,
                                        )
                                        .await
                                    {
                                        Ok(again) => {
                                            let _ =
                                                media_manager.wait_playback_completion(again).await;
                                        }
                                        Err(e) => {
                                            warn!(call_id = %call_id, error = %e, "A repeat of the prompt was not played");
                                            break;
                                        }
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
                tenant_id: self.tenant_of(&call_id),
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
                        tenant_id: self.tenant_of(&call_id),
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
                self.stop_transcript(&call_id);

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
                speech,
                on_error,
            } => {
                info!(
                    call_id = %call_id,
                    variable = %variable,
                    input = ?speech.input,
                    "Flow waiting for the caller's input"
                );
                if speech.input.speech() {
                    // Cleared so a Branch can tell nothing was said.
                    context_guard.set_variable("speech".to_string(), String::new());
                    context_guard.set_variable("speech_confidence".to_string(), String::new());
                }

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
                        tenant_id: self.tenant_of(&call_id),
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

                let Some(media_manager) = self.media_manager.clone() else {
                    warn!(call_id = %call_id, "Media manager not available for a Gather");
                    return Ok(());
                };
                let gather_call_id = media_session_id
                    .unwrap_or_else(|| fcp_core::CallId::from_uuid(*call_id.as_uuid()));
                info!(
                    call_id = %call_id,
                    gather_call_id = %gather_call_id,
                    num_digits = num_digits,
                    finish_on_key = ?finish_on_key,
                    has_prompt = prompt_url.is_some(),
                    transitions_count = transitions.len(),
                    speech_transitions = speech.transitions.len(),
                    "Gather: listening"
                );

                // Listen BEFORE the prompt plays, so input during it is heard.
                let mut listening = crate::flow_listen::Listening::start(
                    &media_manager,
                    &gather_call_id,
                    num_digits,
                    finish_on_key,
                    &speech,
                );
                // Digits that arrive by SIP INFO join the same collection.
                let digit_slot = listening.digit_sender().map(|sender| {
                    self.gather_digits.insert(call_id, sender);
                    GatherDigitSlot {
                        slots: Arc::clone(&self.gather_digits),
                        call_id,
                    }
                });
                if barge_in {
                    listening
                        .listen_for_speech(&media_manager, &gather_call_id, &speech)
                        .await;
                }

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
                        Ok(pid) if barge_in => Some(pid),
                        Ok(pid) => {
                            // No barge-in: the prompt plays out first.
                            if let Err(e) = media_manager.wait_playback_completion(pid).await {
                                warn!(call_id = %call_id, error = %e, "Gather prompt playback failed");
                            }
                            None
                        }
                        Err(e) => {
                            warn!(call_id = %call_id, error = %e, "Failed to play Gather prompt");
                            None
                        }
                    }
                } else {
                    None
                };
                if !barge_in {
                    listening
                        .listen_for_speech(&media_manager, &gather_call_id, &speech)
                        .await;
                }

                let outcome = listening
                    .wait(
                        &media_manager,
                        &gather_call_id,
                        std::time::Duration::from_secs(timeout_secs as u64),
                        playback_id,
                    )
                    .await;
                drop(digit_slot);
                info!(call_id = %call_id, outcome = ?outcome, "Gather complete");

                use crate::flow_listen::GatherOutcome;
                let context_guard = context.write().await;
                let heard = match outcome {
                    GatherOutcome::Digits(digits) => {
                        context_guard.set_variable(variable.clone(), digits.clone());
                        context_guard.set_variable("gather_input".to_string(), "dtmf".to_string());
                        match transitions.get(&digits) {
                            Some(next) => Heard::Chose(next.clone()),
                            None => {
                                warn!(call_id = %call_id, digits = %digits, "No transition found for collected digits");
                                Heard::Invalid
                            }
                        }
                    }
                    GatherOutcome::PartialDigits(digits) => {
                        // The timeout fired before num_digits was reached;
                        // a valid choice may still have been made.
                        context_guard.set_variable(variable.clone(), digits.clone());
                        context_guard.set_variable("gather_input".to_string(), "dtmf".to_string());
                        match transitions.get(&digits) {
                            Some(next) => Heard::Chose(next.clone()),
                            None => Heard::Nothing,
                        }
                    }
                    GatherOutcome::Speech(transcript) => {
                        context_guard.set_variable("speech".to_string(), transcript.text.clone());
                        context_guard.set_variable(
                            "speech_confidence".to_string(),
                            transcript
                                .confidence
                                .map(|c| format!("{c:.2}"))
                                .unwrap_or_default(),
                        );
                        context_guard
                            .set_variable("gather_input".to_string(), "speech".to_string());
                        match fcp_flow::schema::match_speech(&speech.transitions, &transcript.text)
                        {
                            Some(next) => Heard::Chose(next.clone()),
                            None => {
                                warn!(call_id = %call_id, speech = %transcript.text, "No transition found for what was said");
                                Heard::Invalid
                            }
                        }
                    }
                    GatherOutcome::SpeechUnavailable if on_error.is_some() => {
                        Heard::Chose(on_error.clone().unwrap_or_default())
                    }
                    GatherOutcome::Timeout | GatherOutcome::SpeechUnavailable => Heard::Nothing,
                    GatherOutcome::Cancelled => {
                        warn!(call_id = %call_id, "Gather cancelled");
                        // The call has likely ended: nothing more runs.
                        return Ok(());
                    }
                };
                let this_node = context_guard.current_node.clone();
                let attempts_key = format!("__gather_attempts.{this_node}");
                let mut retry_prompt = None;
                let next_node = match heard {
                    Heard::Chose(next) => {
                        context_guard.remove_variable(&attempts_key);
                        next
                    }
                    failed => {
                        let max_attempts = context_guard
                            .get_variable("__gather_max_attempts")
                            .ok()
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(1);
                        let attempt = context_guard
                            .get_variable(&attempts_key)
                            .ok()
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(0)
                            + 1;
                        if attempt < max_attempts {
                            // Asked again, after the retry prompt for what
                            // went wrong.
                            info!(call_id = %call_id, attempt, max_attempts, nothing_heard = matches!(failed, Heard::Nothing), "Gather: asking again");
                            context_guard.set_variable(attempts_key, attempt.to_string());
                            let prompt = match failed {
                                Heard::Nothing => "__gather_no_input_url",
                                _ => "__gather_invalid_url",
                            };
                            retry_prompt = context_guard.get_variable(prompt).ok();
                            this_node
                        } else {
                            context_guard.remove_variable(&attempts_key);
                            let spent = (max_attempts > 1)
                                .then(|| {
                                    context_guard.get_variable("__gather_on_max_attempts").ok()
                                })
                                .flatten();
                            let fallback = match failed {
                                Heard::Nothing => on_timeout_node.or_else(|| default.clone()),
                                _ => default.clone(),
                            };
                            match spent.or(fallback) {
                                Some(next) => next,
                                None => {
                                    // No branch for it: the Gather runs again.
                                    warn!(call_id = %call_id, "Gather: no branch for a failed attempt");
                                    this_node
                                }
                            }
                        }
                    }
                };
                drop(context_guard);
                if let (Some(url), Some(session_id), Some(media_manager)) =
                    (retry_prompt, media_session_id, self.media_manager.as_ref())
                {
                    let url = self.resolve_prompt_url(&url).await.unwrap_or(url);
                    match media_manager
                        .play_file(&session_id, url, fcp_media::AudioTarget::ParticipantA)
                        .await
                    {
                        Ok(pid) => {
                            let _ = media_manager.wait_playback_completion(pid).await;
                        }
                        Err(e) => {
                            warn!(call_id = %call_id, error = %e, "Gather retry prompt not played")
                        }
                    }
                }
                let mut context_guard = context.write().await;
                info!(call_id = %call_id, next_node = %next_node, "Gather continuing to next node");
                context_guard.current_node = next_node;
                drop(context_guard);
                Box::pin(self.execute_next_node(call_id)).await?;
            }

            FlowExecutionResult::Transcribe {
                action,
                leg,
                language,
                hints,
                interim_results,
                next,
                on_error,
            } => {
                let media_session_id = context_guard.media_session_id;
                drop(context_guard);
                let next_node = match action {
                    fcp_flow::schema::TranscribeAction::Stop => {
                        self.stop_transcript(&call_id);
                        next
                    }
                    fcp_flow::schema::TranscribeAction::Start => {
                        let request = crate::flow_listen::TranscriptRequest {
                            leg,
                            language,
                            hints,
                            interim_results,
                        };
                        match self
                            .start_transcript(call_id, media_session_id, &request)
                            .await
                        {
                            Ok(()) => next,
                            Err(e) => {
                                warn!(call_id = %call_id, error = %e, "The call's transcript cannot start");
                                on_error.unwrap_or(next)
                            }
                        }
                    }
                };
                context.write().await.current_node = next_node;
                Box::pin(self.execute_next_node(call_id)).await?;
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

            FlowExecutionResult::WaitingForHotelingLogin {
                device_id,
                max_attempts,
                next,
                on_failure,
            } => {
                drop(context_guard);
                self.handle_hoteling_login(call_id, device_id, max_attempts, next, on_failure)
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

    /// A digit from the caller that arrived by SIP INFO rather than in the
    /// media. It joins the call's Gather in progress, as an RFC 2833 digit
    /// would; outside a Gather it is only noted in `dtmf_input`.
    pub async fn handle_dtmf_input(&self, call_id: CallId, digits: String) -> Result<()> {
        info!(call_id = %call_id, digits = %digits, "Received DTMF input");
        let context = self
            .contexts
            .get(&call_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| anyhow::anyhow!("No flow context for call: {}", call_id))?;
        context
            .read()
            .await
            .set_variable("dtmf_input".to_string(), digits.clone());
        match self.gather_digits.get(&call_id) {
            Some(sender) => {
                for digit in digits.chars() {
                    // A closed collection has just finished: the digit is late.
                    let _ = sender.send(digit);
                }
            }
            None => debug!(call_id = %call_id, "A digit outside a Gather"),
        }
        Ok(())
    }

    /// Start a call's live transcript (`docs/DEVELOPER_SURFACE.md` phase 7),
    /// replacing one already running.
    async fn start_transcript(
        &self,
        call_id: CallId,
        media_session_id: Option<fcp_core::CallId>,
        request: &crate::flow_listen::TranscriptRequest,
    ) -> Result<()> {
        let media = self
            .media_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no media manager"))?;
        let session =
            media_session_id.unwrap_or_else(|| fcp_core::CallId::from_uuid(*call_id.as_uuid()));
        let tenant_id = self
            .call_manager
            .as_ref()
            .and_then(|call_manager| call_manager.get_call(&call_id))
            .map(|call| call.tenant_id)
            .ok_or_else(|| anyhow::anyhow!("the call is gone"))?;
        let tasks = crate::flow_listen::start_transcript(
            media,
            &session,
            call_id,
            tenant_id,
            self.events.clone(),
            request,
        )
        .await?;
        self.stop_transcript(&call_id);
        self.transcripts.insert(call_id, tasks);
        Ok(())
    }

    /// Stop a call's live transcript, if one runs.
    fn stop_transcript(&self, call_id: &CallId) {
        if let Some((_, tasks)) = self.transcripts.remove(call_id) {
            for task in tasks {
                task.abort();
            }
        }
    }

    /// Stop flow execution for a call.
    pub async fn stop_flow(&self, call_id: CallId) -> Result<()> {
        info!(call_id = %call_id, "Stopping flow execution");

        self.stop_transcript(&call_id);
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
                    tenant_id: self.tenant_of(&call_id),
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

    /// The tenant of a call, for the events that name it.
    fn tenant_of(&self, call_id: &CallId) -> Option<fcp_core::TenantId> {
        self.call_manager
            .as_ref()
            .and_then(|cm| cm.get_call(call_id))
            .map(|call| call.tenant_id)
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

/// The voicemail menu's opening: the counts as spoken English, and only the
/// choices there is something behind (`docs/DEEP_TESTING.md` D8: "You have 0
/// new messages… Press 1 for new messages").
fn main_menu_text(new_count: usize, saved_count: usize) -> String {
    let count = |n: usize, kind: &str| match n {
        0 => format!("no {kind} messages"),
        1 => format!("1 {kind} message"),
        n => format!("{n} {kind} messages"),
    };
    let mut text = match (new_count, saved_count) {
        (0, 0) => "You have no messages.".to_string(),
        (n, 0) => format!("You have {}.", count(n, "new")),
        (0, s) => format!("You have no new messages, and {}.", count(s, "saved")),
        (n, s) => format!("You have {} and {}.", count(n, "new"), count(s, "saved")),
    };
    if new_count > 0 {
        text.push_str(" Press 1 for new messages.");
    }
    if saved_count > 0 {
        text.push_str(" Press 2 for saved messages.");
    }
    text.push_str(" Press star to exit.");
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_menu_offers_only_what_there_is() {
        assert_eq!(
            super::main_menu_text(0, 0),
            "You have no messages. Press star to exit."
        );
        assert_eq!(
            super::main_menu_text(1, 0),
            "You have 1 new message. Press 1 for new messages. Press star to exit."
        );
        assert_eq!(
            super::main_menu_text(0, 2),
            "You have no new messages, and 2 saved messages. Press 2 for saved messages. \
             Press star to exit."
        );
        assert_eq!(
            super::main_menu_text(3, 1),
            "You have 3 new messages and 1 saved message. Press 1 for new messages. \
             Press 2 for saved messages. Press star to exit."
        );
    }

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

/// The transport a SIP URI names: `sips:` is TLS, else its `transport`
/// parameter, else UDP.
pub(crate) fn transport_of(uri: &sip_core::SipUri) -> sip_transaction::fsm::TransportKind {
    use sip_transaction::fsm::TransportKind;
    if uri.is_sips() {
        return TransportKind::Tls;
    }
    match uri
        .params()
        .get(&smol_str::SmolStr::new("transport"))
        .and_then(|v| v.as_ref())
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        Some("tcp") => TransportKind::Tcp,
        Some("tls") => TransportKind::Tls,
        Some("ws") => TransportKind::Ws,
        Some("wss") => TransportKind::Wss,
        _ => TransportKind::Udp,
    }
}

#[cfg(test)]
mod transport_of_tests {
    use super::transport_of;
    use sip_transaction::fsm::TransportKind;

    #[test]
    fn the_uri_names_its_transport() {
        let uri = |s: &str| sip_core::SipUri::parse(s).unwrap();
        assert_eq!(
            transport_of(&uri("sip:+1@10.0.0.1:5060")),
            TransportKind::Udp
        );
        assert_eq!(
            transport_of(&uri("sip:+1@10.0.0.1:5060;transport=tcp")),
            TransportKind::Tcp
        );
        assert_eq!(
            transport_of(&uri("sip:+1@carrier.example:5061;transport=tls")),
            TransportKind::Tls
        );
        assert_eq!(
            transport_of(&uri("sips:+1@carrier.example")),
            TransportKind::Tls
        );
    }
}

/// The user an extension and PIN log in: the extension's user (or the
/// username), enabled, with a PIN that matches.
async fn hoteling_guest(
    storage: &fcp_core::DynStorage,
    tenant_id: &fcp_core::TenantId,
    extension: &str,
    pin: &str,
) -> Option<fcp_core::User> {
    use fcp_core::storage::CredentialStore;
    let user_id = crate::ring_plan::user_of(storage, tenant_id, extension).await?;
    let user = storage.get_user_by_id(&user_id).await.ok().flatten()?;
    let hash = user.pin.clone().filter(|_| user.may_use_services())?;
    // bcrypt: off the async workers.
    let pin = pin.to_string();
    let matches = tokio::task::spawn_blocking(move || {
        fcp_core::storage::password::verify_pin(&pin, &hash).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    matches.then_some(user)
}
