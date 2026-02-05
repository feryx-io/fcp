# Ferrous Communications Platform (FCP)
## Architecture Blueprint v0.2

---

## 1. Vision & Strategic Positioning

### 1.1 What is FCP?

**Ferrous Communications Platform** is a next-generation, carrier-grade unified communications platform written entirely in Rust. Think: *"Rust-native FreeSWITCH/Kamailio/FusionPBX had a baby, and it was multi-tenant from day one."*

### 1.2 Key Differentiators

| vs. Legacy Systems | FCP Advantage |
|-------------------|---------------|
| FreeSWITCH/Asterisk | No GC pauses, memory safety, modern async I/O |
| Kamailio/OpenSIPS | Integrated media + signaling, not just proxy |
| Odin/FusionPBX | Multi-tenancy baked in, not bolted on |
| Twilio/SignalWire | Self-hosted, SIP-native, no per-minute fees |

**Core Design Principles:**
- **Multi-tenancy from day one**: `TenantId` + `TenantContext` in every ID, context, and decision
- **API-first**: REST/gRPC/WebSocket as core design, not afterthought
- **Observable by default**: Prometheus, OpenTelemetry, structured logging everywhere
- **Safety & correctness**: Rust's guarantees enable fuzzing, property tests, fearless refactoring

### 1.3 Phased Product Strategy

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         FCP PRODUCT EVOLUTION                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  v1.0 - "Cloud PBX for SaaS Builders"                                   │
│  ════════════════════════════════════                                   │
│  Target: Developers building voice features into their apps             │
│  • Single-node or simple cluster                                        │
│  • Programmable voice (Twilio-like but SIP-native)                      │
│  • REST/WebSocket APIs, webhooks                                        │
│  • Basic IVR, queues, voicemail                                         │
│                                                                          │
│  v2.0 - "Enterprise SBC & Routing Brain"                                │
│  ═══════════════════════════════════════                                │
│  Target: Enterprises with complex routing, multiple carriers            │
│  • Full SBC with topology hiding, NAT traversal                         │
│  • LCR with real-time carrier health                                    │
│  • HA clustering with automatic failover                                │
│  • Advanced call center (skills-based routing, wallboards)              │
│                                                                          │
│  v3.0 - "Carrier-Grade Multi-Region"                                    │
│  ═══════════════════════════════════                                    │
│  Target: Service providers, carriers, large enterprises                 │
│  • Global distribution, geo-redundant                                   │
│  • STIR/SHAKEN, fraud detection                                         │
│  • Millions of subscribers                                              │
│  • Regulatory compliance (E911, lawful intercept)                       │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 2. High-Level Architecture

### 2.1 Control Plane vs Data Plane

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        FERROUS COMMUNICATIONS PLATFORM                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ╔═══════════════════════════════════════════════════════════════════════╗  │
│  ║                         CONTROL PLANE                                  ║  │
│  ║  (Locked down, internal network, scales independently)                 ║  │
│  ║                                                                        ║  │
│  ║  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐          ║  │
│  ║  │  Config    │ │  Routing   │ │  Policy    │ │   CDR      │          ║  │
│  ║  │   Store    │ │  Engine    │ │  Engine    │ │ Processor  │          ║  │
│  ║  └────────────┘ └────────────┘ └────────────┘ └────────────┘          ║  │
│  ║  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐          ║  │
│  ║  │  Cluster   │ │  Admin     │ │  Webhook   │ │  Billing   │          ║  │
│  ║  │ Membership │ │   API      │ │  Dispatcher│ │  Gateway   │          ║  │
│  ║  └────────────┘ └────────────┘ └────────────┘ └────────────┘          ║  │
│  ╚═══════════════════════════════════════════════════════════════════════╝  │
│                                    │                                         │
│                              Event Bus                                       │
│                      (Redis Streams / NATS / Kafka)                         │
│                                    │                                         │
│  ╔═══════════════════════════════════════════════════════════════════════╗  │
│  ║                          DATA PLANE                                    ║  │
│  ║  (Edge-facing, handles live traffic, latency-critical)                 ║  │
│  ║                                                                        ║  │
│  ║  ┌─────────────────────────────────────────────────────────────────┐  ║  │
│  ║  │                    SIGNALING WORKERS                             │  ║  │
│  ║  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐            │  ║  │
│  ║  │  │Registrar │ │  Proxy   │ │  B2BUA   │ │   SBC    │            │  ║  │
│  ║  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘            │  ║  │
│  ║  └─────────────────────────────────────────────────────────────────┘  ║  │
│  ║                                                                        ║  │
│  ║  ┌─────────────────────────────────────────────────────────────────┐  ║  │
│  ║  │                      MEDIA WORKERS                               │  ║  │
│  ║  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐            │  ║  │
│  ║  │  │RTP Engine│ │Transcoder│ │Conference│ │ WebRTC   │            │  ║  │
│  ║  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘            │  ║  │
│  ║  └─────────────────────────────────────────────────────────────────┘  ║  │
│  ║                                                                        ║  │
│  ║  ┌─────────────────────────────────────────────────────────────────┐  ║  │
│  ║  │                   APPLICATION WORKERS                            │  ║  │
│  ║  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐            │  ║  │
│  ║  │  │   IVR    │ │  Queue   │ │Voicemail │ │ Presence │            │  ║  │
│  ║  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘            │  ║  │
│  ║  └─────────────────────────────────────────────────────────────────┘  ║  │
│  ╚═══════════════════════════════════════════════════════════════════════╝  │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Event Bus Architecture

The event bus decouples components and enables reactive patterns:

```rust
/// Core event types that flow through the system
pub enum PlatformEvent {
    // Call lifecycle
    Call(CallEvent),
    // Media events
    Media(MediaEvent),
    // Queue events  
    Queue(QueueEvent),
    // Registration events
    Registration(RegistrationEvent),
    // Tenant events
    Tenant(TenantEvent),
    // System events
    System(SystemEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallEvent {
    Created { call_id: CallId, tenant_id: TenantId, from: String, to: String },
    Ringing { call_id: CallId, ring_time: DateTime<Utc> },
    Answered { call_id: CallId, answer_time: DateTime<Utc> },
    Bridged { call_id: CallId, bridge_id: BridgeId, legs: Vec<CallLegId> },
    Held { call_id: CallId },
    Resumed { call_id: CallId },
    Transferred { call_id: CallId, transfer_target: String },
    Ended { call_id: CallId, duration: Duration, hangup_cause: HangupCause },
    Failed { call_id: CallId, error: String, sip_code: Option<u16> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaEvent {
    SessionStarted { session_id: SessionId, call_id: CallId, codec: String },
    SessionEnded { session_id: SessionId, stats: MediaStats },
    QualityDegraded { session_id: SessionId, mos: f32, packet_loss: f32 },
    RecordingStarted { session_id: SessionId, recording_id: RecordingId },
    RecordingCompleted { recording_id: RecordingId, url: String, duration: Duration },
    DtmfReceived { session_id: SessionId, digit: char },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueueEvent {
    CallerEnqueued { queue_id: QueueId, call_id: CallId, position: usize },
    CallerDequeued { queue_id: QueueId, call_id: CallId, wait_time: Duration },
    CallerAbandoned { queue_id: QueueId, call_id: CallId, wait_time: Duration },
    AgentAssigned { queue_id: QueueId, call_id: CallId, agent_id: AgentId },
    QueueThresholdExceeded { queue_id: QueueId, metric: String, value: f64 },
}
```

**Event Bus Benefits:**
- Applications react to events without tight coupling
- External systems subscribe via gRPC/WebSocket
- Easy bolt-on of AI/transcription, fraud detection, analytics
- Replay capability for debugging and testing

---

## 3. Module Structure

### 3.1 Repository Layout

```
fcp/
├── Cargo.toml                    # Workspace root
│
├── crates/
│   │
│   │── fcp-core/                 # Shared types, traits, utilities
│   │
│   │── siphon/                   # SIP stack (standalone, reusable)
│   │   ├── siphon-core/          # Parser, types, codecs
│   │   ├── siphon-transport/     # UDP/TCP/TLS/WS transports
│   │   ├── siphon-transaction/   # Transaction state machines
│   │   └── siphon-dialog/        # Dialog state management
│   │
│   ├── fcp-control-plane/        # Control plane services
│   │   ├── config-store/         # Distributed configuration
│   │   ├── routing-engine/       # LCR, number translation
│   │   ├── policy-engine/        # Rate limits, fraud, geofencing
│   │   ├── cdr-processor/        # CDR collection & export
│   │   ├── webhook-dispatcher/   # Outbound webhook delivery
│   │   └── admin-api/            # REST/gRPC admin interface
│   │
│   ├── fcp-data-plane/           # Data plane services
│   │   ├── signaling/            # SIP services
│   │   │   ├── registrar/
│   │   │   ├── proxy/
│   │   │   ├── b2bua/
│   │   │   └── sbc/
│   │   ├── media/                # Media services
│   │   │   ├── rtp-engine/
│   │   │   ├── transcoder/
│   │   │   ├── conference/
│   │   │   ├── webrtc/
│   │   │   └── recorder/
│   │   └── applications/         # Application services
│   │       ├── flow-engine/      # Call flow DSL runtime
│   │       ├── ivr/
│   │       ├── queue/
│   │       ├── voicemail/
│   │       └── presence/
│   │
│   ├── fcp-cluster/              # Clustering infrastructure
│   │   ├── membership/           # SWIM protocol
│   │   ├── state/                # Distributed state (reg, dialog)
│   │   └── sharding/             # State partitioning
│   │
│   ├── fcp-event-bus/            # Event infrastructure
│   │   ├── core/                 # Event types, traits
│   │   ├── redis-backend/        # Redis Streams implementation
│   │   └── nats-backend/         # NATS implementation (optional)
│   │
│   ├── fcp-storage/              # Storage abstractions
│   │   ├── core/                 # Traits, migrations
│   │   ├── sqlite/
│   │   ├── postgres/
│   │   └── cockroach/
│   │
│   ├── fcp-api/                  # External APIs
│   │   ├── rest/
│   │   ├── grpc/
│   │   └── websocket/
│   │
│   ├── fcp-plugins/              # Plugin system
│   │   ├── runtime/              # WASM + Lua runtimes
│   │   └── sdk/                  # Plugin development SDK
│   │
│   └── fcp-runtime/              # Deployment runtimes
│       ├── standalone/           # All-in-one binary
│       ├── distributed/          # Multi-process
│       └── cli/                  # Command-line tool
│
├── proto/                        # gRPC/protobuf definitions
├── migrations/                   # Database migrations
├── deploy/                       # Deployment configs
│   ├── docker/
│   ├── kubernetes/
│   └── terraform/
└── docs/                         # Documentation
```

### 3.2 Dependency Graph

```
                         ┌─────────────┐
                         │  fcp-core   │
                         └──────┬──────┘
                                │
            ┌───────────────────┼───────────────────┐
            │                   │                   │
            ▼                   ▼                   ▼
      ┌───────────┐      ┌───────────┐      ┌───────────┐
      │  siphon   │      │ fcp-event │      │fcp-storage│
      │  (stack)  │      │   -bus    │      │           │
      └─────┬─────┘      └─────┬─────┘      └─────┬─────┘
            │                  │                  │
            ▼                  ▼                  ▼
      ┌─────────────────────────────────────────────────┐
      │              fcp-cluster                         │
      └───────────────────────┬─────────────────────────┘
                              │
         ┌────────────────────┼────────────────────┐
         │                    │                    │
         ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ fcp-control-    │  │  fcp-data-      │  │   fcp-plugins   │
│     plane       │  │     plane       │  │                 │
└────────┬────────┘  └────────┬────────┘  └────────┬────────┘
         │                    │                    │
         └────────────────────┼────────────────────┘
                              │
                              ▼
                       ┌───────────┐
                       │  fcp-api  │
                       └─────┬─────┘
                             │
                             ▼
                      ┌───────────┐
                      │fcp-runtime│
                      └───────────┘
```

---

## 4. Signaling Layer (Siphon + Services)

### 4.1 Siphon Stack Architecture

Transaction and Dialog layers are **explicit, reusable components**:

```rust
/// Transaction Manager - shared by all signaling services
pub struct TransactionManager {
    /// Active client transactions
    client_transactions: DashMap<TransactionKey, Arc<ClientTransaction>>,
    /// Active server transactions
    server_transactions: DashMap<TransactionKey, Arc<ServerTransaction>>,
    /// Transport layer
    transport: Arc<TransportManager>,
    /// Timer wheel for transaction timeouts
    timers: TimerWheel,
}

impl TransactionManager {
    /// Send a request and get a transaction handle
    pub async fn send_request(&self, request: Request) -> Result<TransactionHandle, Error> {
        let key = TransactionKey::from_request(&request);
        let (txn, rx) = ClientTransaction::new(request.clone(), self.transport.clone());
        
        self.client_transactions.insert(key.clone(), Arc::new(txn));
        txn.start().await?;
        
        Ok(TransactionHandle { key, responses: rx })
    }

    /// Process incoming message (request or response)
    pub async fn process(&self, msg: SipMessage, source: SocketAddr) -> Result<(), Error> {
        match msg {
            SipMessage::Request(req) => self.process_request(req, source).await,
            SipMessage::Response(resp) => self.process_response(resp).await,
        }
    }
}

/// Dialog Manager - shared by B2BUA, presence, etc.
pub struct DialogManager {
    /// Active dialogs by DialogId
    dialogs: DashMap<DialogId, Arc<RwLock<Dialog>>>,
    /// Index by Call-ID for fast lookup
    by_call_id: DashMap<String, Vec<DialogId>>,
    /// Distributed store for cluster-wide dialog state
    store: Arc<dyn DialogStore>,
}

impl DialogManager {
    /// Create dialog from UAC INVITE
    pub fn create_uac_dialog(&self, request: &Request) -> Result<DialogId, Error> {
        let dialog = Dialog::from_uac_invite(request);
        let id = dialog.id.clone();
        
        self.dialogs.insert(id.clone(), Arc::new(RwLock::new(dialog)));
        self.by_call_id
            .entry(request.call_id.to_string())
            .or_default()
            .push(id.clone());
        
        Ok(id)
    }

    /// Update dialog from response
    pub async fn update_from_response(
        &self, 
        call_id: &str, 
        response: &Response
    ) -> Result<(), Error> {
        // Find matching early dialog and update
        // ...
    }

    /// Create in-dialog request (BYE, re-INVITE, etc.)
    pub fn create_request(&self, dialog_id: &DialogId, method: Method) -> Result<Request, Error> {
        let dialog = self.dialogs.get(dialog_id)
            .ok_or(Error::DialogNotFound)?;
        
        let mut dialog = dialog.write();
        Ok(dialog.create_request(method))
    }
}
```

### 4.2 Services Built on Shared Layers

```rust
/// Proxy service - uses TransactionManager, no dialog state
pub struct ProxyService {
    transaction_mgr: Arc<TransactionManager>,
    routing: Arc<dyn RoutingService>,
    config: ProxyConfig,
}

/// B2BUA service - uses both Transaction and Dialog managers
pub struct B2BUAService {
    transaction_mgr: Arc<TransactionManager>,
    dialog_mgr: Arc<DialogManager>,
    media_controller: Arc<dyn MediaController>,
    config: B2BUAConfig,
}

/// SBC service - B2BUA + topology hiding + security
pub struct SBCService {
    b2bua: B2BUAService,
    topology_hider: TopologyHider,
    rate_limiter: Arc<RateLimiter>,
    interfaces: HashMap<String, InterfaceConfig>,
}
```

### 4.3 Routing Engine with Carrier Intelligence

```rust
pub struct RoutingEngine {
    /// Static routing rules
    rules: Arc<RwLock<Vec<RoutingRule>>>,
    /// Real-time carrier statistics
    carrier_stats: Arc<DashMap<GatewayId, CarrierStats>>,
    /// Policy engine for pre/post-route hooks
    policy: Arc<PolicyEngine>,
}

/// Real-time carrier health metrics
#[derive(Debug, Clone)]
pub struct CarrierStats {
    /// Answer-Seizure Ratio (successful calls / attempts)
    pub asr: f32,
    /// Average Call Duration
    pub acd: Duration,
    /// Recent failure rate
    pub failure_rate: f32,
    /// Last failure timestamp
    pub last_failure: Option<DateTime<Utc>>,
    /// Current concurrent calls
    pub active_calls: u32,
    /// Max concurrent calls
    pub max_calls: u32,
    /// Average Post-Dial Delay
    pub pdd: Duration,
    /// Health status
    pub health: CarrierHealth,
}

#[derive(Debug, Clone, Copy)]
pub enum CarrierHealth {
    Healthy,
    Degraded,
    Down,
    Unknown,
}

impl RoutingEngine {
    /// Find best routes with LCR + health awareness
    pub async fn find_routes(
        &self,
        request: &Request,
        context: &RoutingContext,
    ) -> Result<Vec<Route>, RoutingError> {
        // 1. Pre-route policy check
        self.policy.pre_route(request, context).await?;

        // 2. Find matching rules
        let mut candidates = self.find_matching_rules(request, context).await?;

        // 3. Score routes based on cost + health
        for route in &mut candidates {
            if let Some(stats) = self.carrier_stats.get(&route.gateway_id) {
                route.score = self.calculate_score(route, &stats);
            }
        }

        // 4. Sort by score (lower is better)
        candidates.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

        // 5. Post-route policy (logging, tagging)
        self.policy.post_route(&candidates, context).await;

        Ok(candidates)
    }

    fn calculate_score(&self, route: &Route, stats: &CarrierStats) -> f64 {
        let cost_factor = route.cost_per_min.unwrap_or(0.0) as f64;
        let asr_penalty = (1.0 - stats.asr as f64) * 10.0;  // Penalize low ASR
        let failure_penalty = stats.failure_rate as f64 * 20.0;
        
        cost_factor + asr_penalty + failure_penalty
    }
}

/// Policy engine for routing decisions
pub struct PolicyEngine {
    /// Pre-route checks
    pre_route_policies: Vec<Arc<dyn PreRoutePolicy>>,
    /// Post-route hooks
    post_route_policies: Vec<Arc<dyn PostRoutePolicy>>,
}

#[async_trait]
pub trait PreRoutePolicy: Send + Sync {
    /// Check if routing should proceed
    /// Returns Err to reject the call
    async fn check(&self, request: &Request, context: &RoutingContext) -> Result<(), PolicyError>;
}

/// Example policies
pub struct FraudDetectionPolicy { /* ... */ }
pub struct GeofencingPolicy { /* ... */ }
pub struct TenantLimitsPolicy { /* ... */ }
pub struct TimeOfDayPolicy { /* ... */ }
```

### 4.4 STIR/SHAKEN Support (v3.0)

```rust
/// STIR/SHAKEN identity handling
pub struct StirShakenService {
    /// Certificate store
    certs: Arc<CertificateStore>,
    /// Signing key (for originating calls)
    signing_key: Option<SigningKey>,
    /// Verification policy
    policy: VerificationPolicy,
}

impl StirShakenService {
    /// Sign outgoing call (add Identity header)
    pub async fn sign(&self, request: &mut Request) -> Result<(), Error> {
        let passport = Passport {
            attest: self.determine_attestation(request),
            dest: request.to.uri.to_string(),
            iat: Utc::now().timestamp(),
            orig: request.from.uri.to_string(),
            origid: Uuid::new_v4().to_string(),
        };
        
        let identity = self.create_identity_header(&passport)?;
        request.headers.set("Identity", identity);
        Ok(())
    }

    /// Verify incoming call Identity header
    pub async fn verify(&self, request: &Request) -> VerificationResult {
        let identity = match request.headers.get("Identity") {
            Some(i) => i,
            None => return VerificationResult::NoIdentity,
        };
        
        // Parse and verify passport
        // ...
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Attestation {
    /// Full attestation - carrier authenticated caller
    A,
    /// Partial attestation - carrier knows customer but not specific caller
    B,
    /// Gateway attestation - call received from another carrier
    C,
}

#[derive(Debug)]
pub enum VerificationResult {
    /// Identity verified successfully
    Valid { attestation: Attestation },
    /// Identity verification failed
    Invalid { reason: String },
    /// No Identity header present
    NoIdentity,
    /// Verification not possible (missing certs, etc.)
    Unknown,
}
```

---

## 5. Media Layer

### 5.1 RTP Engine with Admission Control

```rust
pub struct RtpEngineConfig {
    /// Port range for RTP
    pub port_range: Range<u16>,
    /// External IP for NAT traversal
    pub external_ip: Option<IpAddr>,
    /// Jitter buffer configuration
    pub jitter_buffer: JitterBufferConfig,
    /// DTMF handling mode
    pub dtmf_mode: DtmfMode,
    /// QoS configuration
    pub qos: QosConfig,
    /// Admission control
    pub admission: AdmissionConfig,
}

#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    /// Maximum RTP sessions for this node
    pub max_sessions: usize,
    /// Per-tenant session limits
    pub tenant_limits: HashMap<TenantId, usize>,
    /// CPU threshold to start rejecting (0.0-1.0)
    pub cpu_threshold: f32,
    /// Action when limit exceeded
    pub overflow_action: OverflowAction,
}

#[derive(Debug, Clone, Copy)]
pub enum OverflowAction {
    /// Reject the session
    Reject,
    /// Redirect to another node
    Redirect,
    /// Allow but mark as degraded
    Degrade,
}

#[derive(Debug, Clone)]
pub struct QosConfig {
    /// DSCP value for audio packets
    pub audio_dscp: u8,  // Typically 46 (EF)
    /// DSCP value for video packets
    pub video_dscp: u8,  // Typically 34 (AF41)
    /// DSCP value for signaling
    pub signaling_dscp: u8,  // Typically 24 (CS3)
}

impl RtpEngine {
    /// Allocate a new media session with admission control
    pub async fn allocate(&self, params: MediaParams) -> Result<MediaSession, RtpError> {
        // Check global capacity
        let current = self.active_sessions.load(Ordering::SeqCst);
        if current >= self.config.admission.max_sessions {
            return self.handle_overflow(params).await;
        }

        // Check tenant limit
        if let Some(tenant_id) = &params.tenant_id {
            let tenant_current = self.tenant_sessions.get(tenant_id)
                .map(|v| *v)
                .unwrap_or(0);
            
            if let Some(&limit) = self.config.admission.tenant_limits.get(tenant_id) {
                if tenant_current >= limit {
                    return Err(RtpError::TenantLimitExceeded);
                }
            }
        }

        // Check CPU
        let cpu = self.metrics.cpu_usage();
        if cpu > self.config.admission.cpu_threshold {
            tracing::warn!(cpu = %cpu, "CPU threshold exceeded, degrading service");
            // Could redirect or degrade here
        }

        // Allocate ports and create session
        let (rtp_port, rtcp_port) = self.port_allocator.allocate()?;
        
        let session = MediaSession {
            id: SessionId::new(),
            rtp_port,
            rtcp_port,
            // ...
        };

        self.active_sessions.fetch_add(1, Ordering::SeqCst);
        Ok(session)
    }
}
```

### 5.2 Media Resilience Features

```rust
/// Packet loss concealment
pub struct PlcConfig {
    /// Enable PLC
    pub enabled: bool,
    /// Algorithm to use
    pub algorithm: PlcAlgorithm,
    /// Max consecutive packets to conceal
    pub max_concealment: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum PlcAlgorithm {
    /// Simple packet repetition
    Repeat,
    /// Interpolation-based
    Interpolate,
    /// Codec-specific (Opus PLC)
    CodecNative,
}

/// Voice Activity Detection
pub struct VadConfig {
    pub enabled: bool,
    pub threshold_db: f32,
    pub hangover_ms: u32,
}

/// Comfort Noise Generation
pub struct CngConfig {
    pub enabled: bool,
    pub level_db: f32,
}

/// Forward Error Correction (for WebRTC/Opus)
pub struct FecConfig {
    pub enabled: bool,
    /// Percentage of bandwidth to use for FEC
    pub overhead_percent: u8,
}

/// Complete media processing pipeline config
pub struct MediaPipelineConfig {
    pub plc: PlcConfig,
    pub vad: VadConfig,
    pub cng: CngConfig,
    pub fec: Option<FecConfig>,
    /// Jitter buffer config
    pub jitter: JitterBufferConfig,
    /// Automatic gain control
    pub agc: Option<AgcConfig>,
}
```

### 5.3 Hardware Acceleration Strategy

```rust
/// Hardware acceleration configuration
pub struct HwAccelConfig {
    /// Acceleration mode
    pub mode: HwAccelMode,
    /// Specific device (for multi-GPU)
    pub device_id: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub enum HwAccelMode {
    /// CPU only (portable, always available)
    /// Uses SIMD (AVX2/AVX-512/NEON) when available
    CpuOnly,
    
    /// NVIDIA GPU via CUDA/NVENC
    /// Best for heavy video transcoding/mixing
    Cuda,
    
    /// Intel Quick Sync
    /// Good for Intel-based servers
    QuickSync,
    
    /// AMD AMF
    /// For AMD GPUs
    Amf,
    
    /// VA-API (Linux generic)
    VaApi,
    
    /// Auto-detect best available
    Auto,
}

impl TranscoderService {
    pub fn new(config: TranscoderConfig) -> Result<Self, Error> {
        let backend = match config.hw_accel.mode {
            HwAccelMode::CpuOnly => {
                tracing::info!("Using CPU transcoding with SIMD optimizations");
                TranscodeBackend::Cpu(CpuBackend::new()?)
            }
            HwAccelMode::Cuda => {
                tracing::info!("Using NVIDIA CUDA acceleration");
                TranscodeBackend::Cuda(CudaBackend::new(config.hw_accel.device_id)?)
            }
            HwAccelMode::Auto => {
                // Try CUDA -> QuickSync -> VA-API -> CPU
                Self::detect_best_backend()?
            }
            // ...
        };
        
        Ok(Self { backend, config })
    }
}
```

---

## 6. Application Layer

### 6.1 Call Flow Engine (DSL)

A unified flow engine that powers IVR, queues, voicemail, and custom applications:

```rust
/// A call flow is a graph of nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallFlow {
    pub id: FlowId,
    pub name: String,
    pub tenant_id: TenantId,
    pub version: u32,
    pub entry_node: NodeId,
    pub nodes: HashMap<NodeId, FlowNode>,
}

/// Individual node in the flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: NodeId,
    pub node_type: NodeType,
    /// Edges to other nodes based on outcome
    pub transitions: HashMap<String, NodeId>,
}

/// Node types in the flow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NodeType {
    /// Play audio prompt
    Play {
        prompt: Prompt,
        #[serde(default)]
        barge_in: bool,
    },
    
    /// Say text via TTS
    Say {
        text: String,
        voice: Option<String>,
        language: Option<String>,
    },
    
    /// Gather DTMF input
    Gather {
        num_digits: Option<usize>,
        finish_on_key: Option<char>,
        timeout: Duration,
        prompt: Option<Prompt>,
    },
    
    /// Speech recognition
    Recognize {
        grammar: Grammar,
        timeout: Duration,
        prompt: Option<Prompt>,
    },
    
    /// Branch based on condition
    Branch {
        condition: Condition,
    },
    
    /// Set variable
    SetVariable {
        name: String,
        value: Expression,
    },
    
    /// Make HTTP request
    HttpRequest {
        url: String,
        method: HttpMethod,
        body: Option<String>,
        headers: HashMap<String, String>,
        result_var: String,
    },
    
    /// Dial another party
    Dial {
        destination: Expression,
        timeout: Duration,
        caller_id: Option<String>,
        #[serde(default)]
        record: bool,
    },
    
    /// Bridge to another call leg
    Bridge {
        target_leg: Expression,
    },
    
    /// Add to queue
    Enqueue {
        queue_id: QueueId,
        priority: Option<i32>,
    },
    
    /// Record audio
    Record {
        max_duration: Duration,
        beep: bool,
        finish_on_key: Option<char>,
        result_var: String,
    },
    
    /// Send to voicemail
    Voicemail {
        mailbox: Expression,
    },
    
    /// Execute custom plugin (Lua/WASM)
    Plugin {
        plugin_id: String,
        params: HashMap<String, serde_json::Value>,
    },
    
    /// Transfer call
    Transfer {
        destination: Expression,
        transfer_type: TransferType,
    },
    
    /// Hangup
    Hangup {
        cause: Option<HangupCause>,
    },
    
    /// Goto another flow
    Subflow {
        flow_id: FlowId,
        return_node: Option<NodeId>,
    },
}

/// Flow engine runtime
pub struct FlowEngine {
    /// Loaded flows
    flows: Arc<RwLock<HashMap<FlowId, CallFlow>>>,
    /// Plugin runtime
    plugins: Arc<PluginRuntime>,
    /// Media controller
    media: Arc<dyn MediaController>,
    /// TTS service
    tts: Option<Arc<dyn TtsService>>,
    /// ASR service
    asr: Option<Arc<dyn AsrService>>,
}

impl FlowEngine {
    /// Execute a flow for a call
    pub async fn execute(
        &self,
        flow_id: &FlowId,
        call: &mut CallContext,
    ) -> Result<FlowResult, FlowError> {
        let flow = self.flows.read().get(flow_id)
            .ok_or(FlowError::FlowNotFound)?
            .clone();
        
        let mut current_node = flow.entry_node.clone();
        let mut context = ExecutionContext::new(call);
        
        loop {
            let node = flow.nodes.get(&current_node)
                .ok_or(FlowError::NodeNotFound)?;
            
            // Execute the node
            let outcome = self.execute_node(node, &mut context).await?;
            
            // Find next node based on outcome
            match node.transitions.get(&outcome) {
                Some(next) => current_node = next.clone(),
                None => {
                    // No transition - flow complete
                    return Ok(FlowResult::Completed(outcome));
                }
            }
        }
    }
    
    async fn execute_node(
        &self,
        node: &FlowNode,
        ctx: &mut ExecutionContext,
    ) -> Result<String, FlowError> {
        match &node.node_type {
            NodeType::Play { prompt, barge_in } => {
                self.media.play(ctx.session_id, prompt, *barge_in).await?;
                Ok("completed".to_string())
            }
            NodeType::Gather { num_digits, timeout, .. } => {
                let digits = self.media
                    .collect_dtmf(ctx.session_id, *num_digits, *timeout)
                    .await?;
                ctx.set_variable("digits", &digits);
                Ok("collected".to_string())
            }
            NodeType::Branch { condition } => {
                let result = condition.evaluate(ctx)?;
                Ok(if result { "true" } else { "false" }.to_string())
            }
            NodeType::Plugin { plugin_id, params } => {
                self.plugins.execute(plugin_id, params, ctx).await
            }
            // ... handle other node types
            _ => todo!()
        }
    }
}
```

### 6.2 JSON/YAML Flow Definition

```yaml
# Example: Customer support IVR
id: "support-ivr"
name: "Customer Support IVR"
entry_node: "welcome"

nodes:
  welcome:
    type: Play
    prompt: { type: "file", path: "prompts/welcome.wav" }
    transitions:
      completed: "main_menu"

  main_menu:
    type: Gather
    prompt: { type: "tts", text: "Press 1 for sales, 2 for support, 3 for billing" }
    num_digits: 1
    timeout: "5s"
    transitions:
      collected: "route_choice"
      timeout: "main_menu"

  route_choice:
    type: Branch
    condition:
      type: "equals"
      left: { type: "variable", name: "digits" }
      right: "1"
    transitions:
      "true": "sales_queue"
      "false": "check_support"

  check_support:
    type: Branch
    condition:
      type: "equals"
      left: { type: "variable", name: "digits" }
      right: "2"
    transitions:
      "true": "support_queue"
      "false": "check_billing"

  sales_queue:
    type: Enqueue
    queue_id: "sales"
    priority: 5
    transitions:
      answered: "hangup"
      timeout: "voicemail"

  support_queue:
    type: Enqueue
    queue_id: "support"
    transitions:
      answered: "hangup"
      timeout: "voicemail"

  voicemail:
    type: Voicemail
    mailbox: "general"
    transitions:
      completed: "hangup"

  hangup:
    type: Hangup
```

### 6.3 Plugin System (WASM + Lua)

```rust
/// Plugin runtime supporting multiple languages
pub struct PluginRuntime {
    /// WASM runtime (wasmtime)
    wasm: WasmRuntime,
    /// Lua runtime (mlua)
    lua: LuaRuntime,
    /// Loaded plugins
    plugins: DashMap<String, LoadedPlugin>,
}

pub enum LoadedPlugin {
    Wasm(WasmPlugin),
    Lua(LuaPlugin),
}

/// Plugin manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub runtime: PluginRuntime,
    pub entry_point: String,
    /// Resource limits
    pub limits: PluginLimits,
    /// Required permissions
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginLimits {
    /// Max memory (bytes)
    pub max_memory: usize,
    /// Max execution time per call
    pub max_execution_time: Duration,
    /// Max HTTP requests per execution
    pub max_http_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Permission {
    /// Can make HTTP requests
    Http { allowed_domains: Vec<String> },
    /// Can access call variables
    Variables { read: bool, write: bool },
    /// Can control media
    Media,
    /// Can access storage
    Storage { namespace: String },
}

/// Host functions exposed to plugins
#[derive(Debug)]
pub struct PluginHostFunctions {
    // Call control
    fn answer(&self) -> Result<(), Error>;
    fn hangup(&self, cause: Option<&str>) -> Result<(), Error>;
    fn transfer(&self, destination: &str) -> Result<(), Error>;
    
    // Media
    fn play(&self, prompt: &str) -> Result<(), Error>;
    fn say(&self, text: &str, voice: Option<&str>) -> Result<(), Error>;
    fn gather(&self, options: GatherOptions) -> Result<String, Error>;
    fn record(&self, options: RecordOptions) -> Result<String, Error>;
    
    // Variables
    fn get_variable(&self, name: &str) -> Result<Option<String>, Error>;
    fn set_variable(&self, name: &str, value: &str) -> Result<(), Error>;
    
    // HTTP
    fn http_request(&self, req: HttpRequest) -> Result<HttpResponse, Error>;
    
    // Logging
    fn log(&self, level: &str, message: &str);
}
```

### 6.4 AI/ML Integration Hooks

```rust
/// Transcription service interface
#[async_trait]
pub trait TranscriptionService: Send + Sync {
    /// Start real-time transcription for a call
    async fn start_realtime(
        &self,
        session_id: SessionId,
        config: TranscriptionConfig,
    ) -> Result<TranscriptionStream, Error>;
    
    /// Transcribe a recording
    async fn transcribe_recording(
        &self,
        recording_url: &str,
        config: TranscriptionConfig,
    ) -> Result<Transcript, Error>;
}

/// LLM service interface for conversational AI
#[async_trait]
pub trait ConversationalAI: Send + Sync {
    /// Process user input and get response
    async fn process(
        &self,
        input: &str,
        context: &ConversationContext,
    ) -> Result<AIResponse, Error>;
}

#[derive(Debug)]
pub struct AIResponse {
    pub text: String,
    pub action: Option<AIAction>,
    pub confidence: f32,
}

#[derive(Debug)]
pub enum AIAction {
    Transfer { destination: String },
    Escalate { reason: String },
    EndConversation,
    CollectInfo { field: String },
}

/// Event hooks for AI services
impl FlowEngine {
    /// Register handler for recording completion
    pub fn on_recording_completed<F>(&self, handler: F)
    where
        F: Fn(RecordingCompletedEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static
    {
        // Allows external AI services to react to recordings
    }
}
```

---

## 7. Cluster & State Management

### 7.1 Pragmatic Consensus Approach

Given scope, start with **external systems** for consensus, build in-house later:

```rust
/// Configuration store - delegate to external system initially
pub enum ConfigStoreBackend {
    /// Use etcd for distributed config
    Etcd(EtcdConfig),
    /// Use Consul KV
    Consul(ConsulConfig),
    /// Built-in Raft (future)
    Raft(RaftConfig),
}

/// Initially, focus fcp-cluster on:
/// 1. Membership (SWIM) - build this
/// 2. State caching - build this
/// 3. Consensus - delegate to etcd/Consul
pub struct ClusterService {
    /// SWIM membership (custom implementation)
    membership: SwimMembership,
    /// Distributed state cache (custom)
    state_cache: DistributedCache,
    /// Config store (delegated)
    config: Arc<dyn ConfigStore>,
}
```

### 7.2 State Sharding

```rust
/// Sharding strategy for distributed state
pub trait ShardingStrategy: Send + Sync {
    /// Determine which node owns a key
    fn owner(&self, key: &str) -> NodeId;
    
    /// Get all nodes that should have a replica
    fn replicas(&self, key: &str) -> Vec<NodeId>;
}

/// Hash-based sharding
pub struct ConsistentHashSharding {
    ring: ConsistentHashRing,
    replication_factor: usize,
}

impl ShardingStrategy for ConsistentHashSharding {
    fn owner(&self, key: &str) -> NodeId {
        self.ring.get_node(key)
    }
    
    fn replicas(&self, key: &str) -> Vec<NodeId> {
        self.ring.get_nodes(key, self.replication_factor)
    }
}

/// Tenant-aware sharding (keeps tenant data together)
pub struct TenantAwareSharding {
    /// Map tenant to primary node
    tenant_assignments: DashMap<TenantId, NodeId>,
    /// Fallback for unassigned tenants
    fallback: ConsistentHashSharding,
}

impl ShardingStrategy for TenantAwareSharding {
    fn owner(&self, key: &str) -> NodeId {
        // Extract tenant from key (format: "tenant:xxx:...")
        if let Some(tenant_id) = extract_tenant_id(key) {
            if let Some(node) = self.tenant_assignments.get(&tenant_id) {
                return *node;
            }
        }
        self.fallback.owner(key)
    }
}

/// Distributed registration store with sharding
pub struct ShardedRegistrationStore {
    local_cache: DashMap<Aor, Vec<Registration>>,
    sharding: Arc<dyn ShardingStrategy>,
    cluster: Arc<ClusterService>,
}

impl ShardedRegistrationStore {
    pub async fn lookup(&self, aor: &Aor) -> Result<Vec<Registration>, Error> {
        let key = aor.to_string();
        let owner = self.sharding.owner(&key);
        
        if owner == self.cluster.local_node_id() {
            // We own this key - check local cache
            Ok(self.local_cache.get(&aor)
                .map(|v| v.clone())
                .unwrap_or_default())
        } else {
            // Forward to owner
            self.cluster.forward_lookup(owner, aor).await
        }
    }
}
```

---

## 8. CDR & Billing Pipeline

### 8.1 Dedicated CDR Processing

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         CDR PIPELINE                                     │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Data Plane Nodes                                                        │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                                 │
│  │ B2BUA 1  │ │ B2BUA 2  │ │ B2BUA N  │                                 │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘                                 │
│       │            │            │                                        │
│       └────────────┼────────────┘                                        │
│                    │                                                     │
│                    ▼                                                     │
│           ┌────────────────┐                                            │
│           │  Event Bus     │  (CallEnded events)                        │
│           │ (Redis/Kafka)  │                                            │
│           └───────┬────────┘                                            │
│                   │                                                      │
│                   ▼                                                      │
│  Control Plane                                                           │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                      CDR Processor                                  │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │ │
│  │  │  Collector   │─►│  Normalizer  │─►│   Enricher   │              │ │
│  │  │              │  │              │  │              │              │ │
│  │  │ - Receive    │  │ - Validate   │  │ - Add tenant │              │ │
│  │  │ - Dedupe     │  │ - Transform  │  │ - Add cost   │              │ │
│  │  │ - Buffer     │  │ - Standardize│  │ - Add geo    │              │ │
│  │  └──────────────┘  └──────────────┘  └──────┬───────┘              │ │
│  │                                             │                       │ │
│  │                    ┌────────────────────────┼───────────────────┐  │ │
│  │                    │                        │                   │  │ │
│  │                    ▼                        ▼                   ▼  │ │
│  │           ┌──────────────┐         ┌──────────────┐    ┌──────────┐│ │
│  │           │   Database   │         │  Data Lake   │    │ Webhooks ││ │
│  │           │  (Postgres)  │         │ (S3/BigQuery)│    │          ││ │
│  │           └──────────────┘         └──────────────┘    └──────────┘│ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 8.2 CDR Schema

```rust
/// Comprehensive Call Detail Record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDetailRecord {
    // === Identifiers ===
    pub cdr_id: Uuid,
    pub call_id: CallId,
    pub tenant_id: TenantId,
    pub correlation_id: Option<String>,  // For linked calls (transfers, etc.)
    
    // === Parties ===
    pub from_uri: String,
    pub from_user: Option<String>,
    pub from_display: Option<String>,
    pub to_uri: String,
    pub to_user: Option<String>,
    pub final_destination: Option<String>,  // After routing
    
    // === Timestamps ===
    pub start_time: DateTime<Utc>,
    pub ring_time: Option<DateTime<Utc>>,
    pub answer_time: Option<DateTime<Utc>>,
    pub end_time: DateTime<Utc>,
    pub duration_ms: u64,
    pub ring_duration_ms: Option<u64>,
    pub talk_duration_ms: Option<u64>,
    
    // === Disposition ===
    pub disposition: CallDisposition,
    pub hangup_cause: HangupCause,
    pub hangup_source: HangupSource,
    pub sip_response_code: Option<u16>,
    
    // === Routing ===
    pub source_ip: IpAddr,
    pub source_port: u16,
    pub dest_ip: Option<IpAddr>,
    pub dest_port: Option<u16>,
    pub gateway_id: Option<GatewayId>,
    pub gateway_name: Option<String>,
    pub route_attempts: Vec<RouteAttempt>,
    
    // === Media Quality ===
    pub codec: Option<String>,
    pub media_stats: Option<MediaQualityStats>,
    
    // === Billing ===
    pub billing_rate: Option<Decimal>,
    pub billing_increment: Option<u32>,  // seconds
    pub total_cost: Option<Decimal>,
    pub billing_currency: Option<String>,
    
    // === Features Used ===
    pub features: Vec<String>,  // ["recording", "transfer", "conference"]
    pub recording_url: Option<String>,
    pub voicemail_url: Option<String>,
    
    // === Identity/Security ===
    pub stir_shaken: Option<StirShakenInfo>,
    pub spam_score: Option<f32>,
    
    // === Custom ===
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaQualityStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub packet_loss_percent: f32,
    pub jitter_ms: f32,
    pub rtt_ms: Option<f32>,
    pub mos: Option<f32>,
    pub r_factor: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteAttempt {
    pub gateway_id: GatewayId,
    pub attempt_time: DateTime<Utc>,
    pub result: RouteAttemptResult,
    pub sip_code: Option<u16>,
    pub duration_ms: u64,
}
```

---

## 9. API & Webhooks

### 9.1 Webhook System

```rust
/// Webhook configuration per tenant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: WebhookId,
    pub tenant_id: TenantId,
    pub url: String,
    pub events: Vec<WebhookEventType>,
    pub secret: String,  // For HMAC signing
    pub enabled: bool,
    pub retry_config: RetryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WebhookEventType {
    // Call events
    CallCreated,
    CallRinging,
    CallAnswered,
    CallEnded,
    CallFailed,
    
    // Queue events
    QueueJoined,
    QueueAnswered,
    QueueAbandoned,
    
    // Voicemail events
    VoicemailNew,
    VoicemailListened,
    
    // Recording events
    RecordingCompleted,
    
    // Registration events
    RegistrationCreated,
    RegistrationExpired,
}

/// Webhook payload
#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub id: String,
    pub event: String,
    pub timestamp: DateTime<Utc>,
    pub tenant_id: TenantId,
    pub data: serde_json::Value,
}

/// Webhook dispatcher service
pub struct WebhookDispatcher {
    /// Configured webhooks by tenant
    webhooks: Arc<RwLock<HashMap<TenantId, Vec<WebhookConfig>>>>,
    /// HTTP client
    client: reqwest::Client,
    /// Retry queue
    retry_queue: Arc<RetryQueue>,
}

impl WebhookDispatcher {
    /// Dispatch event to all matching webhooks
    pub async fn dispatch(&self, event: PlatformEvent) {
        let tenant_id = event.tenant_id();
        let event_type = event.event_type();
        
        let webhooks = self.webhooks.read();
        let tenant_hooks = match webhooks.get(&tenant_id) {
            Some(h) => h,
            None => return,
        };
        
        for hook in tenant_hooks {
            if !hook.enabled || !hook.events.contains(&event_type) {
                continue;
            }
            
            let payload = WebhookPayload {
                id: Uuid::new_v4().to_string(),
                event: event_type.to_string(),
                timestamp: Utc::now(),
                tenant_id: tenant_id.clone(),
                data: event.to_json(),
            };
            
            self.send_webhook(hook, payload).await;
        }
    }
    
    async fn send_webhook(&self, config: &WebhookConfig, payload: WebhookPayload) {
        let body = serde_json::to_string(&payload).unwrap();
        let signature = self.sign(&body, &config.secret);
        
        let result = self.client
            .post(&config.url)
            .header("Content-Type", "application/json")
            .header("X-FCP-Signature", signature)
            .header("X-FCP-Event", &payload.event)
            .body(body)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        
        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(webhook_id = %config.id, "Webhook delivered");
            }
            _ => {
                // Queue for retry
                self.retry_queue.enqueue(config.clone(), payload).await;
            }
        }
    }
    
    fn sign(&self, body: &str, secret: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}
```

### 9.2 Admin UI Features (Roadmap)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         FCP ADMIN DASHBOARD                              │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    REAL-TIME OVERVIEW                            │    │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌─────────┐ │    │
│  │  │ Active Calls │ │ In Queue     │ │ Conferences  │ │ Agents  │ │    │
│  │  │     247      │ │     12       │ │      8       │ │  45/60  │ │    │
│  │  │   ▲ 15%      │ │   ▼ 3        │ │   ━ 0       │ │ Online  │ │    │
│  │  └──────────────┘ └──────────────┘ └──────────────┘ └─────────┘ │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌───────────────────────────┐  ┌───────────────────────────────────┐   │
│  │    CALL VOLUME (24h)      │  │      QUALITY METRICS              │   │
│  │                           │  │                                   │   │
│  │  ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁▂▃▄▅▆▇   │  │  MOS: 4.2  │  ASR: 94%          │   │
│  │                           │  │  Jitter: 12ms │ Loss: 0.3%       │   │
│  │  Peak: 450 calls/min      │  │                                   │   │
│  └───────────────────────────┘  └───────────────────────────────────┘   │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    CALL FLOW VISUALIZER                          │    │
│  │                                                                  │    │
│  │   [Search by Call-ID: ________________________] [Search]         │    │
│  │                                                                  │    │
│  │   Call: abc-123-def                                             │    │
│  │   ┌──────────┐        ┌──────────┐        ┌──────────┐         │    │
│  │   │  Alice   │──SIP──►│   FCP    │──SIP──►│  PSTN    │         │    │
│  │   │ WebRTC   │◄─RTP──►│  B2BUA   │◄─RTP──►│ Gateway  │         │    │
│  │   └──────────┘        └──────────┘        └──────────┘         │    │
│  │                                                                  │    │
│  │   Timeline: [=====INVITE===][==180==][===200 OK===][===BYE===]  │    │
│  │                                                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 10. Security & Compliance

### 10.1 Per-Tenant Security Profiles

```rust
/// Security profile per tenant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSecurityProfile {
    pub tenant_id: TenantId,
    pub profile_type: SecurityProfileType,
    pub tls: TlsPolicy,
    pub srtp: SrtpPolicy,
    pub authentication: AuthPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecurityProfileType {
    /// Maximum security - TLS required, strong ciphers only
    Strict,
    /// Balanced - TLS preferred, fallback allowed
    Standard,
    /// Legacy interop - weak ciphers allowed for old equipment
    Legacy,
    /// Custom profile
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsPolicy {
    pub required: bool,
    pub min_version: TlsVersion,
    pub allowed_ciphers: Vec<String>,
    pub verify_peer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrtpPolicy {
    pub required: bool,
    pub allowed_suites: Vec<SrtpCryptoSuite>,
}

impl TenantSecurityProfile {
    pub fn strict() -> Self {
        Self {
            tenant_id: TenantId::nil(),
            profile_type: SecurityProfileType::Strict,
            tls: TlsPolicy {
                required: true,
                min_version: TlsVersion::V1_3,
                allowed_ciphers: vec![
                    "TLS_AES_256_GCM_SHA384".to_string(),
                    "TLS_CHACHA20_POLY1305_SHA256".to_string(),
                ],
                verify_peer: true,
            },
            srtp: SrtpPolicy {
                required: true,
                allowed_suites: vec![
                    SrtpCryptoSuite::AeadAes256Gcm,
                    SrtpCryptoSuite::AeadAes128Gcm,
                ],
            },
            authentication: AuthPolicy::MtlsOnly,
        }
    }
    
    pub fn legacy() -> Self {
        Self {
            tenant_id: TenantId::nil(),
            profile_type: SecurityProfileType::Legacy,
            tls: TlsPolicy {
                required: false,
                min_version: TlsVersion::V1_2,
                allowed_ciphers: vec![], // Allow all
                verify_peer: false,
            },
            srtp: SrtpPolicy {
                required: false,
                allowed_suites: vec![
                    SrtpCryptoSuite::AesCm128HmacSha1_80,
                    SrtpCryptoSuite::AesCm128HmacSha1_32,
                ],
            },
            authentication: AuthPolicy::DigestOrNone,
        }
    }
}
```

### 10.2 Data Encryption & Retention

```rust
/// Storage encryption configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    /// KMS provider
    pub kms: KmsProvider,
    /// Key rotation period
    pub key_rotation: Duration,
    /// Per-tenant keys
    pub tenant_keys: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KmsProvider {
    /// AWS KMS
    Aws { region: String, key_id: String },
    /// GCP KMS
    Gcp { project: String, keyring: String, key: String },
    /// HashiCorp Vault
    Vault { url: String, path: String },
    /// Local (development only)
    Local { key_file: PathBuf },
}

/// Data retention policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// CDR retention period
    pub cdr_retention: Duration,
    /// Recording retention period
    pub recording_retention: Duration,
    /// Voicemail retention period
    pub voicemail_retention: Duration,
    /// Transcript retention period
    pub transcript_retention: Duration,
    /// Legal hold flag (overrides retention)
    pub legal_hold: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            cdr_retention: Duration::from_days(365),
            recording_retention: Duration::from_days(90),
            voicemail_retention: Duration::from_days(30),
            transcript_retention: Duration::from_days(90),
            legal_hold: false,
        }
    }
}
```

### 10.3 Regulatory Compliance (v3.0)

```rust
/// E911 integration
pub struct E911Config {
    pub provider: E911Provider,
    pub routing_mode: E911RoutingMode,
}

pub enum E911Provider {
    Bandwidth,
    Intrado,
    Custom { endpoint: String },
}

/// Lawful intercept (CALEA) interface
pub trait LawfulInterceptInterface {
    /// Check if target is under surveillance
    fn is_target(&self, identifier: &str) -> bool;
    
    /// Deliver call content (CDC)
    async fn deliver_content(&self, call_id: &CallId, media: MediaStream);
    
    /// Deliver call data (CCC)
    async fn deliver_metadata(&self, cdr: &CallDetailRecord);
}

/// Recording consent handling
pub struct ConsentConfig {
    /// Require consent before recording
    pub require_consent: bool,
    /// Consent announcement prompt
    pub announcement: Option<Prompt>,
    /// Single-party vs all-party consent
    pub consent_type: ConsentType,
}

pub enum ConsentType {
    SingleParty,  // Only one party needs to consent
    AllParty,     // All parties must consent
}
```

---

## 11. Observability

### 11.1 Structured Logging Standard

```rust
/// Standard log fields that must be present
pub struct LogContext {
    /// Always present
    pub timestamp: DateTime<Utc>,
    pub level: Level,
    pub component: &'static str,
    pub node_id: NodeId,
    
    /// When applicable
    pub tenant_id: Option<TenantId>,
    pub call_id: Option<CallId>,
    pub session_id: Option<SessionId>,
    pub dialog_id: Option<DialogId>,
    
    /// Tracing
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

// Usage with tracing
tracing::info!(
    tenant_id = %tenant_id,
    call_id = %call_id,
    from = %from_uri,
    to = %to_uri,
    "Call initiated"
);

// Produces JSON:
// {
//   "timestamp": "2024-01-15T10:30:00Z",
//   "level": "INFO",
//   "component": "b2bua",
//   "node_id": "node-1",
//   "tenant_id": "tenant-abc",
//   "call_id": "call-123",
//   "trace_id": "abc123",
//   "span_id": "def456",
//   "message": "Call initiated",
//   "from": "sip:alice@example.com",
//   "to": "sip:bob@example.com"
// }
```

### 11.2 Chaos Testing (Roadmap)

```yaml
# Chaos test scenarios to implement
chaos_tests:
  - name: "Mid-call node failure"
    description: "Kill a signaling node during active calls"
    steps:
      - establish_calls: 100
      - wait: 30s
      - kill_node: "signaling-2"
      - verify:
          - calls_dropped: "<5%"
          - failover_time: "<2s"
          - no_zombie_dialogs: true

  - name: "Media node crash"
    description: "Kill media node, verify handoff"
    steps:
      - establish_calls: 50
      - wait: 10s
      - kill_node: "media-1"
      - verify:
          - audio_gap: "<500ms"
          - calls_recovered: ">95%"

  - name: "Database partition"
    description: "Simulate network partition to DB"
    steps:
      - establish_calls: 100
      - partition: "db-primary"
      - wait: 60s
      - verify:
          - existing_calls: "continue"
          - new_registrations: "fail gracefully"
      - heal_partition
      - verify:
          - system_recovers: true
```

---

## 12. Revised Roadmap

### Phase 1: MVP - "Rust Asterisk-lite" (Months 1-4)

**Goal:** Single-node PBX with basic call control

- [ ] `fcp-core`: Types, traits, error handling
- [ ] `siphon-core`: SIP parser (INVITE/BYE/CANCEL/ACK/REGISTER/OPTIONS)
- [ ] `siphon-transport`: UDP transport
- [ ] `siphon-transaction`: Client & server transactions
- [ ] `siphon-dialog`: Basic dialog management
- [ ] `fcp-registrar`: In-memory registration
- [ ] `fcp-proxy`: Stateless proxy
- [ ] `fcp-b2bua`: Basic B2BUA (call setup, teardown)
- [ ] `fcp-rtp`: RTP relay (no transcoding)
- [ ] `fcp-storage-sqlite`: SQLite backend
- [ ] `fcp-api-rest`: Basic REST API (calls, registrations)
- [ ] `fcp-cli`: Command-line tool
- [ ] Prometheus metrics, structured logging
- [ ] Docker image

**Deliverable:** Working PBX that can register phones and complete calls.

### Phase 2: Programmable Voice (Months 5-8)

**Goal:** CPaaS-like programmability

- [ ] `fcp-flow-engine`: Call flow DSL (basic nodes)
- [ ] `fcp-ivr`: IVR service with flow engine
- [ ] Recording support
- [ ] DTMF handling
- [ ] `fcp-api-websocket`: Real-time events
- [ ] `fcp-webhook-dispatcher`: Outbound webhooks
- [ ] TCP + TLS transports
- [ ] PostgreSQL backend
- [ ] Basic web admin UI
- [ ] docker-compose deployment

**Deliverable:** Programmable voice platform with webhooks.

### Phase 3: Applications & Clustering (Months 9-12)

**Goal:** Full application suite, HA clustering

- [ ] `fcp-queue`: ACD/Queue service
- [ ] `fcp-voicemail`: Voicemail system
- [ ] `fcp-presence`: Presence server
- [ ] `fcp-cluster-membership`: SWIM implementation
- [ ] `fcp-cluster-state`: Distributed registration/dialog cache
- [ ] Redis for distributed state
- [ ] `fcp-webrtc`: Basic WebRTC gateway
- [ ] Plugin system (Lua runtime)
- [ ] `fcp-api-grpc`: gRPC API
- [ ] Kubernetes deployment

**Deliverable:** HA cluster with full application suite.

### Phase 4: Enterprise Features (Months 13-16)

**Goal:** Production-ready for enterprise

- [ ] Full SBC functionality
- [ ] `fcp-routing-engine`: LCR with carrier health
- [ ] `fcp-transcoder`: Audio transcoding
- [ ] `fcp-cdr-processor`: Dedicated CDR pipeline
- [ ] `fcp-policy-engine`: Rate limiting, fraud detection
- [ ] WASM plugin runtime
- [ ] Multi-tenant isolation hardening
- [ ] Encryption at rest (KMS integration)
- [ ] `fcp-provisioning`: Device provisioning server (Polycom, Yealink, Cisco, Grandstream) — serves per-device config files over HTTP, templated from device/user records via DHCP option 66 discovery
- [ ] Advanced admin UI (call ladder, quality dashboards)
- [ ] Comprehensive documentation

**Deliverable:** Enterprise-ready platform.

### Phase 5: Carrier Grade (Months 17-24)

**Goal:** Carrier-scale deployment

- [ ] Multi-region support
- [ ] CockroachDB integration
- [ ] STIR/SHAKEN
- [ ] E911 integration
- [ ] Video transcoding/conferencing
- [ ] Full AI integration hooks
- [ ] Regulatory compliance features
- [ ] 1M+ subscriber support

**Deliverable:** Carrier-grade platform.

---

## Appendix A: Naming Conventions

| Old Name | New Name | Reason |
|----------|----------|--------|
| `fcp-signal` | `fcp-signaling` | Consistency |
| `fcp-siphon` | `siphon/*` | Stack is standalone, multiple crates |
| N/A | `fcp-control-plane` | Explicit separation |
| N/A | `fcp-data-plane` | Explicit separation |
| N/A | `fcp-event-bus` | New component |
| N/A | `fcp-plugins` | Plugin system |

## Appendix B: Feature Flags

Optional dependencies to keep builds lean:

```toml
[features]
default = ["sqlite", "rest-api"]

# Storage backends
sqlite = ["sqlx/sqlite"]
postgres = ["sqlx/postgres"]
cockroach = ["postgres"]

# APIs
rest-api = ["axum", "tower-http"]
grpc-api = ["tonic", "prost"]
websocket-api = ["tokio-tungstenite"]

# Transports
tls = ["rustls", "tokio-rustls"]
websocket = ["tokio-tungstenite"]

# Applications
ivr = []
queue = []
voicemail = []
presence = []
conference = []

# Plugins
lua-plugins = ["mlua"]
wasm-plugins = ["wasmtime"]

# Clustering
cluster = ["swim-rs"]
consul = ["consul-rs"]
etcd = ["etcd-client"]

# Observability
otlp = ["opentelemetry-otlp"]
```

---

*Document Version: 0.2*
*Last Updated: 2024*
*Incorporates feedback from architecture review*
