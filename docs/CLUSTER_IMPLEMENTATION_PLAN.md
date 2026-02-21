 FCP Clustering Implementation Plan

 Context

 FCP currently operates as a single-node platform. All registrations, events, configuration,
  and call state live on one machine. To support production HA deployments, we need multiple
  FCP nodes sharing state. The agreed architecture:

 - PostgreSQL cluster — persistent data (users, tenants, CDRs, extensions, webhooks)
 - Redis cluster — ephemeral runtime state (registrations, presence, cluster membership)
 - Per-node in-memory — transient state (active calls, SIP dialogs/transactions)
 - Local cache + event invalidation — hot configuration data (credentials, extensions,
 devices)

 PostgreSQL and Redis run on separate server groups.

 ---
 Phase 1: Redis Registration Store

 Why first: Registrations are on the call-routing hot path (every INVITE). Moving them to
 Redis makes them instantly shared across nodes and gives sub-millisecond lookups with
 native TTL expiry.

 New crate: crates/fcp-storage-redis/

 Dependencies: redis (with tokio-comp, connection-manager, cluster-async features),
 fcp-core, async-trait, serde, serde_json, tracing, tokio

 Core struct:
 pub struct RedisStorage {
     conn: redis::aio::ConnectionManager,
     key_prefix: String,  // default "reg:"
 }

 Redis key design: Hash per AOR
 Key:    reg:{aor}              e.g. "reg:sip:alice@example.com"
 Type:   Hash
 Fields: {contact_uri} => JSON(Registration)
 TTL:    max(expires_at) across all contacts

 On get_registrations: HGETALL + filter expired contacts client-side (1-4 contacts typical
 per AOR, negligible cost). On put_registration: pipeline HSET + EXPIRE with max TTL.
 cleanup_expired is a no-op — Redis TTL handles it.

 Split storage in fcp-call-manager

 DynStorage::new() already accepts individual trait objects. When registration_backend =
 "redis" is configured, construct DynStorage with Redis for registration_store and Postgres
 for everything else:

 let dyn_storage = DynStorage::new(
     pg.clone(),         // credential_store
     redis_reg_store,    // registration_store  ← Redis
     pg.clone(),         // cdr_store
     pg.clone(),         // phone_number_store
     // ... rest from pg
 );

 Backward-compatible: from_all_in_one continues to work for single-node SQLite.

 Config changes (fcp-call-manager/src/config.rs)

 Add to StorageSection:
 pub redis_url: Option<String>,            // e.g. "redis://redis:6379"
 pub registration_backend: Option<String>, // "redis" | None (use main backend)

 Files

 File: crates/fcp-storage-redis/Cargo.toml
 Action: Create
 ────────────────────────────────────────
 File: crates/fcp-storage-redis/src/lib.rs
 Action: Create — RedisStorage struct, connection setup
 ────────────────────────────────────────
 File: crates/fcp-storage-redis/src/registration.rs
 Action: Create — RegistrationStore impl
 ────────────────────────────────────────
 File: Cargo.toml (workspace)
 Action: Add member
 ────────────────────────────────────────
 File: crates/fcp-call-manager/Cargo.toml
 Action: Add fcp-storage-redis dep (feature-gated)
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/config.rs
 Action: Add redis_url, registration_backend to StorageSection
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/storage.rs
 Action: Add init_redis_registrations(), modify initialize_storage() to support split
   backends

 Verification

 - Unit tests: mock Redis or use redis-test for RegistrationStore trait compliance
 - Integration test: start Redis, register a SIP client, verify HGETALL reg:{aor} returns
 the binding, verify call routes to registered contact

 ---
 Phase 2: Distributed Event Bus (Redis Pub/Sub + Streams)

 Why second: With shared registrations, nodes can route calls to any user. But events (call
 state changes, webhooks, WebSocket streaming) are still node-local. The event bus is the
 cluster's nervous system.

 Two-layer architecture

 Node A                              Node B
 ┌──────────────┐                   ┌──────────────┐
 │ InMemoryBus  │ ◄── consume ──── │ InMemoryBus  │
 │ (local fans) │                   │ (local fans) │
 └──────┬───────┘                   └──────┬───────┘
        │ publish                          │ publish
        ▼                                  ▼
 ┌─────────────────────────────────────────────────┐
 │  Redis: PUBLISH fcp:events (real-time fan-out)  │
 │  Redis: XADD fcp:event_stream (durability)      │
 └─────────────────────────────────────────────────┘

 - Pub/Sub for real-time fan-out — every node gets every event
 - Streams for durability — events-server uses a consumer group for reliable processing
 (replay after downtime)
 - Each node filters out self-published events via node_id
 - InMemoryEventBus stays as the local fan-out mechanism

 Implementation in crates/fcp-storage-redis/src/event_bus.rs

 pub struct RedisEventBus {
     conn: redis::aio::ConnectionManager,
     client: redis::Client,
     node_id: String,
     channel: String,       // "fcp:events"
     stream_key: String,    // "fcp:event_stream"
 }

 Implements EventPublisher — on publish: PUBLISH + XADD in a pipeline.

 For subscription: spawn background task that subscribes to the Redis Pub/Sub channel,
 deserializes events, filters self-published, and re-publishes to local InMemoryEventBus.

 Eliminates HTTP event forwarding

 The current POST /ingest flow from call-manager to events-server is replaced by Redis.
 Events-server reads from Redis Streams consumer group instead.

 Files

 File: crates/fcp-storage-redis/src/event_bus.rs
 Action: Create — RedisEventBus implementing EventPublisher + subscriber loop
 ────────────────────────────────────────
 File: crates/fcp-storage-redis/src/lib.rs
 Action: Add event_bus module
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/main.rs
 Action: Replace HTTP event forwarder with Redis bridge
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/config.rs
 Action: Add event_bus section (redis_url, channel, stream_key)
 ────────────────────────────────────────
 File: crates/fcp-events-server/src/main.rs
 Action: Consume from Redis Streams instead of HTTP POST /ingest
 ────────────────────────────────────────
 File: crates/fcp-events-server/Cargo.toml
 Action: Add fcp-storage-redis dep

 Verification

 - Start two call-manager nodes pointing at same Redis
 - Register user A on node 1, user B on node 2
 - Call from A to B — verify CallEvent::Initiated appears on both nodes' event streams
 - Verify events-server WebSocket receives events from both nodes

 ---
 Phase 3: Cluster Membership

 Why third: Phases 1-2 give us shared data and shared events. Phase 3 adds operational
 awareness — which nodes exist, their health, their load.

 Redis-based membership (not SWIM)

 SWIM is unnecessary complexity. Nodes already have Redis from Phases 1-2. Simple
 heartbeat-based membership:

 - Each node writes cluster:node:{node_id} hash to Redis with 15s TTL
 - Refreshed every 5s
 - If a node crashes, its key expires in 15s
 - Any node can SCAN cluster:node:* to discover the cluster

 New crate: crates/fcp-cluster/

 pub struct ClusterMember {
     pub node_id: String,
     pub node_type: String,         // "call_manager", "events_server", etc.
     pub http_address: SocketAddr,
     pub sip_address: SocketAddr,
     pub capabilities: Vec<String>, // ["registrar", "b2bua", "queue"]
     pub active_calls: usize,
     pub started_at: SystemTime,
     pub version: String,
 }

 pub struct ClusterManager {
     conn: redis::aio::ConnectionManager,
     self_info: ClusterMember,
     members: DashMap<String, ClusterMember>,  // local cache
 }

 Methods: start() (heartbeat loop + discovery), members(), get_member(), least_loaded().

 Call ownership registry

 Track which node owns which call for cross-node operations:
 Key:    cluster:call:{call_id}
 Value:  node_id
 TTL:    3600s (max call duration)

 Set on call creation, deleted on termination. Enables any node to find the owning node.

 Files

 File: crates/fcp-cluster/Cargo.toml
 Action: Create
 ────────────────────────────────────────
 File: crates/fcp-cluster/src/lib.rs
 Action: Create — ClusterManager, ClusterMember
 ────────────────────────────────────────
 File: crates/fcp-cluster/src/membership.rs
 Action: Create — heartbeat, discovery, health
 ────────────────────────────────────────
 File: crates/fcp-cluster/src/call_registry.rs
 Action: Create — call ownership tracking
 ────────────────────────────────────────
 File: Cargo.toml (workspace)
 Action: Add member
 ────────────────────────────────────────
 File: crates/fcp-call-manager/Cargo.toml
 Action: Add fcp-cluster dep
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/main.rs
 Action: Start ClusterManager, register calls
 ────────────────────────────────────────
 File: crates/fcp-events-server/src/nodes.rs
 Action: Read membership from Redis instead of implicit tracking

 Verification

 - Start 3 call-manager nodes, verify all discover each other via SCAN cluster:node:*
 - Kill one node, verify its key expires and other nodes see it as gone
 - Make a call, verify cluster:call:{id} key exists in Redis with correct node_id

 ---
 Phase 4: Configuration Cache Layer

 Why fourth: With clustering operational, the next bottleneck is PostgreSQL queries on the
 hot path. Every INVITE queries credentials, extensions, and devices. A local DashMap cache
 with event-driven invalidation fixes this.

 Per-trait cache wrappers

 Create thin wrappers for the 3 hot-path stores:

 pub struct CachedCredentialStore {
     inner: Arc<dyn CredentialStore>,
     cache: DashMap<String, (Option<User>, Instant)>,
     ttl: Duration,  // default 60s
 }

 pub struct CachedExtensionStore { /* same pattern */ }
 pub struct CachedDeviceStore { /* same pattern */ }

 On get_user(username): check DashMap → if present and not expired, return cached. Otherwise
  query inner store, cache result.

 Event-driven invalidation

 Add ConfigChanged variant to SystemEvent:
 ConfigChanged {
     entity_type: String,  // "user", "extension", "device"
     entity_id: String,
     tenant_id: TenantId,
     timestamp: SystemTime,
 }

 When the admin API modifies a user/extension/device, it publishes this event. All nodes
 receive it via the Redis event bus (Phase 2) and evict the matching cache entry. TTL
 provides fallback if an invalidation is missed.

 Wired into DynStorage construction

 let cached_creds = Arc::new(CachedCredentialStore::new(pg.clone(),
 Duration::from_secs(60)));
 let cached_exts = Arc::new(CachedExtensionStore::new(pg.clone(), Duration::from_secs(60)));
 let cached_devs = Arc::new(CachedDeviceStore::new(pg.clone(), Duration::from_secs(60)));

 let dyn_storage = DynStorage::new(
     cached_creds,       // cached
     redis_reg_store,    // Redis (Phase 1)
     pg.clone(),         // CDR - no cache needed
     pg.clone(),         // phone numbers
     pg.clone(),         // line bindings
     cached_exts,        // cached
     cached_devs,        // cached
     // ... rest uncached from pg
 );

 Files

 File: crates/fcp-core/src/cached_storage.rs
 Action: Create — CachedCredentialStore, CachedExtensionStore, CachedDeviceStore
 ────────────────────────────────────────
 File: crates/fcp-core/src/lib.rs
 Action: Add cached_storage module
 ────────────────────────────────────────
 File: crates/fcp-events/src/types.rs
 Action: Add ConfigChanged to SystemEvent
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/storage.rs
 Action: Wrap hot-path stores in cache
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/main.rs
 Action: Subscribe to ConfigChanged events, evict cache
 ────────────────────────────────────────
 File: crates/fcp-admin/src/
 Action: Publish ConfigChanged on user/extension/device mutations

 Verification

 - Query user via SIP REGISTER — cache HIT on second attempt (tracing log)
 - Update user via admin API — verify cache eviction event flows, next lookup is a cache
 MISS
 - Benchmark: measure call setup latency with and without cache

 ---
 Phase 5: Cross-Node Call Operations

 Why last: Requires all previous phases. Most call routing already works cross-node
 (registrations are in shared Redis, so any node can INVITE any user directly). This phase
 handles the edge cases.

 What already works (no changes needed)

 - Call routing — any node looks up registrations in Redis, sends INVITE directly to the
 contact URI
 - Call transfers (REFER) — transferring node looks up target in Redis, sends new INVITE
 itself
 - Event visibility — all nodes see all events via Redis Pub/Sub

 What needs cross-node coordination

 Call parking/pickup: Park on node A, pick up from node B.
 - Move ParkingManager data to Redis:
 Key:    park:{orbit}
 Hash:   { call_id, node_id, parker_extension, tenant_id, parked_at }
 TTL:    parking timeout (120s default)
 - Pickup: read park:{orbit} from Redis, HTTP POST to owning node's /internal/unpark

 Supervisor operations: Monitor/whisper/barge a call on another node.
 - Use call ownership registry (Phase 3) to find owning node
 - HTTP POST to owning node's /internal/supervisor endpoint

 Inter-node HTTP API

 Add /internal/* endpoints to fcp-call-manager (authenticated with shared cluster secret):

 POST /internal/unpark      { orbit }           → unpark and bridge
 POST /internal/hangup      { call_id }         → BYE the call
 POST /internal/supervisor   { call_id, mode }  → inject supervisor

 Node client in fcp-cluster

 pub struct NodeClient {
     http: reqwest::Client,
     cluster_secret: String,
 }

 impl NodeClient {
     pub async fn unpark(&self, node: &ClusterMember, orbit: u16) -> Result<UnparkResult>;
     pub async fn hangup(&self, node: &ClusterMember, call_id: &str) -> Result<()>;
 }

 Files

 File: crates/fcp-cluster/src/node_client.rs
 Action: Create — HTTP client for inter-node calls
 ────────────────────────────────────────
 File: crates/fcp-storage-redis/src/parking.rs
 Action: Create — Redis-backed parking store
 ────────────────────────────────────────
 File: crates/fcp-call-manager/src/health.rs
 Action: Add /internal/* endpoints
 ────────────────────────────────────────
 File: crates/fcp-pbx/src/parking.rs
 Action: Modify — use Redis parking store when clustered

 Verification

 - Two nodes, park call on node A (*70), pick up from node B (*7101) — call bridges
 correctly
 - Kill node A while call is parked — verify park:{orbit} key expires via TTL

 ---
 Dependency Graph

 Phase 1: Redis Registration Store
     └─► Phase 2: Distributed Event Bus (uses same Redis connection)
          ├─► Phase 3: Cluster Membership (uses Redis + event bus)
          └─► Phase 4: Config Cache (uses event bus for invalidation)
               └─► Phase 5: Cross-Node Operations (uses membership + registrations + events)

 Phases 3 and 4 are independent of each other — can be done in parallel.

 ---
 New Crates Summary

 ┌───────────────────┬───────────────────────────────────────────────────────┐
 │       Crate       │                        Purpose                        │
 ├───────────────────┼───────────────────────────────────────────────────────┤
 │ fcp-storage-redis │ Redis backend: registrations, event bus, parking      │
 ├───────────────────┼───────────────────────────────────────────────────────┤
 │ fcp-cluster       │ Cluster membership, call ownership, inter-node client │
 └───────────────────┴───────────────────────────────────────────────────────┘

 Implementation order for coding

 1. fcp-storage-redis crate with RedisStorage + RegistrationStore impl
 2. Split storage support in fcp-call-manager (config + storage.rs)
 3. RedisEventBus in fcp-storage-redis + wiring in call-manager and events-server
 4. fcp-cluster crate with ClusterManager + heartbeat + call registry
 5. CachedCredentialStore / CachedExtensionStore / CachedDeviceStore in fcp-core
 6. ConfigChanged event + cache invalidation wiring
 7. Redis parking store + cross-node /internal/* endpoints
