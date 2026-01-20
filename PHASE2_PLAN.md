# FCP Phase 2: Programmable Voice Platform
## "Rust-native Twilio" - Months 5-8 (16 weeks)

---

## Executive Summary

**Goal:** Transform FCP from a traditional PBX into a programmable voice platform (CPaaS) with:
- Call flow scripting (similar to TwiML)
- Webhook integration for call control
- Real-time event streaming
- IVR with DTMF support
- Call recording
- WebSocket API for live events
- Advanced routing engine

**End State Demo:**
```
┌──────────────┐         ┌─────────────┐         ┌──────────────┐
│   Caller     │◄───────►│             │◄───────►│   External   │
│   (PSTN)     │   SIP    │     FCP     │  HTTPS  │   App/API    │
│              │         │   Server    │         │   Consumer   │
└──────────────┘         │             │         └──────────────┘
                         │   ┌─────┐   │
                         │   │Flow │   │         ┌──────────────┐
                         │   │Engine│  │◄───────►│   Webhook    │
                         │   └─────┘   │  HTTPS  │   Server     │
                         │             │         └──────────────┘
                         │   ┌─────┐   │
                         │   │ IVR │   │         ┌──────────────┐
                         │   └─────┘   │  WSS    │   WebSocket  │
                         │             │◄───────►│   Client     │
                         └─────────────┘         └──────────────┘
                               │
                         Event Stream
                               │
                         ┌─────▼─────┐
                         │  Redis/   │
                         │   NATS    │
                         └───────────┘
```

---

## Product Vision: "Twilio, but Self-Hosted"

### What Twilio Does (that we'll replicate)

1. **Call Control via Webhooks**
   - App sends INVITE → FCP calls webhook → App returns instructions
   - TwiML-like flow definition (but JSON-based)

2. **Real-time Events**
   - WebSocket streaming of call events
   - Call progress, DTMF, transcriptions

3. **IVR/Menu System**
   - Play prompts
   - Gather DTMF input
   - Text-to-Speech (future)

4. **Recording**
   - On-demand call recording
   - Automatic recording with webhooks

5. **REST API for Everything**
   - Initiate calls
   - Control active calls
   - Query CDRs
   - Manage resources

### Our Advantages Over Twilio

| Feature | Twilio | FCP |
|---------|--------|-----|
| **Deployment** | Cloud only | Self-hosted, full control |
| **Cost** | Per-minute billing | Infrastructure cost only |
| **Data** | Leaves your network | Stays on-premise |
| **SIP Native** | Yes, but abstracted | Full SIP exposure |
| **Customization** | Limited | Full source access |
| **Compliance** | Shared infrastructure | Dedicated deployment |
| **Multi-tenancy** | Account-based | Built-in from day 1 |

---

## Sprint Overview (8 x 2-week sprints)

```
Week  1-2:  Sprint 9  - Event System & WebSocket API
Week  3-4:  Sprint 10 - Flow Engine Foundation (JSON-based DSL)
Week  5-6:  Sprint 11 - IVR & DTMF Handling
Week  7-8:  Sprint 12 - Webhook Dispatcher & HTTP Client
Week  9-10: Sprint 13 - Recording Pipeline & Storage
Week 11-12: Sprint 14 - Advanced Routing Engine
Week 13-14: Sprint 15 - Flow Nodes (Say, Play, Gather, Dial)
Week 15-16: Sprint 16 - Integration & Examples
```

---

## Sprint 9: Event System & WebSocket API (Weeks 1-2)

### Goals
- Design and implement event bus architecture
- Create WebSocket API for real-time event streaming
- Publish call lifecycle events

### 9.1 Event Bus Architecture

```rust
// crates/fcp-events/src/lib.rs

/// Core event types that flow through the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlatformEvent {
    Call(CallEvent),
    Media(MediaEvent),
    Ivr(IvrEvent),
    Recording(RecordingEvent),
    System(SystemEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallEvent {
    /// Call initiated (INVITE received/sent)
    Initiated {
        call_id: CallId,
        tenant_id: TenantId,
        from: String,
        to: String,
        direction: CallDirection,
        timestamp: SystemTime,
    },

    /// Call is ringing (180/183 received)
    Ringing {
        call_id: CallId,
        timestamp: SystemTime,
    },

    /// Call answered (200 OK)
    Answered {
        call_id: CallId,
        answer_time: SystemTime,
        codec: String,
    },

    /// Call ended (BYE/timeout/error)
    Ended {
        call_id: CallId,
        end_time: SystemTime,
        duration_ms: u64,
        hangup_cause: HangupCause,
        hangup_party: Option<HangupParty>,
    },

    /// Call failed (4xx/5xx/6xx)
    Failed {
        call_id: CallId,
        timestamp: SystemTime,
        reason: String,
        sip_code: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaEvent {
    /// RTP session started
    SessionStarted {
        session_id: SessionId,
        call_id: CallId,
        codec: String,
        remote_addr: SocketAddr,
    },

    /// DTMF digit received
    DtmfReceived {
        call_id: CallId,
        digit: char,
        timestamp: SystemTime,
    },

    /// Media quality degraded
    QualityWarning {
        call_id: CallId,
        packet_loss: f32,
        jitter_ms: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordingEvent {
    Started {
        recording_id: RecordingId,
        call_id: CallId,
        timestamp: SystemTime,
    },

    Completed {
        recording_id: RecordingId,
        call_id: CallId,
        duration_ms: u64,
        size_bytes: u64,
        url: String,
    },
}

/// Event publisher trait
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: PlatformEvent) -> Result<()>;
}

/// Event subscriber trait
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream>;
}

/// Event stream (async iterator)
pub struct EventStream {
    receiver: tokio::sync::mpsc::Receiver<PlatformEvent>,
}

impl EventStream {
    pub async fn next(&mut self) -> Option<PlatformEvent> {
        self.receiver.recv().await
    }
}
```

### 9.2 WebSocket API

```rust
// crates/fcp-server/src/api/ws/events.rs

/// WebSocket connection handler for event streaming
pub async fn events_handler(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
    Query(params): Query<EventStreamParams>,
) -> Response {
    ws.on_upgrade(move |socket| handle_events_stream(socket, state, params))
}

async fn handle_events_stream(
    mut socket: WebSocket,
    state: ApiState,
    params: EventStreamParams,
) {
    // Subscribe to events based on filters
    let filter = EventFilter {
        tenant_id: params.tenant_id,
        call_id: params.call_id,
        event_types: params.event_types,
    };

    let mut stream = state.event_bus
        .subscribe(filter)
        .await
        .expect("Failed to subscribe to events");

    // Stream events to WebSocket client
    while let Some(event) = stream.next().await {
        let json = serde_json::to_string(&event).unwrap();
        if socket.send(Message::Text(json)).await.is_err() {
            break;
        }
    }
}

// Example client usage (JavaScript):
// const ws = new WebSocket('wss://fcp.example.com/api/v1/events?tenant_id=tenant-123');
// ws.onmessage = (event) => {
//   const data = JSON.parse(event.data);
//   console.log('Call event:', data);
// };
```

### 9.3 Event Bus Implementation Options

**Option 1: In-Memory (Single Node)**
```rust
pub struct InMemoryEventBus {
    subscribers: Arc<RwLock<Vec<Subscriber>>>,
}
```

**Option 2: Redis Streams (Multi-Node)**
```rust
pub struct RedisEventBus {
    client: redis::Client,
    stream_key: String,
}
```

**Option 3: NATS (High Throughput)**
```rust
pub struct NatsEventBus {
    client: async_nats::Client,
}
```

**Decision:** Start with in-memory, design for Redis/NATS later.

### Deliverable
- ✅ Event bus abstraction with in-memory implementation
- ✅ WebSocket API endpoint: `wss://host/api/v1/events`
- ✅ Call lifecycle events published from CallManager
- ✅ Example WebSocket client (JavaScript)

---

## Sprint 10: Flow Engine Foundation (Weeks 3-4)

### Goals
- Design JSON-based call flow DSL
- Implement flow engine runtime
- Create basic flow nodes (Say, Hangup, Redirect)

### 10.1 Flow DSL Design

```json
{
  "version": "1.0",
  "name": "Simple IVR",
  "start_node": "welcome",
  "nodes": [
    {
      "id": "welcome",
      "type": "Say",
      "config": {
        "text": "Welcome to FCP. Press 1 for sales, 2 for support.",
        "voice": "alice",
        "language": "en-US"
      },
      "next": "gather_input"
    },
    {
      "id": "gather_input",
      "type": "Gather",
      "config": {
        "num_digits": 1,
        "timeout_seconds": 5,
        "finish_on_key": "#"
      },
      "on_input": {
        "1": "dial_sales",
        "2": "dial_support",
        "timeout": "no_input",
        "default": "invalid_input"
      }
    },
    {
      "id": "dial_sales",
      "type": "Dial",
      "config": {
        "destination": "sip:sales@example.com",
        "timeout_seconds": 30
      },
      "on_complete": {
        "answered": "end",
        "busy": "sales_busy",
        "no_answer": "sales_voicemail"
      }
    },
    {
      "id": "end",
      "type": "Hangup",
      "config": {
        "reason": "normal"
      }
    }
  ]
}
```

### 10.2 Flow Engine Implementation

```rust
// crates/fcp-flow/src/lib.rs

pub struct FlowEngine {
    flows: Arc<DashMap<FlowId, Flow>>,
    runtime: Arc<FlowRuntime>,
}

pub struct Flow {
    pub id: FlowId,
    pub version: String,
    pub name: String,
    pub start_node: String,
    pub nodes: HashMap<String, FlowNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FlowNode {
    Say {
        id: String,
        config: SayConfig,
        next: Option<String>,
    },
    Play {
        id: String,
        config: PlayConfig,
        next: Option<String>,
    },
    Gather {
        id: String,
        config: GatherConfig,
        on_input: HashMap<String, String>, // input -> next_node_id
    },
    Dial {
        id: String,
        config: DialConfig,
        on_complete: HashMap<String, String>, // outcome -> next_node_id
    },
    Webhook {
        id: String,
        config: WebhookConfig,
        on_response: HashMap<String, String>,
    },
    Hangup {
        id: String,
        config: HangupConfig,
    },
    Branch {
        id: String,
        condition: Condition,
        on_true: String,
        on_false: String,
    },
}

pub struct FlowRuntime {
    pub call_manager: Arc<CallManager>,
    pub media_manager: Arc<MediaManager>,
    pub webhook_client: Arc<WebhookClient>,
}

impl FlowEngine {
    pub async fn execute_flow(
        &self,
        flow_id: &FlowId,
        call_id: &CallId,
        context: FlowContext,
    ) -> Result<()> {
        let flow = self.flows.get(flow_id)
            .ok_or_else(|| anyhow!("Flow not found"))?;

        let mut current_node_id = flow.start_node.clone();
        let mut ctx = context;

        loop {
            let node = flow.nodes.get(&current_node_id)
                .ok_or_else(|| anyhow!("Node not found: {}", current_node_id))?;

            match self.execute_node(node, call_id, &mut ctx).await? {
                NodeResult::Continue(next_id) => {
                    current_node_id = next_id;
                }
                NodeResult::Hangup => {
                    break;
                }
                NodeResult::Wait => {
                    // Node is async (e.g., Gather waiting for DTMF)
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}
```

### 10.3 Flow Storage

```rust
// Store flows in database
pub trait FlowStore: Send + Sync {
    async fn save_flow(&self, flow: &Flow) -> Result<()>;
    async fn load_flow(&self, flow_id: &FlowId) -> Result<Option<Flow>>;
    async fn list_flows(&self, tenant_id: &TenantId) -> Result<Vec<Flow>>;
    async fn delete_flow(&self, flow_id: &FlowId) -> Result<()>;
}

// REST API for flow management
// POST /api/v1/flows - Create flow
// GET /api/v1/flows/:id - Get flow
// PUT /api/v1/flows/:id - Update flow
// DELETE /api/v1/flows/:id - Delete flow
// GET /api/v1/flows - List flows
```

### Deliverable
- ✅ Flow engine runtime
- ✅ JSON DSL parser and validator
- ✅ Basic nodes: Say, Hangup, Redirect
- ✅ Flow storage in database
- ✅ REST API for flow management

---

## Sprint 11: IVR & DTMF Handling (Weeks 5-6)

### Goals
- Implement DTMF detection and processing
- Add Gather node for IVR menus
- Add Play node for audio files
- Create prompt storage system

### 11.1 DTMF Detection

```rust
// crates/fcp-media/src/dtmf.rs

pub struct DtmfDetector {
    detector: goertzel::DtmfDetector, // or custom implementation
}

impl DtmfDetector {
    pub fn process_samples(&mut self, samples: &[i16]) -> Option<char> {
        // Goertzel algorithm for DTMF detection
        self.detector.process(samples)
    }
}

// In media pipeline
impl MediaSession {
    pub async fn enable_dtmf_detection(&mut self) {
        let detector = DtmfDetector::new();

        // Hook into RTP receive pipeline
        self.on_rtp_packet(move |packet| {
            if let Some(digit) = detector.process_samples(&packet.payload) {
                // Publish DTMF event
                event_bus.publish(MediaEvent::DtmfReceived {
                    call_id,
                    digit,
                    timestamp: SystemTime::now(),
                }).await;
            }
        });
    }
}
```

### 11.2 Gather Node Implementation

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatherConfig {
    /// Prompt to play before gathering
    pub prompt: Option<PromptRef>,

    /// Number of digits to collect
    pub num_digits: Option<usize>,

    /// Timeout for first digit (seconds)
    pub timeout_seconds: u32,

    /// Timeout between digits (seconds)
    pub inter_digit_timeout: Option<u32>,

    /// Finish on this key (usually #)
    pub finish_on_key: Option<char>,

    /// Minimum number of digits
    pub min_digits: Option<usize>,
}

pub struct GatherNode {
    config: GatherConfig,
    on_input: HashMap<String, String>,
}

impl FlowNode for GatherNode {
    async fn execute(&self, ctx: &mut FlowContext) -> Result<NodeResult> {
        // Play prompt if specified
        if let Some(prompt) = &self.config.prompt {
            ctx.media_manager.play_prompt(ctx.call_id, prompt).await?;
        }

        // Enable DTMF detection
        ctx.media_manager.enable_dtmf(ctx.call_id).await?;

        // Wait for input (async)
        let input = ctx.wait_for_dtmf_input(
            self.config.num_digits,
            Duration::from_secs(self.config.timeout_seconds as u64),
        ).await?;

        // Match input to next node
        let next_node = self.on_input.get(&input)
            .or_else(|| self.on_input.get("default"))
            .ok_or_else(|| anyhow!("No handler for input: {}", input))?;

        Ok(NodeResult::Continue(next_node.clone()))
    }
}
```

### 11.3 Prompt Management

```rust
// Store audio prompts
pub struct PromptStore {
    storage_path: PathBuf,
    db: Arc<DynStorage>,
}

pub struct Prompt {
    pub id: PromptId,
    pub tenant_id: TenantId,
    pub name: String,
    pub format: AudioFormat, // WAV, MP3, etc.
    pub duration_ms: u32,
    pub file_path: String,
    pub created_at: SystemTime,
}

// REST API
// POST /api/v1/prompts - Upload prompt
// GET /api/v1/prompts/:id - Download prompt
// DELETE /api/v1/prompts/:id - Delete prompt
// GET /api/v1/prompts - List prompts
```

### Deliverable
- ✅ DTMF detection in media pipeline
- ✅ Gather node implementation
- ✅ Play node implementation
- ✅ Prompt storage system
- ✅ Example IVR flow (main menu)

---

## Sprint 12: Webhook Dispatcher (Weeks 7-8)

### Goals
- Implement HTTP client for webhook calls
- Add Webhook node to flow engine
- Handle webhook responses and flow continuation
- Add retry logic and failure handling

### 12.1 Webhook Client

```rust
// crates/fcp-webhook/src/lib.rs

pub struct WebhookClient {
    client: reqwest::Client,
    config: WebhookConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookRequest {
    pub event: String,
    pub timestamp: SystemTime,
    pub call: CallContext,
    pub custom_data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallContext {
    pub call_id: String,
    pub from: String,
    pub to: String,
    pub direction: String,
    pub state: String,
}

impl WebhookClient {
    pub async fn call_webhook(
        &self,
        url: &str,
        request: &WebhookRequest,
    ) -> Result<WebhookResponse> {
        let response = self.client
            .post(url)
            .json(request)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if response.status().is_success() {
            let webhook_response: WebhookResponse = response.json().await?;
            Ok(webhook_response)
        } else {
            Err(anyhow!("Webhook returned {}", response.status()))
        }
    }

    pub async fn call_webhook_with_retry(
        &self,
        url: &str,
        request: &WebhookRequest,
    ) -> Result<WebhookResponse> {
        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;

            match self.call_webhook(url, request).await {
                Ok(response) => return Ok(response),
                Err(e) if attempts < max_attempts => {
                    warn!("Webhook failed (attempt {}/{}): {}", attempts, max_attempts, e);
                    tokio::time::sleep(Duration::from_secs(1 << attempts)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookResponse {
    /// Next action to take
    pub action: WebhookAction,

    /// Additional data to add to flow context
    pub data: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WebhookAction {
    Continue { next_node: String },
    Hangup { reason: Option<String> },
    Dial { destination: String },
    Say { text: String },
    Redirect { flow_id: String },
}
```

### 12.2 Webhook Node

```rust
pub struct WebhookNode {
    pub url: String,
    pub method: HttpMethod,
    pub on_response: HashMap<String, String>, // response.action -> next_node
}

impl FlowNode for WebhookNode {
    async fn execute(&self, ctx: &mut FlowContext) -> Result<NodeResult> {
        let request = WebhookRequest {
            event: "webhook_call".to_string(),
            timestamp: SystemTime::now(),
            call: CallContext {
                call_id: ctx.call_id.to_string(),
                from: ctx.call.caller().to_string(),
                to: ctx.call.callee().unwrap_or("").to_string(),
                direction: "inbound".to_string(), // or "outbound"
                state: format!("{:?}", ctx.call.state),
            },
            custom_data: ctx.custom_data.clone(),
        };

        let response = ctx.webhook_client
            .call_webhook_with_retry(&self.url, &request)
            .await?;

        // Merge response data into context
        if let Some(data) = response.data {
            ctx.custom_data.extend(data);
        }

        // Determine next node based on action
        match response.action {
            WebhookAction::Continue { next_node } => {
                Ok(NodeResult::Continue(next_node))
            }
            WebhookAction::Hangup { reason } => {
                ctx.call_manager.terminate_call(&ctx.call_id, reason).await?;
                Ok(NodeResult::Hangup)
            }
            // ... handle other actions
        }
    }
}
```

### Deliverable
- ✅ HTTP webhook client with retry logic
- ✅ Webhook node implementation
- ✅ Webhook request/response schemas
- ✅ Example webhook server (Python/Node.js)
- ✅ Documentation for webhook integration

---

## Sprint 13: Recording Pipeline (Weeks 9-10)

### Goals
- Implement call recording (already partially done)
- Add recording to flow engine
- Create recording storage and retrieval
- Add recording webhooks

### 13.1 Recording Node

```rust
pub struct RecordNode {
    pub config: RecordConfig,
    pub on_complete: String, // next node after recording
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordConfig {
    /// Maximum recording duration (seconds)
    pub max_duration: u32,

    /// End recording on this DTMF key
    pub finish_on_key: Option<char>,

    /// Silence timeout (seconds)
    pub silence_timeout: Option<u32>,

    /// Webhook to call when recording completes
    pub webhook_url: Option<String>,
}

impl FlowNode for RecordNode {
    async fn execute(&self, ctx: &mut FlowContext) -> Result<NodeResult> {
        // Start recording
        let recording_id = ctx.media_manager
            .start_recording(ctx.call_id, &self.config)
            .await?;

        // Wait for recording to complete
        ctx.wait_for_recording_complete(
            recording_id,
            Duration::from_secs(self.config.max_duration as u64),
        ).await?;

        // Call webhook if specified
        if let Some(webhook_url) = &self.config.webhook_url {
            let request = WebhookRequest {
                event: "recording_complete".to_string(),
                // ... recording details
            };
            ctx.webhook_client.call_webhook(webhook_url, &request).await?;
        }

        Ok(NodeResult::Continue(self.on_complete.clone()))
    }
}
```

### Deliverable
- ✅ Record node implementation
- ✅ Recording webhook notifications
- ✅ Recording download API
- ✅ Example: Voicemail drop flow

---

## Sprint 14: Advanced Routing Engine (Weeks 11-12)

### Goals
- Implement dynamic routing based on rules
- Add time-based routing
- Add LCR (Least Cost Routing) foundation
- Add failover routing

### 14.1 Routing Engine

```rust
// crates/fcp-routing/src/engine.rs

pub struct RoutingEngine {
    rules: Arc<RwLock<Vec<RoutingRule>>>,
    db: Arc<DynStorage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: RuleId,
    pub tenant_id: TenantId,
    pub priority: i32,
    pub conditions: Vec<RoutingCondition>,
    pub actions: Vec<RoutingAction>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingCondition {
    /// Match caller number pattern
    FromMatches { pattern: String },

    /// Match called number pattern
    ToMatches { pattern: String },

    /// Time of day range
    TimeOfDay { start: String, end: String, timezone: String },

    /// Day of week
    DayOfWeek { days: Vec<String> },

    /// Custom header match
    HeaderMatches { header: String, pattern: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingAction {
    /// Route to destination
    Route { destination: String },

    /// Execute flow
    ExecuteFlow { flow_id: FlowId },

    /// Forward to webhook
    Webhook { url: String },

    /// Reject with status
    Reject { code: u16, reason: String },
}

impl RoutingEngine {
    pub async fn route_call(
        &self,
        call: &Call,
        tenant_id: &TenantId,
    ) -> Result<RoutingDecision> {
        let rules = self.rules.read().await;

        // Filter rules by tenant and priority
        let mut applicable_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.tenant_id == *tenant_id && r.enabled)
            .collect();
        applicable_rules.sort_by_key(|r| r.priority);

        // Evaluate rules in order
        for rule in applicable_rules {
            if self.evaluate_conditions(call, &rule.conditions).await? {
                return Ok(RoutingDecision::from_actions(&rule.actions));
            }
        }

        // Default: reject
        Ok(RoutingDecision::Reject {
            code: 404,
            reason: "No route found".to_string(),
        })
    }
}
```

### Deliverable
- ✅ Routing engine with rule evaluation
- ✅ REST API for routing rules
- ✅ Time-based routing
- ✅ Pattern matching (regex support)

---

## Sprint 15: Flow Nodes Completion (Weeks 13-14)

### Goals
- Implement all remaining flow nodes
- Add advanced features (conferencing, transfer)
- Create flow testing framework

### 15.1 Remaining Nodes

```rust
// Dial node (already mostly done)
// Conference node
// Transfer node
// Queue node
// Voicemail node
// Bridge node
```

### Deliverable
- ✅ Complete flow node library
- ✅ Flow testing framework
- ✅ Example flows library

---

## Sprint 16: Integration & Examples (Weeks 15-16)

### Goals
- Create example applications
- Write comprehensive documentation
- Build demo flows

### 16.1 Example Applications

1. **Simple IVR**
   - Main menu
   - Departmental routing
   - Business hours handling

2. **Voicemail System**
   - Record message
   - Email notification
   - Playback interface

3. **Call Center Queue**
   - Queue position announcements
   - Hold music
   - Agent assignment

4. **Survey System**
   - DTMF collection
   - Response storage
   - Webhook integration

### Deliverable
- ✅ 4+ example applications
- ✅ Documentation site
- ✅ Video tutorials

---

## Key Deliverables Summary

By end of Phase 2, FCP will have:

1. ✅ **Event-Driven Architecture**
   - Real-time WebSocket API
   - Event bus for extensibility

2. ✅ **Programmable Call Flows**
   - JSON-based DSL
   - Visual flow builder (future)
   - 10+ flow node types

3. ✅ **Webhook Integration**
   - HTTP client with retry
   - Flexible request/response
   - Example webhook servers

4. ✅ **IVR System**
   - DTMF detection
   - Prompt management
   - Gather/Play nodes

5. ✅ **Call Recording**
   - On-demand recording
   - Automatic recording
   - Webhook notifications

6. ✅ **Advanced Routing**
   - Rule-based routing
   - Time-based routing
   - LCR foundation

7. ✅ **Developer Experience**
   - Comprehensive API
   - Example applications
   - Documentation site

---

## API Endpoints Added in Phase 2

### Flow Management
- POST `/api/v1/flows` - Create flow
- GET `/api/v1/flows/:id` - Get flow
- PUT `/api/v1/flows/:id` - Update flow
- DELETE `/api/v1/flows/:id` - Delete flow
- GET `/api/v1/flows` - List flows

### Prompts
- POST `/api/v1/prompts` - Upload prompt
- GET `/api/v1/prompts/:id` - Download prompt
- DELETE `/api/v1/prompts/:id` - Delete prompt
- GET `/api/v1/prompts` - List prompts

### Routing Rules
- POST `/api/v1/routing/rules` - Create rule
- GET `/api/v1/routing/rules/:id` - Get rule
- PUT `/api/v1/routing/rules/:id` - Update rule
- DELETE `/api/v1/routing/rules/:id` - Delete rule
- GET `/api/v1/routing/rules` - List rules

### Recordings
- GET `/api/v1/recordings/:id` - Download recording
- DELETE `/api/v1/recordings/:id` - Delete recording
- GET `/api/v1/recordings` - List recordings

### WebSocket
- WSS `/api/v1/events` - Real-time event stream

---

## Success Criteria

### Milestone 1 (Week 2): "First Event"
- [ ] Publish call event to event bus
- [ ] Receive event via WebSocket

### Milestone 2 (Week 4): "First Flow"
- [ ] Create flow via API
- [ ] Execute simple flow (Say → Hangup)

### Milestone 3 (Week 8): "First Webhook"
- [ ] Call external webhook
- [ ] Receive and process response

### Milestone 4 (Week 12): "First IVR"
- [ ] Complete IVR menu with DTMF
- [ ] Route based on input

### Milestone 5 (Week 16): "Production CPaaS"
- [ ] All flow nodes implemented
- [ ] Example apps deployed
- [ ] Documentation complete
- [ ] Performance tested

---

## Technical Debt & Future Work

Items deferred to Phase 3:
- Text-to-Speech (TTS)
- Speech-to-Text (STT)
- Conference bridge
- Advanced queue features
- Voicemail system
- WebRTC integration
- Multi-region deployment

---

**Phase 2 Target:** Programmable voice platform comparable to Twilio/SignalWire for basic use cases, but self-hosted and SIP-native.
